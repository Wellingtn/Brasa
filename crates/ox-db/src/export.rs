//! Exportar resultados: CSV, TSV (para colar numa planilha) e JSON, de um resultado na memória ou de uma tabela inteira.
//!
//! Regras: CSV segue a RFC 4180 (campo com vírgula, aspas, quebra de linha ou espaço nas pontas vai entre aspas, e a
//! aspa dentro dele é dobrada); nulo vira campo vazio no CSV e `null` no JSON; valor binário vira só o tamanho, porque
//! o resultado guarda apenas o começo dele. Escrever em arquivo nunca sobrescreve um arquivo que já existe.

use std::io::Write;
use std::path::Path;

use crate::{Db, Error, Table, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
        }
    }
}

/// Quantas linhas se lêem do banco por vez ao exportar uma tabela inteira.
const CHUNK: usize = 2000;

fn csv_field(v: &Value) -> String {
    let text = match v {
        Value::Null => return String::new(),
        Value::Blob { len, .. } => format!("[{len} bytes]"),
        other => other.display(),
    };
    let needs = text.contains([',', '"', '\n', '\r']) || text.starts_with(' ') || text.ends_with(' ');
    if needs {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn csv_line(fields: impl Iterator<Item = String>) -> String {
    let mut line = fields.collect::<Vec<_>>().join(",");
    line.push_str("\r\n");
    line
}

/// Resultado em CSV, com a linha de cabeçalho.
pub fn to_csv(columns: &[String], rows: &[Vec<Value>]) -> String {
    let mut out = csv_line(columns.iter().map(|c| csv_field(&Value::Text(c.clone()))));
    for r in rows {
        out.push_str(&csv_line(r.iter().map(csv_field)));
    }
    out
}

/// Resultado separado por tabulação, para colar numa planilha. Tabulação e quebra de linha dentro de um valor viram
/// espaço (senão quebrariam as colunas e as linhas).
pub fn to_tsv(columns: &[String], rows: &[Vec<Value>]) -> String {
    let clean = |v: &Value| match v {
        Value::Null => String::new(),
        Value::Blob { len, .. } => format!("[{len} bytes]"),
        other => other.display().replace(['\t', '\n', '\r'], " "),
    };
    let mut out = columns.iter().map(|c| c.replace(['\t', '\n', '\r'], " ")).collect::<Vec<_>>().join("\t");
    out.push('\n');
    for r in rows {
        out.push_str(&r.iter().map(clean).collect::<Vec<_>>().join("\t"));
        out.push('\n');
    }
    out
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_value(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Integer(i) => i.to_string(),
        Value::Real(r) if r.is_finite() => r.to_string(),
        Value::Real(_) => "null".into(),
        Value::Text(t) => json_string(t),
        Value::Blob { len, .. } => json_string(&format!("[{len} bytes]")),
    }
}

fn json_object(columns: &[String], row: &[Value]) -> String {
    let fields: Vec<String> = columns.iter().zip(row).map(|(c, v)| format!("{}: {}", json_string(c), json_value(v))).collect();
    format!("{{{}}}", fields.join(", "))
}

/// Resultado como lista JSON de objetos (um por linha), uma linha de texto por objeto.
pub fn to_json(columns: &[String], rows: &[Vec<Value>]) -> String {
    let items: Vec<String> = rows.iter().map(|r| format!("  {}", json_object(columns, r))).collect();
    if items.is_empty() {
        "[]\n".into()
    } else {
        format!("[\n{}\n]\n", items.join(",\n"))
    }
}

fn io_err(e: std::io::Error, path: &Path) -> Error {
    match e.kind() {
        std::io::ErrorKind::AlreadyExists => Error::Sql(format!("o arquivo {} já existe: escolha outro nome", path.display())),
        _ => Error::Sql(format!("não consegui gravar {}: {e}", path.display())),
    }
}

/// Grava um resultado que já está na memória. Não sobrescreve arquivo existente.
pub fn write_result(path: &Path, format: Format, columns: &[String], rows: &[Vec<Value>]) -> Result<(), Error> {
    let text = match format {
        Format::Csv => to_csv(columns, rows),
        Format::Json => to_json(columns, rows),
    };
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| io_err(e, path))?;
    f.write_all(text.as_bytes()).map_err(|e| io_err(e, path))
}

/// Exporta a tabela inteira (na ordem de `sort`), lendo o banco aos poucos. Devolve quantas linhas gravou. Se algo
/// falhar no meio, o arquivo parcial é apagado.
pub fn export_table(db: &mut Db, t: &Table, sort: Option<(&str, bool)>, format: Format, path: &Path) -> Result<u64, Error> {
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| io_err(e, path))?;
    let mut w = std::io::BufWriter::new(file);
    let result = (|| -> Result<u64, Error> {
        let columns: Vec<String> = t.columns.iter().map(|c| c.name.clone()).collect();
        let (mut offset, mut total) = (0usize, 0u64);
        match format {
            Format::Csv => w.write_all(csv_line(columns.iter().map(|c| csv_field(&Value::Text(c.clone())))).as_bytes()),
            Format::Json => w.write_all(b"["),
        }
        .map_err(|e| io_err(e, path))?;
        loop {
            let page = db.page(t, sort, offset, CHUNK)?;
            for r in &page.rows {
                match format {
                    Format::Csv => w.write_all(csv_line(r.iter().map(csv_field)).as_bytes()),
                    Format::Json => w.write_all(format!("{}\n  {}", if total == 0 { "" } else { "," }, json_object(&columns, r)).as_bytes()),
                }
                .map_err(|e| io_err(e, path))?;
                total += 1;
            }
            if !page.truncated {
                break;
            }
            offset += CHUNK;
        }
        if format == Format::Json {
            w.write_all(if total == 0 { b"]\n".as_slice() } else { b"\n]\n".as_slice() }).map_err(|e| io_err(e, path))?;
        }
        w.flush().map_err(|e| io_err(e, path))?;
        Ok(total)
    })();
    if result.is_err() {
        drop(w);
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Exporta o resultado inteiro de uma query (que devolve linhas), lido aos poucos do banco: milhões de linhas cabem
/// sem ocupar memória. Devolve quantas linhas gravou. Se algo falhar no meio, o arquivo parcial é apagado.
pub fn export_query(db: &mut Db, sql: &str, format: Format, path: &Path) -> Result<u64, Error> {
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| io_err(e, path))?;
    let mut w = std::io::BufWriter::new(file);
    let result = (|| -> Result<u64, Error> {
        let (mut started, mut written) = (false, 0u64);
        db.stream(sql, CHUNK, &mut |columns, rows| {
            if !started {
                started = true;
                match format {
                    Format::Csv => w.write_all(csv_line(columns.iter().map(|c| csv_field(&Value::Text(c.clone())))).as_bytes()),
                    Format::Json => w.write_all(b"["),
                }
                .map_err(|e| io_err(e, path))?;
            }
            for r in rows {
                match format {
                    Format::Csv => w.write_all(csv_line(r.iter().map(csv_field)).as_bytes()),
                    Format::Json => w.write_all(format!("{}\n  {}", if written == 0 { "" } else { "," }, json_object(columns, r)).as_bytes()),
                }
                .map_err(|e| io_err(e, path))?;
                written += 1;
            }
            Ok(())
        })?;
        if format == Format::Json {
            w.write_all(if written == 0 { b"]\n".as_slice() } else { b"\n]\n".as_slice() }).map_err(|e| io_err(e, path))?;
        }
        w.flush().map_err(|e| io_err(e, path))?;
        Ok(written)
    })();
    if result.is_err() {
        drop(w);
        let _ = std::fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Database, Options};

    fn hard() -> (Vec<String>, Vec<Vec<Value>>) {
        (
            vec!["id".into(), "texto".into(), "nota".into()],
            vec![
                vec![Value::Integer(1), Value::Text("simples".into()), Value::Real(1.5)],
                vec![Value::Integer(2), Value::Text("com, vírgula".into()), Value::Null],
                vec![Value::Integer(3), Value::Text("aspas \"duplas\"".into()), Value::Text("NULL".into())],
                vec![Value::Integer(4), Value::Text("linha1\nlinha2".into()), Value::Text("  espaços  ".into())],
                vec![Value::Integer(5), Value::Text("ação — 日本語 🔥".into()), Value::Blob { len: 9, preview: vec![1, 2, 3] }],
                vec![Value::Integer(6), Value::Text(String::new()), Value::Text("a\tb".into())],
            ],
        )
    }

    #[test]
    fn csv_quotes_only_what_needs_it_and_doubles_inner_quotes() {
        let (c, r) = hard();
        let csv = to_csv(&c, &r);
        assert_eq!(
            csv,
            "id,texto,nota\r\n1,simples,1.5\r\n2,\"com, vírgula\",\r\n3,\"aspas \"\"duplas\"\"\",NULL\r\n4,\"linha1\nlinha2\",\"  espaços  \"\r\n5,ação — 日本語 🔥,[9 bytes]\r\n6,,a\tb\r\n"
        );
        // cabeçalho com nome estranho também é protegido
        assert_eq!(to_csv(&["a,b".into()], &[]), "\"a,b\"\r\n");
    }

    #[test]
    fn tsv_never_breaks_columns_or_rows() {
        let (c, r) = hard();
        let tsv = to_tsv(&c, &r);
        // 1 cabeçalho + 6 linhas, e nenhuma linha com mais de 3 colunas
        assert_eq!(tsv.lines().count(), 7);
        assert!(tsv.lines().all(|l| l.split('\t').count() == 3), "{tsv:?}");
        assert!(tsv.contains("linha1 linha2") && tsv.contains("a b") && tsv.contains("1\tsimples\t1.5"));
    }

    #[test]
    fn json_has_real_nulls_numbers_and_escapes() {
        let (c, r) = hard();
        let j = to_json(&c, &r);
        assert!(j.starts_with("[\n") && j.ends_with("\n]\n"));
        assert!(j.contains("{\"id\": 1, \"texto\": \"simples\", \"nota\": 1.5}"));
        assert!(j.contains("\"nota\": null")); // nulo de verdade
        assert!(j.contains("\"nota\": \"NULL\"")); // o texto "NULL" continua texto
        assert!(j.contains("aspas \\\"duplas\\\"") && j.contains("linha1\\nlinha2") && j.contains("a\\tb"));
        assert!(j.contains("ação — 日本語 🔥") && j.contains("[9 bytes]"));
        assert_eq!(to_json(&c, &[]), "[]\n");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
        assert_eq!(json_value(&Value::Real(f64::NAN)), "null");
    }

    #[test]
    fn a_written_file_is_never_overwritten() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("saida.csv");
        let (c, r) = hard();
        write_result(&p, Format::Csv, &c, &r).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), to_csv(&c, &r));
        let again = write_result(&p, Format::Csv, &c, &r);
        assert!(matches!(again, Err(Error::Sql(m)) if m.contains("já existe")));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), to_csv(&c, &r)); // intacto
        // pasta que não existe: erro claro
        assert!(matches!(write_result(&d.path().join("nao/existe.csv"), Format::Json, &c, &r), Err(Error::Sql(m)) if m.contains("não consegui gravar")));
    }

    fn sample_db(rows: usize) -> (tempfile::TempDir, Db, Table) {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("e.db");
        let st = std::process::Command::new("sqlite3")
            .arg(&path)
            .arg(format!("CREATE TABLE t (id INTEGER PRIMARY KEY, nome TEXT, nota REAL); WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i < {rows}) INSERT INTO t (nome, nota) SELECT 'n, ' || i, i / 4.0 FROM n; INSERT INTO t (nome, nota) VALUES (NULL, NULL);"))
            .status()
            .unwrap();
        assert!(st.success());
        let mut db = Db::Sqlite(Database::open(&path, &Options::default()).unwrap());
        let t = db.schema().unwrap().into_iter().find(|t| t.name == "t").unwrap();
        (d, db, t)
    }

    #[test]
    fn a_whole_table_is_exported_in_chunks_with_every_row() {
        // mais linhas que um bloco (2000), para exercitar a leitura em pedaços
        let (d, mut db, t) = sample_db(4500);
        let csv = d.path().join("t.csv");
        assert_eq!(export_table(&mut db, &t, None, Format::Csv, &csv).unwrap(), 4501);
        let text = std::fs::read_to_string(&csv).unwrap();
        assert_eq!(text.lines().count(), 4502); // cabeçalho + linhas
        assert!(text.starts_with("id,nome,nota\r\n1,\"n, 1\",0.25\r\n"));
        assert!(text.ends_with("4501,,\r\n")); // nulos viram vazio
        let json = d.path().join("t.json");
        assert_eq!(export_table(&mut db, &t, Some(("id", false)), Format::Json, &json).unwrap(), 4501);
        let j = std::fs::read_to_string(&json).unwrap();
        assert!(j.starts_with("[\n  {\"id\": 4501, \"nome\": null, \"nota\": null},\n  {\"id\": 4500"), "{}", &j[..80]);
        assert!(j.ends_with("}\n]\n"));
        assert_eq!(j.matches("\"id\":").count(), 4501);
        // o sqlite3 relê o CSV exportado e conta as mesmas linhas, com os campos entre aspas intactos
        use std::io::Write as _;
        let db2 = d.path().join("rel.db");
        let mut child = std::process::Command::new("sqlite3").arg(&db2).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(format!(".mode csv\n.import {} x\nSELECT count(*) FROM x;\nSELECT nome FROM x WHERE id = '1';\n", csv.display()).as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("4501"), "{text}");
        assert!(text.contains("n, 1"), "{text}");
    }

    #[test]
    fn an_empty_table_and_an_existing_file() {
        let (d, mut db, t) = sample_db(0);
        // o WITH RECURSIVE sempre gera a linha base: 1 linha dele mais a de nulos
        let p = d.path().join("a.json");
        assert_eq!(export_table(&mut db, &t, None, Format::Json, &p).unwrap(), 2);
        assert!(matches!(export_table(&mut db, &t, None, Format::Json, &p), Err(Error::Sql(m)) if m.contains("já existe")));
        assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 4);
    }

    #[test]
    fn a_whole_query_result_is_exported_past_the_old_limit_without_holding_it_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.db");
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE n (i INTEGER, t TEXT); WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < 25000) INSERT INTO n SELECT i, 'linha ' || i FROM s;").unwrap();
        drop(c);
        let mut db = Db::Sqlite(Database::open(&p, &Options::default()).unwrap());
        let out = dir.path().join("q.csv");
        assert_eq!(export_query(&mut db, "SELECT i, t FROM n ORDER BY i;", Format::Csv, &out).unwrap(), 25000);
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text.lines().count(), 25001);
        assert!(text.starts_with("i,t\r\n1,linha 1\r\n") || text.starts_with("i,t\n1,linha 1\n"), "{:?}", &text[..30]);
        // sem linhas: só o cabeçalho; JSON vazio
        let out2 = dir.path().join("v.json");
        assert_eq!(export_query(&mut db, "SELECT i FROM n WHERE i < 0", Format::Json, &out2).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&out2).unwrap().trim(), "[]");
        // erro: nada fica
        let out3 = dir.path().join("e.csv");
        assert!(export_query(&mut db, "SELECT * FROM nao_existe", Format::Csv, &out3).is_err());
        assert!(!out3.exists());
    }

    #[test]
    fn a_failed_export_leaves_no_partial_file() {
        let (d, mut db, t) = sample_db(10);
        let p = d.path().join("parcial.csv");
        // ordenar por coluna que não existe falha na primeira leitura
        let r = export_table(&mut db, &t, Some(("fantasma", true)), Format::Csv, &p);
        assert!(r.is_err());
        assert!(!p.exists(), "o arquivo parcial deveria ter sido apagado");
    }
}
