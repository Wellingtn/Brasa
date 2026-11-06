//! Notas executáveis: variáveis `{{nome}}` nos blocos de shell e a saída da última execução guardada na
//! própria nota, num bloco ` ```saida ` logo depois do bloco que rodou:
//!
//! ````text
//! ```bash
//! ls {{pasta}}
//! ```
//!
//! ```saida 2026-11-06 12:03 exit=0
//! a.txt
//! b.txt
//! ```
//! ````
//!
//! Tudo aqui trabalha sobre o texto Markdown, sem depender da interface.

use std::collections::HashMap;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::is_runnable;

/// Linguagem do bloco que guarda a saída de uma execução.
pub const OUTPUT_LANG: &str = "saida";
/// Quantas linhas e caracteres da saída ficam guardados na nota.
const MAX_LINES: usize = 200;
const MAX_CHARS: usize = 20_000;

/// Um bloco de código do texto, com o lugar dele no arquivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub lang: String,
    /// o resto da linha de abertura depois da linguagem (hora e código de saída, nos blocos de saída)
    pub info: String,
    pub code: String,
    /// bytes de `src` que o bloco ocupa, das cercas de abertura às de fechamento
    pub start: usize,
    pub end: usize,
    /// linha (de 0) onde o bloco abre
    pub line: usize,
    pub runnable: bool,
}

