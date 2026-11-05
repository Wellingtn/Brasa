//! Realce de sintaxe leve, linha a linha, para os cartões de código das notas e os arquivos abertos
//! no editor. Reconhece comentários, strings, números, palavras-chave, funções, tipos e, no shell,
//! variáveis e opções. Não guarda estado entre linhas: strings/comentários de várias linhas pintam
//! só a primeira.

/// Cores do realce (tema grafite quente).
pub const PLAIN: u32 = 0xe4e0de;
pub const KEYWORD: u32 = 0xc3a6ff;
pub const STRING: u32 = 0xa8c99a;
pub const COMMENT: u32 = 0x7d7579;
pub const NUMBER: u32 = 0xe8a87c;
pub const FUNCTION: u32 = 0x82b8e8;
pub const TYPE: u32 = 0xe6c07b;
pub const PUNCT: u32 = 0xa9a2a5;
pub const VARIABLE: u32 = 0xe0866e;
pub const FLAG: u32 = 0x9ccfd8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Plain,
    Shell,
    Python,
    Rust,
    Js,
    Json,
}

impl Lang {
    /// Linguagem a partir do texto depois de ``` (ou da extensão de um arquivo).
    pub fn from_name(name: &str) -> Lang {
        match name.trim().to_lowercase().as_str() {
            "bash" | "sh" | "shell" | "zsh" | "fish" | "console" => Lang::Shell,
            "py" | "python" | "python3" => Lang::Python,
            "rs" | "rust" => Lang::Rust,
            "js" | "javascript" | "jsx" | "mjs" | "cjs" | "ts" | "typescript" | "tsx" => Lang::Js,
            "json" | "jsonc" => Lang::Json,
            _ => Lang::Plain,
        }
    }

