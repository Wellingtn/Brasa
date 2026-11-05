//! Moldura da janela: cabeçalho do painel principal com os botões padrão (minimizar, maximizar, fechar)
//! à direita, como em qualquer app de Linux ou Windows; arrastar para mover e bordas para redimensionar
//! (a janela usa decoração do cliente).

use gpui::{
    div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, CursorStyle, ElementId, MouseButton, MouseDownEvent, ResizeEdge, Stateful, Div,
    Window, WindowControlArea,
};

use crate::icons::wicon;
use crate::theme::*;
use crate::panes::Rect;
use crate::{shorten, Resize, Workspace, TITLE_H};

const EDGE: f32 = 5.0;
const CORNER: f32 = 12.0;

/// Botão da barra de título: 46 px de largura, ocupa toda a altura.
fn win_btn(id: &'static str, glyph: &'static str, hover_bg: u32, hover_fg: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(46.))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(14.))
        .text_color(rgb(MUTED))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hover_bg)).text_color(rgb(hover_fg)))
        .child(glyph)
}

/// Botão "dividir": retângulo com a linha no meio (vertical = lado a lado, horizontal = embaixo).
fn split_btn(id: &'static str, row: bool) -> Stateful<Div> {
    let line = div().absolute().bg(rgb(MUTED));
    div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(HOVER)))
        .child(
            div()
                .relative()
                .w(px(15.))
                .h(px(13.))
                .rounded(px(2.))
                .border_1()
                .border_color(rgb(MUTED))
                .child(if row { line.left(px(6.)).top_0().bottom_0().w(px(1.)) } else { line.top(px(5.)).left_0().right_0().h(px(1.)) }),
        )
}

