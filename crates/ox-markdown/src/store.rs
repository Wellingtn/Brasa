//! Armazenamento de notas `.md` em uma árvore de pastas (estilo "Drive").
//!
//! Notas e pastas são identificadas por caminho relativo com `/` (ex.: `projetos/deploy.md`).
//! Excluir move para `.lixeira/` dentro da raiz, nunca apaga de verdade.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

const TRASH_DIR: &str = ".lixeira";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("nome de nota inválido: {0:?}")]
    InvalidName(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteMeta {
    /// caminho relativo, com `.md`
    pub name: String,
    pub modified_unix: u64,
}

/// Resultado de uma busca nas notas: pelo nome ou por uma linha do conteúdo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteHit {
    /// caminho relativo, com `.md`
    pub name: String,
    /// primeira linha do conteúdo que contém o termo (vazio quando só o nome combina)
    pub snippet: String,
}

/// Linha da árvore de notas, na ordem de exibição.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteNode {
    /// caminho relativo (notas terminam em `.md`; pastas não)
    pub rel: String,
    /// nome para exibir (sem `.md`)
    pub label: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
}

pub struct NoteStore {
    root: PathBuf,
}

fn valid_segment(seg: &str) -> bool {
    !seg.is_empty() && !seg.starts_with('.') && !seg.contains(['\\', '\0']) && seg != ".."
}

impl NoteStore {
    pub fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Caminho absoluto de uma nota; só aceita segmentos simples (sem `..`, sem ocultos).
    pub fn path_of(&self, rel: &str) -> Result<PathBuf, StoreError> {
        self.resolve(rel, true)
    }

    fn resolve(&self, rel: &str, as_note: bool) -> Result<PathBuf, StoreError> {
        let segs: Vec<&str> = rel.split('/').collect();
        if rel.is_empty() || !segs.iter().all(|s| valid_segment(s)) {
            return Err(StoreError::InvalidName(rel.into()));
        }
        let mut path = self.root.clone();
        for (i, seg) in segs.iter().enumerate() {
            let last = i + 1 == segs.len();
            if last && as_note && !seg.ends_with(".md") {
                path.push(format!("{seg}.md"));
            } else {
                path.push(seg);
            }
        }
        Ok(path)
    }

    fn modified(entry: &std::fs::DirEntry) -> u64 {
        entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    }

    /// Todas as notas (recursivo), mais recentes primeiro.
    pub fn list(&self) -> Result<Vec<NoteMeta>, StoreError> {
        let mut notes = Vec::new();
        self.collect(&self.root, "", &mut notes)?;
        notes.sort_by(|a, b| b.modified_unix.cmp(&a.modified_unix).then(a.name.cmp(&b.name)));
        Ok(notes)
    }

