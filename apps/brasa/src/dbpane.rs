//! Painel Banco: abre um arquivo SQLite, lista as tabelas, mostra os dados em grade paginada e roda SQL.
//! As consultas rodam em segundo plano (a janela não congela com uma consulta lenta) e o banco abre só para
//! leitura. O motor está no crate `ox-db`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui::{div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, FontWeight, MouseButton, MouseDownEvent, ScrollWheelEvent};
use ox_db::{Database, Db, Options, Page, PgConfig, PgDatabase, QueryResult, Table, Value, DEFAULT_LIMIT, DEFAULT_TIMEOUT};

use crate::icons::wicon;
use crate::pane_view::with_alpha;
use crate::panes::{PaneKind, Rect};
use crate::theme::*;
use crate::Workspace;

/// Linhas por página na grade de dados.
pub const PAGE_SIZE: usize = 100;
const ROW_H: f32 = 24.0;
const HEAD_H: f32 = 28.0;
const SIDE_W: f32 = 170.0;
/// largura média de um caractere do texto monoespaçado da grade (12 px)
const CHAR_W: f32 = 7.3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Data,
    Sql,
}

/// Que tipo de conexão o formulário monta.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnKind {
    Postgres,
    Sqlite,
}

/// Formulário de conexão. A senha só existe aqui, na memória, e é apagada assim que a conexão é aberta.
pub struct Form {
    pub kind: ConnKind,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub dbname: String,
    pub sqlite_path: String,
    /// campo com o teclado: 0 servidor, 1 porta, 2 usuário, 3 senha, 4 banco (no SQLite, 0 é o caminho)
    pub focus: usize,
    pub connecting: bool,
}

impl Default for Form {
    fn default() -> Self {
        Self { kind: ConnKind::Postgres, host: "localhost".into(), port: "5432".into(), user: "postgres".into(), password: String::new(), dbname: "postgres".into(), sqlite_path: String::new(), focus: 3, connecting: false }
    }
}

impl Form {
    fn field_count(&self) -> usize {
        if self.kind == ConnKind::Postgres {
            5
        } else {
            1
        }
    }

    fn field_mut(&mut self) -> &mut String {
        match (self.kind, self.focus) {
            (ConnKind::Sqlite, _) => &mut self.sqlite_path,
            (_, 0) => &mut self.host,
            (_, 1) => &mut self.port,
            (_, 2) => &mut self.user,
            (_, 3) => &mut self.password,
            _ => &mut self.dbname,
        }
    }

    /// Configuração do PostgreSQL a partir dos campos; erro legível se algo não serve.
    pub fn pg_config(&self) -> Result<PgConfig, String> {
        let port: u16 = self.port.trim().parse().map_err(|_| "a porta deve ser um número de 1 a 65535".to_string())?;
        if port == 0 || self.host.trim().is_empty() || self.user.trim().is_empty() || self.dbname.trim().is_empty() {
            return Err("preencha servidor, porta, usuário e banco".into());
        }
        Ok(PgConfig { host: self.host.trim().into(), port, user: self.user.trim().into(), password: (!self.password.is_empty()).then(|| self.password.clone()), dbname: self.dbname.trim().into() })
    }
}

pub struct DbPane {
    pub db: Option<Arc<Mutex<Db>>>,
    /// "SQLite" ou "PostgreSQL", e de onde veio (sem senha)
    pub kind: &'static str,
    pub label: String,
    pub tables: Vec<Table>,
    pub selected: Option<usize>,
    pub tab: Tab,
    pub page: Option<Page>,
    pub total: Option<i64>,
    pub offset: usize,
    pub sort: Option<(String, bool)>,
    pub vscroll: usize,
    pub hscroll: f32,
    pub sql: String,
    pub sql_focus: bool,
    pub result: Option<QueryResult>,
    pub running: bool,
    pub form: Form,
    pub error: Option<String>,
    /// muda a cada pedido; resposta de pedido antigo é descartada
    pub epoch: u64,
}

