//! Markdown "ativo": converte `.md` em uma lista de blocos estruturados que a UI desenha nativamente.
//! Blocos de shell (` ```bash `) são marcados como executáveis; a UI mostra o botão "Executar".

mod store;
pub use store::{NoteHit, NoteMeta, NoteNode, NoteStore, StoreError};

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Linguagens cujo bloco ganha o botão "Executar".
const RUNNABLE_LANGS: &[&str] = &["bash", "sh", "zsh", "fish", "shell", "pwsh", "powershell"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// ~~riscado~~
    pub strike: bool,
    /// destino de um [link](url)
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    pub lang: String,
    pub code: String,
    pub runnable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading { level: u8, inlines: Vec<Inline> },
    Paragraph(Vec<Inline>),
    ListItem { depth: usize, number: Option<u64>, checked: Option<bool>, inlines: Vec<Inline> },
    /// `> [!NOTE]` (kind = Some) ou citação comum (kind = None)
    Callout { kind: Option<CalloutKind>, inlines: Vec<Inline> },
    Code(CodeBlock),
    Rule,
}

pub fn is_runnable(lang: &str) -> bool {
    RUNNABLE_LANGS.contains(&lang)
}

fn callout_kind(k: BlockQuoteKind) -> CalloutKind {
    match k {
        BlockQuoteKind::Note => CalloutKind::Note,
        BlockQuoteKind::Tip => CalloutKind::Tip,
        BlockQuoteKind::Important => CalloutKind::Important,
        BlockQuoteKind::Warning => CalloutKind::Warning,
        BlockQuoteKind::Caution => CalloutKind::Caution,
    }
}

#[derive(Default)]
struct Walker {
    blocks: Vec<Block>,
    inlines: Vec<Inline>,
    bold: u32,
    italic: u32,
    strike: u32,
    link: Option<String>,
    heading: Option<u8>,
    quote: Option<Option<CalloutKind>>,
    /// pilha de listas: `Some(n)` = ordenada (próximo número), `None` = com marcadores
    lists: Vec<Option<u64>>,
    item_number: Option<u64>,
    item_checked: Option<bool>,
    code: Option<(String, String)>,
}

impl Walker {
    fn push_text(&mut self, text: &str, code: bool) {
        self.inlines.push(Inline {
            text: text.to_string(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            code,
            strike: self.strike > 0,
            link: self.link.clone(),
        });
    }

    fn take_inlines(&mut self) -> Vec<Inline> {
        std::mem::take(&mut self.inlines)
    }

    fn flush_item(&mut self) {
        if self.inlines.is_empty() {
            return;
        }
        let inlines = self.take_inlines();
        self.blocks.push(Block::ListItem {
            depth: self.lists.len().saturating_sub(1),
            number: self.item_number.take(),
            checked: self.item_checked.take(),
            inlines,
        });
    }

    fn event(&mut self, ev: Event) {
        match ev {
            // --- código ---
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Event::Text(t) if self.code.is_some() => self.code.as_mut().unwrap().1.push_str(&t),
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, code)) = self.code.take() {
                    self.blocks.push(Block::Code(CodeBlock {
                        runnable: is_runnable(&lang),
                        code: code.trim_end_matches('\n').to_string(),
                        lang,
                    }));
                }
            }

            // --- inline ---
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => self.push_text(&t, false), // HTML cru vira texto
            Event::Code(t) => self.push_text(&t, true),
            Event::SoftBreak => self.push_text(" ", false),
            Event::HardBreak => self.push_text("\n", false),
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => self.strike += 1,
            Event::End(TagEnd::Strikethrough) => self.strike = self.strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => self.link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => self.link = None,
            Event::TaskListMarker(done) => self.item_checked = Some(done),

            // --- blocos de texto ---
            Event::Start(Tag::Heading { level, .. }) => {
                self.heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                let inlines = self.take_inlines();
                self.blocks.push(Block::Heading { level: self.heading.take().unwrap_or(1), inlines });
            }
            Event::End(TagEnd::Paragraph) => {
                if !self.lists.is_empty() && self.quote.is_none() {
                    self.flush_item();
                } else if let Some(kind) = self.quote {
                    let inlines = self.take_inlines();
                    self.blocks.push(Block::Callout { kind, inlines });
                } else {
                    let inlines = self.take_inlines();
                    self.blocks.push(Block::Paragraph(inlines));
                }
            }
            Event::Start(Tag::BlockQuote(kind)) => self.quote = Some(kind.map(callout_kind)),
            Event::End(TagEnd::BlockQuote(_)) => self.quote = None,

