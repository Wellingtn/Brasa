//! Painéis na tela, como no Wave: cada terminal ou painel de desempenho num bloco com cabeçalho e
//! contorno (o bloco em foco ganha a cor da seção ativa na barra lateral). Os painéis de desempenho
//! — CPU, Memória e Processos — seguem o "sysinfo" e o "processviewer" do Wave e são abertos pela
//! barra de widgets à direita.

use gpui::{
    canvas, div, linear_color_stop, linear_gradient, point, prelude::*, px, quad, rgb, rgba, size, AnyElement, Bounds, ClickEvent, ClipboardItem, Context, Div,
    FontWeight, MouseButton, MouseDownEvent, PathBuilder, ScrollWheelEvent, Stateful,
};

use crate::icons::wicon;
use crate::panes::{PaneKind, Rect};
use crate::sysmon::{self, human_bytes, short_bytes, ProcInfo, SortKey, HISTORY};
use crate::theme::*;
use crate::{Side, Workspace, TITLE_H};

/// espaço entre painéis (e em volta deles) quando há mais de um
pub const PANE_GAP: f32 = 6.0;
/// altura do cabeçalho de cada painel
pub const PANE_HEAD: f32 = 32.0;
/// contorno de cada painel (o em foco muda só de cor, não de espessura)
const PANE_BORDER: f32 = 2.0;
/// largura da barra de widgets à direita
pub const WIDGET_BAR_W: f32 = 60.0;
const ROW_H: f32 = 24.0;
/// cor do gerenciador de arquivos (ícones de pasta)
const FILES_BLUE: u32 = 0x5aa9e6;
const FILE_ROW_H: f32 = 23.0;

pub fn inset(r: Rect, by: f32) -> Rect {
    Rect { x: r.x + by, y: r.y + by, w: (r.w - by * 2.0).max(40.0), h: (r.h - by * 2.0).max(40.0) }
}

/// Área útil de um painel: dentro do contorno e abaixo do cabeçalho.
pub fn content_rect(r: Rect) -> Rect {
    let b = PANE_BORDER;
    Rect { x: r.x + b, y: r.y + b + PANE_HEAD, w: (r.w - 2.0 * b).max(40.0), h: (r.h - 2.0 * b - PANE_HEAD).max(40.0) }
}

/// Estado da tabela de processos: ordenação, busca, rolagem, seleção e menu de um processo.
pub struct MonitorUi {
    pub sort: SortKey,
    pub desc: bool,
    pub filter: String,
    pub filter_focused: bool,
    /// campo de busca aberto (lupa no resumo)
    pub search_open: bool,
    pub scroll: usize,
    pub wheel: f32,
    pub selected: Option<i32>,
    /// menu do processo: (pid, x, y relativos ao painel)
    pub menu: Option<(i32, f32, f32)>,
    /// resultado da última ação (mensagem, é erro)
    pub status: Option<(String, bool)>,
    /// mouse sobre a tabela: a lista fica parada para dar tempo de clicar
    pub hovered: bool,
    /// de quantos em quantos segundos a lista é relida (1, 2 ou 5)
    pub interval_secs: u32,
}

impl Default for MonitorUi {
    fn default() -> Self {
        MonitorUi { sort: SortKey::Cpu, desc: true, filter: String::new(), filter_focused: false, search_open: false, scroll: 0, wheel: 0.0, selected: None, menu: None, status: None, hovered: false, interval_secs: 2 }
    }
}

fn load_color(pct: f32) -> u32 {
    if pct < 50.0 {
        TEAL
    } else if pct < 80.0 {
        YELLOW
    } else {
        CORAL
    }
}

/// Cor com transparência (0–255).
fn with_alpha(color: u32, a: u8) -> gpui::Rgba {
    rgba((color << 8) | a as u32)
}

/// Ícone desenhado de "dividir": um retângulo com a linha no meio (vertical = lado a lado).
fn split_glyph(row: bool, color: u32) -> Div {
    let line = div().absolute().bg(rgb(color));
    div()
        .relative()
        .w(px(14.))
        .h(px(12.))
        .rounded(px(2.))
        .border_1()
        .border_color(rgb(color))
        .child(if row { line.left(px(5.)).top_0().bottom_0().w(px(1.)) } else { line.top(px(4.)).left_0().right_0().h(px(1.)) })
}

/// Ícone de gráfico de linha (CPU), desenhado.
fn chart_glyph(color: u32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b: Bounds<gpui::Pixels>, _, window, _| {
            let (x, y, w, h) = (b.origin.x, b.origin.y, b.size.width, b.size.height);
            let pts = [(0.0, 0.85), (0.3, 0.45), (0.55, 0.65), (1.0, 0.1)];
            let mut pb = PathBuilder::stroke(px(1.6));
            for (i, (fx, fy)) in pts.iter().enumerate() {
                let p = point(x + w * *fx, y + h * *fy);
                if i == 0 {
                    pb.move_to(p);
                } else {
                    pb.line_to(p);
                }
            }
            if let Ok(path) = pb.build() {
                window.paint_path(path, rgb(color));
            }
        },
    )
    .size(px(18.))
}

/// Olho (mostrar ocultos); riscado quando os ocultos estão escondidos, como no Wave.
fn eye_glyph(struck: bool) -> Div {
    div().relative().size(px(16.)).flex().items_center().justify_center().child(wicon("eye", 15., MUTED)).when(struck, |d| {
        d.child(
            canvas(
                |_, _, _| {},
                |b: Bounds<gpui::Pixels>, _, window, _| {
                    let mut pb = PathBuilder::stroke(px(1.5));
                    pb.move_to(point(b.origin.x + px(2.), b.origin.y + px(2.)));
                    pb.line_to(point(b.origin.x + b.size.width - px(2.), b.origin.y + b.size.height - px(2.)));
                    if let Ok(path) = pb.build() {
                        window.paint_path(path, rgb(MUTED));
                    }
                },
            )
            .absolute()
            .size(px(16.)),
        )
    })
}

