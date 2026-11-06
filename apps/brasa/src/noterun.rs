//! Notas executáveis: variáveis `{{nome}}` (o app pergunta os valores antes de rodar), "Rodar todos" (blocos em
//! sequência, parando no primeiro que falhar) e a saída da última execução guardada na própria nota. A parte
//! de texto fica em `ox_markdown` (testada lá); aqui está o que liga isso ao terminal e à tela.

use std::collections::{HashMap, VecDeque};

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, ClickEvent, Context, FontWeight, MouseButton, MouseDownEvent};
use ox_editor::Pos;
use ox_pty::SessionId;

use crate::settings::field_box;
use crate::theme::*;
use crate::{primary_btn, small_btn, Workspace};

/// Pergunta dos valores das variáveis antes de rodar blocos.
pub struct VarPrompt {
    pub names: Vec<String>,
    pub values: Vec<String>,
    pub focus: usize,
    /// (posição do bloco entre os executáveis, comando com as variáveis ainda por trocar)
    pub plan: Vec<(usize, String)>,
}

/// Execução em andamento de um ou mais blocos de uma nota.
pub struct NoteRun {
    /// nota onde guardar as saídas (None: arquivo do disco, só roda)
    pub rel: Option<String>,
    pub session: SessionId,
    pub queue: VecDeque<(usize, String)>,
    pub current: Option<(usize, String)>,
    pub total: usize,
    pub done: usize,
}

fn vars_path() -> std::path::PathBuf {
    crate::config_dir().join("note-vars.txt")
}

/// Últimos valores usados nas variáveis (`nome=valor`, uma por linha).
pub fn load_var_memory() -> HashMap<String, String> {
    std::fs::read_to_string(vars_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.to_string())))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

fn now_stamp() -> String {
    std::process::Command::new("date").arg("+%F %R").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "agora".to_string())
}

impl Workspace {
    /// A nota aberta no editor, se for uma nota do Brasa (não um arquivo do disco).
    fn open_note_rel(&self) -> Option<String> {
        self.editor.as_ref().filter(|e| e.file.is_none() && !e.name.is_empty()).map(|e| e.name.clone())
    }

    /// Roda os blocos executáveis `ordinals` (posição entre os executáveis), perguntando os valores das variáveis.
    pub fn run_note_blocks(&mut self, ordinals: Vec<usize>) {
        let Some(src) = self.editor.as_ref().map(|e| e.buf.text()) else { return };
        let plan: Vec<(usize, String)> = ordinals.into_iter().filter_map(|o| ox_markdown::runnable_command(&src, o).map(|c| (o, c))).collect();
        if plan.is_empty() {
            return;
        }
        let mut names: Vec<String> = Vec::new();
        for (_, c) in &plan {
            for v in ox_markdown::variables(c) {
                if !names.contains(&v) {
                    names.push(v);
                }
            }
        }
        if names.is_empty() {
            self.start_note_run(plan);
        } else {
            let values = names.iter().map(|n| self.var_memory.get(n).cloned().unwrap_or_default()).collect();
            self.var_prompt = Some(VarPrompt { names, values, focus: 0, plan });
        }
    }

    pub fn run_all_note_blocks(&mut self) {
        let Some(src) = self.editor.as_ref().map(|e| e.buf.text()) else { return };
        self.run_note_blocks((0..ox_markdown::runnable_count(&src)).collect());
    }

    /// No modo de edição: o cartão que abre na linha `line`.
    pub fn run_note_block_at_line(&mut self, line: usize) {
        let Some(src) = self.editor.as_ref().map(|e| e.buf.text()) else { return };
        if let Some(o) = ox_markdown::runnable_ordinal_at_line(&src, line) {
            self.run_note_blocks(vec![o]);
        }
    }

    /// No modo de leitura: o bloco de posição `block` entre todos os blocos da nota.
    pub fn run_note_block_at_index(&mut self, block: usize) {
        let ordinal = self.note_blocks[..block.min(self.note_blocks.len())].iter().filter(|b| matches!(b, ox_markdown::Block::Code(c) if c.runnable)).count();
        self.run_note_blocks(vec![ordinal]);
    }

    pub fn confirm_var_prompt(&mut self) {
        let Some(p) = self.var_prompt.take() else { return };
        let values: HashMap<String, String> = p.names.iter().cloned().zip(p.values.iter().cloned()).collect();
        for (k, v) in &values {
            self.var_memory.insert(k.clone(), v.clone());
        }
        let mut text: Vec<String> = self.var_memory.iter().map(|(k, v)| format!("{k}={}", v.replace('\n', " "))).collect();
        text.sort();
        let _ = std::fs::write(vars_path(), text.join("\n") + "\n");
        let plan = p.plan.into_iter().map(|(o, c)| (o, ox_markdown::substitute(&c, &values))).collect();
        self.start_note_run(plan);
    }

    fn start_note_run(&mut self, plan: Vec<(usize, String)>) {
        let total = plan.len();
        let mut queue: VecDeque<(usize, String)> = plan.into();
        let Some(first) = queue.pop_front() else { return };
        self.note_run = Some(NoteRun { rel: self.open_note_rel(), session: self.session(), queue, current: Some(first.clone()), total, done: 0 });
        self.run_notice = Some(if total > 1 { format!("Rodando 1 de {total}…") } else { "Rodando…".to_string() });
        self.run_in_terminal(&first.1);
    }

