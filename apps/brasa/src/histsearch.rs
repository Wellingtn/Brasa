//! Busca no histórico (Ctrl+R): cartão no meio da janela com os comandos já usados, do mais relevante para
//! o menos (recência, frequência e pasta atual). Setas escolhem, Enter põe o comando na linha do shell
//! (sem executar), Esc fecha. Também grava cada comando executado no log do Brasa.

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent};
use ox_suggest::Ranked;

use crate::icons::wicon;
use crate::theme::*;
use crate::Workspace;

pub struct HistSearch {
    pub query: String,
    pub sel: usize,
}

const MAX_ROWS: usize = 10;

/// "agora", "há 5 min", "há 3 h", "há 2 d"; vazio quando a hora não é conhecida.
fn ago(ts: u64, now: u64) -> String {
    if ts == 0 || ts > now {
        return String::new();
    }
    let s = now - ts;
    match s {
        0..=59 => "agora".into(),
        60..=3599 => format!("há {} min", s / 60),
        3600..=86_399 => format!("há {} h", s / 3600),
        86_400..=2_591_999 => format!("há {} d", s / 86_400),
        _ => format!("há {} meses", s / 2_592_000),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Workspace {
    pub fn history_log_path() -> std::path::PathBuf {
        crate::config_dir().join("history.tsv")
    }

    /// Guarda um comando executado (com a pasta) no histórico compartilhado e no arquivo.
    pub fn record_command(&mut self, cmd: &str) {
        let cwd = self.shell_cwd.to_string_lossy().into_owned();
        if let Ok(mut h) = self.history.write() {
            h.record(&Self::history_log_path(), cmd, &cwd, now_secs());
        }
    }

    fn hist_results(&self) -> Vec<Ranked> {
        let Some(s) = &self.hist else { return Vec::new() };
        let cwd = self.shell_cwd.to_string_lossy().into_owned();
        self.history.read().map(|h| h.search(&s.query, &cwd, MAX_ROWS)).unwrap_or_default()
    }

    pub fn open_hist_search(&mut self) {
        self.hist = Some(HistSearch { query: String::new(), sel: 0 });
        self.gsearch_focused = false;
        self.focus_editor = false;
    }

    /// Põe o comando na linha do shell, no lugar do que estava digitado, sem executar.
    fn insert_history_command(&mut self, cmd: &str) {
        // Ctrl+A e Ctrl+K apagam a linha inteira, esteja onde estiver o cursor
        let mut bytes = vec![0x01, 0x0b];
        bytes.extend_from_slice(cmd.as_bytes());
        let _ = self.pty.write(self.session(), &bytes);
        self.line = cmd.to_string();
        self.hist = None;
        self.refresh_suggestions();
    }

    /// Teclas no cartão. Atalhos com Ctrl/Alt passam adiante, menos o próprio Ctrl+R, que fecha.
    pub fn on_hist_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return false;
        }
        let results = self.hist_results();
        let Some(s) = self.hist.as_mut() else { return false };
        match k.key.to_lowercase().as_str() {
            "escape" => self.hist = None,
            "enter" => {
                let pick = results.get(s.sel.min(results.len().saturating_sub(1))).map(|r| r.cmd.clone());
                match pick {
                    Some(cmd) => self.insert_history_command(&cmd),
                    None => self.hist = None,
                }
            }
            "down" => s.sel = (s.sel + 1) % results.len().max(1),
            "up" => s.sel = (s.sel + results.len().max(1) - 1) % results.len().max(1),
            "backspace" => {
                s.query.pop();
                s.sel = 0;
            }
            "space" => {
                s.query.push(' ');
                s.sel = 0;
            }
            "tab" | "left" | "right" | "home" | "end" | "delete" => {}
            _ => {
                if let Some(c) = &k.key_char {
                    s.query.push_str(c);
                    s.sel = 0;
                }
            }
        }
        cx.notify();
        true
    }

    pub fn render_hist_search(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = self.hist.as_ref()?;
        let (query, sel) = (s.query.clone(), s.sel);
        let results = self.hist_results();
        let sel = sel.min(results.len().saturating_sub(1));
        let now = now_secs();

        let mut list = div().flex().flex_col().py_1();
        if results.is_empty() {
            list = list.child(div().px_3().py_2().text_sm().text_color(rgb(c_muted())).child(if query.trim().is_empty() { "Ainda não há comandos no histórico." } else { "Nenhum comando combina." }));
        }
        for (i, r) in results.iter().enumerate() {
            let on = i == sel;
            let cmd = r.cmd.clone();
            let meta = {
                let when = ago(r.last_ts, now);
                let times = if r.count > 1 { format!("{}×", r.count) } else { String::new() };
                [times, when].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
            };
            list = list.child(
                div()
                    .id(("hist-row", i))
                    .mx_1()
                    .px_2()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_md()
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgb(c_surface())))
                    .when(!on, |d| d.hover(|s| s.bg(rgb(c_hover()))))
                    .child(div().flex_1().min_w_0().truncate().text_sm().font_family(self.mono.clone()).text_color(rgb(c_text())).child(r.cmd.clone()))
                    .child(div().flex_none().text_xs().text_color(rgb(c_disabled())).child(meta))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.insert_history_command(&cmd);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }

        Some(
            div()
                .id("hist-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(110.))
                .bg(rgba(0x00000066))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.hist = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .w(px(640.))
                        .max_w_full()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(c_card_border()))
                        .bg(rgb(c_panel()))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .h(px(44.))
                                .px_4()
                                .flex()
                                .items_center()
                                .gap_3()
                                .border_b_1()
                                .border_color(rgb(c_divider()))
                                .child(wicon("search", 15., c_accent()))
                                .child(if query.is_empty() {
                                    div().flex_1().text_color(rgb(c_muted())).child("Buscar no histórico de comandos")
                                } else {
                                    div().flex_1().min_w_0().truncate().child(format!("{query}▏"))
                                })
                                .child(div().flex_none().text_xs().text_color(rgb(c_disabled())).child("Enter coloca na linha · Esc fecha")),
                        )
                        .child(div().px_4().pt_2().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(rgb(c_muted())).child(if query.trim().is_empty() { "Mais relevantes agora" } else { "Resultados" }))
                        .child(list),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times_are_readable() {
        assert_eq!(ago(0, 1000), "");
        assert_eq!(ago(2000, 1000), "");
        assert_eq!(ago(990, 1000), "agora");
        assert_eq!(ago(1000 - 300, 1000), "há 5 min");
        assert_eq!(ago(100_000 - 7200, 100_000), "há 2 h");
        assert_eq!(ago(1_000_000 - 3 * 86_400, 1_000_000), "há 3 d");
        assert_eq!(ago(10_000_000 - 5_000_000, 10_000_000), "há 1 meses");
    }
}