impl Default for DbPane {
    fn default() -> Self {
        Self {
            db: None,
            kind: "",
            label: String::new(),
            tables: Vec::new(),
            selected: None,
            tab: Tab::Data,
            page: None,
            total: None,
            offset: 0,
            sort: None,
            vscroll: 0,
            hscroll: 0.0,
            sql: "SELECT 1;".into(),
            sql_focus: false,
            result: None,
            running: false,
            form: Form::default(),
            error: None,
            epoch: 0,
        }
    }
}

/// Arquivos que o gerenciador abre no painel Banco.
pub fn is_db_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "sqlite" | "sqlite3" | "db" | "db3" | "s3db"))
}

/// Larguras das colunas (px) pelo cabeçalho e pelas primeiras linhas, entre 70 e 280.
fn col_widths(cols: &[String], rows: &[Vec<Value>]) -> Vec<f32> {
    cols.iter()
        .enumerate()
        .map(|(i, c)| {
            let longest = rows.iter().take(60).map(|r| r.get(i).map_or(0, |v| v.display().chars().take(40).count())).max().unwrap_or(0).max(c.chars().count() + 2);
            (longest as f32 * CHAR_W + 20.0).clamp(70.0, 280.0)
        })
        .collect()
}

impl Workspace {
    fn show_db_pane(&mut self) {
        if !self.panes.contains(PaneKind::Db) {
            self.toggle_widget(PaneKind::Db);
        }
        self.focused_pane = Some(PaneKind::Db);
        self.db.get_or_insert_with(DbPane::default);
    }

    /// Guarda um banco recém-aberto no painel e carrega a primeira tabela.
    fn adopt_db(&mut self, db: Db, tables: Vec<Table>, cx: &mut Context<Self>) {
        let (kind, label) = (db.kind(), db.label());
        let Some(p) = self.db.as_mut() else { return };
        let form = std::mem::take(&mut p.form);
        *p = DbPane { db: Some(Arc::new(Mutex::new(db))), kind, label, selected: (!tables.is_empty()).then_some(0), tables, form: Form { password: String::new(), connecting: false, ..form }, ..DbPane::default() };
        self.db_load_page(cx);
    }

    /// Abre um arquivo SQLite (do gerenciador de arquivos ou do formulário).
    pub fn open_database(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.show_db_pane();
        let res = Database::open(&path, &Options::default()).and_then(|d| {
            let mut db = Db::Sqlite(d);
            db.schema().map(|t| (db, t))
        });
        match res {
            Ok((db, tables)) => self.adopt_db(db, tables, cx),
            Err(e) => {
                if let Some(p) = self.db.as_mut() {
                    p.form.kind = ConnKind::Sqlite;
                    p.form.focus = 0;
                    p.form.connecting = false;
                    p.error = Some(e.to_string());
                }
            }
        }
        cx.notify();
    }

