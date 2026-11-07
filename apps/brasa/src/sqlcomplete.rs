//! Autocompletar da Query: o que sugerir para a palavra sob o cursor, olhando o esquema do banco e o texto do
//! comando. Só lógica, sem janela, para poder testar.
//!
//! Regras (simples de propósito, para o resultado ser previsível):
//! - depois de `FROM`, `JOIN`, `INTO`, `UPDATE`, `TABLE` (ou de uma vírgula na lista do `FROM`): tabelas, visões e esquemas;
//! - `algo.`: as colunas da tabela (ou apelido) `algo`, ou as tabelas do esquema `algo`;
//! - no resto do comando: colunas das tabelas citadas no comando, funções e palavras do SQL.

use ox_db::Table;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// o que aparece na lista
    pub label: String,
    /// o que entra no texto
    pub insert: String,
    /// tipo da coluna, "tabela", "visão", "esquema", "função", "palavra"
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// quantos caracteres antes do cursor fazem parte da palavra que será trocada
    pub prefix_len: usize,
    pub items: Vec<Item>,
}

const KEYWORDS: &[&str] = &[
    "select", "from", "where", "group by", "order by", "having", "limit", "offset", "join", "left join", "right join", "inner join", "full join", "cross join", "on", "as", "and", "or", "not", "in", "is null", "is not null", "like", "ilike", "between", "exists", "distinct", "union", "union all", "case", "when", "then", "else", "end",
    "insert into", "values", "update", "set", "delete from", "returning", "create table", "alter table", "drop table", "with", "asc", "desc", "begin", "commit", "rollback", "explain",
];

const FUNCTIONS: &[&str] = &["count", "sum", "avg", "min", "max", "coalesce", "nullif", "now", "lower", "upper", "length", "substr", "trim", "round", "cast", "date", "extract", "concat", "replace", "abs", "random"];

/// Palavras depois das quais vem o nome de uma tabela.
const TABLE_WORDS: &[&str] = &["from", "join", "into", "update", "table", "truncate", "using"];

/// Palavras que terminam a lista do `FROM`.
const CLAUSE_WORDS: &[&str] = &["where", "group", "order", "having", "limit", "offset", "on", "select", "set", "values", "returning", "union", "as", "and", "or", "inner", "left", "right", "full", "cross", "natural", "outer"];

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// O nome precisa de aspas para valer como foi escrito?
fn needs_quotes(name: &str) -> bool {
    name.is_empty() || name.chars().next().is_some_and(|c| c.is_ascii_digit()) || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn ident(name: &str) -> String {
    if needs_quotes(name) {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        name.to_string()
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Comma,
    Dot,
    Other,
}

/// Quebra o texto em palavras, vírgulas e pontos, ignorando textos entre aspas simples e comentários.
fn tokens(text: &str) -> Vec<Tok> {
    let (mut out, cs) = (Vec::new(), text.chars().collect::<Vec<_>>());
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\'' {
            i += 1;
            while i < cs.len() && cs[i] != '\'' {
                i += 1;
            }
            i += 1;
            out.push(Tok::Other);
        } else if c == '-' && cs.get(i + 1) == Some(&'-') {
            while i < cs.len() && cs[i] != '\n' {
                i += 1;
            }
        } else if c == '"' {
            let start = i + 1;
            i += 1;
            while i < cs.len() && cs[i] != '"' {
                i += 1;
            }
            out.push(Tok::Word(cs[start..i.min(cs.len())].iter().collect::<String>()));
            i += 1;
        } else if is_ident(c) {
            let start = i;
            while i < cs.len() && is_ident(cs[i]) {
                i += 1;
            }
            out.push(Tok::Word(cs[start..i].iter().collect::<String>().to_lowercase()));
        } else {
            out.push(match c {
                ',' => Tok::Comma,
                '.' => Tok::Dot,
                c if c.is_whitespace() => {
                    i += 1;
                    continue;
                }
                _ => Tok::Other,
            });
            i += 1;
        }
    }
    out
}

/// Tabelas citadas no comando (`FROM x`, `JOIN y b`, `UPDATE z`...), com o apelido se houver: (apelido ou nome, índice).
fn referenced(toks: &[Tok], tables: &[Table]) -> Vec<(String, usize)> {
    let find = |schema: Option<&str>, name: &str| tables.iter().position(|t| t.name.eq_ignore_ascii_case(name) && schema.is_none_or(|s| t.schema.as_deref().is_some_and(|ts| ts.eq_ignore_ascii_case(s))));
    let (mut out, mut i) = (Vec::new(), 0);
    let mut in_list = false;
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) if TABLE_WORDS.contains(&w.as_str()) => in_list = true,
            Tok::Word(w) if CLAUSE_WORDS.contains(&w.as_str()) && !matches!(w.as_str(), "as") => in_list = false,
            Tok::Comma if in_list => {}
            _ => {
                i += 1;
                continue;
            }
        }
        // depois de uma palavra de tabela ou de uma vírgula na lista: [esquema.]nome [AS] [apelido]
        let mut j = i + 1;
        let first = match toks.get(j) {
            Some(Tok::Word(w)) if in_list && !CLAUSE_WORDS.contains(&w.as_str()) => w.clone(),
            _ => {
                i += 1;
                continue;
            }
        };
        j += 1;
        let (schema, name) = if toks.get(j) == Some(&Tok::Dot) {
            match toks.get(j + 1) {
                Some(Tok::Word(n)) => {
                    j += 2;
                    (Some(first), n.clone())
                }
                _ => (None, first),
            }
        } else {
            (None, first)
        };
        if toks.get(j).is_some_and(|t| matches!(t, Tok::Word(w) if w == "as")) {
            j += 1;
        }
        let alias = match toks.get(j) {
            Some(Tok::Word(a)) if !CLAUSE_WORDS.contains(&a.as_str()) && !TABLE_WORDS.contains(&a.as_str()) => Some(a.clone()),
            _ => None,
        };
        if let Some(ix) = find(schema.as_deref(), &name) {
            out.push((alias.clone().unwrap_or_else(|| name.to_lowercase()), ix));
            if alias.is_some() {
                out.push((name.to_lowercase(), ix));
            }
        }
        i = j.max(i + 1);
    }
    out
}