impl Workspace {
    /// Pasta e branch da sessão ativa: (nome da pasta, detalhe à direita).
    pub fn session_label(&self, i: usize) -> (String, String) {
        let t = &self.tabs[i];
        let cwd = self.pty.cwd(t.session).unwrap_or_else(|| self.shell_cwd.clone());
        let name = if cwd == crate::home() {
            "~".to_string()
        } else {
            cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())
        };
        let detail = if !t.last_cmd.is_empty() { t.last_cmd.clone() } else { shorten(&cwd) };
        (name, detail)
    }

    /// Indicador de CPU do cabeçalho: medidor + porcentagem. Abre (no futuro) o monitor de desempenho.
    fn cpu_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let sampled = !self.monitor.cpu_hist.is_empty();
        let pct = self.monitor.cpu;
        let color = if pct < 50.0 {
            TEAL
        } else if pct < 80.0 {
            YELLOW
        } else {
            CORAL
        };
        const METER_W: f32 = 38.0;
        div()
            .id("cpu-btn")
            .flex_none()
            .h(px(28.))
            .px(px(10.))
            .mr_2()
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_md()
            .border_1()
            .border_color(rgb(CARD_BORDER))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(HOVER)))
            .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child("CPU"))
            .child(
                div()
                    .w(px(METER_W))
                    .h(px(6.))
                    .rounded_full()
                    .bg(rgb(SURFACE))
                    .child(div().h_full().rounded_full().w(px(METER_W * pct.clamp(0.0, 100.0) / 100.0)).bg(rgb(color))),
            )
            .child(
                div()
                    .w(px(34.))
                    .flex()
                    .justify_end()
                    .font_family(self.mono.clone())
                    .text_size(px(12.))
                    .text_color(rgb(TEXT))
                    .child(if sampled { format!("{pct:.0}%") } else { "–".to_string() }),
            )
            .when(self.panes.contains(crate::panes::PaneKind::Cpu), |d| d.border_color(rgb(color)))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_widget(crate::panes::PaneKind::Cpu);
                cx.notify();
            }))
            .into_any_element()
    }

    /// Cabeçalho do painel principal: o espaço livre arrasta a janela (duplo clique maximiza); à
    /// direita, o indicador de CPU e os botões padrão de janela.
    pub fn render_titlebar(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let maximized = window.is_maximized();
        let cpu = self.cpu_button(cx);
        let icon_btn = |id: &'static str, icon: &'static str, on: bool| {
            div()
                .id(id)
                .flex_none()
                .size(px(30.))
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgb(HOVER)))
                .child(wicon(icon, 15., if on { TEXT } else { MUTED }))
        };

        div()
            .h(px(TITLE_H))
            .flex_none()
            .flex()
            .items_center()
            .bg(rgb(BG))
            .rounded_tr(px(self.corner_r))
            .when(!self.show_sidebar, |d| d.rounded_tl(px(self.corner_r)))
            .border_b_1()
            .border_color(rgb(DIVIDER))
            .when(!self.show_sidebar, |d| {
                d.child(div().pl_2().child(icon_btn("toggle-sidebar", "layout-left", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.show_sidebar = true;
                    cx.notify();
                }))))
            })
            // área de arrastar
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, ev: &MouseDownEvent, window, _| {
                            if ev.click_count >= 2 {
                                window.zoom_window();
                            } else {
                                window.start_window_move();
                            }
                        }),
                    ),
            )
            // dividir: abre um terminal novo ao lado ou embaixo do painel em foco
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .mr_2()
                    .child(split_btn("split-right", true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.split_new(true);
                        cx.notify();
                    })))
                    .child(split_btn("split-down", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.split_new(false);
                        cx.notify();
                    }))),
            )
            .child(cpu)
            // botões padrão de janela, na ponta direita
            .child(
                div()
                    .flex_none()
                    .h_full()
                    .flex()
                    .child(win_btn("win-min", "—", HOVER, TEXT).on_click(cx.listener(|_, _: &ClickEvent, window, _| window.minimize_window())))
                    .child(
                        win_btn("win-max", if maximized { "❐" } else { "☐" }, HOVER, TEXT)
                            .on_click(cx.listener(|_, _: &ClickEvent, window, _| window.zoom_window())),
                    )
                    .child(win_btn("win-close", "✕", WIN_CLOSE_HOVER, 0xffffff).rounded_tr(px(self.corner_r)).on_click(cx.listener(|_, _: &ClickEvent, window, _| window.remove_window()))),
            )
            .into_any_element()
    }

    /// Faixas para ajustar larguras arrastando: borda da barra lateral, borda das notas e as
    /// divisórias entre painéis. Destacam na cor de foco ao passar o mouse.
    pub fn width_handles(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        const GRIP: f32 = 6.0;
        let accent = rgb(AMBER);
        let handle = |id: ElementId, x: f32, y: f32, w: f32, h: f32, row: bool, kind: Resize, cx: &mut Context<Self>| {
            let active = self.resizing == Some(kind);
            div()
                .id(id)
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(w))
                .h(px(h))
                .occlude()
                .cursor(if row { CursorStyle::ResizeLeftRight } else { CursorStyle::ResizeUpDown })
                // linha fina no meio, visível ao passar o mouse ou arrastando
                .child(
                    div()
                        .absolute()
                        .when(row, |d| d.left(px(w / 2.0 - 1.0)).top_0().bottom_0().w(px(2.)))
                        .when(!row, |d| d.top(px(h / 2.0 - 1.0)).left_0().right_0().h(px(2.)))
                        .when(active, |d| d.bg(accent)),
                )
                .hover(move |s| s.bg(gpui::rgba(0xe9b25f33)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.resizing = Some(kind);
                        cx.notify();
                    }),
                )
                // a faixa cobre o que está embaixo: o arraste e o soltar também são tratados nela
                .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _, cx| {
                    if this.resizing.is_some() && ev.pressed_button == Some(MouseButton::Left) {
                        this.on_resize_move(ev.position);
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _: &gpui::MouseUpEvent, _, cx| {
                        this.resizing = None;
                        cx.notify();
                    }),
                )
                .into_any_element()
        };
        let mut out = Vec::new();
        let top = 1.0 + TITLE_H;
        let full_h = 4000.0;
        if self.show_sidebar {
            out.push(handle("rs-side".into(), self.sidebar_px - GRIP / 2.0 + 1.0, 0.0, GRIP, full_h, true, Resize::Sidebar, cx));
        }
        if self.show_notes {
            let x = 1.0 + self.sidebar_w() + self.notes_w();
            out.push(handle("rs-notes".into(), x - GRIP / 2.0, top, GRIP, full_h, true, Resize::Notes, cx));
        }
        if self.pane_rects.len() > 1 {
            let x = 1.0 + self.sidebar_w() + self.notes_w();
            let vh = self.vh;
            let main = Rect { x, y: top, w: (self.vw - x - 1.0 - crate::pane_view::WIDGET_BAR_W).max(80.0), h: (vh - TITLE_H - 2.0).max(80.0) };
            let area = crate::pane_view::inset(main, crate::pane_view::PANE_GAP);
            for (id, row, bar, parent) in self.panes.dividers(area, crate::pane_view::PANE_GAP) {
                out.push(handle(("rs-split", id).into(), bar.x, bar.y, bar.w, bar.h, row, Resize::Split(id, row, parent), cx));
            }
        }
        out
    }

    /// Arrastando uma borda: aplica a nova largura (ou proporção) conforme o mouse.
    pub fn on_resize_move(&mut self, pos: gpui::Point<gpui::Pixels>) {
        let (x, y) = (f32::from(pos.x), f32::from(pos.y));
        match self.resizing {
            Some(Resize::Sidebar) => self.sidebar_px = x.clamp(200.0, 480.0),
            Some(Resize::Notes) => {
                let max = (self.vw - self.sidebar_w() - 320.0).max(300.0);
                self.notes_px = (x - 1.0 - self.sidebar_w()).clamp(300.0, max);
                self.notes_wide = false;
            }
            Some(Resize::Split(id, row, parent)) => {
                let ratio = if row { (x - parent.x) / parent.w.max(1.0) } else { (y - parent.y) / parent.h.max(1.0) };
                self.panes.set_ratio(id, ratio);
            }
            None => {}
        }
    }

    /// Faixas invisíveis nas bordas e cantos para redimensionar a janela com o mouse.
    pub fn resize_handles(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let handle = |edge: ResizeEdge, cursor: CursorStyle, cx: &mut Context<Self>| {
            div().absolute().cursor(cursor).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, window, cx| {
                    window.start_window_resize(edge);
                    cx.stop_propagation();
                }),
            )
        };
        vec![
            handle(ResizeEdge::Top, CursorStyle::ResizeUpDown, cx).top_0().left(px(CORNER)).right(px(CORNER)).h(px(EDGE)).into_any_element(),
            handle(ResizeEdge::Bottom, CursorStyle::ResizeUpDown, cx).bottom_0().left(px(CORNER)).right(px(CORNER)).h(px(EDGE)).into_any_element(),
            handle(ResizeEdge::Left, CursorStyle::ResizeLeftRight, cx).left_0().top(px(CORNER)).bottom(px(CORNER)).w(px(EDGE)).into_any_element(),
            handle(ResizeEdge::Right, CursorStyle::ResizeLeftRight, cx).right_0().top(px(CORNER)).bottom(px(CORNER)).w(px(EDGE)).into_any_element(),
            handle(ResizeEdge::TopLeft, CursorStyle::ResizeUpLeftDownRight, cx).top_0().left_0().size(px(CORNER)).into_any_element(),
            handle(ResizeEdge::TopRight, CursorStyle::ResizeUpRightDownLeft, cx).top_0().right_0().size(px(CORNER)).into_any_element(),
            handle(ResizeEdge::BottomLeft, CursorStyle::ResizeUpRightDownLeft, cx).bottom_0().left_0().size(px(CORNER)).into_any_element(),
            handle(ResizeEdge::BottomRight, CursorStyle::ResizeUpLeftDownRight, cx).bottom_0().right_0().size(px(CORNER)).into_any_element(),
        ]
    }
}
