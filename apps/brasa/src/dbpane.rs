//! Painel Banco: várias conexões salvas (PostgreSQL e arquivos SQLite), tabelas, dados em grade, estrutura,
//! diagrama das relações e uma área de consulta fixa embaixo. As consultas rodam em segundo plano (a janela não
//! congela) e todo banco abre só para leitura. O motor está no crate `ox-db`; a senha nunca é gravada.

use std::collections::HashMap;
use std::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::{div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent};
use ox_editor::TextBuffer;
use ox_db::{export_query, export_table, is_plain_select, to_tsv, write_result, Format, Database, Db, Kind, Options, Page, PgConfig, PgDatabase, QueryResult, SavedConn, Store, Table, Value, DEFAULT_LIMIT, DEFAULT_TIMEOUT};

use crate::dbdiagram;
use crate::icons::wicon;
use crate::pane_view::with_alpha;
use crate::panes::{PaneKind, Rect};
use crate::queryedit::{self, Outcome};
use crate::theme::*;
use crate::Workspace;

/// Linhas por página na grade de dados.
pub const PAGE_SIZE: usize = 100;
/// Linhas lidas de cada vez no resultado de uma query: a grade rola por qualquer quantidade de linhas, buscando
/// outro trecho quando chega perto da borda.
const WINDOW: usize = 1000;
const ROW_H: f32 = 28.0;
const HEAD_H: f32 = 38.0;
const BAR_H: f32 = 40.0;
const TABS_H: f32 = 40.0;
const FOOT_H: f32 = 32.0;
const SPLIT_H: f32 = 10.0;
const EDIT_BAR_H: f32 = 40.0;
const DOCK_H: f32 = 210.0;
/// largura média de um caractere do texto monoespaçado da grade (12 px)
const CHAR_W: f32 = 7.3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Data,
    Structure,
    Objects,
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
    /// PostgreSQL remoto com certificado próprio: arquivo PEM da CA (opcional)
    pub ca: String,
    /// abrir só para leitura
    pub read_only: bool,
    /// campo com o teclado: 0 nome; PostgreSQL: 1 servidor, 2 porta, 3 usuário, 4 banco, 5 CA; SQLite: 1 arquivo
    pub focus: usize,
    /// posição do cursor (em caracteres) no campo com o teclado; passar do fim vale como o fim
    pub cursor: usize,
}

impl EditForm {
    fn new() -> Self {
        Self { id: 0, name: String::new(), kind: Kind::Postgres, host: "localhost".into(), port: "5432".into(), user: "postgres".into(), dbname: "postgres".into(), path: String::new(), ca: String::new(), read_only: false, focus: 0, cursor: usize::MAX }
    }

    fn from_saved(c: &SavedConn) -> Self {
        Self { id: c.id, name: c.name.clone(), kind: c.kind, host: c.host.clone(), port: c.port.to_string(), user: c.user.clone(), dbname: c.dbname.clone(), path: c.path.clone(), ca: c.ca_file.clone(), read_only: c.read_only, focus: 0, cursor: usize::MAX }
    }

    fn count(&self) -> usize {
        if self.kind == Kind::Postgres {
            6
        } else {
            2
        }
    }

    /// Cursor dentro do campo atual (nunca passa do fim).
    fn cur(&mut self) -> usize {
        let n = self.field_mut().chars().count();
        self.cursor = self.cursor.min(n);
        self.cursor
    }

    fn byte_at(text: &str, chars: usize) -> usize {
        text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
    }

    /// Escreve no cursor e avança.
    fn insert(&mut self, s: &str) {
        let at = self.cur();
        let f = self.field_mut();
        let b = Self::byte_at(f, at);
        f.insert_str(b, s);
        self.cursor = at + s.chars().count();
    }

    /// Apaga o caractere antes (`back`) ou depois do cursor.
    fn erase(&mut self, back: bool) {
        let at = self.cur();
        let f = self.field_mut();
        let n = f.chars().count();
        let del = if back {
            if at == 0 {
                return;
            }
            at - 1
        } else {
            if at >= n {
                return;
            }
            at
        };
        let (a, b) = (Self::byte_at(f, del), Self::byte_at(f, del + 1));
        f.replace_range(a..b, "");
        self.cursor = del;
    }

    /// Anda o cursor: `Some(n)` soma n caracteres (negativo volta), Home e End vão às pontas.
    fn step(&mut self, delta: isize) {
        let at = self.cur() as isize;
        let n = self.field_mut().chars().count() as isize;
        self.cursor = (at + delta).clamp(0, n) as usize;
    }

