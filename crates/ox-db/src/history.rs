//! Histórico das queries rodadas, por conexão, em `~/.config/brasa/db-history.json` (0600).
//!
//! Não guarda senha de conexão (ela nunca passa por aqui), mas o texto das queries é guardado como foi digitado:
//! por isso o arquivo é só do dono.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Quantas queries ficam guardadas por conexão.
pub const MAX_PER_CONN: usize = 100;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    entries: BTreeMap<String, Vec<String>>,
}

impl History {
    pub fn load(path: &Path) -> History {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            opts.open(&tmp)?.write_all(text.as_bytes())?;
        }
        std::fs::rename(tmp, path)
    }

    /// Registra uma query rodada: a mais recente fica por último, repetidas são trocadas de lugar, texto vazio é ignorado.
    pub fn push(&mut self, conn: &str, sql: &str) {
        let sql = sql.trim();
        if sql.is_empty() {
            return;
        }
        let list = self.entries.entry(conn.to_string()).or_default();
        list.retain(|s| s != sql);
        list.push(sql.to_string());
        if list.len() > MAX_PER_CONN {
            list.remove(0);
        }
    }

    /// As queries da conexão, da mais antiga para a mais recente.
    pub fn list(&self, conn: &str) -> &[String] {
        self.entries.get(conn).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_dedupes_and_caps() {
        let mut h = History::default();
        h.push("a", "  ");
        h.push("a", "SELECT 1");
        h.push("a", "SELECT 2");
        h.push("a", "SELECT 1");
        h.push("b", "SELECT 9");
        assert_eq!(h.list("a"), ["SELECT 2", "SELECT 1"]);
        assert_eq!(h.list("b"), ["SELECT 9"]);
        assert!(h.list("c").is_empty());
        for i in 0..150 {
            h.push("a", &format!("SELECT {i}"));
        }
        assert_eq!(h.list("a").len(), MAX_PER_CONN);
        assert_eq!(h.list("a").last().unwrap(), "SELECT 149");
    }

    #[test]
    fn round_trips_through_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("h.json");
        let mut h = History::default();
        h.push("x", "SELECT 'çã'");
        h.save(&p).unwrap();
        assert_eq!(History::load(&p), h);
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&p).unwrap().permissions()) & 0o777, 0o600);
        assert_eq!(History::load(&dir.path().join("nao.json")), History::default());
    }
}
