//! Painel da nota: editor com estilo ao vivo (títulos maiores, marcadores esmaecidos, blocos de código
//! em cartão com linguagem, copiar e executar), menu "/" de blocos e referências a notas, e modo leitura.
//!
//! O texto continua sendo Markdown puro num `ox_editor::TextBuffer`. Toda a fonte do editor é
//! monoespaçada; títulos só escalam o tamanho da fonte, então colunas × largura continuam exatas.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    div, img, prelude::*, px, rgb, AnyElement, ClickEvent, ClipboardItem, Context, Font, FontFeatures, FontStyle, FontWeight, Keystroke, MouseButton, SharedString,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollHandle, ScrollWheelEvent, StyledText, TextRun, TextStyle, Window,
};
use ox_editor::{BlockKind, Pos, Row, TextBuffer};
use ox_markdown::{is_runnable, Block, CalloutKind, Inline};

use crate::syntax::{self, Lang};
use crate::theme::*;
use crate::icons::{icon_el, wicon, Icon};
use crate::scrollbar::{scroll_list, thumb, thumb_geometry};
use crate::{hsla, primary_btn, sh_quote, shorten, small_btn, Workspace, UI_FONT};

pub const ED_FONT: f32 = 15.0;
pub const NOTES_W_WIDE: f32 = 780.0;
const AUTOSAVE_MS: u64 = 800;
/// margem à esquerda do texto, onde fica o botão "+" da linha vazia
const GUTTER: f32 = 30.0;
/// largura máxima da coluna de conteúdo (como o editor de notebooks do Warp) e margem do título/detalhes
const CONTENT_MAX_W: f32 = 640.0;
const CONTENT_PAD: f32 = 16.0;
/// o título é 1,4 vez maior que o texto do corpo
const TITLE_SCALE: f32 = 1.4;
const EMPTY_HINT: &str = "Digite texto ou Markdown, ou '/' para inserir conteúdo";

type BufOp = fn(&mut TextBuffer);

/// Barra flutuante sobre a seleção (id, rótulo, ação). Os grupos são separados antes de "om-b" e "om-list".
const OMNIBAR: &[(&str, &str, BufOp)] = &[
    ("om-h1", "H1", |b| b.toggle_line_prefix("# ")),
    ("om-h2", "H2", |b| b.toggle_line_prefix("## ")),
    ("om-b", "B", |b| b.wrap_selection("**", "**")),
    ("om-i", "I", |b| b.wrap_selection("*", "*")),
    ("om-s", "S", |b| b.wrap_selection("~~", "~~")),
    ("om-code", "<>", |b| b.wrap_selection("`", "`")),
    ("om-link", "↗", |b| b.wrap_selection("[", "](url)")),
    ("om-list", "•", |b| b.toggle_line_prefix("- ")),
    ("om-todo", "☐", |b| b.toggle_line_prefix("- [ ] ")),
    ("om-quote", "❝", |b| b.toggle_line_prefix("> ")),
];
/// recuo do texto dentro do cartão de código
const CARD_PAD: f32 = 14.0;
/// altura da borda superior do cartão (cerca escondida) e do rodapé com linguagem e ações
const CARD_TOP_H: f32 = 14.0;
const CARD_FOOT_H: f32 = 48.0;
/// espaço livre acima e abaixo de cada cartão, para dois blocos seguidos não ficarem grudados
const CARD_GAP: f32 = 10.0;
/// linguagens oferecidas no seletor do cartão: (rótulo, texto depois de ```)
const LANG_OPTIONS: &[(&str, &str)] = &[("Code", ""), ("Shell", "bash"), ("Python", "python"), ("Rust", "rust"), ("JavaScript", "js"), ("JSON", "json")];

/// Botão de ícone do rodapé do cartão de código (copiar, executar).
fn card_btn(id: impl Into<gpui::ElementId>, icon: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(28.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(BTN_HOVER)))
        .child(wicon(icon, 16., TEXT))
}

