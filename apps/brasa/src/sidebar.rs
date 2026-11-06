//! Barra lateral, do topo da janela até o rodapé. Um seletor alterna entre Sessões (terminais, em
//! pastas) e Notas, cada seção com a sua cor; embaixo, filtro e perfil. Os arquivos ficam no painel
//! Arquivos, aberto pela barra de widgets à direita.

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, ClickEvent, Context, Div, FontWeight, MouseButton, MouseDownEvent, Stateful, WindowControlArea};

use crate::icons::{chevron_el, folder_icon, icon_el, note_icon, wicon};
use crate::note_editor::NoteMode;
use crate::scrollbar::scroll_list;
use crate::theme::*;
use crate::{DraggedNote, DraggedSession, Side, Workspace, TITLE_H};

const ROW_H: f32 = 30.0;
const INDENT: f32 = 16.0;

/// Linha de lista com realce de seleção/hover (só um `hover` por elemento: escolhe um dos dois).
fn list_row(id: impl Into<gpui::ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(ROW_H))
        .flex()
        .items_center()
        .gap_2()
        .pr_2()
        .rounded_md()
        .cursor_pointer()
        .text_sm()
        .when(selected, |d| d.bg(rgb(c_surface())))
        .when(!selected, |d| d.hover(|s| s.bg(rgb(c_hover()))))
}

/// Botão pequeno de ícone (ou glifo) usado nos cabeçalhos de seção e nos controles da árvore.
fn tiny_btn(id: impl Into<gpui::ElementId>, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .min_w(px(22.))
        .h(px(22.))
        .px_1()
        .rounded_sm()
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(rgb(c_muted()))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(c_btn_hover())).text_color(rgb(c_text())))
        .child(label)
}

/// Botão com contorno, ícone e texto ("Nova Nota", "Nova Pasta").
fn outline_btn(id: &'static str, icon: &'static str, label: &'static str, color: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded_md()
        .border_1()
        .border_color(rgb(c_card_border()))
        .text_size(px(13.))
        .text_color(rgb(c_text()))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(c_hover())).border_color(rgb(color)))
        .child(wicon(icon, 13., color))
        .child(label)
}

/// Ícone clicável dentro de uma linha (renomear, lixeira).
fn row_icon_btn(id: impl Into<gpui::ElementId>, icon: &'static str, color: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(24.))
        .rounded_sm()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(c_btn_hover())))
        .child(wicon(icon, 14., color))
}

/// Campo de nome (nova nota/pasta ou renomear): texto com cursor, ou a dica quando vazio.
fn name_input(text: &str, hint: &str) -> Div {
    div()
        .min_w_0()
        .px_2()
        .py(px(3.))
        .rounded_md()
        .bg(rgb(c_bg()))
        .border_1()
        .border_color(rgb(c_accent()))
        .text_sm()
        .flex()
        // o filho ocupa a largura do campo; com `truncate` sem largura ele encolhia até virar "..."
        .child(if text.is_empty() {
            div().flex_1().min_w_0().truncate().text_color(rgb(c_muted())).child(hint.to_string())
        } else {
            div().flex_1().min_w_0().truncate().child(format!("{text}▏"))
        })
}

/// Rótulo, ícone, cor e fundo de cada seção da barra lateral.
fn side_style(side: Side) -> (&'static str, &'static str, u32, u32) {
    match side {
        Side::Terminals => ("Sessões", "terminal", c_coral(), c_coral_dim()),
        Side::Notes => ("Notas", "notebook", c_amber(), c_amber_dim()),
    }
}

impl Workspace {
    /// Força da brasa do logo e do fio sob o cabeçalho: brasa baixa parada; com comando rodando, respira
    /// devagar (uma onda suave de ~2,4 s, desenhada a cada quadro enquanto algo roda).
    pub(crate) fn ember_glow(&self) -> f32 {
        if self.tabs.iter().any(|t| !t.cmd_starts.is_empty()) {
            static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f32();
            let wave = 0.5 - 0.5 * (t * std::f32::consts::TAU / 2.4).cos();
            0.35 + 0.65 * wave
        } else {
            0.35
        }
    }

    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.side_filter.trim().to_lowercase();
        let mut body: Vec<AnyElement> = Vec::new();
        match self.side {
            Side::Terminals => self.push_terminals(&mut body, &filter, cx),
            Side::Notes => self.push_notes(&mut body, &filter, cx),
        }

