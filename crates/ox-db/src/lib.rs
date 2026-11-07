//! Acesso a bancos SQLite para o painel de banco de dados do Brasa. Sem dependência de janela.
//!
//! Princípios:
//! - abre só leitura por padrão; escrever é uma escolha explícita ([`Options::read_only`]);
//! - nunca monta SQL juntando texto de fora: nomes de tabela e coluna só valem se existirem no esquema
//!   (e entram entre aspas), e valores vão sempre como parâmetros;
//! - toda consulta tem limite de linhas e tempo máximo, para uma consulta enorme ou infinita não travar o app.

mod pg;
mod saved;
pub use pg::{is_local_host, PgConfig, PgDatabase};
pub use saved::{Kind, SavedConn, Store};

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{Connection, ErrorCode, OpenFlags, ToSql};

/// Linhas devolvidas por consulta, se quem chama não disser outro número.
pub const DEFAULT_LIMIT: usize = 1000;
/// Tempo máximo de uma consulta.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Bytes de um valor binário que se guardam para mostrar; o resto só conta no tamanho.
const BLOB_PREVIEW: usize = 32;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("o arquivo não existe: {0}")]
    NotFound(String),
    #[error("o arquivo não é um banco SQLite")]
    NotSqlite,
    #[error("o banco está em uso por outro programa (travado)")]
    Locked,
    #[error("o banco está aberto só para leitura")]
    ReadOnly,
    #[error("a consulta passou do tempo máximo e foi interrompida")]
    Timeout,
    #[error("tabela desconhecida: {0}")]
    UnknownTable(String),
    #[error("coluna desconhecida em {table}: {column}")]
    UnknownColumn { table: String, column: String },
    #[error("{0} é uma visão e não pode ser editada")]
    NotEditable(String),
    #[error("a mudança atingiria {0} linhas em vez de uma; nada foi alterado")]
    NotOneRow(usize),
    #[error("só uma instrução por vez")]
    MultipleStatements,
    #[error("usuário ou senha incorretos")]
    Auth,
    #[error("o servidor pede senha e nenhuma foi digitada")]
    PasswordRequired,
    #[error("{0}")]
    Connect(String),
    #[error("{0}")]
    Sql(String),
}

fn map_err(e: rusqlite::Error) -> Error {
    use rusqlite::Error as E;
    match &e {
        E::SqliteFailure(f, msg) => match f.code {
            ErrorCode::NotADatabase => Error::NotSqlite,
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => Error::Locked,
            ErrorCode::ReadOnly => Error::ReadOnly,
            ErrorCode::OperationInterrupted => Error::Timeout,
            _ => Error::Sql(msg.clone().unwrap_or_else(|| e.to_string())),
        },
        E::MultipleStatement => Error::MultipleStatements,
        _ => Error::Sql(e.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// Valores

/// Um valor de célula.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    /// binário: tamanho total e os primeiros bytes
    Blob { len: usize, preview: Vec<u8> },
}

impl Value {
    /// Texto para mostrar numa grade.
    pub fn display(&self) -> String {
        match self {
            Value::Null => "NULL".into(),
            Value::Integer(i) => i.to_string(),
            Value::Real(r) => r.to_string(),
            Value::Text(t) => t.clone(),
            Value::Blob { len, .. } => format!("[{len} bytes]"),
        }
    }
}

impl ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Value::Null => ToSqlOutput::Owned(rusqlite::types::Value::Null),
            Value::Integer(i) => ToSqlOutput::Owned(rusqlite::types::Value::Integer(*i)),
            Value::Real(r) => ToSqlOutput::Owned(rusqlite::types::Value::Real(*r)),
            Value::Text(t) => ToSqlOutput::Borrowed(ValueRef::Text(t.as_bytes())),
            // um binário cortado para mostrar não pode voltar para o banco
            Value::Blob { .. } => return Err(rusqlite::Error::ToSqlConversionFailure("valor binário não é editável".into())),
        })
    }
}

fn read_value(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Integer(i),
        ValueRef::Real(r) => Value::Real(r),
        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Blob { len: b.len(), preview: b[..b.len().min(BLOB_PREVIEW)].to_vec() },
    }
}

// ---------------------------------------------------------------------------------------------
// Esquema

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// o tipo como foi declarado (`INTEGER`, `TEXT`, vazio...)
    pub decl_type: String,
    pub not_null: bool,
    /// posição na chave primária (1, 2...), 0 se não faz parte dela
    pub pk: usize,
    pub default: Option<String>,
}

/// Uma chave estrangeira: estas colunas apontam para as colunas `ref_columns` de `ref_table`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKey {
    pub columns: Vec<String>,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// esquema (PostgreSQL); `None` no SQLite
    pub schema: Option<String>,
    pub name: String,
    pub is_view: bool,
    pub columns: Vec<Column>,
    pub indexes: Vec<String>,
    pub without_rowid: bool,
    pub foreign_keys: Vec<ForeignKey>,
}

/// O que identifica uma linha para editar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyKind {
    /// colunas da chave primária
    PrimaryKey(Vec<String>),
    /// sem chave primária: o `rowid` do SQLite
    Rowid,
    /// não dá para identificar uma linha (visão)
    None,
}

impl Table {
    /// Nome para mostrar: com o esquema, menos o `public`.
    pub fn label(&self) -> String {
        match self.schema.as_deref() {
            Some(s) if s != "public" => format!("{s}.{}", self.name),
            _ => self.name.clone(),
        }
    }

    /// A coluna faz parte de alguma chave estrangeira?
    pub fn is_fk(&self, column: &str) -> bool {
        self.foreign_keys.iter().any(|f| f.columns.iter().any(|c| c == column))
    }

