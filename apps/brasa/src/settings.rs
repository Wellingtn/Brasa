//! Página de configurações (ocupa a área à direita da barra lateral). Abas na lateral: Geral (sobre,
//! pastas, atalhos, restaurar) e Aparência (cor do tema, fundo, fonte e tamanho com pré-visualização).
//! As escolhas valem na hora e ficam salvas em `~/.config/brasa` (cor e fundo em `theme.txt`, fonte e
//! tamanho em `settings.txt`).

use std::{cell::Cell, rc::Rc};

use gpui::{
    canvas, div, point, prelude::*, px, quad, rgb, size, AnyElement, Bounds, ClickEvent, Context, Div, DispatchPhase, Font, FontFeatures, FontStyle, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, SharedString, Stateful, Window,
};

use crate::theme::*;
use crate::{Workspace, MONO_CANDIDATES};

fn file() -> std::path::PathBuf {
    crate::config_dir().join("settings.txt")
}

/// Fonte e tamanho do terminal salvos, se houver.
pub fn load() -> (Option<String>, Option<f32>) {
    parse(&std::fs::read_to_string(file()).unwrap_or_default())
}

fn parse(text: &str) -> (Option<String>, Option<f32>) {
    let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string());
    (get("fonte="), get("tamanho=").and_then(|v| v.parse().ok()))
}

/// Grava (ou troca) uma chave de `settings.txt`, sem mexer nas outras.
fn write_key(key: &str, value: &str) {
    let text = std::fs::read_to_string(file()).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with(&format!("{key}="))).map(str::to_string).collect();
    lines.push(format!("{key}={value}"));
    let _ = std::fs::write(file(), lines.join("\n") + "\n");
}

pub fn save(font: &str, size: f32) {
    write_key("fonte", font);
    write_key("tamanho", &size.to_string());
}

/// Verificar se há versão nova ao abrir (ligado por padrão).
pub fn updates_enabled() -> bool {
    std::fs::read_to_string(file()).unwrap_or_default().lines().find_map(|l| l.strip_prefix("atualizacoes=")).map_or(true, |v| v.trim() != "nao")
}

pub fn set_updates_enabled(on: bool) {
    write_key("atualizacoes", if on { "sim" } else { "nao" });
}

/// (média, p95, máximo, quantas) de medidas em ms.
pub fn latency_stats(values: impl Iterator<Item = f32>) -> Option<(f32, f32, f32, usize)> {
    let mut v: Vec<f32> = values.collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let avg = v.iter().sum::<f32>() / v.len() as f32;
    let p95 = v[((v.len() as f32 * 0.95).ceil() as usize).clamp(1, v.len()) - 1];
    Some((avg, p95, *v.last().unwrap(), v.len()))
}

/// Campo de texto da página de configurações que está recebendo o teclado.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Font,
    AccentHex,
    BgHex,
    Preview,
}

/// texto de exemplo da pré-visualização; o usuário pode editar
pub const DEFAULT_PREVIEW: &str = "❯ cargo build --release\n   Compiling brasa v1.0.0\n    Finished `release` in 54s\n❯ git status\nmodified: src/main.rs\n0O 1lI {} [] () => != == && ||";

const HUE_CELLS: usize = 72;
/// margens do texto na pré-visualização
const PAD_X: f32 = 16.0;
const PAD_Y: f32 = 12.0;

/// Linha e coluna (de 0) de um cursor em `caret` caracteres.
fn caret_line_col(text: &str, caret: usize) -> (usize, usize) {
    let before: String = text.chars().take(caret).collect();
    (before.matches('\n').count(), before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0))
}

/// Botão de escolha: destacado quando é a opção atual.
fn chip(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .border_1()
        .border_color(rgb(if on { c_accent() } else { c_card_border() }))
        .bg(rgb(if on { c_surface() } else { c_panel() }))
        .text_color(rgb(if on { c_text() } else { c_muted() }))
        .hover(|s| s.bg(rgb(c_btn_hover())))
        .child(label.into())
}

fn row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(div().flex_none().w(px(150.)).text_sm().text_color(rgb(c_muted())).child(label))
        .child(div().flex_1().min_w_0().flex().flex_wrap().items_center().gap_2().child(control))
}

/// Campo de texto: mostra o valor (ou a dica) e um cursor quando em foco; o clique dá o foco.
fn field_box(id: &'static str, value: &str, hint: &'static str, focused: bool, w: f32) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(w))
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .rounded_md()
        .text_sm()
        .cursor_text()
        .border_1()
        .border_color(rgb(if focused { c_accent() } else { c_card_border() }))
        .bg(rgb(if focused { c_bg() } else { c_panel() }))
        .child(if value.is_empty() && !focused {
            div().text_color(rgb(c_muted())).child(hint)
        } else {
            div().child(format!("{value}{}", if focused { "▏" } else { "" }))
        })
}

fn swatch(color: u32, size: f32) -> Div {
    div().flex_none().size(px(size)).rounded_md().bg(rgb(color)).border_1().border_color(rgb(c_card_border()))
}

/// Seção com título e linhas.
fn section(title: &'static str, rows: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .pb_5()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .child(div().h(px(1.)).bg(rgb(c_divider())))
        .children(rows)
}

