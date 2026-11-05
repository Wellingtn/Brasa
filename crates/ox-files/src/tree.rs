//! Árvore de arquivos expansível com carregamento preguiçoso (cada pasta só é lida ao expandir).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::{list_dir, Entry, EntryKind};

#[derive(Debug, Clone)]
pub struct TreeRow {
    pub entry: Entry,
    pub depth: usize,
    pub expanded: bool,
}

pub struct FileTree {
    root: PathBuf,
    show_hidden: bool,
    expanded: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Vec<Entry>>,
}

impl FileTree {
    pub fn new(root: impl Into<PathBuf>, show_hidden: bool) -> Self {
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        Self { root, show_hidden, expanded: HashSet::new(), cache: HashMap::new() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Troca a raiz e recolhe tudo.
    pub fn set_root(&mut self, root: impl Into<PathBuf>) {
        let root = root.into();
        let root = root.canonicalize().unwrap_or(root);
        if root != self.root {
            self.root = root;
            self.expanded.clear();
            self.cache.clear();
        }
    }

    pub fn set_show_hidden(&mut self, show: bool) {
        if show != self.show_hidden {
            self.show_hidden = show;
            self.cache.clear();
        }
    }

    /// Descarta o cache para reler do disco no próximo `rows()`.
    pub fn refresh(&mut self) {
        self.cache.clear();
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub fn toggle(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
    }

    fn children(&mut self, dir: &Path) -> Vec<Entry> {
        if let Some(c) = self.cache.get(dir) {
            return c.clone();
        }
        let entries = list_dir(dir, self.show_hidden).map(|l| l.entries).unwrap_or_default();
        self.cache.insert(dir.to_path_buf(), entries.clone());
        entries
    }

    /// Linhas visíveis em ordem de exibição (pastas expandidas incluem os filhos logo abaixo).
    pub fn rows(&mut self) -> Vec<TreeRow> {
        let mut out = Vec::new();
        let root = self.root.clone();
        self.walk(&root, 0, &mut out);
        out
    }

    fn walk(&mut self, dir: &Path, depth: usize, out: &mut Vec<TreeRow>) {
        for entry in self.children(dir) {
            let expanded = entry.kind == EntryKind::Dir && self.expanded.contains(&entry.path);
            let path = entry.path.clone();
            out.push(TreeRow { entry, depth, expanded });
            if expanded {
                self.walk(&path, depth + 1, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(rows: &[TreeRow]) -> Vec<(usize, String)> {
        rows.iter().map(|r| (r.depth, r.entry.name.clone())).collect()
    }

    #[test]
    fn expands_lazily_and_nests() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("src/deep")).unwrap();
        fs::write(d.path().join("src/main.rs"), "").unwrap();
        fs::write(d.path().join("src/deep/x.txt"), "").unwrap();
        fs::write(d.path().join("README.md"), "").unwrap();

        let mut t = FileTree::new(d.path(), false);
        assert_eq!(names(&t.rows()), [(0, "src".into()), (0, "README.md".into())]);

        let src = t.root().join("src");
        t.toggle(&src);
        assert_eq!(
            names(&t.rows()),
            [(0, "src".into()), (1, "deep".into()), (1, "main.rs".into()), (0, "README.md".into())]
        );
        assert!(t.rows()[0].expanded);

        t.toggle(&src.join("deep"));
        assert!(names(&t.rows()).contains(&(2, "x.txt".into())));

        t.toggle(&src); // recolher esconde os descendentes
        assert_eq!(t.rows().len(), 2);
    }

    #[test]
    fn hidden_toggle_and_root_change_reset_state() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(".env"), "").unwrap();
        fs::create_dir(d.path().join("a")).unwrap();
        let mut t = FileTree::new(d.path(), false);
        assert_eq!(t.rows().len(), 1);
        t.set_show_hidden(true);
        assert_eq!(t.rows().len(), 2);

        let a = t.root().join("a");
        t.toggle(&a);
        t.set_root(&a);
        assert!(!t.is_expanded(&a));
        assert_eq!(t.root(), a.canonicalize().unwrap());
    }

    #[test]
    fn refresh_picks_up_new_files() {
        let d = tempfile::tempdir().unwrap();
        let mut t = FileTree::new(d.path(), false);
        assert!(t.rows().is_empty());
        fs::write(d.path().join("novo.txt"), "").unwrap();
        assert!(t.rows().is_empty()); // ainda em cache
        t.refresh();
        assert_eq!(t.rows().len(), 1);
    }
}