    pub fn key(&self) -> KeyKind {
        if self.is_view {
            return KeyKind::None;
        }
        let mut pk: Vec<&Column> = self.columns.iter().filter(|c| c.pk > 0).collect();
        pk.sort_by_key(|c| c.pk);
        if !pk.is_empty() {
            KeyKind::PrimaryKey(pk.into_iter().map(|c| c.name.clone()).collect())
        } else if self.without_rowid {
            KeyKind::None
        } else {
            KeyKind::Rowid
        }
    }
}

/// Há outra instrução depois do primeiro `;`? Respeita textos entre aspas, nomes entre aspas ou colchetes, texto entre
/// `$$ ... $$` (ou `$tag$ ... $tag$`) e comentários, e ignora `;` repetidos e espaços no fim. Só serve para decidir se
/// um texto é uma instrução só (e pode ir por cursor); scripts com várias instruções rodam inteiros.
pub(crate) fn has_trailing_statement(sql: &str) -> bool {
    let b = sql.as_bytes();
    let (mut i, mut ended) = (0, false);
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' | b'[' => {
                if ended {
                    return true;
                }
                let close = if b[i] == b'[' { b']' } else { b[i] };
                i += 1;
                while i < b.len() && b[i] != close {
                    i += 1;
                }
            }
            b'$' => {
                // `$tag$` abre um texto que só fecha com o mesmo `$tag$`
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if b.get(j) == Some(&b'$') {
                    if ended {
                        return true;
                    }
                    let tag = &b[i..=j];
                    let mut k = j + 1;
                    while k + tag.len() <= b.len() && &b[k..k + tag.len()] != tag {
                        k += 1;
                    }
                    i = (k + tag.len()).min(b.len());
                    continue;
                } else if ended {
                    return true;
                }
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b';' => ended = true,
            c if c.is_ascii_whitespace() => {}
            _ => {
                if ended {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Entre aspas, para usar como nome de tabela ou coluna (só depois de conferir que existe no esquema).
pub(crate) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

// ---------------------------------------------------------------------------------------------
// Resultado de consulta

#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// havia mais linhas que o limite
    pub truncated: bool,
    pub elapsed: Duration,
    /// linhas afetadas, para instruções que não devolvem linhas (INSERT, UPDATE...)
    pub changes: Option<usize>,
}

/// Uma página de uma tabela, com a chave de cada linha para poder editá-la.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// por linha, os valores que a identificam (na ordem de [`Table::key`]; vazio se não editável)
    pub keys: Vec<Vec<Value>>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub read_only: bool,
    /// quanto esperar um banco travado por outro programa
    pub busy: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self { read_only: true, busy: Duration::from_millis(1500) }
    }
}

/// Uma mudança pedida na grade. Todas as pendentes vão juntas, numa transação: ou entram todas ou nenhuma.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// troca o valor de uma coluna da linha identificada por `key` (os valores da chave, na ordem de [`Table::key`])
    Update { key: Vec<Value>, column: String, value: Value },
    Delete { key: Vec<Value> },
    /// só as colunas preenchidas; as outras ficam com o padrão do banco
    Insert { values: Vec<(String, Value)> },
}

pub struct Database {
    conn: Connection,
    path: PathBuf,
    read_only: bool,
    busy: Duration,
}