            // --- listas ---
            Event::Start(Tag::List(start)) => {
                self.flush_item(); // texto pendente do item pai antes da sublista
                self.lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                self.lists.pop();
            }
            Event::Start(Tag::Item) => {
                self.item_number = self.lists.last_mut().and_then(|n| {
                    let cur = *n;
                    if let Some(v) = n {
                        *v += 1;
                    }
                    cur
                });
            }
            Event::End(TagEnd::Item) => self.flush_item(),

            Event::Rule => self.blocks.push(Block::Rule),
            _ => {}
        }
    }
}

pub fn parse(md: &str) -> Vec<Block> {
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_GFM;
    let mut w = Walker::default();
    for ev in Parser::new_ext(md, opts) {
        w.event(ev);
    }
    w.blocks
}

/// Todos os blocos de código, na ordem do documento.
pub fn code_blocks(blocks: &[Block]) -> impl Iterator<Item = &CodeBlock> {
    blocks.iter().filter_map(|b| if let Block::Code(c) = b { Some(c) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(inl: &[Inline]) -> String {
        inl.iter().map(|i| i.text.as_str()).collect()
    }

    #[test]
    fn inline_strike_and_links() {
        let b = parse("veja ~~velho~~ e [Google](https://google.com) **forte**");
        let Block::Paragraph(inl) = &b[0] else { panic!("parágrafo") };
        assert!(inl.iter().any(|i| i.text == "velho" && i.strike));
        assert!(inl.iter().any(|i| i.text == "Google" && i.link.as_deref() == Some("https://google.com")));
        assert!(inl.iter().any(|i| i.text == "forte" && i.bold && i.link.is_none()));
    }

    #[test]
    fn heading_and_runnable_bash_block() {
        let b = parse("# Título\n\n```bash\ncargo build --release\n```\n");
        assert!(matches!(&b[0], Block::Heading { level: 1, inlines } if text(inlines) == "Título"));
        assert_eq!(
            b[1],
            Block::Code(CodeBlock { lang: "bash".into(), code: "cargo build --release".into(), runnable: true })
        );
    }

    #[test]
    fn non_shell_block_is_not_runnable() {
        let b = parse("```rust\nfn main() {}\n```");
        assert!(code_blocks(&b).all(|c| !c.runnable));
    }

    #[test]
    fn info_string_attributes_and_multiline() {
        let b = parse("```sh title=\"x\"\na\nb\n```");
        let c = code_blocks(&b).next().unwrap();
        assert_eq!((c.lang.as_str(), c.code.as_str(), c.runnable), ("sh", "a\nb", true));
    }

    #[test]
    fn callout_kind_is_detected() {
        let b = parse("> [!WARNING]\n> cuidado\n");
        assert!(matches!(&b[0], Block::Callout { kind: Some(CalloutKind::Warning), inlines } if text(inlines).contains("cuidado")));
    }

    #[test]
    fn lists_nested_ordered_and_tasks() {
        let b = parse("- a\n  - b\n1. x\n2. y\n\n- [x] feito\n");
        let items: Vec<_> = b
            .iter()
            .filter_map(|b| match b {
                Block::ListItem { depth, number, checked, inlines } => Some((*depth, *number, *checked, text(inlines))),
                _ => None,
            })
            .collect();
        assert_eq!(items[0], (0, None, None, "a".into()));
        assert_eq!(items[1], (1, None, None, "b".into()));
        assert_eq!(items[2], (0, Some(1), None, "x".into()));
        assert_eq!(items[3], (0, Some(2), None, "y".into()));
        assert_eq!(items[4], (0, None, Some(true), "feito".into()));
    }

    #[test]
    fn inline_styles_and_raw_html_as_text() {
        let b = parse("**negrito** e `cod` <b>x</b>");
        let Block::Paragraph(i) = &b[0] else { panic!() };
        assert!(i[0].bold && i[0].text == "negrito");
        assert!(i.iter().any(|x| x.code && x.text == "cod"));
        assert!(text(i).contains("<b>"));
    }
}
