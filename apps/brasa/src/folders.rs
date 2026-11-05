//! Pastas da aba Sessões guardadas entre execuções, num arquivo de texto simples em
//! `~/.config/brasa/session-folders.txt`: uma pasta por linha, `+nome` (aberta) ou `-nome` (fechada).
//! Os terminais não são guardados (morrem com o app); só as pastas voltam.

use std::path::PathBuf;

pub fn path() -> PathBuf {
    crate::config_dir().join("session-folders.txt")
}

/// (nome, aberta) de cada pasta, na ordem do arquivo; linhas vazias ou estranhas são ignoradas.
pub fn parse(text: &str) -> Vec<(String, bool)> {
    text.lines()
        .filter_map(|l| {
            let (open, name) = match l.chars().next()? {
                '+' => (true, &l[1..]),
                '-' => (false, &l[1..]),
                _ => return None,
            };
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), open))
        })
        .collect()
}

pub fn format<'a>(folders: impl IntoIterator<Item = (&'a str, bool)>) -> String {
    folders.into_iter().map(|(name, open)| format!("{}{}\n", if open { '+' } else { '-' }, name.replace('\n', " "))).collect()
}

pub fn load() -> Vec<(String, bool)> {
    std::fs::read_to_string(path()).map(|t| parse(&t)).unwrap_or_default()
}

pub fn save<'a>(folders: impl IntoIterator<Item = (&'a str, bool)>) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, format(folders));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_order_and_open_state() {
        let text = format([("web", true), ("servidores de teste", false)]);
        assert_eq!(text, "+web\n-servidores de teste\n");
        assert_eq!(parse(&text), [("web".to_string(), true), ("servidores de teste".to_string(), false)]);
        // linhas sem marcador ou sem nome são ignoradas
        assert_eq!(parse("lixo\n+\n\n-ok\n"), [("ok".to_string(), false)]);
    }
}