    /// Conecta ao PostgreSQL com o que está no formulário, em segundo plano.
    pub fn db_connect_pg(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        if p.form.connecting {
            return;
        }
        let cfg = match p.form.pg_config() {
            Ok(c) => c,
            Err(e) => {
                p.error = Some(e);
                return;
            }
        };
        p.error = None;
        p.form.connecting = true;
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
                    Ok((db, tables)) => ws.adopt_db(db, tables, cx),
                    Err(e) => {
                        p.form.connecting = false;
                        p.form.password.clear();
                        p.form.focus = 3;
                        p.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Fecha a conexão e volta ao formulário (o servidor, a porta e o usuário ficam; a senha não).
    pub fn db_disconnect(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.db.as_mut() {
            let form = std::mem::take(&mut p.form);
            *p = DbPane { form: Form { password: String::new(), connecting: false, focus: 3, ..form }, epoch: p.epoch + 1, ..DbPane::default() };
        }
        cx.notify();
    }

    pub fn db_select_table(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(p) = self.db.as_mut() else { return };
        if i >= p.tables.len() {
            return;
        }
        p.selected = Some(i);
        p.tab = Tab::Data;
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

    /// Roda o SQL do editor, em segundo plano.
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
                        p.vscroll = 0;
                        p.hscroll = 0.0;
                    }
                    Err(e) => {
                        p.result = None;
                        p.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Teclas com o painel Banco em foco: digitação no editor de SQL e no campo do caminho.
    pub fn on_db_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.to_lowercase();
        let (ctrl, alt) = (k.modifiers.control, k.modifiers.alt);
        let Some(p) = self.db.as_mut() else { return false };
        if p.db.is_none() {
            // formulário de conexão: Tab troca de campo, Enter conecta
            let n = p.form.field_count();
            match key.as_str() {
                "enter" => {
                    if p.form.kind == ConnKind::Postgres {
                        self.db_connect_pg(cx);
                    } else {
                        let path = PathBuf::from(expand_home(p.form.sqlite_path.trim()));
                        self.open_database(path, cx);
                    }
                    return true;
                }
                "tab" => p.form.focus = if k.modifiers.shift { (p.form.focus + n - 1) % n } else { (p.form.focus + 1) % n },
                "backspace" => {
                    p.form.field_mut().pop();
                }
                _ if !ctrl && !alt => match (&k.key_char, key.as_str()) {
                    (Some(c), _) => {
                        // a porta só aceita dígitos
                        if p.form.kind == ConnKind::Postgres && p.form.focus == 1 && !c.chars().all(|ch| ch.is_ascii_digit()) {
                            return true;
                        }
                        p.form.field_mut().push_str(c)
                    }
                    (None, "space") if !(p.form.kind == ConnKind::Postgres && p.form.focus == 1) => p.form.field_mut().push(' '),
                    _ => return false,
                },
                _ => return false,
            }
            cx.notify();
            return true;
        }
        if p.sql_focus && p.tab == Tab::Sql {
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
            "pagedown" if p.tab == Tab::Data && p.offset + PAGE_SIZE < pages => {
                p.offset += PAGE_SIZE;
                self.db_load_page(cx);
            }
            "pageup" if p.tab == Tab::Data && p.offset > 0 => {
                p.offset = p.offset.saturating_sub(PAGE_SIZE);
                self.db_load_page(cx);
            }
            "down" => p.vscroll += 1,
            "up" => p.vscroll = p.vscroll.saturating_sub(1),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Grade de resultados: cabeçalho fixo, linhas com rolagem vertical e horizontal. `sortable`: clicar no cabeçalho ordena.
    fn db_grid(&self, id: &'static str, cols: &[String], rows: &[Vec<Value>], view: (f32, f32), sortable: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = self.db.as_ref().expect("painel aberto");
        let mono = self.mono.clone();
        let widths = col_widths(cols, rows);
        let total_w: f32 = widths.iter().sum();
        let visible = ((view.1 - HEAD_H) / ROW_H).floor().max(1.0) as usize;
        let max_scroll = rows.len().saturating_sub(visible);
        let first = p.vscroll.min(max_scroll);
        let hscroll = p.hscroll.clamp(0.0, (total_w - view.0).max(0.0));
        let sort = p.sort.clone();

        let header = div().flex_none().h(px(HEAD_H)).flex().items_center().ml(px(-hscroll)).border_b_1().border_color(rgb(c_card_border())).children(cols.iter().enumerate().map(|(i, c)| {
            let arrow = match &sort {
                Some((s, true)) if sortable && s == c => " ↑",
                Some((s, false)) if sortable && s == c => " ↓",
                _ => "",
            };
            let name = c.clone();
            div()
                .id((id, i))
                .flex_none()
                .w(px(widths[i]))
                .h_full()
                .px(px(8.))
                .flex()
                .items_center()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(c_text()))
                .when(i > 0, |d| d.border_l_1().border_color(rgb(c_card_border())))
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
                .child(format!("{c}{arrow}"))
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
                        .px(px(8.))
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

    fn db_tab_button(&self, id: &'static str, label: &'static str, on: bool, tab: Tab, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = self.focus_color();
        div()
            .id(id)
            .px(px(12.))
            .h(px(26.))
            .flex()
            .items_center()
            .rounded(px(5.))
            .text_size(px(12.))
            .cursor_pointer()
            .font_weight(FontWeight::SEMIBOLD)
            .when(on, |d| d.bg(rgb(c_surface())).text_color(rgb(accent)))
            .when(!on, |d| d.text_color(rgb(c_muted())).hover(|s| s.bg(rgb(c_hover()))))
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(p) = this.db.as_mut() {
                    p.tab = tab;
                    p.vscroll = 0;
                    p.hscroll = 0.0;
                    p.sql_focus = tab == Tab::Sql;
                }
                cx.notify();
            }))
    }

    pub fn render_db_pane(&mut self, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let mono = self.mono.clone();
        let root = div().size_full().flex().flex_col().bg(rgb(c_bg())).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.focused_pane = Some(PaneKind::Db);
                cx.notify();
            }),
        );
        let Some(p) = self.db.as_ref().filter(|p| p.db.is_some()) else {
            return self.render_db_form(r, cx);
        };

        let (selected, tab, error, running) = (p.selected, p.tab, p.error.clone(), p.running);
        let path_label = p.label.clone();
        let kind_label = p.kind;
        let tables: Vec<(String, bool, usize)> = p.tables.iter().map(|t| (t.label(), t.is_view, t.columns.len())).collect();
        let main_w = r.w - SIDE_W;
        let content_h = r.h - 38.0;

        // lista de tabelas
        let list = div().flex_none().w(px(SIDE_W)).h_full().flex().flex_col().border_r_1().border_color(rgb(c_card_border())).child(
            div().flex_none().h(px(30.)).px(px(10.)).flex().items_center().text_size(px(11.)).text_color(rgb(c_muted())).font_weight(FontWeight::SEMIBOLD).child(format!("Tabelas ({})", tables.len())),
        ).child(
            div().id("db-tables").flex_1().min_h_0().overflow_hidden().flex().flex_col().children(tables.iter().enumerate().map(|(i, (name, is_view, ncols))| {
                let on = selected == Some(i);
                div()
                    .id(("db-table", i))
                    .flex_none()
                    .h(px(26.))
                    .mx(px(4.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .when(on, |d| d.bg(with_alpha(accent, 0x55)))
                    .when(!on, |d| d.hover(|s| s.bg(rgb(c_hover()))))
                    .child(wicon(if *is_view { "eye" } else { "block-bulletedlist" }, 12., if on { accent } else { c_muted() }))
                    .child(div().flex_1().min_w_0().truncate().text_color(rgb(c_text())).child(name.clone()))
                    .child(div().flex_none().text_size(px(10.)).text_color(rgb(c_disabled())).child(format!("{ncols}")))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.focused_pane = Some(PaneKind::Db);
                        this.db_select_table(i, cx);
                    }))
            })),
        ).child(
            div()
                .flex_none()
                .p(px(8.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .border_t_1()
                .border_color(rgb(c_card_border()))
                .child(div().text_size(px(10.)).text_color(rgb(c_disabled())).child(kind_label))
                .child(div().w(px(SIDE_W - 18.0)).overflow_hidden().truncate().text_size(px(11.)).text_color(rgb(c_muted())).child(path_label.clone()))
                .child(
                    div()
                        .id("db-disconnect")
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(c_card_border()))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(rgb(c_muted()))
                        .hover(|s| s.bg(rgb(c_hover())).text_color(rgb(c_text())))
                        .child("Desconectar")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_disconnect(cx))),
                ),
        );