/// Ícone de barras (memória), desenhado.
fn bars_glyph(color: u32) -> Div {
    div()
        .w(px(18.))
        .h(px(16.))
        .flex()
        .items_end()
        .gap(px(2.))
        .children([7.0, 13.0, 10.0, 16.0].map(|h| div().w(px(3.)).h(px(h)).rounded(px(1.)).bg(rgb(color))))
}

fn widget_glyph(kind: PaneKind, color: u32) -> AnyElement {
    match kind {
        PaneKind::Cpu => chart_glyph(color).into_any_element(),
        PaneKind::Mem => bars_glyph(color).into_any_element(),
        PaneKind::Procs => wicon("block-bulletedlist", 17., color).into_any_element(),
        PaneKind::Files => wicon("folder-closed", 17., color).into_any_element(),
        PaneKind::Term(_) => wicon("terminal", 17., color).into_any_element(),
    }
}

fn widget_title(kind: PaneKind) -> &'static str {
    match kind {
        PaneKind::Cpu => "CPU",
        PaneKind::Mem => "Memória",
        PaneKind::Procs => "Processos",
        PaneKind::Files => "Arquivos",
        PaneKind::Term(_) => "Terminal",
    }
}

fn head_btn(id: impl Into<gpui::ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(24.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(BTN_HOVER)))
}

/// Gráfico de linha com área preenchida, eixo Y (0–100 %) e marcas de tempo, como o sysinfo do Wave.
/// `values` vão do mais antigo ao mais recente, um por segundo; o mais recente fica na borda direita.
fn line_chart(values: Vec<f32>, color: u32, w: f32, h: f32, unit: &str) -> Div {
    const AXIS_W: f32 = 38.0;
    const TIME_H: f32 = 20.0;
    const TOP: f32 = 26.0;
    let plot_w = (w - AXIS_W - 8.0).max(40.0);
    let plot_h = (h - TIME_H - TOP).max(30.0);
    // passo do eixo Y: o menor que deixe ~26 px entre marcas
    let step = [5.0f32, 10.0, 20.0, 25.0, 50.0].into_iter().find(|s| plot_h * s / 100.0 >= 26.0).unwrap_or(100.0);
    let ticks: Vec<f32> = (0..=(100.0 / step) as usize).map(|i| i as f32 * step).collect();
    // marcas de tempo a cada 10 s × k, sem encavalar os rótulos
    let per_sec = plot_w / (HISTORY - 1) as f32;
    let every = [10usize, 20, 30, 60].into_iter().find(|k| per_sec * *k as f32 >= 52.0).unwrap_or(120);
    let marks: Vec<usize> = (0..HISTORY).step_by(every).collect();

    let (ticks_p, marks_p) = (ticks.clone(), marks.clone());
    let plot = canvas(
        |_, _, _| {},
        move |b: Bounds<gpui::Pixels>, _, window, _| {
            let (x0, y0, pw, ph) = (b.origin.x, b.origin.y, b.size.width, b.size.height);
            let grid = with_alpha(0x3a2f2c, 0xff);
            for t in &ticks_p {
                let y = y0 + ph * (1.0 - t / 100.0);
                window.paint_quad(quad(Bounds::new(point(x0, y), size(pw, px(1.))), px(0.), grid, px(0.), gpui::transparent_black(), Default::default()));
            }
            for s in &marks_p {
                let x = x0 + pw * (1.0 - *s as f32 / (HISTORY - 1) as f32);
                window.paint_quad(quad(Bounds::new(point(x, y0), size(px(1.), ph)), px(0.), with_alpha(0x2c2321, 0xff), px(0.), gpui::transparent_black(), Default::default()));
            }
            if values.len() < 2 {
                return;
            }
            let n = values.len();
            let pt = |i: usize| {
                let ago = (n - 1 - i) as f32;
                point(x0 + pw * (1.0 - ago / (HISTORY - 1) as f32), y0 + ph * (1.0 - values[i].clamp(0.0, 100.0) / 100.0))
            };
            // área: da base, pelos pontos, de volta à base
            let mut area = PathBuilder::fill();
            area.move_to(point(pt(0).x, y0 + ph));
            for i in 0..n {
                area.line_to(pt(i));
            }
            area.line_to(point(pt(n - 1).x, y0 + ph));
            area.close();
            if let Ok(path) = area.build() {
                window.paint_path(path, linear_gradient(180., linear_color_stop(with_alpha(color, 0x70), 0.0), linear_color_stop(with_alpha(color, 0x08), 1.0)));
            }
            let mut line = PathBuilder::stroke(px(1.6));
            line.move_to(pt(0));
            for i in 1..n {
                line.line_to(pt(i));
            }
            if let Ok(path) = line.build() {
                window.paint_path(path, rgb(color));
            }
        },
    )
    .w(px(plot_w))
    .h(px(plot_h));

    let label = |text: String| div().absolute().text_size(px(11.)).text_color(rgb(MUTED)).whitespace_nowrap().child(text);
    div()
        .relative()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .font_family("Hack")
        // unidade no topo do eixo, como "↑ %" no Wave
        .child(label(format!("↑ {unit}")).left(px(4.)).top_0())
        .children(ticks.iter().map(|t| label(format!("{t:.0} –")).right(px(w - AXIS_W + 2.0)).top(px(TOP + plot_h * (1.0 - t / 100.0) - 7.0))))
        .child(div().absolute().left(px(AXIS_W)).top(px(TOP)).child(plot))
        .children(marks.iter().map(|s| {
            let x = AXIS_W + plot_w * (1.0 - *s as f32 / (HISTORY - 1) as f32);
            label(if *s == 0 { "agora".into() } else { format!("-{s}s") }).top(px(TOP + plot_h + 4.0)).left(px((x - 14.0).max(AXIS_W - 6.0)))
        }))
}

impl Workspace {
    /// Cor do contorno do painel em foco: a da seção ativa na barra lateral.
    fn focus_color(&self) -> u32 {
        match self.side {
            Side::Terminals => CORAL,
            Side::Notes => AMBER,
        }
    }

