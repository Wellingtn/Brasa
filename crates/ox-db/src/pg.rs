//! PostgreSQL pelo protocolo simples (o mesmo do `psql`): todo valor chega como texto, e `NULL` como nulo.
//!
//! Abre só para leitura (`default_transaction_read_only`) e com tempo máximo por consulta (`statement_timeout`).
//! Esta versão conecta sem criptografia, então só aceita servidor na própria máquina ou por túnel SSH:
//! mandar senha e dados em claro por uma rede seria pior que não ter o recurso.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use postgres::error::SqlState;
use postgres::{Client, Config, NoTls, SimpleQueryMessage};

use crate::{has_trailing_statement, quote, Column, Error, ForeignKey, Page, QueryResult, Table, Value};

/// Tempo máximo para conectar.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct PgConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// só vive na memória; nunca é gravada
    pub password: Option<String>,
    pub dbname: String,
}

impl Default for PgConfig {
    fn default() -> Self {
        Self { host: "localhost".into(), port: 5432, user: "postgres".into(), password: None, dbname: "postgres".into() }
    }
}

/// O servidor está nesta máquina? (Só esses recebem conexão sem TLS.)
pub fn is_local_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']').to_lowercase();
    // endereço de verdade (127.0.0.0/8 ou ::1), nunca "começa com 127."; ou o nome localhost; ou socket unix
    h == "localhost" || h.starts_with('/') || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn map_pg(e: postgres::Error) -> Error {
    if let Some(db) = e.as_db_error() {
        let code = db.code();
        if *code == SqlState::INVALID_PASSWORD || *code == SqlState::INVALID_AUTHORIZATION_SPECIFICATION {
            return Error::Auth;
        }
        if *code == SqlState::QUERY_CANCELED {
            return Error::Timeout;
        }
        if *code == SqlState::READ_ONLY_SQL_TRANSACTION {
            return Error::ReadOnly;
        }
        return Error::Sql(db.message().to_string());
    }
    Error::Connect(e.to_string())
}

pub struct PgDatabase {
    client: Client,
    cfg: PgConfig,
    timeout_ms: u64,
}

fn text(v: Option<&str>) -> Value {
    v.map_or(Value::Null, |s| Value::Text(s.to_string()))
}

/// Linhas e colunas de uma resposta do protocolo simples.
fn collect(msgs: Vec<SimpleQueryMessage>) -> (Vec<String>, Vec<Vec<Value>>, Option<u64>) {
    let (mut cols, mut rows, mut done) = (Vec::new(), Vec::new(), None);
    for m in msgs {
        match m {
            SimpleQueryMessage::RowDescription(d) if cols.is_empty() => cols = d.iter().map(|c| c.name().to_string()).collect(),
            SimpleQueryMessage::Row(r) => {
                if cols.is_empty() {
                    cols = r.columns().iter().map(|c| c.name().to_string()).collect();
                }
                rows.push((0..r.len()).map(|i| text(r.get(i))).collect());
            }
            SimpleQueryMessage::CommandComplete(n) => done = Some(n),
            _ => {}
        }
    }
    (cols, rows, done)
}

impl PgDatabase {
    pub fn connect(cfg: &PgConfig) -> Result<PgDatabase, Error> {
        if !is_local_host(&cfg.host) {
            return Err(Error::RemoteNeedsTls(cfg.host.clone()));
        }
        let timeout_ms = 10_000;
        let mut c = Config::new();
        c.host(&cfg.host).port(cfg.port).user(&cfg.user).dbname(&cfg.dbname).connect_timeout(CONNECT_TIMEOUT).application_name("brasa");
        c.options(&format!("-c default_transaction_read_only=on -c statement_timeout={timeout_ms}"));
        if let Some(p) = &cfg.password {
            c.password(p);
        }
        let client = c.connect(NoTls).map_err(|e| {
            // sem senha, o cliente só diz "invalid configuration"; a causa é a senha que faltou
            let missing = std::error::Error::source(&e).is_some_and(|s| s.to_string().contains("password missing"));
            if missing && cfg.password.is_none() {
                Error::PasswordRequired
            } else {
                map_pg(e)
            }
        })?;
        Ok(PgDatabase { client, cfg: PgConfig { password: None, ..cfg.clone() }, timeout_ms })
    }

    /// `usuario@servidor:porta/banco`, sem senha.
    pub fn label(&self) -> String {
        format!("{}@{}:{}/{}", self.cfg.user, self.cfg.host, self.cfg.port, self.cfg.dbname)
    }

    fn simple(&mut self, sql: &str) -> Result<Vec<SimpleQueryMessage>, Error> {
        self.client.simple_query(sql).map_err(map_pg)
    }

