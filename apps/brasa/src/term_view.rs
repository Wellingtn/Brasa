//! Área central: cartão do terminal (grade de células) e barra inferior com autocomplete/busca.
//!
//! Como no Warp, o caminho não fica numa barra: a linha do prompt do shell é substituída, no
//! próprio lugar, por chips (venv, pasta, branch). Cada comando antigo mantém o chip da pasta
//! em que foi executado, e o texto digitado fica na linha de baixo.

use gpui::{
    div, prelude::*, px, rgb, AnyElement, ClickEvent, Context, Div, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ScrollWheelEvent,
};

use crate::block_view::fit_hints;
use crate::icons::{icon_el, Icon, BRANCH, FOLDER, TERMINAL};
use crate::terminal::RowView;
use crate::theme::*;
use crate::{shorten, Workspace, MARGIN, TERM_PAD};

/// Linhas separadoras por linha da tela, como no Warp: (acima, abaixo).
///  - acima de cada prompt (mais forte no prompt atual, o último da tela);
///  - abaixo do comando digitado, quando já há saída logo depois.
fn separators(screen: &[RowView]) -> Vec<(Option<u32>, Option<u32>)> {
    let is_prompt = |r: Option<&RowView>| matches!(r, Some(RowView::Prompt(_)));
    let has_text = |r: Option<&RowView>| matches!(r, Some(RowView::Cells(runs)) if runs.iter().any(|x| !x.text.trim().is_empty()));
    let last_prompt = screen.iter().rposition(|r| matches!(r, RowView::Prompt(_)));
    (0..screen.len())
        .map(|i| {
            let top = (i > 0 && is_prompt(screen.get(i))).then_some(if Some(i) == last_prompt { c_sep_active() } else { c_sep() });
            let is_command_row = i > 0 && is_prompt(screen.get(i - 1)) && !is_prompt(screen.get(i));
            let bottom = (is_command_row && has_text(screen.get(i + 1))).then_some(c_sep());
            (top, bottom)
        })
        .collect()
}

const PLACEHOLDER: &str = "Digite um comando, ex.: cargo build --release";