    /// Todos os painéis da área principal.
    pub fn render_panes(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let multi = self.pane_rects.len() > 1;
        let (x0, y0) = (1.0 + self.sidebar_w() + self.notes_w(), 1.0 + TITLE_H);
        let focused = self.active;
        let focus_kind = self.focused_pane.unwrap_or(PaneKind::Term(self.session()));
        let accent = self.focus_color();
        let mut out: Vec<AnyElement> = Vec::new();
        for (kind, r) in self.pane_rects.clone() {
            let body_rect = if multi { content_rect(r) } else { r };
            let (body, title, running): (AnyElement, String, bool) = match kind {
                PaneKind::Term(sid) => {
                    let Some(idx) = self.tabs.iter().position(|t| t.session == sid) else { continue };
                    // desenha a sessão como se fosse a ativa (as funções de desenho usam `self.active`)
                    self.active = idx;
                    self.focused_render = idx == focused;
                    let body = self.render_terminal_area(cx);
                    self.active = focused;
                    self.focused_render = true;
                    let (folder, detail) = self.session_label(idx);
                    let running = !self.tabs[idx].cmd_starts.is_empty();
                    // terminal com nome: o nome no cabeçalho do painel; senão pasta · último comando
                    let title = match &self.tabs[idx].name {
                        Some(n) => format!("{n}  ·  {folder}"),
                        None => format!("{folder}  ·  {detail}"),
                    };
                    (body, title, running)
                }
                PaneKind::Cpu => (self.render_cpu(body_rect), widget_title(kind).to_string(), false),
                PaneKind::Mem => (self.render_mem(body_rect), widget_title(kind).to_string(), false),
                PaneKind::Procs => (self.render_procs(body_rect, cx), widget_title(kind).to_string(), false),
                PaneKind::Files => {
                    let title = self.files.as_ref().map(|f| match &f.viewing {
                        Some(img) => img.display().to_string(),
                        None => crate::files_pane::path_label(&f.dir, &crate::home()),
                    });
                    // caminho longo: corta o começo (o fim é o que interessa), como no Wave
                    let fit = ((r.w - 170.0) / 7.3).max(8.0) as usize;
                    let title = title.unwrap_or_default();
                    let n = title.chars().count();
                    let title = if n > fit { format!("…{}", title.chars().skip(n - fit + 1).collect::<String>()) } else { title };
                    (self.render_files_pane(body_rect, cx), title, false)
                }
            };
            let is_focus = kind == focus_kind;
            if !multi {
                out.push(div().absolute().left(px(r.x - x0)).top(px(r.y - y0)).w(px(r.w)).h(px(r.h)).flex().flex_col().child(body).into_any_element());
                continue;
            }
            let pane_id = match kind {
                PaneKind::Term(s) => s as usize,
                PaneKind::Cpu => usize::MAX,
                PaneKind::Mem => usize::MAX - 1,
                PaneKind::Procs => usize::MAX - 2,
                PaneKind::Files => usize::MAX - 3,
            };
            let icon_color = if running { CORAL } else if is_focus { accent } else { MUTED };
            let header = div()
                .id(("pane-head", pane_id))
                .flex_none()
                .h(px(PANE_HEAD))
                .px(px(10.))
                .flex()
                .items_center()
                .gap_2()
                .bg(rgb(PANEL))
                .border_b_1()
                .border_color(rgb(CARD_BORDER))
                .when(kind == PaneKind::Files, |d| {
                    d.child(head_btn("files-back").child(wicon("arrow-left", 14., MUTED)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(f) = this.files.as_mut() {
                            f.go_back();
                        }
                        cx.notify();
                    })))
                })
                .child(div().flex_none().w(px(16.)).flex().justify_center().child(match kind {
                    PaneKind::Term(_) => wicon("terminal", 13., icon_color).into_any_element(),
                    k => widget_glyph(k, icon_color),
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .when(kind.is_widget() && kind != PaneKind::Files, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .when(kind == PaneKind::Files, |d| d.font_family(self.mono.clone()).text_size(px(12.)))
                        .text_color(rgb(if is_focus { TEXT } else { MUTED }))
                        .child(title),
                )
                .when(matches!(kind, PaneKind::Term(_)), |d| {
                    let PaneKind::Term(sid) = kind else { unreachable!() };
                    d.child(head_btn(("pane-split-r", pane_id)).child(split_glyph(true, MUTED)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.focus_pane(sid);
                        this.split_new(true);
                        cx.notify();
                    })))
                    .child(head_btn(("pane-split-d", pane_id)).child(split_glyph(false, MUTED)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.focus_pane(sid);
                        this.split_new(false);
                        cx.notify();
                    })))
                })
                .when(kind == PaneKind::Files, |d| {
                    let hidden = self.files.as_ref().is_some_and(|f| f.show_hidden);
                    d.child(head_btn("files-hidden").child(eye_glyph(!hidden)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(f) = this.files.as_mut() {
                            f.show_hidden = !f.show_hidden;
                            f.reload();
                        }
                        cx.notify();
                    })))
                    .child(head_btn("files-refresh").child(wicon("refresh", 13., MUTED)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(f) = this.files.as_mut() {
                            f.reload();
                        }
                        cx.notify();
                    })))
                })
                .child(head_btn(("pane-close", pane_id)).child(wicon("x-close", 13., MUTED)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.close_pane(kind);
                    cx.notify();
                })));
            out.push(
                div()
                    .id(("pane", pane_id))
                    .absolute()
                    .left(px(r.x - x0))
                    .top(px(r.y - y0))
                    .w(px(r.w))
                    .h(px(r.h))
                    .flex()
                    .flex_col()
                    .rounded(px(8.))
                    .border(px(PANE_BORDER))
                    .border_color(rgb(if is_focus { accent } else { CARD_BORDER }))
                    .bg(rgb(BG))
                    .overflow_hidden()
                    // clicar em qualquer ponto do painel o põe em foco
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            match kind {
                                PaneKind::Term(sid) => {
                                    this.focused_pane = None;
                                    this.focus_pane(sid);
                                }
                                k => this.focused_pane = Some(k),
                            }
                            cx.notify();
                        }),
                    )
                    .child(header)
                    .child(div().flex_1().min_h_0().flex().flex_col().child(body))
                    .into_any_element(),
            );
        }
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(if multi { 0x0f0b0b } else { BG }))
            .children(out)
            .into_any_element()
    }

    /// Foco do teclado para o terminal do painel.
    pub fn focus_pane(&mut self, sid: ox_pty::SessionId) {
        if let Some(i) = self.tabs.iter().position(|t| t.session == sid) {
            if i != self.active {
                self.switch_tab(i);
            }
        }
    }

    /// Barra de widgets à direita, como no Wave: abre o gerenciador de arquivos e os painéis de desempenho.
    pub fn render_widget_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: &'static str, kind: PaneKind, open: bool, color: u32| {
            div()
                .id(id)
                .w(px(WIDGET_BAR_W - 8.0))
                .py(px(8.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .rounded_md()
                .cursor_pointer()
                .when(open, |d| d.bg(rgb(SURFACE)))
                .when(!open, |d| d.hover(|s| s.bg(rgb(HOVER))))
                .child(div().h(px(18.)).flex().items_center().child(widget_glyph(kind, if open { color } else { MUTED })))
                .child(div().text_size(px(11.)).text_color(rgb(if open { TEXT } else { MUTED })).child(widget_title(kind)))
        };
        let open = |k: PaneKind| self.panes.contains(k);
        div()
            .w(px(WIDGET_BAR_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .pt_2()
            .bg(rgb(BG))
            .rounded_br(px(self.corner_r))
            .border_l_1()
            .border_color(rgb(DIVIDER))
            .child(item("w-files", PaneKind::Files, open(PaneKind::Files), FILES_BLUE).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_widget(PaneKind::Files);
                cx.notify();
            })))
            .child(item("w-cpu", PaneKind::Cpu, open(PaneKind::Cpu), PEACH).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_widget(PaneKind::Cpu);
                cx.notify();
            })))
            .child(item("w-mem", PaneKind::Mem, open(PaneKind::Mem), AMBER).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_widget(PaneKind::Mem);
                cx.notify();
            })))
            .child(item("w-procs", PaneKind::Procs, open(PaneKind::Procs), YELLOW).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_widget(PaneKind::Procs);
                cx.notify();
            })))
            .into_any_element()
    }

    // ----- CPU e memória -----

    /// Linha de destaque acima do gráfico: valor atual grande + detalhes.
    fn chart_summary(&self, value: String, color: u32, detail: String) -> Div {
        div()
            .flex_none()
            .h(px(34.))
            .px(px(12.))
            .flex()
            .items_baseline()
            .gap(px(10.))
            .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(color)).child(value))
            .child(div().min_w_0().truncate().text_size(px(12.)).text_color(rgb(MUTED)).child(detail))
    }

    fn render_cpu(&self, r: Rect) -> AnyElement {
        let m = &self.monitor;
        let summary = self.chart_summary(
            format!("{:.0}%", m.cpu),
            PEACH,
            format!("{} núcleos · carga {:.2} {:.2} {:.2}", m.cores.len().max(1), m.load.0, m.load.1, m.load.2),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .pt_2()
            .bg(rgb(BG))
            .child(summary)
            .child(line_chart(m.cpu_hist.iter().copied().collect(), PEACH, r.w - 12.0, (r.h - 34.0 - 16.0).max(60.0), "%"))
            .into_any_element()
    }

    fn render_mem(&self, r: Rect) -> AnyElement {
        let m = &self.monitor;
        let pct = if m.mem_total_kb > 0 { m.mem_used_kb as f32 * 100.0 / m.mem_total_kb as f32 } else { 0.0 };
        let swap = if m.swap_total_kb > 0 {
            format!(" · swap {} de {}", human_bytes(m.swap_used_kb * 1024), human_bytes(m.swap_total_kb * 1024))
        } else {
            String::new()
        };
        let summary = self.chart_summary(format!("{pct:.0}%"), AMBER, format!("{} de {}{swap}", human_bytes(m.mem_used_kb * 1024), human_bytes(m.mem_total_kb * 1024)));
        div()
            .size_full()
            .flex()
            .flex_col()
            .pt_2()
            .bg(rgb(BG))
            .child(summary)
            .child(line_chart(m.mem_hist.iter().copied().collect(), AMBER, r.w - 12.0, (r.h - 34.0 - 16.0).max(60.0), "%"))
            .into_any_element()
    }

    // ----- processos -----

    fn render_procs(&mut self, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let m = &self.monitor;
        let filter = self.mon.filter.trim().to_lowercase();
        let mut procs: Vec<ProcInfo> = m
            .procs
            .iter()
            .filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter) || p.command.to_lowercase().contains(&filter) || p.pid.to_string().starts_with(&filter) || p.user.to_lowercase().contains(&filter))
            .cloned()
            .collect();
        sysmon::sort_procs(&mut procs, self.mon.sort, self.mon.desc);
        let total = m.procs.len();
        let search_h = if self.mon.search_open { 36.0 } else { 0.0 };
        let visible = ((r.h - 26.0 - 26.0 - search_h) / ROW_H).floor().max(1.0) as usize;
        self.mon.scroll = self.mon.scroll.min(procs.len().saturating_sub(visible));
        let first = self.mon.scroll;
        let mono = self.mono.clone();

        // resumo: carga, memória, núcleos e uso total; à direita, contagem e lupa
        let sep = || div().w(px(1.)).h(px(12.)).bg(rgb(CARD_BORDER));
        let kv = |k: &str, v: String| div().flex().gap(px(6.)).child(div().text_color(rgb(MUTED)).child(k.to_string())).child(div().text_color(rgb(TEXT)).child(v));
        let summary = div()
            .flex_none()
            .h(px(26.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(rgb(CARD_BORDER))
            .font_family(mono.clone())
            .text_size(px(12.))
            .overflow_hidden()
            .child(div().flex_none().size(px(8.)).rounded_full().bg(rgb(TEAL)))
            // painel estreito: sem a carga, para "pausado" e o intervalo continuarem visíveis
            .when(r.w >= 720.0, |d| d.child(kv("Carga", format!("{:.2} {:.2} {:.2}", m.load.0, m.load.1, m.load.2))).child(sep()))
            .child(kv("Mem", format!("{} / {}", short_bytes(m.mem_used_kb * 1024), short_bytes(m.mem_total_kb * 1024))))
            .when(r.w >= 640.0, |d| d.child(sep()).child(kv(&format!("CPU×{}", m.cores.len().max(1)), format!("{:.1}%", m.cpu))))
            .child(div().flex_1())
            .when_some(self.mon.status.clone(), |d, (msg, err)| d.child(div().min_w_0().truncate().text_color(rgb(if err { RED } else { TEAL })).child(msg)))
            .when(self.mon.hovered || self.mon.menu.is_some(), |d| d.child(div().text_color(rgb(YELLOW)).child("pausado")))
            .child(
                // intervalo de atualização: clique alterna 1 s → 2 s → 5 s
                div()
                    .id("mon-interval")
                    .px(px(6.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(CARD_BORDER))
                    .text_color(rgb(MUTED))
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(TEXT)).bg(rgb(HOVER)))
                    .child(format!("⟳ {} s", self.mon.interval_secs))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.mon.interval_secs = match this.mon.interval_secs {
                            1 => 2,
                            2 => 5,
                            _ => 1,
                        };
                        cx.notify();
                    })),
            )
            .child(kv("Procs", total.to_string()))
            .child(
                head_btn("mon-search")
                    .when(self.mon.search_open, |d| d.bg(rgb(SURFACE)))
                    .child(wicon("search", 13., if self.mon.search_open { TEXT } else { MUTED }))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.mon.search_open = !this.mon.search_open;
                        this.mon.filter_focused = this.mon.search_open;
                        if !this.mon.search_open {
                            this.mon.filter.clear();
                        }
                        cx.notify();
                    })),
            );

        let focused = self.mon.filter_focused;
        let search = div()
            .flex_none()
            .h(px(36.))
            .px(px(12.))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(CARD_BORDER))
            .child(
                div()
                    .id("mon-filter")
                    .flex_1()
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if focused { ACCENT } else { CARD_BORDER }))
                    .text_size(px(13.))
                    .child(if self.mon.filter.is_empty() {
                        div().text_color(rgb(DISABLED)).child("Filtrar por nome, PID ou usuário")
                    } else {
                        div().flex_1().min_w_0().truncate().child(format!("{}{}", self.mon.filter, if focused { "▏" } else { "" }))
                    })
                    .child(div().flex_none().text_size(px(12.)).text_color(rgb(DISABLED)).child(if filter.is_empty() { String::new() } else { format!("{} de {total}", procs.len()) }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.mon.filter_focused = true;
                            this.focus_editor = false;
                            this.filter_focused = false;
                            cx.notify();
                        }),
                    ),
            );

        // painel estreito: some com Status, Usuário e NT para o nome continuar legível
        let narrow = r.w < 560.0;
        let all_cols: [(&str, SortKey, Option<f32>, bool); 7] = [
            ("PID", SortKey::Pid, Some(70.0), true),
            ("Comando", SortKey::Command, None, false),
            ("Status", SortKey::Pid, Some(80.0), false),
            ("Usuário", SortKey::User, Some(100.0), false),
            ("NT", SortKey::Threads, Some(40.0), true),
            ("CPU%", SortKey::Cpu, Some(70.0), true),
            ("Memória", SortKey::Mem, Some(76.0), true),
        ];
        let cols: Vec<_> = all_cols.into_iter().filter(|c| !narrow || !matches!(c.0, "Status" | "Usuário" | "NT")).collect();
        let cell = |w: Option<f32>, right: bool| {
            let d = div().px(px(8.)).flex().items_center().overflow_hidden().whitespace_nowrap();
            let d = match w {
                Some(w) => d.flex_none().w(px(w)),
                None => d.flex_1().min_w_0(),
            };
            if right {
                d.justify_end()
            } else {
                d
            }
        };
        let header = div()
            .flex_none()
            .h(px(26.))
            .pr(px(10.))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(CARD_BORDER))
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(MUTED))
            .children(cols.iter().enumerate().map(|(i, (label, key, w, right))| {
                let key = *key;
                let sortable = *label != "Status";
                let active = sortable && self.mon.sort == key;
                let arrow = if !active {
                    ""
                } else if self.mon.desc {
                    " ↓"
                } else {
                    " ↑"
                };
                cell(*w, *right)
                    .id(("mon-col", i))
                    .h_full()
                    .when(active, |d| d.text_color(rgb(TEXT)))
                    .when(sortable, |d| d.cursor_pointer().hover(|s| s.text_color(rgb(TEXT))))
                    .child(format!("{label}{arrow}"))
                    .when(sortable, |d| {
                        d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if this.mon.sort == key {
                                this.mon.desc = !this.mon.desc;
                            } else {
                                this.mon.sort = key;
                                // números começam do maior; texto em ordem alfabética
                                this.mon.desc = !matches!(key, SortKey::Command | SortKey::User | SortKey::Pid);
                            }
                            cx.notify();
                        }))
                    })
            }));

        let rows = procs.iter().skip(first).take(visible).map(|p| {
            let pid = p.pid;
            let sel = self.mon.selected == Some(pid);
            div()
                .id(("proc", pid as usize))
                .flex_none()
                .h(px(ROW_H))
                .pr(px(10.))
                .flex()
                .items_center()
                .font_family(mono.clone())
                .text_size(px(12.5))
                .cursor_pointer()
                .when(sel, |d| d.bg(rgb(SURFACE)))
                .when(!sel, |d| d.hover(|s| s.bg(rgb(HOVER))))
                .child(cell(Some(70.0), true).text_color(rgb(MUTED)).child(pid.to_string()))
                .child(cell(None, false).text_color(rgb(TEXT)).child(p.name.clone()))
                .when(!narrow, |d| {
                    d.child(cell(Some(80.0), false).text_size(px(11.5)).text_color(rgb(if p.state == 'R' { TEAL } else if p.state == 'Z' { RED } else { MUTED })).child(sysmon::state_label(p.state)))
                        .child(cell(Some(100.0), false).text_color(rgb(MUTED)).child(p.user.clone()))
                        .child(cell(Some(40.0), true).text_color(rgb(MUTED)).child(p.threads.to_string()))
                })
                .child(cell(Some(70.0), true).text_color(rgb(if p.cpu >= 1.0 { load_color(p.cpu.min(100.0)) } else { TEXT })).child(if p.cpu >= 10.0 {
                    format!("{:.1}%", p.cpu)
                } else {
                    format!("{:.2}%", p.cpu)
                }))
                .child(cell(Some(76.0), true).text_color(rgb(TEXT)).child(short_bytes(p.mem_bytes)))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.mon.selected = Some(pid);
                    this.mon.menu = None;
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.mon.selected = Some(pid);
                        this.mon.menu = Some((pid, f32::from(ev.position.x) - r.x, f32::from(ev.position.y) - r.y));
                        cx.notify();
                    }),
                )
        });

        let scroll_total = procs.len();
        let table = div()
            .id("mon-table")
            .relative()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.mon.hovered = *hovered;
                cx.notify();
            }))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                this.mon.wheel -= f32::from(ev.delta.pixel_delta(px(ROW_H)).y) / ROW_H;
                let steps = this.mon.wheel.trunc() as i64;
                this.mon.wheel -= steps as f32;
                let max = scroll_total.saturating_sub(visible) as i64;
                this.mon.scroll = (this.mon.scroll as i64 + steps).clamp(0, max) as usize;
                cx.notify();
            }))
            .children(rows)
            .when(scroll_total > visible, |d| {
                let track = visible as f32 * ROW_H;
                let len = (track * visible as f32 / scroll_total as f32).max(24.0);
                let top = (track - len) * first as f32 / (scroll_total - visible) as f32;
                d.child(div().absolute().right(px(2.)).top(px(top)).w(px(5.)).h(px(len)).rounded_full().bg(rgb(BTN_HOVER)))
            })
            .when(procs.is_empty(), |d| {
                d.child(div().p_3().text_size(px(13.)).text_color(rgb(MUTED)).child(if total == 0 { "Lendo processos…" } else { "Nenhum processo combina com o filtro." }))
            });

        let menu = self.mon.menu.map(|(pid, mx, my)| self.render_proc_menu(pid, mx.min(r.w - 230.0).max(4.0), my.min(r.h - 250.0).max(4.0), cx));

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.mon.menu = None;
                    cx.notify();
                }),
            )
            .child(summary)
            .when(self.mon.search_open, |d| d.child(search))
            .child(header)
            .child(table)
            .when_some(menu, |d, m| d.child(m))
            .into_any_element()
    }

    /// Menu do processo (clique direito): copiar PID, sinais e encerrar, como no Wave.
    fn render_proc_menu(&self, pid: i32, x: f32, y: f32, cx: &mut Context<Self>) -> AnyElement {
        let name = self.monitor.procs.iter().find(|p| p.pid == pid).map(|p| p.command.clone()).unwrap_or_default();
        let item = |id: &'static str, label: String, danger: bool| {
            div()
                .id(id)
                .px_3()
                .py(px(6.))
                .rounded(px(4.))
                .text_size(px(13.))
                .text_color(rgb(if danger { RED } else { TEXT }))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(HOVER)))
                .child(label)
        };
        let signal = move |this: &mut Workspace, sig: &'static str, verb: &str| {
            this.mon.menu = None;
            this.mon.status = Some(match sysmon::send_signal(pid, sig) {
                Ok(()) => (format!("{verb}: processo {pid} (SIG{sig})"), false),
                Err(e) => (format!("Não deu para enviar SIG{sig} ao {pid}: {e}"), true),
            });
        };
        div()
            .id("proc-menu")
            .absolute()
            .occlude()
            .left(px(x))
            .top(px(y))
            .w(px(220.))
            .p_1()
            .rounded(px(6.))
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(CARD_BORDER))
            .shadow_lg()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
            .child(div().px_3().py_1().text_size(px(12.)).text_color(rgb(DISABLED)).truncate().child(format!("{pid} · {name}")))
            .child(item("pm-copy", "Copiar PID".into(), false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(pid.to_string()));
                this.mon.menu = None;
                this.mon.status = Some((format!("PID {pid} copiado"), false));
                cx.notify();
            })))
            .child(div().h(px(1.)).my_1().bg(rgb(CARD_BORDER)))
            .child(item("pm-int", "Interromper (SIGINT)".into(), false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                signal(this, "INT", "Interrompido");
                cx.notify();
            })))
            .child(item("pm-hup", "Recarregar (SIGHUP)".into(), false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                signal(this, "HUP", "Sinal enviado");
                cx.notify();
            })))
            .child(item("pm-term", "Encerrar (SIGTERM)".into(), true).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                signal(this, "TERM", "Encerrado");
                cx.notify();
            })))
            .child(item("pm-kill", "Forçar encerramento (SIGKILL)".into(), true).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                signal(this, "KILL", "Forçado");
                cx.notify();
            })))
            .into_any_element()
    }

    /// Teclas com o filtro de processos focado.
    pub fn on_monitor_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.to_lowercase();
        match key.as_str() {
            "escape" | "enter" => self.mon.filter_focused = false,
            "backspace" => {
                self.mon.filter.pop();
            }
            "space" => self.mon.filter.push(' '),
            _ if !k.modifiers.control && !k.modifiers.alt => {
                if let Some(c) = &k.key_char {
                    self.mon.filter.push_str(c);
                }
            }
            _ => return false,
        }
        self.mon.scroll = 0;
        cx.notify();
        true
    }
}

