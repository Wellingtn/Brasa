//! Backend do gerenciador de arquivos: listagem de diretório e pré-visualização rápida.

mod tree;
pub use tree::{FileTree, TreeRow};

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use serde::Serialize;

const TEXT_PREVIEW_BYTES: usize = 64 * 1024;
const IMAGE_PREVIEW_MAX: u64 = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Nome do branch git (ou hash curto em HEAD destacado) do repositório que contém `path`.
pub fn git_branch(path: &Path) -> Option<String> {
    for dir in path.ancestors() {
        let dot_git = dir.join(".git");
        if !dot_git.exists() {
            continue;
        }
        // worktrees/submódulos: .git é um arquivo "gitdir: <caminho>"
        let git_dir = if dot_git.is_file() {
            let txt = fs::read_to_string(&dot_git).ok()?;
            let target = PathBuf::from(txt.strip_prefix("gitdir:")?.trim());
            if target.is_absolute() { target } else { dir.join(target) }
        } else {
            dot_git
        };
        let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
        let head = head.trim();
        return Some(match head.strip_prefix("ref: refs/heads/") {
            Some(b) => b.to_string(),
            None => head.chars().take(7).collect(),
        });
    }
    None
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: u64,
    pub modified_unix: u64,
    pub hidden: bool,
}

#[derive(Debug, Serialize)]
pub struct Listing {
    pub path: PathBuf,
    pub parent: Option<PathBuf>,
    pub entries: Vec<Entry>,
}

/// Lista um diretório: pastas primeiro, depois ordem alfabética sem distinguir caixa.
pub fn list_dir(path: &Path, show_hidden: bool) -> Result<Listing, FilesError> {
    let path = path.canonicalize()?;
    let mut entries = Vec::new();
    for e in fs::read_dir(&path)? {
        let Ok(e) = e else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let hidden = name.starts_with('.');
        if hidden && !show_hidden {
            continue;
        }
        let Ok(lmeta) = e.path().symlink_metadata() else { continue };
        // segue o symlink para decidir se é pasta; se quebrado, mostra como Symlink
        let meta = fs::metadata(e.path()).unwrap_or_else(|_| lmeta.clone());
        let kind = if meta.is_dir() {
            EntryKind::Dir
        } else if lmeta.file_type().is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };
        let modified_unix = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        entries.push(Entry { name, path: e.path(), kind, size: meta.len(), modified_unix, hidden });
    }
    entries.sort_by(|a, b| {
        let a_dir = a.kind == EntryKind::Dir;
        let b_dir = b.kind == EntryKind::Dir;
        b_dir.cmp(&a_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing { parent: path.parent().map(Path::to_path_buf), path, entries })
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Preview {
    Text { content: String, truncated: bool, language: String },
    Image { mime: String, data_base64: String },
    Binary { size: u64 },
    TooLarge { size: u64 },
}

fn image_mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

pub fn preview(path: &Path) -> Result<Preview, FilesError> {
    let size = fs::metadata(path)?.len();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    if let Some(mime) = image_mime(&ext) {
        if size > IMAGE_PREVIEW_MAX {
            return Ok(Preview::TooLarge { size });
        }
        let bytes = fs::read(path)?;
        return Ok(Preview::Image {
            mime: mime.into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }

    let mut buf = Vec::with_capacity(TEXT_PREVIEW_BYTES.min(size as usize));
    fs::File::open(path)?.take(TEXT_PREVIEW_BYTES as u64).read_to_end(&mut buf)?;
    if buf.contains(&0) {
        return Ok(Preview::Binary { size });
    }
    // corte no meio de um caractere UTF-8 não deve contar como binário
    let content = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            if e.as_bytes().len() - valid > 3 {
                return Ok(Preview::Binary { size });
            }
            String::from_utf8_lossy(&e.as_bytes()[..valid]).into_owned()
        }
    };
    Ok(Preview::Text { content, truncated: size > TEXT_PREVIEW_BYTES as u64, language: ext })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_first_hidden_filtered() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("b.txt"), "x").unwrap();
        fs::write(d.path().join(".secret"), "x").unwrap();
        fs::create_dir(d.path().join("zdir")).unwrap();
        let l = list_dir(d.path(), false).unwrap();
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zdir", "b.txt"]);
        assert_eq!(list_dir(d.path(), true).unwrap().entries.len(), 3);
    }

    #[test]
    fn git_branch_walks_up_and_handles_detached_head() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join(".git")).unwrap();
        fs::create_dir_all(d.path().join("src/deep")).unwrap();
        fs::write(d.path().join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(git_branch(&d.path().join("src/deep")).as_deref(), Some("feature/x"));
        fs::write(d.path().join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(git_branch(d.path()).as_deref(), Some("0123456"));
        let other = tempfile::tempdir().unwrap();
        assert_eq!(git_branch(other.path()), None);
    }

    #[test]
    fn text_image_binary_previews() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.rs"), "fn main() {}").unwrap();
        fs::write(d.path().join("p.png"), [0x89, b'P', b'N', b'G']).unwrap();
        fs::write(d.path().join("blob"), [1, 0, 2]).unwrap();
        assert!(matches!(preview(&d.path().join("a.rs")).unwrap(), Preview::Text { language, .. } if language == "rs"));
        assert!(matches!(preview(&d.path().join("p.png")).unwrap(), Preview::Image { .. }));
        assert!(matches!(preview(&d.path().join("blob")).unwrap(), Preview::Binary { size: 3 }));
    }
}
