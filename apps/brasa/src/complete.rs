//! Completar caminhos com Tab, como o Warp: em vez de o shell imprimir as opções no terminal, o app
//! abre um cartão com as pastas/arquivos que combinam com a palavra sob o cursor; Enter completa.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    pub is_dir: bool,
}

/// Opções para a palavra antes do cursor; `base` é o pedaço já digitado do nome (a ser trocado).
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub base: String,
    pub items: Vec<Item>,
}

/// Comandos que só fazem sentido com pastas.
const DIR_COMMANDS: &[&str] = &["cd", "pushd", "rmdir", "z"];
const MAX_ITEMS: usize = 300;

/// Calcula as opções para o texto antes do cursor. `None` quando não é caso de completar caminho
/// (ainda digitando o nome do comando) — aí o Tab segue para o shell.
pub fn complete(before: &str, cwd: &Path, home: &Path) -> Option<Completion> {
    let words: Vec<&str> = before.split_whitespace().collect();
    let ends_in_space = before.ends_with(char::is_whitespace);
    // só o nome do comando, sem espaço depois: não é um caminho
    if words.is_empty() || (words.len() == 1 && !ends_in_space) {
        return None;
    }
    let dirs_only = DIR_COMMANDS.contains(&words[0]);
    let word = if ends_in_space { "" } else { words[words.len() - 1] };
    let (dir_part, base) = match word.rfind('/') {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word),
    };
    let dir: PathBuf = if dir_part.is_empty() {
        cwd.to_path_buf()
    } else if let Some(rest) = dir_part.strip_prefix('~') {
        home.join(rest.trim_start_matches('/'))
    } else if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else {
        cwd.join(dir_part)
    };
    let base_lc = base.to_lowercase();
    let show_hidden = base.starts_with('.');
    let mut items: Vec<Item> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.path().is_dir(); // segue links para pastas
            let visible = show_hidden || !name.starts_with('.');
            (visible && (is_dir || !dirs_only) && name.to_lowercase().starts_with(&base_lc)).then_some(Item { name, is_dir })
        })
        .collect();
    items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    items.truncate(MAX_ITEMS);
    Some(Completion { base: base.to_string(), items })
}

/// Escapa caracteres especiais do shell num nome (espaço, aspas, parênteses...).
pub fn shell_escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_whitespace() || "\\'\"()[]{}$&;|<>*?!#`".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Bytes para o PTY que trocam `base` (já digitado) pelo nome escolhido: apaga com backspace e digita
/// o nome inteiro (assim a caixa das letras também fica certa). Pastas ganham "/", arquivos um espaço.
pub fn accept_bytes(base: &str, item: &Item) -> Vec<u8> {
    let mut out = vec![0x7f; base.chars().count()];
    out.extend(shell_escape(&item.name).bytes());
    out.push(if item.is_dir { b'/' } else { b' ' });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pasta de teste própria por teste (eles rodam em paralelo).
    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("oxiterm-complete-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["assets", "dependencies", "liquidGlass", "media", ".git", "Meu Projeto"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("README.md"), "").unwrap();
        root
    }

    #[test]
    fn cd_lists_only_visible_folders_sorted() {
        let root = tree("cd");
        let c = complete("cd ", &root, &root).unwrap();
        let names: Vec<_> = c.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["assets", "dependencies", "liquidGlass", "media", "Meu Projeto"]);
        assert_eq!(c.base, "");
        // prefixo, sem diferenciar maiúsculas
        let c = complete("cd m", &root, &root).unwrap();
        assert_eq!(c.items.len(), 2);
        assert_eq!(c.base, "m");
        // ponto mostra ocultos
        assert_eq!(complete("cd .g", &root, &root).unwrap().items[0].name, ".git");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn other_commands_include_files_and_subpaths() {
        let root = tree("files");
        let c = complete("cat R", &root, &root).unwrap();
        assert_eq!(c.items, [Item { name: "README.md".into(), is_dir: false }]);
        let c = complete("ls ~/ass", &root, &root).unwrap();
        assert_eq!(c.items[0].name, "assets");
        assert_eq!(c.base, "ass");
        let abs = format!("ls {}/dep", root.display());
        assert_eq!(complete(&abs, Path::new("/"), &root).unwrap().items[0].name, "dependencies");
        // ainda digitando o comando: Tab fica com o shell
        assert_eq!(complete("gi", &root, &root), None);
        assert_eq!(complete("", &root, &root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn accepting_replaces_typed_part_and_escapes() {
        let it = Item { name: "Meu Projeto".into(), is_dir: true };
        assert_eq!(accept_bytes("me", &it), b"\x7f\x7fMeu\\ Projeto/".to_vec());
        let f = Item { name: "a.txt".into(), is_dir: false };
        assert_eq!(accept_bytes("", &f), b"a.txt ".to_vec());
    }
}
