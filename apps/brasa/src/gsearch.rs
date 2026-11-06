//! Busca global na barra superior (Ctrl+K): procura de uma vez em notas (nome e texto) e sessões (nome,
//! pasta e saída do terminal). Setas escolhem, Enter abre, Esc fecha.

use gpui::{deferred, div, prelude::*, px, rgb, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent};

use crate::icons::wicon;
use crate::theme::*;
use crate::{NoteMode, Workspace};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    Note { rel: String, label: String, snippet: String },
    Session { idx: usize, label: String, detail: String },
}

const MAX_NOTES: usize = 8;
const MAX_SESSIONS: usize = 6;

impl Workspace {
    /// Resultados para o texto digitado: notas primeiro, depois sessões.
    pub fn gsearch_hits(&self) -> Vec<Hit> {
        let q = self.gsearch.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<Hit> = self
            .notes
            .search(&q)
            .unwrap_or_default()
            .into_iter()
            .take(MAX_NOTES)
            .map(|n| Hit::Note { label: n.name.trim_end_matches(".md").to_string(), rel: n.name, snippet: n.snippet })
            .collect();
        let mut sessions = 0;
        for i in 0..self.tabs.len() {
            if sessions == MAX_SESSIONS {
                break;
            }
            let t = &self.tabs[i];
            let folder = t.folder.and_then(|id| self.session_folders.iter().find(|f| f.id == id)).map(|f| f.name.to_lowercase()).unwrap_or_default();
            let label = self.terminal_label(i);
            let by_name = label.to_lowercase().contains(&q) || self.session_label(i).0.to_lowercase().contains(&q) || folder.contains(&q);
            let in_output = t.term.search(&q).len();
            if by_name || in_output > 0 {
                let detail = match in_output {
                    0 => self.session_label(i).1,
                    1 => "1 ocorrência na saída".to_string(),
                    n => format!("{n} ocorrências na saída"),
                };
                hits.push(Hit::Session { idx: i, label, detail });
                sessions += 1;
            }
        }
        hits
    }

    pub fn open_hit(&mut self, hit: &Hit) {
        match hit {
            Hit::Note { rel, .. } => {
                self.open_note(rel);
                self.show_notes = true;
                self.set_note_mode(NoteMode::Edit);
            }
            Hit::Session { idx, .. } => self.switch_tab(*idx),
        }
        self.close_gsearch();
    }

    pub fn close_gsearch(&mut self) {
        self.gsearch_focused = false;
        self.gsearch.clear();
        self.gsearch_sel = 0;
    }

    /// Teclas na busca global. Atalhos com Ctrl/Alt passam adiante.
    pub fn on_gsearch_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return false;
        }
        match k.key.to_lowercase().as_str() {
            "escape" => self.close_gsearch(),
            "enter" => {
                let hits = self.gsearch_hits();
                if let Some(hit) = hits.get(self.gsearch_sel.min(hits.len().saturating_sub(1))) {
                    let hit = hit.clone();
                    self.open_hit(&hit);
                }
            }
            "down" => {
                let n = self.gsearch_hits().len();
                if n > 0 {
                    self.gsearch_sel = (self.gsearch_sel + 1) % n;
                }
            }
            "up" => {
                let n = self.gsearch_hits().len();
                if n > 0 {
                    self.gsearch_sel = (self.gsearch_sel + n - 1) % n;
                }
            }
            "backspace" => {
                self.gsearch.pop();
                self.gsearch_sel = 0;
            }
            "space" => {
                self.gsearch.push(' ');
                self.gsearch_sel = 0;
            }
            "tab" | "left" | "right" | "home" | "end" | "delete" => {}
            _ => {
                if let Some(c) = &k.key_char {
                    self.gsearch.push_str(c);
                    self.gsearch_sel = 0;
                }
            }
        }
        cx.notify();
        true
    }

    /// A caixa de busca do cabeçalho e, enquanto em foco, a lista de resultados logo abaixo.
    pub fn render_gsearch(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.gsearch_focused;
        let hits = if focused { self.gsearch_hits() } else { Vec::new() };
        let sel = self.gsearch_sel.min(hits.len().saturating_sub(1));

        let field = div()
            .id("gsearch")
            .w(px(360.))
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(if focused { c_accent() } else { c_card_border() }))
            .bg(rgb(if focused { c_bg() } else { c_panel() }))
            .text_sm()
            .cursor_text()
            .child(wicon("search", 14., if focused { c_accent() } else { c_muted() }))
            .child(if self.gsearch.is_empty() {
                div().flex_1().text_color(rgb(c_muted())).child("Buscar em notas e sessões")
            } else {
                div().flex_1().min_w_0().truncate().child(format!("{}{}", self.gsearch, if focused { "▏" } else { "" }))
            })
            .when(!focused, |d| d.child(div().flex_none().text_xs().text_color(rgb(c_disabled())).child("Ctrl+K")))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.gsearch_focused = true;
                    this.filter_focused = false;
                    this.focus_editor = false;
                    cx.stop_propagation(); // o contêiner raiz tiraria o foco e a janela começaria a ser arrastada
                    cx.notify();
                }),
            );

        let dropdown = focused.then(|| {
            let mut list = div()
                .id("gsearch-list")
                .absolute()
                .top(px(34.))
                .left(px(-50.))
                .w(px(460.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .py_1()
                .rounded_lg()
                .border_1()
                .border_color(rgb(c_card_border()))
                .bg(rgb(c_panel()))
                .shadow_lg()
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            if self.gsearch.trim().is_empty() {
                list = list.child(div().px_3().py_2().text_sm().text_color(rgb(c_muted())).child("Digite para buscar pelo nome ou pelo conteúdo das notas, e pelo nome ou pela saída das sessões."));
            } else if hits.is_empty() {
                list = list.child(div().px_3().py_2().text_sm().text_color(rgb(c_muted())).child("Nada encontrado."));
            }
            let mut last_kind = "";
            for (i, hit) in hits.iter().enumerate() {
                let (kind, icon, color, title, detail) = match hit {
                    Hit::Note { label, snippet, .. } => ("Notas", "notebook", c_amber(), label.clone(), snippet.clone()),
                    Hit::Session { label, detail, .. } => ("Sessões", "terminal", c_coral(), label.clone(), detail.clone()),
                };
                if kind != last_kind {
                    list = list.child(div().px_3().pt_2().pb_1().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(rgb(c_muted())).child(kind));
                    last_kind = kind;
                }
                let on = i == sel;
                let open = hit.clone();
                list = list.child(
                    div()
                        .id(("gsearch-hit", i))
                        .mx_1()
                        .px_2()
                        .h(px(34.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .cursor_pointer()
                        .when(on, |d| d.bg(rgb(c_surface())))
                        .when(!on, |d| d.hover(|s| s.bg(rgb(c_hover()))))
                        .child(wicon(icon, 14., color))
                        .child(div().flex_none().max_w(px(180.)).truncate().text_sm().text_color(rgb(c_text())).child(title))
                        .child(div().flex_1().min_w_0().truncate().text_xs().text_color(rgb(c_disabled())).child(detail))
                        // abre no clique do mouse (o contêiner raiz fecharia a busca antes de o clique terminar)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                this.open_hit(&open);
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                );
            }
            deferred(list).with_priority(10)
        });

        div().relative().flex_none().child(field).children(dropdown).into_any_element()
    }
}