pub fn code_spans(src: &str) -> Vec<Span> {
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_GFM;
    let mut out = Vec::new();
    let mut cur: Option<(usize, String, String)> = None;
    let mut code = String::new();
    for (ev, range) in Parser::new_ext(src, opts).into_offset_iter() {
        match ev {
            Event::Start(Tag::CodeBlock(kind)) => {
                let (lang, info) = match kind {
                    CodeBlockKind::Fenced(i) => {
                        let mut words = i.split_whitespace();
                        (words.next().unwrap_or("").to_lowercase(), words.collect::<Vec<_>>().join(" "))
                    }
                    CodeBlockKind::Indented => (String::new(), String::new()),
                };
                cur = Some((range.start, lang, info));
                code.clear();
            }
            Event::Text(t) if cur.is_some() => code.push_str(&t),
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, lang, info)) = cur.take() {
                    out.push(Span {
                        runnable: is_runnable(&lang),
                        code: code.trim_end_matches('\n').to_string(),
                        line: src[..start].matches('\n').count(),
                        lang,
                        info,
                        start,
                        end: range.end,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn runnable_spans(src: &str) -> Vec<Span> {
    code_spans(src).into_iter().filter(|s| s.runnable).collect()
}

/// Quantos blocos executáveis a nota tem.
pub fn runnable_count(src: &str) -> usize {
    runnable_spans(src).len()
}

/// Posição (entre os blocos executáveis) do bloco que abre na linha `line` (de 0).
pub fn runnable_ordinal_at_line(src: &str, line: usize) -> Option<usize> {
    runnable_spans(src).iter().position(|s| s.line == line)
}

/// Código do bloco executável de posição `ordinal`.
pub fn runnable_command(src: &str, ordinal: usize) -> Option<String> {
    runnable_spans(src).into_iter().nth(ordinal).map(|s| s.code)
}

/// Nomes das variáveis `{{nome}}` do texto, na ordem em que aparecem, sem repetir. O nome é letras, números
/// e `_`, começando por letra ou `_`; espaços dentro das chaves são ignorados (`{{ nome }}`).
pub fn variables(code: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (name, _, _) in placeholders(code) {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// (nome, início, fim) de cada `{{nome}}` do texto.
fn placeholders(code: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(open) = code[i..].find("{{").map(|p| p + i) {
        let Some(close) = code[open + 2..].find("}}").map(|p| p + open + 2) else { break };
        let name = code[open + 2..close].trim();
        let valid = name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if valid {
            out.push((name.to_string(), open, close + 2));
            i = close + 2;
        } else {
            i = open + 2;
        }
    }
    out
}

/// Troca cada `{{nome}}` pelo valor; variável sem valor fica como está.
pub fn substitute(code: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (name, start, end) in placeholders(code) {
        out.push_str(&code[last..start]);
        match values.get(&name) {
            Some(v) => out.push_str(v),
            None => out.push_str(&code[start..end]),
        }
        last = end;
    }
    out.push_str(&code[last..]);
    out
}

/// Resumo de uma saída guardada: texto do bloco, hora e código de saída.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub text: String,
    pub stamp: String,
    pub status: Option<i32>,
}

/// Lê hora e código de saída da linha de abertura de um bloco de saída ("2026-11-06 12:03 exit=0").
pub fn output_meta(info: &str) -> (String, Option<i32>) {
    let mut status = None;
    let mut stamp = Vec::new();
    for w in info.split_whitespace() {
        match w.strip_prefix("exit=") {
            Some(n) => status = n.parse().ok(),
            None => stamp.push(w),
        }
    }
    (stamp.join(" "), status)
}

/// Saída guardada logo depois do bloco executável `ordinal`, se houver.
pub fn stored_output(src: &str, ordinal: usize) -> Option<Output> {
    let spans = code_spans(src);
    let idx = spans.iter().enumerate().filter(|(_, s)| s.runnable).nth(ordinal)?.0;
    let next = spans.get(idx + 1)?;
    if next.lang != OUTPUT_LANG || !src[spans[idx].end..next.start].trim().is_empty() {
        return None;
    }
    let (stamp, status) = output_meta(&next.info);
    Some(Output { text: next.code.clone(), stamp, status })
}

/// Guarda no texto a saída da última execução do bloco `ordinal`: troca o bloco de saída que já existe logo
/// depois dele ou cria um. Devolve o texto novo, ou `None` se o bloco não existe. Só ficam as últimas
/// linhas se a saída for grande.
pub fn with_output(src: &str, ordinal: usize, output: &str, status: Option<i32>, stamp: &str) -> Option<String> {
    let spans = code_spans(src);
    let idx = spans.iter().enumerate().filter(|(_, s)| s.runnable).nth(ordinal)?.0;
    let block = &spans[idx];
    let fence_len = longest_backtick_run(output).max(2) + 1;
    let fence = "`".repeat(fence_len);
    let exit = status.map(|s| format!(" exit={s}")).unwrap_or_default();
    let body = clip(output);
    let new_block = format!("{fence}{OUTPUT_LANG} {stamp}{exit}\n{body}\n{fence}\n");

    let replace = spans.get(idx + 1).filter(|n| n.lang == OUTPUT_LANG && src[block.end..n.start].trim().is_empty());
    let mut out = String::with_capacity(src.len() + new_block.len() + 2);
    match replace {
        Some(n) => {
            out.push_str(&src[..n.start]);
            out.push_str(&new_block);
            out.push_str(src[n.end..].trim_start_matches('\n').chars().next().map_or("", |_| "\n"));
            out.push_str(src[n.end..].trim_start_matches('\n'));
        }
        None => {
            out.push_str(&src[..block.end]);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
            out.push_str(&new_block);
            let rest = &src[block.end..];
            if !rest.trim().is_empty() {
                out.push('\n');
            }
            out.push_str(rest.trim_start_matches('\n'));
        }
    }
    Some(out)
}

fn longest_backtick_run(s: &str) -> usize {
    let (mut best, mut cur) = (0, 0);
    for c in s.chars() {
        if c == '`' {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Limita a saída às últimas `MAX_LINES` linhas e `MAX_CHARS` caracteres, avisando do que ficou de fora.
fn clip(output: &str) -> String {
    let text = output.trim_end_matches(['\n', '\r']);
    let lines: Vec<&str> = text.lines().collect();
    let mut kept: &[&str] = &lines;
    let mut dropped = 0;
    if lines.len() > MAX_LINES {
        dropped = lines.len() - MAX_LINES;
        kept = &lines[dropped..];
    }
    let mut body = kept.join("\n");
    if body.chars().count() > MAX_CHARS {
        let skip = body.chars().count() - MAX_CHARS;
        body = body.chars().skip(skip).collect();
        dropped = dropped.max(1);
    }
    if dropped > 0 {
        format!("[… saída cortada: só o final foi guardado]\n{body}")
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Deploy\n\n```bash\nls {{pasta}}\n```\n\nTexto.\n\n```python\nprint(1)\n```\n\n```sh\necho {{pasta}} {{ nome }}\n```\n";

    #[test]
    fn spans_know_their_place_and_language() {
        let s = code_spans(NOTE);
        assert_eq!(s.iter().map(|x| x.lang.as_str()).collect::<Vec<_>>(), ["bash", "python", "sh"]);
        assert_eq!(s.iter().map(|x| x.runnable).collect::<Vec<_>>(), [true, false, true]);
        assert_eq!(s[0].line, 2);
        assert_eq!(s[0].code, "ls {{pasta}}");
        assert_eq!(&NOTE[s[0].start..s[0].end], "```bash\nls {{pasta}}\n```");
        assert_eq!(runnable_count(NOTE), 2);
        assert_eq!(runnable_ordinal_at_line(NOTE, 2), Some(0));
        assert_eq!(runnable_ordinal_at_line(NOTE, 12), Some(1));
        assert_eq!(runnable_ordinal_at_line(NOTE, 8), None); // python não executa
        assert_eq!(runnable_command(NOTE, 1).as_deref(), Some("echo {{pasta}} {{ nome }}"));
    }

    #[test]
    fn variables_are_found_once_in_order() {
        assert_eq!(variables("ls {{pasta}} && echo {{ nome }} {{pasta}}"), ["pasta", "nome"]);
        assert!(variables("echo ${HOME} {{}} {{1x}} {{a b}} {{ aberta").is_empty());
        assert_eq!(variables("{{_x1}}"), ["_x1"]);
    }

    #[test]
    fn variables_are_replaced() {
        let mut v = HashMap::new();
        v.insert("pasta".to_string(), "/tmp/a b".to_string());
        assert_eq!(substitute("ls {{pasta}} {{ nome }} {{pasta}}", &v), "ls /tmp/a b {{ nome }} /tmp/a b");
        assert_eq!(substitute("sem nada", &v), "sem nada");
        assert_eq!(substitute("{{pasta}}", &HashMap::new()), "{{pasta}}");
    }

    #[test]
    fn output_is_inserted_after_the_block_that_ran() {
        let out = with_output(NOTE, 0, "a.txt\nb.txt\n", Some(0), "2026-11-06 12:03").unwrap();
        assert!(out.contains("```bash\nls {{pasta}}\n```\n\n```saida 2026-11-06 12:03 exit=0\na.txt\nb.txt\n```\n\nTexto."), "{out}");
        let o = stored_output(&out, 0).unwrap();
        assert_eq!((o.text.as_str(), o.stamp.as_str(), o.status), ("a.txt\nb.txt", "2026-11-06 12:03", Some(0)));
        // o segundo bloco executável continua sendo o segundo, e sem saída
        assert_eq!(runnable_count(&out), 2);
        assert_eq!(runnable_command(&out, 1).as_deref(), Some("echo {{pasta}} {{ nome }}"));
        assert_eq!(stored_output(&out, 1), None);
        assert_eq!(stored_output(NOTE, 0), None);
    }

    #[test]
    fn output_is_replaced_not_duplicated() {
        let once = with_output(NOTE, 0, "velho", Some(1), "2026-11-05 09:00").unwrap();
        let twice = with_output(&once, 0, "novo", Some(0), "2026-11-06 12:03").unwrap();
        assert_eq!(code_spans(&twice).iter().filter(|s| s.lang == OUTPUT_LANG).count(), 1);
        let o = stored_output(&twice, 0).unwrap();
        assert_eq!((o.text.as_str(), o.status), ("novo", Some(0)));
        assert!(!twice.contains("velho"));
        // o resto da nota ficou como estava
        assert!(twice.starts_with("# Deploy\n"));
        assert!(twice.contains("Texto.\n\n```python"));
        // só os dois blocos de código "comuns" mais a saída
        assert_eq!(code_spans(&twice).len(), 4);
    }

    #[test]
    fn output_at_the_end_of_the_file_and_without_trailing_newline() {
        let src = "```bash\necho oi\n```";
        let out = with_output(src, 0, "oi", Some(0), "t").unwrap();
        assert_eq!(out, "```bash\necho oi\n```\n\n```saida t exit=0\noi\n```\n");
        assert_eq!(with_output(&out, 0, "oi2", None, "t2").unwrap(), "```bash\necho oi\n```\n\n```saida t2\noi2\n```\n");
        assert_eq!(with_output(src, 5, "x", None, "t"), None);
    }

    #[test]
    fn output_with_fences_and_big_output_are_safe() {
        let out = with_output(NOTE, 0, "antes\n```\ndepois", Some(0), "t").unwrap();
        // a cerca da saída é maior que a que aparece dentro dela
        assert!(out.contains("````saida t exit=0\nantes\n```\ndepois\n````\n"), "{out}");
        assert_eq!(stored_output(&out, 0).unwrap().text, "antes\n```\ndepois");
        let big: String = (0..500).map(|i| format!("linha {i}\n")).collect();
        let out = with_output(NOTE, 0, &big, Some(0), "t").unwrap();
        let text = stored_output(&out, 0).unwrap().text;
        assert!(text.starts_with("[… saída cortada"));
        assert!(text.ends_with("linha 499"));
        assert_eq!(text.lines().count(), 201);
    }

    #[test]
    fn output_meta_is_read() {
        assert_eq!(output_meta("2026-11-06 12:03 exit=2"), ("2026-11-06 12:03".to_string(), Some(2)));
        assert_eq!(output_meta("exit=0"), (String::new(), Some(0)));
        assert_eq!(output_meta(""), (String::new(), None));
    }
}
