//! Terminal em blocos no estilo do Warp: cada comando é uma faixa de largura total (sem cartão),
//! separada por uma linha de contorno, com a linha de contexto (pasta, branch, duração), o comando
//! em negrito e a saída. Erro = faixa vermelha; selecionado = faixa no acento. Ao passar o mouse
//! aparecem as ferramentas do bloco; o clique direito abre o menu com as opções do Warp.
//! Programas de tela cheia (vim, htop) e `BRASA_PROMPT=native` usam a grade crua de `term_view.rs`.

use std::collections::HashMap;

use gpui::{
    div, prelude::*, px, rgb, AnyElement, ClickEvent, ClipboardItem, Context, Div, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ScrollWheelEvent, Stateful,
};

use crate::blocks::{self, BlockBox, Metrics};
use crate::icons::wicon;
use crate::scrollbar::{thumb, thumb_geometry};
use crate::terminal::{BlockInfo, BlockModel};
use crate::theme::*;
use crate::{shorten, BlockLayout, Workspace};

/// recuo horizontal do conteúdo do bloco (como no Warp)
pub const BLOCK_PAD_X: f32 = 20.0;
const PAD_TOP: f32 = 14.0;
const CONTEXT_H: f32 = 20.0;
const BODY_GAP: f32 = 2.0;
const PAD_B: f32 = 10.0;
/// altura da barra de entrada (chips + linha digitada + dicas)
pub const INPUT_H: f32 = 108.0;
const PLACEHOLDER: &str = "Digite um comando, ex.: git status";

/// Menu de contexto de um bloco aberto em (x, y) da janela.
pub struct BlockMenu {
    pub id: i64,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy)]
enum BlockAction {
    Copy,
    CopyCommand,
    CopyOutput,
    CopyBoth,
    CopyDir,
    CopyBranch,
    CreateNote,
    Rerun,
    ToggleCollapse,
    ToggleBookmark,
    Close,
}

/// Junta as dicas com " · " enquanto couberem em `max_chars` caracteres (as menos importantes ficam por último).
pub fn fit_hints(items: &[&str], max_chars: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for it in items {
        let add = it.chars().count() + if out.is_empty() { 0 } else { 3 };
        if used + add > max_chars {
            break;
        }
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(it);
        used += add;
    }
    out
}

/// Duração no formato do Warp: "(0.052s)", "(12.3s)", "(2m 05s)".
pub fn format_duration(secs: f32) -> String {
    if secs < 10.0 {
        format!("({secs:.3}s)")
    } else if secs < 60.0 {
        format!("({secs:.1}s)")
    } else {
        let s = secs.round() as u64;
        format!("({}m {:02}s)", s / 60, s % 60)
    }
}

fn tool_btn(id: (&'static str, usize), icon: &str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(24.))
        .rounded(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(c_btn_hover())))
        .child(wicon(icon, 14., c_muted()))
}

impl Workspace {
    pub fn block_metrics(&self) -> Metrics {
        let header = PAD_TOP + CONTEXT_H + self.cell_h + 6.0;
        Metrics { cell_h: self.cell_h, header_h: header, cmd_h: BODY_GAP, pad_b: PAD_B, gap: 0.0 }
    }

