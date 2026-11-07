//! Painel Banco: várias conexões salvas (PostgreSQL e arquivos SQLite), tabelas, dados em grade, estrutura,
//! diagrama das relações e uma área de consulta fixa embaixo. As consultas rodam em segundo plano (a janela não
//! congela) e todo banco abre só para leitura. O motor está no crate `ox-db`; a senha nunca é gravada.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::{div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, FontWeight, MouseButton, MouseDownEvent, ScrollWheelEvent};
use ox_db::{Database, Db, Kind, Options, Page, PgConfig, PgDatabase, QueryResult, SavedConn, Store, Table, Value, DEFAULT_LIMIT, DEFAULT_TIMEOUT};

use crate::dbdiagram;
use crate::icons::wicon;
use crate::pane_view::with_alpha;
use crate::panes::{PaneKind, Rect};
use crate::theme::*;
use crate::Workspace;

/// Linhas por página na grade de dados.
pub const PAGE_SIZE: usize = 100;
const ROW_H: f32 = 28.0;
const HEAD_H: f32 = 38.0;
const BAR_H: f32 = 40.0;
const TABS_H: f32 = 40.0;
const FOOT_H: f32 = 32.0;
const SPLIT_H: f32 = 10.0;
const DOCK_H: f32 = 210.0;
/// largura média de um caractere do texto monoespaçado da grade (12 px)
const CHAR_W: f32 = 7.3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Data,
    Structure,
    Diagram,
    Result,
}

/// Formulário de uma conexão salva (nova ou em edição). Não tem campo de senha: ela é pedida só ao conectar.
pub struct EditForm {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    pub host: String,
    pub port: String,
    pub user: String,
    pub dbname: String,
    pub path: String,
    /// campo com o teclado: 0 nome; PostgreSQL: 1 servidor, 2 porta, 3 usuário, 4 banco; SQLite: 1 arquivo
    pub focus: usize,
}

impl EditForm {
    fn new() -> Self {
        Self { id: 0, name: String::new(), kind: Kind::Postgres, host: "localhost".into(), port: "5432".into(), user: "postgres".into(), dbname: "postgres".into(), path: String::new(), focus: 0 }
    }

    fn from_saved(c: &SavedConn) -> Self {
        Self { id: c.id, name: c.name.clone(), kind: c.kind, host: c.host.clone(), port: c.port.to_string(), user: c.user.clone(), dbname: c.dbname.clone(), path: c.path.clone(), focus: 0 }
    }

    fn count(&self) -> usize {
        if self.kind == Kind::Postgres {
            5
        } else {
            2
        }
    }

    fn is_port(&self) -> bool {
        self.kind == Kind::Postgres && self.focus == 2
    }

    fn field_mut(&mut self) -> &mut String {
        match (self.kind, self.focus) {
            (_, 0) => &mut self.name,
            (Kind::Postgres, 1) => &mut self.host,
            (Kind::Postgres, 2) => &mut self.port,
            (Kind::Postgres, 3) => &mut self.user,
            (Kind::Postgres, _) => &mut self.dbname,
            (Kind::Sqlite, _) => &mut self.path,
        }
    }

    /// A conexão a salvar, ou o que falta preencher.
    pub fn to_saved(&self) -> Result<SavedConn, String> {
        match self.kind {
            Kind::Postgres => {
                let port: u16 = self.port.trim().parse().map_err(|_| "a porta deve ser um número de 1 a 65535".to_string())?;
                let (host, user, dbname) = (self.host.trim(), self.user.trim(), self.dbname.trim());
                if port == 0 || host.is_empty() || user.is_empty() || dbname.is_empty() {
                    return Err("preencha servidor, porta, usuário e banco".into());
                }
                let name = if self.name.trim().is_empty() { dbname.to_string() } else { self.name.trim().to_string() };
                Ok(SavedConn { id: self.id, name, kind: Kind::Postgres, host: host.into(), port, user: user.into(), dbname: dbname.into(), path: String::new() })
            }
            Kind::Sqlite => {
                let path = self.path.trim();
                if path.is_empty() {
                    return Err("informe o caminho do arquivo".into());
                }
                let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("banco").to_string();
                let name = if self.name.trim().is_empty() { stem } else { self.name.trim().to_string() };
                Ok(SavedConn { id: self.id, name, kind: Kind::Sqlite, host: String::new(), port: 0, user: String::new(), dbname: String::new(), path: path.into() })
            }
        }
    }
}

/// Tela de conexões: o que está acontecendo com a lista.
#[derive(Default)]
pub struct Manager {
    /// conexão PostgreSQL que está pedindo a senha
    pub asking: Option<u64>,
    /// a senha digitada; só vive aqui, na memória
    pub password: String,
    pub confirm_delete: Option<u64>,
    pub edit: Option<EditForm>,
    pub connecting: bool,
}

pub struct DbPane {
    pub store: Store,
    pub manager: Manager,
    pub db: Option<Arc<Mutex<Db>>>,
    /// conexão salva que está aberta (`None`: aberta direto de um arquivo)
    pub active: Option<u64>,
    pub name: String,
    pub kind: &'static str,
    pub label: String,
    pub tables: Vec<Table>,
    pub selected: Option<usize>,
    pub view: View,
    pub page: Option<Page>,
    pub total: Option<i64>,
    pub offset: usize,
    pub sort: Option<(String, bool)>,
    pub vscroll: usize,
    pub hscroll: f32,
    pub dg_x: f32,
    pub dg_y: f32,
    pub sql: String,
    pub sql_focus: bool,
    pub result: Option<QueryResult>,
    pub running: bool,
    pub filter: String,
    pub filter_focus: bool,
    pub side_scroll: usize,
    pub error: Option<String>,
    /// muda a cada pedido; resposta de pedido antigo é descartada
    pub epoch: u64,
}

impl Default for DbPane {
    fn default() -> Self {
        Self {
            store: Store::default(),
            manager: Manager::default(),
            db: None,
            active: None,
            name: String::new(),
            kind: "",
            label: String::new(),
            tables: Vec::new(),
            selected: None,
            view: View::Data,
            page: None,
            total: None,
            offset: 0,
            sort: None,
            vscroll: 0,
            hscroll: 0.0,
            dg_x: 0.0,
            dg_y: 0.0,
            sql: "SELECT 1;".into(),
            sql_focus: false,
            result: None,
            running: false,
            filter: String::new(),
            filter_focus: false,
            side_scroll: 0,
            error: None,
            epoch: 0,
        }
    }
}

impl DbPane {
    /// Painel novo, com as conexões salvas lidas do disco.
    pub fn load() -> Self {
        let (store, err) = Store::load(&store_path());
        Self { store, error: err, ..Self::default() }
    }
}

fn store_path() -> PathBuf {
    crate::config_dir().join("databases.json")
}

/// Arquivos que o gerenciador abre no painel Banco.
pub fn is_db_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "sqlite" | "sqlite3" | "db" | "db3" | "s3db"))
}

/// Larguras das colunas (px) pelo cabeçalho e pelas primeiras linhas, entre 80 e 300.
fn col_widths(cols: &[String], rows: &[Vec<Value>]) -> Vec<f32> {
    cols.iter()
        .enumerate()
        .map(|(i, c)| {
            let longest = rows.iter().take(60).map(|r| r.get(i).map_or(0, |v| v.display().chars().take(40).count())).max().unwrap_or(0).max(c.chars().count() + 2);
            (longest as f32 * CHAR_W + 28.0).clamp(80.0, 300.0)
        })
        .collect()
}