    fn collect(&self, dir: &Path, prefix: &str, out: &mut Vec<NoteMeta>) -> Result<(), StoreError> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !valid_segment(&name) {
                continue; // oculta a lixeira e arquivos de sistema
            }
            let rel = format!("{prefix}{name}");
            if entry.path().is_dir() {
                self.collect(&entry.path(), &format!("{rel}/"), out)?;
            } else if name.ends_with(".md") {
                out.push(NoteMeta { name: rel, modified_unix: Self::modified(&entry) });
            }
        }
        Ok(())
    }

    /// Busca sem diferenciar maiúsculas: primeiro as notas cujo nome combina, depois as que têm o termo no
    /// conteúdo (com a primeira linha encontrada). Dentro de cada grupo, as mais recentes primeiro.
    pub fn search(&self, query: &str) -> Result<Vec<NoteHit>, StoreError> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let (mut by_name, mut by_text) = (Vec::new(), Vec::new());
        for meta in self.list()? {
            let snippet = self.read(&meta.name).ok().and_then(|text| {
                text.lines().find(|l| l.to_lowercase().contains(&q)).map(|l| {
                    let l = l.trim();
                    let cut: String = l.chars().take(90).collect();
                    if cut.len() < l.len() { format!("{cut}…") } else { cut }
                })
            });
            if meta.name.to_lowercase().contains(&q) {
                by_name.push(NoteHit { name: meta.name, snippet: snippet.unwrap_or_default() });
            } else if let Some(snippet) = snippet {
                by_text.push(NoteHit { name: meta.name, snippet });
            }
        }
        by_name.extend(by_text);
        Ok(by_name)
    }

    /// Árvore para a barra lateral: pastas primeiro, ordem alfabética; só desce nas pastas em `expanded`.
    pub fn tree(&self, expanded: &HashSet<String>) -> Result<Vec<NoteNode>, StoreError> {
        let mut out = Vec::new();
        self.walk(&self.root, "", 0, expanded, &mut out)?;
        Ok(out)
    }

    fn walk(&self, dir: &Path, prefix: &str, depth: usize, expanded: &HashSet<String>, out: &mut Vec<NoteNode>) -> Result<(), StoreError> {
        let mut items: Vec<(String, bool)> = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !valid_segment(&name) {
                continue;
            }
            let is_dir = entry.path().is_dir();
            if is_dir || name.ends_with(".md") {
                items.push((name, is_dir));
            }
        }
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        for (name, is_dir) in items {
            let rel = format!("{prefix}{name}");
            let open = is_dir && expanded.contains(&rel);
            let label = name.strip_suffix(".md").unwrap_or(&name).to_string();
            out.push(NoteNode { rel: rel.clone(), label, is_dir, depth, expanded: open });
            if open {
                self.walk(&dir.join(&name), &format!("{rel}/"), depth + 1, expanded, out)?;
            }
        }
        Ok(())
    }

    pub fn read(&self, rel: &str) -> Result<String, StoreError> {
        Ok(std::fs::read_to_string(self.resolve(rel, true)?)?)
    }

    /// Grava a nota, criando as pastas intermediárias.
    pub fn write(&self, rel: &str, content: &str) -> Result<(), StoreError> {
        let path = self.resolve(rel, true)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(std::fs::write(path, content)?)
    }

    /// Renomeia uma nota mantendo a pasta. `new_stem` é só o nome (sem `/` e sem `.md`).
    /// Devolve o novo caminho relativo; falha se o nome for inválido ou já existir.
    pub fn rename(&self, rel: &str, new_stem: &str) -> Result<String, StoreError> {
        let stem = new_stem.trim().trim_end_matches(".md");
        if stem.contains('/') || !valid_segment(stem) {
            return Err(StoreError::InvalidName(new_stem.into()));
        }
        let from = self.resolve(rel, true)?;
        let new_rel = match rel.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{stem}.md"),
            None => format!("{stem}.md"),
        };
        let to = self.resolve(&new_rel, true)?;
        if to == from {
            return Ok(new_rel);
        }
        if to.exists() {
            return Err(StoreError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "já existe uma nota com esse nome")));
        }
        std::fs::rename(from, to)?;
        Ok(new_rel)
    }

    pub fn create_folder(&self, rel: &str) -> Result<(), StoreError> {
        Ok(std::fs::create_dir_all(self.resolve(rel, false)?)?)
    }

    /// Move uma nota ou pasta para dentro de `dest_dir` (`""` = raiz); devolve o novo caminho relativo.
    /// Recusa mover uma pasta para dentro dela mesma e sobrescrever algo com o mesmo nome.
    pub fn move_into(&self, rel: &str, is_dir: bool, dest_dir: &str) -> Result<String, StoreError> {
        let from = self.resolve(rel, !is_dir)?;
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let name = if is_dir || name.ends_with(".md") { name.to_string() } else { format!("{name}.md") };
        if is_dir && (dest_dir == rel || dest_dir.starts_with(&format!("{rel}/"))) {
            return Err(StoreError::InvalidName(format!("{rel} não pode ir para dentro de si mesma")));
        }
        let new_rel = if dest_dir.is_empty() { name } else { format!("{dest_dir}/{name}") };
        let to = self.resolve(&new_rel, !is_dir)?;
        if to == from {
            return Ok(new_rel);
        }
        if to.exists() {
            return Err(StoreError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "já existe um item com esse nome na pasta")));
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to)?;
        Ok(new_rel)
    }

    /// Renomeia uma pasta (mesmo lugar, nome novo); devolve o novo caminho relativo.
    pub fn rename_folder(&self, rel: &str, new_name: &str) -> Result<String, StoreError> {
        let name = new_name.trim();
        if name.contains('/') || !valid_segment(name) {
            return Err(StoreError::InvalidName(new_name.into()));
        }
        let from = self.resolve(rel, false)?;
        let new_rel = match rel.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{name}"),
            None => name.to_string(),
        };
        let to = self.resolve(&new_rel, false)?;
        if to == from {
            return Ok(new_rel);
        }
        if to.exists() {
            return Err(StoreError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "já existe uma pasta com esse nome")));
        }
        std::fs::rename(from, to)?;
        Ok(new_rel)
    }

    /// Move uma nota ou pasta para `.lixeira/`; devolve o novo caminho.
    pub fn trash(&self, rel: &str, is_dir: bool) -> Result<PathBuf, StoreError> {
        let from = self.resolve(rel, !is_dir)?;
        let trash = self.root.join(TRASH_DIR);
        std::fs::create_dir_all(&trash)?;
        let stamp = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let base = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let to = trash.join(format!("{stamp}-{base}"));
        std::fs::rename(from, &to)?;
        Ok(to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, NoteStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = NoteStore::new(dir.path()).unwrap();
        (dir, s)
    }

    #[test]
    fn search_finds_notes_by_name_and_by_content() {
        let (_d, s) = store();
        s.write("receitas", "# Bolo\nmisture a Farinha com ovos\n").unwrap();
        s.write("farinha-de-trigo", "só o nome combina aqui").unwrap();
        s.write("tarefas", "comprar leite\n").unwrap();
        let hits = s.search("farinha").unwrap();
        // nome primeiro, depois conteúdo; sem diferenciar maiúsculas
        assert_eq!(hits.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["farinha-de-trigo.md", "receitas.md"]);
        assert_eq!(hits[0].snippet, "");
        assert_eq!(hits[1].snippet, "misture a Farinha com ovos");
        assert!(s.search("  ").unwrap().is_empty());
        assert!(s.search("inexistente").unwrap().is_empty());
        // limita o trecho
        s.write("longa", &format!("{} agulha", "x".repeat(200))).unwrap();
        assert!(s.search("agulha").unwrap()[0].snippet.ends_with('…'));
    }

    #[test]
    fn move_notes_and_folders_between_folders() {
        let (_d, s) = store();
        s.write("ideia", "x").unwrap();
        s.create_folder("trabalho").unwrap();
        s.create_folder("trabalho/sub").unwrap();
        assert_eq!(s.move_into("ideia.md", false, "trabalho").unwrap(), "trabalho/ideia.md");
        assert_eq!(s.read("trabalho/ideia.md").unwrap(), "x");
        // de volta para a raiz
        assert_eq!(s.move_into("trabalho/ideia.md", false, "").unwrap(), "ideia.md");
        // pasta para dentro dela mesma ou de uma subpasta: recusado
        assert!(s.move_into("trabalho", true, "trabalho/sub").is_err());
        assert!(s.move_into("trabalho", true, "trabalho").is_err());
        s.create_folder("arquivo").unwrap();
        assert_eq!(s.move_into("trabalho", true, "arquivo").unwrap(), "arquivo/trabalho");
        assert!(s.path_of("arquivo/trabalho/sub").is_ok());
        // nome repetido no destino: recusado
        s.write("arquivo/ideia", "y").unwrap();
        assert!(s.move_into("ideia.md", false, "arquivo").is_err());
    }

    #[test]
    fn rename_folder_keeps_contents() {
        let (_d, s) = store();
        s.write("velha/nota", "z").unwrap();
        assert_eq!(s.rename_folder("velha", "nova").unwrap(), "nova");
        assert_eq!(s.read("nova/nota.md").unwrap(), "z");
        assert!(s.rename_folder("nova", "a/b").is_err());
        s.create_folder("outra").unwrap();
        assert!(s.rename_folder("nova", "outra").is_err());
    }

    #[test]
    fn roundtrip_nested_and_list() {
        let (_d, s) = store();
        s.write("deploy", "# Deploy").unwrap();
        s.write("projetos/api/notas", "x").unwrap(); // cria as pastas
        assert_eq!(s.read("deploy.md").unwrap(), "# Deploy");
        assert_eq!(s.read("projetos/api/notas").unwrap(), "x");
        let mut names: Vec<_> = s.list().unwrap().into_iter().map(|n| n.name).collect();
        names.sort();
        assert_eq!(names, ["deploy.md", "projetos/api/notas.md"]);
    }

    #[test]
    fn rejects_path_traversal_and_hidden() {
        let (_d, s) = store();
        for bad in ["../x", "a/../b", ".hidden", "", "a\\b", "/abs", "a//b", ".lixeira/x"] {
            assert!(s.write(bad, "x").is_err(), "{bad:?} deveria falhar");
            assert!(s.create_folder(bad).is_err(), "{bad:?} deveria falhar (pasta)");
        }
    }

    #[test]
    fn tree_orders_dirs_first_and_respects_expansion() {
        let (_d, s) = store();
        s.write("zeta", "").unwrap();
        s.write("Alfa", "").unwrap();
        s.create_folder("docs").unwrap();
        s.write("docs/guia", "").unwrap();
        s.create_folder("docs/sub").unwrap();

        let none = HashSet::new();
        let t = s.tree(&none).unwrap();
        let flat: Vec<_> = t.iter().map(|n| (n.depth, n.rel.as_str())).collect();
        assert_eq!(flat, [(0, "docs"), (0, "Alfa.md"), (0, "zeta.md")]);

        let open: HashSet<String> = ["docs".to_string()].into();
        let t = s.tree(&open).unwrap();
        let flat: Vec<_> = t.iter().map(|n| (n.depth, n.rel.as_str(), n.is_dir)).collect();
        assert_eq!(flat[..3], [(0, "docs", true), (1, "docs/sub", true), (1, "docs/guia.md", false)]);
        assert_eq!(t[2].label, "guia");
        assert!(t[0].expanded);
    }

    #[test]
    fn rename_keeps_folder_and_rejects_collisions_and_bad_names() {
        let (_d, s) = store();
        s.write("pasta/velha", "conteúdo").unwrap();
        s.write("pasta/outra", "x").unwrap();
        s.write("raiz", "r").unwrap();

        assert_eq!(s.rename("pasta/velha.md", "nova").unwrap(), "pasta/nova.md");
        assert_eq!(s.read("pasta/nova").unwrap(), "conteúdo");
        assert!(s.read("pasta/velha").is_err());
        assert_eq!(s.rename("raiz", "  Com Espaço.md ").unwrap(), "Com Espaço.md"); // aparado e sem .md duplicado
        assert_eq!(s.rename("pasta/nova.md", "nova").unwrap(), "pasta/nova.md"); // mesmo nome: sem efeito

        assert!(s.rename("pasta/nova.md", "outra").is_err(), "colisão não sobrescreve");
        assert_eq!(s.read("pasta/outra").unwrap(), "x");
        for bad in ["", "a/b", "../x", ".oculta", "  "] {
            assert!(s.rename("pasta/nova.md", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn trash_moves_out_of_sight_but_keeps_data() {
        let (_d, s) = store();
        s.write("pasta/velha", "conteúdo").unwrap();
        let moved = s.trash("pasta/velha.md", false).unwrap();
        assert_eq!(std::fs::read_to_string(moved).unwrap(), "conteúdo");
        assert!(s.read("pasta/velha").is_err());
        assert!(s.list().unwrap().is_empty()); // a lixeira não aparece na listagem

        s.create_folder("tmp").unwrap();
        s.trash("tmp", true).unwrap();
        let t = s.tree(&HashSet::new()).unwrap();
        assert!(t.iter().all(|n| n.rel != "tmp" && !n.rel.starts_with('.')));
    }
}