impl Database {
    /// Abre um arquivo que já existe (nunca cria).
    pub fn open(path: &Path, opts: &Options) -> Result<Database, Error> {
        if !path.is_file() {
            return Err(Error::NotFound(path.display().to_string()));
        }
        let flags = if opts.read_only { OpenFlags::SQLITE_OPEN_READ_ONLY } else { OpenFlags::SQLITE_OPEN_READ_WRITE } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags).map_err(map_err)?;
        conn.busy_timeout(opts.busy).map_err(map_err)?;
        // lê o cabeçalho agora: se não for SQLite, falha aqui e não na primeira consulta
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).map_err(map_err)?;
        Ok(Database { conn, path: path.to_path_buf(), read_only: opts.read_only, busy: opts.busy })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Tabelas e visões (sem as internas do SQLite), em ordem de nome.
    pub fn schema(&self) -> Result<Vec<Table>, Error> {
        let mut st = self.conn.prepare("SELECT name, type, sql FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name COLLATE NOCASE").map_err(map_err)?;
        let found: Vec<(String, String, Option<String>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(map_err)?.collect::<Result<_, _>>().map_err(map_err)?;
        let mut out = Vec::new();
        for (name, kind, sql) in found {
            let q = quote(&name);
            let mut info = self.conn.prepare(&format!("PRAGMA table_info({q})")).map_err(map_err)?;
            let columns: Vec<Column> = info
                .query_map([], |r| Ok(Column { name: r.get(1)?, decl_type: r.get(2)?, not_null: r.get::<_, i64>(3)? != 0, default: r.get(4)?, pk: r.get::<_, i64>(5)? as usize }))
                .map_err(map_err)?
                .collect::<Result<_, _>>()
                .map_err(map_err)?;
            let mut il = self.conn.prepare(&format!("PRAGMA index_list({q})")).map_err(map_err)?;
            let indexes: Vec<String> = il.query_map([], |r| r.get::<_, String>(1)).map_err(map_err)?.collect::<Result<_, _>>().map_err(map_err)?;
            let without_rowid = sql.as_deref().is_some_and(|s| s.to_uppercase().contains("WITHOUT ROWID"));
            let mut fk_rows = self.conn.prepare(&format!("PRAGMA foreign_key_list({q})")).map_err(map_err)?;
            let fk_raw: Vec<(i64, String, String, String)> = fk_rows.query_map([], |r| Ok((r.get(0)?, r.get(2)?, r.get(3)?, r.get::<_, Option<String>>(4)?.unwrap_or_default()))).map_err(map_err)?.collect::<Result<_, _>>().map_err(map_err)?;
            let mut foreign_keys: Vec<ForeignKey> = Vec::new();
            let mut last_id = None;
            for (id, ref_table, from, to) in fk_raw {
                if last_id == Some(id) {
                    if let Some(f) = foreign_keys.last_mut() {
                        f.columns.push(from);
                        f.ref_columns.push(to);
                    }
                } else {
                    foreign_keys.push(ForeignKey { columns: vec![from], ref_schema: None, ref_table, ref_columns: vec![to] });
                    last_id = Some(id);
                }
            }
            out.push(Table { schema: None, name, is_view: kind == "view", columns, indexes, without_rowid, foreign_keys });
        }
        Ok(out)
    }

    fn table(&self, name: &str) -> Result<Table, Error> {
        self.schema()?.into_iter().find(|t| t.name == name).ok_or_else(|| Error::UnknownTable(name.to_string()))
    }

    pub fn count_rows(&self, table: &str) -> Result<i64, Error> {
        let t = self.table(table)?;
        self.conn.query_row(&format!("SELECT count(*) FROM {}", quote(&t.name)), [], |r| r.get(0)).map_err(map_err)
    }

    /// Roda uma instrução SQL (só uma). Devolve no máximo `limit` linhas e se interrompe depois de `timeout`.
    pub fn query(&self, sql: &str, limit: usize, timeout: Duration) -> Result<QueryResult, Error> {
        let started = Instant::now();
        let handle = self.conn.get_interrupt_handle();
        let (done_tx, done_rx) = channel::<()>();
        let watcher = std::thread::spawn(move || {
            if done_rx.recv_timeout(timeout) == Err(RecvTimeoutError::Timeout) {
                handle.interrupt();
            }
        });
        let result = self.run(sql, limit);
        drop(done_tx);
        let _ = watcher.join();
        result.map(|mut r| {
            r.elapsed = started.elapsed();
            r
        })
    }

    /// Roda um texto SQL com uma ou várias instruções (um script, com triggers incluídos). Devolve o resultado da
    /// última instrução que devolve linhas; se nenhuma devolve, o total de linhas afetadas.
    fn run(&self, sql: &str, limit: usize) -> Result<QueryResult, Error> {
        let mut batch = rusqlite::Batch::new(&self.conn, sql);
        let (mut last, mut changes, mut any) = (None::<QueryResult>, 0usize, false);
        while let Some(mut st) = batch.next().map_err(map_err)? {
            any = true;
            let cols = st.column_count();
            if cols == 0 {
                changes += st.execute([]).map_err(map_err)?;
                continue;
            }
            let columns: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
            let mut rows = st.query([]).map_err(map_err)?;
            let (mut out, mut truncated) = (Vec::new(), false);
            while let Some(r) = rows.next().map_err(map_err)? {
                if out.len() >= limit {
                    truncated = true;
                    break;
                }
                out.push((0..cols).map(|i| r.get_ref(i).map(read_value)).collect::<Result<Vec<_>, _>>().map_err(map_err)?);
            }
            last = Some(QueryResult { columns, rows: out, truncated, elapsed: Duration::ZERO, changes: None });
        }
        Ok(last.unwrap_or(QueryResult { columns: Vec::new(), rows: Vec::new(), truncated: false, elapsed: Duration::ZERO, changes: Some(if any { changes } else { 0 }) }))
    }

    /// Uma página de uma tabela ou visão, na ordem de `sort` (coluna, crescente), com a chave de cada linha.
    pub fn page(&self, table: &str, sort: Option<(&str, bool)>, offset: usize, limit: usize) -> Result<Page, Error> {
        let t = self.table(table)?;
        let key_cols: Vec<String> = match t.key() {
            KeyKind::PrimaryKey(c) => c.iter().map(|c| quote(c)).collect(),
            KeyKind::Rowid => vec!["rowid".to_string()],
            KeyKind::None => Vec::new(),
        };
        let order = match sort {
            Some((col, asc)) => {
                let c = t.columns.iter().find(|c| c.name == col).ok_or_else(|| Error::UnknownColumn { table: t.name.clone(), column: col.to_string() })?;
                format!(" ORDER BY {} {}", quote(&c.name), if asc { "ASC" } else { "DESC" })
            }
            None => String::new(),
        };
        let all: Vec<String> = t.columns.iter().map(|c| quote(&c.name)).collect();
        let select: Vec<String> = key_cols.iter().cloned().chain(all).collect();
        let sql = format!("SELECT {} FROM {}{order} LIMIT {} OFFSET {}", select.join(", "), quote(&t.name), limit + 1, offset);
        let res = self.query(&sql, limit, DEFAULT_TIMEOUT)?;
        let n = key_cols.len();
        let (mut keys, mut rows) = (Vec::new(), Vec::new());
        for mut r in res.rows {
            let data = r.split_off(n);
            keys.push(r);
            rows.push(data);
        }
        Ok(Page { columns: t.columns.iter().map(|c| c.name.clone()).collect(), rows, keys, truncated: res.truncated })
    }

    /// Passa a abrir para escrita (ou volta a só leitura) reabrindo o arquivo.
    pub fn set_read_only(&mut self, read_only: bool) -> Result<(), Error> {
        if read_only == self.read_only {
            return Ok(());
        }
        let reopened = Database::open(&self.path, &Options { read_only, busy: self.busy })?;
        *self = reopened;
        Ok(())
    }

    /// Aplica todas as mudanças numa transação; qualquer falha desfaz tudo.
    pub fn apply(&self, table: &str, changes: &[Change]) -> Result<(), Error> {
        let t = self.writable(table)?;
        let tx = self.conn.unchecked_transaction().map_err(map_err)?;
        for ch in changes {
            match ch {
                Change::Update { key, column, value } => {
                    Self::check_column(&t, column)?;
                    let sql = format!("UPDATE {} SET {} = ? WHERE {}", quote(&t.name), quote(column), Self::where_key(&t, key)?);
                    let mut params: Vec<&dyn ToSql> = vec![value];
                    params.extend(key.iter().map(|k| k as &dyn ToSql));
                    let n = tx.execute(&sql, params.as_slice()).map_err(map_err)?;
                    if n != 1 {
                        return Err(Error::NotOneRow(n));
                    }
                }
                Change::Delete { key } => {
                    let sql = format!("DELETE FROM {} WHERE {}", quote(&t.name), Self::where_key(&t, key)?);
                    let params: Vec<&dyn ToSql> = key.iter().map(|k| k as &dyn ToSql).collect();
                    let n = tx.execute(&sql, params.as_slice()).map_err(map_err)?;
                    if n != 1 {
                        return Err(Error::NotOneRow(n));
                    }
                }
                Change::Insert { values } => {
                    for (c, _) in values {
                        Self::check_column(&t, c)?;
                    }
                    let sql = if values.is_empty() {
                        format!("INSERT INTO {} DEFAULT VALUES", quote(&t.name))
                    } else {
                        let cols: Vec<String> = values.iter().map(|(c, _)| quote(c)).collect();
                        format!("INSERT INTO {} ({}) VALUES ({})", quote(&t.name), cols.join(", "), vec!["?"; values.len()].join(", "))
                    };
                    let params: Vec<&dyn ToSql> = values.iter().map(|(_, v)| v as &dyn ToSql).collect();
                    tx.execute(&sql, params.as_slice()).map_err(map_err)?;
                }
            }
        }
        tx.commit().map_err(map_err)
    }

    // ---- edição ----

    fn writable(&self, table: &str) -> Result<Table, Error> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let t = self.table(table)?;
        if t.is_view {
            return Err(Error::NotEditable(t.name));
        }
        Ok(t)
    }

    fn check_column(t: &Table, column: &str) -> Result<(), Error> {
        t.columns.iter().any(|c| c.name == column).then_some(()).ok_or_else(|| Error::UnknownColumn { table: t.name.clone(), column: column.to_string() })
    }

    /// `WHERE` que identifica a linha pela chave, com um `?` por valor.
    fn where_key(t: &Table, key: &[Value]) -> Result<String, Error> {
        let cols: Vec<String> = match t.key() {
            KeyKind::PrimaryKey(c) => c.iter().map(|c| quote(c)).collect(),
            KeyKind::Rowid => vec!["rowid".to_string()],
            KeyKind::None => return Err(Error::NotEditable(t.name.clone())),
        };
        if cols.len() != key.len() {
            return Err(Error::Sql(format!("a chave de {} tem {} valor(es), vieram {}", t.name, cols.len(), key.len())));
        }
        Ok(cols.iter().map(|c| format!("{c} IS ?")).collect::<Vec<_>>().join(" AND "))
    }

    /// Roda uma mudança que deve atingir exatamente uma linha; senão desfaz.
    fn one_row(&self, sql: &str, params: &[&dyn ToSql]) -> Result<(), Error> {
        let tx = self.conn.unchecked_transaction().map_err(map_err)?;
        let n = tx.execute(sql, params).map_err(map_err)?;
        if n != 1 {
            drop(tx); // desfaz
            return Err(Error::NotOneRow(n));
        }
        tx.commit().map_err(map_err)
    }

    pub fn update_cell(&self, table: &str, key: &[Value], column: &str, value: &Value) -> Result<(), Error> {
        let t = self.writable(table)?;
        Self::check_column(&t, column)?;
        let sql = format!("UPDATE {} SET {} = ? WHERE {}", quote(&t.name), quote(column), Self::where_key(&t, key)?);
        let mut params: Vec<&dyn ToSql> = vec![value];
        params.extend(key.iter().map(|k| k as &dyn ToSql));
        self.one_row(&sql, &params)
    }

    pub fn delete_row(&self, table: &str, key: &[Value]) -> Result<(), Error> {
        let t = self.writable(table)?;
        let sql = format!("DELETE FROM {} WHERE {}", quote(&t.name), Self::where_key(&t, key)?);
        let params: Vec<&dyn ToSql> = key.iter().map(|k| k as &dyn ToSql).collect();
        self.one_row(&sql, &params)
    }

    /// Insere uma linha com os valores dados (as demais colunas ficam com o padrão). Devolve o `rowid` novo.
    pub fn insert_row(&self, table: &str, values: &[(String, Value)]) -> Result<i64, Error> {
        let t = self.writable(table)?;
        for (c, _) in values {
            Self::check_column(&t, c)?;
        }
        let sql = if values.is_empty() {
            format!("INSERT INTO {} DEFAULT VALUES", quote(&t.name))
        } else {
            let cols: Vec<String> = values.iter().map(|(c, _)| quote(c)).collect();
            format!("INSERT INTO {} ({}) VALUES ({})", quote(&t.name), cols.join(", "), vec!["?"; values.len()].join(", "))
        };
        let params: Vec<&dyn ToSql> = values.iter().map(|(_, v)| v as &dyn ToSql).collect();
        let tx = self.conn.unchecked_transaction().map_err(map_err)?;
        tx.execute(&sql, params.as_slice()).map_err(map_err)?;
        let id = tx.last_insert_rowid();
        tx.commit().map_err(map_err)?;
        Ok(id)
    }
}

