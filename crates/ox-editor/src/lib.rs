//! Buffer de texto multilinha para o editor de notas: cursor, seleção, desfazer/refazer,
//! continuação automática de listas Markdown e quebra visual de linhas.
//!
//! Colunas são contadas em `char` (assume largura 1 por caractere, fonte monoespaçada).

const UNDO_LIMIT: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// Trecho visual de uma linha lógica, depois da quebra por largura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub start: usize,
    /// exclusivo
    pub end: usize,
}

/// Tipos de bloco do menu "/" (estilo Warp Drive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Text,
    Heading(u8),
    Bullet,
    Numbered,
    Todo,
    Quote,
    Divider,
    /// bloco de código cercado; `lang` vazio = sem linguagem
    Code(&'static str),
}

#[derive(Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Pos,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    None,
    TypeChar,
    Delete,
}

pub struct TextBuffer {
    lines: Vec<String>,
    cursor: Pos,
    anchor: Option<Pos>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: Kind,
    version: u64,
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn byte_at(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(i, _)| i)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextBuffer {
    pub fn new(text: &str) -> Self {
        Self::from_normalized(text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    "))
    }

    /// Para arquivos de código: mantém as tabulações (um `Makefile` não pode virar espaços).
    pub fn new_preserving_tabs(text: &str) -> Self {
        Self::from_normalized(text.replace("\r\n", "\n").replace('\r', "\n"))
    }

    fn from_normalized(text: String) -> Self {
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self { lines, cursor: Pos::default(), anchor: None, undo: Vec::new(), redo: Vec::new(), last: Kind::None, version: 0 }
    }

    // ----- leitura -----

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, i: usize) -> &str {
        &self.lines[i]
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    /// Incrementa a cada modificação do texto.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Seleção ordenada (início, fim), se houver.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some(if a < self.cursor { (a, self.cursor) } else { (self.cursor, a) })
    }

    pub fn selected_text(&self) -> Option<String> {
        let (s, e) = self.selection()?;
        if s.line == e.line {
            return Some(self.lines[s.line].chars().skip(s.col).take(e.col - s.col).collect());
        }
        let mut out: Vec<String> = vec![self.lines[s.line].chars().skip(s.col).collect()];
        out.extend(self.lines[s.line + 1..e.line].iter().cloned());
        out.push(self.lines[e.line].chars().take(e.col).collect());
        Some(out.join("\n"))
    }

    // ----- histórico -----

    fn snapshot(&self) -> Snapshot {
        Snapshot { lines: self.lines.clone(), cursor: self.cursor }
    }

    /// Chamar antes de qualquer modificação. Digitação e apagamento contínuos viram um só passo.
    fn begin_edit(&mut self, kind: Kind) {
        if kind == Kind::None || kind != self.last || self.anchor.is_some() {
            self.undo.push(self.snapshot());
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = kind;
        self.version += 1;
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(self.snapshot());
        self.restore(prev);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(self.snapshot());
        self.restore(next);
        true
    }

    fn restore(&mut self, s: Snapshot) {
        self.lines = s.lines;
        self.cursor = s.cursor;
        self.anchor = None;
        self.last = Kind::None;
        self.version += 1;
    }

    // ----- edição -----

    fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        Pos { line, col: p.col.min(char_len(&self.lines[line])) }
    }

    fn delete_range(&mut self, s: Pos, e: Pos) {
        let head: String = self.lines[s.line].chars().take(s.col).collect();
        let tail: String = self.lines[e.line].chars().skip(e.col).collect();
        self.lines.splice(s.line..=e.line, [head + &tail]);
        self.cursor = s;
        self.anchor = None;
    }

    fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some((s, e)) => {
                self.delete_range(s, e);
                true
            }
            None => {
                self.anchor = None;
                false
            }
        }
    }

    fn insert_raw(&mut self, text: &str) {
        let Pos { line, col } = self.cursor;
        let at = byte_at(&self.lines[line], col);
        let tail = self.lines[line].split_off(at);
        let mut parts = text.split('\n');
        self.lines[line].push_str(parts.next().unwrap_or(""));
        let mut cur = line;
        for part in parts {
            cur += 1;
            self.lines.insert(cur, part.to_string());
        }
        let new_col = char_len(&self.lines[cur]);
        self.lines[cur].push_str(&tail);
        self.cursor = Pos { line: cur, col: new_col };
    }

    /// Insere texto (aceita `\n`), substituindo a seleção. Usado também para colar.
    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
        let kind = if text.chars().count() == 1 && text != "\n" { Kind::TypeChar } else { Kind::None };
        self.begin_edit(kind);
        self.delete_selection();
        self.insert_raw(&text);
    }

    /// Enter: mantém a indentação e continua listas (`- `, `* `, `1. `, `- [ ] `).
    /// Enter numa linha que só tem o marcador remove o marcador.
    pub fn insert_newline(&mut self) {
        self.begin_edit(Kind::None);
        self.delete_selection();
        let Pos { line, col } = self.cursor;
        let before: String = self.lines[line].chars().take(col).collect();
        let indent: String = before.chars().take_while(|c| *c == ' ').collect();
        let rest = &before[indent.len()..];

        let marker = list_marker(rest);
        if let Some(m) = &marker {
            if rest.trim_end() == m.trim_end() && col == char_len(&self.lines[line]) {
                // linha vazia de lista: sai da lista
                self.lines[line].clear();
                self.cursor = Pos { line, col: 0 };
                return;
            }
        }
        let prefix = format!("{indent}{}", marker.map(|m| next_marker(&m)).unwrap_or_default());
        self.insert_raw(&format!("\n{prefix}"));
    }

    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.begin_edit(Kind::None);
            self.delete_selection();
            return;
        }
        let Pos { line, col } = self.cursor;
        if line == 0 && col == 0 {
            return;
        }
        self.begin_edit(Kind::Delete);
        if col == 0 {
            let cur = self.lines.remove(line);
            let new_col = char_len(&self.lines[line - 1]);
            self.lines[line - 1].push_str(&cur);
            self.cursor = Pos { line: line - 1, col: new_col };
        } else {
            let s = &mut self.lines[line];
            let (a, b) = (byte_at(s, col - 1), byte_at(s, col));
            s.replace_range(a..b, "");
            self.cursor.col -= 1;
        }
    }

    pub fn delete(&mut self) {
        if self.selection().is_some() {
            self.begin_edit(Kind::None);
            self.delete_selection();
            return;
        }
        let Pos { line, col } = self.cursor;
        let len = char_len(&self.lines[line]);
        if line + 1 == self.lines.len() && col == len {
            return;
        }
        self.begin_edit(Kind::Delete);
        if col == len {
            let next = self.lines.remove(line + 1);
            self.lines[line].push_str(&next);
        } else {
            let s = &mut self.lines[line];
            let (a, b) = (byte_at(s, col), byte_at(s, col + 1));
            s.replace_range(a..b, "");
        }
    }

    /// Ctrl+Backspace.
    pub fn delete_word_back(&mut self) {
        if self.selection().is_some() || self.cursor.col == 0 {
            return self.backspace();
        }
        let end = self.cursor;
        self.anchor = Some(end);
        self.move_word_left(true);
        self.begin_edit(Kind::None);
        self.delete_selection();
    }

    /// Cercar a seleção (ou inserir o par vazio com o cursor no meio): `**negrito**`, `` `código` ``.
    pub fn wrap_selection(&mut self, prefix: &str, suffix: &str) {
        self.begin_edit(Kind::None);
        let selected = self.selected_text().unwrap_or_default();
        self.delete_selection();
        self.insert_raw(&format!("{prefix}{selected}{suffix}"));
        if selected.is_empty() {
            self.cursor.col -= char_len(suffix);
        }
    }

    /// Troca o prefixo de bloco da linha atual (`# `, `## `, `- `, `> `...). Repetir o mesmo prefixo remove.
    pub fn toggle_line_prefix(&mut self, prefix: &str) {
        self.begin_edit(Kind::None);
        let line = self.cursor.line;
        let text = &self.lines[line];
        let stripped = strip_block_prefix(text);
        let new = if text.starts_with(prefix) { stripped.to_string() } else { format!("{prefix}{stripped}") };
        let delta = char_len(&new) as isize - char_len(text) as isize;
        self.lines[line] = new;
        self.cursor.col = (self.cursor.col as isize + delta).max(0) as usize;
        self.anchor = None;
    }

    /// Converte a linha atual no bloco escolhido (descarta o texto dela, p.ex. o "/cmd" digitado).
    pub fn apply_block(&mut self, kind: BlockKind) {
        self.begin_edit(Kind::None);
        self.anchor = None;
        let line = self.cursor.line;
        let set = |b: &mut Self, text: String| {
            b.cursor = Pos::new(line, char_len(&text));
            b.lines[line] = text;
        };
        match kind {
            BlockKind::Text => set(self, String::new()),
            BlockKind::Heading(n) => set(self, format!("{} ", "#".repeat(n.clamp(1, 6) as usize))),
            BlockKind::Bullet => set(self, "- ".into()),
            BlockKind::Numbered => set(self, "1. ".into()),
            BlockKind::Todo => set(self, "- [ ] ".into()),
            BlockKind::Quote => set(self, "> ".into()),
            BlockKind::Divider => {
                set(self, "---".into());
                self.lines.insert(line + 1, String::new());
                self.cursor = Pos::new(line + 1, 0);
            }
            BlockKind::Code(lang) => {
                set(self, format!("```{lang}"));
                self.lines.insert(line + 1, String::new());
                self.lines.insert(line + 2, "```".into());
                self.cursor = Pos::new(line + 1, 0);
            }
        }
    }

    /// Remove as linhas `from..=to` (ex.: um bloco de código inteiro), com desfazer. O cursor vai
    /// para o fim da linha anterior (ou o começo do documento). Sempre sobra pelo menos uma linha.
    pub fn remove_lines(&mut self, from: usize, to: usize) {
        if from > to || to >= self.lines.len() {
            return;
        }
        self.begin_edit(Kind::None);
        self.anchor = None;
        self.lines.drain(from..=to);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor = if from == 0 { Pos::new(0, 0) } else { Pos::new(from - 1, char_len(&self.lines[from - 1])) };
    }

    /// Troca o texto de uma linha inteira (ex.: a linguagem na abertura de um bloco de código), com desfazer.
    pub fn set_line(&mut self, i: usize, text: &str) {
        if i >= self.lines.len() || self.lines[i] == text {
            return;
        }
        self.begin_edit(Kind::None);
        self.anchor = None;
        self.lines[i] = text.to_string();
        self.cursor = self.clamp(self.cursor);
    }

    /// Insere um bloco de código cercado; o cursor fica dentro dele.
    pub fn insert_code_block(&mut self, lang: &str) {
        self.begin_edit(Kind::None);
        self.delete_selection();
        let leading = if self.lines[self.cursor.line].is_empty() { "" } else { "\n" };
        self.insert_raw(&format!("{leading}```{lang}\n\n```"));
        self.cursor = Pos { line: self.cursor.line - 1, col: 0 };
    }

    /// Tab: recuo de 2 espaços (na seleção, em todas as linhas). Shift+Tab remove até 2 espaços.
    pub fn indent(&mut self, outdent: bool) {
        self.begin_edit(Kind::None);
        let (first, last) = match self.selection() {
            Some((s, e)) => (s.line, e.line),
            None => (self.cursor.line, self.cursor.line),
        };
        if !outdent && self.selection().is_none() {
            self.insert_raw("  ");
            return;
        }
        for l in first..=last {
            if outdent {
                let n = self.lines[l].chars().take(2).take_while(|c| *c == ' ').count();
                self.lines[l].replace_range(..n, "");
                if l == self.cursor.line {
                    self.cursor.col = self.cursor.col.saturating_sub(n);
                }
            } else {
                self.lines[l].insert_str(0, "  ");
                if l == self.cursor.line {
                    self.cursor.col += 2;
                }
            }
        }
        self.anchor = None;
    }

    // ----- movimento -----

    fn set_cursor(&mut self, p: Pos, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = self.clamp(p);
        self.last = Kind::None;
    }

    pub fn move_to(&mut self, p: Pos, extend: bool) {
        self.set_cursor(p, extend);
    }

    pub fn move_left(&mut self, extend: bool) {
        // sem Shift, a seta leva ao início da seleção em vez de andar mais um caractere
        if !extend {
            if let Some((s, _)) = self.selection() {
                return self.set_cursor(s, false);
            }
        }
        let Pos { line, col } = self.cursor;
        let p = if col > 0 {
            Pos::new(line, col - 1)
        } else if line > 0 {
            Pos::new(line - 1, char_len(&self.lines[line - 1]))
        } else {
            self.cursor
        };
        self.set_cursor(p, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend {
            if let Some((_, e)) = self.selection() {
                return self.set_cursor(e, false);
            }
        }
        let Pos { line, col } = self.cursor;
        let p = if col < char_len(&self.lines[line]) {
            Pos::new(line, col + 1)
        } else if line + 1 < self.lines.len() {
            Pos::new(line + 1, 0)
        } else {
            self.cursor
        };
        self.set_cursor(p, extend);
    }

    pub fn move_up(&mut self, n: usize, extend: bool) {
        let Pos { line, col } = self.cursor;
        let p = if line == 0 { Pos::new(0, 0) } else { Pos::new(line.saturating_sub(n), col) };
        self.set_cursor(p, extend);
    }

    pub fn move_down(&mut self, n: usize, extend: bool) {
        let Pos { line, col } = self.cursor;
        let last = self.lines.len() - 1;
        let p = if line == last { Pos::new(last, char_len(&self.lines[last])) } else { Pos::new((line + n).min(last), col) };
        self.set_cursor(p, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        // Home alterna entre o início do texto (após a indentação) e a coluna 0
        let line = self.cursor.line;
        let indent = self.lines[line].chars().take_while(|c| *c == ' ').count();
        let col = if self.cursor.col == indent { 0 } else { indent };
        self.set_cursor(Pos::new(line, col), extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        let line = self.cursor.line;
        self.set_cursor(Pos::new(line, char_len(&self.lines[line])), extend);
    }

    pub fn move_doc_start(&mut self, extend: bool) {
        self.set_cursor(Pos::new(0, 0), extend);
    }

    pub fn move_doc_end(&mut self, extend: bool) {
        let last = self.lines.len() - 1;
        self.set_cursor(Pos::new(last, char_len(&self.lines[last])), extend);
    }

    pub fn move_word_left(&mut self, extend: bool) {
        let Pos { line, col } = self.cursor;
        if col == 0 {
            return self.move_left(extend);
        }
        let chars: Vec<char> = self.lines[line].chars().collect();
        let mut i = col;
        while i > 0 && !is_word(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(chars[i - 1]) {
            i -= 1;
        }
        self.set_cursor(Pos::new(line, i), extend);
    }

    pub fn move_word_right(&mut self, extend: bool) {
        let Pos { line, col } = self.cursor;
        let chars: Vec<char> = self.lines[line].chars().collect();
        if col >= chars.len() {
            return self.move_right(extend);
        }
        let mut i = col;
        while i < chars.len() && !is_word(chars[i]) {
            i += 1;
        }
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        self.set_cursor(Pos::new(line, i), extend);
    }

    /// Duplo clique: seleciona a palavra (ou o trecho de símbolos/espaços) sob `p`.
    pub fn select_word_at(&mut self, p: Pos) {
        let p = self.clamp(p);
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        if chars.is_empty() {
            return self.set_cursor(p, false);
        }
        let i = p.col.min(chars.len() - 1);
        let class = |c: char| if is_word(c) { 0 } else if c == ' ' { 1 } else { 2 };
        let k = class(chars[i]);
        let (mut a, mut b) = (i, i + 1);
        while a > 0 && class(chars[a - 1]) == k {
            a -= 1;
        }
        while b < chars.len() && class(chars[b]) == k {
            b += 1;
        }
        self.anchor = Some(Pos::new(p.line, a));
        self.cursor = Pos::new(p.line, b);
        self.last = Kind::None;
    }

    /// Triplo clique: seleciona a linha inteira.
    pub fn select_line(&mut self, line: usize) {
        let line = line.min(self.lines.len() - 1);
        self.anchor = Some(Pos::new(line, 0));
        self.cursor = Pos::new(line, char_len(&self.lines[line]));
        self.last = Kind::None;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Pos::new(0, 0));
        let last = self.lines.len() - 1;
        self.cursor = Pos::new(last, char_len(&self.lines[last]));
        self.last = Kind::None;
    }

    // ----- layout visual -----

    /// Quebra cada linha em trechos de no máximo `width` colunas, preferindo quebrar em espaços.
    pub fn layout(&self, width: usize) -> Vec<Row> {
        self.layout_with(|_| width)
    }

    /// Como `layout`, mas a largura (em colunas) pode variar por linha lógica, p.ex. títulos com fonte maior.
    pub fn layout_with(&self, width_of: impl Fn(usize) -> usize) -> Vec<Row> {
        let mut rows = Vec::with_capacity(self.lines.len());
        for (line, text) in self.lines.iter().enumerate() {
            let width = width_of(line).max(4);
            let chars: Vec<char> = text.chars().collect();
            let mut start = 0;
            loop {
                if chars.len() - start <= width {
                    rows.push(Row { line, start, end: chars.len() });
                    break;
                }
                let limit = start + width;
                // quebra depois do último espaço dentro do limite; senão, corte duro
                let end = chars[start..=limit].iter().rposition(|c| *c == ' ').map(|i| start + i + 1).filter(|e| *e > start).unwrap_or(limit);
                rows.push(Row { line, start, end });
                start = end;
            }
        }
        rows
    }

    /// Índice da linha visual que contém o cursor.
    pub fn cursor_row(&self, rows: &[Row]) -> usize {
        let Pos { line, col } = self.cursor;
        let mut found = 0;
        for (i, r) in rows.iter().enumerate() {
            if r.line != line {
                continue;
            }
            found = i;
            if col < r.end {
                break;
            }
        }
        found
    }
}

/// Marcador de lista no início de `s` (já sem indentação), incluindo o espaço final.
fn list_marker(s: &str) -> Option<String> {
    for m in ["- [ ] ", "- [x] ", "- ", "* ", "+ "] {
        if s.starts_with(m) {
            return Some(m.to_string());
        }
    }
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() && s[digits.len()..].starts_with(". ") {
        return Some(format!("{digits}. "));
    }
    None
}

fn next_marker(m: &str) -> String {
    if m.starts_with("- [") {
        return "- [ ] ".into();
    }
    match m.trim_end_matches(". ").parse::<u64>() {
        Ok(n) if m.ends_with(". ") => format!("{}. ", n + 1),
        _ => m.to_string(),
    }
}

fn strip_block_prefix(s: &str) -> &str {
    let t = s.trim_start_matches('#');
    let t = if t.len() != s.len() { t.trim_start() } else { s };
    for m in ["- [ ] ", "- [x] ", "- ", "* ", "> "] {
        if let Some(r) = t.strip_prefix(m) {
            return r;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> TextBuffer {
        TextBuffer::new(s)
    }

    #[test]
    fn insert_and_newline_split_lines() {
        let mut b = buf("olá mundo");
        b.move_to(Pos::new(0, 3), false);
        b.insert_newline();
        assert_eq!(b.text(), "olá\n mundo");
        assert_eq!(b.cursor(), Pos::new(1, 0));
        b.insert_str("x\ny");
        assert_eq!(b.text(), "olá\nx\ny mundo");
        assert_eq!(b.cursor(), Pos::new(2, 1));
    }

    #[test]
    fn backspace_and_delete_join_lines_and_handle_unicode() {
        let mut b = buf("aé\nb");
        b.move_to(Pos::new(1, 0), false);
        b.backspace();
        assert_eq!(b.text(), "aéb");
        assert_eq!(b.cursor(), Pos::new(0, 2));
        b.backspace();
        assert_eq!(b.text(), "ab"); // removeu 'é' inteiro, sem cortar o UTF-8
        b.move_to(Pos::new(0, 0), false);
        b.delete();
        assert_eq!(b.text(), "b");
    }

    #[test]
    fn selection_replace_copy_and_multiline_delete() {
        let mut b = buf("um\ndois\ntrês");
        b.move_to(Pos::new(0, 1), false);
        b.move_to(Pos::new(2, 2), true);
        assert_eq!(b.selected_text().unwrap(), "m\ndois\ntr");
        b.insert_str("X");
        assert_eq!(b.text(), "uXês");
        b.select_all();
        assert_eq!(b.selected_text().unwrap(), "uXês");
        b.backspace();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn undo_redo_coalesce_typing() {
        let mut b = buf("");
        for c in "abc".chars() {
            b.insert_str(&c.to_string());
        }
        b.insert_newline();
        b.insert_str("d");
        assert!(b.undo()); // desfaz "d"
        assert_eq!(b.text(), "abc\n");
        assert!(b.undo()); // desfaz o Enter
        assert_eq!(b.text(), "abc");
        assert!(b.undo()); // "abc" inteiro de uma vez
        assert_eq!(b.text(), "");
        assert!(!b.undo());
        assert!(b.redo());
        assert_eq!(b.text(), "abc");
    }

    #[test]
    fn newline_continues_lists_and_exits_on_empty_item() {
        let mut b = buf("- item");
        b.move_doc_end(false);
        b.insert_newline();
        assert_eq!(b.text(), "- item\n- ");
        b.insert_newline(); // item vazio sai da lista
        assert_eq!(b.text(), "- item\n");

        let mut b = buf("  1. um");
        b.move_doc_end(false);
        b.insert_newline();
        assert_eq!(b.text(), "  1. um\n  2. ");

        let mut b = buf("- [x] feito");
        b.move_doc_end(false);
        b.insert_newline();
        assert_eq!(b.text(), "- [x] feito\n- [ ] ");
    }

    #[test]
    fn word_movement_and_delete_word() {
        let mut b = buf("foo bar_baz  qux");
        b.move_doc_end(false);
        b.move_word_left(false);
        assert_eq!(b.cursor().col, 13);
        b.move_word_left(false);
        assert_eq!(b.cursor().col, 4);
        b.move_word_right(false);
        assert_eq!(b.cursor().col, 11);
        b.move_doc_end(false);
        b.delete_word_back();
        assert_eq!(b.text(), "foo bar_baz  ");
    }

    #[test]
    fn wrap_selection_and_block_helpers() {
        let mut b = buf("negrito");
        b.select_all();
        b.wrap_selection("**", "**");
        assert_eq!(b.text(), "**negrito**");
        let mut b = buf("");
        b.wrap_selection("`", "`");
        assert_eq!(b.text(), "``");
        assert_eq!(b.cursor().col, 1);

        let mut b = buf("título");
        b.toggle_line_prefix("## ");
        assert_eq!(b.text(), "## título");
        b.toggle_line_prefix("# ");
        assert_eq!(b.text(), "# título");
        b.toggle_line_prefix("# ");
        assert_eq!(b.text(), "título");

        let mut b = buf("antes");
        b.move_doc_end(false);
        b.insert_code_block("bash");
        assert_eq!(b.text(), "antes\n```bash\n\n```");
        assert_eq!(b.cursor(), Pos::new(2, 0));
    }

    #[test]
    fn indent_outdent() {
        let mut b = buf("a\nb");
        b.select_all();
        b.indent(false);
        assert_eq!(b.text(), "  a\n  b");
        b.select_all();
        b.indent(true);
        assert_eq!(b.text(), "a\nb");
    }

    #[test]
    fn layout_wraps_at_spaces_and_hard_breaks_long_words() {
        let b = buf("uma frase bem longa aqui\n\nxxxxxxxxxxxx");
        let rows = b.layout(10);
        let slices: Vec<String> = rows.iter().map(|r| b.line(r.line).chars().skip(r.start).take(r.end - r.start).collect()).collect();
        assert_eq!(slices, ["uma frase ", "bem longa ", "aqui", "", "xxxxxxxxxx", "xx"]);
        // cursor no fim da linha 0 fica na última linha visual dela
        let mut b2 = buf("uma frase bem longa aqui");
        b2.move_doc_end(false);
        assert_eq!(b2.cursor_row(&b2.layout(10)), 2);
        // cursor exatamente na quebra pertence à linha visual seguinte
        b2.move_to(Pos::new(0, 10), false);
        assert_eq!(b2.cursor_row(&b2.layout(10)), 1);
    }

    #[test]
    fn word_and_line_selection() {
        let mut b = buf("foo bar_baz  --");
        b.select_word_at(Pos::new(0, 6));
        assert_eq!(b.selected_text().unwrap(), "bar_baz");
        b.select_word_at(Pos::new(0, 3));
        assert_eq!(b.selected_text().unwrap(), " ");
        b.select_line(0);
        assert_eq!(b.selected_text().unwrap(), "foo bar_baz  --");
    }

    #[test]
    fn remove_lines_drops_a_block_and_undoes() {
        let mut b = buf("a\n```\nx\n```\nb");
        b.remove_lines(1, 3);
        assert_eq!(b.text(), "a\nb");
        assert_eq!(b.cursor(), Pos::new(0, 1));
        assert!(b.undo());
        assert_eq!(b.text(), "a\n```\nx\n```\nb");
        let mut b = buf("```\n```");
        b.remove_lines(0, 1);
        assert_eq!(b.text(), "");
    }

    #[test]
    fn set_line_replaces_and_undoes() {
        let mut b = buf("```\nls\n```");
        b.move_doc_end(false);
        b.set_line(0, "```bash");
        assert_eq!(b.text(), "```bash\nls\n```");
        assert!(b.undo());
        assert_eq!(b.text(), "```\nls\n```");
        b.set_line(9, "x"); // fora do texto: nada muda
        assert_eq!(b.line_count(), 3);
    }

    #[test]
    fn apply_block_converts_current_line() {
        let mut b = buf("antes\n/tit");
        b.move_doc_end(false);
        b.apply_block(BlockKind::Heading(2));
        assert_eq!(b.text(), "antes\n## ");
        assert_eq!(b.cursor(), Pos::new(1, 3));
        b.apply_block(BlockKind::Todo);
        assert_eq!(b.text(), "antes\n- [ ] ");
        b.apply_block(BlockKind::Text);
        assert_eq!(b.text(), "antes\n");

        let mut b = buf("/cmd");
        b.move_doc_end(false);
        b.apply_block(BlockKind::Code("bash"));
        assert_eq!(b.text(), "```bash\n\n```");
        assert_eq!(b.cursor(), Pos::new(1, 0)); // cursor dentro do bloco

        let mut b = buf("/div");
        b.move_doc_end(false);
        b.apply_block(BlockKind::Divider);
        assert_eq!(b.text(), "---\n");
        assert_eq!(b.cursor(), Pos::new(1, 0));
        b.undo();
        assert_eq!(b.text(), "/div");
    }

    #[test]
    fn layout_with_per_line_widths() {
        let b = buf("aaaa bbbb\ncccc dddd");
        let rows = b.layout_with(|l| if l == 0 { 5 } else { 20 });
        let per_line: Vec<usize> = (0..2).map(|l| rows.iter().filter(|r| r.line == l).count()).collect();
        assert_eq!(per_line, [2, 1]);
    }

    #[test]
    fn preserving_tabs_keeps_them_for_code_files() {
        let b = TextBuffer::new_preserving_tabs("all:\r\n\tcc main.c");
        assert_eq!(b.text(), "all:\n\tcc main.c");
        assert_eq!(b.line(1).chars().next(), Some('\t'));
    }

    #[test]
    fn crlf_and_tabs_are_normalized() {
        assert_eq!(buf("a\r\nb\tc").text(), "a\nb    c");
    }
}