    fn set_timeout(&mut self, timeout: Duration) -> Result<(), Error> {
        let ms = timeout.as_millis().clamp(100, 600_000) as u64;
        if ms != self.timeout_ms {
            self.client.batch_execute(&format!("SET statement_timeout = {ms}")).map_err(map_pg)?;
            self.timeout_ms = ms;
        }
        Ok(())
    }

    /// Tabelas e visões dos esquemas do usuário, com colunas, chave primária e índices.
    pub fn schema(&mut self) -> Result<Vec<Table>, Error> {
        const NOT_SYSTEM: &str = "table_schema NOT IN ('pg_catalog', 'information_schema') AND table_schema NOT LIKE 'pg_toast%'";
        let tables = self.simple(&format!("SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE {NOT_SYSTEM} ORDER BY 1, 2"))?;
        let cols = self.simple(&format!("SELECT table_schema, table_name, column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE {NOT_SYSTEM} ORDER BY table_schema, table_name, ordinal_position"))?;
        let pks = self.simple(
            "SELECT kcu.table_schema, kcu.table_name, kcu.column_name FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema AND tc.table_name = kcu.table_name \
             WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY kcu.ordinal_position",
        )?;
        let idx = self.simple("SELECT schemaname, tablename, indexname FROM pg_indexes WHERE schemaname NOT IN ('pg_catalog', 'information_schema') ORDER BY 1, 2, 3")?;

        let key = |a: Option<&str>, b: Option<&str>| (a.unwrap_or("").to_string(), b.unwrap_or("").to_string());
        let mut by_table: BTreeMap<(String, String), Table> = BTreeMap::new();
        for m in tables {
            if let SimpleQueryMessage::Row(r) = m {
                let (schema, name) = key(r.get(0), r.get(1));
                by_table.insert((schema.clone(), name.clone()), Table { schema: Some(schema), name, is_view: r.get(2) == Some("VIEW"), columns: Vec::new(), indexes: Vec::new(), without_rowid: false, foreign_keys: Vec::new() });
            }
        }
        for m in cols {
            if let SimpleQueryMessage::Row(r) = m {
                if let Some(t) = by_table.get_mut(&key(r.get(0), r.get(1))) {
                    t.columns.push(Column { name: r.get(2).unwrap_or("").to_string(), decl_type: r.get(3).unwrap_or("").to_string(), not_null: r.get(4) == Some("NO"), pk: 0, default: r.get(5).map(str::to_string) });
                }
            }
        }
        let mut pk_pos: BTreeMap<(String, String), usize> = BTreeMap::new();
        for m in pks {
            if let SimpleQueryMessage::Row(r) = m {
                let k = key(r.get(0), r.get(1));
                let pos = pk_pos.entry(k.clone()).or_insert(0);
                *pos += 1;
                if let Some(c) = by_table.get_mut(&k).and_then(|t| t.columns.iter_mut().find(|c| Some(c.name.as_str()) == r.get(2))) {
                    c.pk = *pos;
                }
            }
        }
        for m in idx {
            if let SimpleQueryMessage::Row(r) = m {
                if let Some(t) = by_table.get_mut(&key(r.get(0), r.get(1))) {
                    t.indexes.push(r.get(2).unwrap_or("").to_string());
                }
            }
        }
        // chaves estrangeiras pelo catálogo; os nomes das colunas vêm juntos, separados pelo caractere 31
        let fks = self.simple(
            "SELECT ns.nspname, cl.relname, fns.nspname, fcl.relname, \
             array_to_string(ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord) JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum ORDER BY k.ord), chr(31)), \
             array_to_string(ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord) JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum ORDER BY k.ord), chr(31)) \
             FROM pg_constraint con JOIN pg_class cl ON cl.oid = con.conrelid JOIN pg_namespace ns ON ns.oid = cl.relnamespace \
             JOIN pg_class fcl ON fcl.oid = con.confrelid JOIN pg_namespace fns ON fns.oid = fcl.relnamespace \
             WHERE con.contype = 'f' AND ns.nspname NOT IN ('pg_catalog', 'information_schema') ORDER BY 1, 2, con.conname",
        )?;
        for m in fks {
            if let SimpleQueryMessage::Row(r) = m {
                if let Some(t) = by_table.get_mut(&key(r.get(0), r.get(1))) {
                    let list = |i: usize| r.get(i).unwrap_or("").split('\u{1f}').map(str::to_string).collect::<Vec<_>>();
                    t.foreign_keys.push(ForeignKey { columns: list(4), ref_schema: r.get(2).map(str::to_string), ref_table: r.get(3).unwrap_or("").to_string(), ref_columns: list(5) });
                }
            }
        }
        // `public` primeiro, depois os outros esquemas, cada um em ordem de nome
        let mut out: Vec<Table> = by_table.into_values().collect();
        out.sort_by_key(|t| (t.schema.as_deref() != Some("public"), t.schema.clone(), t.name.to_lowercase()));
        Ok(out)
    }