        div()
            .w(px(self.sidebar_px))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(rgba(c_panel_glass()))
            .rounded_l(px(self.corner_r))
            .border_r_1()
            .border_color(rgb(c_divider()))
            // topo: arrasta a janela, como a barra de título
            .child(
                div()
                    .h(px(TITLE_H))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl_4()
                    .pr_2()
                    .border_b(px(2.))
                    .border_color(rgb(c_divider()))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(c_muted()))
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
                            )
                            // logo da Brasa (a chama com o prompt e o nome, branding/brasa-logo-dark.png); a brasa tremeluz enquanto um comando roda
                            .child(
                                gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded(logo_path().into())))
                                    .h(px(30.))
                                    .w(px(92.))
                                    .object_fit(gpui::ObjectFit::Contain)
                                    .opacity(0.72 + 0.28 * self.ember_glow()),
                            ),
                    )
                    .child(
                        div()
                            .id("toggle-sidebar")
                            .size(px(28.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(c_hover())))
                            .child(wicon("layout-left", 15., c_muted()))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.show_sidebar = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.render_side_switch(cx))
            // soltar em qualquer lugar da lista que não seja uma pasta tira o item da pasta (volta para a raiz);
            // as pastas e as linhas dentro delas pegam o drop primeiro, então só o que sobra chega aqui
            .child(scroll_list(
                div()
                    .id("side-list")
                    .px_2()
                    .pb_4()
                    .flex()
                    .flex_col()
                    .children(body)
                    .on_drop(cx.listener(|this, item: &DraggedSession, _, cx| {
                        this.move_session(item, None);
                        cx.notify();
                    }))
                    .on_drop(cx.listener(|this, item: &DraggedNote, _, cx| {
                        this.move_note(item, "");
                        cx.notify();
                    })),
                &self.side_scroll,
            ))
            .child(self.render_side_footer(cx))
            .into_any_element()
    }

    /// Seletor de seção: três botões lado a lado; o ativo ganha a cor da sua seção.
    fn render_side_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.side;
        let (_, _, _, dim) = side_style(current);
        div()
            .flex_none()
            .m_2()
            .p(px(3.))
            .flex()
            .gap(px(2.))
            .rounded_lg()
            .bg(rgb(c_bg()))
            .border_1()
            .border_color(rgb(dim))
            .children([Side::Terminals, Side::Notes].into_iter().map(|side| {
                let (label, icon, color, dim) = side_style(side);
                let active = side == current;
                div()
                    .id(label)
                    .flex_1()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w_0()
                    .gap(px(5.))
                    .rounded_md()
                    .text_size(px(13.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(rgb(dim)).text_color(rgb(color)).font_weight(FontWeight::MEDIUM))
                    .when(!active, |d| d.text_color(rgb(c_muted())).hover(|s| s.bg(rgb(c_hover())).text_color(rgb(c_text()))))
                    .child(wicon(icon, 13., if active { color } else { c_muted() }))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if this.side != side {
                            this.side = side;
                            this.side_filter.clear();
                            this.naming = None;
                        }
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }

    // ----- terminais -----

    fn push_terminals(&mut self, body: &mut Vec<AnyElement>, filter: &str, cx: &mut Context<Self>) {
        body.push(
            div()
                .flex_none()
                .flex()
                .gap_2()
                .px_1()
                .pt_1()
                .child(outline_btn("term-new", "terminal", "Novo Terminal", c_coral()).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    let cwd = this.shell_cwd.clone();
                    this.new_tab(Some(cwd));
                    cx.notify();
                })))
                .child(outline_btn("term-add-folder", "folder-closed", "Nova Pasta", c_coral()).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.start_session_folder();
                    cx.notify();
                })))
                .into_any_element(),
        );
        // "Todas as sessões": soltar aqui tira o terminal da pasta
        body.push(
            div()
                .id("terms-root")
                .flex_none()
                .h(px(28.))
                .mt_2()
                .px_2()
                .flex()
                .items_center()
                .rounded_md()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(c_muted()))
                .drag_over::<DraggedSession>(|s, _, _, _| s.bg(rgb(c_coral_dim())).text_color(rgb(c_text())))
                .on_drop(cx.listener(|this, item: &DraggedSession, _, cx| {
                    this.move_session(item, None);
                    cx.notify();
                }))
                .child("Todas as sessões")
                .into_any_element(),
        );
        if let Some(n) = self.naming.as_ref().filter(|n| n.sessions && n.rename.is_none() && n.terminal.is_none()) {
            body.push(name_input(&n.text, "Nome da pasta").mx_1().my_1().into_any_element());
        }

        let label_of = |i: usize, _t: &crate::Tab| self.terminal_label(i);
        if !filter.is_empty() {
            // busca: lista plana, por nome, comando ou pasta e também pelo que apareceu na saída do terminal
            let found: Vec<(usize, usize)> = (0..self.tabs.len())
                .filter_map(|i| {
                    let t = &self.tabs[i];
                    let in_folder = t.folder.and_then(|id| self.session_folders.iter().find(|f| f.id == id)).map(|f| f.name.to_lowercase()).unwrap_or_default();
                    let by_name = label_of(i, t).to_lowercase().contains(filter) || self.session_label(i).0.to_lowercase().contains(filter) || in_folder.contains(filter);
                    let in_output = t.term.search(filter).len();
                    (by_name || in_output > 0).then_some((i, in_output))
                })
                .collect();
            if found.is_empty() {
                body.push(div().px_2().py_1().text_xs().text_color(rgb(c_muted())).child("Nenhuma sessão combina.").into_any_element());
            }
            for (i, in_output) in found {
                let label = label_of(i, &self.tabs[i]);
                body.push(self.session_row(i, label, 0, cx));
                if in_output > 0 {
                    let text = if in_output == 1 { "1 ocorrência na saída".to_string() } else { format!("{in_output} ocorrências na saída") };
                    body.push(div().flex_none().pl(px(38.)).pb_1().text_xs().text_color(rgb(c_disabled())).child(text).into_any_element());
                }
            }
            return;
        }

        // pastas primeiro, cada uma com os seus terminais
        let font = self.icon_font.clone();
        let folders: Vec<(u64, String, bool)> = self.session_folders.iter().map(|f| (f.id, f.name.clone(), f.open)).collect();
        for (k, (id, name, open)) in folders.into_iter().enumerate() {
            let inside: Vec<usize> = (0..self.tabs.len()).filter(|&i| self.tabs[i].folder == Some(id)).collect();
            if self.confirm_trash.as_deref() == Some(format!("sf:{id}").as_str()) {
                body.push(self.confirm_card(
                    ("sf-confirm", k),
                    format!("Remover a pasta \"{name}\"? Os terminais dela continuam abertos, de volta à lista."),
                    "Remover",
                    8.,
                    move |this, _| this.remove_session_folder(id),
                    cx,
                ));
                continue;
            }
            let selected = self.session_folder_sel == Some(id);
            let renaming = self.naming.as_ref().filter(|n| n.sessions && n.rename.as_deref() == Some(id.to_string().as_str())).map(|n| n.text.clone());
            body.push(
                list_row(("sf", k), false)
                    .pl_2()
                    .child(chevron_el(&font, open))
                    .child(icon_el(&font, &folder_icon(open), 14.))
                    .child(match renaming {
                        Some(text) => name_input(&text, "Nome da pasta").flex_1().into_any_element(),
                        None => div().flex_1().min_w_0().truncate().when(selected, |d| d.text_color(rgb(c_coral()))).child(name).into_any_element(),
                    })
                    .when(!open && !inside.is_empty(), |d| d.child(div().text_xs().text_color(rgb(c_disabled())).child(inside.len().to_string())))
                    .when(selected && self.naming.is_none(), |d| {
                        d.child(row_icon_btn(("sf-ren", k), "pencil-02", c_muted()).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.start_rename_session_folder(id);
                            cx.notify();
                        })))
                        .child(row_icon_btn(("sf-x", k), "trash-02", c_muted()).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.confirm_trash = Some(format!("sf:{id}"));
                            cx.notify();
                        })))
                    })
                    .drag_over::<DraggedSession>(|s, _, _, _| s.bg(rgb(c_coral_dim())))
                    .on_drop(cx.listener(move |this, item: &DraggedSession, _, cx| {
                        this.move_session(item, Some(id));
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.confirm_trash = None;
                        if ev.click_count() >= 2 {
                            this.start_rename_session_folder(id);
                        } else if let Some(f) = this.session_folders.iter_mut().find(|f| f.id == id) {
                            f.open = !f.open;
                            this.session_folder_sel = Some(id);
                            this.save_session_folders();
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            if open {
                if inside.is_empty() {
                    body.push(div().pl(px(8. + INDENT + 22.)).py_1().text_xs().text_color(rgb(c_disabled())).child("Arraste um terminal para cá").into_any_element());
                }
                for i in inside {
                    let label = label_of(i, &self.tabs[i]);
                    body.push(self.session_row(i, label, 1, cx));
                }
            }
        }
        // terminais soltos, fora de pastas
        for i in (0..self.tabs.len()).filter(|&i| self.tabs[i].folder.is_none()) {
            let label = label_of(i, &self.tabs[i]);
            body.push(self.session_row(i, label, 0, cx));
        }
    }

    /// Um terminal na lista: comando atual, pasta do shell à direita; arrastável para uma pasta.
    fn session_row(&self, i: usize, label: String, depth: usize, cx: &mut Context<Self>) -> AnyElement {
        let active = i == self.active;
        let running = !self.tabs[i].cmd_starts.is_empty();
        let cwd = self.session_label(i).0;
        let sid = self.tabs[i].session;
        let dragged = DraggedSession { session: sid, label: label.clone() };
        let folder = self.tabs[i].folder;
        let renaming = self.naming.as_ref().filter(|n| n.terminal == Some(sid)).map(|n| n.text.clone());
        list_row(("term", i), active)
            .pl(px(8. + INDENT * depth as f32 + if depth > 0 { 6. } else { 0. }))
            .child(wicon("terminal", 13., if running { c_coral() } else if active { c_text() } else { c_disabled() }))
            .child(match renaming {
                Some(text) => name_input(&text, "Nome do terminal").flex_1().into_any_element(),
                None => div().flex_1().min_w_0().truncate().when(!active, |d| d.text_color(rgb(c_muted()))).child(label).into_any_element(),
            })
            .child(div().flex_none().max_w(px(90.)).truncate().text_xs().text_color(rgb(c_disabled())).child(cwd))
            .when(running, |d| d.child(div().flex_none().text_xs().text_color(rgb(c_coral())).child("✳")))
            .when(active && self.naming.is_none(), |d| {
                d.child(row_icon_btn(("term-ren", i), "pencil-02", c_muted()).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.start_rename_terminal(i);
                    cx.notify();
                })))
            })
            .when(active && !running, |d| {
                d.child(
                    tiny_btn(("term-x", i), "✕").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.close_tab(i);
                        cx.notify();
                    })),
                )
            })
            .on_drag(dragged, |item, _, _, cx| item.preview(cx))
            // dentro de uma pasta: soltar outro terminal em cima dele o põe na mesma pasta
            .when(folder.is_some(), |d| {
                d.drag_over::<DraggedSession>(|s, _, _, _| s.bg(rgb(c_coral_dim()))).on_drop(cx.listener(move |this, item: &DraggedSession, _, cx| {
                    this.move_session(item, folder);
                    cx.notify();
                }))
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                this.switch_tab(i);
                // duplo clique renomeia o terminal
                if ev.click_count() >= 2 {
                    this.start_rename_terminal(i);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    /// Confirmação na própria lista (mover para a lixeira, remover pasta): mensagem + botões.
    fn confirm_card(
        &self,
        id: (&'static str, usize),
        message: String,
        yes: &'static str,
        pad: f32,
        on_yes: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .flex_none()
            .my_1()
            .ml(px(pad - 4.))
            .p_2()
            .rounded_md()
            .bg(rgb(ERROR_BG))
            .border_1()
            .border_color(rgb(0x5a3236))
            .flex()
            .flex_col()
            .gap_2()
            .text_sm()
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
            .child(div().child(message))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id((id.0, id.1 * 2))
                            .px_3()
                            .py(px(4.))
                            .rounded_md()
                            .bg(rgb(0x8a3b3b))
                            .text_color(rgb(c_text()))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0xa34646)))
                            .child(yes)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                on_yes(this, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id((id.0, id.1 * 2 + 1))
                            .px_3()
                            .py(px(4.))
                            .rounded_md()
                            .text_color(rgb(c_muted()))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(c_hover())).text_color(rgb(c_text())))
                            .child("Cancelar")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.confirm_trash = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    // ----- notas -----

    fn push_notes(&mut self, body: &mut Vec<AnyElement>, filter: &str, cx: &mut Context<Self>) {
        // ações em botões de verdade, lado a lado
        body.push(
            div()
                .flex_none()
                .flex()
                .gap_2()
                .px_1()
                .pt_1()
                .child(outline_btn("note-add", "notebook", "Nova Nota", c_amber()).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.start_naming(false);
                    cx.notify();
                })))
                .child(outline_btn("note-add-folder", "folder-closed", "Nova Pasta", c_amber()).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.start_naming(true);
                    cx.notify();
                })))
                .into_any_element(),
        );
        // "Todas as notas": também é onde se solta uma nota para tirá-la de uma pasta
        let at_root = self.note_dir.is_empty();
        body.push(
            div()
                .id("notes-root")
                .flex_none()
                .h(px(28.))
                .mt_2()
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if at_root { c_muted() } else { c_accent() }))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(c_hover())))
                .drag_over::<DraggedNote>(|s, _, _, _| s.bg(rgb(c_amber_dim())).text_color(rgb(c_text())))
                .on_drop(cx.listener(|this, item: &DraggedNote, _, cx| {
                    this.move_note(item, "");
                    cx.notify();
                }))
                .child(div().flex_1().min_w_0().truncate().child(if at_root { "Todas as notas".to_string() } else { format!("Criando em {}  ·  voltar para todas", self.note_dir) }))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.note_dir.clear();
                    cx.notify();
                }))
                .into_any_element(),
        );
        if let Some(msg) = self.side_notice.clone() {
            body.push(div().flex_none().px_2().py_1().text_xs().text_color(rgb(RED)).child(msg).into_any_element());
        }
        if let Some(n) = self.naming.as_ref().filter(|n| !n.sessions && n.rename.is_none()) {
            let in_dir = if self.note_dir.is_empty() { String::new() } else { format!(" em {}", self.note_dir) };
            let hint = if n.folder { format!("Nome da pasta{in_dir}") } else { format!("Nome da nota{in_dir}") };
            body.push(name_input(&n.text, &hint).mx_1().my_1().into_any_element());
        }

        if !filter.is_empty() {
            // busca: lista plana
            let label_of = |rel: &str| rel.rsplit('/').next().unwrap_or(rel).trim_end_matches(".md").to_string();
            // pelo nome e também pelo que está escrito dentro (com a primeira linha encontrada)
            let found = self.notes.search(filter).unwrap_or_default();
            if found.is_empty() {
                body.push(div().px_2().py_1().text_xs().text_color(rgb(c_muted())).child("Nenhuma nota combina.").into_any_element());
            }
            for (i, n) in found.into_iter().enumerate() {
                let label = label_of(&n.name);
                body.push(self.note_row(i, n.name, label, 0, None, cx));
                if !n.snippet.is_empty() {
                    body.push(div().flex_none().pl(px(38.)).pr_2().pb_1().text_xs().truncate().text_color(rgb(c_disabled())).child(n.snippet).into_any_element());
                }
            }
            return;
        }
        let nodes = self.notes.tree(&self.note_expanded).unwrap_or_default();
        if nodes.is_empty() {
            body.push(div().px_2().py_1().text_xs().text_color(rgb(c_muted())).child("Sem notas ainda. Crie a primeira em \"Nova Nota\".").into_any_element());
        }
        for (i, n) in nodes.into_iter().enumerate() {
            body.push(self.note_row(i, n.rel, n.label, n.depth, n.is_dir.then_some(n.expanded), cx));
        }
    }

    /// Uma nota ou pasta de notas; `dir` = Some(aberta) para pastas. Arrasta-se para dentro de pastas;
    /// pastas recebem itens soltos. A lixeira pede confirmação na própria linha.
    fn note_row(&self, i: usize, rel: String, label: String, depth: usize, dir: Option<bool>, cx: &mut Context<Self>) -> AnyElement {
        let font = self.icon_font.clone();
        let is_dir = dir.is_some();
        let is_current = !is_dir && self.note_current.as_deref() == Some(rel.as_str());
        let is_dir_sel = is_dir && self.note_dir == rel;
        let ic = note_icon(is_dir, dir.unwrap_or(false));
        let pad = 8. + INDENT * depth as f32;

        // confirmação de exclusão no lugar da linha
        if self.confirm_trash.as_deref() == Some(rel.as_str()) {
            let what = if is_dir { "a pasta e o que tem dentro" } else { "a nota" };
            let rel_del = rel.clone();
            return self.confirm_card(
                ("note-confirm", i),
                format!("Mover {what} \"{label}\" para a lixeira?"),
                "Mover",
                pad,
                move |this, _| this.trash_note(&rel_del, is_dir),
                cx,
            );
        }

        // renomeando esta pasta: campo no lugar do nome
        let renaming = self.naming.as_ref().filter(|n| !n.sessions && n.rename.as_deref() == Some(rel.as_str())).map(|n| n.text.clone());
        let dragged = DraggedNote { rel: rel.clone(), is_dir, label: label.clone() };
        // pasta onde a linha está (vazio = raiz): onde cai uma nota solta em cima de outra nota
        let parent_dir = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        let (rel_open, rel_trash, rel_ren, rel_drop) = (rel.clone(), rel.clone(), rel.clone(), rel);
        list_row(("note-row", i), is_current)
            .pl(px(pad))
            .child(match dir {
                Some(open) => chevron_el(&font, open),
                None => div().flex_none().w(px(14.)),
            })
            .child(icon_el(&font, &ic, 14.))
            .child(match renaming {
                Some(text) => name_input(&text, "Nome da pasta").flex_1().into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(is_dir_sel, |d| d.text_color(rgb(c_accent())))
                    .when(!is_current && !is_dir_sel, |d| d.text_color(rgb(c_muted())))
                    .child(label)
                    .into_any_element(),
            })
            .when(is_dir_sel && self.naming.is_none(), |d| {
                d.child(row_icon_btn(("note-ren", i), "pencil-02", c_muted()).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.start_rename_folder(&rel_ren);
                    cx.notify();
                })))
            })
            .when(is_current || is_dir_sel, |d| {
                d.child(row_icon_btn(("note-x", i), "trash-02", c_muted()).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.confirm_trash = Some(rel_trash.clone());
                    cx.notify();
                })))
            })
            .on_drag(dragged, |item, _, _, cx| item.preview(cx))
            .when(is_dir, |d| {
                d.drag_over::<DraggedNote>(|s, _, _, _| s.bg(rgb(c_amber_dim()))).on_drop(cx.listener(move |this, item: &DraggedNote, _, cx| {
                    this.move_note(item, &rel_drop);
                    cx.notify();
                }))
            })
            .when(!is_dir && !parent_dir.is_empty(), |d| {
                d.drag_over::<DraggedNote>(|s, _, _, _| s.bg(rgb(c_amber_dim()))).on_drop(cx.listener(move |this, item: &DraggedNote, _, cx| {
                    this.move_note(item, &parent_dir);
                    cx.notify();
                }))
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                this.confirm_trash = None;
                this.side_notice = None;
                if is_dir {
                    if ev.click_count() >= 2 {
                        this.start_rename_folder(&rel_open);
                    } else {
                        if !this.note_expanded.remove(&rel_open) {
                            this.note_expanded.insert(rel_open.clone());
                        }
                        // clicar de novo na pasta selecionada volta a criar em "todas as notas"
                        this.note_dir = if this.note_dir == rel_open { String::new() } else { rel_open.clone() };
                    }
                } else {
                    this.open_note(&rel_open);
                    this.show_notes = true;
                    this.set_note_mode(NoteMode::Edit);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    // ----- rodapé -----

    fn render_side_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.filter_focused;
        let filter_box = div()
            .id("side-filter")
            .h(px(ROW_H))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded_md()
            .text_sm()
            .when(focused, |d| d.bg(rgb(c_bg())))
            .when(!focused, |d| d.hover(|s| s.bg(rgb(c_hover()))))
            .child(wicon("filter-funnel", 14., if focused { c_accent() } else { c_muted() }))
            .child(if self.side_filter.is_empty() {
                div().flex_1().text_color(rgb(c_muted())).child(if focused {
                    "Digite para filtrar…"
                } else if self.side == Side::Notes {
                    "Buscar em notas"
                } else {
                    "Buscar em sessões"
                })
            } else {
                div().flex_1().min_w_0().truncate().child(format!("{}{}", self.side_filter, if focused { "▏" } else { "" }))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.filter_focused = true;
                    this.focus_editor = false;
                    cx.stop_propagation(); // o contêiner raiz tiraria o foco do campo
                    cx.notify();
                }),
            );
        let user = std::env::var("USER").unwrap_or_else(|_| "usuário".into());
        let initial = user.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into());
        let shell = self.pty.shell_name(self.tabs[self.active].session).unwrap_or_else(|| "shell".into());
        let profile = div()
            .h(px(36.))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .child(
                div()
                    .flex_none()
                    .size(px(24.))
                    .rounded_full()
                    .bg(rgb(c_surface()))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(initial),
            )
            .child(div().flex_1().min_w_0().text_sm().truncate().child(user))
            .child(div().flex_none().text_xs().text_color(rgb(c_disabled())).child(shell));

        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_t_1()
            .border_color(rgb(c_divider()))
            .child(filter_box)
            .child(profile)
            .into_any_element()
    }
}
