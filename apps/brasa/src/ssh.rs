//! Conectar por SSH (Ctrl+Shift+H): cartão com os hosts do `~/.ssh/config`. Enter abre uma sessão nova
//! que já roda `ssh <host>`, com o nome do host no lugar de "shell N". Também aceita digitar
//! `usuario@maquina` à mão. A sessão remota é um comando em execução como qualquer outro: o Brasa não
//! instala nada na máquina remota.

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent};
use ox_suggest::{parse_ssh_config, valid_ssh_target, SshHost};

use crate::icons::wicon;
use crate::theme::*;
use crate::Workspace;

pub struct SshPicker {
    pub query: String,
    pub sel: usize,
    hosts: Vec<SshHost>,
}

const MAX_ROWS: usize = 9;

/// Uma linha do cartão: um host conhecido ou o destino digitado.
#[derive(Clone)]
struct Choice {
    target: String,
    detail: String,
    typed: bool,
}

fn read_hosts() -> Vec<SshHost> {
    let path = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join(".ssh").join("config");
    parse_ssh_config(&std::fs::read_to_string(path).unwrap_or_default())
}

/// Hosts que combinam com o texto (começo do apelido primeiro, depois qualquer pedaço) e, no fim, o que foi digitado.
fn choices(hosts: &[SshHost], query: &str) -> Vec<Choice> {
    let q = query.trim().to_lowercase();
    let hit = |h: &SshHost| h.alias.to_lowercase().contains(&q) || h.detail().to_lowercase().contains(&q);
    let mut found: Vec<&SshHost> = hosts.iter().filter(|h| q.is_empty() || hit(h)).collect();
    found.sort_by_key(|h| !h.alias.to_lowercase().starts_with(&q));
    let mut out: Vec<Choice> = found.into_iter().take(MAX_ROWS).map(|h| Choice { target: h.alias.clone(), detail: h.detail(), typed: false }).collect();
    let typed = query.trim();
    if !typed.is_empty() && valid_ssh_target(typed) && !out.iter().any(|c| c.target == typed) {
        out.push(Choice { target: typed.to_string(), detail: String::new(), typed: true });
    }
    out
}

impl Workspace {
    pub fn open_ssh_picker(&mut self) {
        self.ssh = Some(SshPicker { query: String::new(), sel: 0, hosts: read_hosts() });
        self.hist = None;
        self.gsearch_focused = false;
        self.focus_editor = false;
    }

    /// Abre uma sessão nova e manda `ssh <destino>` para ela.
    pub fn connect_ssh(&mut self, target: &str) {
        self.ssh = None;
        if !valid_ssh_target(target) {
            return;
        }
        let cwd = self.shell_cwd.clone();
        self.new_tab(Some(cwd));
        let idx = self.active;
        if let Some(t) = self.tabs.get_mut(idx) {
            t.name = Some(target.to_string());
        }
        let _ = self.pty.write(self.session(), format!("ssh {target}\n").as_bytes());
    }

    pub fn on_ssh_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return false;
        }
        let Some(s) = self.ssh.as_ref() else { return false };
        let list = choices(&s.hosts, &s.query);
        let n = list.len().max(1);
        let s = self.ssh.as_mut().unwrap();
        match k.key.to_lowercase().as_str() {
            "escape" => self.ssh = None,
            "enter" => {
                let pick = list.get(s.sel.min(list.len().saturating_sub(1))).map(|c| c.target.clone());
                match pick {
                    Some(t) => self.connect_ssh(&t),
                    None => self.ssh = None,
                }
            }
            "down" => s.sel = (s.sel + 1) % n,
            "up" => s.sel = (s.sel + n - 1) % n,
            "backspace" => {
                s.query.pop();
                s.sel = 0;
            }
            "tab" | "left" | "right" | "home" | "end" | "delete" | "space" => {}
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

    pub fn render_ssh_picker(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = self.ssh.as_ref()?;
        let query = s.query.clone();
        let list = choices(&s.hosts, &query);
        let sel = s.sel.min(list.len().saturating_sub(1));
        let no_hosts = s.hosts.is_empty();

        let mut rows = div().flex().flex_col().py_1();
        if list.is_empty() {
            let msg = if query.trim().is_empty() && no_hosts {
                "Nenhum host em ~/.ssh/config. Digite usuario@maquina para conectar."
            } else if query.trim().is_empty() {
                "Nenhum host."
            } else {
                "Nenhum host combina. O destino não pode ter espaço nem começar com -."
            };
            rows = rows.child(div().px_3().py_2().text_sm().text_color(rgb(c_muted())).child(msg));
        }
        for (i, c) in list.iter().enumerate() {
            let on = i == sel;
            let target = c.target.clone();
            rows = rows.child(
                div()
                    .id(("ssh-row", i))
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
                    .child(wicon("terminal", 14., c_accent()))
                    .child(div().flex_none().text_sm().font_family(self.mono.clone()).text_color(rgb(c_text())).child(if c.typed { format!("ssh {}", c.target) } else { c.target.clone() }))
                    .child(div().flex_1().min_w_0().truncate().text_xs().text_color(rgb(c_disabled())).child(c.detail.clone()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.connect_ssh(&target);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }

        Some(
            div()
                .id("ssh-backdrop")
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
                        this.ssh = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .w(px(560.))
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
                                    div().flex_1().text_color(rgb(c_muted())).child("Host ou usuario@maquina")
                                } else {
                                    div().flex_1().min_w_0().truncate().child(format!("{query}▏"))
                                })
                                .child(div().flex_none().text_xs().text_color(rgb(c_disabled())).child("Enter conecta · Esc fecha")),
                        )
                        .child(div().px_4().pt_2().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(rgb(c_muted())).child("Hosts do ~/.ssh/config"))
                        .child(rows),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts() -> Vec<SshHost> {
        parse_ssh_config("Host web\n HostName 10.0.0.5\nHost banco\n HostName db.exemplo.com\n User ana\nHost webmail\n")
    }

    #[test]
    fn empty_query_lists_every_host() {
        let c = choices(&hosts(), "");
        assert_eq!(c.iter().map(|x| x.target.as_str()).collect::<Vec<_>>(), ["web", "banco", "webmail"]);
        assert!(c.iter().all(|x| !x.typed));
    }

    #[test]
    fn query_filters_and_prefers_the_start_of_the_name() {
        let c = choices(&hosts(), "web");
        assert_eq!(c[0].target, "web");
        assert_eq!(c.len(), 2); // web e webmail; "web" já existe, então não repete o digitado
        let by_detail = choices(&hosts(), "exemplo");
        assert_eq!(by_detail[0].target, "banco");
    }

    #[test]
    fn unknown_valid_text_becomes_a_typed_target() {
        let c = choices(&hosts(), "ana@novo.exemplo.com");
        assert_eq!(c.len(), 1);
        assert!(c[0].typed);
        assert!(choices(&hosts(), "-oProxyCommand=x").is_empty());
        assert!(choices(&hosts(), "a b").is_empty());
    }
}