    fn qualified(t: &Table) -> String {
        match &t.schema {
            Some(s) => format!("{}.{}", quote(s), quote(&t.name)),
            None => quote(&t.name),
        }
    }

    pub fn count_rows(&mut self, t: &Table) -> Result<i64, Error> {
        let (_, rows, _) = collect(self.simple(&format!("SELECT count(*) FROM {}", Self::qualified(t)))?);
        match rows.first().and_then(|r| r.first()) {
            Some(Value::Text(n)) => n.parse().map_err(|_| Error::Sql("contagem inválida".into())),
            _ => Err(Error::Sql("contagem vazia".into())),
        }
    }

    /// Uma página da tabela. `t` precisa vir de [`PgDatabase::schema`]: o nome só vale se existia no esquema.
    pub fn page(&mut self, t: &Table, sort: Option<(&str, bool)>, offset: usize, limit: usize) -> Result<Page, Error> {
        let order = match sort {
            Some((col, asc)) => {
                let c = t.columns.iter().find(|c| c.name == col).ok_or_else(|| Error::UnknownColumn { table: t.name.clone(), column: col.to_string() })?;
                format!(" ORDER BY {} {}", quote(&c.name), if asc { "ASC" } else { "DESC" })
            }
            None => String::new(),
        };
        let all: Vec<String> = t.columns.iter().map(|c| quote(&c.name)).collect();
        let sql = format!("SELECT {} FROM {}{order} LIMIT {} OFFSET {}", all.join(", "), Self::qualified(t), limit + 1, offset);
        let (_, mut rows, _) = collect(self.simple(&sql)?);
        let truncated = rows.len() > limit;
        rows.truncate(limit);
        let keys = vec![Vec::new(); rows.len()];
        Ok(Page { columns: t.columns.iter().map(|c| c.name.clone()).collect(), rows, keys, truncated })
    }

    /// Roda SQL. Consulta de leitura (`SELECT`, `WITH`, `VALUES`, `TABLE`) vai por cursor e traz no máximo `limit`
    /// linhas; o resto (comandos, `SHOW`, `EXPLAIN`) roda direto e é cortado em `limit`.
    pub fn query(&mut self, sql: &str, limit: usize, timeout: Duration) -> Result<QueryResult, Error> {
        let started = Instant::now();
        if has_trailing_statement(sql) {
            return Err(Error::MultipleStatements);
        }
        self.set_timeout(timeout)?;
        let first = sql.trim_start().split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("").to_lowercase();
        let sql = sql.trim().trim_end_matches(';');
        let result = if matches!(first.as_str(), "select" | "with" | "values" | "table") {
            self.client.batch_execute("BEGIN").map_err(map_pg)?;
            let run = (|| {
                self.client.batch_execute(&format!("DECLARE brasa_cursor NO SCROLL CURSOR FOR {sql}")).map_err(map_pg)?;
                self.simple(&format!("FETCH {} FROM brasa_cursor", limit + 1))
            })();
            let _ = self.client.batch_execute("ROLLBACK");
            let (columns, mut rows, _) = collect(run?);
            let truncated = rows.len() > limit;
            rows.truncate(limit);
            QueryResult { columns, rows, truncated, elapsed: Duration::ZERO, changes: None }
        } else {
            let (columns, mut rows, done) = collect(self.simple(sql)?);
            let truncated = rows.len() > limit;
            rows.truncate(limit);
            let changes = columns.is_empty().then_some(done.unwrap_or(0) as usize);
            QueryResult { columns, rows, truncated, elapsed: Duration::ZERO, changes }
        };
        Ok(QueryResult { elapsed: started.elapsed(), ..result })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_machine_is_local() {
        for ok in ["localhost", "LOCALHOST", "127.0.0.1", "127.1.2.3", "::1", "[::1]", "/var/run/postgresql"] {
            assert!(is_local_host(ok), "{ok}");
        }
        for no in ["db.exemplo.com", "10.0.0.5", "192.168.1.2", "localhost.exemplo.com", "127.evil.com.br", "0.0.0.0", "::", "127.0.0.1.evil.com", ""] {
            assert!(!is_local_host(no), "{no}");
        }
    }

    #[test]
    fn a_remote_server_is_refused_before_any_connection() {
        let cfg = PgConfig { host: "db.exemplo.com".into(), ..PgConfig::default() };
        assert_eq!(PgDatabase::connect(&cfg).err(), Some(Error::RemoteNeedsTls("db.exemplo.com".into())));
    }
}

/// Testes contra um PostgreSQL de verdade. Rodam só com `BRASA_TEST_PG=servidor:porta` (e `BRASA_TEST_PG_PASSWORD`,
/// `BRASA_TEST_PG_DB`), num banco com as tabelas `pessoas`, `tags`, a visão `adultos` e `extra.log`; sem isso, passam sem fazer nada.
#[cfg(test)]
mod live {
    use super::*;