        // barra de cima: abas, caminho e aviso de só leitura
        let bar = div()
            .flex_none()
            .h(px(36.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(c_card_border()))
            .child(self.db_tab_button("db-tab-data", "Dados", tab == Tab::Data, Tab::Data, cx))
            .child(self.db_tab_button("db-tab-sql", "SQL", tab == Tab::Sql, Tab::Sql, cx))
            .child(div().flex_1())
            .child(div().flex_none().px(px(8.)).py(px(2.)).rounded_full().bg(rgb(c_surface())).text_size(px(10.)).text_color(rgb(c_muted())).child("só leitura"));

        let body: AnyElement = match tab {
            Tab::Data => {
                let (page, total, offset) = {
                    let p = self.db.as_ref().unwrap();
                    (p.page.clone(), p.total, p.offset)
                };
                match page {
                    Some(pg) => {
                        let grid = self.db_grid("db-data", &pg.columns, &pg.rows, (main_w, content_h - 30.0), true, cx);
                        let from = if pg.rows.is_empty() { 0 } else { offset + 1 };
                        let to = offset + pg.rows.len();
                        let label = match total {
                            Some(t) => format!("{from}–{to} de {t}"),
                            None => format!("{from}–{to}"),
                        };
                        let has_next = pg.truncated;
                        let has_prev = offset > 0;
                        let nav = |id: &'static str, glyph: &'static str, on: bool, delta: i64| {
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
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(grid)
                            .child(
                                div()
                                    .flex_none()
                                    .h(px(30.))
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .border_t_1()
                                    .border_color(rgb(c_card_border()))
                                    .child(nav("db-prev", "arrow-left", has_prev, -1))
                                    .child(nav("db-next", "arrow-right", has_next, 1))
                                    .child(div().text_size(px(12.)).text_color(rgb(c_muted())).child(label)),
                            )
                            .into_any_element()
                    }
                    None => div().flex_1().p_3().text_size(px(12.)).text_color(rgb(if error.is_some() { RED } else { c_muted() })).child(error.clone().unwrap_or_else(|| if selected.is_some() { "Carregando…".into() } else { "O banco não tem tabelas.".into() })).into_any_element(),
                }
            }
            Tab::Sql => {
                let (sql, focus, result) = {
                    let p = self.db.as_ref().unwrap();
                    (p.sql.clone(), p.sql_focus, p.result.clone())
                };
                let status = if running {
                    "Executando…".to_string()
                } else if let Some(e) = &error {
                    e.clone()
                } else if let Some(res) = &result {
                    match res.changes {
                        Some(n) => format!("{n} linha(s) afetada(s)"),
                        None => format!("{} linha(s){} em {} ms", res.rows.len(), if res.truncated { format!(" (cortado em {DEFAULT_LIMIT})") } else { String::new() }, res.elapsed.as_millis()),
                    }
                } else {
                    "Ctrl+Enter executa".to_string()
                };
                let grid = result.as_ref().filter(|r| !r.columns.is_empty()).map(|r| self.db_grid("db-result", &r.columns, &r.rows, (main_w, content_h - 150.0), false, cx));
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("db-sql")
                            .flex_none()
                            .h(px(110.))
                            .m(px(8.))
                            .p(px(8.))
                            .overflow_hidden()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if focus { accent } else { c_card_border() }))
                            .bg(rgb(c_panel()))
                            .font_family(mono)
                            .text_size(px(12.))
                            .text_color(rgb(c_text()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                    if let Some(p) = this.db.as_mut() {
                                        p.sql_focus = true;
                                    }
                                    this.focused_pane = Some(PaneKind::Db);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .child(format!("{sql}{}", if focus { "▏" } else { "" })),
                    )
                    .child(
                        div()
                            .flex_none()
                            .h(px(26.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .id("db-run")
                                    .px(px(10.))
                                    .h(px(22.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(4.))
                                    .bg(rgb(accent))
                                    .text_color(rgb(c_bg()))
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.85))
                                    .child("Executar")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.db_run_sql(cx))),
                            )
                            .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(rgb(if error.is_some() { RED } else { c_muted() })).child(status)),
                    )
                    .children(grid)
                    .into_any_element()
            }
        };

        root.child(div().flex_1().min_h_0().flex().child(list).child(div().flex_1().min_w_0().flex().flex_col().child(bar).child(body))).into_any_element()
    }
}