fn open_path(path: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

impl Workspace {
    fn field_buf(&mut self, f: SettingsField) -> &mut String {
        match f {
            SettingsField::Font => &mut self.font_query,
            SettingsField::AccentHex => &mut self.accent_hex,
            SettingsField::BgHex => &mut self.bg_hex,
            SettingsField::Preview => &mut self.preview_text,
        }
    }

    /// Teclado na página de configurações. Devolve `true` quando a tecla foi usada aqui (inclusive digitar
    /// sem campo em foco, que não deve chegar ao terminal escondido); atalhos com Ctrl/Alt passam adiante.
    pub fn on_settings_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let key = k.key.to_lowercase();
        // gravando um atalho: a próxima combinação com Ctrl, Alt ou Super vira o atalho da ação
        if let Some(action) = self.rebinding {
            let combo = crate::keys::combo_of(k);
            if key == "escape" {
                self.rebinding = None;
                self.keys_notice = None;
            } else if crate::keys::is_valid(&combo) {
                match self.keymap.set(action, &combo) {
                    Ok(()) => {
                        self.keymap.save();
                        self.rebinding = None;
                        self.keys_notice = None;
                    }
                    Err(other) => {
                        let name = crate::keys::ACTIONS.iter().find(|x| x.0 == other).map(|x| x.2).unwrap_or("outra ação");
                        self.keys_notice = Some(format!("{} já é de \"{name}\". Escolha outro ou aperte Esc.", crate::keys::pretty(&combo)));
                    }
                }
            } else if !combo.is_empty() {
                self.keys_notice = Some("Use Ctrl, Alt ou Super junto com a tecla (ex.: Ctrl+Shift+K). Esc cancela.".to_string());
            }
            cx.notify();
            return true;
        }
        let Some(field) = self.settings_field else {
            if key == "escape" {
                self.show_settings = false;
                cx.notify();
                return true;
            }
            return !(k.modifiers.control || k.modifiers.alt || k.modifiers.platform);
        };
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return false;
        }
        if field == SettingsField::Preview {
            return self.preview_key(k, &key, cx);
        }
        let hex = matches!(field, SettingsField::AccentHex | SettingsField::BgHex);
        match key.as_str() {
            "escape" | "tab" => self.settings_field = None,
            "enter" => {
                if field == SettingsField::Preview {
                    self.preview_text.push('\n');
                } else {
                    self.settings_field = None;
                }
            }
            "backspace" => {
                self.field_buf(field).pop();
            }
            "space" => {
                if !hex {
                    self.field_buf(field).push(' ');
                }
            }
            _ => {
                if let Some(c) = &k.key_char {
                    for ch in c.chars() {
                        if hex && !(ch.is_ascii_hexdigit() || (ch == '#' && self.field_buf(field).is_empty())) {
                            continue;
                        }
                        if hex && self.field_buf(field).chars().filter(|c| *c != '#').count() >= 6 && ch != '#' {
                            continue;
                        }
                        self.field_buf(field).push(ch);
                    }
                }
            }
        }
        // cor válida: aplica na hora
        match field {
            SettingsField::AccentHex => {
                if let Some(c) = parse_hex(&self.accent_hex).filter(|_| self.accent_hex.trim_start_matches('#').len() >= 3) {
                    set_custom_accent(c);
                }
            }
            SettingsField::BgHex => {
                if let Some(c) = parse_hex(&self.bg_hex).filter(|_| self.bg_hex.trim_start_matches('#').len() >= 3) {
                    set_custom_bg(c);
                }
            }
            SettingsField::Font | SettingsField::Preview => {}
        }
        cx.notify();
        true
    }

    /// Edição do texto da pré-visualização: cursor em qualquer posição, setas, Home/End, Delete.
    fn preview_key(&mut self, k: &gpui::Keystroke, key: &str, cx: &mut Context<Self>) -> bool {
        let mut chars: Vec<char> = self.preview_text.chars().collect();
        let mut c = self.preview_caret.min(chars.len());
        match key {
            "escape" | "tab" => self.settings_field = None,
            "enter" => {
                chars.insert(c, '\n');
                c += 1;
            }
            "backspace" => {
                if c > 0 {
                    chars.remove(c - 1);
                    c -= 1;
                }
            }
            "delete" => {
                if c < chars.len() {
                    chars.remove(c);
                }
            }
            "left" => c = c.saturating_sub(1),
            "right" => c = (c + 1).min(chars.len()),
            "home" | "end" | "up" | "down" => {
                let mut starts = vec![0usize];
                starts.extend(chars.iter().enumerate().filter(|(_, ch)| **ch == '\n').map(|(i, _)| i + 1));
                let line = starts.iter().rposition(|&s| s <= c).unwrap_or(0);
                let col = c - starts[line];
                let end_of = |l: usize| if l + 1 < starts.len() { starts[l + 1] - 1 } else { chars.len() };
                match key {
                    "home" => c = starts[line],
                    "end" => c = end_of(line),
                    "up" => {
                        if line > 0 {
                            c = starts[line - 1] + col.min(end_of(line - 1) - starts[line - 1]);
                        }
                    }
                    _ => {
                        if line + 1 < starts.len() {
                            c = starts[line + 1] + col.min(end_of(line + 1) - starts[line + 1]);
                        }
                    }
                }
            }
            "space" => {
                chars.insert(c, ' ');
                c += 1;
            }
            _ => {
                if let Some(s) = &k.key_char {
                    for ch in s.chars().filter(|ch| !ch.is_control()) {
                        chars.insert(c, ch);
                        c += 1;
                    }
                }
            }
        }
        self.preview_text = chars.into_iter().collect();
        self.preview_caret = c;
        self.follow_preview_caret();
        cx.notify();
        true
    }

    /// Rola a pré-visualização para o cursor continuar à vista, em linhas e colunas.
    pub fn follow_preview_caret(&mut self) {
        let w = f32::from_bits(self.preview_size[0].load(std::sync::atomic::Ordering::Relaxed));
        let h = f32::from_bits(self.preview_size[1].load(std::sync::atomic::Ordering::Relaxed));
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let cols = (((w - 2.0 * PAD_X) / self.cell_w).floor() as usize).max(1);
        let rows = (((h - 2.0 * PAD_Y) / self.cell_h).floor() as usize).max(1);
        let (line, col) = caret_line_col(&self.preview_text, self.preview_caret);
        let (sx, sy) = &mut self.preview_scroll;
        if col < *sx {
            *sx = col;
        } else if col >= *sx + cols {
            *sx = col + 1 - cols;
        }
        if line < *sy {
            *sy = line;
        } else if line >= *sy + rows {
            *sy = line + 1 - rows;
        }
    }

    /// Fontes de largura fixa instaladas (o terminal só funciona bem com elas): a largura de "i" e "m" é igual.
    fn mono_font_list(&mut self, window: &Window) -> Vec<String> {
        if let Some(list) = &self.mono_fonts {
            return list.clone();
        }
        let ts = window.text_system();
        let mut names: Vec<String> = ts.all_font_names().into_iter().filter(|n| !n.starts_with('.')).collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        let list: Vec<String> = names
            .into_iter()
            .filter(|name| {
                let font = Font { family: name.clone().into(), features: FontFeatures::default(), fallbacks: None, weight: FontWeight::NORMAL, style: FontStyle::Normal };
                let id = ts.resolve_font(&font);
                let adv = |c: char| ts.advance(id, px(16.), c).ok().map(|s| f32::from(s.width));
                matches!((adv('i'), adv('m'), adv('W')), (Some(a), Some(b), Some(w)) if (a - b).abs() < 0.05 && (w - b).abs() < 0.05 && b > 1.0)
            })
            .collect();
        let list = if list.is_empty() { MONO_CANDIDATES.iter().map(|s| s.to_string()).collect() } else { list };
        self.mono_fonts = Some(list.clone());
        list
    }

    pub fn render_settings(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let tabs = ["Geral", "Aparência"];
        let nav = div()
            .flex_none()
            .w(px(210.))
            .h_full()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_4()
            .border_r_1()
            .border_color(rgb(c_divider()))
            .child(div().px_3().pb_3().text_xs().text_color(rgb(c_muted())).child("Configurações"))
            .children(tabs.iter().enumerate().map(|(i, name)| {
                let on = i == self.settings_tab;
                div()
                    .id(("settings-tab", i))
                    .px_3()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgb(c_surface())).text_color(rgb(c_accent())).font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.text_color(rgb(c_muted())).hover(|s| s.bg(rgb(c_hover())).text_color(rgb(c_text()))))
                    .child(*name)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.settings_tab = i;
                        this.settings_field = None;
                        cx.notify();
                    }))
            }));

        let title = tabs[self.settings_tab.min(tabs.len() - 1)];
        let content = if self.settings_tab == 0 { self.settings_general(cx) } else { self.settings_appearance(window, cx) };

        div()
            .id("settings-page")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .bg(rgb(c_bg()))
            .child(nav)
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .px_10()
                    .py_6()
                    .child(
                        div()
                            .max_w(px(760.))
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(title))
                            .child(content),
                    ),
            )
            .into_any_element()
    }

    fn settings_general(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let notes_dir = crate::config_dir().join("notes");
        let conf_dir = crate::config_dir();
        let shell = self.pty.shell_name(self.tabs[self.active].session).unwrap_or_else(|| "shell".into());

        let mut shortcut_rows: Vec<AnyElement> = Vec::new();
        if let Some(msg) = self.keys_notice.clone() {
            shortcut_rows.push(div().text_sm().text_color(rgb(YELLOW)).child(msg).into_any_element());
        }
        // duas tabelas lado a lado, cada uma com Nome | Comando
        let all: Vec<(usize, crate::keys::Action, &'static str)> = crate::keys::ACTIONS.iter().enumerate().map(|(n, x)| (n, x.0, x.2)).collect();
        let right = all[all.len().div_ceil(2)..].to_vec();
        let left = all[..all.len().div_ceil(2)].to_vec();
        let (left, right) = (self.shortcut_table(left, cx), self.shortcut_table(right, cx));
        shortcut_rows.push(div().w_full().flex().items_start().gap_4().child(left).child(right).into_any_element());
        shortcut_rows.push(div().text_xs().text_color(rgb(c_disabled())).child("Clique num atalho e aperte o novo. Eles ficam em ~/.config/brasa/keys.txt.").into_any_element());
        let updates = self.updates_section(cx);
        let diagnostics = self.diagnostics_section(cx);
        let folder = |id: &'static str, label: &'static str, path: std::path::PathBuf| {
            let shown = path.display().to_string();
            row(
                label,
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_sm().font_family(self.mono.clone()).text_color(rgb(c_text())).child(shown))
                    .child(chip(id, "Abrir", false).on_click(cx.listener(move |_, _: &ClickEvent, _, _| open_path(&path)))),
            )
            .into_any_element()
        };
        let text_row = |label: &'static str, value: String| row(label, div().text_sm().child(value)).into_any_element();


        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(section(
                "Sobre",
                vec![text_row("Versão", env!("CARGO_PKG_VERSION").to_string()), text_row("Shell", shell.to_string())],
            ))
            .child(updates)
            .child(section("Pastas", vec![folder("open-notes", "Notas", notes_dir), folder("open-config", "Configuração", conf_dir)]))
            .child(section("Atalhos", shortcut_rows))
            .child(diagnostics)
            .child(section(
                "Restaurar",
                vec![row(
                    "Aparência",
                    chip("reset-appearance", "Restaurar padrões", false).on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.reset_appearance(window);
                        cx.notify();
                    })),
                )
                .into_any_element()],
            ))
            .child(div().text_xs().text_color(rgb(c_disabled())).child("Mais opções aparecem aqui nas próximas versões."))
            .into_any_element()
    }

    /// Média, p95 e máximo das últimas medidas de latência (ms); `None` sem amostras.
    pub fn latency_summary(&self) -> Option<(f32, f32, f32, usize)> {
        latency_stats(self.latency_ms.iter().copied())
    }

    fn diagnostics_section(&mut self, cx: &mut Context<Self>) -> Div {
        let text = match self.latency_summary() {
            Some((avg, p95, max, n)) => format!("média {avg:.1} ms · p95 {p95:.1} ms · máx {max:.1} ms · {n} teclas"),
            None => "Digite no terminal para medir".to_string(),
        };
        section(
            "Diagnóstico",
            vec![
                row("Latência do teclado", div().text_sm().font_family(self.mono.clone()).child(text)).into_any_element(),
                row(
                    "",
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(chip("latency-reset", "Zerar", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.latency_ms.clear();
                            cx.notify();
                        })))
                        .child(div().text_xs().text_color(rgb(c_disabled())).child("Da tecla até o início do desenho na tela (sem contar o monitor). Só conta quando o shell responde com eco.")),
                )
                .into_any_element(),
            ],
        )
    }

    fn updates_section(&mut self, cx: &mut Context<Self>) -> Div {
        use crate::update::UpdateState;
        let enabled = updates_enabled();
        let (status, color) = match &self.update {
            UpdateState::Unknown => ("Ainda não verificado".to_string(), c_muted()),
            UpdateState::Checking => ("Verificando…".to_string(), c_muted()),
            UpdateState::UpToDate => ("Você está na versão mais recente".to_string(), GREEN),
            UpdateState::Available { version, .. } => (format!("Nova versão disponível: {version}"), c_accent()),
            UpdateState::Installing => (self.update_progress.lock().unwrap().clone(), c_accent()),
            UpdateState::InstallFailed(why) => (format!("Não deu para atualizar: {why}"), YELLOW),
            UpdateState::NoPublicRelease => ("Nenhuma versão pública encontrada no GitHub".to_string(), c_muted()),
            UpdateState::Failed => ("Não foi possível verificar (sem internet ou sem o curl)".to_string(), c_muted()),
        };
        let mut actions = div().flex().items_center().gap_2().child(
            chip("check-updates", "Verificar agora", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.check_updates(cx);
                cx.notify();
            })),
        );
        if let UpdateState::Available { url, .. } = &self.update {
            let url = url.clone();
            if crate::update::install_kind() != crate::update::InstallKind::Source {
                actions = actions.child(chip("update-now", "Atualizar agora", true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.start_update(cx))));
            }
            actions = actions.child(chip("open-release", "Abrir página da versão", false).on_click(cx.listener(move |_, _: &ClickEvent, _, _| open_path(std::path::Path::new(&url)))));
        }
        let toggle = div()
            .flex()
            .gap_2()
            .child(chip("updates-on", "Sim", enabled).on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                self::set_updates_enabled(true);
                cx.notify();
            })))
            .child(chip("updates-off", "Não", !enabled).on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                self::set_updates_enabled(false);
                cx.notify();
            })));
        section(
            "Atualizações",
            vec![
                row("Status", div().text_sm().text_color(rgb(color)).child(status)).into_any_element(),
                row("Verificar ao abrir", toggle).into_any_element(),
                row("", actions).into_any_element(),
            ],
        )
    }

    /// Pergunta ao GitHub se há versão nova, sem travar a janela.
    pub fn check_updates(&mut self, cx: &mut Context<Self>) {
        self.update = crate::update::UpdateState::Checking;
        cx.spawn(async move |this, cx| {
            let state = cx.background_executor().spawn(async { crate::update::check(env!("CARGO_PKG_VERSION")) }).await;
            let _ = this.update(cx, |ws, cx| {
                ws.update = state;
                cx.notify();
            });
        })
        .detach();
    }

    /// Uma tabela de atalhos (Nome | Comando) com as ações dadas. As duas colunas têm largura fixa (metade
    /// cada), então a linha vertical fica no mesmo lugar no cabeçalho e em todas as linhas.
    fn shortcut_table(&self, items: Vec<(usize, crate::keys::Action, &'static str)>, cx: &mut Context<Self>) -> Div {
        let line = c_divider();
        let name_col = || div().flex_none().w(gpui::relative(0.5)).h_full().flex().items_center().px_3().border_r_1().border_color(rgb(line));
        let cmd_col = || div().flex_1().min_w_0().h_full().flex().items_center().justify_between().gap_2().px_3();
        let mut table = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(rgb(c_card_border()))
            .overflow_hidden()
            .child(
                div()
                    .h(px(32.))
                    .flex_none()
                    .flex()
                    .bg(rgb(c_panel()))
                    .border_b_1()
                    .border_color(rgb(line))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(c_muted()))
                    .child(name_col().child("Nome"))
                    .child(cmd_col().child("Comando")),
            );
        for (i, (n, action, desc)) in items.into_iter().enumerate() {
            let recording = self.rebinding == Some(action);
            let shown = if recording { "Aperte o atalho…".to_string() } else { self.keymap.combos(action).iter().map(|c| crate::keys::pretty(c)).collect::<Vec<_>>().join(" ou ") };
            // combinação: etiqueta pequena em fonte monoespaçada; ao gravar, ganha a cor do tema
            let key_chip = div()
                .id(("key", n))
                .flex_none()
                .h(px(26.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded_md()
                .border_1()
                .border_color(rgb(if recording { c_accent() } else { c_card_border() }))
                .bg(rgb(if recording { c_surface() } else { c_bg() }))
                .text_xs()
                .font_family(self.mono.clone())
                .text_color(rgb(if recording { c_accent() } else { c_text() }))
                .cursor_pointer()
                .hover(|s| s.border_color(rgb(c_muted())))
                .child(shown)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.rebinding = Some(action);
                    this.keys_notice = None;
                    this.settings_field = None;
                    cx.notify();
                }));
            let mut cmd = cmd_col().child(key_chip);
            if !self.keymap.is_default(action) {
                cmd = cmd.child(
                    div()
                        .id(("key-reset", n))
                        .flex_none()
                        .text_xs()
                        .text_color(rgb(c_muted()))
                        .cursor_pointer()
                        .hover(|s| s.text_color(rgb(c_accent())))
                        .child("Restaurar")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.keymap.reset(action);
                            this.keymap.save();
                            cx.notify();
                        })),
                );
            }
            table = table.child(
                div()
                    .h(px(42.))
                    .flex_none()
                    .flex()
                    .when(i > 0, |d| d.border_t_1().border_color(rgb(line)))
                    .child(name_col().text_sm().text_color(rgb(c_text())).child(div().min_w_0().child(desc)))
                    .child(cmd),
            );
        }
        table
    }

    /// Baixa, confere e instala a versão nova em segundo plano; no fim, reabre o Brasa.
    pub fn start_update(&mut self, cx: &mut Context<Self>) {
        let crate::update::UpdateState::Available { assets, .. } = self.update.clone() else { return };
        let kind = crate::update::install_kind();
        self.update = crate::update::UpdateState::Installing;
        *self.update_progress.lock().unwrap() = "Baixando…".to_string();
        let progress = self.update_progress.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::update::install(&kind, &assets, &|step| *progress.lock().unwrap() = step.to_string()) })
                .await;
            let _ = this.update(cx, |ws, cx| match result {
                Ok(program) => ws.restart_into(program, cx),
                Err(e) => {
                    ws.update = crate::update::UpdateState::InstallFailed(e);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Fecha o Brasa e abre `program` em seguida (a pausa dá tempo de esta janela fechar de vez).
    fn restart_into(&mut self, program: std::path::PathBuf, cx: &mut Context<Self>) {
        self.save_note();
        let _ = std::process::Command::new("sh")
            .args(["-c", "sleep 1.5; exec \"$0\""])
            .arg(&program)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        cx.quit();
    }

    /// Volta cor, fundo, fonte e tamanho ao padrão de fábrica.
    pub fn reset_appearance(&mut self, window: &Window) {
        set_accent(0);
        set_bg(0);
        self.accent_hex = hex_string(ACCENTS[0].3);
        self.bg_hex = hex_string(custom_bg());
        let fonts = window.text_system().all_font_names();
        let family = MONO_CANDIDATES.iter().find(|c| fonts.iter().any(|f| f == *c)).copied().unwrap_or("monospace");
        self.mono = family.to_string().into();
        self.set_font_size(crate::DEFAULT_FONT, window);
        self.ed_cell_w = crate::measure(window, &self.mono, crate::ED_FONT).0;
    }

    /// Escolhe o matiz na roda: cor própria com o matiz dado.
    pub fn pick_hue(&mut self, hue: f32) {
        let color = hsl_to_rgb(hue, 0.72, 0.58);
        set_custom_accent(color);
        self.accent_hex = hex_string(color);
    }

    fn settings_appearance(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // ---- tema: roda de cores, cores prontas e hexadecimal ----
        let current = accent_index();
        let entity = cx.entity();
        let hue_now = hue_of(custom_accent());
        let wheel = color_wheel(entity, if current == CUSTOM { Some(hue_now) } else { None });
        let wheel_box = div().relative().flex_none().size(px(WHEEL)).child(wheel).child(
            // miolo da roda: a cor em uso e o hexadecimal
            div()
                .absolute()
                .top(px(WHEEL / 2.0 - 40.0))
                .left(px(WHEEL / 2.0 - 40.0))
                .size(px(80.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .child(div().size(px(34.)).rounded_full().bg(rgb(c_coral())).border_2().border_color(rgb(c_card_border())))
                .child(div().text_xs().font_family(self.mono.clone()).text_color(rgb(c_muted())).child(hex_string(c_coral()))),
        );

        let presets = div().flex().gap_3().children(ACCENTS.iter().enumerate().map(|(i, a)| {
            let on = i == current;
            div()
                .id(("accent", i))
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .child(
                    div()
                        .size(px(36.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_2()
                        .border_color(rgb(if on { c_text() } else { c_bg() }))
                        .child(div().size(px(26.)).rounded_full().bg(rgb(a.3))),
                )
                .child(div().text_xs().text_color(rgb(if on { c_text() } else { c_muted() })).child(a.1))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    set_accent(i);
                    this.accent_hex = hex_string(ACCENTS[i].3);
                    cx.notify();
                }))
        }));

        let accent_hex_row = div()
            .flex()
            .items_center()
            .gap_3()
            .child(swatch(c_coral(), 30.))
            .child(field_box("accent-hex", &self.accent_hex, "#e8805f", self.settings_field == Some(SettingsField::AccentHex), 120.).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.settings_field = Some(SettingsField::AccentHex);
                    cx.stop_propagation(); // o contêiner raiz tiraria o foco do campo
                    cx.notify();
                }),
            ));

        let theme_card = card(
            "Cor do tema",
            "Arraste na roda ou digite um hexadecimal. Vale o matiz: o logo, os destaques e as bordas acompanham.",
            div()
                .flex()
                .items_center()
                .gap_8()
                .child(wheel_box)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_5()
                        .child(field_group("Cores prontas", presets))
                        .child(field_group("Hexadecimal", accent_hex_row)),
                ),
        );

        // ---- fundo: mosaicos com miniatura ----
        let bg_on = bg_index();
        let mut tiles: Vec<AnyElement> = Vec::new();
        for i in 0..=BG_CUSTOM {
            let color = bg_preview(i);
            let label = if i == BG_CUSTOM { "Próprio" } else { BACKGROUNDS[i].1 };
            let on = i == bg_on;
            tiles.push(
                div()
                    .id(("bg", i))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(112.))
                            .h(px(70.))
                            .rounded_md()
                            .overflow_hidden()
                            .flex()
                            .border_2()
                            .border_color(rgb(if on { c_accent() } else { c_card_border() }))
                            .bg(rgb(color))
                            // miniatura: barra lateral e três linhas de texto
                            .child(div().w(px(26.)).h_full().bg(rgb(scale(color, 1.35))))
                            .child(
                                div()
                                    .flex_1()
                                    .p_2()
                                    .flex()
                                    .flex_col()
                                    .gap(px(5.))
                                    .child(div().h(px(5.)).w(px(46.)).rounded_full().bg(rgb(c_coral())))
                                    .child(div().h(px(4.)).w(px(60.)).rounded_full().bg(rgb(c_disabled())))
                                    .child(div().h(px(4.)).w(px(38.)).rounded_full().bg(rgb(c_disabled()))),
                            ),
                    )
                    .child(div().text_sm().text_color(rgb(if on { c_text() } else { c_muted() })).child(label))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        set_bg(i);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let bg_hex_row = div()
            .flex()
            .items_center()
            .gap_3()
            .child(swatch(bg_preview(BG_CUSTOM), 30.))
            .child(field_box("bg-hex", &self.bg_hex, "#161213", self.settings_field == Some(SettingsField::BgHex), 120.).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.settings_field = Some(SettingsField::BgHex);
                    cx.stop_propagation();
                    cx.notify();
                }),
            ));
        let bg_card = card(
            "Fundo",
            "Escolha um dos fundos prontos ou digite um hexadecimal para o seu (de preferência escuro).",
            div().flex().flex_col().gap_5().child(div().flex().flex_wrap().gap_4().children(tiles)).child(field_group("Hexadecimal do fundo próprio", bg_hex_row)),
        );

        // ---- fonte: lista com busca à esquerda; tamanho e pré-visualização à direita ----
        let fonts = self.mono_font_list(window);
        let query = self.font_query.trim().to_lowercase();
        let shown: Vec<String> = fonts.into_iter().filter(|f| query.is_empty() || f.to_lowercase().contains(&query)).collect();
        let current_font = self.mono.to_string();
        let font_search = field_box("font-search", &self.font_query, "Buscar fonte pelo nome", self.settings_field == Some(SettingsField::Font), 280.).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.settings_field = Some(SettingsField::Font);
                cx.stop_propagation();
                cx.notify();
            }),
        );
        let font_list = div()
            .id("font-list")
            .w(px(280.))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            // no topo ou no fim da lista a roda não passa para a página
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .rounded_md()
            .border_1()
            .border_color(rgb(c_card_border()))
            .bg(rgb(c_bg()))
            .children(shown.iter().enumerate().map(|(i, name)| {
                let on = *name == current_font;
                let pick = name.clone();
                div()
                    .id(("font", i))
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_sm()
                    .border_l_2()
                    .border_color(rgb(if on { c_accent() } else { c_bg() }))
                    .when(on, |d| d.bg(rgb(c_surface())))
                    .when(!on, |d| d.hover(|s| s.bg(rgb(c_hover()))))
                    .child(div().flex_1().min_w_0().truncate().font_family(SharedString::from(name.clone())).text_color(rgb(if on { c_text() } else { c_muted() })).child(name.clone()))
                    .child(div().flex_none().text_xs().font_family(SharedString::from(name.clone())).text_color(rgb(c_disabled())).child("Aa 0O"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.set_mono(&pick, window);
                        cx.notify();
                    }))
            }))
            .when(shown.is_empty(), |d| d.child(div().p_4().text_sm().text_color(rgb(c_muted())).child("Nenhuma fonte de largura fixa com esse nome.")));

        let size_now = self.font_size as u32;
        let size_slider = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().w(px(SLIDER_W)).flex().justify_between().child(div().text_xs().text_color(rgb(c_muted())).child("Tamanho")).child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(format!("{size_now} px"))))
            .child(size_slider(cx.entity(), self.font_size));

        let editing = self.settings_field == Some(SettingsField::Preview);
        let (cell_w, cell_h) = (self.cell_w, self.cell_h);
        let raw = self.preview_text.clone();
        let caret = self.preview_caret.min(raw.chars().count());
        // linha e coluna do cursor
        let (caret_line, caret_col) = caret_line_col(&raw, caret);
        let (scroll_x, scroll_y) = self.preview_scroll;
        let size_cells = self.preview_size.clone();
        let empty = raw.is_empty() && !editing;
        let body_lines = raw.split('\n').map(|l| {
            let color = if l.trim_start().starts_with("Finished") { GREEN } else if l.starts_with("   ") { c_muted() } else { c_term_fg() };
            let row = div().flex().h(px(cell_h)).whitespace_nowrap();
            if empty {
                return row.child(div().text_color(rgb(c_disabled())).child("Clique aqui e escreva um texto de teste"));
            }
            match l.strip_prefix('❯') {
                Some(rest) => row.child(div().text_color(rgb(c_coral())).child("❯")).child(div().text_color(rgb(c_term_fg())).child(rest.to_string())),
                None => row.child(div().text_color(rgb(color)).child(l.to_string())),
            }
        });
        // limites do corpo, medidos ao desenhar, para converter o clique em linha e coluna
        let body_bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
        let measure_bounds = body_bounds.clone();
        let preview = div()
            .min_w_0()
            .h(px(300.))
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .border_color(rgb(c_card_border()))
            .flex()
            .flex_col()
            // barra de título da janela de exemplo
            .child(
                div()
                    .h(px(30.))
                    .flex_none()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .bg(rgb(c_panel()))
                    .border_b_1()
                    .border_color(rgb(c_divider()))
                    .child(div().size(px(10.)).rounded_full().bg(rgb(0xff5f57)))
                    .child(div().size(px(10.)).rounded_full().bg(rgb(0xfebc2e)))
                    .child(div().size(px(10.)).rounded_full().bg(rgb(0x28c840)))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().flex().justify_center().text_xs().text_color(rgb(c_muted())).child(format!("brasa · {current_font} · {size_now} px"))),
            )
            .child(
                div()
                    .id("preview-body")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .cursor_text()
                    .bg(rgb(c_term_bg()))
                    .font_family(self.mono.clone())
                    .text_size(px(self.font_size))
                    .text_color(rgb(c_term_fg()))
                    .flex()
                    .flex_col()
                    .border_t_2()
                    .border_color(rgb(if editing { c_accent() } else { c_term_bg() }))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |b: Bounds<Pixels>, _, _, _| {
                                measure_bounds.set(b);
                                size_cells[0].store(f32::from(b.size.width).to_bits(), std::sync::atomic::Ordering::Relaxed);
                                size_cells[1].store(f32::from(b.size.height).to_bits(), std::sync::atomic::Ordering::Relaxed);
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(div().absolute().top(px(PAD_Y - scroll_y as f32 * cell_h)).left(px(PAD_X - scroll_x as f32 * cell_w)).flex().flex_col().children(body_lines))
                    .when(editing, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(PAD_X + (caret_col as f32 - scroll_x as f32) * cell_w))
                                .top(px(PAD_Y + (caret_line as f32 - scroll_y as f32) * cell_h + 1.0))
                                .w(px(2.))
                                .h(px((cell_h - 2.0).max(8.0)))
                                .bg(rgb(c_text()))
                                .opacity(0.9),
                        )
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let b = body_bounds.get();
                            let x = f32::from(ev.position.x - b.origin.x) - PAD_X;
                            let y = f32::from(ev.position.y - b.origin.y) - PAD_Y;
                            let (line, col) = ((y / cell_h).floor().max(0.0) as usize + this.preview_scroll.1, (x / cell_w).round().max(0.0) as usize + this.preview_scroll.0);
                            // índice do caractere na linha e coluna clicadas (limitado ao fim da linha)
                            let mut idx = 0usize;
                            for (n, l) in this.preview_text.split('\n').enumerate() {
                                let len = l.chars().count();
                                if n == line {
                                    idx += col.min(len);
                                    break;
                                }
                                idx += len + 1;
                                if idx > this.preview_text.chars().count() {
                                    idx = this.preview_text.chars().count();
                                }
                            }
                            this.preview_caret = idx.min(this.preview_text.chars().count());
                            this.follow_preview_caret();
                            this.settings_field = Some(SettingsField::Preview);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(rgb(c_panel()))
                    .border_t_1()
                    .border_color(rgb(c_divider()))
                    .child(div().text_xs().text_color(rgb(c_muted())).child(if editing { "Digitando… Esc para sair" } else { "Clique no texto para editar" }))
                    .child(chip("preview-reset", "Restaurar texto", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.preview_text = DEFAULT_PREVIEW.to_string();
                        this.preview_caret = this.preview_text.chars().count();
                        this.preview_scroll = (0, 0);
                        cx.notify();
                    }))),
            );

        let font_card = card(
            "Fonte do terminal",
            "Só aparecem fontes de largura fixa. A escolha vale também para o editor das notas.",
            div()
                .flex()
                .gap_6()
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(div().h(px(300.)).flex().flex_col().gap_3().child(font_search).child(font_list))
                        .child(size_slider),
                )
                .child(div().flex_1().min_w_0().overflow_hidden().flex().flex_col().child(preview)),
        );

        div().flex().flex_col().gap_5().child(theme_card).child(bg_card).child(font_card).into_any_element()
    }
}