/// Rótulo da linguagem de um bloco de código, como no seletor do Warp.
fn lang_label(lang: &str) -> String {
    match lang.to_lowercase().as_str() {
        "" => "Code".into(),
        "bash" | "sh" | "shell" | "zsh" | "fish" | "console" => "Shell".into(),
        "py" | "python" => "Python".into(),
        "rs" | "rust" => "Rust".into(),
        "js" | "javascript" => "JavaScript".into(),
        "ts" | "typescript" => "TypeScript".into(),
        "json" => "JSON".into(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CardRole {
    Open,
    Body,
    Close,
}

/// Parte de um cartão de código: a linha de abertura (```lang) e, se houver, a de fechamento.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Card {
    open: usize,
    close: Option<usize>,
    role: CardRole,
}

/// Cartão fechado (com ``` de abertura e de fechamento) a que a linha pertence.
fn closed_card(lines: &[String], line: usize) -> Option<Card> {
    let (kinds, _) = classify(lines);
    cards(&kinds).get(line).copied().flatten().filter(|c| c.close.is_some())
}

/// As cercas ``` de um bloco fechado ficam escondidas: se o cursor parar numa delas, ele passa para o
/// lado de onde veio (`down` = estava indo para baixo). Depois do último bloco cria uma linha vazia,
/// para sempre dar para continuar escrevendo.
fn skip_hidden_fence(buf: &mut TextBuffer, down: bool, extend: bool) {
    let c = buf.cursor();
    let Some(card) = closed_card(buf.lines(), c.line).filter(|k| k.role != CardRole::Body) else { return };
    let close = card.close.unwrap_or(card.open);
    let n = buf.line_count();
    let end_of = |b: &TextBuffer, l: usize| Pos::new(l, b.line(l).chars().count());
    let target = match (card.role, down) {
        // abertura: descendo entra no bloco; subindo vai para o fim da linha de cima
        (CardRole::Open, true) => Pos::new(card.open + 1, 0),
        (CardRole::Open, false) if card.open > 0 => end_of(buf, card.open - 1),
        (CardRole::Open, false) => Pos::new(card.open + 1, 0),
        // fechamento: subindo volta para o fim do bloco; descendo sai dele
        (_, false) => end_of(buf, close - 1),
        (_, true) if close + 1 < n => Pos::new(close + 1, 0),
        (_, true) => {
            buf.move_to(end_of(buf, close), false);
            buf.insert_newline();
            return;
        }
    };
    buf.move_to(target, extend);
}

/// Se a linha do cursor acabou de virar exatamente ``` (e ainda não é um bloco fechado), troca por
/// um bloco de código vazio com o cursor dentro.
fn turn_backticks_into_card(buf: &mut TextBuffer) {
    let c = buf.cursor();
    if buf.line(c.line).trim() != "```" || closed_card(buf.lines(), c.line).is_some() {
        return;
    }
    buf.apply_block(BlockKind::Code(""));
}

/// Desfaz um bloco: tira as cercas e deixa o conteúdo como texto, com o cursor no começo dele.
fn unwrap_card(buf: &mut TextBuffer, open: usize, close: usize) {
    buf.remove_lines(close, close);
    buf.remove_lines(open, open);
    buf.move_to(Pos::new(open, 0), false);
}

/// O cursor está no fim de uma abertura ```lang que não tem fechamento?
fn fence_needs_close(lines: &[String], cursor: Pos) -> bool {
    let line = &lines[cursor.line];
    if !line.trim_start().starts_with("```") || cursor.col != line.chars().count() {
        return false;
    }
    let (kinds, _) = classify(lines);
    cards(&kinds)[cursor.line].is_some_and(|c| c.role == CardRole::Open && c.close.is_none())
}

/// Para cada linha, a que cartão de código ela pertence (`None` fora de blocos).
fn cards(kinds: &[LineKind]) -> Vec<Option<Card>> {
    let mut out = vec![None; kinds.len()];
    let mut i = 0;
    while i < kinds.len() {
        if kinds[i] != LineKind::Fence {
            i += 1;
            continue;
        }
        let open = i;
        let mut j = i + 1;
        while j < kinds.len() && kinds[j] == LineKind::Code {
            j += 1;
        }
        let close = (j < kinds.len() && kinds[j] == LineKind::Fence).then_some(j);
        out[open] = Some(Card { open, close, role: CardRole::Open });
        for k in open + 1..j {
            out[k] = Some(Card { open, close, role: CardRole::Body });
        }
        if let Some(c) = close {
            out[c] = Some(Card { open, close, role: CardRole::Close });
        }
        i = close.map_or(j, |c| c + 1);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NoteMode {
    View,
    Edit,
}

struct Slash {
    line: usize,
    sel: usize,
    /// primeira opção visível (a lista mostra SLASH_VISIBLE por vez) e total de opções no último desenho
    first: usize,
    count: usize,
    /// rolagem do mouse acumulada (trackpads mandam frações de linha)
    wheel: f32,
}

impl Slash {
    fn at(line: usize) -> Self {
        Slash { line, sel: 0, first: 0, count: 0, wheel: 0.0 }
    }

    /// Mantém a opção selecionada dentro da janela visível.
    fn follow_sel(&mut self) {
        if self.sel < self.first {
            self.first = self.sel;
        } else if self.sel >= self.first + SLASH_VISIBLE {
            self.first = self.sel + 1 - SLASH_VISIBLE;
        }
        self.first = self.first.min(self.count.saturating_sub(SLASH_VISIBLE));
    }
}

/// opções visíveis de uma vez no menu "/" e altura de cada uma
const SLASH_VISIBLE: usize = 7;
const SLASH_ROW_H: f32 = 46.0;
const SLASH_SEARCH_H: f32 = 38.0;

pub struct NoteEditor {
    pub buf: TextBuffer,
    /// caminho relativo da nota (vazio quando é um arquivo)
    pub name: String,
    /// arquivo aberto do disco (código, texto...): salva com Ctrl+S, sem salvamento automático
    pub file: Option<PathBuf>,
    /// o arquivo usava fim de linha CRLF: é mantido ao salvar
    crlf: bool,
    pub saved_version: u64,
    pub scroll: ScrollHandle,
    /// rolar até o cursor no próximo desenho
    pub reveal: bool,
    pub drag: bool,
    slash: Option<Slash>,
    /// seletor de linguagem aberto no cartão cuja abertura é esta linha
    lang_menu: Option<usize>,
}

impl NoteEditor {
    pub fn new(name: &str, text: &str) -> Self {
        let buf = TextBuffer::new(text);
        Self { saved_version: buf.version(), buf, name: name.to_string(), file: None, crlf: false, scroll: ScrollHandle::new(), reveal: false, drag: false, slash: None, lang_menu: None }
    }

    /// Editor para um arquivo do disco (preserva tabulações e CRLF).
    pub fn for_file(path: PathBuf, text: &str) -> Self {
        let buf = TextBuffer::new_preserving_tabs(text);
        Self { saved_version: buf.version(), buf, name: String::new(), crlf: text.contains("\r\n"), file: Some(path), scroll: ScrollHandle::new(), reveal: false, drag: false, slash: None, lang_menu: None }
    }

    /// Arquivo que não é Markdown: sem estilos de nota, com números de linha.
    pub fn is_code(&self) -> bool {
        self.file.as_deref().is_some_and(|p| !is_markdown_path(p))
    }

    fn dirty(&self) -> bool {
        self.buf.version() != self.saved_version
    }

    /// Texto digitado depois do "/" na linha do cursor, enquanto o menu estiver aberto.
    fn slash_query(&self) -> Option<String> {
        let s = self.slash.as_ref()?;
        let c = self.buf.cursor();
        if c.line != s.line {
            return None;
        }
        self.buf.line(c.line).strip_prefix('/').filter(|q| !q.contains(' ')).map(String::from)
    }
}

pub fn is_markdown_path(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(), Some("md" | "markdown"))
}

/// Comando para rodar um arquivo no terminal (num subshell, na pasta do arquivo), se o tipo for conhecido.
pub fn run_command_for(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    let runner = match ext.as_str() {
        "py" => "python3",
        "sh" | "bash" => "bash",
        "zsh" => "zsh",
        "fish" => "fish",
        "js" | "mjs" | "cjs" => "node",
        "ts" => "npx tsx",
        "rb" => "ruby",
        "pl" => "perl",
        "php" => "php",
        "lua" => "lua",
        "go" => "go run",
        "java" => "java",
        "r" => "Rscript",
        _ => return None,
    };
    let dir = path.parent()?.to_string_lossy().into_owned();
    let file = path.file_name()?.to_string_lossy().into_owned();
    Some(format!("(cd {} && {runner} {})", sh_quote(&dir), sh_quote(&file)))
}

/// Quebra uma linha em trechos que cabem em `max_w` px, preferindo quebrar depois de espaços.
/// `xs[i]` = posição x do início do caractere i (xs tem len+1 posições).
pub fn wrap_by_width(chars: &[char], xs: &[f32], max_w: f32) -> Vec<(usize, usize)> {
    let n = chars.len();
    if n == 0 {
        return vec![(0, 0)];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < n {
        if xs[n] - xs[start] <= max_w {
            out.push((start, n));
            break;
        }
        let mut end = start + 1;
        while end < n && xs[end + 1] - xs[start] <= max_w {
            end += 1;
        }
        if let Some(sp) = (start + 1..=end).rev().find(|&k| chars[k - 1] == ' ') {
            end = sp;
        }
        out.push((start, end));
        start = end;
    }
    out
}

/// Índice do limite entre caracteres mais próximo de `x` dentro de [lo, hi].
fn nearest_boundary(xs: &[f32], lo: usize, hi: usize, x: f32) -> usize {
    let mut best = lo;
    for k in lo..=hi.min(xs.len() - 1) {
        if (xs[k] - x).abs() < (xs[best] - x).abs() {
            best = k;
        }
    }
    best
}

// ---------- menu "/" ----------

struct SlashItem {
    /// ícone svg (assets/icons) ou, sem ele, um glifo curto
    icon: Option<&'static str>,
    glyph: &'static str,
    label: &'static str,
    keys: &'static str,
    kind: BlockKind,
}

const SLASH_ITEMS: &[SlashItem] = &[
    SlashItem { icon: Some("terminal"), glyph: "›_", label: "Comando", keys: "command comando bash shell run executar", kind: BlockKind::Code("bash") },
    SlashItem { icon: Some("code-01"), glyph: "<>", label: "Código", keys: "code codigo", kind: BlockKind::Code("") },
    SlashItem { icon: None, glyph: "H1", label: "Título 1", keys: "heading titulo h1", kind: BlockKind::Heading(1) },
    SlashItem { icon: None, glyph: "H2", label: "Título 2", keys: "heading titulo h2", kind: BlockKind::Heading(2) },
    SlashItem { icon: None, glyph: "H3", label: "Título 3", keys: "heading titulo h3", kind: BlockKind::Heading(3) },
    SlashItem { icon: None, glyph: "H4", label: "Título 4", keys: "heading titulo h4", kind: BlockKind::Heading(4) },
    SlashItem { icon: None, glyph: "H5", label: "Título 5", keys: "heading titulo h5", kind: BlockKind::Heading(5) },
    SlashItem { icon: None, glyph: "H6", label: "Título 6", keys: "heading titulo h6", kind: BlockKind::Heading(6) },
    SlashItem { icon: Some("block-text"), glyph: "T", label: "Texto", keys: "text texto paragrafo", kind: BlockKind::Text },
    SlashItem { icon: Some("block-bulletedlist"), glyph: "•", label: "Lista", keys: "bullet list lista marcadores", kind: BlockKind::Bullet },
    SlashItem { icon: Some("block-ordered-list"), glyph: "1.", label: "Lista numerada", keys: "numbered numerada list lista", kind: BlockKind::Numbered },
    SlashItem { icon: Some("block-tasklist"), glyph: "☑", label: "Tarefas", keys: "todo task tarefa checklist", kind: BlockKind::Todo },
    SlashItem { icon: None, glyph: "❝", label: "Citação", keys: "quote citacao", kind: BlockKind::Quote },
    SlashItem { icon: None, glyph: "—", label: "Divisor", keys: "divider divisor linha hr", kind: BlockKind::Divider },
];

fn slash_matches(query: &str) -> Vec<&'static SlashItem> {
    let q = query.to_lowercase();
    SLASH_ITEMS.iter().filter(|i| q.is_empty() || i.label.to_lowercase().contains(&q) || i.keys.contains(&q)).collect()
}

/// Uma opção do menu "/": um tipo de bloco ou uma referência a outra nota.
#[derive(Clone)]
enum SlashChoice {
    Block(&'static SlashItem),
    Note { rel: String, label: String, dir: String },
}

/// Blocos que combinam com a busca e, depois deles, até 8 notas pelo nome (`notes` = caminhos relativos).
fn slash_choices(query: &str, notes: &[String]) -> Vec<SlashChoice> {
    let q = query.to_lowercase();
    let mut out: Vec<SlashChoice> = slash_matches(query).into_iter().map(SlashChoice::Block).collect();
    out.extend(
        notes
            .iter()
            .filter_map(|rel| {
                let stem = rel.trim_end_matches(".md");
                let (dir, label) = stem.rsplit_once('/').unwrap_or(("", stem));
                (q.is_empty() || label.to_lowercase().contains(&q)).then(|| SlashChoice::Note { rel: rel.clone(), label: label.to_string(), dir: dir.to_string() })
            })
            .take(8),
    );
    out
}

/// Aplica a opção na linha do "/": converte o bloco ou troca a linha por um link para a nota.
fn apply_choice(buf: &mut TextBuffer, choice: &SlashChoice) {
    match choice {
        SlashChoice::Block(it) => buf.apply_block(it.kind),
        SlashChoice::Note { rel, label, .. } => {
            buf.apply_block(BlockKind::Text);
            buf.insert_str(&format!("[{label}]({rel})"));
        }
    }
}

enum SlashOutcome {
    NotHandled,
    Consumed,
    Applied,
}

fn handle_slash(ed: &mut NoteEditor, key: &str, notes: &[String]) -> SlashOutcome {
    if ed.slash.is_none() {
        return SlashOutcome::NotHandled;
    }
    let items = slash_choices(&ed.slash_query().unwrap_or_default(), notes);
    let n = items.len();
    let sel = ed.slash.as_ref().map_or(0, |s| s.sel).min(n.saturating_sub(1));
    match key {
        "escape" => {
            ed.slash = None;
            SlashOutcome::Consumed
        }
        "up" if n > 0 => {
            ed.slash.as_mut().unwrap().sel = (sel + n - 1) % n;
            SlashOutcome::Consumed
        }
        "down" if n > 0 => {
            ed.slash.as_mut().unwrap().sel = (sel + 1) % n;
            SlashOutcome::Consumed
        }
        "enter" | "tab" if n > 0 => {
            ed.slash = None;
            apply_choice(&mut ed.buf, &items[sel]);
            SlashOutcome::Applied
        }
        _ => SlashOutcome::NotHandled,
    }
}

// ---------- classificação de linhas ----------

#[derive(Debug, Clone, Copy, PartialEq)]
enum LineKind {
    Normal,
    Heading(u8),
    Fence,
    Code,
    Quote,
    Divider,
}

fn scale_of(k: LineKind) -> f32 {
    match k {
        LineKind::Heading(1) => 1.7,
        LineKind::Heading(2) => 1.45,
        LineKind::Heading(3) => 1.25,
        LineKind::Heading(4) => 1.12,
        LineKind::Heading(_) => 1.05,
        _ => 1.0,
    }
}

/// Linhas lógicas classificadas + blocos de código executáveis (linha de abertura -> (lang, código)).
fn classify(lines: &[String]) -> (Vec<LineKind>, HashMap<usize, String>) {
    let mut kinds = Vec::with_capacity(lines.len());
    let mut runnable = HashMap::new();
    let mut open: Option<(usize, String)> = None;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if t.starts_with("```") {
            match open.take() {
                Some((start, lang)) => {
                    if is_runnable(&lang) {
                        runnable.insert(start, lines[start + 1..i].join("\n"));
                    }
                }
                None => open = Some((i, t.trim_start_matches('`').split_whitespace().next().unwrap_or("").to_lowercase())),
            }
            kinds.push(LineKind::Fence);
        } else if open.is_some() {
            kinds.push(LineKind::Code);
        } else if t.starts_with('#') {
            let level = t.chars().take_while(|c| *c == '#').count().min(6) as u8;
            kinds.push(LineKind::Heading(level));
        } else if t.starts_with('>') {
            kinds.push(LineKind::Quote);
        } else if matches!(t, "---" | "***" | "___") {
            kinds.push(LineKind::Divider);
        } else {
            kinds.push(LineKind::Normal);
        }
    }
    (kinds, runnable)
}

/// Faixa de colunas do marcador (`# `, `> `, `- `, `1. `, `- [ ] `) que deve ser esmaecida/colorida.
fn marker_range(kind: LineKind, line: &str) -> Option<(usize, usize)> {
    let indent = line.chars().take_while(|c| *c == ' ').count();
    let rest = &line[indent..];
    let len = match kind {
        LineKind::Heading(_) => {
            let h = rest.chars().take_while(|c| *c == '#').count();
            h + usize::from(rest.chars().nth(h) == Some(' '))
        }
        LineKind::Quote => 1 + usize::from(rest.starts_with("> ")),
        LineKind::Normal => {
            if rest.starts_with("- [ ] ") || rest.starts_with("- [x] ") {
                6
            } else if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
                2
            } else {
                let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
                if digits > 0 && rest[digits..].starts_with(". ") {
                    digits + 2
                } else {
                    0
                }
            }
        }
        _ => 0,
    };
    (len > 0).then_some((indent, indent + len))
}

/// Estilo de cada caractere de uma linha de texto da nota, para o editor mostrar a formatação
/// enquanto você escreve: os símbolos (`**`, `*`, `~~`, `` ` ``, `[ ](url)`) ficam esmaecidos e o
/// conteúdo entre eles aparece em negrito, itálico, riscado, código ou link.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct CharStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: bool,
    /// símbolo de formatação (esmaecido)
    mark: bool,
}

fn inline_styles(chars: &[char]) -> Vec<CharStyle> {
    let n = chars.len();
    let mut st = vec![CharStyle::default(); n];
    // ocupado = já é símbolo ou código (outras marcas não valem ali dentro)
    let busy = |st: &Vec<CharStyle>, i: usize| st[i].mark || st[i].code;
    // `código`
    let mut i = 0;
    while i < n {
        if chars[i] == '`' {
            if let Some(j) = (i + 1..n).find(|&j| chars[j] == '`') {
                if j > i + 1 {
                    st[i].mark = true;
                    st[j].mark = true;
                    for s in &mut st[i + 1..j] {
                        s.code = true;
                    }
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    // [texto](url)
    let mut i = 0;
    while i < n {
        if chars[i] == '[' && !busy(&st, i) {
            if let Some(close) = (i + 1..n).find(|&j| chars[j] == ']') {
                if close + 1 < n && chars[close + 1] == '(' {
                    if let Some(end) = (close + 2..n).find(|&j| chars[j] == ')') {
                        st[i].mark = true;
                        for s in &mut st[i + 1..close] {
                            s.link = true;
                        }
                        for s in &mut st[close..=end] {
                            s.mark = true;
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    // pares de símbolos de 2 caracteres (**negrito**, ~~riscado~~) e de 1 (*itálico*, _itálico_)
    let pair = |st: &mut Vec<CharStyle>, open: &[char], set: fn(&mut CharStyle)| {
        let w = open.len();
        let at = |st: &Vec<CharStyle>, i: usize| i + w <= n && chars[i..i + w] == *open && (i..i + w).all(|k| !st[k].mark && !st[k].code);
        let mut i = 0;
        while i < n {
            if at(st, i) {
                // "_" só vale no começo de palavra (snake_case continua texto normal)
                let word_ok = open != ['_'] || i == 0 || !chars[i - 1].is_alphanumeric();
                if word_ok {
                    if let Some(j) = (i + w + 1..n).find(|&j| at(st, j)) {
                        for k in i..i + w {
                            st[k].mark = true;
                        }
                        for k in j..j + w {
                            st[k].mark = true;
                        }
                        for s in &mut st[i + w..j] {
                            set(s);
                        }
                        i = j + w;
                        continue;
                    }
                }
            }
            i += 1;
        }
    };
    pair(&mut st, &['*', '*'], |s| s.bold = true);
    pair(&mut st, &['_', '_'], |s| s.bold = true);
    pair(&mut st, &['~', '~'], |s| s.strike = true);
    pair(&mut st, &['*'], |s| s.italic = true);
    pair(&mut st, &['_'], |s| s.italic = true);
    st
}

/// Pedaço de linha desenhado com um mesmo estilo.
struct Seg {
    text: String,
    color: u32,
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    bg: Option<u32>,
}

impl Seg {
    fn plain(text: String, color: u32) -> Self {
        Seg { text, color, bold: false, italic: false, strike: false, underline: false, bg: None }
    }
}

/// Divide o trecho `[start, end)` da linha em pedaços (texto, é_marcador).
fn split_segments(chars: &[char], start: usize, end: usize, marker: Option<(usize, usize)>) -> Vec<(String, bool)> {
    let mut cuts = vec![start, end];
    if let Some((a, b)) = marker {
        cuts.extend([a, b].into_iter().filter(|c| *c > start && *c < end));
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .map(|w| (chars[w[0]..w[1]].iter().collect(), marker.is_some_and(|(a, b)| w[0] >= a && w[1] <= b)))
        .collect()
}

impl Workspace {
    pub fn notes_w(&self) -> f32 {
        match (self.show_notes, self.notes_wide) {
            (false, _) => 0.0,
            (true, false) => self.notes_px,
            (true, true) => NOTES_W_WIDE.max(self.notes_px),
        }
    }

    /// Título da nota aberta: o nome do arquivo, sem pasta nem `.md`.
    pub fn note_title(&self) -> String {
        if let Some(path) = self.editor.as_ref().and_then(|e| e.file.as_ref()).or(self.image_view.as_ref()) {
            return path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        }
        self.note_current.as_deref().map(|r| r.rsplit('/').next().unwrap_or(r).trim_end_matches(".md").to_string()).unwrap_or_default()
    }

    /// Teclas no campo de título. Devolve `false` para deixar passar os atalhos globais.
    pub fn on_title_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        if k.modifiers.control || k.modifiers.alt {
            return false;
        }
        match k.key.to_lowercase().as_str() {
            "enter" | "tab" => self.commit_title(true),
            "escape" => self.title_focused = false,
            "backspace" => {
                self.title_text.pop();
            }
            "space" => self.title_text.push(' '),
            _ => {
                if let Some(c) = k.key_char.as_deref().filter(|c| !c.contains('/')) {
                    self.title_text.push_str(c);
                }
            }
        }
        cx.notify();
        true
    }

    /// Aplica o título digitado: renomeia o arquivo (o título é o nome da nota).
    fn commit_title(&mut self, then_edit: bool) {
        self.title_focused = false;
        let new = self.title_text.trim().to_string();
        if let Some(old) = self.editor.as_ref().and_then(|e| e.file.clone()) {
            if !new.is_empty() && !new.contains('/') && Some(new.as_str()) != old.file_name().and_then(|n| n.to_str()) {
                let to = old.with_file_name(&new);
                if !to.exists() {
                    self.save_note();
                    if std::fs::rename(&old, &to).is_ok() {
                        if let Some(ed) = self.editor.as_mut() {
                            ed.file = Some(to);
                        }
                    }
                }
            }
        } else if let Some(old) = self.note_current.clone() {
            let stem = self.note_title();
            if !new.is_empty() && new != stem {
                self.save_note();
                if let Ok(new_rel) = self.notes.rename(&old, &new) {
                    if let Some(ed) = self.editor.as_mut() {
                        ed.name = new_rel.clone();
                    }
                    self.note_current = Some(new_rel);
                }
            }
        }
        if then_edit && self.editor.is_some() {
            self.set_note_mode(NoteMode::Edit);
        }
    }

    // ----- ciclo de vida -----

    pub fn save_note(&mut self) {
        if let Some(ed) = self.editor.as_mut() {
            if !ed.dirty() {
                return;
            }
            let text = if ed.crlf { ed.buf.text().replace('\n', "\r\n") } else { ed.buf.text() };
            let ok = match &ed.file {
                Some(path) => std::fs::write(path, text).is_ok(),
                None => self.notes.write(&ed.name, &text).is_ok(),
            };
            if ok {
                ed.saved_version = ed.buf.version();
            }
        }
    }

    /// Abre um arquivo do disco no painel, como uma nota: texto/código no editor, imagem no visualizador.
    pub fn open_file(&mut self, path: PathBuf) {
        self.save_note();
        self.show_notes = true;
        self.image_view = None;
        self.file_notice = None;
        self.note_current = None;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico") {
            self.editor = None;
            self.image_view = Some(path);
            return;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > 2 * 1024 * 1024 {
            self.editor = None;
            self.file_notice = Some(format!("{name}: grande demais para abrir no editor ({} KB).", size / 1024));
            return;
        }
        match std::fs::read(&path).map(String::from_utf8) {
            Ok(Ok(text)) if !text.contains('\0') => {
                self.editor = Some(NoteEditor::for_file(path, &text));
                self.note_blocks.clear();
                self.note_mode = NoteMode::Edit;
                self.focus_editor = true;
            }
            Ok(_) => {
                self.editor = None;
                self.file_notice = Some(format!("{name}: arquivo binário, sem pré-visualização."));
            }
            Err(e) => {
                self.editor = None;
                self.file_notice = Some(format!("{name}: {e}"));
            }
        }
    }

    pub fn set_note_mode(&mut self, mode: NoteMode) {
        self.note_mode = mode;
        match mode {
            NoteMode::View => {
                self.save_note();
                self.focus_editor = false;
                if let Some(ed) = self.editor.as_mut() {
                    ed.slash = None;
                }
                if let Some(ed) = &self.editor {
                    self.note_blocks = ox_markdown::parse(&ed.buf.text());
                }
            }
            NoteMode::Edit => {
                self.focus_editor = true;
                if let Some(ed) = self.editor.as_mut() {
                    ed.reveal = true;
                }
            }
        }
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(ed) = self.editor.as_mut() {
            ed.reveal = true;
            if ed.file.is_some() {
                return; // arquivos: só Ctrl+S / botão Salvar
            }
        }
        let version = self.editor.as_ref().map(|e| e.buf.version());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(AUTOSAVE_MS)).await;
            let _ = this.update(cx, |ws, cx| {
                if ws.editor.as_ref().map(|e| e.buf.version()) == version {
                    ws.save_note();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Notas que o menu "/" pode referenciar (todas menos a aberta), as recentes primeiro.
    fn reference_notes(&self) -> Vec<String> {
        let current = self.note_current.clone().unwrap_or_default();
        self.notes.list().unwrap_or_default().into_iter().map(|n| n.name).filter(|n| *n != current).collect()
    }

    /// Troca a linguagem do cartão de código (reescreve a linha de abertura ```lang).
    fn set_card_lang(&mut self, open: usize, lang: &str, cx: &mut Context<Self>) {
        self.focus_editor = true;
        if let Some(ed) = self.editor.as_mut() {
            ed.lang_menu = None;
            let indent: String = ed.buf.line(open).chars().take_while(|c| *c == ' ').collect();
            ed.buf.set_line(open, &format!("{indent}```{lang}"));
        }
        self.after_edit(cx);
        cx.notify();
    }

    fn apply_slash(&mut self, choice: SlashChoice, cx: &mut Context<Self>) {
        self.focus_editor = true;
        if let Some(ed) = self.editor.as_mut() {
            ed.slash = None;
            apply_choice(&mut ed.buf, &choice);
        }
        self.after_edit(cx);
        cx.notify();
    }

    /// Aplica uma operação do buffer (barra flutuante) mantendo o foco no editor.
    fn edit_with(&mut self, f: impl FnOnce(&mut TextBuffer), cx: &mut Context<Self>) {
        self.focus_editor = true;
        let changed = self.editor.as_mut().is_some_and(|ed| {
            let before = ed.buf.version();
            f(&mut ed.buf);
            ed.buf.version() != before
        });
        if changed {
            self.after_edit(cx);
        }
        cx.notify();
    }

    /// Botão "+" da linha vazia: insere "/" e abre o menu de blocos.
    fn editor_plus(&mut self, line: usize, cx: &mut Context<Self>) {
        self.focus_editor = true;
        if let Some(ed) = self.editor.as_mut() {
            ed.buf.move_to(Pos::new(line, 0), false);
            ed.buf.insert_str("/");
            ed.slash = Some(Slash::at(line));
            ed.reveal = true;
        }
        self.after_edit(cx);
        cx.notify();
    }

    fn editor_click(&mut self, pos: Pos, clicks: usize, shift: bool, cx: &mut Context<Self>) {
        self.focus_editor = true;
        if let Some(ed) = self.editor.as_mut() {
            ed.drag = true;
            ed.reveal = false;
            ed.slash = None;
            ed.lang_menu = None;
            match clicks {
                2 => ed.buf.select_word_at(pos),
                3.. => ed.buf.select_line(pos.line),
                _ => ed.buf.move_to(pos, shift),
            }
            // clique numa cerca escondida: a de cima leva para o começo do bloco, a de baixo para o fim
            if !ed.is_code() {
                let down = closed_card(ed.buf.lines(), pos.line).is_some_and(|c| c.role == CardRole::Open);
                skip_hidden_fence(&mut ed.buf, down, shift);
            }
        }
        cx.notify();
    }

    fn editor_drag(&mut self, pos: Pos, cx: &mut Context<Self>) {
        if let Some(ed) = self.editor.as_mut().filter(|e| e.drag) {
            ed.buf.move_to(pos, true);
            cx.notify();
        }
    }

    // ----- teclado -----

    /// Devolve `true` se a tecla foi consumida pelo editor.
    pub fn on_editor_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) -> bool {
        let m = &k.modifiers;
        let key = k.key.to_lowercase();
        let (ctrl, shift) = (m.control, m.shift);

        // atalhos globais do app continuam valendo (abas, zoom, painéis)
        if ctrl && shift && key != "z" {
            return false;
        }
        if ctrl && matches!(key.as_str(), "=" | "+" | "-" | "0") {
            return false;
        }
        let slash_open = self.editor.as_ref().is_some_and(|e| e.slash.is_some());
        // Esc fecha primeiro o seletor de linguagem; só depois sai do modo edição
        if key == "escape" && self.editor.as_mut().and_then(|e| e.lang_menu.take()).is_some() {
            cx.notify();
            return true;
        }
        if key == "escape" && !slash_open {
            self.set_note_mode(NoteMode::View);
            cx.notify();
            return true;
        }
        if ctrl && key == "s" {
            self.save_note();
            cx.notify();
            return true;
        }
        let notes = if slash_open { self.reference_notes() } else { Vec::new() };
        let Some(ed) = self.editor.as_mut() else { return false };
        let before = ed.buf.version();
        ed.reveal = true;

        let outcome = handle_slash(ed, &key, &notes);
        if matches!(outcome, SlashOutcome::Consumed) {
            cx.notify();
            return true;
        }

        if matches!(outcome, SlashOutcome::NotHandled) {
            let auto_close = !ed.is_code() && fence_needs_close(ed.buf.lines(), ed.buf.cursor());
            let start = ed.buf.cursor();
            // Backspace no começo de um bloco: num bloco vazio apaga o bloco; com texto, desfaz o bloco e
            // o texto vira texto normal (as cercas ``` nunca ficam à mostra nem se juntam ao código)
            if !ed.is_code() && key == "backspace" && !ctrl && start.col == 0 && ed.buf.selection().is_none() {
                if let Some(card) = closed_card(ed.buf.lines(), start.line).filter(|c| c.role == CardRole::Body && c.open + 1 == start.line) {
                    let close = card.close.unwrap_or(start.line);
                    if close == start.line + 1 && ed.buf.line(start.line).is_empty() {
                        ed.buf.remove_lines(card.open, close);
                    } else {
                        unwrap_card(&mut ed.buf, card.open, close);
                    }
                    self.after_edit(cx);
                    cx.notify();
                    return true;
                }
            }
            // Delete no fim do bloco não puxa a cerca de fechamento
            if !ed.is_code() && key == "delete" && ed.buf.selection().is_none() && start.col == ed.buf.line(start.line).chars().count() {
                if closed_card(ed.buf.lines(), start.line).is_some_and(|c| c.role == CardRole::Body && c.close == Some(start.line + 1)) {
                    cx.notify();
                    return true;
                }
            }
            let b = &mut ed.buf;
            match key.as_str() {
                "left" if ctrl => b.move_word_left(shift),
                "left" => b.move_left(shift),
                "right" if ctrl => b.move_word_right(shift),
                "right" => b.move_right(shift),
                "up" => b.move_up(1, shift),
                "down" => b.move_down(1, shift),
                "pageup" => b.move_up(12, shift),
                "pagedown" => b.move_down(12, shift),
                "home" if ctrl => b.move_doc_start(shift),
                "home" => b.move_home(shift),
                "end" if ctrl => b.move_doc_end(shift),
                "end" => b.move_end(shift),
                "enter" if auto_close => {
                    // "```lang" + Enter sem cerca de fechamento: cria o fechamento e entra no bloco
                    b.insert_newline();
                    b.insert_str("\n```");
                    b.move_up(1, false);
                    b.move_end(false);
                }
                "enter" => b.insert_newline(),
                "backspace" if ctrl => b.delete_word_back(),
                "backspace" => b.backspace(),
                "delete" => b.delete(),
                "tab" => b.indent(shift),
                "space" if !ctrl => b.insert_str(" "),
                "a" if ctrl => b.select_all(),
                "z" if ctrl && shift => {
                    b.redo();
                }
                "z" if ctrl => {
                    b.undo();
                }
                "y" if ctrl => {
                    b.redo();
                }
                "b" if ctrl => b.wrap_selection("**", "**"),
                "i" if ctrl => b.wrap_selection("*", "*"),
                "c" if ctrl => {
                    if let Some(t) = b.selected_text() {
                        cx.write_to_clipboard(ClipboardItem::new_string(t));
                    }
                }
                "x" if ctrl => {
                    if let Some(t) = b.selected_text() {
                        cx.write_to_clipboard(ClipboardItem::new_string(t));
                        b.backspace();
                    }
                }
                "v" if ctrl => {
                    if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        b.insert_str(&t);
                    }
                }
                _ if !ctrl && !m.platform => {
                    if let Some(c) = &k.key_char {
                        b.insert_str(c);
                    }
                }
                _ => {}
            }

            // digitar ``` numa linha vira na hora um bloco de código (as cercas ficam escondidas; a
            // linguagem se escolhe no seletor do bloco)
            if !ed.is_code() && k.key_char.as_deref() == Some("`") {
                turn_backticks_into_card(&mut ed.buf);
            }
            // o cursor não para em cercas ``` escondidas: segue para o lado para onde ia
            if !ed.is_code() {
                let now = ed.buf.cursor();
                let down = now.line > start.line || (now.line == start.line && now.col >= start.col);
                skip_hidden_fence(&mut ed.buf, down, shift);
            }

            // "/" numa linha vazia abre o menu; o menu fecha se a linha deixar de ser um "/comando"
            let c = ed.buf.cursor();
            if !ctrl && !ed.is_code() && k.key_char.as_deref() == Some("/") && ed.buf.line(c.line) == "/" {
                ed.slash = Some(Slash::at(c.line));
            }
            if ed.slash.is_some() && ed.slash_query().is_none() {
                ed.slash = None;
            }
        }

        if ed.buf.version() != before {
            self.after_edit(cx);
        }
        cx.notify();
        true
    }

    // ----- desenho -----

    pub fn render_notes(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let base = window.text_style();
        let mode = self.note_mode;
        let font = self.icon_font.clone();
        let note_title = self.note_title();
        let file_path = self.editor.as_ref().and_then(|e| e.file.clone());
        let status = match self.editor.as_ref() {
            Some(e) if e.dirty() => (if e.file.is_some() { "● não salvo" } else { "● alterações" }, YELLOW),
            Some(_) => ("✓ salvo", GREEN),
            None => ("", MUTED),
        };

        // cabeçalho do painel: título centralizado e botões à direita; sem divisória
        let header = div()
            .h(px(40.))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(div().max_w(px(260.)).text_sm().text_color(rgb(MUTED)).truncate().child(if note_title.is_empty() { "Notas".to_string() } else { note_title.clone() }))
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().text_xs().text_color(rgb(status.1)).child(status.0))
                    .child(small_btn("notes-wide", if self.notes_wide { "⇥" } else { "⇤" }).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.notes_wide = !this.notes_wide;
                        cx.notify();
                    })))
                    .child(small_btn("notes-close", "✕").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.show_notes = false;
                        this.focus_editor = false;
                        cx.notify();
                    }))),
            );

        // título: campo separado, acima do conteúdo, em negrito e 1,4× maior (renomeia nota/arquivo)
        let focused = self.title_focused;
        let shown = if focused { self.title_text.clone() } else { note_title.clone() };
        let title = div()
            .id("note-title")
            .flex_none()
            .px(px(CONTENT_PAD))
            .pb(px(12.))
            .cursor_pointer()
            .text_size(px(ED_FONT * TITLE_SCALE))
            .font_weight(FontWeight::BOLD)
            .child(if shown.is_empty() {
                div().text_color(rgb(MUTED)).child(format!("Sem título{}", if focused { "▏" } else { "" }))
            } else {
                div().text_color(rgb(TEXT)).truncate().child(format!("{shown}{}", if focused { "▏" } else { "" }))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.title_text = this.note_title();
                    this.title_focused = true;
                    this.focus_editor = false;
                    cx.stop_propagation(); // o contêiner raiz tiraria o foco do título
                    cx.notify();
                }),
            );

        let column = |details: AnyElement, body: AnyElement| {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .justify_center()
                .child(div().w_full().max_w(px(CONTENT_MAX_W)).h_full().flex().flex_col().child(details).child(body))
                .into_any_element()
        };
        let details_row = || div().flex_none().flex().items_center().justify_between().gap_2().px(px(CONTENT_PAD)).pt(px(CONTENT_PAD)).pb(px(4.));

        let content: AnyElement = if let Some(path) = self.image_view.clone() {
            // visualizador de imagem
            let crumbs = path.parent().map(|d| shorten(d)).unwrap_or_default();
            column(
                div().flex().flex_col().child(details_row().child(div().min_w_0().text_xs().text_color(rgb(MUTED)).truncate().child(crumbs))).child(title).into_any_element(),
                div().id("image-view").flex_1().min_h_0().overflow_y_scroll().px(px(CONTENT_PAD)).child(img(path).max_w_full()).into_any_element(),
            )
        } else if let Some(msg) = self.file_notice.clone() {
            div().flex_1().flex().items_center().justify_center().p_4().text_sm().text_color(rgb(MUTED)).child(msg).into_any_element()
        } else if self.editor.is_none() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(rgb(MUTED))
                .child("Abra uma nota ou um arquivo pela barra lateral.")
                .into_any_element()
        } else if let Some(path) = file_path {
            // arquivo do disco: caminho, Rodar (se o tipo for conhecido), Salvar, desfazer/refazer
            let crumbs = path.parent().map(|d| shorten(d)).unwrap_or_default();
            let run = run_command_for(&path);
            let tools = div()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .when_some(run, |d, cmd| {
                    d.child(primary_btn("file-run", "▶ Rodar").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.save_note();
                        this.run_in_terminal(&cmd);
                        cx.notify();
                    })))
                })
                .child(small_btn("file-save", "Salvar").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.save_note();
                    cx.notify();
                })))
                .child(small_btn("file-undo", "↶").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.edit_with(|b| {
                        b.undo();
                    }, cx)
                })))
                .child(small_btn("file-redo", "↷").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.edit_with(|b| {
                        b.redo();
                    }, cx)
                })));
            let details = details_row().child(div().min_w_0().text_xs().text_color(rgb(MUTED)).truncate().child(crumbs)).child(tools);
            let editor = self.render_editor(window, cx);
            column(div().flex().flex_col().child(details).child(title).into_any_element(), editor)
        } else {
            // nota: caminho à esquerda; "Editando/Visualizando" + lápis à direita (como o Warp)
            let crumbs = {
                let mut parts = vec!["Notas".to_string()];
                if let Some(rel) = self.note_current.as_deref() {
                    parts.extend(rel.rsplit_once('/').map(|(d, _)| d.split('/').map(String::from).collect::<Vec<_>>()).unwrap_or_default());
                }
                parts.join("  ›  ")
            };
            let editing = mode == NoteMode::Edit;
            let pencil = Icon { svg: Some("icons/pencil-02.svg"), glyph: '\u{f040}', color: if editing { ACCENT } else { MUTED }, fallback: "✎".into() };
            let details = details_row().child(div().min_w_0().text_xs().text_color(rgb(MUTED)).truncate().child(crumbs)).child(
                div()
                    .id("mode-toggle")
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .child(div().text_xs().text_color(rgb(MUTED)).child(if editing { "Editando" } else { "Visualizando" }))
                    .child(icon_el(&font, &pencil, 13.))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let next = if this.note_mode == NoteMode::Edit { NoteMode::View } else { NoteMode::Edit };
                        this.set_note_mode(next);
                        cx.notify();
                    })),
            );
            let body: AnyElement = if editing {
                self.render_editor(window, cx)
            } else {
                let blocks: Vec<AnyElement> = self.note_blocks.iter().enumerate().map(|(i, b)| self.render_block(i, b, &base, cx)).collect();
                scroll_list(div().id("note-body").px(px(CONTENT_PAD)).py(px(4.)).flex().flex_col().gap_2().children(blocks), &self.note_scroll).into_any_element()
            };
            column(div().flex().flex_col().child(details).child(title).into_any_element(), body)
        };

        div()
            .w(px(self.notes_w()))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .when(!self.show_sidebar, |d| d.rounded_bl(px(self.corner_r)))
            .border_r_1()
            .border_color(rgb(DIVIDER))
            .child(header)
            .child(content)
            .into_any_element()
    }

    fn render_editor(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let (code, n_lines) = match self.editor.as_ref() {
            Some(e) => (e.is_code(), e.buf.line_count()),
            None => return div().into_any_element(),
        };
        let panel_w = self.notes_w();
        let content_w = panel_w.min(CONTENT_MAX_W);
        let cw = self.ed_cell_w;
        // margem esquerda: botão "+" nas notas; números de linha nos arquivos de código
        let gutter = if code { n_lines.to_string().len().max(2) as f32 * cw * 0.85 + 18.0 } else { GUTTER };
        let text_w = (content_w - CONTENT_PAD * 2.0 - gutter - 8.0).max(40.0);
        let x0 = self.sidebar_w() + 1.0 + (panel_w - content_w) / 2.0 + CONTENT_PAD + gutter;
        let focused = self.focus_editor;
        let caret_on = self.caret_on;
        let mono = self.mono.clone();
        let ui_font: SharedString = UI_FONT.into();
        let slash_open = self.editor.as_ref().is_some_and(|e| e.slash.is_some());
        let ref_notes = if slash_open && !code { self.reference_notes() } else { Vec::new() };
        let ed = self.editor.as_mut().expect("editor aberto");

        let (kinds, runnable) = if code { (vec![LineKind::Normal; n_lines], HashMap::new()) } else { classify(ed.buf.lines()) };
        // blocos de código das notas viram cartões; as cercas ``` somem quando o cursor não está nelas
        let card_of = if code { vec![None; n_lines] } else { cards(&kinds) };
        let cur_line = ed.buf.cursor().line;
        // cercas sempre escondidas; só a abertura que você está digitando (sem fechamento) aparece
        let fence_hidden = |line: usize| card_of[line].is_some_and(|c| c.role != CardRole::Body && !(line == cur_line && c.close.is_none()));
        // realce de sintaxe: arquivos pela extensão, cartões pela linguagem escrita depois de ```
        let file_lang = if code {
            ed.file.as_ref().and_then(|p| p.extension()).map(|e| Lang::from_name(&e.to_string_lossy())).unwrap_or(Lang::Plain)
        } else {
            Lang::Plain
        };
        // só a linguagem de cada linha; as cores são calculadas para as linhas que aparecem na tela
        let line_lang: Vec<Option<Lang>> = (0..n_lines)
            .map(|i| match card_of[i] {
                Some(c) if c.role == CardRole::Body => Some(Lang::from_name(ed.buf.line(c.open).trim_start().trim_start_matches('`'))),
                _ if code && file_lang != Lang::Plain => Some(file_lang),
                _ => None,
            })
            .collect();

        // medidas por linha: notas em fonte proporcional (Roboto, medida pelo GPUI); código em Hack
        let mut xs_all: Vec<Rc<Vec<f32>>> = Vec::with_capacity(n_lines);
        let mut metrics: Vec<(bool, f32, f32)> = Vec::with_capacity(n_lines); // (mono, tamanho, altura)
        for (i, line) in ed.buf.lines().iter().enumerate() {
            let kind = kinds[i];
            let scale = scale_of(kind);
            let is_mono = code || matches!(kind, LineKind::Fence | LineKind::Code);
            let size = ED_FONT * scale;
            let xs: Vec<f32> = if is_mono {
                let w = cw * scale;
                let mut v = Vec::with_capacity(line.len() + 1);
                let mut x = 0.0;
                v.push(0.0);
                for c in line.chars() {
                    x += if c == '\t' { 4.0 * w } else { w };
                    v.push(x);
                }
                v
            } else {
                let disp: String = line.chars().map(|c| if c == '\t' { ' ' } else { c }).collect();
                if disp.is_empty() {
                    vec![0.0]
                } else {
                    // negrito e itálico mudam a largura: mede com os mesmos estilos que serão desenhados
                    let heading = matches!(kind, LineKind::Heading(_));
                    let styles = inline_styles(&disp.chars().collect::<Vec<_>>());
                    // o marcador do título ("# ") é desenhado sem negrito: mede igual
                    let head_mark = marker_range(kind, line);
                    let mut runs: Vec<TextRun> = Vec::new();
                    let mut last: Option<(bool, bool)> = None;
                    for (k, (c, cs)) in disp.chars().zip(&styles).enumerate() {
                        let in_mark = head_mark.is_some_and(|(a, b)| k >= a && k < b);
                        let key = ((heading && !in_mark) || cs.bold, cs.italic);
                        match runs.last_mut() {
                            Some(r) if last == Some(key) => r.len += c.len_utf8(),
                            _ => {
                                runs.push(TextRun {
                                    len: c.len_utf8(),
                                    font: Font {
                                        family: ui_font.clone(),
                                        features: FontFeatures::default(),
                                        fallbacks: None,
                                        weight: if key.0 { FontWeight::BOLD } else { FontWeight::NORMAL },
                                        style: if key.1 { FontStyle::Italic } else { FontStyle::Normal },
                                    },
                                    color: hsla(TEXT),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                });
                                last = Some(key);
                            }
                        }
                    }
                    let shaped = window.text_system().shape_line(SharedString::from(disp.clone()), px(size), &runs, None);
                    let mut v: Vec<f32> = disp.char_indices().map(|(b, _)| f32::from(shaped.x_for_index(b))).collect();
                    v.push(f32::from(shaped.x_for_index(disp.len())));
                    v
                }
            };
            let lh = match card_of[i] {
                Some(c) if fence_hidden(i) && c.role == CardRole::Open => CARD_TOP_H,
                Some(c) if fence_hidden(i) && c.role == CardRole::Close => CARD_FOOT_H,
                _ if is_mono => (size * 1.45).round(),
                _ => (size * 1.6).round(),
            };
            xs_all.push(Rc::new(xs));
            metrics.push((is_mono, size, lh));
        }

        // quebra por largura em pixels
        let mut rows: Vec<Row> = Vec::new();
        for (i, line) in ed.buf.lines().iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            if fence_hidden(i) {
                rows.push(Row { line: i, start: 0, end: chars.len() });
                continue;
            }
            let w = if card_of[i].is_some() { text_w - CARD_PAD * 2.0 } else { text_w };
            for (a, b) in wrap_by_width(&chars, &xs_all[i], w) {
                rows.push(Row { line: i, start: a, end: b });
            }
        }
        let cur_row = ed.buf.cursor_row(&rows);
        let (sel, cursor) = (ed.buf.selection(), ed.buf.cursor());

        // posição de cada linha visual (com o respiro em volta dos cartões)
        let mut tops: Vec<f32> = Vec::with_capacity(rows.len());
        let mut gaps: Vec<(f32, f32)> = Vec::with_capacity(rows.len());
        let mut y = 0.0f32;
        for (i, r) in rows.iter().enumerate() {
            let card = card_of[r.line];
            let gap_before = if card.is_some_and(|c| c.role == CardRole::Open) && r.start == 0 { CARD_GAP } else { 0.0 };
            let ends_card = card.is_some_and(|c| c.role == CardRole::Close || (c.close.is_none() && r.line + 1 == n_lines));
            let gap_after = if ends_card && rows.get(i + 1).is_none_or(|n| n.line != r.line) { CARD_GAP } else { 0.0 };
            y += gap_before;
            tops.push(y);
            y += metrics[r.line].2 + gap_after;
            gaps.push((gap_before, gap_after));
        }
        let total_h = y;
        let bottom_of = |i: usize| tops[i] + metrics[rows[i].line].2 + gaps[i].1;

        // só as linhas na tela (com folga) viram elementos: arquivos grandes continuam leves
        const PAD_Y: f32 = 4.0;
        let view_h = f32::from(ed.scroll.bounds().size.height);
        let view_h = if view_h > 1.0 { view_h } else { 2000.0 };
        let mut scroll_top = -f32::from(ed.scroll.offset().y);
        if ed.reveal {
            // rola o mínimo para o cursor aparecer
            let (row_top, row_bot) = (PAD_Y + tops[cur_row], PAD_Y + tops[cur_row] + metrics[rows[cur_row].line].2);
            if row_top < scroll_top {
                scroll_top = (row_top - 8.0).max(0.0);
            } else if row_bot > scroll_top + view_h {
                scroll_top = row_bot - view_h + 8.0;
            }
            ed.scroll.set_offset(gpui::point(px(0.), px(-scroll_top)));
            ed.reveal = false;
        }
        const MARGIN: f32 = 400.0;
        let vis_start = (0..rows.len()).find(|&i| PAD_Y + bottom_of(i) >= scroll_top - MARGIN).unwrap_or(rows.len().saturating_sub(1));
        let vis_end = (vis_start..rows.len()).find(|&i| PAD_Y + tops[i] > scroll_top + view_h + MARGIN).unwrap_or(rows.len()).max(vis_start + 1).min(rows.len());
        let top_spacer = tops[vis_start] - gaps[vis_start].0;
        let bottom_spacer = (total_h - bottom_of(vis_end - 1)).max(0.0);

        let mut row_els: Vec<AnyElement> = Vec::with_capacity(vis_end - vis_start);
        for (i, r) in rows.iter().copied().enumerate().skip(vis_start).take(vis_end - vis_start) {
            let kind = kinds[r.line];
            let card = card_of[r.line];
            let hidden = fence_hidden(r.line);
            let pad = if card.is_some() { CARD_PAD } else { 0.0 };
            let (is_mono, size, rh) = metrics[r.line];
            let xs = xs_all[r.line].clone();
            let x_at = |col: usize| xs[col.min(xs.len() - 1)] - xs[r.start];
            let (gap_before, gap_after) = gaps[i];

            let is_last = rows.get(i + 1).is_none_or(|n| n.line != r.line);
            let line_text = ed.buf.line(r.line);
            let chars: Vec<char> = line_text.chars().collect();
            let marker = if code { None } else { marker_range(kind, line_text) };
            let (text_color, marker_color, bold) = match kind {
                LineKind::Normal => (TEXT, ACCENT, false),
                LineKind::Heading(_) => (TEXT, MUTED, true),
                LineKind::Fence => (MUTED, MUTED, false),
                LineKind::Code => (syntax::PLAIN, syntax::PLAIN, false),
                LineKind::Quote => (MUTED, ACCENT, false),
                LineKind::Divider => (MUTED, MUTED, false),
            };
            // trechos (texto, cor, negrito): realce de sintaxe ou marcadores de Markdown esmaecidos
            let segments: Vec<Seg> = if hidden {
                Vec::new()
            } else if let Some(lang) = line_lang[r.line] {
                let cols = syntax::colors(lang, line_text);
                syntax::spans(&chars, &cols, r.start, r.end).into_iter().map(|(t, c)| Seg::plain(t, c)).collect()
            } else if code || matches!(kind, LineKind::Fence | LineKind::Code | LineKind::Divider) {
                split_segments(&chars, r.start, r.end, marker)
                    .into_iter()
                    .map(|(t, is_marker)| Seg { bold: bold && !is_marker, ..Seg::plain(t, if is_marker { marker_color } else { text_color }) })
                    .collect()
            } else {
                // texto da nota: marcador do começo da linha (#, -, >) + formatação dentro da linha
                let styles = inline_styles(&chars);
                let mut out: Vec<Seg> = Vec::new();
                let mut key_prev = None;
                for k in r.start..r.end.min(chars.len()) {
                    let cs = styles[k];
                    let line_mark = marker.is_some_and(|(a, b)| k >= a && k < b);
                    let color = if line_mark {
                        marker_color
                    } else if cs.mark {
                        DISABLED
                    } else if cs.link {
                        AMBER
                    } else if cs.code {
                        ACCENT
                    } else if cs.strike {
                        MUTED
                    } else {
                        text_color
                    };
                    let seg = Seg {
                        text: String::new(),
                        color,
                        bold: (bold && !line_mark) || cs.bold,
                        italic: cs.italic,
                        strike: cs.strike && !cs.mark,
                        underline: cs.link,
                        bg: (cs.code && !cs.mark).then_some(CODE_BG),
                    };
                    let key = (seg.color, seg.bold, seg.italic, seg.strike, seg.underline, seg.bg);
                    match out.last_mut() {
                        Some(last) if key_prev == Some(key) => last.text.push(chars[k]),
                        _ => {
                            out.push(Seg { text: chars[k].to_string(), ..seg });
                            key_prev = Some(key);
                        }
                    }
                }
                out
            };
            let show_plus = !code && cur_row == i && line_text.is_empty() && kind == LineKind::Normal && sel.is_none();

            // trecho selecionado nesta linha visual, em pixels
            let sel_span = sel.and_then(|(s, e)| {
                if r.line < s.line || r.line > e.line {
                    return None;
                }
                let a = if r.line == s.line { s.col } else { 0 };
                let b = if r.line == e.line { e.col } else { usize::MAX };
                let continues = r.line < e.line && is_last; // destaca também o "\n" selecionado
                let (lo, hi) = (a.max(r.start), b.min(r.end));
                (lo < hi || (continues && lo <= r.end)).then(|| (x_at(lo), x_at(hi.max(lo)) + if continues { 6.0 } else { 0.0 }))
            });


            // clique -> coluna: limite entre caracteres mais próximo, usando as posições medidas
            let xs_click = xs.clone();
            let col_at = move |x: f32| {
                let hi = if is_last { r.end } else { r.end.saturating_sub(1).max(r.start) };
                nearest_boundary(&xs_click, r.start, hi, x - x0 - pad + xs_click[r.start])
            };
            let col_at2 = col_at.clone();
            let cursor_x = x_at(cursor.col.max(r.start)) + pad;
            // rodapé do cartão (no lugar da cerca de fechamento): linguagem à esquerda, copiar/executar à direita
            let footer = card.filter(|c| hidden && c.role == CardRole::Close).map(|c| {
                let body = ed.buf.lines()[c.open + 1..c.close.unwrap_or(c.open + 1)].join("\n");
                let lang = ed.buf.line(c.open).trim_start().trim_start_matches('`').trim().to_string();
                (c.open, lang_label(&lang), body, runnable.get(&c.open).cloned())
            });

            row_els.push(
                div()
                    .id(("ed-row", i))
                    .relative()
                    .flex_none()
                    .w_full()
                    .h(px(rh))
                    .whitespace_nowrap()
                    .font_family(if is_mono { mono.clone() } else { ui_font.clone() })
                    .text_size(px(size))
                    .line_height(px(rh))
                    .mt(px(gap_before))
                    .mb(px(gap_after))
                    .when_some(card, |d, c| {
                        let top = c.role == CardRole::Open && r.start == 0;
                        // bloco sem fechamento vai até o fim da nota: fecha o contorno na última linha
                        let bottom = (c.role == CardRole::Close || (c.close.is_none() && r.line + 1 == n_lines)) && is_last;
                        d.bg(rgb(CODE_BG))
                            .border_l_1()
                            .border_r_1()
                            .border_color(rgb(CARD_BORDER))
                            .when(top, |d| d.border_t_1().rounded_t(px(8.)))
                            .when(bottom, |d| d.border_b_1().rounded_b(px(8.)))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let pos = Pos::new(r.line, col_at(f32::from(ev.position.x)));
                            this.editor_click(pos, ev.click_count, ev.modifiers.shift, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        if ev.pressed_button == Some(MouseButton::Left) {
                            this.editor_drag(Pos::new(r.line, col_at2(f32::from(ev.position.x))), cx);
                        }
                    }))
                    .when(code && r.start == 0, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(-gutter))
                                .w(px(gutter - 10.))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_end()
                                .text_size(px(size * 0.85))
                                .text_color(rgb(if r.line == cursor.line { MUTED } else { DISABLED }))
                                .child((r.line + 1).to_string()),
                        )
                    })
                    .when_some(sel_span.filter(|_| !hidden), |d, (a, b)| d.child(div().absolute().left(px(a + pad)).w(px((b - a).max(0.0))).h_full().bg(rgb(SELECTION))))
                    .when(kind == LineKind::Divider, |d| d.child(div().absolute().left_0().right_0().top(px(rh / 2.)).h(px(1.)).bg(rgb(BORDER))))
                    .child(div().flex().flex_row().h_full().ml(px(pad)).items_center().children(segments.into_iter().map(|sg| {
                        // tabulação: 4 espaços no código (bate com as medidas), 1 espaço no texto proporcional
                        let t = if is_mono { sg.text.replace('\t', "    ") } else { sg.text.replace('\t', " ") };
                        div()
                            .flex_none()
                            .text_color(rgb(sg.color))
                            .when(sg.bold, |d| d.font_weight(FontWeight::BOLD))
                            .when(sg.italic, |d| d.italic())
                            .when(sg.strike, |d| d.line_through())
                            .when(sg.underline, |d| d.underline())
                            .when_some(sg.bg, |d, c| d.bg(rgb(c)).rounded(px(3.)))
                            .child(t)
                    })))
                    .when(show_plus, |d| {
                        let line = r.line;
                        d.child(
                            div()
                                .id(("ed-plus", i))
                                .absolute()
                                .left(px(-(GUTTER - 2.)))
                                .top(px(1.))
                                .size(px(rh - 2.))
                                .rounded_md()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(HOVER)))
                                .child(wicon("plus", 14., MUTED))
                                .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.editor_plus(line, cx))),
                        )
                        .child(div().absolute().left(px(4.)).top_0().h_full().flex().items_center().text_color(rgb(DISABLED)).child(EMPTY_HINT))
                    })
                    .when(cur_row == i && !hidden, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(cursor_x))
                                .top(px(2.))
                                .h(px(rh - 4.))
                                .w(px(if focused { 2. } else { 1. }))
                                .when(caret_on || !focused, |d| d.bg(rgb(if focused { ACCENT } else { MUTED }))),
                        )
                    })
                    .when_some(footer, |d, (open, label, body, run)| {
                        let menu_open = ed.lang_menu == Some(open);
                        d.child(
                            div()
                                .absolute()
                                .left(px(CARD_PAD - 4.))
                                .right(px(8.))
                                .top(px(4.))
                                .bottom(px(6.))
                                .flex()
                                .items_center()
                                .justify_between()
                                .font_family(SharedString::from(UI_FONT))
                                .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
                                .child(
                                    div()
                                        .id(("ed-lang", i))
                                        .h(px(28.))
                                        .min_w(px(96.))
                                        .px(px(10.))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .rounded(px(6.))
                                        .border_1()
                                        .border_color(rgb(if menu_open { ACCENT } else { CARD_BORDER }))
                                        .text_size(px(13.))
                                        .text_color(rgb(MUTED))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                                        .child(label)
                                        .child(wicon("chevron-down", 12., MUTED))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            this.focus_editor = true; // Esc e as setas continuam no editor
                                            if let Some(e) = this.editor.as_mut() {
                                                e.lang_menu = if e.lang_menu == Some(open) { None } else { Some(open) };
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_1()
                                        .child(card_btn(("ed-del", i), "trash-02").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            if let Some(e) = this.editor.as_mut() {
                                                let close = closed_card(e.buf.lines(), open).and_then(|c| c.close).unwrap_or(open);
                                                e.buf.remove_lines(open, close);
                                                e.lang_menu = None;
                                            }
                                            this.after_edit(cx);
                                            cx.notify();
                                        })))
                                        .child(card_btn(("ed-copy", i), "copy").on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(body.clone()));
                                        })))
                                        .when_some(run, |d, code| {
                                            d.child(card_btn(("ed-run", i), "terminal").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                                this.run_in_terminal(&code);
                                                cx.notify();
                                            })))
                                        }),
                                ),
                        )
                    })
                    .into_any_element(),
            );
        }

        // menu "/" ancorado abaixo da linha do cursor (só em notas)
        let view_h = f32::from(ed.scroll.bounds().size.height);
        let query = if code { None } else { ed.slash_query() };
        let slash_ui = query.and_then(|query| {
            let items = slash_choices(&query, &ref_notes);
            let s = ed.slash.as_mut()?;
            s.count = items.len();
            s.sel = s.sel.min(items.len().saturating_sub(1));
            s.follow_sel();
            let menu_h = SLASH_SEARCH_H + SLASH_VISIBLE as f32 * SLASH_ROW_H + 10.0;
            let row_top = tops[cur_row] + f32::from(ed.scroll.offset().y);
            let below = 4.0 + row_top + metrics[rows[cur_row].line].2;
            // sem espaço embaixo da linha: abre para cima, para o menu nunca sair da área visível
            let top = if below + menu_h > view_h && row_top - menu_h - 4.0 >= 0.0 { row_top - menu_h - 4.0 } else { below };
            Some((query, items, s.sel, s.first, top.max(0.0), menu_h))
        });
        // seletor de linguagem, logo abaixo do rodapé do cartão
        let lang_ui = ed.lang_menu.and_then(|open| {
            let close = card_of.get(open).copied().flatten()?.close?;
            let ri = rows.iter().position(|r| r.line == close)?;
            let current = ed.buf.line(open).trim_start().trim_start_matches('`').trim().to_lowercase();
            Some((open, current, 4.0 + tops[ri] + CARD_FOOT_H - 6.0 + f32::from(ed.scroll.offset().y)))
        });
        // barra flutuante de formatação acima do início da seleção (só em notas)
        let omni_pos = sel.filter(|_| ed.slash.is_none() && !code).map(|(s, _)| {
            let ri = rows.iter().position(|r| r.line == s.line && s.col < r.end).or_else(|| rows.iter().rposition(|r| r.line == s.line)).unwrap_or(0);
            let r = rows[ri];
            let xs = &xs_all[r.line];
            let x = (CONTENT_PAD + gutter + xs[s.col.min(xs.len() - 1)] - xs[r.start]).clamp(8.0, (content_w - 380.0).max(8.0));
            (x, (4.0 + tops[ri] + f32::from(ed.scroll.offset().y) - 44.0).max(4.0))
        });
        let scroll = ed.scroll.clone();
        let editor_thumb = thumb_geometry(f32::from(scroll.bounds().size.height), f32::from(scroll.max_offset().height), -f32::from(scroll.offset().y));

        // menu "/": campo de busca no topo; blocos e notas com ícone, nome e onde ficam
        let menu = slash_ui.map(|(query, items, sel, first, top, menu_h)| {
            let total = items.len();
            let search = div()
                .flex_none()
                .h(px(38.))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(rgb(CARD_BORDER))
                .child(wicon("search", 14., MUTED))
                .child(div().w(px(2.)).h(px(16.)).bg(rgb(ACCENT)))
                .child(if query.is_empty() {
                    div().text_color(rgb(DISABLED)).child("Buscar bloco ou nota")
                } else {
                    div().text_color(rgb(TEXT)).child(query)
                });
            let list = div().id("slash-list").relative().flex_1().min_h_0().overflow_hidden().p_1().flex().flex_col();
            let list = if items.is_empty() {
                list.child(div().px_2().py_2().text_color(rgb(MUTED)).child("Nada com esse nome. Apague para ver tudo."))
            } else {
                list.children(items.into_iter().enumerate().skip(first).take(SLASH_VISIBLE).map(|(i, it)| {
                    let (icon, glyph, color, title, sub) = match &it {
                        SlashChoice::Block(b) => {
                            let color = if b.label == "Comando" { CORAL } else { MUTED };
                            (b.icon, b.glyph, color, b.label.to_string(), "Bloco".to_string())
                        }
                        SlashChoice::Note { label, dir, .. } => {
                            let sub = if dir.is_empty() { "Notas".to_string() } else { format!("Notas / {}", dir.replace('/', " / ")) };
                            (Some("notebook"), "", ACCENT, label.clone(), sub)
                        }
                    };
                    let active = i == sel;
                    div()
                        .id(("slash-item", i))
                        .flex_none()
                        .h(px(SLASH_ROW_H))
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_2()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .when(active, |d| d.bg(rgb(SELECTION)))
                        .when(!active, |d| d.hover(|s| s.bg(rgb(HOVER))))
                        .child(
                            div()
                                .flex_none()
                                .w(px(22.))
                                .flex()
                                .justify_center()
                                .text_size(px(12.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(color))
                                .child(match icon {
                                    Some(name) => wicon(name, 16., if active { TEXT } else { color }).into_any_element(),
                                    None => div().child(glyph).into_any_element(),
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().truncate().text_color(rgb(TEXT)).child(title))
                                .child(div().text_size(px(12.)).line_height(px(16.)).truncate().text_color(rgb(if active { TEXT } else { MUTED })).child(sub)),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.apply_slash(it.clone(), cx)))
                }))
            };
            div()
                .id("slash-menu")
                .absolute()
                .occlude()
                .left(px(CONTENT_PAD + gutter))
                .top(px(top))
                .w(px((content_w - CONTENT_PAD * 2.0 - gutter).clamp(240.0, 500.0)))
                .h(px(menu_h))
                .rounded(px(6.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(CARD_BORDER))
                .shadow_lg()
                .text_size(px(14.))
                .line_height(px(20.))
                .flex()
                .flex_col()
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
                // a roda do mouse rola a lista (e não a nota por baixo); a seleção acompanha se sair da vista
                .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                    cx.stop_propagation();
                    if let Some(s) = this.editor.as_mut().and_then(|e| e.slash.as_mut()) {
                        s.wheel -= f32::from(ev.delta.pixel_delta(px(SLASH_ROW_H)).y) / SLASH_ROW_H;
                        let steps = s.wheel.trunc() as i64;
                        s.wheel -= steps as f32;
                        let max_first = s.count.saturating_sub(SLASH_VISIBLE) as i64;
                        s.first = (s.first as i64 + steps).clamp(0, max_first) as usize;
                        s.sel = s.sel.clamp(s.first, (s.first + SLASH_VISIBLE).min(s.count).saturating_sub(1));
                    }
                    cx.notify();
                }))
                .child(search)
                .child(list.when(total > SLASH_VISIBLE, |d| {
                    // barra de rolagem: posição e tamanho da janela visível na lista
                    let track = SLASH_VISIBLE as f32 * SLASH_ROW_H;
                    let len = (track * SLASH_VISIBLE as f32 / total as f32).max(24.0);
                    let top = (track - len) * first as f32 / (total - SLASH_VISIBLE) as f32;
                    d.child(div().absolute().right(px(3.)).top(px(4.0 + top)).w(px(4.)).h(px(len)).rounded_full().bg(rgb(BTN_HOVER)))
                }))
        });

        let lang_menu = lang_ui.map(|(open, current, top)| {
            div()
                .id("lang-menu")
                .absolute()
                .occlude()
                .left(px(CONTENT_PAD + gutter + CARD_PAD - 4.))
                .top(px(top))
                .w(px(160.))
                .p_1()
                .rounded(px(6.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(CARD_BORDER))
                .shadow_lg()
                .text_size(px(13.))
                .flex()
                .flex_col()
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _: &MouseDownEvent, _, cx| cx.stop_propagation()))
                .children(LANG_OPTIONS.iter().enumerate().map(|(i, (label, lang))| {
                    let on = lang_label(&current) == *label;
                    div()
                        .id(("lang-opt", i))
                        .px_2()
                        .py(px(5.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .text_color(rgb(if on { TEXT } else { MUTED }))
                        .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                        .child(*label)
                        .when(on, |d| d.child(wicon("check", 13., ACCENT)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_card_lang(open, lang, cx)))
                }))
        });

        let omnibar = omni_pos.map(|(x, top)| {
            let mut bar = div()
                .absolute()
                // bloqueia o clique para a linha de texto embaixo (senão a seleção some antes do botão agir)
                .occlude()
                .left(px(x))
                .top(px(top))
                .flex()
                .items_center()
                .gap(px(2.))
                .p(px(3.))
                .rounded(px(4.))
                .bg(rgb(SURFACE))
                .border_1()
                .border_color(rgb(BTN_HOVER))
                .shadow_md();
            for (id, label, op) in OMNIBAR.iter().copied() {
                if matches!(id, "om-b" | "om-list") {
                    bar = bar.child(div().w(px(1.)).h(px(16.)).mx(px(3.)).bg(rgb(BTN_HOVER)));
                }
                bar = bar.child(
                    div()
                        .id(id)
                        .w(px(30.))
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_size(px(14.))
                        .hover(|s| s.bg(rgb(BTN_HOVER)))
                        .when(label == "B", |d| d.font_weight(FontWeight::BOLD))
                        .when(label == "I", |d| d.italic())
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.edit_with(op, cx))),
                );
            }
            bar
        });

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.focus_editor = true;
                    this.focused_pane = None; // as setas voltam a mover o cursor do editor
                    cx.stop_propagation(); // o contêiner raiz tiraria o foco do editor
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    if let Some(e) = this.editor.as_mut() {
                        e.drag = false;
                    }
                }),
            )
            .child(
                div()
                    .id("note-editor")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .py(px(4.))
                    .pr(px(CONTENT_PAD))
                    .pl(px(CONTENT_PAD + gutter))
                    .flex()
                    .flex_col()
                    .font_family(ui_font)
                    // a lista desenha só o trecho visível: ao rolar, redesenha com o trecho novo
                    .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| cx.notify()))
                    .child(div().flex_none().h(px(top_spacer)))
                    .children(row_els)
                    .child(div().flex_none().h(px(bottom_spacer))),
            )
            .when_some(menu, |d, m| d.child(m))
            .when_some(lang_menu, |d, m| d.child(m))
            .when_some(omnibar, |d, m| d.child(m))
            .when_some(editor_thumb, |d, (top, len)| d.child(thumb(top, len)))
            .into_any_element()
    }

    // ----- modo leitura -----

    fn render_block(&self, i: usize, b: &Block, base: &TextStyle, cx: &mut Context<Self>) -> AnyElement {
        match b {
            Block::Heading { level, inlines } => {
                let d = div().mt_2();
                let d = match level {
                    1 => d.text_size(px(26.)),
                    2 => d.text_size(px(22.)),
                    3 => d.text_size(px(19.)),
                    _ => d.text_size(px(17.)),
                };
                d.child(self.rich(inlines, base, true, TEXT)).into_any_element()
            }
            Block::Paragraph(inlines) => div().child(self.rich(inlines, base, false, TEXT)).into_any_element(),
            Block::ListItem { depth, number, checked, inlines } => {
                let marker = match (number, checked) {
                    (_, Some(true)) => "☑".to_string(),
                    (_, Some(false)) => "☐".to_string(),
                    (Some(n), _) => format!("{n}."),
                    _ => "•".to_string(),
                };
                div()
                    .flex()
                    .gap_2()
                    .pl(px(*depth as f32 * 16.))
                    .child(div().flex_none().text_color(rgb(MUTED)).child(marker))
                    .child(div().flex_1().min_w_0().child(self.rich(inlines, base, false, TEXT)))
                    .into_any_element()
            }
            Block::Callout { kind, inlines } => {
                let (label, color) = match kind {
                    Some(CalloutKind::Note) => ("NOTA", ACCENT),
                    Some(CalloutKind::Tip) => ("DICA", GREEN),
                    Some(CalloutKind::Important) => ("IMPORTANTE", PURPLE),
                    Some(CalloutKind::Warning) => ("ATENÇÃO", YELLOW),
                    Some(CalloutKind::Caution) => ("CUIDADO", RED),
                    None => ("", MUTED),
                };
                div()
                    .p_2()
                    .rounded_sm()
                    .border_l_2()
                    .border_color(rgb(color))
                    .bg(rgb(CODE_BG))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(!label.is_empty(), |d| d.child(div().text_xs().text_color(rgb(color)).child(label)))
                    .child(self.rich(inlines, base, false, TEXT))
                    .into_any_element()
            }
            Block::Code(c) => {
                // mesmo cartão do modo edição: código em cima; linguagem, copiar e executar no rodapé
                let run_code = c.code.clone();
                let copy_code = c.code.clone();
                let footer = div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_2()
                    .child(
                        div()
                            .h(px(28.))
                            .min_w(px(96.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(rgb(CARD_BORDER))
                            .text_size(px(13.))
                            .text_color(rgb(MUTED))
                            .child(lang_label(&c.lang)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(card_btn(("copy", i), "copy").on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_code.clone()));
                            })))
                            .when(c.runnable, |d| {
                                d.child(card_btn(("run", i), "terminal").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.run_in_terminal(&run_code);
                                    cx.notify();
                                })))
                            }),
                    );
                div()
                    .flex()
                    .flex_col()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(CARD_BORDER))
                    .bg(rgb(CODE_BG))
                    .pl(px(CARD_PAD))
                    .pr(px(8.))
                    .pt(px(CARD_TOP_H))
                    .pb(px(8.))
                    .child(
                        div().flex().flex_col().font_family(self.mono.clone()).text_size(px(14.)).children(c.code.lines().map(|line| {
                            let chars: Vec<char> = line.chars().collect();
                            let cols = syntax::colors(Lang::from_name(&c.lang), line);
                            div()
                                .flex()
                                .min_h(px(20.))
                                .whitespace_nowrap()
                                .children(syntax::spans(&chars, &cols, 0, chars.len()).into_iter().map(|(t, col)| div().flex_none().text_color(rgb(col)).child(t.replace('\t', "    "))))
                        })),
                    )
                    .child(footer)
                    .into_any_element()
            }
            Block::Rule => div().h(px(1.)).bg(rgb(BORDER)).into_any_element(),
        }
    }

    fn rich(&self, inlines: &[Inline], base: &TextStyle, bold_all: bool, color: u32) -> StyledText {
        let mut text = String::new();
        let mut runs: Vec<TextRun> = Vec::new();
        for i in inlines {
            let mut r = base.to_run(i.text.len());
            // a fonte padrão da janela não tem negrito/itálico carregados: usa a Roboto do app
            r.font.family = UI_FONT.into();
            r.color = hsla(color);
            if i.bold || bold_all {
                r.font.weight = FontWeight::BOLD;
            }
            if i.italic {
                r.font.style = FontStyle::Italic;
            }
            if i.code {
                r.font.family = self.mono.clone();
                r.background_color = Some(hsla(CODE_BG));
                r.color = hsla(ACCENT);
            }
            if i.strike {
                r.strikethrough = Some(gpui::StrikethroughStyle { thickness: px(1.), color: Some(hsla(MUTED)) });
                r.color = hsla(MUTED);
            }
            if i.link.is_some() {
                r.color = hsla(AMBER);
                r.underline = Some(gpui::UnderlineStyle { thickness: px(1.), color: Some(hsla(AMBER)), wavy: false });
            }
            text.push_str(&i.text);
            runs.push(r);
        }
        StyledText::new(text).with_runs(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.split('\n').map(String::from).collect()
    }

    #[test]
    fn classify_finds_headings_fences_and_runnable_blocks() {
        let l = lines("# T\ntexto\n```bash\nls -la\necho oi\n```\n> cit\n---\n```rust\nfn x(){}\n```");
        let (k, run) = classify(&l);
        assert_eq!(k[0], LineKind::Heading(1));
        assert_eq!(k[1], LineKind::Normal);
        assert_eq!(k[2], LineKind::Fence);
        assert_eq!(k[3], LineKind::Code);
        assert_eq!(k[6], LineKind::Quote);
        assert_eq!(k[7], LineKind::Divider);
        assert_eq!(run.get(&2).map(String::as_str), Some("ls -la\necho oi")); // só o bloco bash é executável
        assert!(!run.contains_key(&8));
    }

    #[test]
    fn unclosed_fence_is_not_runnable_and_swallows_following_lines() {
        let (k, run) = classify(&lines("```bash\nls\n# nao e titulo"));
        assert_eq!(k, [LineKind::Fence, LineKind::Code, LineKind::Code]);
        assert!(run.is_empty());
    }

    #[test]
    fn marker_ranges_for_lists_headings_and_quotes() {
        assert_eq!(marker_range(LineKind::Heading(2), "## Oi"), Some((0, 3)));
        assert_eq!(marker_range(LineKind::Normal, "  - item"), Some((2, 4)));
        assert_eq!(marker_range(LineKind::Normal, "- [x] ok"), Some((0, 6)));
        assert_eq!(marker_range(LineKind::Normal, "12. dois"), Some((0, 4)));
        assert_eq!(marker_range(LineKind::Normal, "texto"), None);
        assert_eq!(marker_range(LineKind::Quote, "> x"), Some((0, 2)));
    }

    #[test]
    fn segments_split_around_marker_across_wrapped_rows() {
        let chars: Vec<char> = "## titulo longo".chars().collect();
        let seg = split_segments(&chars, 0, 9, Some((0, 3)));
        assert_eq!(seg, [("## ".to_string(), true), ("titulo".to_string(), false)]);
        // linha visual seguinte não contém o marcador
        let seg = split_segments(&chars, 9, 15, Some((0, 3)));
        assert_eq!(seg, [(" longo".to_string(), false)]);
    }

    #[test]
    fn omnibar_actions_format_the_selection() {
        let apply = |id: &str, text: &str| {
            let mut b = TextBuffer::new(text);
            b.select_all();
            let (_, _, op) = OMNIBAR.iter().copied().find(|(i, _, _)| *i == id).unwrap();
            op(&mut b);
            b.text()
        };
        assert_eq!(apply("om-b", "oi"), "**oi**");
        assert_eq!(apply("om-i", "oi"), "*oi*");
        assert_eq!(apply("om-s", "oi"), "~~oi~~");
        assert_eq!(apply("om-code", "ls"), "`ls`");
        assert_eq!(apply("om-link", "site"), "[site](url)");
        assert_eq!(apply("om-h2", "titulo"), "## titulo");
        assert_eq!(apply("om-list", "item"), "- item");
        assert_eq!(apply("om-todo", "fazer"), "- [ ] fazer");
        assert_eq!(apply("om-quote", "fala"), "> fala");
    }

    #[test]
    fn code_cards_group_fences_and_bodies() {
        let l = lines("texto\n```bash\nls\npwd\n```\nfim\n```\nsem fim");
        let (k, _) = classify(&l);
        let c = cards(&k);
        assert_eq!(c[0], None);
        assert_eq!(c[1], Some(Card { open: 1, close: Some(4), role: CardRole::Open }));
        assert_eq!(c[3].map(|x| x.role), Some(CardRole::Body));
        assert_eq!(c[4].map(|x| x.role), Some(CardRole::Close));
        assert_eq!(c[5], None);
        // bloco não fechado: abertura + corpo até o fim, sem rodapé
        assert_eq!(c[6], Some(Card { open: 6, close: None, role: CardRole::Open }));
        assert_eq!(c[7].map(|x| x.role), Some(CardRole::Body));
    }

    #[test]
    fn enter_after_open_fence_needs_closing_only_when_missing() {
        let l = lines("# T\n```python");
        assert!(fence_needs_close(&l, Pos::new(1, 9)));
        assert!(!fence_needs_close(&l, Pos::new(1, 3))); // cursor no meio da cerca
        let closed = lines("```bash\nls\n```");
        assert!(!fence_needs_close(&closed, Pos::new(0, 7)));
        assert!(!fence_needs_close(&closed, Pos::new(2, 3))); // cerca de fechamento
    }

    #[test]
    fn typing_backticks_makes_a_card_and_backspace_unwraps_it() {
        let mut b = TextBuffer::new("# T\n```");
        b.move_doc_end(false);
        turn_backticks_into_card(&mut b);
        assert_eq!(b.text(), "# T\n```\n\n```");
        assert_eq!(b.cursor(), Pos::new(2, 0));
        // um bloco fechado não é recriado
        let mut c = TextBuffer::new("```\nx\n```");
        c.move_doc_end(false);
        turn_backticks_into_card(&mut c);
        assert_eq!(c.text(), "```\nx\n```");
        // desfazer o bloco mantém o texto
        let mut d = TextBuffer::new("a\n```bash\necho oi\n```\nb");
        unwrap_card(&mut d, 1, 3);
        assert_eq!(d.text(), "a\necho oi\nb");
        assert_eq!(d.cursor(), Pos::new(1, 0));
    }

    #[test]
    fn inline_styles_mark_symbols_and_style_contents() {
        let line: Vec<char> = "a **b** *c* ~~d~~ `e` [f](u) snake_case".chars().collect();
        let st = inline_styles(&line);
        let at = |s: &str| line.iter().collect::<String>().find(s).map(|b| line.iter().collect::<String>()[..b].chars().count()).unwrap();
        assert!(st[at("**b")].mark && st[at("b**")].bold && !st[at("b**")].mark);
        assert!(st[at("c*")].italic);
        assert!(st[at("d~")].strike && st[at("~~d")].mark);
        assert!(st[at("e`")].code && st[at("`e")].mark);
        assert!(st[at("f]")].link && st[at("](u)")].mark && st[at("u)")].mark);
        // "_" no meio de uma palavra não vira itálico
        assert!(!st[at("_case")].mark && !st[at("case")].italic);
        // símbolo sem par continua texto
        assert!(inline_styles(&"2 * 3".chars().collect::<Vec<_>>()).iter().all(|s| *s == CharStyle::default()));
    }

    #[test]
    fn language_labels_match_the_picker() {
        assert_eq!(lang_label(""), "Code");
        assert_eq!(lang_label("bash"), "Shell");
        assert_eq!(lang_label("py"), "Python");
        assert_eq!(lang_label("go"), "Go");
        assert!(LANG_OPTIONS.iter().all(|(label, lang)| lang_label(lang) == *label));
    }

    #[test]
    fn slash_choices_list_blocks_then_notes_and_insert_links() {
        let notes = vec!["ideias.md".to_string(), "trabalho/deploy.md".to_string()];
        let all = slash_choices("", &notes);
        assert_eq!(all.len(), SLASH_ITEMS.len() + 2);
        let found = slash_choices("depl", &notes);
        assert!(matches!(&found[..], [SlashChoice::Note { label, dir, .. }] if label == "deploy" && dir == "trabalho"));
        let mut b = TextBuffer::new("antes\n/depl");
        b.move_doc_end(false);
        apply_choice(&mut b, &found[0]);
        assert_eq!(b.text(), "antes\n[deploy](trabalho/deploy.md)");
    }

    #[test]
    fn slash_filter_matches_label_and_keywords() {
        assert_eq!(slash_matches("").len(), SLASH_ITEMS.len());
        assert_eq!(slash_matches("cmd").len(), 0);
        assert_eq!(slash_matches("coma")[0].label, "Comando");
        assert_eq!(slash_matches("h2")[0].label, "Título 2");
        assert!(slash_matches("lista").iter().any(|i| i.label == "Lista numerada"));
    }

    #[test]
    fn slash_menu_flow_apply_block() {
        let mut ed = NoteEditor::new("n.md", "/tit");
        ed.buf.move_doc_end(false);
        ed.slash = Some(Slash::at(0));
        assert_eq!(ed.slash_query().as_deref(), Some("tit"));
        assert!(matches!(handle_slash(&mut ed, "down", &[]), SlashOutcome::Consumed));
        assert!(matches!(handle_slash(&mut ed, "enter", &[]), SlashOutcome::Applied));
        assert_eq!(ed.buf.text(), "## "); // "Título 2" (segundo item filtrado)
        assert!(ed.slash.is_none());
    }
}