// ----- gerenciador de arquivos -----

impl Workspace {
    /// Pasta em tabela (ou a imagem aberta), como o "preview" de pasta do Wave.
    fn render_files_pane(&mut self, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let Some(f) = self.files.as_mut() else { return div().into_any_element() };
        if let Some(img) = f.viewing.clone() {
            let zoom = f.zoom;
            return self.render_image_view(img, zoom, r, cx);
        }
        let header_h = 28.0;
        let visible = ((r.h - header_h) / FILE_ROW_H).floor().max(1.0) as usize;
        let at_root = f.dir.parent().is_none();
        let total = f.rows.len() + usize::from(!at_root);
        f.scroll = f.scroll.min(total.saturating_sub(visible));
        let (first, sort, asc, selected) = (f.scroll, f.sort, f.asc, f.selected);
        let rows: Vec<crate::files_pane::Row> = f.rows.clone();
        let error = f.error.clone();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let tz = self.tz_offset;
        let font = self.icon_font.clone();
        let mono = self.mono.clone();

        use crate::files_pane::{date_string, size_label, Col};
        // colunas: (rótulo, ordenação, largura; None = ocupa o resto), alinhada à direita.
        // Em painel estreito as colunas saem na ordem Tipo, Modificado, Permissões; o nome (mínimo 110 px) e o
        // tamanho ficam sempre.
        let mut room = r.w - 12.0 - 110.0;
        let mut keep: Vec<&str> = Vec::new();
        for (label, w) in [("Tamanho", 76.0), ("Permissões", 96.0), ("Modificado", 118.0), ("Tipo", 124.0)] {
            if room >= w || label == "Tamanho" {
                room -= w;
                keep.push(label);
            }
        }
        let (show_perm, show_date, show_kind) = (keep.contains(&"Permissões"), keep.contains(&"Modificado"), keep.contains(&"Tipo"));
        let cols: Vec<(&str, Option<Col>, Option<f32>, bool)> = [
            ("Nome", Some(Col::Name), None, false),
            ("Permissões", None, Some(96.0), false),
            ("Modificado", Some(Col::Modified), Some(118.0), false),
            ("Tamanho", Some(Col::Size), Some(76.0), true),
            ("Tipo", Some(Col::Kind), Some(124.0), false),
        ]
        .into_iter()
        .filter(|c| c.0 == "Nome" || keep.contains(&c.0))
        .collect();
        let cell = |w: Option<f32>, right: bool| {
            let d = div().px(px(8.)).flex().items_center().overflow_hidden().whitespace_nowrap();
            let d = match w {
                Some(w) => d.flex_none().w(px(w)),
                None => d.flex_1().min_w_0(),
            };
            if right {
                d.justify_end()
            } else {
                d
            }
        };
        let header = div()
            .flex_none()
            .h(px(header_h))
            .px(px(6.))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(CARD_BORDER))
            .text_size(px(13.))
            .text_color(rgb(TEXT))
            .children(cols.iter().enumerate().map(|(i, (label, col, w, right))| {
                let col = *col;
                let active = col == Some(sort);
                let arrow = if !active {
                    ""
                } else if asc {
                    " ↑"
                } else {
                    " ↓"
                };
                cell(*w, *right)
                    .id(("fcol", i))
                    .h_full()
                    .when(i > 0, |d| d.border_l_1().border_color(rgb(CARD_BORDER)))
                    .when(!active, |d| d.text_color(rgb(MUTED)))
                    .when(col.is_some(), |d| d.cursor_pointer().hover(|s| s.text_color(rgb(TEXT))))
                    .child(format!("{label}{arrow}"))
                    .when_some(col, |d, col| {
                        d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(f) = this.files.as_mut() {
                                f.set_sort(col);
                            }
                            cx.notify();
                        }))
                    })
            }));

        // linhas: ".." (subir) e as entradas; listra alternada e seleção na cor da seção
        let mut els: Vec<AnyElement> = Vec::new();
        let up_row = (!at_root).then_some(());
        for vi in first..(first + visible).min(total) {
            let (name, is_dir, perm, date, size, kind, idx): (String, bool, String, String, String, String, Option<usize>) = match (up_row, vi) {
                (Some(()), 0) => ("..".into(), true, String::new(), String::new(), "-".into(), "pasta".into(), None),
                _ => {
                    let i = vi - usize::from(up_row.is_some());
                    let e = &rows[i];
                    (e.name.clone(), e.is_dir, e.perm.clone(), date_string(e.mtime, now, tz), size_label(e), e.kind.clone(), Some(i))
                }
            };
            let sel = idx.is_some() && idx == selected;
            let ic = crate::icons::file_icon(&name, if is_dir { &ox_files::EntryKind::Dir } else { &ox_files::EntryKind::File }, false);
            let path = idx.map(|i| rows[i].path.clone());
            els.push(
                div()
                    .id(("frow", vi))
                    .flex_none()
                    .h(px(FILE_ROW_H))
                    .mx(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .when(sel, |d| d.bg(with_alpha(accent, 0x55)))
                    .when(!sel && vi % 2 == 1, |d| d.bg(rgb(0x1f1817)))
                    .when(!sel, |d| d.hover(|s| s.bg(rgb(HOVER))))
                    .child(
                        cell(None, false)
                            .gap_2()
                            .child(div().flex_none().w(px(16.)).flex().justify_center().child(if is_dir {
                                wicon("folder-closed", 14., FILES_BLUE).into_any_element()
                            } else {
                                crate::icons::icon_el(&font, &ic, 14.).into_any_element()
                            }))
                            .child(div().min_w_0().truncate().text_color(rgb(TEXT)).child(name)),
                    )
                    .when(show_perm, |d| d.child(cell(Some(96.0), false).font_family(mono.clone()).text_size(px(12.)).text_color(rgb(MUTED)).child(perm)))
                    .when(show_date, |d| d.child(cell(Some(118.0), false).font_family(mono.clone()).text_size(px(12.)).text_color(rgb(MUTED)).child(date)))
                    .child(cell(Some(76.0), true).font_family(mono.clone()).text_size(px(12.)).text_color(rgb(MUTED)).child(size))
                    .when(show_kind, |d| d.child(cell(Some(124.0), false).text_size(px(12.)).text_color(rgb(MUTED)).child(kind)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.focused_pane = Some(PaneKind::Files);
                        let open = ev.click_count() >= 2;
                        match (&path, open) {
                            // "..": um clique já sobe, como no Wave
                            (None, _) => {
                                if let Some(f) = this.files.as_mut() {
                                    if let Some(parent) = f.dir.parent().map(|p| p.to_path_buf()) {
                                        f.open_dir(parent);
                                    }
                                }
                            }
                            (Some(_), false) => {
                                if let Some(f) = this.files.as_mut() {
                                    f.selected = idx;
                                }
                            }
                            (Some(p), true) => this.open_files_entry(p.clone()),
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .child(header)
            .child(
                div()
                    .id("files-table")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .pt_1()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, _, cx| {
                        cx.stop_propagation();
                        if let Some(f) = this.files.as_mut() {
                            f.wheel -= f32::from(ev.delta.pixel_delta(px(FILE_ROW_H)).y) / FILE_ROW_H;
                            let steps = f.wheel.trunc() as i64;
                            f.wheel -= steps as f32;
                            f.scroll = (f.scroll as i64 + steps).clamp(0, total.saturating_sub(visible) as i64) as usize;
                        }
                        cx.notify();
                    }))
                    .children(els)
                    .when_some(error, |d, e| d.child(div().p_3().text_size(px(13.)).text_color(rgb(RED)).child(e)))
                    .when(total == 0, |d| d.child(div().p_3().text_size(px(13.)).text_color(rgb(MUTED)).child("Pasta vazia.")))
                    .when(total > visible, |d| {
                        let track = r.h - header_h - 6.0;
                        let len = (track * visible as f32 / total as f32).max(24.0);
                        let top = (track - len) * first as f32 / (total - visible) as f32;
                        d.child(div().absolute().right(px(2.)).top(px(top + 4.0)).w(px(5.)).h(px(len)).rounded_full().bg(rgb(BTN_HOVER)))
                    }),
            )
            .into_any_element()
    }

    /// Abre uma entrada da tabela: pasta navega, imagem abre no visualizador, o resto no editor.
    fn open_files_entry(&mut self, path: std::path::PathBuf) {
        let Some(f) = self.files.as_mut() else { return };
        if path.is_dir() {
            f.open_dir(path);
        } else if crate::files_pane::is_image(&path) {
            f.viewing = Some(path);
            f.zoom = 1.0;
        } else {
            self.open_file(path);
            self.show_notes = true;
            // o arquivo aberto recebe o teclado (setas movem o cursor do editor, não a tabela)
            self.focus_editor = true;
            self.focused_pane = None;
        }
    }

    /// Visualizador de imagem: a imagem cabe no painel; + / − / ⟲ mudam o zoom (como no Wave).
    fn render_image_view(&self, img: std::path::PathBuf, zoom: f32, r: Rect, cx: &mut Context<Self>) -> AnyElement {
        let accent = self.focus_color();
        let btn = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .w(px(52.))
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(rgb(accent))
                .text_color(rgb(BG))
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .cursor_pointer()
                .hover(|s| s.opacity(0.85))
                .child(label)
        };
        let set = |f: fn(f32) -> f32| {
            cx.listener(move |this: &mut Workspace, _: &ClickEvent, _, cx: &mut Context<Workspace>| {
                if let Some(p) = this.files.as_mut() {
                    p.zoom = f(p.zoom).clamp(0.1, 8.0);
                }
                cx.notify();
            })
        };
        let (w, h) = ((r.w - 24.0) * zoom, (r.h - 24.0) * zoom);
        div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(rgb(0x0f0b0b))
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                // roda do mouse = zoom
                let dy = f32::from(ev.delta.pixel_delta(px(20.)).y);
                if let Some(p) = this.files.as_mut() {
                    p.zoom = (p.zoom * if dy > 0.0 { 1.1 } else { 1.0 / 1.1 }).clamp(0.1, 8.0);
                }
                cx.notify();
            }))
            .child(gpui::img(img).w(px(w)).h(px(h)).flex_none().object_fit(gpui::ObjectFit::Contain))
            .child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .flex()
                    .gap_1()
                    .child(btn("img-in", "+").on_click(set(|z| z * 1.25)))
                    .child(btn("img-out", "−").on_click(set(|z| z / 1.25)))
                    .child(btn("img-reset", "⟲").on_click(set(|_| 1.0))),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(8.))
                    .right(px(10.))
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child(format!("{:.0}%", zoom * 100.0)),
            )
            .into_any_element()
    }

    /// Teclas com o gerenciador de arquivos em foco.
    pub fn on_files_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.to_lowercase();
        let Some(f) = self.files.as_mut() else { return false };
        if f.viewing.is_some() {
            match key.as_str() {
                "escape" | "backspace" => f.go_back(),
                "+" | "=" => f.zoom = (f.zoom * 1.25).min(8.0),
                "-" => f.zoom = (f.zoom / 1.25).max(0.1),
                "0" => f.zoom = 1.0,
                _ => return false,
            }
            cx.notify();
            return true;
        }
        let n = f.rows.len();
        match key.as_str() {
            "down" if n > 0 => f.selected = Some(f.selected.map_or(0, |i| (i + 1).min(n - 1))),
            "up" if n > 0 => f.selected = Some(f.selected.map_or(0, |i| i.saturating_sub(1))),
            "home" if n > 0 => f.selected = Some(0),
            "end" if n > 0 => f.selected = Some(n - 1),
            "backspace" => f.go_back(),
            "enter" => {
                if let Some(p) = f.selected.and_then(|i| f.rows.get(i)).map(|r| r.path.clone()) {
                    self.open_files_entry(p);
                }
            }
            _ => return false,
        }
        // mantém a seleção na área visível (a rolagem é ajustada no desenho)
        if let Some(f) = self.files.as_mut() {
            if let Some(i) = f.selected {
                let row = i + usize::from(f.dir.parent().is_some());
                if row < f.scroll {
                    f.scroll = row;
                } else if row >= f.scroll + 20 {
                    f.scroll = row.saturating_sub(19);
                }
            }
        }
        cx.notify();
        true
    }
}