/// O texto do comando que contém o cursor (entre `;`), e onde o cursor está nele (em caracteres).
fn statement(text: &str, cursor: usize) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let start = chars[..cursor].iter().rposition(|&c| c == ';').map_or(0, |p| p + 1);
    let end = chars[cursor..].iter().position(|&c| c == ';').map_or(chars.len(), |p| cursor + p);
    (chars[start..end].iter().collect(), cursor - start)
}

fn matches_prefix(candidate: &str, prefix: &str) -> bool {
    candidate.to_lowercase().starts_with(&prefix.to_lowercase())
}

/// Sugestões para a palavra antes do cursor (`cursor` em caracteres a partir do começo de `text`). `explicit`: o
/// usuário apertou Tab; só então se sugere com a palavra vazia. `None`: nada a sugerir.
pub fn suggest(text: &str, cursor: usize, tables: &[Table], explicit: bool) -> Option<Suggestion> {
    let (stmt, cur) = statement(text, cursor);
    let chars: Vec<char> = stmt.chars().collect();
    let mut ps = cur;
    while ps > 0 && is_ident(chars[ps - 1]) {
        ps -= 1;
    }
    let prefix: String = chars[ps..cur].iter().collect();
    // não sugere dentro de texto entre aspas simples
    if chars[..ps].iter().filter(|&&c| c == '\'').count() % 2 == 1 {
        return None;
    }
    let all = tokens(&stmt);
    let refs = referenced(&all, tables);
    let mut items: Vec<Item> = Vec::new();

    // "algo." → colunas de algo, ou tabelas do esquema algo
    if ps > 0 && chars[ps - 1] == '.' {
        let mut qs = ps - 1;
        while qs > 0 && is_ident(chars[qs - 1]) {
            qs -= 1;
        }
        let q: String = chars[qs..ps - 1].iter().collect::<String>().to_lowercase();
        let q = q.trim_matches('"').to_lowercase();
        let mut cols: Vec<&Table> = refs.iter().filter(|(a, _)| *a == q).map(|(_, i)| &tables[*i]).collect();
        if cols.is_empty() {
            cols = tables.iter().filter(|t| t.name.eq_ignore_ascii_case(&q)).collect();
        }
        for t in cols.into_iter().take(1) {
            for c in &t.columns {
                if matches_prefix(&c.name, &prefix) {
                    items.push(Item { label: c.name.clone(), insert: ident(&c.name), detail: c.decl_type.to_lowercase() });
                }
            }
        }
        if items.is_empty() {
            for t in tables.iter().filter(|t| t.schema.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&q))) {
                if matches_prefix(&t.name, &prefix) {
                    items.push(Item { label: t.name.clone(), insert: ident(&t.name), detail: if t.is_view { "visão" } else { "tabela" }.into() });
                }
            }
        }
        return finish(items, prefix);
    }

    // o que vem logo antes da palavra: decide se é lugar de tabela
    let before = tokens(&chars[..ps].iter().collect::<String>());
    let table_place = match before.last() {
        Some(Tok::Word(w)) => TABLE_WORDS.contains(&w.as_str()),
        Some(Tok::Comma) => {
            // vírgula na lista do FROM: a última palavra-chave decide
            before.iter().rev().find_map(|t| match t {
                Tok::Word(w) if TABLE_WORDS.contains(&w.as_str()) => Some(true),
                Tok::Word(w) if CLAUSE_WORDS.contains(&w.as_str()) => Some(false),
                _ => None,
            }) == Some(true)
        }
        _ => false,
    };

    if table_place {
        let mut schemas: Vec<String> = tables.iter().filter_map(|t| t.schema.clone()).filter(|s| s != "public").collect();
        schemas.sort();
        schemas.dedup();
        for t in tables {
            let label = t.label();
            if matches_prefix(&label, &prefix) || matches_prefix(&t.name, &prefix) {
                let insert = match t.schema.as_deref() {
                    Some(s) if s != "public" => format!("{}.{}", ident(s), ident(&t.name)),
                    _ => ident(&t.name),
                };
                items.push(Item { label, insert, detail: if t.is_view { "visão" } else { "tabela" }.into() });
            }
        }
        for s in schemas {
            if matches_prefix(&s, &prefix) {
                items.push(Item { label: format!("{s}."), insert: format!("{}.", ident(&s)), detail: "esquema".into() });
            }
        }
        return finish(items, prefix);
    }

    // sem palavra e sem lugar de tabela: Tab continua recuando o texto
    if prefix.is_empty() {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    for (_, ix) in &refs {
        for c in &tables[*ix].columns {
            if matches_prefix(&c.name, &prefix) && seen.insert(c.name.clone()) {
                items.push(Item { label: c.name.clone(), insert: ident(&c.name), detail: c.decl_type.to_lowercase() });
            }
        }
    }
    // palavras no mesmo caso em que o usuário digitou
    let upper = prefix.chars().next().is_some_and(char::is_uppercase) && prefix.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
    let case = |w: &str| if upper { w.to_uppercase() } else { w.to_string() };
    for f in FUNCTIONS {
        if matches_prefix(f, &prefix) {
            items.push(Item { label: format!("{}()", case(f)), insert: format!("{}(", case(f)), detail: "função".into() });
        }
    }
    for k in KEYWORDS {
        if matches_prefix(k, &prefix) {
            items.push(Item { label: case(k), insert: case(k), detail: "palavra".into() });
        }
    }
    // uma tabela citada pelo nome também pode ser escrita em colunas qualificadas: sugere os nomes das tabelas
    for t in tables {
        if matches_prefix(&t.name, &prefix) && explicit {
            items.push(Item { label: t.label(), insert: ident(&t.name), detail: if t.is_view { "visão" } else { "tabela" }.into() });
        }
    }
    let _ = explicit;
    finish(items, prefix)
}