impl Workspace {
    /// Tela de conexão: PostgreSQL (servidor, porta, usuário, senha, banco) ou arquivo SQLite.
    fn render_db_form(&mut self, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let mono = self.mono.clone();
        let p = self.db.as_ref().expect("painel");
        let f = &p.form;
        let (kind, focus, connecting, err) = (f.kind, f.focus, f.connecting, p.error.clone());
        let width = (r.w - 40.0).clamp(180.0, 420.0);

        let seg = |id: &'static str, label: &'static str, on: bool, k: ConnKind, cx: &mut Context<Self>| {
            div()
                .id(id)
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
                    if let Some(p) = this.db.as_mut() {
                        p.form.kind = k;
                        p.form.focus = if k == ConnKind::Postgres { 3 } else { 0 };
                        p.error = None;
                    }
                    cx.notify();
                }))
        };

        // um campo: rótulo em cima, caixa embaixo; a senha aparece como pontos
        let field = |label: &'static str, value: String, idx: usize, masked: bool, cx: &mut Context<Self>| {
            let on = focus == idx;
            let shown = if masked { "•".repeat(value.chars().count()) } else { value };
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().text_size(px(11.)).text_color(rgb(c_muted())).child(label))
                .child(
                    div()
                        .id(("db-field", idx))
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
                        .cursor_text()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(format!("{shown}{}", if on { "▏" } else { "" }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.focused_pane = Some(PaneKind::Db);
                            if let Some(p) = this.db.as_mut() {
                                p.form.focus = idx;
                            }
                            cx.notify();
                        })),
                )
        };

        let fields: Vec<AnyElement> = match kind {
            ConnKind::Postgres => {
                let (h, po, u, pw, d) = (f.host.clone(), f.port.clone(), f.user.clone(), f.password.clone(), f.dbname.clone());
                vec![
                    div().flex().gap_2().child(div().flex_1().min_w_0().child(field("Servidor", h, 0, false, cx))).child(div().flex_none().w(px(76.)).child(field("Porta", po, 1, false, cx))).into_any_element(),
                    field("Usuário", u, 2, false, cx).into_any_element(),
                    field("Senha", pw, 3, true, cx).into_any_element(),
                    field("Banco", d, 4, false, cx).into_any_element(),
                ]
            }
            ConnKind::Sqlite => vec![field("Arquivo", f.sqlite_path.clone(), 0, false, cx).into_any_element()],
        };

        let go_label = if connecting { "Conectando…" } else if kind == ConnKind::Postgres { "Conectar" } else { "Abrir" };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_4()
            .bg(rgb(c_bg()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.focused_pane = Some(PaneKind::Db);
                    cx.notify();
                }),
            )
            .child(wicon("database", 26., c_muted()))
            .child(div().text_size(px(14.)).text_color(rgb(c_text())).font_weight(FontWeight::SEMIBOLD).child("Conectar a um banco"))
            .child(
                div()
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().flex().gap_1().p(px(2.)).rounded_md().bg(rgb(c_code_bg())).child(seg("db-k-pg", "PostgreSQL", kind == ConnKind::Postgres, ConnKind::Postgres, cx)).child(seg("db-k-sq", "Arquivo SQLite", kind == ConnKind::Sqlite, ConnKind::Sqlite, cx)))
                    .children(fields)
                    .child(
                        div()
                            .id("db-connect")
                            .h(px(32.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(rgb(accent))
                            .text_color(rgb(c_bg()))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(!connecting, |d| d.cursor_pointer().hover(|s| s.opacity(0.85)))
                            .when(connecting, |d| d.opacity(0.6))
                            .child(go_label)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                let pg = this.db.as_ref().is_some_and(|p| p.form.kind == ConnKind::Postgres);
                                if pg {
                                    this.db_connect_pg(cx);
                                } else if let Some(path) = this.db.as_ref().map(|p| PathBuf::from(expand_home(p.form.sqlite_path.trim()))) {
                                    this.open_database(path, cx);
                                }
                            })),
                    )
                    .when_some(err, |d, e| d.child(div().text_size(px(12.)).text_color(rgb(RED)).child(e)))
                    .child(div().text_size(px(11.)).text_color(rgb(c_disabled())).child(if kind == ConnKind::Postgres {
                        "Abre só para leitura. A senha não é gravada. Servidor remoto: use um túnel SSH (ssh -L) por enquanto."
                    } else {
                        "Abre só para leitura. Também dá para dar dois cliques no arquivo, no gerenciador de arquivos."
                    })),
            )
            .into_any_element()
    }
}

/// `~/x` vira o caminho dentro da pasta pessoal.
fn expand_home(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => crate::home().join(rest).display().to_string(),
        None => path.to_string(),
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
        assert_eq!(w[0], 70.0); // curta: no mínimo
        assert_eq!(w[1], 280.0); // longa: no máximo
        assert!(col_widths(&[], &[]).is_empty());
    }

    #[test]
    fn home_is_expanded_only_at_the_start() {
        assert_eq!(expand_home("/tmp/x.db"), "/tmp/x.db");
        assert!(expand_home("~/x.db").ends_with("/x.db") && !expand_home("~/x.db").starts_with('~'));
        assert_eq!(expand_home("a~/b"), "a~/b");
    }
}