    fn line_comment(self) -> Option<&'static str> {
        match self {
            Lang::Shell | Lang::Python => Some("#"),
            Lang::Rust | Lang::Js => Some("//"),
            Lang::Json | Lang::Plain => None,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Lang::Shell => &[
                "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return", "export",
                "local", "readonly", "source", "alias", "unset", "set", "exit", "break", "continue",
            ],
            Lang::Python => &[
                "def", "class", "return", "if", "elif", "else", "for", "while", "in", "not", "and", "or", "is", "import", "from", "as", "with", "try",
                "except", "finally", "raise", "pass", "break", "continue", "lambda", "yield", "global", "nonlocal", "assert", "del", "async", "await",
                "True", "False", "None", "self",
            ],
            Lang::Rust => &[
                "fn", "let", "mut", "const", "static", "struct", "enum", "impl", "trait", "pub", "use", "mod", "crate", "super", "self", "Self", "match",
                "if", "else", "for", "while", "loop", "in", "return", "break", "continue", "as", "where", "move", "ref", "async", "await", "dyn", "unsafe",
                "type", "true", "false",
            ],
            Lang::Js => &[
                "function", "const", "let", "var", "return", "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "new",
                "class", "extends", "import", "export", "from", "as", "async", "await", "try", "catch", "finally", "throw", "typeof", "instanceof", "in",
                "of", "this", "null", "undefined", "true", "false", "interface", "type", "enum", "implements",
            ],
            Lang::Json => &["true", "false", "null"],
            Lang::Plain => &[],
        }
    }

    fn builtins(self) -> &'static [&'static str] {
        match self {
            Lang::Python => &["print", "len", "range", "str", "int", "float", "list", "dict", "set", "tuple", "open", "input", "type", "bool"],
            Lang::Rust => &["Vec", "String", "Option", "Result", "Box", "Some", "None", "Ok", "Err", "HashMap", "println", "format", "vec"],
            Lang::Js => &["console", "document", "window", "Math", "JSON", "Promise", "Array", "Object", "String", "Number"],
            Lang::Shell => &["echo", "cd", "ls", "cat", "grep", "sudo", "printf", "read", "test"],
            _ => &[],
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Cor de cada caractere da linha (mesmo comprimento que `line.chars()`).
pub fn colors(lang: Lang, line: &str) -> Vec<u32> {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    if lang == Lang::Plain {
        return vec![PLAIN; n];
    }
    let mut out = vec![PLAIN; n];
    let comment: Option<Vec<char>> = lang.line_comment().map(|c| c.chars().collect());
    let (kw, bi) = (lang.keywords(), lang.builtins());
    // primeira palavra de cada comando do shell (início da linha, depois de | ; && ||)
    let mut shell_cmd = true;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        // comentário até o fim da linha
        if let Some(cm) = &comment {
            let starts = chars[i..].starts_with(cm);
            let at_word = lang != Lang::Shell || i == 0 || chars[i - 1].is_whitespace();
            if starts && at_word {
                out[i..].fill(COMMENT);
                break;
            }
        }
        // strings
        if c == '"' || c == '\'' || (c == '`' && lang == Lang::Js) {
            let start = i;
            i += 1;
            while i < n && chars[i] != c {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            let end = (i + 1).min(n);
            // em JSON, uma string seguida de ":" é uma chave
            let key = lang == Lang::Json && chars[end..].iter().find(|ch| !ch.is_whitespace()) == Some(&':');
            out[start..end].fill(if key { FUNCTION } else { STRING });
            i = end;
            shell_cmd = false;
            continue;
        }
        // variáveis do shell: $X, ${X}, $1
        if lang == Lang::Shell && c == '$' {
            let start = i;
            i += 1;
            if i < n && chars[i] == '{' {
                while i < n && chars[i] != '}' {
                    i += 1;
                }
                i = (i + 1).min(n);
            } else {
                while i < n && (is_ident(chars[i]) || (i == start + 1 && "?#@*!$".contains(chars[i]))) {
                    i += 1;
                }
            }
            out[start..i].fill(VARIABLE);
            shell_cmd = false;
            continue;
        }
        // números
        if c.is_ascii_digit() && (i == 0 || !is_ident(chars[i - 1])) {
            let start = i;
            while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                i += 1;
            }
            out[start..i].fill(NUMBER);
            shell_cmd = false;
            continue;
        }
        // opções do shell: -x, --flag
        if lang == Lang::Shell && c == '-' && (i == 0 || chars[i - 1].is_whitespace()) && i + 1 < n && (chars[i + 1] == '-' || chars[i + 1].is_alphanumeric()) {
            let start = i;
            while i < n && !chars[i].is_whitespace() {
                i += 1;
            }
            out[start..i].fill(FLAG);
            continue;
        }
        // identificadores
        if is_ident_start(c) {
            let start = i;
            while i < n && (is_ident(chars[i]) || (lang == Lang::Shell && (chars[i] == '-' || chars[i] == '.' || chars[i] == '/'))) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let next = chars[i..].iter().find(|ch| !ch.is_whitespace()).copied();
            let color = if kw.contains(&word.as_str()) {
                KEYWORD
            } else if lang == Lang::Shell && shell_cmd {
                FUNCTION
            } else if bi.contains(&word.as_str()) {
                TYPE
            } else if next == Some('(') || (lang == Lang::Rust && next == Some('!')) {
                FUNCTION
            } else if lang == Lang::Rust && word.starts_with(|ch: char| ch.is_uppercase()) {
                TYPE
            } else {
                PLAIN
            };
            out[start..i].fill(color);
            // depois de "if", "then", "do"... vem um comando
            shell_cmd = lang == Lang::Shell && kw.contains(&word.as_str());
            continue;
        }
        if !c.is_whitespace() {
            out[i] = PUNCT;
            if lang == Lang::Shell {
                shell_cmd = matches!(c, '|' | ';' | '&' | '(' | '`');
            }
        }
        i += 1;
    }
    out
}

/// Junta caracteres vizinhos de mesma cor no trecho `[start, end)`: (texto, cor).
pub fn spans(chars: &[char], colors: &[u32], start: usize, end: usize) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for k in start..end.min(chars.len()) {
        let col = colors.get(k).copied().unwrap_or(PLAIN);
        match out.last_mut() {
            Some((t, c)) if *c == col => t.push(chars[k]),
            _ => out.push((chars[k].to_string(), col)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lang: Lang, line: &str, word: &str) -> u32 {
        let i = line.find(word).expect("palavra na linha");
        colors(lang, line)[line[..i].chars().count()]
    }

    #[test]
    fn python_keywords_strings_calls_and_comments() {
        let l = r#"def ola(n): print("Hello World")  # saudação"#;
        assert_eq!(at(Lang::Python, l, "def"), KEYWORD);
        assert_eq!(at(Lang::Python, l, "ola"), FUNCTION);
        assert_eq!(at(Lang::Python, l, "print"), TYPE);
        assert_eq!(at(Lang::Python, l, "\"Hello"), STRING);
        assert_eq!(at(Lang::Python, l, "# sauda"), COMMENT);
        assert_eq!(colors(Lang::Python, l).len(), l.chars().count());
    }

    #[test]
    fn shell_commands_flags_variables() {
        let l = "ls -la $HOME | grep -v x # fim";
        assert_eq!(at(Lang::Shell, l, "ls"), FUNCTION);
        assert_eq!(at(Lang::Shell, l, "-la"), FLAG);
        assert_eq!(at(Lang::Shell, l, "$HOME"), VARIABLE);
        assert_eq!(at(Lang::Shell, l, "grep"), FUNCTION);
        assert_eq!(at(Lang::Shell, l, "# fim"), COMMENT);
        // "#" colado numa palavra não é comentário no shell
        assert_ne!(at(Lang::Shell, "echo a#b", "#"), COMMENT);
    }

    #[test]
    fn rust_json_and_plain() {
        let l = "let v: Vec<u8> = vec![1, 2]; // ok";
        assert_eq!(at(Lang::Rust, l, "let"), KEYWORD);
        assert_eq!(at(Lang::Rust, l, "Vec"), TYPE);
        assert_eq!(at(Lang::Rust, l, "1"), NUMBER);
        assert_eq!(at(Lang::Rust, l, "// ok"), COMMENT);
        let j = r#"{"nome": "oxi", "n": 3, "ok": true}"#;
        assert_eq!(at(Lang::Json, j, "\"nome\""), FUNCTION);
        assert_eq!(at(Lang::Json, j, "\"oxi\""), STRING);
        assert_eq!(at(Lang::Json, j, "true"), KEYWORD);
        assert!(colors(Lang::Plain, "print(1)").iter().all(|c| *c == PLAIN));
        assert_eq!(Lang::from_name("bash"), Lang::Shell);
        assert_eq!(Lang::from_name(""), Lang::Plain);
    }

    #[test]
    fn spans_merge_same_colors() {
        let line = "echo oi";
        let chars: Vec<char> = line.chars().collect();
        let c = colors(Lang::Shell, line);
        let s = spans(&chars, &c, 0, chars.len());
        assert_eq!(s.iter().map(|(t, _)| t.as_str()).collect::<String>(), line);
        assert_eq!(s[0], ("echo".to_string(), FUNCTION));
    }
}