    /// Para a sequência (o comando que já está rodando continua no terminal).
    pub fn stop_note_run(&mut self) {
        if self.note_run.take().is_some() {
            self.run_notice = Some("Sequência interrompida.".to_string());
        }
    }

    /// Chamado quando um comando termina no terminal. Se era o do bloco em andamento, guarda a saída na nota e
    /// segue para o próximo, ou para no primeiro que falhou.
    pub fn note_command_finished(&mut self, session: SessionId, command: &str, status: Option<i32>, output: &str) {
        let Some(run) = self.note_run.as_ref() else { return };
        let Some((ordinal, cmd)) = run.current.clone() else { return };
        // outro comando que o usuário digitou no meio do caminho não conta
        let first = |s: &str| s.lines().next().unwrap_or("").trim().to_string();
        if run.session != session || (!command.is_empty() && first(command) != first(&cmd)) {
            return;
        }
        let rel = run.rel.clone();
        let (done, total) = (run.done + 1, run.total);
        if let Some(rel) = rel {
            self.store_note_output(&rel, ordinal, output, status);
        }
        let ok = status.unwrap_or(0) == 0;
        let next = if ok { self.note_run.as_mut().and_then(|r| r.queue.pop_front()) } else { None };
        match next {
            Some(item) => {
                if let Some(r) = self.note_run.as_mut() {
                    r.current = Some(item.clone());
                    r.done = done;
                }
                self.run_notice = Some(format!("Rodando {} de {total}…", done + 1));
                self.run_in_terminal(&item.1);
            }
            None => {
                self.note_run = None;
                self.run_notice = Some(if ok {
                    if total > 1 { format!("Terminou: {total} blocos.") } else { "Terminou.".to_string() }
                } else {
                    format!("O bloco {} falhou (código {}); parei a sequência.", done, status.unwrap_or(-1))
                });
            }
        }
    }

    /// Guarda a saída de uma execução na nota: no editor, se ela está aberta; senão, direto no arquivo.
    fn store_note_output(&mut self, rel: &str, ordinal: usize, output: &str, status: Option<i32>) {
        let stamp = now_stamp();
        if let Some(ed) = self.editor.as_mut().filter(|e| e.file.is_none() && e.name == rel) {
            if let Some(new) = ox_markdown::with_output(&ed.buf.text(), ordinal, output, status, &stamp) {
                let cur = ed.buf.cursor();
                ed.buf.select_all();
                ed.buf.insert_str(&new);
                let line = cur.line.min(ed.buf.line_count().saturating_sub(1));
                ed.buf.move_to(Pos::new(line, cur.col), false);
                self.note_blocks = ox_markdown::parse(&new);
                self.save_note();
            }
        } else if let Ok(src) = self.notes.read(rel) {
            if let Some(new) = ox_markdown::with_output(&src, ordinal, output, status, &stamp) {
                let _ = self.notes.write(rel, &new);
            }
        }
    }

    /// Teclas no cartão das variáveis: Tab e setas trocam de campo, Enter roda, Esc cancela.
    pub fn on_var_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.var_prompt.as_mut() else { return false };
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            return false;
        }
        let n = p.names.len().max(1);
        match k.key.to_lowercase().as_str() {
            "escape" => self.var_prompt = None,
            "enter" => self.confirm_var_prompt(),
            "tab" | "down" => p.focus = (p.focus + 1) % n,
            "up" => p.focus = (p.focus + n - 1) % n,
            "backspace" => {
                p.values[p.focus].pop();
            }
            "space" => p.values[p.focus].push(' '),
            _ => {
                if let Some(c) = &k.key_char {
                    p.values[p.focus].push_str(c);
                }
            }
        }
        cx.notify();
        true
    }

    pub fn render_var_prompt(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.var_prompt.as_ref()?;
        let blocks = p.plan.len();
        let rows: Vec<AnyElement> = p
            .names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().font_family(self.mono.clone()).text_color(rgb(c_muted())).child(format!("{{{{{name}}}}}")))
                    .child(field_box(("var", i), &p.values[i], "valor", p.focus == i, 420.).on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            if let Some(p) = this.var_prompt.as_mut() {
                                p.focus = i;
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ))
                    .into_any_element()
            })
            .collect();
        Some(
            div()
                .id("var-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .bg(rgba(0x00000066))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.var_prompt = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .w(px(480.))
                        .max_w_full()
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(c_card_border()))
                        .bg(rgb(c_panel()))
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Valores das variáveis"))
                        .child(div().text_sm().text_color(rgb(c_muted())).child(if blocks > 1 { format!("Vale para os {blocks} blocos que vão rodar. O último valor fica lembrado.") } else { "O último valor usado fica lembrado.".to_string() }))
                        .children(rows)
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(small_btn("var-cancel", "Cancelar").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.var_prompt = None;
                                    cx.notify();
                                })))
                                .child(primary_btn("var-run", "Rodar").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.confirm_var_prompt();
                                    cx.notify();
                                }))),
                        ),
                )
                .into_any_element(),
        )
    }
}
