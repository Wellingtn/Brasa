//! Emulação de terminal (alacritty_terminal) + conversão da grade em "runs" prontos para desenhar.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, Processor};
use gpui::Keystroke;

use ox_pty::{parse_prompt_row, PromptInfo, PROMPT_BRANCH, PROMPT_START, PROMPT_VENV};
#[cfg(test)]
use ox_pty::{PROMPT_STATUS, PROMPT_TIME};

use crate::theme::*;

const ANSI: [u32; 16] = [
    // paleta do tema Dark do Warp: normais e brilhantes
    0x616161, 0xff8272, 0xb4fa72, 0xfefdc2, 0xa5d5fe, 0xff8ffd, 0xd0d1fe, 0xf1f1f1, //
    0x8e8e8e, 0xffc4bd, 0xd6fcb9, 0xfefdd5, 0xc1e3fe, 0xffb1fe, 0xe5e6fe, 0xfeffff,
];

/// Respostas que o terminal precisa devolver ao shell (ex.: posição do cursor).
#[derive(Clone, Default)]
pub struct Listener {
    replies: Arc<Mutex<Vec<String>>>,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => self.replies.lock().unwrap().push(text),
            Event::Title(t) => *self.title.lock().unwrap() = Some(t),
            Event::ResetTitle => *self.title.lock().unwrap() = Some(String::new()),
            _ => {}
        }
    }
}

struct Dims {
    cols: usize,
    rows: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Trecho contíguo de células com o mesmo estilo.
pub struct Run {
    pub text: String,
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub underline: bool,
}

/// Ocorrência de busca: `line` é a linha da grade (negativa = histórico); `col`/`len` em colunas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: i32,
    pub col: usize,
    pub len: usize,
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Sequência de relatório de mouse para apps TUI. `button` já inclui movimento (+32) e modificadores.
pub fn mouse_report(
    mode: TermMode,
    button: u8,
    col: usize,
    row: usize,
    release: bool,
) -> Option<Vec<u8>> {
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if release { 'm' } else { 'M' };
        Some(format!("\x1b[<{button};{};{}{end}", col + 1, row + 1).into_bytes())
    } else {
        // protocolo X10/normal: só alcança 222 colunas/linhas; solta = botão 3
        if col >= 222 || row >= 222 {
            return None;
        }
        let b = if release { 3 } else { button };
        Some(vec![
            0x1b,
            b'[',
            b'M',
            32 + b,
            32 + (col + 1) as u8,
            32 + (row + 1) as u8,
        ])
    }
}

/// Uma linha da tela pronta para desenhar.
pub enum RowView {
    Cells(Vec<Run>),
    /// linha de prompt do Brasa: a UI desenha chips (pasta/branch/venv) no lugar do texto
    Prompt(PromptInfo),
}

impl RowView {
    pub fn runs(&self) -> &[Run] {
        match self {
            RowView::Cells(r) => r,
            RowView::Prompt(_) => &[],
        }
    }
}

struct StyledCell {
    ch: char,
    fg: u32,
    bg: u32,
    bold: bool,
    underline: bool,
}

/// Remove os marcadores invisíveis do prompt ao copiar texto.
fn strip_prompt_markers(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            PROMPT_START => None,
            PROMPT_BRANCH | PROMPT_VENV => Some(' '),
            c => Some(c),
        })
        .collect()
}

/// Um comando e a sua saída, delimitados pelas linhas de prompt do Brasa.
/// As linhas são "linhas da grade" (negativas = histórico).
#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub prompt: i32,
    /// primeira linha da saída
    pub out_start: i32,
    /// fim (exclusivo) da saída
    pub out_end: i32,
    pub info: PromptInfo,
    pub command: String,
    /// código de saída; `None` enquanto o comando roda (ou se o shell não informou)
    pub status: Option<i32>,
    pub running: bool,
    /// identidade estável: linha absoluta contada desde a linha mais antiga do histórico
    pub id: i64,
}

pub struct BlockModel {
    pub blocks: Vec<BlockInfo>,
    /// o último prompt está aguardando digitação (nenhum comando em execução)
    pub idle: bool,
    /// primeira linha da entrada do último prompt
    pub input_line: i32,
    pub cursor_line: i32,
}

/// A linha de entrada, dividida ao redor do cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputView {
    pub before: String,
    pub at: char,
    pub after: String,
}

pub struct TermView {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    pub cols: usize,
    pub rows: usize,
}