    fn cfg() -> Option<PgConfig> {
        let target = std::env::var("BRASA_TEST_PG").ok()?;
        let (host, port) = target.split_once(':')?;
        Some(PgConfig { host: host.into(), port: port.parse().ok()?, user: "postgres".into(), password: std::env::var("BRASA_TEST_PG_PASSWORD").ok(), dbname: std::env::var("BRASA_TEST_PG_DB").unwrap_or_else(|_| "postgres".into()) })
    }

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn schema_has_tables_views_keys_and_other_schemas() {
        let Some(cfg) = cfg() else { return };
        let mut db = PgDatabase::connect(&cfg).unwrap();
        let s = db.schema().unwrap();
        let labels: Vec<_> = s.iter().map(Table::label).collect();
        assert!(labels.starts_with(&["adultos".to_string(), "pessoas".to_string()]) && labels.contains(&"extra.log".to_string()), "{labels:?}");
        let pessoas = s.iter().find(|t| t.name == "pessoas").unwrap();
        assert_eq!(pessoas.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "nome", "idade", "nota", "ativo"]);
        assert_eq!(pessoas.columns[0].pk, 1);
        assert!(pessoas.columns[1].not_null && !pessoas.columns[2].not_null);
        assert_eq!(pessoas.columns[3].decl_type, "numeric");
        assert!(pessoas.indexes.contains(&"idx_nome".to_string()));
        let tags = s.iter().find(|t| t.name == "tags").unwrap();
        assert_eq!(tags.columns.iter().map(|c| c.pk).collect::<Vec<_>>(), [1, 2, 0]);
        assert!(s.iter().find(|t| t.name == "adultos").unwrap().is_view);
        // chave estrangeira: a tabela `tags_pessoas` (criada no preparo do teste) aponta para pessoas.id
        if let Some(tp) = s.iter().find(|t| t.name == "tags_pessoas") {
            assert_eq!(tp.foreign_keys.len(), 1);
            assert_eq!((tp.foreign_keys[0].columns.as_slice(), tp.foreign_keys[0].ref_table.as_str()), (&["pessoa_id".to_string()][..], "pessoas"));
            assert!(tp.is_fk("pessoa_id") && !tp.is_fk("rotulo"));
        }
        assert_eq!(s.iter().find(|t| t.name == "log").unwrap().schema.as_deref(), Some("extra"));
    }

    #[test]
    fn pages_are_sorted_counted_and_cut() {
        let Some(cfg) = cfg() else { return };
        let mut db = PgDatabase::connect(&cfg).unwrap();
        let t = db.schema().unwrap().into_iter().find(|t| t.name == "pessoas").unwrap();
        assert_eq!(db.count_rows(&t).unwrap(), 251);
        let p = db.page(&t, Some(("id", false)), 0, 100).unwrap();
        assert_eq!((p.rows.len(), p.truncated), (100, true));
        assert_eq!(p.rows[0][0], Value::Text("251".into()));
        assert_eq!(p.rows[0][2], Value::Null); // NULL é nulo, não o texto "NULL"
        let last = db.page(&t, None, 200, 100).unwrap();
        assert_eq!((last.rows.len(), last.truncated), (51, false));
        assert!(db.page(&t, Some(("fantasma; DROP TABLE pessoas", true)), 0, 10).is_err());
        assert_eq!(db.count_rows(&t).unwrap(), 251);
    }