/// Tipo abreviado para caber no cartão do diagrama (`timestamp without time zone` vira `timestamp`).
pub fn short_type(t: &str) -> String {
    let t = t.trim().to_lowercase();
    let short = match t.as_str() {
        "timestamp without time zone" => "timestamp",
        "timestamp with time zone" => "timestamptz",
        "time without time zone" => "time",
        "time with time zone" => "timetz",
        "character varying" => "varchar",
        "character" => "char",
        "double precision" => "double",
        "bigint" => "bigint",
        _ => return if t.chars().count() > 14 { format!("{}…", t.chars().take(13).collect::<String>()) } else { t },
    };
    short.to_string()
}

/// `~/x` vira o caminho dentro da pasta pessoal.
fn expand_home(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => crate::home().join(rest).display().to_string(),
        None => path.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// Destaque de SQL (para o editor da consulta)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Keyword,
    Str,
    Num,
    Comment,
}

const KEYWORDS: &[&str] = &[
    "select", "from", "where", "and", "or", "not", "in", "is", "null", "join", "left", "right", "inner", "outer", "full", "cross", "on", "as", "group", "by", "order", "having", "limit", "offset", "distinct", "union",
    "all", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "with", "case", "when", "then", "else", "end", "asc", "desc", "like", "ilike", "between", "exists", "true", "false",
    "explain", "analyze", "show", "using", "returning", "over", "partition", "primary", "key", "references", "default", "index",
];