    fn focus_on(&mut self, idx: usize) {
        self.focus = idx;
        self.cursor = usize::MAX;
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
            (Kind::Postgres, 4) => &mut self.dbname,
            (Kind::Postgres, _) => &mut self.ca,
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
                Ok(SavedConn { id: self.id, name, kind: Kind::Postgres, host: host.into(), port, user: user.into(), dbname: dbname.into(), path: String::new(), ca_file: self.ca.trim().into(), read_only: self.read_only })
            }
            Kind::Sqlite => {
                let path = self.path.trim();
                if path.is_empty() {
                    return Err("informe o caminho do arquivo".into());
                }
                let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("banco").to_string();
                let name = if self.name.trim().is_empty() { stem } else { self.name.trim().to_string() };
                Ok(SavedConn { id: self.id, name, kind: Kind::Sqlite, host: String::new(), port: 0, user: String::new(), dbname: String::new(), path: path.into(), ca_file: String::new(), read_only: self.read_only })
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

/// De onde veio o trecho de resultado que a grade mostra, quando a query devolve linhas e pode ser lida aos poucos.
#[derive(Clone)]
pub struct Stream {
    pub sql: String,
    /// primeira linha do trecho carregado (em `result.rows`)
    pub base: usize,
    /// há mais linhas depois do trecho
    pub more: bool,
    /// total de linhas, se já foi contado ou se a leitura chegou ao fim
    pub total: Option<u64>,
    pub loading: bool,
    pub req: u64,
}

/// Exportar a grade ou o resultado: para onde gravar e em que formato.
pub struct ExportForm {
    pub path: String,
    pub json: bool,
}

/// Arrasto de uma tabela no diagrama.
pub struct Drag {
    pub table: usize,
    pub start_mouse: (f32, f32),
    pub start_pos: (f32, f32),
    /// já andou o bastante para ser um arrasto (e não um clique)
    pub moved: bool,
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
    /// índices, triggers, funções e sequências do banco
    pub objects: Vec<ox_db::DbObject>,
    /// interrompe a query em andamento sem esperar a conexão
    pub canceller: Option<ox_db::Canceller>,
    /// queries já rodadas (de todas as conexões) e a posição atual ao andar por elas com Alt+↑/↓
    pub history: ox_db::History,
    pub hist_pos: Option<usize>,
    /// o usuário pediu para cancelar a query que está rodando
    pub cancelling: bool,
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
    /// posições escolhidas arrastando as tabelas do diagrama, por índice de tabela
    pub moved: HashMap<usize, (f32, f32)>,
    pub drag: Option<Drag>,
    /// modo edição ligado (a sessão aceita escrita)
    pub writable: bool,
    /// mudanças pendentes na página atual: células trocadas, linhas apagadas e linhas novas
    /// célula escolhida com um clique (para copiar com Ctrl+C)
    pub sel_cell: Option<(usize, usize)>,
    pub export: Option<ExportForm>,
    /// aviso de sucesso (por exemplo, o arquivo exportado)
    pub notice: Option<String>,
    /// o texto da Query, com cursor, seleção e desfazer
    pub editor: TextBuffer,
    pub sql_scroll: usize,
    /// o botão do mouse está apertado dentro do editor (arrastando uma seleção)
    pub sql_drag: bool,
    pub sql_focus: bool,
    pub result: Option<QueryResult>,
    pub stream: Option<Stream>,
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
            objects: Vec::new(),
            canceller: None,
            history: ox_db::History::default(),
            hist_pos: None,
            cancelling: false,
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
            moved: HashMap::new(),
            drag: None,
            writable: false,
            sel_cell: None,
            export: None,
            notice: None,
            editor: TextBuffer::new("SELECT 1;"),
            sql_scroll: 0,
            sql_drag: false,
            sql_focus: false,
            result: None,
            stream: None,
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
        Self { store, error: err, history: ox_db::History::load(&history_path()), ..Self::default() }
    }
}

fn history_path() -> PathBuf {
    crate::config_dir().join("db-history.json")
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

/// Tecla de digitação (letra, Enter, Backspace...) sem Ctrl, Alt nem Super? Com a página do banco aberta ela não vai
/// para o terminal escondido.
pub fn swallows_typing(key: &str, has_char: bool, ctrl: bool, alt: bool, platform: bool) -> bool {
    !ctrl && !alt && !platform && (has_char || matches!(key, "enter" | "backspace" | "space" | "tab" | "delete"))
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

/// Primeiro nome livre a partir de `base`: `a.csv`, `a-2.csv`, `a-3.csv`...
pub fn unique_path(base: &Path) -> PathBuf {
    if !base.exists() {
        return base.to_path_buf();
    }
    let stem = base.file_stem().and_then(|s| s.to_str()).unwrap_or("saida").to_string();
    let ext = base.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    (2..10_000).map(|n| base.with_file_name(format!("{stem}-{n}{ext}"))).find(|p| !p.exists()).unwrap_or_else(|| base.to_path_buf())
}

/// Nome de arquivo seguro para uma tabela ou resultado (`public.clientes` vira `public.clientes`; barras e espaços somem).
pub fn export_file_name(name: &str, json: bool) -> String {
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    format!("{}.{}", if clean.trim_matches('_').is_empty() { "resultado" } else { &clean }, if json { "json" } else { "csv" })
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
    fn adopt_db(&mut self, db: Db, tables: Vec<Table>, active: Option<u64>, name: String, read_only: bool, cx: &mut Context<Self>) {
        let (kind, label) = (db.kind(), db.label());
        let mut db = db;
        let objects = db.objects().unwrap_or_default();
        let canceller = Some(db.canceller());
        let Some(p) = self.db.as_mut() else { return };
        let store = std::mem::take(&mut p.store);
        let history = std::mem::take(&mut p.history);
        *p = DbPane { store, history, db: Some(Arc::new(Mutex::new(db))), active, name, kind, label, writable: !read_only, selected: (!tables.is_empty()).then_some(0), tables, objects, canceller, ..DbPane::default() };
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

    fn open_sqlite(&mut self, path: PathBuf, active: Option<u64>, name: String, read_only: bool, cx: &mut Context<Self>) {
        let res = Database::open(&path, &Options { read_only, ..Options::default() }).and_then(|d| {
            let mut db = Db::Sqlite(d);
            db.schema().map(|t| (db, t))
        });
        match res {
            Ok((db, tables)) => self.adopt_db(db, tables, active, name, read_only, cx),
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
        self.open_sqlite(path, None, name, false, cx);
    }

    /// Clicou em Conectar numa conexão salva: SQLite abre direto; PostgreSQL pede a senha.
    pub fn db_connect_saved(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let Some(conn) = p.store.get(id).cloned() else { return };
        p.error = None;
        match conn.kind {
            Kind::Sqlite => self.open_sqlite(PathBuf::from(expand_home(&conn.path)), Some(id), conn.name, conn.read_only, cx),
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
        let cfg = PgConfig { host: conn.host.clone(), port: conn.port, user: conn.user.clone(), password: (!password.is_empty()).then_some(password), dbname: conn.dbname.clone(), ca_file: (!conn.ca_file.trim().is_empty()).then(|| expand_home(conn.ca_file.trim())) };
        p.error = None;
        p.manager.connecting = true;
        p.epoch += 1;
        let epoch = p.epoch;
        let read_only = conn.read_only;
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_executor()
                .spawn(async move {
                    let mut db = Db::Postgres(PgDatabase::connect(&cfg)?);
                    // a conexão abre protegida; só se permite escrever se ela não for marcada como somente leitura
                    db.set_writable(!read_only)?;
                    let tables = db.schema()?;
                    Ok::<_, ox_db::Error>((db, tables))
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                match out {
                    Ok((db, tables)) => ws.adopt_db(db, tables, Some(id), conn.name.clone(), read_only, cx),
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
        let id = p.store.upsert(SavedConn { id: 0, name: p.name.clone(), kind: Kind::Sqlite, host: String::new(), port: 0, user: String::new(), dbname: String::new(), path: p.label.clone(), ca_file: String::new(), read_only: false });
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
        p.sel_cell = None;
        p.notice = None;
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
            // o botão vira "Cancelar": interrompe a query, a conexão continua aberta
            if let Some(c) = p.canceller.clone().filter(|_| !p.cancelling) {
                p.cancelling = true;
                cx.background_executor().spawn(async move { c.cancel() }).detach();
                cx.notify();
            }
            return;
        }
        p.running = true;
        p.cancelling = false;
        p.error = None;
        p.epoch += 1;
        let (epoch, sql) = (p.epoch, p.editor.text());
        let key = p.label.clone();
        p.history.push(&key, &sql);
        p.hist_pos = None;
        let _ = p.history.save(&history_path());
        cx.spawn(async move |this, cx| {
            let text = sql.clone();
            let out = cx
                .background_executor()
                .spawn(async move {
                    let mut db = db.lock().unwrap();
                    // uma query que devolve linhas é lida aos poucos, sem o corte de antes; o resto (scripts, INSERT...) roda inteiro
                    if is_plain_select(&sql) {
                        db.window(&sql, 0, WINDOW).map(|r| (r, true))
                    } else {
                        db.query(&sql, DEFAULT_LIMIT, DEFAULT_TIMEOUT).map(|r| (r, false))
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                p.running = false;
                let cancelled = std::mem::take(&mut p.cancelling);
                match out {
                    Ok((mut r, streamed)) => {
                        p.stream = streamed.then(|| {
                            let more = std::mem::take(&mut r.truncated);
                            Stream { sql: text, base: 0, more, total: (!more).then_some(r.rows.len() as u64), loading: false, req: 0 }
                        });
                        p.result = Some(r);
                        p.view = View::Result;
                        p.vscroll = 0;
                        p.hscroll = 0.0;
                    }
                    Err(_) if cancelled => p.error = Some("Query cancelada.".into()),
                    Err(e) => p.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Rolou o resultado? Busca o trecho certo se a grade chegou perto da borda do que está carregado.
    pub fn db_ensure_window(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let (Some(db), Some(st), Some(res)) = (p.db.clone(), p.stream.as_mut(), p.result.as_ref()) else { return };
        if st.loading || p.running || p.view != View::Result {
            return;
        }
        let (first, end) = (p.vscroll, st.base + res.rows.len());
        // o que a grade mostra cabe em ~120 linhas
        if !(first < st.base || (st.more && first + 120 > end)) {
            return;
        }
        let start = first.saturating_sub(WINDOW / 4);
        st.loading = true;
        st.req += 1;
        let (req, sql) = (st.req, st.sql.clone());
        let epoch = p.epoch;
        cx.spawn(async move |this, cx| {
            let out = cx.background_executor().spawn(async move { db.lock().unwrap().window(&sql, start, WINDOW) }).await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                let (Some(st), Some(res)) = (p.stream.as_mut().filter(|s| s.req == req), p.result.as_mut()) else { return };
                st.loading = false;
                match out {
                    Ok(r) => {
                        st.base = start;
                        st.more = r.truncated;
                        if !r.truncated {
                            st.total = Some((start + r.rows.len()) as u64);
                        }
                        res.rows = r.rows;
                    }
                    Err(e) => p.error = Some(e.to_string()),
                }
                ws.db_ensure_window(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Conta as linhas do resultado (pode demorar em tabelas enormes; dá para cancelar).
    pub fn db_count(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let (Some(db), Some(st)) = (p.db.clone(), p.stream.as_ref()) else { return };
        if p.running {
            return;
        }
        p.running = true;
        p.cancelling = false;
        p.error = None;
        p.epoch += 1;
        let (epoch, sql) = (p.epoch, st.sql.clone());
        cx.spawn(async move |this, cx| {
            let out = cx.background_executor().spawn(async move { db.lock().unwrap().count(&sql, Duration::from_secs(600)) }).await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                p.running = false;
                let cancelled = std::mem::take(&mut p.cancelling);
                match out {
                    Ok(n) => {
                        if let Some(st) = p.stream.as_mut() {
                            st.total = Some(n);
                        }
                    }
                    Err(_) if cancelled => p.error = Some("Contagem cancelada.".into()),
                    Err(e) => p.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Cola `text` no campo que está com o teclado. Campos de uma linha só recebem a primeira linha. `false` se nenhum
    /// campo de texto está ativo (então a tecla segue o caminho normal).
    fn db_paste(&mut self, text: &str) -> bool {
        let one_line = text.lines().next().unwrap_or("").trim_end_matches('\r');
        let Some(p) = self.db.as_mut() else { return false };
        if p.db.is_none() {
            if let Some(f) = p.manager.edit.as_mut() {
                if f.is_port() {
                    f.insert(&one_line.chars().filter(char::is_ascii_digit).collect::<String>());
                } else {
                    f.insert(one_line);
                }
                return true;
            }
            if p.manager.asking.is_some() {
                p.manager.password.push_str(one_line);
                return true;
            }
            return false;
        }
        if let Some(e) = p.export.as_mut() {
            e.path.push_str(one_line);
        } else if p.filter_focus {
            p.filter.push_str(one_line);
            p.side_scroll = 0;
        } else if p.sql_focus {
            p.editor.insert_str(&text.replace("\r\n", "\n"));
            p.sql_scroll = queryedit::scroll_to_cursor(p.sql_scroll, p.editor.cursor().line, p.editor.line_count());
        } else {
            return false;
        }
        true
    }

    /// Teclas com o painel Banco em foco.
    pub fn on_db_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.to_lowercase();
        let (ctrl, alt) = (k.modifiers.control, k.modifiers.alt);
        // com a página do banco aberta o terminal fica escondido: digitação solta não pode chegar nele (um Enter
        // rodaria um comando às cegas). Atalhos com Ctrl, Alt ou Super continuam passando.
        let typing = swallows_typing(&key, k.key_char.is_some(), ctrl, alt, k.modifiers.platform);
        // Ctrl+C numa célula escolhida (e nenhum campo de texto ativo) copia o valor dela
        if ctrl && !alt && key == "c" {
            let busy = self.db.as_ref().is_none_or(|p| p.sql_focus || p.filter_focus || p.export.is_some() || p.db.is_none());
            if !busy && self.db_copy_cell(cx) {
                return true;
            }
        }
        // Ctrl+V cola no campo que está com o teclado (query, célula, senha, formulário, filtro)
        if ctrl && !alt && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                if self.db_paste(&text) {
                    cx.notify();
                    return true;
                }
            }
        }
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
                        let next = if k.modifiers.shift { (f.focus + n - 1) % n } else { (f.focus + 1) % n };
                        f.focus_on(next);
                    }
                    "backspace" => f.erase(true),
                    "delete" => f.erase(false),
                    "left" if !ctrl && !alt => f.step(-1),
                    "right" if !ctrl && !alt => f.step(1),
                    "home" => f.step(isize::MIN / 2),
                    "end" => f.step(isize::MAX / 2),
                    _ if !ctrl && !alt => match (&k.key_char, key.as_str()) {
                        (Some(c), _) if f.is_port() && !c.chars().all(|ch| ch.is_ascii_digit()) => return true,
                        (Some(c), _) => f.insert(c),
                        (None, "space") if !f.is_port() => f.insert(" "),
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
            return typing;
        }

        if p.export.is_some() {
            match key.as_str() {
                "enter" => {
                    self.db_run_export(cx);
                    return true;
                }
                "escape" => p.export = None,
                "backspace" => {
                    if let Some(e) = p.export.as_mut() {
                        e.path.pop();
                    }
                }
                _ if !ctrl && !alt => match (&k.key_char, key.as_str()) {
                    (Some(t), _) => {
                        if let Some(e) = p.export.as_mut() {
                            e.path.push_str(t);
                        }
                    }
                    (None, "space") => {
                        if let Some(e) = p.export.as_mut() {
                            e.path.push(' ');
                        }
                    }
                    _ => return false,
                },
                _ => return false,
            }
            cx.notify();
            return true;
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
        if p.sql_focus && alt && !ctrl && matches!(key.as_str(), "up" | "down") {
            let list = p.history.list(&p.label).to_vec();
            if list.is_empty() {
                return true;
            }
            // Alt+↑ vai para as mais antigas; Alt+↓ volta, e depois da mais recente esvazia o editor
            let next = match (p.hist_pos, key.as_str()) {
                (None, "up") => Some(list.len() - 1),
                (Some(i), "up") => Some(i.saturating_sub(1)),
                (Some(i), _) if i + 1 < list.len() => Some(i + 1),
                _ => None,
            };
            p.hist_pos = next;
            p.editor = TextBuffer::new(next.map_or("", |i| list[i].as_str()));
            cx.notify();
            return true;
        }
        if p.sql_focus {
            let outcome = queryedit::handle_key(&mut p.editor, &key, k.key_char.as_deref(), ctrl, k.modifiers.shift, alt);
            match outcome {
                Outcome::Run => {
                    self.db_run_sql(cx);
                    return true;
                }
                Outcome::Blur => p.sql_focus = false,
                Outcome::Copy(t) => cx.write_to_clipboard(gpui::ClipboardItem::new_string(t)),
                Outcome::Cut(t) => cx.write_to_clipboard(gpui::ClipboardItem::new_string(t)),
                Outcome::Paste => {
                    if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        p.editor.insert_str(&t.replace("\r\n", "\n"));
                    }
                }
                Outcome::Handled => {}
                Outcome::Ignored => return false,
            }
            p.sql_scroll = queryedit::scroll_to_cursor(p.sql_scroll, p.editor.cursor().line, p.editor.line_count());
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
            "pagedown" if p.view == View::Result => p.vscroll += 30,
            "pageup" if p.view == View::Result => p.vscroll = p.vscroll.saturating_sub(30),
            "down" => p.vscroll += 1,
            "up" => p.vscroll = p.vscroll.saturating_sub(1),
            "escape" => self.show_db = false,
            _ => return typing,
        }
        if self.db.as_ref().is_some_and(|p| p.view == View::Result) {
            self.db_ensure_window(cx);
        }
        cx.notify();
        true
    }
}

// ---------------------------------------------------------------------------------------------
// Edição na grade

impl Workspace {
    /// Colunas e linhas que a aba atual mostra (com as mudanças pendentes já aplicadas na tela); `None` nas abas sem grade.
    fn db_grid_data(&self) -> Option<(Vec<String>, Vec<Vec<Value>>)> {
        let p = self.db.as_ref()?;
        match p.view {
            View::Data => {
                let pg = p.page.as_ref()?;
                let rows = pg.rows.clone();
                Some((pg.columns.clone(), rows))
            }
            View::Result => p.result.as_ref().filter(|r| !r.columns.is_empty()).map(|r| (r.columns.clone(), r.rows.clone())),
            _ => None,
        }
    }

    /// Ctrl+C numa célula escolhida: o valor dela vai para a área de transferência.
    pub fn db_copy_cell(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((_, rows)) = self.db_grid_data() else { return false };
        let Some((r, c)) = self.db.as_ref().and_then(|p| p.sel_cell) else { return false };
        let Some(v) = rows.get(r).and_then(|row| row.get(c)) else { return false };
        let text = match v {
            Value::Null => String::new(),
            other => other.display(),
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        if let Some(p) = self.db.as_mut() {
            p.notice = Some("Célula copiada".into());
        }
        cx.notify();
        true
    }

    /// Copia a grade inteira da aba (cabeçalho e linhas) em TSV, que as planilhas entendem ao colar.
    pub fn db_copy_grid(&mut self, cx: &mut Context<Self>) {
        let Some((cols, rows)) = self.db_grid_data() else { return };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(to_tsv(&cols, &rows)));
        if let Some(p) = self.db.as_mut() {
            p.notice = Some(format!("{} linha(s) copiada(s) (TSV)", rows.len()));
            p.error = None;
        }
        cx.notify();
    }

    /// Abre a barra de exportar, com um nome livre na pasta pessoal.
    pub fn db_open_export(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let base = match p.view {
            View::Data => p.selected.and_then(|i| p.tables.get(i)).map(|t| t.label()).unwrap_or_default(),
            _ => "resultado".to_string(),
        };
        let json = p.export.as_ref().is_some_and(|e| e.json);
        let path = unique_path(&crate::home().join(export_file_name(&base, json)));
        p.export = Some(ExportForm { path: path.display().to_string(), json });
        p.notice = None;
        cx.notify();
    }

    /// Troca o formato e a extensão do nome (CSV e JSON).
    pub fn db_toggle_export_format(&mut self, cx: &mut Context<Self>) {
        if let Some(e) = self.db.as_mut().and_then(|p| p.export.as_mut()) {
            e.json = !e.json;
            let (from, to) = if e.json { (".csv", ".json") } else { (".json", ".csv") };
            if let Some(stem) = e.path.strip_suffix(from) {
                e.path = format!("{stem}{to}");
            }
        }
        cx.notify();
    }

    /// Grava o arquivo, em segundo plano. Na aba Dados exporta a tabela inteira; no Resultado, o resultado da query.
    pub fn db_run_export(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        let Some(form) = p.export.as_ref() else { return };
        if form.path.trim().is_empty() {
            p.error = Some("informe onde gravar o arquivo".into());
            return;
        }
        let path = PathBuf::from(expand_home(form.path.trim()));
        let format = if form.json { Format::Json } else { Format::Csv };
        let (db, table, sort) = (p.db.clone(), p.selected.and_then(|i| p.tables.get(i)).cloned(), p.sort.clone());
        let view = p.view;
        let stream_sql = p.stream.as_ref().filter(|_| view == View::Result).map(|s| s.sql.clone());
        let memory = self.db_grid_data();
        let Some(p) = self.db.as_mut() else { return };
        if p.running {
            return;
        }
        p.running = true;
        p.error = None;
        p.epoch += 1;
        let epoch = p.epoch;
        let shown = path.display().to_string();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_executor()
                .spawn(async move {
                    match (view, db, table, memory) {
                        // o resultado inteiro, lido aos poucos do banco (sem o limite do que está na tela)
                        (View::Result, Some(db), _, _) if stream_sql.is_some() => export_query(&mut db.lock().unwrap(), stream_sql.as_deref().unwrap_or(""), format, &path),
                        (View::Data, Some(db), Some(t), _) => export_table(&mut db.lock().unwrap(), &t, sort.as_ref().map(|(c, a)| (c.as_str(), *a)), format, &path),
                        (_, _, _, Some((cols, rows))) => write_result(&path, format, &cols, &rows).map(|()| rows.len() as u64),
                        _ => Err(ox_db::Error::Sql("não há nada para exportar nesta aba".into())),
                    }
                })
                .await;
            let _ = this.update(cx, |ws, cx| {
                let Some(p) = ws.db.as_mut().filter(|p| p.epoch == epoch) else { return };
                p.running = false;
                match out {
                    Ok(n) => {
                        p.export = None;
                        p.notice = Some(format!("Exportado: {n} linha(s) em {shown}"));
                    }
                    Err(e) => p.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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

    /// Grade de resultados: cabeçalho fixo, linhas com rolagem vertical e horizontal. `sortable`: clicar no cabeçalho
    /// ordena. `edit`: na aba Dados com o modo edição, dois cliques abrem a célula e há um "×" por linha.
    fn db_grid(&self, id: &'static str, cols: &[String], rows: &[Vec<Value>], types: &[String], view: (f32, f32), sortable: bool, win: (usize, usize), cx: &mut Context<Self>) -> AnyElement {
        let p = self.db.as_ref().expect("painel aberto");
        let mono = self.mono.clone();
        let accent = self.focus_color();
        let widths = col_widths(cols, rows);
        let total_w: f32 = widths.iter().sum::<f32>();
        let (base, total) = win;
        let visible = ((view.1 - HEAD_H) / ROW_H).floor().max(1.0) as usize;
        let max_scroll = total.saturating_sub(visible);
        let first = p.vscroll.min(max_scroll);
        let hscroll = p.hscroll.clamp(0.0, (total_w - view.0).max(0.0));
        let sort = p.sort.clone();

        let header = div()
            .flex_none()
            .h(px(HEAD_H))
            .flex()
            .items_center()
            .ml(px(-hscroll))
            .bg(rgb(c_panel()))
            .border_b_1()
            .border_color(rgb(c_card_border()))
            .children(cols.iter().enumerate().map(|(i, c)| {
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

        let ncols = cols.len();
        let body = (first..(first + visible).min(total)).map(|ri| {
            let bg: Option<gpui::Hsla> = if ri % 2 == 1 { Some(rgb(c_code_bg()).into()) } else { None };
            let mut row = div().flex_none().h(px(ROW_H)).flex().items_center().ml(px(-hscroll)).when_some(bg, |d, c| d.bg(c));
            for ci in 0..ncols {
                let v = ri.checked_sub(base).and_then(|k| rows.get(k)).and_then(|r| r.get(ci));
                let (text, muted) = (v.map_or(String::new(), Value::display), matches!(v, Some(Value::Null | Value::Blob { .. })));
                let mut cell = div()
                    .id(("db-cell", ri * 512 + ci))
                    .flex_none()
                    .w(px(widths.get(ci).copied().unwrap_or(80.0)))
                    .h_full()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .font_family(mono.clone())
                    .text_color(rgb(if muted { c_muted() } else { c_text() }))
                    .child(text.replace('\n', "↵"));
                if p.sel_cell == Some((ri, ci)) {
                    cell = cell.border_1().border_color(rgb(accent));
                }
                cell = cell.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.focused_pane = Some(PaneKind::Db);
                    if let Some(p) = this.db.as_mut() {
                        p.sel_cell = Some((ri, ci));
                        p.notice = None;
                        p.filter_focus = false;
                        p.sql_focus = false;
                    }
                    cx.notify();
                }));
                row = row.child(cell);
            }
            row
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
                this.db_ensure_window(cx);
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
        let form_cursor = p.manager.edit.as_ref().map_or(0, |f| f.cursor);
        let edit = p.manager.edit.as_ref().map(|f| (f.id, f.kind, f.focus, f.name.clone(), f.host.clone(), f.port.clone(), f.user.clone(), f.dbname.clone(), f.path.clone(), f.ca.clone(), f.read_only));
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
                        .child(if on {
                            let at = form_cursor.min(value.chars().count());
                            let b = value.char_indices().nth(at).map_or(value.len(), |(i, _)| i);
                            format!("{}▏{}", &value[..b], &value[b..])
                        } else {
                            value
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.focused_pane = Some(PaneKind::Db);
                            if let Some(f) = this.db.as_mut().and_then(|p| p.manager.edit.as_mut()) {
                                f.focus_on(idx);
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
        if let Some((id, kind, focus, name, host, port, user, dbname, path, ca, read_only)) = edit {
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
                            f.focus_on(0);
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
                    .child(div().flex().child(field("Certificado da CA (opcional, .pem)", ca, 5, focus, cx)))
                    .child(div().text_size(px(11.)).text_color(rgb(c_disabled())).child("Servidor em outra máquina: a conexão é criptografada (TLS) e o certificado é conferido. Se for um certificado próprio, informe o arquivo da CA."));
            } else {
                form = form.child(div().flex().child(field("Arquivo", path, 1, focus, cx)));
            }
            form = form.child(
                div()
                    .id("db-ro")
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(div().flex_none().size(px(14.)).flex().items_center().justify_center().rounded(px(3.)).border_1().border_color(rgb(if read_only { accent } else { c_muted() })).bg(rgb(if read_only { accent } else { c_bg() })).text_size(px(10.)).text_color(rgb(c_bg())).child(if read_only { "✓" } else { "" }))
                    .child(div().text_size(px(12.)).text_color(rgb(c_text())).child("Somente leitura"))
                    .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child("— protege contra mudanças sem querer; desmarcado, dá para editar e rodar INSERT, UPDATE e DELETE"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(f) = this.db.as_mut().and_then(|p| p.manager.edit.as_mut()) {
                            f.read_only = !f.read_only;
                        }
                        cx.notify();
                    })),
            );
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
                .child(cell(Some(150.0)).child("Tipo"))
                .child(cell(None).child("Endereço"))
                .child(cell(Some(262.0)).child("")),
        );
        for (n, c) in conns.iter().enumerate() {
            let id = c.id;
            let kind_label = format!("{}{}", if c.kind == Kind::Postgres { "PostgreSQL" } else { "SQLite" }, if c.read_only { " · só leitura" } else { "" });
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
                    .child(cell(Some(150.0)).text_color(rgb(c_muted())).child(kind_label))
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
                    p.sel_cell = None;
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
        let result_rows = p.result.as_ref().map(|x| match &p.stream {
            Some(s) => s.total.map_or(format!("{}+", s.base + x.rows.len()), |t| t.to_string()),
            None => x.rows.len().to_string(),
        });
        let (writable, applying) = (p.writable, p.running);
        let export_form = p.export.as_ref().map(|e| (e.path.clone(), e.json));
        let show_export_bar = export_form.is_some() && matches!(view, View::Data | View::Result);

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
            .child(div().flex_none().h(px(20.)).px(px(8.)).flex().items_center().rounded_full().bg(rgb(c_surface())).border_1().border_color(rgb(c_card_border())).text_size(px(10.)).text_color(rgb(c_muted())).child(if writable { kind_label.to_string() } else { format!("{kind_label} · só leitura") }));
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
                        let keep = this.db.as_ref().map(|p| p.view).filter(|v| matches!(v, View::Structure | View::Objects | View::Diagram)).unwrap_or(View::Data);
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
            .child(self.db_tab("db-tab-objs", format!("Objetos · {}", self.db.as_ref().map_or(0, |p| p.objects.len())), view == View::Objects, View::Objects, cx))
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
                                p.stream = None;
                                p.view = View::Data;
                            }
                            cx.notify();
                        })),
                );
        }

        // conteúdo e rodapé de cada aba
        let area = (main_w, content_h - if show_export_bar { EDIT_BAR_H } else { 0.0 });
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
                        self.db_grid("db-data", &pg.columns, &pg.rows, &types, area, true, (0, pg.rows.len()), cx)
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
                        self.db_grid("db-struct", &cols, &rows, &[], area, false, (0, rows.len()), cx)
                    }
                    None => div().flex_1().p_3().text_size(px(12.)).text_color(rgb(c_muted())).child("Escolha uma tabela.").into_any_element(),
                }
            }
            View::Objects => {
                let p = self.db.as_ref().unwrap();
                let cols = ["Tipo", "Nome", "Tabela", "Definição"].map(String::from).to_vec();
                let rows: Vec<Vec<Value>> = p
                    .objects
                    .iter()
                    .map(|o| {
                        let name = match &o.schema {
                            Some(sc) if sc != "public" => format!("{sc}.{}", o.name),
                            _ => o.name.clone(),
                        };
                        vec![Value::Text(o.kind.label().into()), Value::Text(name), o.table.clone().map_or(Value::Null, Value::Text), Value::Text(o.definition.clone())]
                    })
                    .collect();
                footer_left = format!("{} objetos · clique numa célula e use Ctrl+C para copiar a definição", rows.len());
                self.db_grid("db-objects", &cols, &rows, &[], area, false, (0, rows.len()), cx)
            }
            View::Diagram => {
                footer_left = format!("{n_tables} tabelas · {n_rel} relações");
                self.render_db_diagram(area, cx)
            }
            View::Result => {
                let res = self.db.as_ref().unwrap().result.clone();
                match res {
                    Some(res) if !res.columns.is_empty() => {
                        let stream = self.db.as_ref().unwrap().stream.clone();
                        match stream {
                            Some(st) => {
                                let end = st.base + res.rows.len();
                                // com o total desconhecido a grade deixa rolar um pouco além do que já foi lido
                                let total = st.total.map_or(if st.more { end + 100 } else { end }, |t| t as usize);
                                footer_left = format!(
                                    "linhas {}–{} lidas · {} · {} ms{}",
                                    st.base + 1,
                                    end,
                                    st.total.map_or("total ainda não contado".to_string(), |t| format!("{t} no total")),
                                    res.elapsed.as_millis(),
                                    if st.loading { " · carregando…" } else { "" }
                                );
                                self.db_grid("db-result", &res.columns, &res.rows, &[], area, false, (st.base, total), cx)
                            }
                            None => {
                                footer_left = format!("{} linhas em {} ms{}", res.rows.len(), res.elapsed.as_millis(), if res.truncated { format!(" (cortado em {DEFAULT_LIMIT})") } else { String::new() });
                                self.db_grid("db-result", &res.columns, &res.rows, &[], area, false, (0, res.rows.len()), cx)
                            }
                        }
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
        if matches!(view, View::Data | View::Result) && footer_nav.is_some() | (view == View::Result) {
            footer = footer
                .children((view == View::Result && self.db.as_ref().is_some_and(|p| p.stream.as_ref().is_some_and(|s| s.total.is_none()))).then(|| self.db_btn("db-count", "Contar", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_count(cx)))))
                .child(self.db_btn("db-copy-grid", "Copiar", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_copy_grid(cx))))
                .child(self.db_btn("db-export", "Exportar", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_open_export(cx))));
        }
        // barra de exportar: onde gravar e em que formato
        let export_bar = export_form.filter(|_| show_export_bar).map(|(path, json)| {
            div()
                .flex_none()
                .h(px(EDIT_BAR_H))
                .px(px(10.))
                .flex()
                .items_center()
                .gap_2()
                .bg(with_alpha(accent, 0x14))
                .border_b_1()
                .border_color(rgb(c_card_border()))
                .child(div().flex_none().text_size(px(12.)).text_color(rgb(c_text())).child(if view == View::Data { "Exportar a tabela inteira para:" } else { "Exportar o resultado inteiro para:" }))
                .child(div().flex_1().min_w_0().h(px(28.)).px(px(10.)).flex().items_center().rounded(px(5.)).border_1().border_color(rgb(accent)).bg(rgb(c_bg())).font_family(mono.clone()).text_size(px(12.)).overflow_hidden().whitespace_nowrap().text_color(rgb(c_text())).child(format!("{path}▏")))
                .child(self.db_btn("db-fmt", if json { "JSON" } else { "CSV" }, false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_toggle_export_format(cx))))
                .child(self.db_btn("db-export-go", if applying { "Exportando…" } else { "Exportar" }, true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_run_export(cx))))
                .child(self.db_btn("db-export-no", "Cancelar", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(p) = this.db.as_mut() {
                        p.export = None;
                    }
                    cx.notify();
                })))
        });

        // divisória e consulta embaixo
        let split = div().flex_none().h(px(SPLIT_H)).flex().items_center().justify_center().gap(px(3.)).bg(rgb(c_bg())).border_t_1().border_color(rgb(c_card_border())).children((0..3).map(|_| div().size(px(3.)).rounded_full().bg(rgb(c_disabled()))));
        let dock = self.render_db_dock(main_w > 520.0, (r.x + side_w + 40.0, r.y + r.h - DOCK_H + 36.0 + 8.0), cx);

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
            .child(div().flex_1().min_h_0().flex().child(sidebar).child(div().flex_1().min_w_0().flex().flex_col().child(tabs).children(export_bar).child(content).child(footer).child(split).child(dock)))
            .into_any_element()
    }

    /// Área da consulta, fixa embaixo: editor com destaque de SQL, executar e estado.
    fn render_db_dock(&mut self, wide: bool, origin: (f32, f32), cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let mono = self.mono.clone();
        let p = self.db.as_ref().expect("painel aberto");
        let (focus, running, error, cancelling) = (p.sql_focus, p.running, p.error.clone(), p.cancelling);
        let (scroll, n_lines) = (p.sql_scroll.min(p.editor.line_count().saturating_sub(queryedit::VISIBLE_LINES)), p.editor.line_count());
        let cursor = p.editor.cursor();
        let status = if running {
            ("Executando… clique em Cancelar para interromper".to_string(), c_muted())
        } else if let Some(e) = &error {
            (e.clone(), RED)
        } else if let Some(n) = &p.notice {
            (n.clone(), 0x8fd7ae)
        } else if let Some(res) = &p.result {
            match res.changes {
                Some(n) => (format!("{n} linha(s) afetada(s)"), c_muted()),
                None if p.stream.is_some() => (format!("Executada · {} ms", res.elapsed.as_millis()), 0x8fd7ae),
                None => (format!("Executada · {} linhas em {} ms", res.rows.len(), res.elapsed.as_millis()), 0x8fd7ae),
            }
        } else {
            ("Pronto · Ctrl+Enter executa · Alt+↑/↓ histórico".to_string(), c_muted())
        };
        let color = |t: Tok| match t {
            Tok::Plain => c_text(),
            Tok::Keyword => accent,
            Tok::Str => 0x8fd0a8,
            Tok::Num => BLUE,
            Tok::Comment => c_muted(),
        };
        // as linhas visíveis, já cortadas em pedaços (cor, seleção, cursor)
        let last_visible = (scroll + queryedit::VISIBLE_LINES).min(n_lines);
        let rows: Vec<Vec<queryedit::Seg>> = (scroll..last_visible)
            .map(|i| {
                let tokens = highlight(p.editor.line(i));
                queryedit::line_segments(&tokens, (focus && cursor.line == i).then_some(cursor.col), queryedit::selection_on_line(&p.editor, i))
            })
            .collect();
        let (ox, oy) = origin;
        let text_pos = move |ev_x: f32, ev_y: f32| (ev_x - ox, ev_y - oy);
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
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.focused_pane = Some(PaneKind::Db);
                    let (x, y) = text_pos(f32::from(ev.position.x), f32::from(ev.position.y));
                    if let Some(p) = this.db.as_mut() {
                        p.sql_focus = true;
                        p.filter_focus = false;
                        let pos = queryedit::pos_at(&p.editor, p.sql_scroll, x, y, CHAR_W);
                        if ev.click_count >= 2 {
                            p.editor.select_word_at(pos);
                        } else {
                            p.editor.move_to(pos, ev.modifiers.shift);
                            p.sql_drag = true;
                        }
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                let (x, y) = text_pos(f32::from(ev.position.x), f32::from(ev.position.y));
                if let Some(p) = this.db.as_mut().filter(|p| p.sql_drag) {
                    if ev.pressed_button == Some(MouseButton::Left) {
                        let pos = queryedit::pos_at(&p.editor, p.sql_scroll, x, y, CHAR_W);
                        p.editor.move_to(pos, true);
                        cx.notify();
                    } else {
                        p.sql_drag = false;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    if let Some(p) = this.db.as_mut() {
                        p.sql_drag = false;
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                if let Some(p) = this.db.as_mut() {
                    let dy = f32::from(ev.delta.pixel_delta(px(queryedit::LINE_H)).y);
                    let max = p.editor.line_count().saturating_sub(queryedit::VISIBLE_LINES) as i64;
                    p.sql_scroll = (p.sql_scroll as i64 - (dy / queryedit::LINE_H).round() as i64).clamp(0, max) as usize;
                }
                cx.notify();
            }))
            .child(div().flex_none().w(px(40.)).pt(px(8.)).pr(px(10.)).flex().flex_col().items_end().text_color(rgb(c_disabled())).children((scroll..last_visible).map(|i| div().h(px(queryedit::LINE_H)).child(format!("{}", i + 1)))))
            .child(div().flex_1().min_w_0().pt(px(8.)).pr(px(12.)).flex().flex_col().overflow_hidden().children(rows.into_iter().map(|segs| {
                div().h(px(queryedit::LINE_H)).flex().items_center().whitespace_nowrap().children(segs.into_iter().flat_map(|sg| {
                    let caret = sg.caret_before.then(|| div().flex_none().w(px(2.)).h(px(15.)).bg(rgb(accent)));
                    let body = (!sg.text.is_empty()).then(|| div().flex_none().text_color(rgb(color(sg.tok))).when(sg.sel, |d| d.bg(with_alpha(accent, 0x55))).child(sg.text));
                    caret.into_iter().map(IntoElement::into_any_element).chain(body.into_iter().map(IntoElement::into_any_element))
                }))
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
                    .child(div().flex_none().h(px(24.)).px(px(10.)).flex().items_center().rounded(px(5.)).bg(rgb(c_surface())).text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(rgb(accent)).child("Query"))
                    .child(div().flex_1())
                    .child(self.db_btn("db-run", if running { if cancelling { "Cancelando…" } else { "Cancelar" } } else if wide { "Executar  Ctrl+Enter" } else { "Executar" }, true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_run_sql(cx)))),
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
        let lay = dbdiagram::layout(&p.tables, &p.moved);
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
        // a tabela selecionada (a que está sendo arrastada) é desenhada por último, para ficar por cima das outras
        let order: Vec<usize> = (0..lay.placed.len()).filter(|&i| selected != Some(i)).chain(selected.filter(|&i| i < lay.placed.len())).collect();
        for i in order {
            let Some(pl) = &lay.placed[i] else { continue };
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
            let (px0, py0) = (pl.x, pl.y);
            host = host.child(card.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    if let Some(p) = this.db.as_mut() {
                        p.drag = Some(Drag { table: i, start_mouse: (f32::from(ev.position.x), f32::from(ev.position.y)), start_pos: (px0, py0), moved: false });
                    }
                    cx.stop_propagation();
                }),
            ));
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
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some(p) = this.db.as_mut() {
                    if let Some(d) = p.drag.as_mut() {
                        if ev.pressed_button != Some(MouseButton::Left) {
                            p.drag = None;
                        } else {
                            let (dx, dy) = (f32::from(ev.position.x) - d.start_mouse.0, f32::from(ev.position.y) - d.start_mouse.1);
                            if d.moved || dx.abs() + dy.abs() > 4.0 {
                                d.moved = true;
                                let pos = ((d.start_pos.0 + dx).max(0.0), (d.start_pos.1 + dy).max(0.0));
                                let t = d.table;
                                p.moved.insert(t, pos);
                            }
                            cx.notify();
                        }
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    // sem ter andado, foi um clique: seleciona a tabela
                    let click = this.db.as_mut().and_then(|p| p.drag.take()).filter(|d| !d.moved).map(|d| d.table);
                    if let Some(i) = click {
                        this.db_select_table(i, View::Diagram, cx);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if let Some(p) = this.db.as_mut() {
                        p.drag = None;
                    }
                    cx.notify();
                }),
            )
            .child(host)
            .when(empty, |d| d.child(div().p_3().text_size(px(12.)).text_color(rgb(c_muted())).child("O banco não tem tabelas para desenhar.")))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn form_fields_have_a_cursor_that_moves_and_edits_in_the_middle() {
        let mut f = EditForm::new();
        f.focus_on(1); // servidor: "localhost"
        f.step(-4); // antes de "host"
        f.insert("-é");
        assert_eq!(f.host, "local-éhost");
        f.erase(true); // apaga o "é"
        f.erase(false); // apaga o "h"
        assert_eq!(f.host, "local-ost");
        f.step(isize::MIN / 2);
        f.erase(true); // no começo não apaga nada
        f.insert(">");
        assert_eq!(f.host, ">local-ost");
        f.step(isize::MAX / 2);
        f.erase(false); // no fim não apaga nada
        f.insert("!");
        assert_eq!(f.host, ">local-ost!");
        // trocar de campo põe o cursor no fim do novo
        f.focus_on(3);
        f.insert("2");
        assert_eq!(f.user, "postgres2");
    }

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
    fn long_text_edits_keep_working_with_pasted_multiline_sql() {
        // o texto da query guarda as quebras de linha; o destaque trata linha por linha
        let sql = "SELECT 1;\nSELECT 2;\n-- fim";
        assert_eq!(sql.split('\n').count(), 3);
        assert_eq!(highlight("-- fim"), vec![("-- fim".to_string(), Tok::Comment)]);
    }

    #[test]
    fn export_names_are_safe_and_never_collide() {
        assert_eq!(export_file_name("clientes", false), "clientes.csv");
        assert_eq!(export_file_name("public.itens_pedido", true), "public.itens_pedido.json");
        assert_eq!(export_file_name("../../etc/passwd", false), ".._.._etc_passwd.csv");
        assert_eq!(export_file_name("a b/c", false), "a_b_c.csv");
        assert_eq!(export_file_name("", false), "resultado.csv");
        assert_eq!(export_file_name("///", true), "resultado.json");
        let d = std::env::temp_dir().join(format!("brasa-unique-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let base = d.join("t.csv");
        assert_eq!(unique_path(&base), base);
        std::fs::write(&base, "x").unwrap();
        assert_eq!(unique_path(&base), d.join("t-2.csv"));
        std::fs::write(d.join("t-2.csv"), "x").unwrap();
        assert_eq!(unique_path(&base), d.join("t-3.csv"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn plain_typing_never_reaches_the_hidden_terminal_but_shortcuts_do() {
        assert!(swallows_typing("a", true, false, false, false));
        assert!(swallows_typing("enter", false, false, false, false));
        assert!(swallows_typing("backspace", false, false, false, false));
        assert!(swallows_typing("space", false, false, false, false));
        // atalhos com Ctrl, Alt ou Super passam (trocar de aba, abrir configurações...)
        assert!(!swallows_typing("t", true, true, false, false));
        assert!(!swallows_typing("k", true, true, false, false));
        assert!(!swallows_typing("1", true, false, true, false));
        assert!(!swallows_typing("a", true, false, false, true));
        // teclas de função e setas não são digitação
        assert!(!swallows_typing("f5", false, false, false, false));
        assert!(!swallows_typing("left", false, false, false, false));
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