impl TermView {
    pub fn new(cols: usize, rows: usize) -> Self {
        let listener = Listener::default();
        let term = Term::new(Config::default(), &Dims { cols, rows }, listener.clone());
        Self {
            term,
            parser: Processor::default(),
            listener,
            cols,
            rows,
        }
    }

    /// Processa bytes vindos do PTY. Devolve respostas que devem ser escritas de volta no PTY.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.parser.advance(&mut self.term, bytes);
        std::mem::take(&mut *self.listener.replies.lock().unwrap())
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if (cols, rows) != (self.cols, self.rows) && cols > 0 && rows > 0 {
            self.term.resize(Dims { cols, rows });
            self.cols = cols;
            self.rows = rows;
        }
    }

    /// Novo título definido pelo programa (OSC 0/2) desde a última chamada.
    pub fn take_title(&mut self) -> Option<String> {
        self.listener.title.lock().unwrap().take()
    }

    /// (linhas acima da tela atual, linhas de histórico)
    pub fn scrollback(&self) -> (usize, usize) {
        (
            self.term.grid().display_offset(),
            self.term.grid().history_size(),
        )
    }

    fn point_at(&self, col: usize, row: usize) -> Point {
        let off = self.term.grid().display_offset() as i32;
        Point::new(
            Line(row as i32 - off),
            Column(col.min(self.cols.saturating_sub(1))),
        )
    }

    /// `clicks`: 1 = caractere, 2 = palavra, 3 = linha.
    pub fn start_selection(&mut self, col: usize, row: usize, right_half: bool, clicks: usize) {
        let ty = match clicks {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        let side = if right_half { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(ty, self.point_at(col, row), side));
    }

    pub fn update_selection(&mut self, col: usize, row: usize, right_half: bool) {
        let side = if right_half { Side::Right } else { Side::Left };
        let point = self.point_at(col, row);
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|t| !t.is_empty()).map(|t| strip_prompt_markers(&t))
    }

    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// O app em execução pediu eventos de mouse (vim, htop, tmux...).
    pub fn mouse_mode(&self) -> bool {
        self.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// Busca (sem distinguir caixa) em todo o histórico + tela. Ordem: do mais antigo ao mais novo.
    pub fn search(&self, query: &str) -> Vec<Match> {
        let q: Vec<char> = query.chars().map(lower).collect();
        if q.is_empty() {
            return Vec::new();
        }
        let grid = self.term.grid();
        let (top, bottom) = (self.term.topmost_line().0, self.term.bottommost_line().0);
        let mut out = Vec::new();
        for l in top..=bottom {
            let row = &grid[Line(l)];
            let (mut chars, mut cols) = (Vec::new(), Vec::new());
            for c in 0..self.cols {
                let cell = &row[Column(c)];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    chars.push(lower(cell.c));
                    cols.push(c);
                }
            }
            let mut i = 0;
            while i + q.len() <= chars.len() {
                if chars[i..i + q.len()] == q[..] {
                    let len = cols[i + q.len() - 1] - cols[i] + 1;
                    out.push(Match {
                        line: l,
                        col: cols[i],
                        len,
                    });
                    i += q.len();
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    /// Rola para deixar `line` (linha da grade) aproximadamente no meio da tela.
    pub fn scroll_to_line(&mut self, line: i32) {
        let (offset, history) = self.scrollback();
        let target = (self.rows as i32 / 2 - line).clamp(0, history as i32);
        self.term
            .scroll_display(Scroll::Delta(target - offset as i32));
    }

    // ---------- blocos (comando + saída) ----------

    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn is_alt_screen(&self) -> bool {
        self.mode().contains(TermMode::ALT_SCREEN)
    }

    /// Texto de uma linha da grade (sem tratar espaços finais).
    pub fn line_text(&self, line: i32) -> String {
        let row = &self.term.grid()[Line(line)];
        (0..self.cols).map(|c| &row[Column(c)]).filter(|c| !c.flags.contains(Flags::WIDE_CHAR_SPACER)).map(|c| c.c).collect()
    }

    fn wraps(&self, line: i32) -> bool {
        self.cols > 0 && self.term.grid()[Line(line)][Column(self.cols - 1)].flags.contains(Flags::WRAPLINE)
    }

    /// Linhas de `from` a `to` (exclusivo) como texto, sem espaços à direita.
    pub fn lines_text(&self, from: i32, to: i32) -> String {
        (from..to).map(|l| self.line_text(l).trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    /// Última linha em [from, to) com conteúdo; `from - 1` se tudo estiver em branco.
    pub fn last_content_line(&self, from: i32, to: i32) -> i32 {
        (from..to).rev().find(|l| !self.line_text(*l).trim().is_empty()).unwrap_or(from - 1)
    }

    /// Agrupa o histórico + tela em blocos usando as linhas de prompt (`prompt_text` = símbolo, p.ex. "❯").
    pub fn blocks(&self, prompt_text: &str) -> BlockModel {
        let grid = self.term.grid();
        let (top, bottom) = (self.term.topmost_line().0, self.term.bottommost_line().0);
        let history = grid.history_size() as i64;
        let cursor_line = grid.cursor.point.line.0;

        // a linha do prompt (pasta, branch...) pode quebrar em várias linhas num painel estreito
        // ou com um caminho longo: junta as continuações e lembra onde ela termina
        let prompts: Vec<(i32, i32, PromptInfo)> = (top..=bottom)
            .filter(|l| grid[Line(*l)][Column(0)].c == PROMPT_START)
            .filter_map(|l| {
                let (mut end, mut text) = (l, self.line_text(l));
                while end < bottom && self.wraps(end) {
                    end += 1;
                    text.push_str(&self.line_text(end));
                }
                parse_prompt_row(&text).map(|i| (l, end, i))
            })
            .collect();

        let mut blocks = Vec::with_capacity(prompts.len());
        let (mut idle, mut input_line) = (false, cursor_line);
        for (i, (pl, prompt_end, info)) in prompts.iter().enumerate() {
            let input = prompt_end + 1;
            // comando: a linha de entrada, mais as continuações de linhas longas
            let mut last_input_row = input;
            let mut text = self.line_text(input);
            while last_input_row < bottom && self.wraps(last_input_row) {
                last_input_row += 1;
                text.push_str(&self.line_text(last_input_row));
            }
            let command = text.trim_end().strip_prefix(prompt_text).unwrap_or(text.trim_end()).trim().to_string();

            let next = prompts.get(i + 1);
            let is_last = next.is_none();
            let this_idle = is_last && cursor_line <= last_input_row;
            let out_start = last_input_row + 1;
            let out_end = match next {
                Some((np, _, _)) => *np,
                None if this_idle => out_start,
                None => cursor_line.max(out_start),
            };
            if is_last {
                idle = this_idle;
                input_line = input;
            }
            blocks.push(BlockInfo {
                prompt: *pl,
                out_start,
                out_end,
                info: info.clone(),
                command,
                status: next.and_then(|(_, _, n)| n.status),
                running: is_last && !this_idle,
                id: *pl as i64 + history,
            });
        }
        BlockModel { blocks, idle, input_line, cursor_line }
    }

    /// Entrada atual separada ao redor do cursor. Ocioso: o texto digitado depois do prompt;
    /// executando: a linha do cursor (ex.: o `>>>` de um REPL ou "Password:").
    pub fn input_view(&self, model: &BlockModel, prompt_text: &str) -> InputView {
        let grid = self.term.grid();
        let col = grid.cursor.point.column.0;
        let (text, idx) = if model.idle {
            let mut text = String::new();
            for l in model.input_line..=model.cursor_line {
                text.push_str(&self.line_text(l));
            }
            let idx = (model.cursor_line - model.input_line).max(0) as usize * self.cols + col;
            let skip = if text.starts_with(prompt_text) { prompt_text.chars().count() + 1 } else { 0 };
            (text.chars().skip(skip).collect::<String>(), idx.saturating_sub(skip))
        } else {
            (self.line_text(model.cursor_line), col)
        };
        let chars: Vec<char> = text.chars().collect();
        InputView {
            before: chars.iter().take(idx).collect(),
            at: chars.get(idx).copied().unwrap_or(' '),
            after: chars.iter().skip(idx + 1).collect::<String>().trim_end().to_string(),
        }
    }

    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.term.selection.as_ref().and_then(|s| s.to_range(&self.term))
    }

    /// Células de uma linha da grade prontas para desenhar (seleção e buscas destacadas).
    pub fn line_runs(&self, line: i32, sel: Option<SelectionRange>, hits: &[(usize, usize, bool)]) -> Vec<Run> {
        let row = &self.term.grid()[Line(line)];
        let mut runs: Vec<Run> = Vec::new();
        for c in 0..self.cols {
            let cell = &row[Column(c)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let (mut fg, mut bg) = (resolve(cell.fg), resolve(cell.bg));
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            if sel.is_some_and(|s| s.contains(Point::new(Line(line), Column(c)))) {
                bg = c_selection();
                if fg == bg {
                    fg = c_term_fg();
                }
            }
            if let Some(&(_, _, current)) = hits.iter().find(|(a, b, _)| c >= *a && c < *b) {
                (bg, fg) = if current { (YELLOW, c_bg()) } else { (0x4a3d12, c_term_fg()) };
            }
            let (bold, underline) = (cell.flags.contains(Flags::BOLD), cell.flags.intersects(Flags::ALL_UNDERLINES));
            match runs.last_mut() {
                Some(r) if (r.fg, r.bg, r.bold, r.underline) == (fg, bg, bold, underline) => r.text.push(cell.c),
                _ => runs.push(Run { text: cell.c.to_string(), fg, bg, bold, underline }),
            }
        }
        runs
    }

    /// Seleção por linha da grade (necessária na lista de blocos, onde não há "linhas da tela").
    pub fn start_selection_at(&mut self, line: i32, col: usize, right_half: bool, clicks: usize) {
        let ty = match clicks {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        let side = if right_half { Side::Right } else { Side::Left };
        let p = Point::new(Line(line), Column(col.min(self.cols.saturating_sub(1))));
        self.term.selection = Some(Selection::new(ty, p, side));
    }

    pub fn update_selection_at(&mut self, line: i32, col: usize, right_half: bool) {
        let side = if right_half { Side::Right } else { Side::Left };
        let p = Point::new(Line(line), Column(col.min(self.cols.saturating_sub(1))));
        if let Some(sel) = self.term.selection.as_mut() {
            sel.update(p, side);
        }
    }

    /// `placeholder`: (texto do prompt, dica). Mostra a dica em cinza na linha de digitação vazia.
    pub fn rows_to_runs(&self, search: Option<(&[Match], usize)>, placeholder: Option<(&str, &str)>) -> Vec<RowView> {
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        let show_cursor = offset == 0 && content.mode.contains(TermMode::SHOW_CURSOR);
        let cursor = content.cursor.point;
        let selection = content.selection;
        let mut grid: Vec<Vec<StyledCell>> = (0..self.rows).map(|_| Vec::with_capacity(self.cols)).collect();
        // posição do cursor dentro da linha (índice na lista de células)
        let mut cursor_at: Option<(usize, usize)> = None;
        // ocorrências visíveis por linha da tela: (col_ini, col_fim, é_a_atual)
        let mut hits: Vec<Vec<(usize, usize, bool)>> = vec![Vec::new(); self.rows];
        if let Some((matches, current)) = search {
            for (i, m) in matches.iter().enumerate() {
                let r = m.line + offset;
                if r >= 0 && (r as usize) < self.rows {
                    hits[r as usize].push((m.col, m.col + m.len, i == current));
                }
            }
        }

        for ic in content.display_iter {
            let cell = ic.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let ridx = (ic.point.line.0 + offset).max(0) as usize;
            let Some(row) = grid.get_mut(ridx) else { continue };

            let (mut fg, mut bg) = (resolve(cell.fg), resolve(cell.bg));
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if show_cursor && ic.point == cursor {
                cursor_at = Some((ridx, row.len()));
                std::mem::swap(&mut fg, &mut bg);
                if fg == bg {
                    fg = c_term_bg();
                    bg = c_term_fg();
                }
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            if selection.is_some_and(|sel| sel.contains(ic.point)) {
                bg = c_selection();
                if fg == bg {
                    fg = c_term_fg();
                }
            }
            let col = ic.point.column.0;
            if let Some(&(_, _, is_current)) = hits[ridx].iter().find(|(a, b, _)| col >= *a && col < *b) {
                (bg, fg) = if is_current { (YELLOW, c_bg()) } else { (0x4a3d12, c_term_fg()) };
            }
            row.push(StyledCell {
                ch: cell.c,
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            });
        }

        // dica na linha de digitação: só quando a linha contém apenas o prompt e o cursor está logo depois
        if let (Some((prompt, hint)), Some((r, idx))) = (placeholder, cursor_at) {
            let text: String = grid[r].iter().map(|c| c.ch).collect();
            if text.trim_end() == prompt && idx > prompt.chars().count() {
                for (i, ch) in hint.chars().enumerate() {
                    if let Some(cell) = grid[r].get_mut(idx + 1 + i) {
                        (cell.ch, cell.fg) = (ch, c_muted());
                    }
                }
            }
        }

        grid.into_iter()
            .map(|cells| {
                if cells.first().is_some_and(|c| c.ch == PROMPT_START) {
                    let text: String = cells.iter().map(|c| c.ch).collect();
                    if let Some(info) = parse_prompt_row(&text) {
                        return RowView::Prompt(info);
                    }
                }
                let mut runs: Vec<Run> = Vec::new();
                for c in cells {
                    match runs.last_mut() {
                        Some(r) if (r.fg, r.bg, r.bold, r.underline) == (c.fg, c.bg, c.bold, c.underline) => r.text.push(c.ch),
                        _ => runs.push(Run { text: c.ch.to_string(), fg: c.fg, bg: c.bg, bold: c.bold, underline: c.underline }),
                    }
                }
                RowView::Cells(runs)
            })
            .collect()
    }
}

fn resolve(color: Color) -> u32 {
    match color {
        Color::Spec(rgb) => (rgb.r as u32) << 16 | (rgb.g as u32) << 8 | rgb.b as u32,
        Color::Indexed(i) => indexed(i),
        Color::Named(n) => match n as usize {
            i if i < 16 => ANSI[i],
            257 => c_term_bg(), // Background
            _ => c_term_fg(),   // Foreground, Cursor, Dim*
        },
    }
}

fn indexed(i: u8) -> u32 {
    match i {
        0..=15 => ANSI[i as usize],
        16..=231 => {
            let n = i as u32 - 16;
            let level = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
            level(n / 36) << 16 | level(n / 6 % 6) << 8 | level(n % 6)
        }
        _ => {
            let g = 8 + (i as u32 - 232) * 10;
            g << 16 | g << 8 | g
        }
    }
}

/// Traduz uma tecla do GPUI para os bytes que um terminal xterm espera.
pub fn key_bytes(k: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &k.modifiers;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let csi = |c: char| -> Vec<u8> {
        if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };

    let named: Option<Vec<u8>> = match k.key.as_str() {
        "enter" => Some(b"\r".to_vec()),
        "tab" if m.shift => Some(b"\x1b[Z".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "escape" => Some(b"\x1b".to_vec()),
        "backspace" => Some(b"\x7f".to_vec()),
        "up" => Some(csi('A')),
        "down" => Some(csi('B')),
        "right" => Some(csi('C')),
        "left" => Some(csi('D')),
        "home" => Some(csi('H')),
        "end" => Some(csi('F')),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "insert" => Some(b"\x1b[2~".to_vec()),
        _ => None,
    };
    if named.is_some() {
        return named;
    }

    if m.control {
        let c = k
            .key
            .chars()
            .next()
            .filter(|_| k.key.chars().count() == 1)?;
        let byte = match c.to_ascii_lowercase() {
            c @ 'a'..='z' => c as u8 - b'a' + 1,
            ' ' | '2' | '@' => 0,
            '[' => 27,
            '\\' => 28,
            ']' => 29,
            _ => return None,
        };
        return Some(vec![byte]);
    }

    if k.key == "space" {
        return Some(if m.alt {
            b"\x1b ".to_vec()
        } else {
            b" ".to_vec()
        });
    }
    let text = k.key_char.as_deref()?;
    if m.platform {
        return None;
    }
    let mut out = Vec::new();
    if m.alt {
        out.push(0x1b);
    }
    out.extend_from_slice(text.as_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_renders_text_into_runs() {
        let mut t = TermView::new(20, 3);
        t.feed(b"\x1b[31mhi\x1b[0m ok");
        let rows = t.rows_to_runs(None, None);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].runs()[0].text, "hi");
        assert_eq!(rows[0].runs()[0].fg, ANSI[1]); // vermelho
    }

    #[test]
    fn selection_copies_text_and_title_is_captured() {
        let mut t = TermView::new(20, 3);
        t.feed(b"\x1b]2;meu titulo\x07hello world");
        assert_eq!(t.take_title().as_deref(), Some("meu titulo"));
        assert_eq!(t.take_title(), None);
        t.start_selection(0, 0, false, 1);
        t.update_selection(4, 0, true);
        assert_eq!(t.selection_text().as_deref(), Some("hello"));
        t.start_selection(7, 0, false, 2); // duplo clique: palavra
        assert_eq!(t.selection_text().as_deref(), Some("world"));
        t.clear_selection();
        assert_eq!(t.selection_text(), None);
    }

    #[test]
    fn mouse_report_sgr_and_x10() {
        let sgr = TermMode::SGR_MOUSE;
        assert_eq!(mouse_report(sgr, 0, 4, 9, false).unwrap(), b"\x1b[<0;5;10M");
        assert_eq!(mouse_report(sgr, 0, 4, 9, true).unwrap(), b"\x1b[<0;5;10m");
        let x10 = TermMode::empty();
        assert_eq!(
            mouse_report(x10, 0, 0, 0, false).unwrap(),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(mouse_report(x10, 0, 0, 0, true).unwrap()[3], 35); // solta = botão 3
        assert!(mouse_report(x10, 0, 300, 0, false).is_none());
    }

    #[test]
    fn mouse_mode_follows_app_requests() {
        let mut t = TermView::new(20, 3);
        assert!(!t.mouse_mode());
        t.feed(b"\x1b[?1000h\x1b[?1006h");
        assert!(t.mouse_mode());
        assert!(t.mode().contains(TermMode::SGR_MOUSE));
        t.feed(b"\x1b[?1000l");
        assert!(!t.mouse_mode());
    }

    #[test]
    fn search_spans_history_and_scrolls_to_match() {
        let mut t = TermView::new(20, 3);
        for i in 0..30 {
            t.feed(format!("linha {i} Foo\r\n").as_bytes());
        }
        let m = t.search("linha 2");
        // "linha 2" casa com 2 e 20..=29
        assert_eq!(m.len(), 11);
        assert!(m[0].line < 0, "primeira ocorrência está no histórico");
        assert_eq!(m[0].len, 7);
        assert_eq!(t.search("FOO").len(), 30); // sem distinguir caixa
        assert!(t.search("").is_empty());
        t.scroll_to_line(m[0].line);
        let runs = t.rows_to_runs(Some((&m, 0)), None);
        let shown: String = runs
            .iter()
            .flat_map(|r| r.runs())
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("|");
        assert!(shown.contains("linha 2"), "{shown}");
        assert!(
            runs.iter().flat_map(|r| r.runs()).any(|r| r.bg == YELLOW),
            "ocorrência atual destacada"
        );
    }

    #[test]
    fn prompt_row_becomes_chips_and_copy_strips_markers() {
        let mut t = TermView::new(40, 4);
        t.feed(format!("{PROMPT_START}~/proj{PROMPT_BRANCH}main{PROMPT_VENV}venv\r\n❯ ").as_bytes());
        let rows = t.rows_to_runs(None, None);
        let RowView::Prompt(info) = &rows[0] else { panic!("linha 0 deveria ser o prompt") };
        assert_eq!((info.path.as_str(), info.branch.as_deref(), info.venv.as_deref()), ("~/proj", Some("main"), Some("venv")));
        assert!(matches!(rows[1], RowView::Cells(_)));

        t.start_selection(0, 0, false, 3); // triplo clique seleciona a linha do prompt
        assert_eq!(t.selection_text().unwrap().trim_end(), "~/proj main venv"); // seleção de linha inclui o \n
    }

    #[test]
    fn placeholder_shows_only_on_empty_input_line() {
        let mut t = TermView::new(40, 3);
        t.feed("❯ ".as_bytes());
        let text = |rows: &[RowView]| rows[0].runs().iter().map(|r| r.text.clone()).collect::<String>();
        let with = t.rows_to_runs(None, Some(("❯", "dica aqui")));
        assert!(text(&with).contains("dica aqui"));
        assert!(rows_have_color(&with[0], c_muted())); // desenhada em cinza

        t.feed(b"ls");
        let typed = t.rows_to_runs(None, Some(("❯", "dica aqui")));
        assert!(!text(&typed).contains("dica"), "com texto digitado a dica some");
    }

    fn rows_have_color(row: &RowView, color: u32) -> bool {
        row.runs().iter().any(|r| r.fg == color)
    }

    fn session() -> TermView {
        let mut t = TermView::new(30, 6);
        let p = |st: &str, time: &str| format!("{PROMPT_START}~/p{PROMPT_STATUS}{st}{PROMPT_TIME}{time}\r\n❯ ");
        t.feed(format!("{}ls\r\nfile1\r\nfile2\r\n{}false\r\n{}", p("", "10:00"), p("0", "10:01"), p("1", "10:02")).as_bytes());
        t
    }

    #[test]
    fn blocks_group_commands_outputs_and_status_across_history() {
        let t = session(); // 8 linhas em 6: duas já foram para o histórico
        assert!(t.history_size() >= 2);
        let m = t.blocks("❯");
        assert_eq!(m.blocks.len(), 3);
        assert_eq!(m.blocks[0].command, "ls");
        assert_eq!(m.blocks[0].status, Some(0)); // vem do prompt seguinte
        assert_eq!(t.lines_text(m.blocks[0].out_start, m.blocks[0].out_end), "file1\nfile2");
        assert_eq!((m.blocks[1].command.as_str(), m.blocks[1].status), ("false", Some(1)));
        assert_eq!(t.lines_text(m.blocks[1].out_start, m.blocks[1].out_end), ""); // sem saída
        assert_eq!(m.blocks[0].info.time.as_deref(), Some("10:00"));
        assert!(m.idle && !m.blocks[2].running, "último prompt aguardando digitação");
        assert_eq!(m.blocks[2].status, None);
        // identidades estáveis e diferentes
        let ids: Vec<_> = m.blocks.iter().map(|b| b.id).collect();
        assert!(ids[0] < ids[1] && ids[1] < ids[2]);
    }

    #[test]
    fn block_ids_stay_stable_when_new_output_scrolls_the_screen() {
        let mut t = session();
        let before = t.blocks("❯").blocks[1].id;
        t.feed(b"x\r\ny\r\nz\r\n");
        assert_eq!(t.blocks("❯").blocks[1].id, before);
    }

    #[test]
    fn input_view_idle_running_and_typed() {
        let mut t = session();
        let m = t.blocks("❯");
        let empty = t.input_view(&m, "❯");
        assert_eq!((empty.before.as_str(), empty.at, empty.after.as_str()), ("", ' ', ""));

        t.feed(b"echo hi");
        let m = t.blocks("❯");
        assert_eq!(t.input_view(&m, "❯").before, "echo hi");

        // comando em execução: a linha do cursor (ex.: pergunta de um programa)
        t.feed("\r\nPassword: ".as_bytes());
        let m = t.blocks("❯");
        assert!(!m.idle && m.blocks.last().unwrap().running);
        assert_eq!(t.input_view(&m, "❯").before, "Password: ");
        assert_eq!(m.blocks.last().unwrap().command, "echo hi");
    }

    #[test]
    fn line_runs_and_selection_by_grid_line() {
        let mut t = session();
        let m = t.blocks("❯");
        let b = &m.blocks[0];
        t.start_selection_at(b.out_start, 0, false, 1);
        t.update_selection_at(b.out_start, 4, true);
        assert_eq!(t.selection_text().as_deref(), Some("file1"));
        let runs = t.line_runs(b.out_start, t.selection_range(), &[]);
        assert!(runs.iter().any(|r| r.bg == c_selection() && r.text.starts_with("file1")));
        assert!(t.line_runs(b.out_start + 1, None, &[(0, 4, true)]).iter().any(|r| r.bg == YELLOW));
    }

    /// Caminho principal do produto, sem mocks: bash real -> PTY -> emulador -> blocos.
    #[cfg(unix)]
    /// Medição de desempenho (rode com `cargo test --release -p brasa -- --ignored desempenho --nocapture`):
    /// quantos MB/s de saída o terminal processa e quanto custa achar os blocos com histórico cheio.
    #[test]
    #[ignore]
    fn desempenho_vazao_e_blocos() {
        use std::time::Instant;
        let mut t = TermView::new(120, 40);
        // texto comum com cores, como a saída de um build ou de um ls --color
        let line = "\x1b[32mcompilando\x1b[0m crate_exemplo v0.1.0 (/home/usuario/projeto/crates/exemplo) \x1b[1;34m[ok]\x1b[0m\r\n";
        let chunk = line.repeat(1000);
        let mb = 50.0;
        let reps = (mb * 1024.0 * 1024.0 / chunk.len() as f64) as usize;
        let t0 = Instant::now();
        for _ in 0..reps {
            t.feed(chunk.as_bytes());
        }
        let secs = t0.elapsed().as_secs_f64();
        println!("vazão: {:.0} MB/s ({mb} MB em {:.2} s)", mb / secs, secs);
        // histórico com muitos blocos (prompts) e o custo de montar a lista de blocos
        let mut t = TermView::new(120, 40);
        for i in 0..2000 {
            t.feed(format!("{PROMPT_START}/tmp\r\n❯ echo {i}\r\n{i}\r\n").as_bytes());
        }
        let t0 = Instant::now();
        let n = 50;
        let mut blocks = 0;
        for _ in 0..n {
            blocks = t.blocks("❯").blocks.len();
        }
        println!("blocos: {blocks} no histórico, montar a lista leva {:.2} ms", t0.elapsed().as_secs_f64() * 1000.0 / n as f64);
    }

    #[test]
    fn long_prompt_path_that_wraps_is_not_read_as_the_command() {
        use ox_pty::{PromptConfig, PtyEvents, PtyManager, SessionId, SpawnConfig};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        if !std::path::Path::new("/bin/bash").exists() {
            return;
        }
        struct Feed(Arc<Mutex<TermView>>);
        impl PtyEvents for Feed {
            fn on_output(&self, _: SessionId, bytes: &[u8]) {
                self.0.lock().unwrap().feed(bytes);
            }
            fn on_exit(&self, _: SessionId) {}
        }
        // pasta com caminho bem mais largo que o terminal (30 colunas)
        let dir = std::env::temp_dir().join(format!("oxiterm-um-caminho-de-pasta-bem-comprido-{}", std::process::id())).join("e-mais-um-nivel");
        std::fs::create_dir_all(&dir).unwrap();
        let term = Arc::new(Mutex::new(TermView::new(30, 24)));
        let mgr = PtyManager::new();
        let id = mgr
            .spawn(
                SpawnConfig { shell: Some("/bin/bash".into()), cols: 30, rows: 24, cwd: Some(dir.clone()), prompt: Some(PromptConfig { text: "❯".into(), color: None }), ..Default::default() },
                Arc::new(Feed(term.clone())),
            )
            .unwrap();
        let wait = |n: usize| {
            let t0 = Instant::now();
            loop {
                let m = term.lock().unwrap().blocks("❯");
                if m.blocks.len() >= n && m.idle {
                    return m;
                }
                assert!(t0.elapsed() < Duration::from_secs(10), "timeout esperando {n} prompts");
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        wait(1);
        mgr.run_command(id, "echo ok").unwrap();
        let m = wait(2);
        let t = term.lock().unwrap();
        let b = &m.blocks[0];
        assert_eq!(b.command, "echo ok");
        assert!(b.info.path.ends_with("e-mais-um-nivel"), "caminho inteiro, juntando as linhas: {}", b.info.path);
        assert_eq!(t.lines_text(b.out_start, t.last_content_line(b.out_start, b.out_end) + 1), "ok");
        assert!(!b.running);
        drop(t);
        let _ = mgr.kill(id);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn end_to_end_real_bash_produces_blocks_with_status() {
        use ox_pty::{PromptConfig, PtyEvents, PtyManager, SessionId, SpawnConfig};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        if !std::path::Path::new("/bin/bash").exists() {
            return;
        }
        struct Feed(Arc<Mutex<TermView>>);
        impl PtyEvents for Feed {
            fn on_output(&self, _: SessionId, bytes: &[u8]) {
                self.0.lock().unwrap().feed(bytes);
            }
            fn on_exit(&self, _: SessionId) {}
        }

        let term = Arc::new(Mutex::new(TermView::new(80, 24)));
        let mgr = PtyManager::new();
        let id = mgr
            .spawn(
                SpawnConfig { shell: Some("/bin/bash".into()), cols: 80, rows: 24, prompt: Some(PromptConfig { text: "❯".into(), color: None }), ..Default::default() },
                Arc::new(Feed(term.clone())),
            )
            .unwrap();
        // espera até haver `n` prompts e o último estar aguardando digitação
        let wait = |n: usize| {
            let t0 = Instant::now();
            loop {
                let m = term.lock().unwrap().blocks("❯");
                if m.blocks.len() >= n && m.idle {
                    return m;
                }
                assert!(t0.elapsed() < Duration::from_secs(10), "timeout esperando {n} prompts");
                std::thread::sleep(Duration::from_millis(50));
            }
        };

        wait(1);
        mgr.run_command(id, "echo hello").unwrap();
        wait(2);
        mgr.run_command(id, "false").unwrap();
        mgr.run_command(id, "printf 'a\\nb\\n'").unwrap();
        let m = wait(4);
        let t = term.lock().unwrap();
        let out = |i: usize| t.lines_text(m.blocks[i].out_start, t.last_content_line(m.blocks[i].out_start, m.blocks[i].out_end) + 1);

        // o status mostrado de um bloco vem do prompt SEGUINTE; o do primeiro prompt não é usado
        assert_eq!((m.blocks[0].command.as_str(), out(0).as_str(), m.blocks[0].status), ("echo hello", "hello", Some(0)));
        assert_eq!((m.blocks[1].command.as_str(), out(1).as_str(), m.blocks[1].status), ("false", "", Some(1)));
        assert_eq!((m.blocks[2].command.as_str(), out(2).as_str(), m.blocks[2].status), ("printf 'a\\nb\\n'", "a\nb", Some(0)));
        assert!(m.blocks[0].info.time.as_deref().is_some_and(|t| t.len() == 5), "hora HH:MM no prompt");
        assert!(m.idle && !m.blocks[3].running);
        drop(t);
        mgr.kill(id).unwrap();
    }

    #[test]
    fn palette_256_cube_and_gray() {
        assert_eq!(indexed(16), 0x000000);
        assert_eq!(indexed(231), 0xffffff);
        assert_eq!(indexed(232), 0x080808);
    }
}