/// Parte uma linha de SQL em pedaços com a sua cor (palavra-chave, texto entre aspas, número, comentário).
pub fn highlight(line: &str) -> Vec<(String, Tok)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, Tok)> = Vec::new();
    let push = |out: &mut Vec<(String, Tok)>, s: String, t: Tok| match out.last_mut() {
        Some((prev, pt)) if *pt == t && t == Tok::Plain => prev.push_str(&s),
        _ => out.push((s, t)),
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            push(&mut out, chars[i..].iter().collect(), Tok::Comment);
            break;
        } else if c == '\'' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '\'' {
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            push(&mut out, chars[i..end].iter().collect(), Tok::Str);
            i = end;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                j += 1;
            }
            push(&mut out, chars[i..j].iter().collect(), Tok::Num);
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let t = if KEYWORDS.contains(&word.to_lowercase().as_str()) { Tok::Keyword } else { Tok::Plain };
            push(&mut out, word, t);
            i = j;
        } else {
            push(&mut out, c.to_string(), Tok::Plain);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Conexões, abertura e consultas

impl Workspace {
    /// Abre a página do Banco (ocupa o lugar do terminal, como as Configurações).
    fn show_db_pane(&mut self) {
        self.show_db = true;
        self.show_settings = false;
        self.db.get_or_insert_with(DbPane::load);
    }

    /// Guarda um banco recém-aberto no painel e carrega a primeira tabela.
    fn adopt_db(&mut self, db: Db, tables: Vec<Table>, active: Option<u64>, name: String, cx: &mut Context<Self>) {
        let (kind, label) = (db.kind(), db.label());
        let Some(p) = self.db.as_mut() else { return };
        let store = std::mem::take(&mut p.store);
        *p = DbPane { store, db: Some(Arc::new(Mutex::new(db))), active, name, kind, label, selected: (!tables.is_empty()).then_some(0), tables, ..DbPane::default() };
        self.db_load_page(cx);
    }

    /// Fecha o banco aberto e volta à lista de conexões.
    pub fn db_back_to_manager(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.db.as_mut() {
            let (store, epoch) = (std::mem::take(&mut p.store), p.epoch + 1);
            *p = DbPane { store, epoch, ..DbPane::default() };
        }
        cx.notify();
    }

    fn open_sqlite(&mut self, path: PathBuf, active: Option<u64>, name: String, cx: &mut Context<Self>) {
        let res = Database::open(&path, &Options::default()).and_then(|d| {
            let mut db = Db::Sqlite(d);
            db.schema().map(|t| (db, t))
        });
        match res {
            Ok((db, tables)) => self.adopt_db(db, tables, active, name, cx),
            Err(e) => {
                if let Some(p) = self.db.as_mut() {
                    p.manager.connecting = false;
                    p.error = Some(e.to_string());
                }
            }
        }
        cx.notify();
    }

    /// Abre um arquivo SQLite solto (dois cliques no gerenciador de arquivos).
    pub fn open_database(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.show_db_pane();
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("banco").to_string();
        self.open_sqlite(path, None, name, cx);
    }

    /// Clicou em Conectar numa conexão salva: SQLite abre direto; PostgreSQL pede a senha.
    pub fn db_connect_saved(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let Some(conn) = p.store.get(id).cloned() else { return };
        p.error = None;
        match conn.kind {
            Kind::Sqlite => self.open_sqlite(PathBuf::from(expand_home(&conn.path)), Some(id), conn.name, cx),
            Kind::Postgres => {
                p.manager.asking = Some(id);
                p.manager.password.clear();
                p.manager.confirm_delete = None;
                p.manager.edit = None;
            }
        }
        cx.notify();
    }

    /// Enter na senha: conecta ao PostgreSQL em segundo plano.
    pub fn db_connect_with_password(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let Some(id) = p.manager.asking else { return };
        if p.manager.connecting {
            return;
        }
        let Some(conn) = p.store.get(id).cloned() else { return };
        let password = std::mem::take(&mut p.manager.password);
        let cfg = PgConfig { host: conn.host.clone(), port: conn.port, user: conn.user.clone(), password: (!password.is_empty()).then_some(password), dbname: conn.dbname.clone() };
        p.error = None;
        p.manager.connecting = true;
        p.epoch += 1;
        let epoch = p.epoch;
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_executor()
                .spawn(async move {
                    let mut db = Db::Postgres(PgDatabase::connect(&cfg)?);
                    let tables = db.schema()?;
                    Ok::<_, ox_db::Error>((db, tables))
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                match out {
                    Ok((db, tables)) => ws.adopt_db(db, tables, Some(id), conn.name.clone(), cx),
                    Err(e) => {
                        p.manager.connecting = false;
                        p.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn persist_store(&mut self) {
        let Some(p) = self.db.as_mut() else { return };
        if let Err(e) = p.store.save(&store_path()) {
            p.error = Some(format!("não consegui gravar as conexões: {e}"));
        }
    }

    pub fn db_new_conn(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.db.as_mut() {
            p.manager = Manager { edit: Some(EditForm::new()), ..Manager::default() };
            p.error = None;
        }
        cx.notify();
    }

    pub fn db_edit_conn(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(p) = self.db.as_mut() {
            if let Some(c) = p.store.get(id) {
                p.manager = Manager { edit: Some(EditForm::from_saved(c)), ..Manager::default() };
                p.error = None;
            }
        }
        cx.notify();
    }

    /// Salva o formulário na lista (nova conexão ou edição).
    pub fn db_save_form(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let Some(form) = p.manager.edit.as_ref() else { return };
        match form.to_saved() {
            Ok(conn) => {
                p.store.upsert(conn);
                p.manager = Manager::default();
                p.error = None;
                self.persist_store();
            }
            Err(e) => p.error = Some(e),
        }
        cx.notify();
    }

    pub fn db_delete_conn(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(p) = self.db.as_mut() {
            p.store.remove(id);
            p.manager = Manager::default();
        }
        self.persist_store();
        cx.notify();
    }

    /// Guarda nas conexões salvas o SQLite que foi aberto direto de um arquivo.
    pub fn db_save_current(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        if p.active.is_some() || p.kind != "SQLite" {
            return;
        }
        let id = p.store.upsert(SavedConn { id: 0, name: p.name.clone(), kind: Kind::Sqlite, host: String::new(), port: 0, user: String::new(), dbname: String::new(), path: p.label.clone() });
        p.active = Some(id);
        self.persist_store();
        cx.notify();
    }

    pub fn db_select_table(&mut self, i: usize, view: View, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        if i >= p.tables.len() {
            return;
        }
        p.selected = Some(i);
        p.view = view;
        p.offset = 0;
        p.sort = None;
        p.total = None;
        p.vscroll = 0;
        p.hscroll = 0.0;
        self.db_load_page(cx);
    }

    /// Carrega a página atual da tabela escolhida, em segundo plano.
    pub fn db_load_page(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let (Some(db), Some(t)) = (p.db.clone(), p.selected.and_then(|i| p.tables.get(i)).cloned()) else {
            p.page = None;
            return;
        };
        p.epoch += 1;
        let epoch = p.epoch;
        let (sort, offset, need_total) = (p.sort.clone(), p.offset, p.total.is_none());
        p.error = None;
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_executor()
                .spawn(async move {
                    let mut db = db.lock().unwrap();
                    let page = db.page(&t, sort.as_ref().map(|(c, a)| (c.as_str(), *a)), offset, PAGE_SIZE)?;
                    let total = if need_total { Some(db.count_rows(&t)?) } else { None };
                    Ok::<_, ox_db::Error>((page, total))
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                match out {
                    Ok((page, total)) => {
                        p.page = Some(page);
                        p.total = total.or(p.total);
                        p.vscroll = 0;
                    }
                    Err(e) => {
                        p.page = None;
                        p.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Roda o SQL do editor, em segundo plano; o resultado aparece na aba Resultado.
    pub fn db_run_sql(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let Some(db) = p.db.clone() else { return };
        if p.running {
            return;
        }
        p.running = true;
        p.error = None;
        p.epoch += 1;
        let (epoch, sql) = (p.epoch, p.sql.clone());
        cx.spawn(async move |this, cx| {
            let out = cx.background_executor().spawn(async move { db.lock().unwrap().query(&sql, DEFAULT_LIMIT, DEFAULT_TIMEOUT) }).await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                p.running = false;
                match out {
                    Ok(r) => {
                        p.result = Some(r);
                        p.view = View::Result;
                        p.vscroll = 0;
                        p.hscroll = 0.0;
                    }
                    Err(e) => p.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Teclas com o painel Banco em foco.
    pub fn on_db_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.to_lowercase();
        let (ctrl, alt) = (k.modifiers.control, k.modifiers.alt);
        let Some(p) = self.db.as_mut() else { return false };

        if p.db.is_none() {
            // formulário de conexão: Tab troca de campo, Enter salva, Esc cancela
            if let Some(f) = p.manager.edit.as_mut() {
                match key.as_str() {
                    "enter" => {
                        self.db_save_form(cx);
                        return true;
                    }
                    "escape" => p.manager.edit = None,
                    "tab" => {
                        let n = f.count();
                        f.focus = if k.modifiers.shift { (f.focus + n - 1) % n } else { (f.focus + 1) % n };
                    }
                    "backspace" => {
                        f.field_mut().pop();
                    }
                    _ if !ctrl && !alt => match (&k.key_char, key.as_str()) {
                        (Some(c), _) if f.is_port() && !c.chars().all(|ch| ch.is_ascii_digit()) => return true,
                        (Some(c), _) => f.field_mut().push_str(c),
                        (None, "space") if !f.is_port() => f.field_mut().push(' '),
                        _ => return false,
                    },
                    _ => return false,
                }
                cx.notify();
                return true;
            }
            // senha da conexão PostgreSQL
            if p.manager.asking.is_some() {
                match key.as_str() {
                    "enter" => {
                        self.db_connect_with_password(cx);
                        return true;
                    }
                    "escape" => {
                        p.manager.asking = None;
                        p.manager.password.clear();
                    }
                    "backspace" => {
                        p.manager.password.pop();
                    }
                    _ if !ctrl && !alt => match (&k.key_char, key.as_str()) {
                        (Some(c), _) => p.manager.password.push_str(c),
                        (None, "space") => p.manager.password.push(' '),
                        _ => return false,
                    },
                    _ => return false,
                }
                cx.notify();
                return true;
            }
            if key == "escape" {
                // primeiro desfaz a confirmação de exclusão; só depois fecha a página
                if p.manager.confirm_delete.take().is_none() {
                    self.show_db = false;
                }
                cx.notify();
                return true;
            }
            return false;
        }

        if p.filter_focus {
            match key.as_str() {
                "escape" | "enter" => p.filter_focus = false,
                "backspace" => {
                    p.filter.pop();
                    p.side_scroll = 0;
                }
                _ if !ctrl && !alt => match &k.key_char {
                    Some(c) => {
                        p.filter.push_str(c);
                        p.side_scroll = 0;
                    }
                    None => return false,
                },
                _ => return false,
            }
            cx.notify();
            return true;
        }
        if p.sql_focus {
            match key.as_str() {
                "enter" if ctrl => {
                    self.db_run_sql(cx);
                    return true;
                }
                "escape" => p.sql_focus = false,
                "enter" => p.sql.push('\n'),
                "backspace" => {
                    p.sql.pop();
                }
                "tab" => p.sql.push_str("  "),
                _ if !ctrl && !alt => match (&k.key_char, key.as_str()) {
                    (Some(c), _) => p.sql.push_str(c),
                    (None, "space") => p.sql.push(' '),
                    _ => return false,
                },
                _ => return false,
            }
            cx.notify();
            return true;
        }
        let pages = p.total.map_or(usize::MAX, |t| t as usize);
        match key.as_str() {
            "pagedown" if p.view == View::Data && p.offset + PAGE_SIZE < pages => {
                p.offset += PAGE_SIZE;
                self.db_load_page(cx);
            }
            "pageup" if p.view == View::Data && p.offset > 0 => {
                p.offset = p.offset.saturating_sub(PAGE_SIZE);
                self.db_load_page(cx);
            }
            "down" => p.vscroll += 1,
            "up" => p.vscroll = p.vscroll.saturating_sub(1),
            "escape" => self.show_db = false,
            _ => return false,
        }
        cx.notify();
        true
    }
}

// ---------------------------------------------------------------------------------------------
// Desenho

const AMBER: u32 = 0xe9b872;
const BLUE: u32 = 0x7fb4e8;

impl Workspace {
    pub fn render_db_pane(&mut self, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let open = self.db.as_ref().is_some_and(|p| p.db.is_some());
        if open {
            self.render_db_open(r, cx)
        } else {
            self.render_db_manager(r, cx)
        }
    }

    /// Grade de resultados: cabeçalho fixo, linhas com rolagem vertical e horizontal. `sortable`: clicar no cabeçalho ordena.
    fn db_grid(&self, id: &'static str, cols: &[String], rows: &[Vec<Value>], types: &[String], view: (f32, f32), sortable: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = self.db.as_ref().expect("painel aberto");
        let mono = self.mono.clone();
        let accent = self.focus_color();
        let widths = col_widths(cols, rows);
        let total_w: f32 = widths.iter().sum();
        let visible = ((view.1 - HEAD_H) / ROW_H).floor().max(1.0) as usize;
        let max_scroll = rows.len().saturating_sub(visible);
        let first = p.vscroll.min(max_scroll);
        let hscroll = p.hscroll.clamp(0.0, (total_w - view.0).max(0.0));
        let sort = p.sort.clone();

        let header = div().flex_none().h(px(HEAD_H)).flex().items_center().ml(px(-hscroll)).bg(rgb(c_panel())).border_b_1().border_color(rgb(c_card_border())).children(cols.iter().enumerate().map(|(i, c)| {
            let (arrow, active) = match &sort {
                Some((s, true)) if sortable && s == c => (" ↑", true),
                Some((s, false)) if sortable && s == c => (" ↓", true),
                _ => ("", false),
            };
            let name = c.clone();
            let ty = types.get(i).cloned().unwrap_or_default();
            div()
                .id((id, i))
                .flex_none()
                .w(px(widths[i]))
                .h_full()
                .px(px(12.))
                .flex()
                .flex_col()
                .justify_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .when(i > 0, |d| d.border_l_1().border_color(rgb(c_divider())))
                .when(sortable, |d| {
                    d.cursor_pointer().hover(|s| s.bg(rgb(c_hover()))).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(p) = this.db.as_mut() {
                            p.sort = match p.sort.take() {
                                Some((s, true)) if s == name => Some((s, false)),
                                Some((s, false)) if s == name => None,
                                _ => Some((name.clone(), true)),
                            };
                            p.offset = 0;
                        }
                        this.db_load_page(cx);
                    }))
                })
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(if active { accent } else { c_text() })).child(format!("{c}{arrow}")))
                .when(!ty.is_empty(), |d| d.child(div().font_family(mono.clone()).text_size(px(10.)).text_color(rgb(c_muted())).child(ty)))
        }));

        let body = (first..(first + visible).min(rows.len())).map(|ri| {
            div()
                .flex_none()
                .h(px(ROW_H))
                .flex()
                .items_center()
                .ml(px(-hscroll))
                .when(ri % 2 == 1, |d| d.bg(rgb(c_code_bg())))
                .children(rows[ri].iter().enumerate().map(|(i, v)| {
                    let muted = matches!(v, Value::Null | Value::Blob { .. });
                    div()
                        .flex_none()
                        .w(px(widths.get(i).copied().unwrap_or(80.0)))
                        .px(px(12.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .font_family(mono.clone())
                        .text_color(rgb(if muted { c_muted() } else { c_text() }))
                        .child(v.display().replace('\n', "↵"))
                }))
        });

        div()
            .id((id, usize::MAX))
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                if let Some(p) = this.db.as_mut() {
                    let d = ev.delta.pixel_delta(px(ROW_H));
                    let (dx, dy) = (f32::from(d.x), f32::from(d.y));
                    if dx.abs() > dy.abs() || ev.modifiers.shift {
                        let amount = if dx.abs() > dy.abs() { dx } else { dy };
                        p.hscroll = (p.hscroll - amount).clamp(0.0, 100_000.0);
                    } else {
                        p.vscroll = (p.vscroll as i64 - (dy / ROW_H).round() as i64).clamp(0, max_scroll as i64) as usize;
                    }
                }
                cx.notify();
            }))
            .child(header)
            .children(body)
            .into_any_element()
    }

    fn db_btn(&self, id: impl Into<gpui::ElementId>, label: &str, primary: bool) -> gpui::Stateful<gpui::Div> {
        let accent = self.focus_color();
        div()
            .id(id)
            .flex_none()
            .h(px(28.))
            .px(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .text_size(px(12.))
            .cursor_pointer()
            .when(primary, |d| d.bg(rgb(accent)).text_color(rgb(c_bg())).font_weight(FontWeight::BOLD).hover(|s| s.opacity(0.85)))
            .when(!primary, |d| d.border_1().border_color(rgb(c_card_border())).text_color(rgb(c_text())).hover(|s| s.bg(rgb(c_hover()))))
            .child(label.to_string())
    }

    // ----- lista de conexões -----

    fn render_db_manager(&mut self, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let mono = self.mono.clone();
        let Some(p) = self.db.as_ref() else { return div().into_any_element() };
        let (asking, confirm, connecting) = (p.manager.asking, p.manager.confirm_delete, p.manager.connecting);
        let dots = "•".repeat(p.manager.password.chars().count());
        let error = p.error.clone();
        let conns: Vec<SavedConn> = p.store.conns.clone();
        let scroll = p.dg_y.clamp(0.0, 4000.0);
        let edit = p.manager.edit.as_ref().map(|f| (f.id, f.kind, f.focus, f.name.clone(), f.host.clone(), f.port.clone(), f.user.clone(), f.dbname.clone(), f.path.clone()));
        let width = (r.w - 64.0).clamp(240.0, 1100.0);

        // um campo de texto do formulário; clicar nele dá o foco do teclado
        let field = |label: &'static str, value: String, idx: usize, focus: usize, cx: &mut Context<Self>| {
            let on = focus == idx;
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child(label))
                .child(
                    div()
                        .id(("dbf", idx))
                        .h(px(30.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(if on { accent } else { c_card_border() }))
                        .bg(rgb(c_panel()))
                        .font_family(mono.clone())
                        .text_size(px(12.))
                        .text_color(rgb(c_text()))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(format!("{value}{}", if on { "▏" } else { "" }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.focused_pane = Some(PaneKind::Db);
                            if let Some(f) = this.db.as_mut().and_then(|p| p.manager.edit.as_mut()) {
                                f.focus = idx;
                            }
                            cx.notify();
                        })),
                )
        };

        let mut list = div().w(px(width)).flex().flex_col().gap(px(12.)).mt(px(-scroll));
        list = list.child(
            div()
                .flex_none()
                .flex()
                .items_end()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).text_color(rgb(c_text())).child("Conexões"))
                        .child(div().mt(px(4.)).text_size(px(12.)).text_color(rgb(c_muted())).child("As conexões ficam salvas neste computador, sem a senha.")),
                )
                .child(self.db_btn("db-new", "Nova conexão", true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_new_conn(cx)))),
        );
        if let Some(e) = error {
            list = list.child(div().text_size(px(12.)).text_color(rgb(RED)).child(e));
        }

        // formulário de nova conexão ou edição
        if let Some((id, kind, focus, name, host, port, user, dbname, path)) = edit {
            let seg = |sid: &'static str, label: &'static str, k: Kind, cx: &mut Context<Self>| {
                let on = kind == k;
                div()
                    .id(sid)
                    .flex_1()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgb(c_surface())).text_color(rgb(accent)))
                    .when(!on, |d| d.text_color(rgb(c_muted())).hover(|s| s.bg(rgb(c_hover()))))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(f) = this.db.as_mut().and_then(|p| p.manager.edit.as_mut()) {
                            f.kind = k;
                            f.focus = 0;
                        }
                        cx.notify();
                    }))
            };
            let mut form = div().flex_none().flex().flex_col().gap(px(10.)).p(px(16.)).rounded_lg().bg(rgb(c_panel())).border_1().border_color(rgb(accent));
            form = form.child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).text_color(rgb(c_text())).child(if id == 0 { "Nova conexão" } else { "Editar conexão" }));
            form = form.child(div().flex().gap_1().p(px(2.)).rounded_md().bg(rgb(c_code_bg())).child(seg("db-k-pg", "PostgreSQL", Kind::Postgres, cx)).child(seg("db-k-sq", "Arquivo SQLite", Kind::Sqlite, cx)));
            form = form.child(div().flex().child(field("Nome (opcional)", name, 0, focus, cx)));
            if kind == Kind::Postgres {
                form = form
                    .child(div().flex().gap_2().child(field("Servidor", host, 1, focus, cx)).child(div().flex_none().w(px(80.)).flex().child(field("Porta", port, 2, focus, cx))))
                    .child(div().flex().gap_2().child(field("Usuário", user, 3, focus, cx)).child(field("Banco", dbname, 4, focus, cx)))
                    .child(div().text_size(px(11.)).text_color(rgb(c_disabled())).child("Servidor remoto: por enquanto use um túnel SSH (ssh -L) e conecte em localhost."));
            } else {
                form = form.child(div().flex().child(field("Arquivo", path, 1, focus, cx)));
            }
            form = form.child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.db_btn("db-save", "Salvar", true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_save_form(cx))))
                    .child(self.db_btn("db-cancel", "Cancelar", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(p) = this.db.as_mut() {
                            p.manager.edit = None;
                            p.error = None;
                        }
                        cx.notify();
                    }))),
            );
            list = list.child(form);
        }

        // tabela simples: uma linha por conexão
        let cell = |w: Option<f32>| {
            let d = div().px(px(12.)).flex().items_center().overflow_hidden().whitespace_nowrap().text_size(px(12.));
            match w {
                Some(w) => d.flex_none().w(px(w)),
                None => d.flex_1().min_w_0(),
            }
        };
        let mut table = div().flex_none().flex().flex_col().rounded_md().border_1().border_color(rgb(c_card_border())).overflow_hidden();
        table = table.child(
            div()
                .flex_none()
                .h(px(32.))
                .flex()
                .items_center()
                .bg(rgb(c_panel()))
                .border_b_1()
                .border_color(rgb(c_card_border()))
                .text_color(rgb(c_muted()))
                .font_weight(FontWeight::SEMIBOLD)
                .child(cell(None).child("Nome"))
                .child(cell(Some(110.0)).child("Tipo"))
                .child(cell(None).child("Endereço"))
                .child(cell(Some(262.0)).child("")),
        );
        for (n, c) in conns.iter().enumerate() {
            let id = c.id;
            let kind_label = if c.kind == Kind::Postgres { "PostgreSQL" } else { "SQLite" };
            let go_label = if c.kind == Kind::Sqlite { "Abrir" } else { "Conectar" };
            table = table.child(
                div()
                    .flex_none()
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(c_divider()))
                    .when(n % 2 == 1, |d| d.bg(rgb(c_code_bg())))
                    .child(cell(None).font_weight(FontWeight::MEDIUM).text_color(rgb(c_text())).child(c.name.clone()))
                    .child(cell(Some(110.0)).text_color(rgb(c_muted())).child(kind_label))
                    .child(cell(None).font_family(mono.clone()).text_color(rgb(c_muted())).child(c.address()))
                    .child(
                        cell(Some(262.0))
                            .justify_end()
                            .gap_2()
                            .child(self.db_btn(("db-go", n), go_label, false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.db_connect_saved(id, cx))))
                            .child(self.db_btn(("db-edit", n), "Editar", false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.db_edit_conn(id, cx))))
                            .child(self.db_btn(("db-del", n), "Excluir", false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if let Some(p) = this.db.as_mut() {
                                    p.manager = Manager { confirm_delete: Some(id), ..Manager::default() };
                                }
                                cx.notify();
                            }))),
                    ),
            );
            if asking == Some(id) {
                table = table.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .px(px(12.))
                        .py(px(10.))
                        .bg(rgb(c_panel()))
                        .border_b_1()
                        .border_color(rgb(c_divider()))
                        .child(div().flex_none().text_size(px(12.)).text_color(rgb(c_text())).child(format!("Senha de {}:", c.user)))
                        .child(div().w(px(220.)).h(px(28.)).px(px(10.)).flex().items_center().rounded(px(5.)).border_1().border_color(rgb(accent)).bg(rgb(c_bg())).font_family(mono.clone()).text_size(px(12.)).text_color(rgb(c_text())).child(format!("{dots}▏")))
                        .child(self.db_btn("db-go-pw", if connecting { "Conectando…" } else { "Conectar" }, true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_connect_with_password(cx))))
                        .child(self.db_btn("db-ask-no", "Cancelar", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(p) = this.db.as_mut() {
                                p.manager.asking = None;
                                p.manager.password.clear();
                            }
                            cx.notify();
                        })))
                        .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child("A senha fica só na memória.")),
                );
            }
            if confirm == Some(id) {
                table = table.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .px(px(12.))
                        .py(px(10.))
                        .bg(with_alpha(RED, 0x1f))
                        .border_b_1()
                        .border_color(rgb(c_divider()))
                        .child(div().flex_none().text_size(px(12.)).text_color(rgb(c_text())).child(format!("Excluir a conexão “{}”? {}", c.name, if c.kind == Kind::Sqlite { "O arquivo não é apagado." } else { "O banco não é mexido." })))
                        .child(self.db_btn("db-del-yes", "Excluir", true).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.db_delete_conn(id, cx))))
                        .child(self.db_btn("db-del-no", "Manter", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(p) = this.db.as_mut() {
                                p.manager.confirm_delete = None;
                            }
                            cx.notify();
                        }))),
                );
            }
        }
        if conns.is_empty() {
            table = table.child(div().p(px(16.)).text_size(px(12.)).text_color(rgb(c_muted())).child("Nenhuma conexão salva. Crie uma em “Nova conexão”, ou dê dois cliques num arquivo .sqlite no gerenciador de arquivos."));
        }
        list = list.child(table);

        div()
            .id("db-manager")
            .size_full()
            .flex()
            .overflow_hidden()
            .px(px(32.))
            .pt(px(24.))
            .bg(rgb(c_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.focused_pane = Some(PaneKind::Db);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                if let Some(p) = this.db.as_mut() {
                    p.dg_y = (p.dg_y - f32::from(ev.delta.pixel_delta(px(24.)).y)).clamp(0.0, 4000.0);
                }
                cx.notify();
            }))
            .child(list)
            .into_any_element()
    }
}