const WHEEL: f32 = 200.0;
const RING_OUT: f32 = 98.0;
const RING_IN: f32 = 68.0;

/// Roda de matizes: anel colorido com um marcador; clicar ou arrastar escolhe o matiz.
fn color_wheel(entity: gpui::Entity<Workspace>, marker: Option<f32>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b: Bounds<Pixels>, _, window, _| {
            let (cx0, cy0) = (f32::from(b.origin.x) + f32::from(b.size.width) / 2.0, f32::from(b.origin.y) + f32::from(b.size.height) / 2.0);
            let at = |deg: f32, r: f32| {
                let a = deg.to_radians();
                point(px(cx0 + r * a.sin()), px(cy0 - r * a.cos()))
            };
            const SEGS: usize = 120;
            for i in 0..SEGS {
                let (a0, a1) = (i as f32 * 360.0 / SEGS as f32, (i as f32 + 1.4) * 360.0 / SEGS as f32);
                let mut pb = PathBuilder::fill();
                pb.move_to(at(a0, RING_IN));
                pb.line_to(at(a0, RING_OUT));
                pb.line_to(at(a1, RING_OUT));
                pb.line_to(at(a1, RING_IN));
                pb.close();
                if let Ok(path) = pb.build() {
                    window.paint_path(path, rgb(hsl_to_rgb(a0 + 180.0 / SEGS as f32, 0.72, 0.58)));
                }
            }
            if let Some(h) = marker {
                let p = at(h, (RING_IN + RING_OUT) / 2.0);
                let r = 13.0;
                window.paint_quad(quad(Bounds::new(point(p.x - px(r), p.y - px(r)), size(px(r * 2.0), px(r * 2.0))), px(r), rgb(hsl_to_rgb(h, 0.72, 0.58)), px(3.), rgb(0xffffff), Default::default()));
            }

            let hue_at = move |pos: gpui::Point<Pixels>| ((f32::from(pos.x) - cx0).atan2(-(f32::from(pos.y) - cy0)).to_degrees()).rem_euclid(360.0);
            let dist = move |pos: gpui::Point<Pixels>| ((f32::from(pos.x) - cx0).powi(2) + (f32::from(pos.y) - cy0).powi(2)).sqrt();
            window.on_mouse_event({
                let e = entity.clone();
                move |ev: &MouseDownEvent, phase, _, cx| {
                    let d = dist(ev.position);
                    if phase == DispatchPhase::Bubble && ev.button == MouseButton::Left && d >= RING_IN - 6.0 && d <= RING_OUT + 6.0 {
                        e.update(cx, |ws, cx| {
                            ws.wheel_drag = true;
                            ws.settings_field = None;
                            ws.pick_hue(hue_at(ev.position));
                            cx.notify();
                        });
                        cx.stop_propagation();
                    }
                }
            });
            window.on_mouse_event({
                let e = entity.clone();
                move |ev: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && ev.pressed_button == Some(MouseButton::Left) {
                        e.update(cx, |ws, cx| {
                            if ws.wheel_drag {
                                ws.pick_hue(hue_at(ev.position));
                                cx.notify();
                            }
                        });
                    }
                }
            });
            window.on_mouse_event({
                let e = entity.clone();
                move |_: &MouseUpEvent, _, _, cx| {
                    e.update(cx, |ws, _| ws.wheel_drag = false);
                }
            });
        },
    )
    .size(px(WHEEL))
}