/// Um banco aberto, de qualquer tipo. O painel só fala com isto.
pub enum Db {
    Sqlite(Database),
    Postgres(PgDatabase),
}

impl Db {
    pub fn kind(&self) -> &'static str {
        match self {
            Db::Sqlite(_) => "SQLite",
            Db::Postgres(_) => "PostgreSQL",
        }
    }

    /// De onde veio, para mostrar no cabeçalho.
    pub fn label(&self) -> String {
        match self {
            Db::Sqlite(d) => d.path().display().to_string(),
            Db::Postgres(d) => d.label(),
        }
    }

    pub fn is_read_only(&self) -> bool {
        match self {
            Db::Sqlite(d) => d.is_read_only(),
            Db::Postgres(d) => !d.is_writable(),
        }
    }

    /// Liga ou desliga a escrita. Em PostgreSQL muda o padrão da sessão; em SQLite reabre o arquivo.
    pub fn set_writable(&mut self, on: bool) -> Result<(), Error> {
        match self {
            Db::Sqlite(d) => d.set_read_only(!on),
            Db::Postgres(d) => d.set_writable(on),
        }
    }

    /// Aplica as mudanças pendentes de uma tabela numa transação só.
    pub fn apply(&mut self, t: &Table, changes: &[Change]) -> Result<(), Error> {
        match self {
            Db::Sqlite(d) => d.apply(&t.name, changes),
            Db::Postgres(d) => d.apply(t, changes),
        }
    }

    pub fn schema(&mut self) -> Result<Vec<Table>, Error> {
        match self {
            Db::Sqlite(d) => d.schema(),
            Db::Postgres(d) => d.schema(),
        }
    }

    pub fn count_rows(&mut self, t: &Table) -> Result<i64, Error> {
        match self {
            Db::Sqlite(d) => d.count_rows(&t.name),
            Db::Postgres(d) => d.count_rows(t),
        }
    }

    pub fn page(&mut self, t: &Table, sort: Option<(&str, bool)>, offset: usize, limit: usize) -> Result<Page, Error> {
        match self {
            Db::Sqlite(d) => d.page(&t.name, sort, offset, limit),
            Db::Postgres(d) => d.page(t, sort, offset, limit),
        }
    }

    pub fn query(&mut self, sql: &str, limit: usize, timeout: Duration) -> Result<QueryResult, Error> {
        match self {
            Db::Sqlite(d) => d.query(sql, limit, timeout),
            Db::Postgres(d) => d.query(sql, limit, timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Banco de exemplo: chave simples, chave composta, sem chave, sem rowid e uma visão.
    fn sample() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exemplo.db");
        let c = Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE pessoas (id INTEGER PRIMARY KEY, nome TEXT NOT NULL, idade INTEGER, nota REAL, foto BLOB);
             CREATE TABLE tags (a TEXT, b TEXT, rotulo TEXT, PRIMARY KEY (a, b));
             CREATE TABLE solta (x TEXT);
             CREATE TABLE sem_rowid (k TEXT PRIMARY KEY, v TEXT) WITHOUT ROWID;
             CREATE VIEW adultos AS SELECT nome FROM pessoas WHERE idade >= 18;
             CREATE INDEX idx_nome ON pessoas(nome);
             INSERT INTO pessoas (nome, idade, nota, foto) VALUES ('Ana', 30, 8.5, x'0102030405'), ('Bia', 12, NULL, NULL), ('Caio', 41, 7.25, NULL);
             INSERT INTO tags VALUES ('x', 'y', 'um'), ('x', 'z', 'dois');
             INSERT INTO solta VALUES ('a'), ('b'), ('a');
             INSERT INTO sem_rowid VALUES ('k1', 'v1');",
        )
        .unwrap();
        (dir, path)
    }

    fn rw(path: &Path) -> Database {
        Database::open(path, &Options { read_only: false, ..Options::default() }).unwrap()
    }

    fn ro(path: &Path) -> Database {
        Database::open(path, &Options::default()).unwrap()
    }

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn schema_lists_tables_views_columns_keys_and_indexes() {
        let (_d, p) = sample();
        let s = ro(&p).schema().unwrap();
        let names: Vec<_> = s.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["adultos", "pessoas", "sem_rowid", "solta", "tags"]);
        let pessoas = s.iter().find(|t| t.name == "pessoas").unwrap();
        assert_eq!(pessoas.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "nome", "idade", "nota", "foto"]);
        assert!(pessoas.columns[1].not_null && !pessoas.columns[2].not_null);
        assert_eq!(pessoas.columns[3].decl_type, "REAL");
        assert_eq!(pessoas.indexes, ["idx_nome"]);
        assert_eq!(pessoas.key(), KeyKind::PrimaryKey(vec!["id".into()]));
        assert_eq!(s.iter().find(|t| t.name == "tags").unwrap().key(), KeyKind::PrimaryKey(vec!["a".into(), "b".into()]));
        assert_eq!(s.iter().find(|t| t.name == "solta").unwrap().key(), KeyKind::Rowid);
        assert_eq!(s.iter().find(|t| t.name == "sem_rowid").unwrap().key(), KeyKind::PrimaryKey(vec!["k".into()]));
        let v = s.iter().find(|t| t.name == "adultos").unwrap();
        assert!(v.is_view);
        assert_eq!(v.key(), KeyKind::None);
    }

    #[test]
    fn changes_are_applied_together_or_not_at_all() {
        let (_d, p) = sample();
        let db = rw(&p);
        let ok = vec![
            Change::Update { key: vec![Value::Integer(1)], column: "nome".into(), value: Value::Text("Ana Maria".into()) },
            Change::Insert { values: vec![("nome".into(), Value::Text("Dani".into()))] },
            Change::Delete { key: vec![Value::Integer(2)] },
        ];
        db.apply("pessoas", &ok).unwrap();
        let names: Vec<_> = db.query("SELECT nome FROM pessoas ORDER BY id", 10, T).unwrap().rows.into_iter().map(|r| r[0].display()).collect();
        assert_eq!(names, ["Ana Maria", "Caio", "Dani"]);
        // a terceira mudança falha (linha que não existe): as duas primeiras não podem ficar
        let bad = vec![
            Change::Update { key: vec![Value::Integer(1)], column: "nome".into(), value: Value::Text("MUDOU".into()) },
            Change::Insert { values: vec![("nome".into(), Value::Text("Fantasma".into()))] },
            Change::Delete { key: vec![Value::Integer(999)] },
        ];
        assert_eq!(db.apply("pessoas", &bad).unwrap_err(), Error::NotOneRow(0));
        let after: Vec<_> = db.query("SELECT nome FROM pessoas ORDER BY id", 10, T).unwrap().rows.into_iter().map(|r| r[0].display()).collect();
        assert_eq!(after, names);
        // coluna que não existe e violação de NOT NULL também desfazem tudo
        let unknown = vec![Change::Update { key: vec![Value::Integer(1)], column: "fantasma".into(), value: Value::Null }];
        assert!(matches!(db.apply("pessoas", &unknown), Err(Error::UnknownColumn { .. })));
        let not_null = vec![Change::Insert { values: vec![("nome".into(), Value::Text("Eva".into()))] }, Change::Insert { values: vec![("idade".into(), Value::Integer(5))] }];
        assert!(matches!(db.apply("pessoas", &not_null), Err(Error::Sql(_))));
        assert_eq!(db.count_rows("pessoas").unwrap(), 3);
    }

    #[test]
    fn writing_must_be_turned_on_and_can_be_turned_off_again() {
        let (_d, p) = sample();
        let mut db = ro(&p);
        let ch = vec![Change::Delete { key: vec![Value::Integer(1)] }];
        assert_eq!(db.apply("pessoas", &ch).unwrap_err(), Error::ReadOnly);
        db.set_read_only(false).unwrap();
        assert!(!db.is_read_only());
        db.apply("pessoas", &ch).unwrap();
        db.set_read_only(true).unwrap();
        assert_eq!(db.apply("pessoas", &[Change::Delete { key: vec![Value::Integer(2)] }]).unwrap_err(), Error::ReadOnly);
        assert_eq!(db.count_rows("pessoas").unwrap(), 2);
        assert_eq!(db.query("DELETE FROM pessoas", 10, T).unwrap_err(), Error::ReadOnly);
    }

    #[test]
    fn tables_without_a_key_are_edited_by_rowid_through_apply() {
        let (_d, p) = sample();
        let db = rw(&p);
        let page = db.page("solta", None, 0, 10).unwrap();
        db.apply("solta", &[Change::Update { key: page.keys[1].clone(), column: "x".into(), value: Value::Text("B!".into()) }]).unwrap();
        let x: Vec<_> = db.query("SELECT x FROM solta ORDER BY rowid", 10, T).unwrap().rows.into_iter().map(|r| r[0].display()).collect();
        assert_eq!(x, ["a", "B!", "a"]);
        assert_eq!(db.apply("adultos", &[Change::Delete { key: vec![] }]).unwrap_err(), Error::NotEditable("adultos".into()));
    }

    #[test]
    fn foreign_keys_are_read_including_composite_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fk.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE pai (id INTEGER PRIMARY KEY);
                 CREATE TABLE par (a TEXT, b TEXT, PRIMARY KEY (a, b));
                 CREATE TABLE filho (id INTEGER PRIMARY KEY, pai_id INTEGER REFERENCES pai(id), x TEXT, y TEXT, FOREIGN KEY (x, y) REFERENCES par(a, b));",
            )
            .unwrap();
        let s = ro(&path).schema().unwrap();
        let filho = s.iter().find(|t| t.name == "filho").unwrap();
        assert_eq!(filho.foreign_keys.len(), 2);
        let simples = filho.foreign_keys.iter().find(|f| f.ref_table == "pai").unwrap();
        assert_eq!((simples.columns.clone(), simples.ref_columns.clone()), (vec!["pai_id".to_string()], vec!["id".to_string()]));
        let composta = filho.foreign_keys.iter().find(|f| f.ref_table == "par").unwrap();
        assert_eq!((composta.columns.clone(), composta.ref_columns.clone()), (vec!["x".to_string(), "y".to_string()], vec!["a".to_string(), "b".to_string()]));
        assert!(filho.is_fk("pai_id") && filho.is_fk("y") && !filho.is_fk("id"));
        assert!(s.iter().find(|t| t.name == "pai").unwrap().foreign_keys.is_empty());
    }

    #[test]
    fn query_returns_the_five_value_types() {
        let (_d, p) = sample();
        let r = ro(&p).query("SELECT id, nome, nota, foto, NULL FROM pessoas WHERE id = 1", 10, T).unwrap();
        assert_eq!(r.columns, ["id", "nome", "nota", "foto", "NULL"]);
        assert_eq!(r.rows[0][0], Value::Integer(1));
        assert_eq!(r.rows[0][1], Value::Text("Ana".into()));
        assert_eq!(r.rows[0][2], Value::Real(8.5));
        assert_eq!(r.rows[0][3], Value::Blob { len: 5, preview: vec![1, 2, 3, 4, 5] });
        assert_eq!(r.rows[0][4], Value::Null);
        assert_eq!(r.rows[0][3].display(), "[5 bytes]");
        assert!(!r.truncated);
    }

    #[test]
    fn row_limit_cuts_and_says_so() {
        let (_d, p) = sample();
        let db = ro(&p);
        let r = db.query("SELECT id FROM pessoas ORDER BY id", 2, T).unwrap();
        assert_eq!((r.rows.len(), r.truncated), (2, true));
        let all = db.query("SELECT id FROM pessoas", 3, T).unwrap();
        assert_eq!((all.rows.len(), all.truncated), (3, false));
    }

    #[test]
    fn an_endless_query_is_interrupted() {
        let (_d, p) = sample();
        let started = Instant::now();
        let r = ro(&p).query("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT count(*) FROM c", 10, Duration::from_millis(300));
        assert_eq!(r.unwrap_err(), Error::Timeout);
        assert!(started.elapsed() < Duration::from_secs(5));
        // e a conexão continua servindo
        assert!(ro(&p).query("SELECT 1", 10, T).is_ok());
    }

    #[test]
    fn trailing_statements_are_detected_but_not_inside_strings_or_comments() {
        for ok in ["SELECT 1", "SELECT 1;", "SELECT 1;;  ", "SELECT ';'", "SELECT 1; -- fim", "SELECT 1 /* a; b */", "SELECT \"a;b\" FROM t", "SELECT [a;b] FROM t", "SELECT 1; /* nada */ "] {
            assert!(!has_trailing_statement(ok), "{ok}");
        }
        for bad in ["SELECT 1; SELECT 2", "SELECT 1;DROP TABLE x", "SELECT 1; -- c\nDELETE FROM t", "SELECT 1; 'a'"] {
            assert!(has_trailing_statement(bad), "{bad}");
        }
    }

    #[test]
    fn scripts_run_whole_and_errors_are_readable() {
        let (_d, p) = sample();
        let db = rw(&p);
        // vários comandos: roda todos e devolve o resultado da última consulta
        let r = db.query("INSERT INTO solta VALUES ('x'); INSERT INTO solta VALUES ('y'); SELECT count(*) FROM solta", 10, T).unwrap();
        assert_eq!(r.rows[0][0], Value::Integer(5));
        // sem nenhuma consulta no meio: o total de linhas afetadas
        let r = db.query("UPDATE solta SET x = 'z' WHERE x = 'x'; DELETE FROM solta WHERE x = 'y'", 10, T).unwrap();
        assert_eq!(r.changes, Some(2));
        match db.query("SELECT * FROM nao_existe", 10, T) {
            Err(Error::Sql(m)) => assert!(m.contains("nao_existe"), "{m}"),
            other => panic!("{other:?}"),
        }
        // um comando do meio que falha interrompe o script
        assert!(db.query("INSERT INTO solta VALUES ('ok'); SELECT * FROM nao_existe; INSERT INTO solta VALUES ('nunca')", 10, T).is_err());
        assert_eq!(db.query("SELECT count(*) FROM solta WHERE x = 'nunca'", 1, T).unwrap().rows[0][0], Value::Integer(0));
    }

    #[test]
    fn triggers_can_be_created_and_they_fire() {
        let (_d, p) = sample();
        let db = rw(&p);
        db.query("CREATE TABLE auditoria (msg TEXT)", 10, T).unwrap();
        // o corpo do trigger tem `;` dentro de BEGIN ... END
        db.query("CREATE TRIGGER tr_nome AFTER INSERT ON pessoas BEGIN INSERT INTO auditoria VALUES ('novo: ' || NEW.nome); UPDATE pessoas SET idade = 99 WHERE id = NEW.id; END", 10, T).unwrap();
        db.query("INSERT INTO pessoas (nome, idade) VALUES ('Dani', 20)", 10, T).unwrap();
        assert_eq!(db.query("SELECT msg FROM auditoria", 10, T).unwrap().rows[0][0], Value::Text("novo: Dani".into()));
        assert_eq!(db.query("SELECT idade FROM pessoas WHERE nome = 'Dani'", 10, T).unwrap().rows[0][0], Value::Integer(99));
        // criar o trigger e usar numa execução só
        let r = db.query("CREATE TABLE log2 (n INT); CREATE TRIGGER t2 AFTER INSERT ON log2 BEGIN INSERT INTO auditoria VALUES ('log2'); END; INSERT INTO log2 VALUES (1); SELECT count(*) FROM auditoria", 10, T).unwrap();
        assert_eq!(r.rows[0][0], Value::Integer(2));
    }

    #[test]
    fn dollar_quoted_bodies_do_not_count_as_extra_statements() {
        assert!(!has_trailing_statement("CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql"));
        assert!(!has_trailing_statement("DO $corpo$ BEGIN PERFORM 1; PERFORM 2; END $corpo$"));
        assert!(!has_trailing_statement("SELECT '$$ ; $$'"));
        assert!(has_trailing_statement("CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql; SELECT f()"));
        assert!(has_trailing_statement("SELECT $1; SELECT 2"));
        assert!(!has_trailing_statement("SELECT $1"));
    }

    #[test]
    fn read_only_blocks_every_write() {
        let (_d, p) = sample();
        let db = ro(&p);
        assert!(db.is_read_only());
        assert_eq!(db.update_cell("pessoas", &[Value::Integer(1)], "nome", &Value::Text("X".into())).unwrap_err(), Error::ReadOnly);
        assert_eq!(db.delete_row("pessoas", &[Value::Integer(1)]).unwrap_err(), Error::ReadOnly);
        assert_eq!(db.insert_row("pessoas", &[("nome".into(), Value::Text("X".into()))]).unwrap_err(), Error::ReadOnly);
        // nem por SQL escrito à mão
        assert_eq!(db.query("DELETE FROM pessoas", 10, T).unwrap_err(), Error::ReadOnly);
        assert_eq!(db.count_rows("pessoas").unwrap(), 3);
    }

    #[test]
    fn cells_can_be_edited_by_primary_key_and_rowid() {
        let (_d, p) = sample();
        let db = rw(&p);
        db.update_cell("pessoas", &[Value::Integer(2)], "idade", &Value::Integer(13)).unwrap();
        db.update_cell("pessoas", &[Value::Integer(2)], "nota", &Value::Null).unwrap();
        db.update_cell("tags", &[Value::Text("x".into()), Value::Text("z".into())], "rotulo", &Value::Text("DOIS".into())).unwrap();
        let r = db.query("SELECT idade FROM pessoas WHERE id = 2 UNION ALL SELECT rotulo FROM tags WHERE b = 'z'", 10, T).unwrap();
        assert_eq!((r.rows[0][0].clone(), r.rows[1][0].clone()), (Value::Integer(13), Value::Text("DOIS".into())));
        // tabela sem chave: pelo rowid; os valores iguais não se confundem
        let page = db.page("solta", None, 0, 10).unwrap();
        assert_eq!(page.keys.len(), 3);
        db.update_cell("solta", &page.keys[2], "x", &Value::Text("c".into())).unwrap();
        let x: Vec<_> = db.query("SELECT x FROM solta ORDER BY rowid", 10, T).unwrap().rows.into_iter().map(|r| r[0].display()).collect();
        assert_eq!(x, ["a", "b", "c"]);
    }

    #[test]
    fn a_change_that_hits_no_row_or_many_is_refused() {
        let (_d, p) = sample();
        let db = rw(&p);
        assert_eq!(db.update_cell("pessoas", &[Value::Integer(999)], "nome", &Value::Text("X".into())).unwrap_err(), Error::NotOneRow(0));
        assert_eq!(db.delete_row("pessoas", &[Value::Integer(999)]).unwrap_err(), Error::NotOneRow(0));
        // chave com NULL na tabela sem chave casa com nada (IS ? com rowid nunca é nulo)
        assert_eq!(db.count_rows("pessoas").unwrap(), 3);
    }

    #[test]
    fn rows_can_be_inserted_and_deleted() {
        let (_d, p) = sample();
        let db = rw(&p);
        let id = db.insert_row("pessoas", &[("nome".into(), Value::Text("Dani".into())), ("idade".into(), Value::Integer(22))]).unwrap();
        assert_eq!(id, 4);
        assert_eq!(db.count_rows("pessoas").unwrap(), 4);
        db.delete_row("pessoas", &[Value::Integer(4)]).unwrap();
        assert_eq!(db.count_rows("pessoas").unwrap(), 3);
        // violação de NOT NULL vira erro e não deixa linha
        assert!(matches!(db.insert_row("pessoas", &[("idade".into(), Value::Integer(1))]), Err(Error::Sql(_))));
        assert_eq!(db.count_rows("pessoas").unwrap(), 3);
    }

    #[test]
    fn views_and_unknown_names_are_not_editable() {
        let (_d, p) = sample();
        let db = rw(&p);
        assert_eq!(db.delete_row("adultos", &[]).unwrap_err(), Error::NotEditable("adultos".into()));
        assert_eq!(db.update_cell("nao_existe", &[Value::Integer(1)], "x", &Value::Null).unwrap_err(), Error::UnknownTable("nao_existe".into()));
        assert_eq!(db.update_cell("pessoas", &[Value::Integer(1)], "fantasma", &Value::Null).unwrap_err(), Error::UnknownColumn { table: "pessoas".into(), column: "fantasma".into() });
        assert!(db.page("pessoas", Some(("fantasma", true)), 0, 10).is_err());
    }

    #[test]
    fn names_and_values_cannot_inject_sql() {
        let (_d, p) = sample();
        let db = rw(&p);
        // nome de tabela malicioso: não existe no esquema, então nem chega ao SQL
        let evil = "pessoas\"; DROP TABLE pessoas; --";
        assert_eq!(db.count_rows(evil).unwrap_err(), Error::UnknownTable(evil.into()));
        assert!(db.page(evil, None, 0, 10).is_err());
        // valor malicioso: gravado como texto, sem executar nada
        let bad = "x'); DROP TABLE pessoas; --";
        db.update_cell("pessoas", &[Value::Integer(1)], "nome", &Value::Text(bad.into())).unwrap();
        assert_eq!(db.count_rows("pessoas").unwrap(), 3);
        assert_eq!(db.query("SELECT nome FROM pessoas WHERE id = 1", 1, T).unwrap().rows[0][0], Value::Text(bad.into()));
        // coluna de ordenação maliciosa
        assert!(db.page("pessoas", Some(("id; DROP TABLE pessoas", true)), 0, 10).is_err());
        assert_eq!(db.count_rows("pessoas").unwrap(), 3);
    }

    #[test]
    fn pages_are_sorted_and_carry_keys() {
        let (_d, p) = sample();
        let db = ro(&p);
        let pg = db.page("pessoas", Some(("idade", false)), 0, 2).unwrap();
        assert_eq!(pg.columns, ["id", "nome", "idade", "nota", "foto"]);
        assert_eq!(pg.rows[0][1], Value::Text("Caio".into()));
        assert_eq!(pg.keys[0], vec![Value::Integer(3)]);
        assert!(pg.truncated);
        let next = db.page("pessoas", Some(("idade", false)), 2, 2).unwrap();
        assert_eq!((next.rows.len(), next.truncated), (1, false));
        // visão: lê, mas sem chave
        let v = db.page("adultos", None, 0, 10).unwrap();
        assert_eq!(v.rows.len(), 2);
        assert!(v.keys.iter().all(|k| k.is_empty()));
        // sem rowid com chave primária
        assert_eq!(db.page("sem_rowid", None, 0, 10).unwrap().keys[0], vec![Value::Text("k1".into())]);
    }

    #[test]
    fn bad_files_are_reported_clearly() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(Database::open(&d.path().join("nada.db"), &Options::default()), Err(Error::NotFound(_))));
        let txt = d.path().join("texto.db");
        std::fs::write(&txt, "isto é um texto comum, não um banco de dados SQLite, com bytes suficientes para ter cabeçalho").unwrap();
        assert!(matches!(Database::open(&txt, &Options::default()), Err(Error::NotSqlite)));
        // abrir não cria arquivo
        assert!(!d.path().join("nada.db").exists());
    }

    #[test]
    fn a_database_locked_by_another_program_is_reported() {
        let (_d, p) = sample();
        let other = Connection::open(&p).unwrap();
        other.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let r = Database::open(&p, &Options { read_only: false, busy: Duration::from_millis(150) }).and_then(|db| db.insert_row("solta", &[("x".into(), Value::Text("n".into()))]));
        assert_eq!(r.unwrap_err(), Error::Locked);
        other.execute_batch("ROLLBACK").unwrap();
    }
}