    /// Desenha o modo em blocos; `None` quando deve valer a grade crua (tela cheia, prompt nativo, sem prompts ainda).
    pub fn render_block_area(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = self.prompt_text.clone()?;
        let model = {
            let tab = &self.tabs[self.active];
            if tab.term.is_alt_screen() {
                self.tabs[self.active].layout = None;
                return None;
            }
            tab.term.blocks(&prompt)
        };
        if model.blocks.is_empty() {
            self.tabs[self.active].layout = None;
            return None;
        }

        // ---- quais blocos aparecem e quantas linhas de saída cada um tem ----
        let m = self.block_metrics();
        let view_h = (self.tabs[self.active].rect.h - INPUT_H).max(60.0);
        let mut shown: Vec<(usize, usize)> = Vec::new(); // (índice no modelo, linhas visíveis)
        {
            let tab = &self.tabs[self.active];
            let last = model.blocks.len() - 1;
            for (i, b) in model.blocks.iter().enumerate() {
                if (model.idle && i == last) || tab.hidden.contains(&b.id) {
                    continue; // o prompt ativo vive na barra de entrada
                }
                let end = tab.term.last_content_line(b.out_start, b.out_end);
                let n = (end + 1 - b.out_start).max(0) as usize;
                if b.command.is_empty() && n == 0 && !b.running {
                    continue; // Enter em linha vazia
                }
                shown.push((i, if tab.collapsed.contains(&b.id) { 0 } else { n }));
            }
        }
        let counts: Vec<usize> = shown.iter().map(|(_, n)| *n).collect();
        let (boxes, total) = blocks::layout(&counts, &m);
        let max = blocks::max_scroll(total, view_h);
        let scroll = {
            let tab = &mut self.tabs[self.active];
            tab.scroll_y = if tab.follow { max } else { tab.scroll_y.clamp(0.0, max) };
            tab.layout = Some(BlockLayout {
                boxes: boxes.clone(),
                out_starts: shown.iter().map(|(i, _)| model.blocks[*i].out_start).collect(),
                total,
                view_h,
            });
            tab.scroll_y
        };

        // ---- destaque de busca por linha da grade ----
        let mut hits: HashMap<i32, Vec<(usize, usize, bool)>> = HashMap::new();
        if let Some(s) = self.search.as_ref().filter(|_| self.focused_render) {
            for (i, mt) in s.matches.iter().enumerate() {
                hits.entry(mt.line).or_default().push((mt.col, mt.col + mt.len, i == s.current));
            }
        }

        // ---- blocos visíveis ----
        let mono = self.mono.clone();
        let cards: Vec<AnyElement> = {
            let tab = &self.tabs[self.active];
            let sel = tab.term.selection_range();
            blocks::visible_blocks(&boxes, scroll, view_h)
                .map(|idx| {
                    let (mi, _) = shown[idx];
                    let b = &model.blocks[mi];
                    let bx = boxes[idx];
                    let first = blocks::visible_rows(&bx, scroll, view_h, &m);
                    let row_els: Vec<AnyElement> = first
                        .clone()
                        .map(|k| {
                            let line = b.out_start + k as i32;
                            let runs = tab.term.line_runs(line, sel, hits.get(&line).map_or(&[][..], |v| &v[..]));
                            div()
                                .flex()
                                .flex_row()
                                .h(px(self.cell_h))
                                .flex_none()
                                .whitespace_nowrap()
                                .children(runs.into_iter().map(|r| {
                                    div()
                                        .flex_none()
                                        .text_color(rgb(r.fg))
                                        .when(r.bg != c_term_bg(), |d| d.bg(rgb(r.bg)))
                                        .when(r.bold, |d| d.font_weight(FontWeight::BOLD))
                                        .when(r.underline, |d| d.underline())
                                        .child(r.text)
                                }))
                                .into_any_element()
                        })
                        .collect();
                    self.block_row(b, bx, first.start, row_els, &mono, scroll, cx)
                })
                .collect()
        };

        let to_bottom = !self.tabs[self.active].follow;
        let list = div()
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| this.on_scroll(ev, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.on_term_down(ev, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.on_term_down(ev, cx)))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.on_term_move(ev, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _, cx| this.on_term_up(ev, cx)))
            .children(cards)
            .when_some(thumb_geometry(view_h, max, scroll), |d, (top, len)| d.child(thumb(top, len)))
            .when(to_bottom, |d| {
                d.child(
                    div()
                        .id("to-bottom")
                        .absolute()
                        .bottom(px(12.))
                        .right(px(18.))
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .py_1()
                        .rounded(px(4.))
                        .text_xs()
                        .bg(rgb(c_surface()))
                        .border_1()
                        .border_color(rgb(c_btn_hover()))
                        .cursor_pointer()
                        .child(wicon("chevron-down", 12., c_text()))
                        .child("Ir ao fim")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.tab().follow = true;
                            cx.notify();
                        })),
                )
            });

        let bar = self.render_input_bar(&model, &prompt);
        let menu = if self.focused_render { self.render_completion(&model, &prompt, cx) } else { None };
        Some(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .bg(rgb(c_bg()))
                .child(list)
                .child(bar)
                .when_some(menu, |d, m| d.child(m))
                .into_any_element(),
        )
    }

    /// Um bloco: linha de contexto (pasta, branch, duração), comando em negrito e saída.
    #[allow(clippy::too_many_arguments)]
    fn block_row(
        &self,
        b: &BlockInfo,
        bx: BlockBox,
        first_row: usize,
        row_els: Vec<AnyElement>,
        mono: &gpui::SharedString,
        scroll: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = &self.tabs[self.active];
        let id = b.id;
        let uid = id as usize;
        let failed = !b.running && b.status.is_some_and(|s| s != 0);
        let selected = tab.selected_block == Some(id);
        let hovered = tab.hovered_block == Some(id);
        let bookmarked = tab.bookmarks.contains(&id);
        let stripe = if failed {
            Some(RED)
        } else if selected {
            Some(c_accent())
        } else {
            None
        };
        let bg = if failed {
            ERROR_BG
        } else if selected {
            c_selected_block_bg()
        } else {
            c_bg()
        };
        let small = (self.font_size - 2.0).max(11.0);
        let duration = tab.durations.get(&id).map(|s| format_duration(*s));

        // linha de contexto embaixo do comando: pasta, branch, duração (discreta)
        let context = div()
            .h(px(CONTEXT_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(8.))
            .font_family(mono.clone())
            .text_size(px(small))
            .text_color(rgb(c_disabled()))
            .when_some(b.info.venv.clone(), |d, v| d.child(div().flex_none().child(v)))
            .child(div().flex_none().child(b.info.path.clone()))
            .when_some(b.info.branch.clone(), |d, br| {
                d.child(div().flex_none().flex().items_center().gap(px(4.)).child(wicon("git-branch-02", 12., c_disabled())).child(br))
            })
            .when_some(duration, |d, t| d.child(div().flex_none().child(t)))
            .when(b.running, |d| d.child(div().flex_none().text_color(rgb(c_coral())).child("✳ executando…")))
            .when(failed, |d| d.child(div().flex_none().text_color(rgb(RED)).child(format!("exit {}", b.status.unwrap_or(0)))))
            .when(bookmarked, |d| d.child(wicon("bookmark", 12., c_accent())))
            .child(div().flex_1())
            // ferramentas do bloco, visíveis ao passar o mouse (como o "toolbelt" do Warp)
            .when(hovered || selected, |d| {
                d.child(div().flex_none().text_color(rgb(c_muted())).child(b.info.time.clone().unwrap_or_default()))
                    .child(tool_btn(("blk-copy", uid), "copy").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.block_action(id, BlockAction::CopyOutput, cx);
                    })))
                    .child(tool_btn(("blk-bm", uid), "bookmark").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.block_action(id, BlockAction::ToggleBookmark, cx);
                    })))
                    .child(tool_btn(("blk-more", uid), "dots-vertical").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.block_menu = Some(BlockMenu { id, x: f32::from(ev.position.x), y: f32::from(ev.position.y) });
                            cx.notify();
                        }),
                    ))
            });

        // o comando numa faixa clara, como a linha do prompt destacada
        let command = div()
            .h(px(self.cell_h + 6.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(self.cell_w))
            .px(px(8.))
            .rounded(px(6.))
            .bg(rgb(c_cmd_bg()))
            .font_family(mono.clone())
            .text_size(px(self.font_size))
            .whitespace_nowrap()
            .overflow_hidden()
            .child(div().flex_none().text_color(rgb(if failed { RED } else { c_muted() })).child("❯"))
            .child(div().flex_none().text_color(rgb(c_text())).child(if b.command.is_empty() { "—".to_string() } else { b.command.clone() }));

        div()
            .id(("blk", uid))
            .absolute()
            .left_0()
            .right_0()
            .top(px(bx.top - scroll))
            .h(px(bx.h))
            .bg(rgb(bg))
            .overflow_hidden()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let t = this.tab();
                if *hovered {
                    t.hovered_block = Some(id);
                } else if t.hovered_block == Some(id) {
                    t.hovered_block = None;
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation(); // no bloco o clique direito abre o menu (fora dele cola)
                    this.tab().selected_block = Some(id);
                    this.block_menu = Some(BlockMenu { id, x: f32::from(ev.position.x), y: f32::from(ev.position.y) });
                    cx.notify();
                }),
            )
            .when_some(stripe, |d, c| d.child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(rgb(c))))
            .child(
                div()
                    .id(("blk-head", uid))
                    .pt(px(PAD_TOP))
                    .pl(px(BLOCK_PAD_X - 8.0))
                    .pr(px(BLOCK_PAD_X))
                    .flex()
                    .flex_col()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let t = this.tab();
                        t.selected_block = if t.selected_block == Some(id) { None } else { Some(id) };
                        cx.notify();
                    }))
                    .child(command)
                    .child(context),
            )
            .when(bx.rows > 0, |d| {
                d.child(div().flex_none().h(px(BODY_GAP))).child(
                    div()
                        .px(px(BLOCK_PAD_X))
                        .flex()
                        .flex_col()
                        .font_family(mono.clone())
                        .text_size(px(self.font_size))
                        .line_height(px(self.cell_h))
                        .text_color(rgb(c_out_text()))
                        .child(div().flex_none().h(px(first_row as f32 * self.cell_h)))
                        .children(row_els),
                )
            })
            .into_any_element()
    }

    /// Menu de contexto do bloco (mesmas opções do Warp, em português).
    pub fn render_block_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.block_menu.as_ref()?;
        let id = menu.id;
        let (mx, my) = (menu.x, menu.y);
        let b = self.find_block(id)?;
        let collapsed = self.tabs[self.active].collapsed.contains(&id);
        let bookmarked = self.tabs[self.active].bookmarks.contains(&id);
        let has_branch = b.info.branch.is_some();
        let (x, y) = (mx.min(self.vw - 280.0).max(4.0), my);

        let mut items: Vec<Option<(&'static str, &'static str, BlockAction)>> = vec![
            Some(("copy", "Copiar", BlockAction::Copy)),
            Some(("terminal-input", "Copiar comando", BlockAction::CopyCommand)),
            Some(("file", "Copiar saída", BlockAction::CopyOutput)),
            Some(("copy", "Copiar comando e saída", BlockAction::CopyBoth)),
            Some(("folder-closed", "Copiar diretório", BlockAction::CopyDir)),
        ];
        if has_branch {
            items.push(Some(("git-branch-02", "Copiar branch git", BlockAction::CopyBranch)));
        }
        items.extend([
            None,
            Some(("notebook", "Criar nota a partir do bloco", BlockAction::CreateNote)),
            None,
            Some(("refresh", "Reexecutar comando", BlockAction::Rerun)),
            Some(("bookmark", if bookmarked { "Remover marcador" } else { "Marcar bloco" }, BlockAction::ToggleBookmark)),
            Some(("chevron-down", if collapsed { "Expandir saída" } else { "Recolher saída" }, BlockAction::ToggleCollapse)),
            None,
            Some(("x-close", "Fechar bloco", BlockAction::Close)),
        ]);

        let rows: Vec<AnyElement> = items
            .into_iter()
            .enumerate()
            .map(|(i, it)| match it {
                None => div().h(px(1.)).my_1().bg(rgb(c_border())).into_any_element(),
                Some((icon, label, action)) => div()
                    .id(("blk-menu", i))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(4.))
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(c_btn_hover())))
                    .child(wicon(icon, 14., c_muted()))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.block_menu = None;
                        this.block_action(id, action, cx);
                    }))
                    .into_any_element(),
            })
            .collect();

        Some(
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(260.))
                .occlude()
                .p_1()
                .rounded(px(6.))
                .bg(rgb(c_surface()))
                .border_1()
                .border_color(rgb(c_btn_hover()))
                .shadow_lg()
                .flex()
                .flex_col()
                // clique dentro do menu não deve fechá-lo antes de a ação rodar
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
                .children(rows)
                .into_any_element(),
        )
    }

    fn block_action(&mut self, id: i64, action: BlockAction, cx: &mut Context<Self>) {
        let Some(b) = self.find_block(id) else { return };
        let output = self.block_output(&b);
        let copy = |cx: &mut Context<Self>, text: String| cx.write_to_clipboard(ClipboardItem::new_string(text));
        match action {
            BlockAction::Copy => {
                let sel = self.tabs[self.active].term.selection_text();
                copy(cx, sel.unwrap_or(output));
            }
            BlockAction::CopyCommand => copy(cx, b.command.clone()),
            BlockAction::CopyOutput => copy(cx, output),
            BlockAction::CopyBoth => copy(cx, format!("{}\n{output}", b.command)),
            BlockAction::CopyDir => copy(cx, b.info.path.clone()),
            BlockAction::CopyBranch => copy(cx, b.info.branch.clone().unwrap_or_default()),
            BlockAction::CreateNote => self.block_to_note(id),
            BlockAction::Rerun => self.run_in_terminal(&b.command),
            BlockAction::ToggleCollapse => {
                let t = self.tab();
                if !t.collapsed.remove(&id) {
                    t.collapsed.insert(id);
                }
            }
            BlockAction::ToggleBookmark => {
                let t = self.tab();
                if !t.bookmarks.remove(&id) {
                    t.bookmarks.insert(id);
                }
            }
            BlockAction::Close => {
                self.tab().hidden.insert(id);
            }
        }
        cx.notify();
    }


    /// Barra de entrada no estilo do Warp: chips de contexto, a linha digitada e as dicas de atalho.
    /// Cartão de completar caminhos, logo acima da linha digitada e alinhado ao começo da palavra.
    fn render_completion(&self, model: &BlockModel, prompt: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.completion.as_ref()?;
        const ROW_H: f32 = 30.0;
        const VISIBLE: usize = 8;
        let tab = &self.tabs[self.active];
        let iv = tab.term.input_view(model, prompt);
        let main_w = self.tabs[self.active].rect.w;
        let word_col = iv.before.chars().count().saturating_sub(menu.base.chars().count());
        let w = 380.0_f32;
        let left = (BLOCK_PAD_X + self.cell_w * (2.0 + word_col as f32) - 12.0).clamp(8.0, (main_w - w - 12.0).max(8.0));
        // janela de até 8 linhas que acompanha a seleção
        let first = menu.sel.saturating_sub(VISIBLE - 1).min(menu.items.len().saturating_sub(VISIBLE));
        let font = self.icon_font.clone();
        let rows = menu.items.iter().enumerate().skip(first).take(VISIBLE).map(|(i, it)| {
            let active = i == menu.sel;
            let kind = if it.is_dir { ox_files::EntryKind::Dir } else { ox_files::EntryKind::File };
            let ic = crate::icons::file_icon(&it.name, &kind, false);
            div()
                .id(("compl", i))
                .h(px(ROW_H))
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .rounded(px(4.))
                .cursor_pointer()
                .when(active, |d| d.bg(rgb(c_surface())))
                .when(!active, |d| d.hover(|s| s.bg(rgb(c_hover()))))
                .child(crate::icons::icon_el(&font, &ic, 14.))
                .child(
                    div()
                        .flex_none()
                        .font_family(self.mono.clone())
                        .text_size(px(14.))
                        .text_color(rgb(c_text()))
                        .child(format!("{}{}", it.name, if it.is_dir { "/" } else { "" })),
                )
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(rgb(c_disabled())).child(if it.is_dir { "Pasta" } else { "Arquivo" }))
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(m) = this.completion.take() {
                        let item = m.items[i].clone();
                        this.accept_completion(&m.base, &item, &m.before);
                    }
                    cx.notify();
                }))
        });
        let more = menu.items.len().saturating_sub(VISIBLE);
        Some(
            div()
                .absolute()
                .occlude()
                .left(px(left))
                .bottom(px(INPUT_H - 30.0))
                .w(px(w))
                .p_1()
                .rounded(px(8.))
                .bg(rgb(c_panel()))
                .border_1()
                .border_color(rgb(c_card_border()))
                .shadow_lg()
                .flex()
                .flex_col()
                // a roda do mouse percorre as opções (a seleção sempre fica visível)
                .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(m) = this.completion.as_mut() {
                        m.wheel -= f32::from(ev.delta.pixel_delta(px(30.)).y) / 30.0;
                        let steps = m.wheel.trunc() as i64;
                        m.wheel -= steps as f32;
                        m.sel = (m.sel as i64 + steps).clamp(0, m.items.len() as i64 - 1) as usize;
                    }
                    cx.notify();
                }))
                .children(rows)
                .when(more > 0, |d| {
                    d.child(
                        div()
                            .h(px(24.))
                            .px_3()
                            .flex()
                            .items_center()
                            .text_size(px(12.))
                            .text_color(rgb(c_disabled()))
                            .child(format!("{} de {} · ↑↓ escolher · Enter completar · Esc fechar", menu.sel + 1, menu.items.len())),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_input_bar(&self, model: &BlockModel, prompt: &str) -> AnyElement {
        let tab = &self.tabs[self.active];
        let iv = tab.term.input_view(model, prompt);
        let running = !model.idle;
        let main_w = self.tabs[self.active].rect.w;
        let venv = model.blocks.last().and_then(|b| b.info.venv.clone());

        // linha de estado: o que está rodando (coral) ou onde você está (discreto)
        let status = if running {
            let secs = tab.cmd_starts.front().map(|t| t.elapsed().as_secs()).unwrap_or(0);
            let what = if tab.last_cmd.is_empty() { String::new() } else { format!(" · {}", tab.last_cmd) };
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .text_color(rgb(c_coral()))
                .child(div().flex_none().child("✳"))
                .child(div().min_w_0().truncate().child(format!("Executando… ({secs}s{what})")))
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .text_color(rgb(c_muted()))
                .when_some(venv, |d, v| d.child(div().flex_none().child(v)))
                .child(div().flex_none().child(shorten(&self.shell_cwd)))
                .when_some(self.git.clone(), |d, b| {
                    d.child(div().flex_none().flex().items_center().gap(px(5.)).child(wicon("git-branch-02", 12., c_muted())).child(b))
                })
        };
        let status = div().h(px(24.)).flex_none().px(px(BLOCK_PAD_X)).flex().items_center().font_family(self.mono.clone()).text_size(px(13.)).child(status);

        // linha digitada entre duas linhas finas: "❯", texto, cursor e a sugestão em cinza
        let empty = !running && iv.before.is_empty() && iv.at == ' ' && iv.after.is_empty();
        let ghost = if running || !iv.after.is_empty() { String::new() } else { self.suggestions.first().map(|s| s.ghost(&self.line).to_string()).unwrap_or_default() };
        let input = div()
            .h(px(self.cell_h + 16.0))
            .flex_none()
            .min_w_0()
            .flex()
            .flex_row()
            .items_center()
            .px(px(BLOCK_PAD_X))
            .border_t_1()
            .border_b_1()
            .border_color(rgb(c_surface()))
            .overflow_hidden()
            .whitespace_nowrap()
            .font_family(self.mono.clone())
            .text_size(px(self.font_size))
            .child(div().flex_none().w(px(self.cell_w * 2.0)).text_color(rgb(if running { c_disabled() } else { c_text() })).child("❯"))
            .child(div().flex_none().text_color(rgb(c_text())).child(iv.before))
            .child(div().flex_none().w(px(self.cell_w * 0.6)).h(px(self.cell_h)).when(self.caret_on, |d| d.bg(rgb(c_text())).opacity(0.85)))
            .child(div().flex_none().text_color(rgb(c_text())).child(if iv.at == ' ' { String::new() } else { iv.at.to_string() }))
            .child(div().flex_none().text_color(rgb(c_text())).child(iv.after))
            .child(div().flex_none().text_color(rgb(c_disabled())).child(ghost))
            .when(empty, |d| d.child(div().min_w_0().pl(px(4.)).text_color(rgb(c_disabled())).truncate().child(PLACEHOLDER)));

        let hint_items: &[&str] = if running {
            &["ctrl-c interrompe", "o que você digita vai para o programa", "ctrl-shift-f busca na saída"]
        } else {
            &["shift-tab aceita a sugestão", "ctrl-r histórico", "ctrl-shift-f busca na saída", "ctrl-shift-t novo terminal"]
        };
        let hints = div()
            .h(px(22.))
            .flex_none()
            .px(px(BLOCK_PAD_X))
            .flex()
            .items_center()
            .gap_2()
            .font_family(self.mono.clone())
            .text_size(px(12.))
            .text_color(rgb(c_amber()))
            .whitespace_nowrap()
            .child(div().flex_none().child("▸▸"))
            .child(div().text_color(rgb(c_muted())).child(fit_hints(hint_items, (((main_w - BLOCK_PAD_X * 2.0 - 30.0) / 7.4).max(0.0)) as usize)));

        div()
            .w_full()
            .h(px(INPUT_H))
            .flex_none()
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(4.))
            .bg(rgb(c_bg()))
            .child(status)
            .child(input)
            .child(hints)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{fit_hints, format_duration};

    #[test]
    fn hints_are_dropped_from_the_end_when_space_is_short() {
        let items = ["aceitar (shift+tab)", "histórico (ctrl+r)", "interromper (ctrl+c)"];
        assert_eq!(fit_hints(&items, 200), "aceitar (shift+tab) · histórico (ctrl+r) · interromper (ctrl+c)");
        assert_eq!(fit_hints(&items, 40), "aceitar (shift+tab) · histórico (ctrl+r)");
        assert_eq!(fit_hints(&items, 19), "aceitar (shift+tab)"); // cabe exatamente
        assert_eq!(fit_hints(&items, 18), ""); // nem a primeira cabe: melhor nada do que cortado
        assert_eq!(fit_hints(&[], 50), "");
    }

    #[test]
    fn durations_use_warp_format() {
        assert_eq!(format_duration(0.052), "(0.052s)");
        assert_eq!(format_duration(12.34), "(12.3s)");
        assert_eq!(format_duration(125.0), "(2m 05s)");
    }
}