impl Workspace {
    /// Chip compacto (cabe numa linha do terminal): ícone + texto, como o pwd do Warp.
    pub fn chip(&self, glyph: char, fallback: &str, icon_color: u32, text: String, text_color: u32) -> Div {
        let svg = match glyph {
            TERMINAL => Some("icons/terminal.svg"),
            FOLDER => Some("icons/folder-closed.svg"),
            BRANCH => Some("icons/git-branch-02.svg"),
            _ => None,
        };
        let ic = Icon { svg, glyph, color: icon_color, fallback: fallback.to_string() };
        let size = (self.font_size - 2.0).max(10.0);
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(self.cell_h - 3.))
            .px(px(8.))
            .rounded_md()
            .bg(rgb(c_surface()))
            .border_1()
            .border_color(rgb(c_border()))
            .text_size(px(size))
            .font_weight(FontWeight::SEMIBOLD)
            .child(icon_el(&self.icon_font, &ic, size))
            .child(div().text_color(rgb(text_color)).whitespace_nowrap().child(text))
    }

    /// Área central: lista de blocos (modo padrão) ou grade crua (tela cheia / prompt nativo).
    pub fn render_terminal_area(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(el) = self.render_block_area(cx) {
            return el;
        }
        self.render_raw_area(cx)
    }

    fn render_raw_area(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let h = px(self.cell_h);
        let tab = &self.tabs[self.active];
        let (offset, _history) = tab.term.scrollback();
        let (cols, rows) = (tab.term.cols, tab.term.rows);

        let placeholder = self.prompt_text.as_deref().map(|p| (p, PLACEHOLDER));
        let screen = tab.term.rows_to_runs(self.search.as_ref().filter(|_| self.focused_render).map(|s| (s.matches.as_slice(), s.current)), placeholder);

        let separators = separators(&screen);

        let lines: Vec<AnyElement> = screen
            .into_iter()
            .zip(separators)
            .map(|(row, (top, bottom))| {
                let line = |c: u32| div().absolute().left_0().right_0().h(px(1.)).bg(rgb(c));
                let el = match row {
                    // linha de prompt -> chips (venv, pasta, branch)
                    RowView::Prompt(info) => div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .when_some(info.venv, |d, v| d.child(self.chip(TERMINAL, "›_", c_accent(), v, c_text())))
                        .child(self.chip(FOLDER, "▣", 0x7aa2f7, info.path, c_text()))
                        .when_some(info.branch, |d, b| d.child(self.chip(BRANCH, "⎇", GREEN, b, GREEN))),
                    RowView::Cells(runs) => div().flex().flex_row().whitespace_nowrap().children(runs.into_iter().map(|r| {
                        div()
                            .flex_none()
                            .text_color(rgb(r.fg))
                            .when(r.bg != c_term_bg(), |d| d.bg(rgb(r.bg)))
                            .when(r.bold, |d| d.font_weight(FontWeight::BOLD))
                            .when(r.underline, |d| d.underline())
                            .child(r.text)
                    })),
                };
                el.relative()
                    .h(h)
                    .flex_none()
                    .when_some(top, |d, c| d.child(line(c).top_0()))
                    .when_some(bottom, |d, c| d.child(line(c).bottom_0()))
                    .into_any_element()
            })
            .collect();

        // ---- barra inferior: busca, sugestões ou atalhos; contexto à direita ----
        let mut left = div().flex().flex_1().min_w_0().items_center().gap_2().overflow_hidden();
        if let Some(s) = self.search.as_ref().filter(|_| self.focused_render) {
            let count = if s.query.is_empty() {
                String::new()
            } else if s.matches.is_empty() {
                "sem resultados".to_string()
            } else {
                format!("{}/{}", s.current + 1, s.matches.len())
            };
            left = left
                .child(div().text_color(rgb(c_accent())).child("buscar ›"))
                .child(div().font_family(self.mono.clone()).child(format!("{}▏", s.query)))
                .child(div().text_color(rgb(YELLOW)).child(count))
                .child(div().text_color(rgb(c_muted())).child("enter: acima · shift-enter: abaixo · esc: fechar"));
        } else if let Some((first, rest)) = self.suggestions.split_first() {
            let ghost = first.ghost(&self.line).to_string();
            left = left
                .child(div().text_color(rgb(c_accent())).child(format!("{} ›", first.source)))
                .child(div().flex_none().font_family(self.mono.clone()).child(self.line.clone()))
                .child(div().flex_none().font_family(self.mono.clone()).text_color(rgb(c_muted())).child(ghost))
                .child(div().flex_none().text_color(rgb(c_muted())).child(first.description.clone().map_or("· → aceitar".into(), |d| format!("· {d} · → aceitar"))));
            for s in rest {
                left = left.child(div().flex_none().font_family(self.mono.clone()).text_color(rgb(c_muted())).child(format!("· {}", s.text)));
            }
        } else {
            left = left.child(
                div()
                    .font_family(self.mono.clone())
                    .text_size(px(12.))
                    .text_color(rgb(c_muted()))
                    .whitespace_nowrap()
                    .child(fit_hints(
                        &["ctrl-shift-t novo terminal", "ctrl-shift-f buscar", "ctrl-shift-n notas", "ctrl-shift-e barra lateral"],
                        (((self.tabs[self.active].rect.w - 240.0) / 7.4).max(0.0)) as usize,
                    )),
            );
        }

        // com o prompt nativo (BRASA_PROMPT=native) não há chips no terminal: mostra pasta e branch aqui
        let info = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .text_xs()
            .text_color(rgb(c_muted()))
            .when(self.prompt_text.is_none(), |d| {
                d.when_some(self.git.clone(), |d, b| d.child(div().text_color(rgb(GREEN)).child(format!("⎇ {b}"))))
                    .child(shorten(&self.shell_cwd))
            })
            .child(format!("{cols}×{rows} · {}px", self.font_size as u32));

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(MARGIN))
            .px(px(MARGIN))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .p(px(TERM_PAD))
                    .overflow_hidden()
                    .bg(rgb(c_term_bg()))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(c_border()))
                    .font_family(self.mono.clone())
                    .text_size(px(self.font_size))
                    .line_height(h)
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| this.on_scroll(ev, cx)))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.on_term_down(ev, cx)))
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.on_term_down(ev, cx)))
                    .on_mouse_down(MouseButton::Middle, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.on_term_down(ev, cx)))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.on_term_move(ev, cx)))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _, cx| this.on_term_up(ev, cx)))
                    .on_mouse_up(MouseButton::Right, cx.listener(|this, ev: &MouseUpEvent, _, cx| this.on_term_up(ev, cx)))
                    .on_mouse_up(MouseButton::Middle, cx.listener(|this, ev: &MouseUpEvent, _, cx| this.on_term_up(ev, cx)))
                    .child(div().flex().flex_col().children(lines))
                    .when(offset > 0, |d| {
                        d.child(
                            div()
                                .id("to-bottom")
                                .absolute()
                                .bottom(px(14.))
                                .right(px(20.))
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .text_xs()
                                .bg(rgb(c_accent()))
                                .text_color(rgb(c_bg()))
                                .cursor_pointer()
                                .child(format!("↓ {offset} linhas acima · ir ao fim"))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.tab().term.scroll_to_bottom();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(div().h(px(crate::STATUS_H)).flex_none().px_2().flex().items_center().gap_3().text_xs().child(left).child(info))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TermView;
    use ox_pty::{PROMPT_BRANCH, PROMPT_START};

    fn screen(text: &str) -> Vec<RowView> {
        let mut t = TermView::new(40, 10);
        t.feed(text.as_bytes());
        t.rows_to_runs(None, None)
    }

    fn prompt(path: &str) -> String {
        format!("{PROMPT_START}{path}{PROMPT_BRANCH}main\r\n❯ ")
    }

    #[test]
    fn separators_between_prompts_commands_and_output() {
        // bloco 1 (com saída), bloco 2 (sem saída) e o prompt atual
        let text = format!("{}ls\r\narq1 arq2\r\n{}true\r\n{}", prompt("~/a"), prompt("~/a"), prompt("~/a"));
        let sep = separators(&screen(&text));
        // linhas: 0 prompt1 | 1 comando "ls" | 2 saída | 3 prompt2 | 4 "true" | 5 prompt3 | 6 comando vazio
        assert_eq!(sep[0], (None, None), "primeira linha não tem linha acima");
        assert_eq!(sep[1], (None, Some(c_sep())), "comando com saída ganha linha embaixo");
        assert_eq!(sep[3].0, Some(c_sep()), "prompt antigo: linha comum acima");
        assert_eq!(sep[4].1, None, "comando sem saída não ganha linha embaixo");
        assert_eq!(sep[5].0, Some(c_sep_active()), "prompt atual: linha destacada");
        assert_eq!(sep[6], (None, None), "comando ainda vazio não tem linha embaixo");
    }

    #[test]
    fn no_separators_without_prompts() {
        // por exemplo, dentro de vim/less: nenhuma linha extra
        assert!(separators(&screen("texto solto\r\noutra linha")).iter().all(|s| *s == (None, None)));
    }
}
