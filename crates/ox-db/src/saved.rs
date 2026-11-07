//! Conexões salvas do painel Banco, em `~/.config/brasa/databases.json`.
//!
//! Guarda só o que identifica o banco (nome, tipo, servidor, porta, usuário, banco ou arquivo). **A senha nunca
//! entra aqui**: o tipo `SavedConn` nem tem esse campo, e o arquivo é lido ignorando qualquer campo desconhecido
//! (se alguém puser uma `password` à mão, ela é descartada na próxima gravação).

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Postgres,
    Sqlite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedConn {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub dbname: String,
    /// caminho do arquivo (SQLite)
    #[serde(default)]
    pub path: String,
}

impl SavedConn {
    /// `usuario@servidor:porta/banco` ou o caminho do arquivo.
    pub fn address(&self) -> String {
        match self.kind {
            Kind::Postgres => format!("{}@{}:{}/{}", self.user, self.host, self.port, self.dbname),
            Kind::Sqlite => self.path.clone(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Store {
    pub conns: Vec<SavedConn>,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    connections: Vec<SavedConn>,
}

impl Store {
    /// Lê o arquivo. Não existe: lista vazia. Corrompido: lista vazia e o motivo (o arquivo ruim não é apagado).
    pub fn load(path: &Path) -> (Store, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Store::default(), None),
            Err(e) => return (Store::default(), Some(e.to_string())),
        };
        match serde_json::from_str::<File>(&text) {
            Ok(f) => (Store { conns: f.connections }, None),
            Err(e) => (Store::default(), Some(format!("databases.json inválido: {e}"))),
        }
    }

    /// Grava só para o dono (0600), trocando o arquivo de uma vez.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&File { connections: self.conns.clone() }).map_err(std::io::Error::other)? + "\n";
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

    fn next_id(&self) -> u64 {
        self.conns.iter().map(|c| c.id).max().unwrap_or(0) + 1
    }

    /// Adiciona (id novo) ou, se `c.id` já existe, substitui. Devolve o id.
    pub fn upsert(&mut self, mut c: SavedConn) -> u64 {
        match self.conns.iter_mut().find(|x| x.id == c.id && c.id != 0) {
            Some(slot) => {
                let id = slot.id;
                *slot = c;
                id
            }
            None => {
                c.id = self.next_id();
                let id = c.id;
                self.conns.push(c);
                id
            }
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let n = self.conns.len();
        self.conns.retain(|c| c.id != id);
        self.conns.len() != n
    }

    pub fn get(&self, id: u64) -> Option<&SavedConn> {
        self.conns.iter().find(|c| c.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(name: &str) -> SavedConn {
        SavedConn { id: 0, name: name.into(), kind: Kind::Postgres, host: "localhost".into(), port: 5432, user: "ana".into(), dbname: name.into(), path: String::new() }
    }

    #[test]
    fn connections_are_added_edited_and_removed() {
        let mut s = Store::default();
        let a = s.upsert(pg("loja"));
        let b = s.upsert(pg("financeiro"));
        assert_ne!(a, b);
        assert_eq!(s.conns.len(), 2);
        s.upsert(SavedConn { id: a, port: 5433, ..pg("loja") });
        assert_eq!((s.conns.len(), s.get(a).unwrap().port), (2, 5433));
        assert!(s.remove(a));
        assert!(!s.remove(a));
        assert_eq!(s.conns.len(), 1);
        // o id de uma conexão apagada não é reaproveitado por engano no meio da lista
        assert_eq!(s.upsert(pg("novo")), b + 1);
    }

    #[test]
    fn the_file_round_trips_and_is_owner_only() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("cfg").join("databases.json");
        let mut s = Store::default();
        s.upsert(pg("loja"));
        s.upsert(SavedConn { id: 0, name: "vendas".into(), kind: Kind::Sqlite, host: String::new(), port: 0, user: String::new(), dbname: String::new(), path: "/x/v.sqlite".into() });
        s.save(&path).unwrap();
        let (back, err) = Store::load(&path);
        assert_eq!((back, err), (s, None));
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions()) & 0o777, 0o600);
    }

    #[test]
    fn a_password_never_reaches_the_file() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("databases.json");
        // alguém pôs a senha à mão: ela é ignorada ao ler e some ao gravar
        std::fs::write(&path, r#"{"connections":[{"id":1,"name":"x","kind":"postgres","host":"localhost","port":5432,"user":"u","dbname":"d","password":"segredo-123"}]}"#).unwrap();
        let (s, err) = Store::load(&path);
        assert_eq!((s.conns.len(), err), (1, None));
        s.save(&path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("segredo-123"));
    }

    #[test]
    fn missing_and_broken_files_are_handled() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Store::load(&d.path().join("nada.json")), (Store::default(), None));
        let bad = d.path().join("ruim.json");
        std::fs::write(&bad, "{ isto não é json").unwrap();
        let (s, err) = Store::load(&bad);
        assert!(s.conns.is_empty() && err.is_some_and(|e| e.contains("inválido")));
        // o arquivo ruim continua lá, para a pessoa poder consertar
        assert!(bad.exists());
    }

    #[test]
    fn address_shows_what_identifies_the_database() {
        assert_eq!(pg("loja").address(), "ana@localhost:5432/loja");
        assert_eq!(SavedConn { kind: Kind::Sqlite, path: "/a/b.db".into(), ..pg("x") }.address(), "/a/b.db");
    }
}