    #[test]
    fn queries_return_text_limit_rows_and_report_columns_even_when_empty() {
        let Some(cfg) = cfg() else { return };
        let mut db = PgDatabase::connect(&cfg).unwrap();
        let r = db.query("SELECT id, nome, nota FROM pessoas ORDER BY id;", 5, T).unwrap();
        assert_eq!(r.columns, ["id", "nome", "nota"]);
        assert_eq!((r.rows.len(), r.truncated), (5, true));
        assert_eq!(r.rows[0], vec![Value::Text("1".into()), Value::Text("Pessoa 1".into()), Value::Text("1.37".into())]);
        let all = db.query("SELECT id FROM pessoas WHERE id <= 3", 10, T).unwrap();
        assert_eq!((all.rows.len(), all.truncated), (3, false));
        let none = db.query("SELECT id, nome FROM pessoas WHERE false", 10, T).unwrap();
        assert_eq!((none.columns.as_slice(), none.rows.len()), (&["id".to_string(), "nome".to_string()][..], 0));
        assert_eq!(db.query("SHOW server_version_num", 10, T).unwrap().rows.len(), 1);
        assert_eq!(db.query("WITH x AS (SELECT 1 AS a) SELECT a FROM x", 10, T).unwrap().rows[0][0], Value::Text("1".into()));
    }

    #[test]
    fn the_connection_is_read_only() {
        let Some(cfg) = cfg() else { return };
        let mut db = PgDatabase::connect(&cfg).unwrap();
        assert_eq!(db.query("DELETE FROM pessoas", 10, T).unwrap_err(), Error::ReadOnly);
        assert_eq!(db.query("INSERT INTO tags VALUES ('a','b','c')", 10, T).unwrap_err(), Error::ReadOnly);
        assert_eq!(db.query("DROP TABLE pessoas", 10, T).unwrap_err(), Error::ReadOnly);
        // A proteção é contra acidente, não contra o próprio usuário: quem digita SET ... off no editor a desliga
        // (o botão de edição do M3 fará isso de forma explícita). Documentado aqui para ninguém achar que é cofre.
        assert!(db.query("SET default_transaction_read_only = off", 10, T).is_ok());
        assert!(db.query("INSERT INTO tags VALUES ('tmp', 'tmp', 'x')", 10, T).is_ok());
        let mut limpo = PgDatabase::connect(&cfg).unwrap();
        assert_eq!(limpo.query("DELETE FROM tags WHERE a = 'tmp'", 10, T).unwrap_err(), Error::ReadOnly); // outra conexão continua protegida
        assert!(db.query("DELETE FROM tags WHERE a = 'tmp'", 10, T).is_ok());
        let t = db.schema().unwrap().into_iter().find(|t| t.name == "pessoas").unwrap();
        assert_eq!(db.count_rows(&t).unwrap(), 251);
    }

    #[test]
    fn a_slow_query_is_cancelled_and_the_connection_survives() {
        let Some(cfg) = cfg() else { return };
        let mut db = PgDatabase::connect(&cfg).unwrap();
        let started = Instant::now();
        assert_eq!(db.query("SELECT pg_sleep(8)", 10, Duration::from_millis(400)).unwrap_err(), Error::Timeout);
        assert!(started.elapsed() < Duration::from_secs(4), "{:?}", started.elapsed());
        assert_eq!(db.query("SELECT 1 AS um", 10, T).unwrap().rows[0][0], Value::Text("1".into()));
    }

    #[test]
    fn errors_are_readable() {
        let Some(cfg) = cfg() else { return };
        let mut db = PgDatabase::connect(&cfg).unwrap();
        match db.query("SELECT * FROM nao_existe", 10, T) {
            Err(Error::Sql(m)) => assert!(m.contains("nao_existe"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(db.query("SELECT 1; SELECT 2", 10, T).unwrap_err(), Error::MultipleStatements);
        // depois de um erro dentro do cursor a conexão continua boa
        assert!(db.query("SELECT 1", 10, T).is_ok());
    }

    #[test]
    fn wrong_password_wrong_database_and_nothing_listening() {
        let Some(cfg) = cfg() else { return };
        assert_eq!(PgDatabase::connect(&PgConfig { password: Some("errada".into()), ..cfg.clone() }).err(), Some(Error::Auth));
        assert!(matches!(PgDatabase::connect(&PgConfig { dbname: "nao_existe".into(), ..cfg.clone() }).err(), Some(Error::Sql(m)) if m.contains("nao_existe")));
        assert!(matches!(PgDatabase::connect(&PgConfig { port: 1, ..cfg.clone() }).err(), Some(Error::Connect(_))));
        // sem senha num servidor que exige: mensagem clara, não "invalid configuration"
        if cfg.password.is_some() {
            assert_eq!(PgDatabase::connect(&PgConfig { password: None, ..cfg.clone() }).err(), Some(Error::PasswordRequired));
        }
        // a senha nunca aparece nas mensagens
        let e = PgDatabase::connect(&PgConfig { password: Some("segredo-xyz".into()), port: 1, ..cfg }).err().unwrap().to_string();
        assert!(!e.contains("segredo-xyz"), "{e}");
    }
}