impl Workspace {
    fn db_tab(&self, id: &'static str, label: String, on: bool, view: View, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let accent = self.focus_color();
        div()
            .id(id)
            .flex_none()
            .h(px(28.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(12.))
            .cursor_pointer()
            .font_weight(FontWeight::MEDIUM)
            .when(on, |d| d.bg(rgb(c_surface())).text_color(rgb(accent)))
            .when(!on, |d| d.text_color(rgb(c_muted())).hover(|s| s.bg(rgb(c_hover()))))
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(p) = this.db.as_mut() {
                    p.view = view;
                    p.vscroll = 0;
                    p.hscroll = 0.0;
                }
                cx.notify();
            }))
    }

    // ----- banco aberto -----

    fn render_db_open(&mut self, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let mono = self.mono.clone();
        let p = self.db.as_ref().expect("painel aberto");
        let (view, selected, error) = (p.view, p.selected, p.error.clone());
        let side_w = if r.w < 620.0 { 156.0 } else { 224.0 };
        let main_w = r.w - side_w;
        let content_h = (r.h - BAR_H - TABS_H - FOOT_H - SPLIT_H - DOCK_H).max(90.0);
        let can_save = p.active.is_none() && p.kind == "SQLite";
        let (name, label, kind_label) = (p.name.clone(), p.label.clone(), p.kind);
        let filter = p.filter.clone();
        let filter_focus = p.filter_focus;
        let side_scroll = p.side_scroll;
        let tables: Vec<(usize, String, bool, usize)> = p.tables.iter().enumerate().filter(|(_, t)| filter.is_empty() || t.label().to_lowercase().contains(&filter.to_lowercase())).map(|(i, t)| (i, t.label(), t.is_view, t.columns.len())).collect();
        let n_tables = p.tables.iter().filter(|t| !t.is_view).count();
        let n_views = p.tables.len() - n_tables;
        let n_rel: usize = p.tables.iter().filter(|t| !t.is_view).map(|t| t.foreign_keys.len()).sum();
        let result_rows = p.result.as_ref().map(|x| x.rows.len());

        // barra de cima
        let mut bar = div()
            .flex_none()
            .h(px(BAR_H))
            .px(px(12.))
            .flex()
            .items_center()
            .gap_2()
            .bg(rgb(c_panel()))
            .border_b_1()
            .border_color(rgb(c_card_border()))
            .child(wicon("database", 16., accent))
            .child(div().flex_none().text_size(px(13.)).font_weight(FontWeight::BOLD).text_color(rgb(c_text())).child(name))
            .child(div().flex_1().min_w_0().truncate().font_family(mono.clone()).text_size(px(11.)).text_color(rgb(c_muted())).child(label))
            .child(div().flex_none().h(px(20.)).px(px(8.)).flex().items_center().rounded_full().bg(rgb(c_surface())).border_1().border_color(rgb(c_card_border())).text_size(px(10.)).text_color(rgb(c_muted())).child(format!("{kind_label} · só leitura")));
        if can_save {
            bar = bar.child(self.db_btn("db-save-conn", "Salvar conexão", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_save_current(cx))));
        }
        bar = bar.child(self.db_btn("db-switch", "Trocar de banco", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_back_to_manager(cx))));

        // lista de tabelas
        let visible_rows = ((r.h - BAR_H - 120.0) / 30.0).floor().max(3.0) as usize;
        let max_side = tables.len().saturating_sub(visible_rows);
        let first_side = side_scroll.min(max_side);
        let filter_box = div()
            .id("db-filter")
            .flex_none()
            .h(px(30.))
            .mx(px(10.))
            .mt(px(10.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .bg(rgb(c_bg()))
            .border_1()
            .border_color(rgb(if filter_focus { accent } else { c_card_border() }))
            .child(wicon("search", 12., c_muted()))
            .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(rgb(if filter.is_empty() { c_muted() } else { c_text() })).child(if filter.is_empty() && !filter_focus { "Filtrar tabelas".to_string() } else { format!("{filter}{}", if filter_focus { "▏" } else { "" }) }))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.focused_pane = Some(PaneKind::Db);
                if let Some(p) = this.db.as_mut() {
                    p.filter_focus = true;
                    p.sql_focus = false;
                }
                cx.notify();
            }));
        let list = div()
            .id("db-tables")
            .flex_1()
            .min_h_0()
            .mt(px(6.))
            .px(px(6.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                if let Some(p) = this.db.as_mut() {
                    let dy = f32::from(ev.delta.pixel_delta(px(28.)).y);
                    p.side_scroll = (p.side_scroll as i64 - (dy / 28.0).round() as i64).clamp(0, max_side as i64) as usize;
                }
                cx.notify();
            }))
            .children(tables.iter().skip(first_side).take(visible_rows + 1).map(|(i, label, is_view, ncols)| {
                let i = *i;
                let on = selected == Some(i);
                div()
                    .id(("db-table", i))
                    .flex_none()
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .when(on, |d| d.bg(with_alpha(accent, 0x30)))
                    .when(!on, |d| d.hover(|s| s.bg(rgb(c_hover()))))
                    .child(wicon(if *is_view { "eye" } else { "block-bulletedlist" }, 12., if on { accent } else { c_muted() }))
                    .child(div().flex_1().min_w_0().truncate().text_color(rgb(if on { accent } else { c_text() })).child(label.clone()))
                    .child(div().flex_none().font_family(mono.clone()).text_size(px(10.)).text_color(rgb(c_muted())).child(format!("{ncols}")))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.focused_pane = Some(PaneKind::Db);
                        let keep = this.db.as_ref().map(|p| p.view).filter(|v| matches!(v, View::Structure | View::Diagram)).unwrap_or(View::Data);
                        this.db_select_table(i, keep, cx);
                    }))
            }));
        let sidebar = div().flex_none().w(px(side_w)).h_full().flex().flex_col().bg(rgb(c_panel())).border_r_1().border_color(rgb(c_card_border())).child(filter_box).child(list).child(
            div().flex_none().h(px(30.)).px(px(12.)).flex().items_center().text_size(px(11.)).text_color(rgb(c_muted())).child(format!("{n_tables} tabelas · {n_views} visões")),
        );

        // abas
        let mut tabs = div()
            .flex_none()
            .h(px(TABS_H))
            .px(px(10.))
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(rgb(c_card_border()))
            .child(self.db_tab("db-tab-data", "Dados".into(), view == View::Data, View::Data, cx))
            .child(self.db_tab("db-tab-struct", "Estrutura".into(), view == View::Structure, View::Structure, cx))
            .child(self.db_tab("db-tab-diag", "Diagrama".into(), view == View::Diagram, View::Diagram, cx));
        if let Some(n) = result_rows {
            tabs = tabs
                .child(self.db_tab("db-tab-res", format!("Resultado · {n}"), view == View::Result, View::Result, cx))
                .child(
                    div()
                        .id("db-res-close")
                        .flex_none()
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_size(px(14.))
                        .text_color(rgb(c_muted()))
                        .hover(|s| s.bg(rgb(c_hover())))
                        .child("×")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if let Some(p) = this.db.as_mut() {
                                p.result = None;
                                p.view = View::Data;
                            }
                            cx.notify();
                        })),
                );
        }

        // conteúdo e rodapé de cada aba
        let area = (main_w, content_h);
        let mut footer_left = String::new();
        let mut footer_nav: Option<(bool, bool)> = None;
        let content: AnyElement = match view {
            View::Data => {
                let (page, total, offset) = {
                    let p = self.db.as_ref().unwrap();
                    (p.page.clone(), p.total, p.offset)
                };
                match page {
                    Some(pg) => {
                        let types: Vec<String> = selected.and_then(|i| self.db.as_ref().unwrap().tables.get(i)).map(|t| t.columns.iter().map(|c| c.decl_type.clone()).collect()).unwrap_or_default();
                        let from = if pg.rows.is_empty() { 0 } else { offset + 1 };
                        let to = offset + pg.rows.len();
                        footer_left = match total {
                            Some(t) => format!("{from}–{to} de {t} linhas"),
                            None => format!("{from}–{to}"),
                        };
                        footer_nav = Some((offset > 0, pg.truncated));
                        self.db_grid("db-data", &pg.columns, &pg.rows, &types, area, true, cx)
                    }
                    None => div().flex_1().p_3().text_size(px(12.)).text_color(rgb(if error.is_some() { RED } else { c_muted() })).child(error.clone().unwrap_or_else(|| if selected.is_some() { "Carregando…".into() } else { "O banco não tem tabelas.".into() })).into_any_element(),
                }
            }
            View::Structure => {
                let p = self.db.as_ref().unwrap();
                match selected.and_then(|i| p.tables.get(i)) {
                    Some(t) => {
                        let cols = ["Coluna", "Tipo", "Aceita nulo", "Padrão", "Chave"].map(String::from).to_vec();
                        let rows: Vec<Vec<Value>> = t
                            .columns
                            .iter()
                            .map(|c| {
                                let mut key: Vec<String> = Vec::new();
                                if c.pk > 0 {
                                    key.push("PK".into());
                                }
                                for f in t.foreign_keys.iter().filter(|f| f.columns.contains(&c.name)) {
                                    let at = f.columns.iter().position(|x| *x == c.name).unwrap_or(0);
                                    key.push(format!("FK → {}.{}", f.ref_table, f.ref_columns.get(at).cloned().unwrap_or_default()));
                                }
                                vec![Value::Text(c.name.clone()), Value::Text(c.decl_type.clone()), Value::Text(if c.not_null { "não" } else { "sim" }.into()), c.default.clone().map_or(Value::Null, Value::Text), if key.is_empty() { Value::Null } else { Value::Text(key.join(", ")) }]
                            })
                            .collect();
                        footer_left = if t.indexes.is_empty() { "Sem índices".to_string() } else { format!("Índices: {}", t.indexes.join(", ")) };
                        self.db_grid("db-struct", &cols, &rows, &[], area, false, cx)
                    }
                    None => div().flex_1().p_3().text_size(px(12.)).text_color(rgb(c_muted())).child("Escolha uma tabela.").into_any_element(),
                }
            }
            View::Diagram => {
                footer_left = format!("{n_tables} tabelas · {n_rel} relações");
                self.render_db_diagram(area, cx)
            }
            View::Result => {
                let res = self.db.as_ref().unwrap().result.clone();
                match res {
                    Some(res) if !res.columns.is_empty() => {
                        footer_left = format!("{} linhas em {} ms{}", res.rows.len(), res.elapsed.as_millis(), if res.truncated { format!(" (cortado em {DEFAULT_LIMIT})") } else { String::new() });
                        self.db_grid("db-result", &res.columns, &res.rows, &[], area, false, cx)
                    }
                    Some(res) => {
                        footer_left = format!("{} ms", res.elapsed.as_millis());
                        div().flex_1().p_3().text_size(px(12.)).text_color(rgb(c_text())).child(format!("{} linha(s) afetada(s)", res.changes.unwrap_or(0))).into_any_element()
                    }
                    None => div().flex_1().into_any_element(),
                }
            }
        };

        let nav = |id: &'static str, glyph: &'static str, on: bool, delta: i64, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .when(on, |d| d.cursor_pointer().hover(|s| s.bg(rgb(c_hover()))))
                .child(wicon(glyph, 13., if on { c_text() } else { c_disabled() }))
                .when(on, |d| {
                    d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(p) = this.db.as_mut() {
                            p.offset = (p.offset as i64 + delta * PAGE_SIZE as i64).max(0) as usize;
                        }
                        this.db_load_page(cx);
                    }))
                })
        };
        let mut footer = div().flex_none().h(px(FOOT_H)).px(px(10.)).flex().items_center().gap_2().border_t_1().border_color(rgb(c_card_border())).bg(rgb(c_panel()));
        if let Some((prev, next)) = footer_nav {
            footer = footer.child(nav("db-prev", "arrow-left", prev, -1, cx)).child(nav("db-next", "arrow-right", next, 1, cx));
        }
        footer = footer.child(div().flex_1().min_w_0().truncate().text_size(px(11.)).text_color(rgb(c_muted())).child(footer_left));

        // divisória e consulta embaixo
        let split = div().flex_none().h(px(SPLIT_H)).flex().items_center().justify_center().gap(px(3.)).bg(rgb(c_bg())).border_t_1().border_color(rgb(c_card_border())).children((0..3).map(|_| div().size(px(3.)).rounded_full().bg(rgb(c_disabled()))));
        let dock = self.render_db_dock(main_w > 520.0, cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(c_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.focused_pane = Some(PaneKind::Db);
                    if let Some(p) = this.db.as_mut() {
                        p.filter_focus = false;
                    }
                    cx.notify();
                }),
            )
            .child(bar)
            .child(div().flex_1().min_h_0().flex().child(sidebar).child(div().flex_1().min_w_0().flex().flex_col().child(tabs).child(content).child(footer).child(split).child(dock)))
            .into_any_element()
    }

    /// Área da consulta, fixa embaixo: editor com destaque de SQL, executar e estado.
    fn render_db_dock(&mut self, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let mono = self.mono.clone();
        let p = self.db.as_ref().expect("painel aberto");
        let (sql, focus, running, error) = (p.sql.clone(), p.sql_focus, p.running, p.error.clone());
        let status = if running {
            ("Executando…".to_string(), c_muted())
        } else if let Some(e) = &error {
            (e.clone(), RED)
        } else if let Some(res) = &p.result {
            match res.changes {
                Some(n) => (format!("{n} linha(s) afetada(s)"), c_muted()),
                None => (format!("Executada · {} linhas em {} ms", res.rows.len(), res.elapsed.as_millis()), 0x8fd7ae),
            }
        } else {
            ("Pronto · Ctrl+Enter executa".to_string(), c_muted())
        };
        const SHOWN: usize = 6;
        let lines: Vec<&str> = sql.split('\n').collect();
        let skip = lines.len().saturating_sub(SHOWN);
        let last = lines.len() - 1;
        let color = |t: Tok| match t {
            Tok::Plain => c_text(),
            Tok::Keyword => accent,
            Tok::Str => 0x8fd0a8,
            Tok::Num => BLUE,
            Tok::Comment => c_muted(),
        };
        let editor = div()
            .id("db-sql")
            .flex_1()
            .min_h_0()
            .flex()
            .overflow_hidden()
            .font_family(mono.clone())
            .text_size(px(12.))
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    if let Some(p) = this.db.as_mut() {
                        p.sql_focus = true;
                        p.filter_focus = false;
                    }
                    this.focused_pane = Some(PaneKind::Db);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(div().flex_none().w(px(40.)).pt(px(8.)).pr(px(10.)).flex().flex_col().items_end().text_color(rgb(c_disabled())).children((skip..lines.len()).map(|i| div().h(px(20.)).child(format!("{}", i + 1)))))
            .child(div().flex_1().min_w_0().pt(px(8.)).pr(px(12.)).flex().flex_col().children(lines.iter().enumerate().skip(skip).map(|(i, l)| {
                let mut row = div().h(px(20.)).flex().whitespace_nowrap().children(highlight(l).into_iter().map(|(text, t)| div().flex_none().text_color(rgb(color(t))).child(text)));
                if i == last && focus {
                    row = row.child(div().flex_none().w(px(2.)).h(px(14.)).mt(px(3.)).bg(rgb(accent)));
                }
                row
            })));
        div()
            .flex_none()
            .h(px(DOCK_H))
            .flex()
            .flex_col()
            .bg(rgb(c_panel()))
            .border_t_1()
            .border_color(rgb(if focus { accent } else { c_card_border() }))
            .child(
                div()
                    .flex_none()
                    .h(px(36.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(c_card_border()))
                    .child(div().flex_none().h(px(24.)).px(px(10.)).flex().items_center().rounded(px(5.)).bg(rgb(c_surface())).text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(rgb(accent)).child("Consulta"))
                    .child(div().flex_1())
                    .children(wide.then(|| div().flex_none().text_size(px(11.)).text_color(rgb(c_muted())).child(format!("Limite {DEFAULT_LIMIT} linhas"))))
                    .child(self.db_btn("db-run", if running { "Executando…" } else if wide { "Executar  Ctrl+Enter" } else { "Executar" }, true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_run_sql(cx)))),
            )
            .child(editor)
            .child(div().flex_none().h(px(24.)).px(px(12.)).flex().items_center().border_t_1().border_color(rgb(c_card_border())).text_size(px(11.)).text_color(rgb(status.1)).child(div().min_w_0().truncate().child(status.0)))
            .into_any_element()
    }

    /// Diagrama das tabelas e das chaves estrangeiras; arrastar com a roda do mouse rola (Shift: para o lado).
    fn render_db_diagram(&self, view: (f32, f32), cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let mono = self.mono.clone();
        let p = self.db.as_ref().expect("painel aberto");
        let lay = dbdiagram::layout(&p.tables);
        let selected = p.selected;
        let (dx, dy) = (p.dg_x.clamp(0.0, (lay.width - view.0).max(0.0)), p.dg_y.clamp(0.0, (lay.height - view.1).max(0.0)));
        let mut host = div().absolute().left(px(-dx)).top(px(-dy)).w(px(lay.width.max(view.0))).h(px(lay.height.max(view.1)));

        // setas primeiro (ficam atrás das tabelas)
        for e in &lay.edges {
            let hot = selected == Some(e.from) || selected == Some(e.to);
            let color = if hot { accent } else { c_muted() };
            for &(x1, y1, x2, y2) in &e.segs {
                let (l, t, w, h) = if y1 == y2 { (x1.min(x2), y1 - 1.0, (x2 - x1).abs().max(2.0), 2.0) } else { (x1 - 1.0, y1.min(y2), 2.0, (y2 - y1).abs().max(2.0)) };
                host = host.child(div().absolute().left(px(l)).top(px(t)).w(px(w)).h(px(h)).bg(rgb(color)));
            }
            host = host.child(div().absolute().left(px(e.tip.0 + 1.0)).top(px(e.tip.1 - 9.0)).text_size(px(12.)).text_color(rgb(color)).child("◀"));
        }
        for (i, pl) in lay.placed.iter().enumerate() {
            let Some(pl) = pl else { continue };
            let t = &p.tables[i];
            let on = selected == Some(i);
            let shown = t.columns.len().min(dbdiagram::MAX_ROWS);
            let mut card = div()
                .id(("db-card", i))
                .absolute()
                .left(px(pl.x))
                .top(px(pl.y))
                .w(px(pl.w))
                .h(px(pl.h))
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded_lg()
                .bg(rgb(if on { c_surface() } else { c_panel() }))
                .border_1()
                .border_color(rgb(if on { accent } else { c_card_border() }))
                .cursor_pointer()
                .child(div().flex_none().h(px(dbdiagram::HEAD_H)).px(px(12.)).flex().items_center().text_size(px(13.)).font_weight(FontWeight::BOLD).text_color(rgb(if on { accent } else { c_text() })).truncate().child(t.label()));
            for c in t.columns.iter().take(shown) {
                let badge = if c.pk > 0 {
                    Some(("PK", AMBER))
                } else if t.is_fk(&c.name) {
                    Some(("FK", BLUE))
                } else {
                    None
                };
                card = card.child(
                    div()
                        .flex_none()
                        .h(px(dbdiagram::ROW_H))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(11.))
                        .font_family(mono.clone())
                        .child(div().flex_none().w(px(22.)).flex().justify_center().children(badge.map(|(l, col)| div().px(px(3.)).rounded(px(3.)).bg(with_alpha(col, 0x33)).text_size(px(9.)).font_weight(FontWeight::BOLD).text_color(rgb(col)).child(l))))
                        .child(div().flex_1().min_w(px(40.)).truncate().text_color(rgb(c_text())).child(c.name.clone()))
                        .child(div().flex_none().max_w(px(96.)).truncate().text_color(rgb(c_muted())).child(short_type(&c.decl_type))),
                );
            }
            if t.columns.len() > shown {
                card = card.child(div().flex_none().h(px(dbdiagram::ROW_H)).px(px(12.)).flex().items_center().text_size(px(11.)).text_color(rgb(c_muted())).child(format!("+ {} colunas", t.columns.len() - shown)));
            }
            host = host.child(card.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.focused_pane = Some(PaneKind::Db);
                this.db_select_table(i, View::Diagram, cx);
            })));
        }
        let empty = lay.placed.iter().all(|p| p.is_none());
        div()
            .id("db-diagram")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(c_bg()))
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                if let Some(p) = this.db.as_mut() {
                    let d = ev.delta.pixel_delta(px(24.));
                    let (ddx, ddy) = (f32::from(d.x), f32::from(d.y));
                    if ev.modifiers.shift || ddx.abs() > ddy.abs() {
                        p.dg_x = (p.dg_x - if ddx.abs() > ddy.abs() { ddx } else { ddy }).max(0.0);
                    } else {
                        p.dg_y = (p.dg_y - ddy).max(0.0);
                    }
                }
                cx.notify();
            }))
            .child(host)
            .when(empty, |d| d.child(div().p_3().text_size(px(12.)).text_color(rgb(c_muted())).child("O banco não tem tabelas para desenhar.")))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_files_are_recognized_by_extension() {
        for ok in ["a.sqlite", "a.SQLITE3", "dados.db", "x.db3", "y.s3db"] {
            assert!(is_db_file(Path::new(ok)), "{ok}");
        }
        for no in ["a.txt", "db", "a.dbf", "sqlite"] {
            assert!(!is_db_file(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn column_widths_follow_content_within_limits() {
        let cols = vec!["id".to_string(), "descricao".to_string()];
        let rows = vec![vec![Value::Integer(1), Value::Text("x".repeat(100))], vec![Value::Integer(22), Value::Null]];
        let w = col_widths(&cols, &rows);
        assert_eq!(w[0], 80.0);
        assert_eq!(w[1], 300.0);
        assert!(col_widths(&[], &[]).is_empty());
    }

    #[test]
    fn home_is_expanded_only_at_the_start() {
        assert_eq!(expand_home("/tmp/x.db"), "/tmp/x.db");
        assert!(expand_home("~/x.db").ends_with("/x.db") && !expand_home("~/x.db").starts_with('~'));
        assert_eq!(expand_home("a~/b"), "a~/b");
    }

    #[test]
    fn long_types_are_shortened_for_the_diagram() {
        assert_eq!(short_type("timestamp without time zone"), "timestamp");
        assert_eq!(short_type("TIMESTAMP WITH TIME ZONE"), "timestamptz");
        assert_eq!(short_type("character varying"), "varchar");
        assert_eq!(short_type("double precision"), "double");
        assert_eq!(short_type("integer"), "integer");
        assert_eq!(short_type("numeric"), "numeric");
        assert_eq!(short_type(""), "");
        // tipo desconhecido e comprido é cortado com reticências
        assert_eq!(short_type("a_very_long_custom_type_name"), "a_very_long_c…");
    }

    #[test]
    fn sql_is_highlighted_by_kind() {
        let t = highlight("SELECT count(*), 'a b' FROM t WHERE x > 10 -- fim");
        let kinds: Vec<(&str, Tok)> = t.iter().map(|(s, k)| (s.as_str(), *k)).collect();
        assert!(kinds.contains(&("SELECT", Tok::Keyword)));
        assert!(kinds.contains(&("'a b'", Tok::Str)));
        assert!(kinds.contains(&("10", Tok::Num)));
        assert!(kinds.contains(&("-- fim", Tok::Comment)));
        assert!(kinds.contains(&("FROM", Tok::Keyword)) && kinds.contains(&("WHERE", Tok::Keyword)));
        // "selecionado" não é a palavra select; o texto todo se reconstrói sem perder nada
        assert!(highlight("selecionado").iter().all(|(_, k)| *k == Tok::Plain));
        let line = "INSERT INTO t VALUES (1, 'x''y');";
        assert_eq!(highlight(line).iter().map(|(s, _)| s.as_str()).collect::<String>(), line);
        // texto sem fechar aspas não trava nem perde nada
        assert_eq!(highlight("SELECT 'abc").iter().map(|(s, _)| s.as_str()).collect::<String>(), "SELECT 'abc");
        assert!(highlight("").is_empty());
    }

    #[test]
    fn the_form_validates_before_saving() {
        let mut f = EditForm::new();
        let c = f.to_saved().unwrap();
        assert_eq!((c.name.as_str(), c.port, c.kind), ("postgres", 5432, Kind::Postgres)); // sem nome: usa o do banco
        f.port = "abc".into();
        assert!(f.to_saved().unwrap_err().contains("porta"));
        f.port = "0".into();
        assert!(f.to_saved().is_err());
        f.port = "5432".into();
        f.host = "  ".into();
        assert!(f.to_saved().unwrap_err().contains("preencha"));
        let mut s = EditForm::new();
        s.kind = Kind::Sqlite;
        assert!(s.to_saved().is_err());
        s.path = "~/dados/vendas.sqlite".into();
        let c = s.to_saved().unwrap();
        assert_eq!((c.name.as_str(), c.kind), ("vendas", Kind::Sqlite));
        // editar uma conexão guarda o id e o formulário não tem senha em lugar nenhum
        let e = EditForm::from_saved(&SavedConn { id: 7, ..c });
        assert_eq!(e.to_saved().unwrap().id, 7);
    }
}