/// Cartão de seção: título, explicação e conteúdo.
fn card(title: &'static str, hint: &'static str, body: impl IntoElement) -> Div {
    div()
        .w_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_5()
        .rounded_lg()
        .border_1()
        .border_color(rgb(c_card_border()))
        .bg(rgb(c_panel()))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_sm().text_color(rgb(c_muted())).child(hint)),
        )
        .child(body)
}

/// Rótulo pequeno acima de um controle.
fn field_group(label: &'static str, control: impl IntoElement) -> Div {
    div().min_w_0().flex().flex_col().gap_2().child(div().text_xs().text_color(rgb(c_muted())).child(label)).child(control)
}

/// Multiplica os canais de uma cor (para a barra lateral das miniaturas de fundo).
fn scale(c: u32, f: f32) -> u32 {
    let ch = |s: u32| ((((c >> s) & 0xff) as f32 * f).round().min(255.0)) as u32;
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

const SIZE_MIN: f32 = 9.0;
const SIZE_MAX: f32 = 32.0;
const SLIDER_W: f32 = 280.0;
const SLIDER_PAD: f32 = 12.0;
/// tamanhos mais usados, marcados e nomeados na régua
const COMMON_SIZES: [u32; 5] = [12, 14, 16, 18, 20];

fn slider_x(v: f32) -> f32 {
    SLIDER_PAD + (v - SIZE_MIN) / (SIZE_MAX - SIZE_MIN) * (SLIDER_W - 2.0 * SLIDER_PAD)
}

/// Régua do tamanho da fonte: do menor ao maior, com marcas nos tamanhos comuns; arraste o marcador.
fn size_slider(entity: gpui::Entity<Workspace>, value: f32) -> impl IntoElement {
    let labels = std::iter::once(SIZE_MIN as u32).chain(COMMON_SIZES).chain(std::iter::once(SIZE_MAX as u32)).map(|n| {
        let on = n == value as u32;
        div()
            .absolute()
            .left(px(slider_x(n as f32) - 14.0))
            .top(px(34.))
            .w(px(28.))
            .flex()
            .justify_center()
            .text_xs()
            .text_color(rgb(if on { c_accent() } else { c_muted() }))
            .child(format!("{n}"))
    });
    div()
        .relative()
        .w(px(SLIDER_W))
        .h(px(54.))
        .child(
            canvas(
                |_, _, _| {},
                move |b: Bounds<Pixels>, _, window, _| {
                    let (ox, oy) = (f32::from(b.origin.x), f32::from(b.origin.y));
                    let ty = oy + 16.0;
                    let rect = |x: f32, y: f32, w: f32, h: f32, r: f32, color: u32| {
                        quad(Bounds::new(point(px(x), px(y)), size(px(w), px(h))), px(r), rgb(color), px(0.), gpui::transparent_black(), Default::default())
                    };
                    // trilho e parte já percorrida
                    window.paint_quad(rect(ox + SLIDER_PAD, ty - 2.0, SLIDER_W - 2.0 * SLIDER_PAD, 4.0, 2.0, c_surface()));
                    window.paint_quad(rect(ox + SLIDER_PAD, ty - 2.0, slider_x(value) - SLIDER_PAD, 4.0, 2.0, c_accent()));
                    // marcas: uma pequena por tamanho, uma maior nos comuns e nas pontas
                    for s in SIZE_MIN as u32..=SIZE_MAX as u32 {
                        let big = COMMON_SIZES.contains(&s) || s == SIZE_MIN as u32 || s == SIZE_MAX as u32;
                        let h = if big { 12.0 } else { 6.0 };
                        window.paint_quad(rect(ox + slider_x(s as f32) - 0.5, ty + 6.0, 1.0, h, 0.0, if big { c_muted() } else { c_disabled() }));
                    }
                    // marcador
                    let x = ox + slider_x(value);
                    window.paint_quad(quad(
                        Bounds::new(point(px(x - 9.0), px(ty - 9.0)), size(px(18.), px(18.))),
                        px(9.),
                        rgb(c_text()),
                        px(3.),
                        rgb(c_accent()),
                        Default::default(),
                    ));

                    let value_at = move |pos: gpui::Point<Pixels>| {
                        let t = ((f32::from(pos.x) - ox - SLIDER_PAD) / (SLIDER_W - 2.0 * SLIDER_PAD)).clamp(0.0, 1.0);
                        (SIZE_MIN + t * (SIZE_MAX - SIZE_MIN)).round()
                    };
                    window.on_mouse_event({
                        let e = entity.clone();
                        move |ev: &MouseDownEvent, phase, window, cx| {
                            let (px_, py_) = (f32::from(ev.position.x), f32::from(ev.position.y));
                            if phase == DispatchPhase::Bubble && ev.button == MouseButton::Left && px_ >= ox && px_ <= ox + SLIDER_W && py_ >= oy && py_ <= oy + 34.0 {
                                let v = value_at(ev.position);
                                e.update(cx, |ws, cx| {
                                    ws.size_drag = true;
                                    ws.settings_field = None;
                                    if v != ws.font_size {
                                        ws.set_font_size(v, window);
                                    }
                                    cx.notify();
                                });
                                cx.stop_propagation();
                            }
                        }
                    });
                    window.on_mouse_event({
                        let e = entity.clone();
                        move |ev: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble && ev.pressed_button == Some(MouseButton::Left) {
                                let v = value_at(ev.position);
                                e.update(cx, |ws, cx| {
                                    if ws.size_drag && v != ws.font_size {
                                        ws.set_font_size(v, window);
                                        cx.notify();
                                    }
                                });
                            }
                        }
                    });
                    window.on_mouse_event({
                        let e = entity.clone();
                        move |_: &MouseUpEvent, _, _, cx| {
                            e.update(cx, |ws, _| ws.size_drag = false);
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        )
        .children(labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_line_and_column() {
        assert_eq!(caret_line_col("abc\ndef", 0), (0, 0));
        assert_eq!(caret_line_col("abc\ndef", 3), (0, 3));
        assert_eq!(caret_line_col("abc\ndef", 4), (1, 0));
        assert_eq!(caret_line_col("abc\ndef", 7), (1, 3));
        assert_eq!(caret_line_col("❯ ok\n", 5), (1, 0));
        assert_eq!(caret_line_col("", 0), (0, 0));
    }

    #[test]
    fn latency_summary_is_computed() {
        assert_eq!(latency_stats(std::iter::empty()), None);
        let (avg, p95, max, n) = latency_stats([4.0, 8.0, 6.0, 2.0].into_iter()).unwrap();
        assert_eq!((avg, max, n), (5.0, 8.0, 4));
        assert_eq!(p95, 8.0);
        let many: Vec<f32> = (1..=100).map(|x| x as f32).collect();
        assert_eq!(latency_stats(many.into_iter()).unwrap().1, 95.0);
    }

    #[test]
    fn slider_marks_cover_the_whole_track() {
        assert_eq!(slider_x(SIZE_MIN), SLIDER_PAD);
        assert_eq!(slider_x(SIZE_MAX), SLIDER_W - SLIDER_PAD);
        assert!(COMMON_SIZES.windows(2).all(|w| slider_x(w[0] as f32) < slider_x(w[1] as f32)));
        assert!(COMMON_SIZES.iter().all(|s| (*s as f32) > SIZE_MIN && (*s as f32) < SIZE_MAX));
    }

    #[test]
    fn saved_font_settings_are_read_back() {
        assert_eq!(parse("fonte=JetBrains Mono\ntamanho=17\n"), (Some("JetBrains Mono".to_string()), Some(17.0)));
        assert_eq!(parse("tamanho=abc\n"), (None, None));
        assert_eq!(parse(""), (None, None));
    }
}