fn finish(mut items: Vec<Item>, prefix: String) -> Option<Suggestion> {
    // exatamente o que já está escrito, e mais nada: não há o que completar
    items.dedup_by(|a, b| a.insert == b.insert);
    if items.is_empty() || (items.len() == 1 && items[0].insert.eq_ignore_ascii_case(&prefix)) {
        return None;
    }
    Some(Suggestion { prefix_len: prefix.chars().count(), items })
}

/// O maior começo comum (sem diferenciar maiúsculas) das sugestões: é o que o Tab completa de uma vez, como no shell.
pub fn common_prefix(items: &[Item]) -> String {
    let Some(first) = items.first() else { return String::new() };
    let mut len = first.insert.chars().count();
    for it in &items[1..] {
        len = len.min(first.insert.chars().zip(it.insert.chars()).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count());
    }
    first.insert.chars().take(len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ox_db::Column;

    fn col(n: &str, t: &str) -> Column {
        Column { name: n.into(), decl_type: t.into(), not_null: false, pk: 0, default: None }
    }

    fn table(schema: Option<&str>, name: &str, view: bool, cols: &[(&str, &str)]) -> Table {
        Table { schema: schema.map(String::from), name: name.into(), is_view: view, columns: cols.iter().map(|(n, t)| col(n, t)).collect(), indexes: vec![], without_rowid: false, foreign_keys: vec![] }
    }

    fn db() -> Vec<Table> {
        vec![
            table(Some("public"), "pessoas", false, &[("id", "integer"), ("nome", "text"), ("idade", "integer")]),
            table(Some("public"), "pedidos", false, &[("id", "integer"), ("pessoa_id", "integer"), ("total", "numeric")]),
            table(Some("public"), "adultos", true, &[("nome", "text")]),
            table(Some("extra"), "log", false, &[("id", "bigint"), ("msg", "text")]),
            table(Some("public"), "Clientes", false, &[("Código", "int")]),
        ]
    }

    fn labels(s: &Option<Suggestion>) -> Vec<String> {
        s.as_ref().map(|s| s.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default()
    }

    fn at_end(text: &str, explicit: bool) -> Option<Suggestion> {
        suggest(text, text.chars().count(), &db(), explicit)
    }

    #[test]
    fn after_from_it_offers_tables_views_and_schemas() {
        let s = at_end("select * from ", true);
        let l = labels(&s);
        for want in ["pessoas", "pedidos", "adultos", "extra.log", "Clientes", "extra."] {
            assert!(l.contains(&want.to_string()), "{want} em {l:?}");
        }
        assert_eq!(labels(&at_end("select * from pe", true)), ["pessoas", "pedidos"]);
        assert_eq!(common_prefix(&at_end("select * from pe", true).unwrap().items), "pe");
        assert_eq!(common_prefix(&at_end("select * from pess", true).map_or(vec![], |s| s.items)), "pessoas");
    }

    #[test]
    fn joins_commas_and_other_table_words_also_expect_tables() {
        assert!(labels(&at_end("select * from pessoas p join ped", false)).contains(&"pedidos".to_string()));
        assert!(labels(&at_end("select * from pessoas, ped", false)).contains(&"pedidos".to_string()));
        assert!(labels(&at_end("insert into pes", false)).contains(&"pessoas".to_string()));
        assert!(labels(&at_end("update pes", false)).contains(&"pessoas".to_string()));
        // depois de `where` a vírgula não volta ao lugar de tabela
        assert!(!labels(&at_end("select * from pessoas where a in (1, ped", false)).contains(&"pedidos".to_string()));
    }

    #[test]
    fn a_dot_after_a_table_or_alias_lists_its_columns() {
        let t = "select p. from pessoas p";
        let s = suggest(t, 9, &db(), true);
        assert_eq!(labels(&s), ["id", "nome", "idade"]);
        assert_eq!(s.unwrap().items[1].detail, "text");
        let s = suggest("select pessoas.ni from pessoas", 17, &db(), false);
        assert_eq!(labels(&s), Vec::<String>::new());
        let s = suggest("select pessoas.nome from pessoas", 19, &db(), false);
        assert!(s.is_none(), "o que já está completo não sugere");
        let s = suggest("select pessoas.i from pessoas", 16, &db(), false);
        assert_eq!(labels(&s), ["id", "idade"]);
        assert_eq!(labels(&at_end("select * from extra.", true)), ["log"]);
    }

    #[test]
    fn columns_come_from_every_table_in_the_statement_even_after_the_cursor() {
        let t = "select no from pessoas p join pedidos o on o.pessoa_id = p.id";
        let s = suggest(t, 9, &db(), false);
        assert_eq!(labels(&s)[0], "nome");
        let s = suggest("select pe from pessoas", 9, &db(), false);
        assert!(!labels(&s).contains(&"pedidos".to_string()), "fora do lugar de tabela só entram colunas e palavras");
        // dois comandos: só vale o do cursor
        let t = "select * from pedidos; select no from pessoas";
        let s = suggest(t, 32, &db(), false);
        assert_eq!(labels(&s)[0], "nome");
    }

    #[test]
    fn keywords_and_functions_follow_the_case_being_typed() {
        let l = labels(&at_end("sel", false));
        assert_eq!(l, ["select"]);
        let l = labels(&at_end("SEL", false));
        assert_eq!(l, ["SELECT"]);
        assert!(labels(&at_end("select co", false)).contains(&"count()".to_string()));
        assert!(labels(&at_end("select * from pessoas wh", false)).contains(&"where".to_string()));
    }

    #[test]
    fn names_that_need_quotes_are_quoted_and_nothing_pops_up_in_strings_or_empty_words() {
        let s = at_end("select * from Cl", true).unwrap();
        assert_eq!(s.items[0].insert, "\"Clientes\"");
        assert_eq!(suggest("select \"Código\" from \"Clientes\" c where c.", 43, &db(), true).map(|s| s.items[0].insert.clone()), Some("\"Código\"".to_string()));
        assert!(at_end("select 'pe", true).is_none());
        assert!(at_end("select ", true).is_none(), "palavra vazia fora de lugar de tabela: Tab recua o texto");
        assert!(at_end("", true).is_none());
    }

    #[test]
    fn the_common_prefix_ignores_case_and_stops_at_the_first_difference() {
        let it = |s: &str| Item { label: s.into(), insert: s.into(), detail: String::new() };
        assert_eq!(common_prefix(&[it("pessoas"), it("pessoa_id"), it("pessimo")]), "pess");
        assert_eq!(common_prefix(&[it("id")]), "id");
        assert_eq!(common_prefix(&[]), "");
    }
}
