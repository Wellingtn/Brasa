//! Teclado e desenho do editor de Query: o que cada tecla faz num `TextBuffer` e como uma linha é cortada em pedaços
//! (cor, seleção, cursor). Só lógica, sem janela, para poder testar.

use ox_editor::{Pos, TextBuffer};

use crate::dbpane::Tok;

/// Linhas do editor visíveis ao mesmo tempo.
pub const VISIBLE_LINES: usize = 7;
pub const LINE_H: f32 = 20.0;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// o editor mudou (ou o cursor andou)
    Handled,
    /// Ctrl+Enter: executar
    Run,
    /// Esc: tirar o foco do editor
    Blur,
    /// Ctrl+C: este texto vai para a área de transferência
    Copy(String),
    /// Ctrl+X: idem, e já saiu do editor
    Cut(String),
    /// Ctrl+V: quem chama cola o que está na área de transferência
    Paste,
    /// a tecla não é do editor (atalhos globais seguem o caminho deles)
    Ignored,
}

/// Aplica uma tecla ao editor. `ch` é o texto que a tecla produz (letra, símbolo), se houver.
pub fn handle_key(buf: &mut TextBuffer, key: &str, ch: Option<&str>, ctrl: bool, shift: bool, alt: bool) -> Outcome {
    if alt {
        return Outcome::Ignored;
    }
    if ctrl {
        return match key {
            "enter" => Outcome::Run,
            "a" => {
                buf.select_all();
                Outcome::Handled
            }
            "c" => buf.selected_text().map_or(Outcome::Handled, Outcome::Copy),
            "x" => match buf.selected_text() {
                Some(t) => {
                    buf.backspace();
                    Outcome::Cut(t)
                }
                None => Outcome::Handled,
            },
            "v" => Outcome::Paste,
            "z" => {
                buf.undo();
                Outcome::Handled
            }
            "y" => {
                buf.redo();
                Outcome::Handled
            }
            "left" => {
                buf.move_word_left(shift);
                Outcome::Handled
            }
            "right" => {
                buf.move_word_right(shift);
                Outcome::Handled
            }
            "home" => {
                buf.move_doc_start(shift);
                Outcome::Handled
            }
            "end" => {
                buf.move_doc_end(shift);
                Outcome::Handled
            }
            "backspace" => {
                buf.delete_word_back();
                Outcome::Handled
            }
            _ => Outcome::Ignored,
        };
    }
    match key {
        "escape" => Outcome::Blur,
        "enter" => {
            buf.insert_newline();
            Outcome::Handled
        }
        "backspace" => {
            buf.backspace();
            Outcome::Handled
        }
        "delete" => {
            buf.delete();
            Outcome::Handled
        }
        "tab" => {
            buf.insert_str("  ");
            Outcome::Handled
        }
        "left" => {
            buf.move_left(shift);
            Outcome::Handled
        }
        "right" => {
            buf.move_right(shift);
            Outcome::Handled
        }
        "up" => {
            buf.move_up(1, shift);
            Outcome::Handled
        }
        "down" => {
            buf.move_down(1, shift);
            Outcome::Handled
        }
        "home" => {
            buf.move_home(shift);
            Outcome::Handled
        }
        "end" => {
            buf.move_end(shift);
            Outcome::Handled
        }
        "pageup" => {
            buf.move_up(VISIBLE_LINES, shift);
            Outcome::Handled
        }
        "pagedown" => {
            buf.move_down(VISIBLE_LINES, shift);
            Outcome::Handled
        }
        "space" => {
            buf.insert_str(" ");
            Outcome::Handled
        }
        _ => match ch {
            Some(c) if !c.is_empty() => {
                buf.insert_str(c);
                Outcome::Handled
            }
            _ => Outcome::Ignored,
        },
    }
}

/// Parte da seleção que cai na linha `line`, em colunas de caractere (`end` exclusivo).
pub fn selection_on_line(buf: &TextBuffer, line: usize) -> Option<(usize, usize)> {
    let (s, e) = buf.selection()?;
    if line < s.line || line > e.line {
        return None;
    }
    let len = buf.line(line).chars().count();
    let from = if line == s.line { s.col } else { 0 };
    let to = if line == e.line { e.col } else { len };
    // uma linha inteira selecionada mostra também o "fim de linha": um espaço a mais
    (from < to || (line < e.line && from <= len)).then_some((from, to.max(from)))
}

/// Pedaço de uma linha: texto, cor, se está selecionado e se o cursor fica logo antes dele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub tok: Tok,
    pub sel: bool,
    pub caret_before: bool,
}

/// Corta os pedaços coloridos de uma linha nas bordas da seleção e na posição do cursor.
pub fn line_segments(tokens: &[(String, Tok)], cursor_col: Option<usize>, sel: Option<(usize, usize)>) -> Vec<Seg> {
    let mut out: Vec<Seg> = Vec::new();
    let mut at = 0usize;
    let mut caret_done = false;
    for (text, tok) in tokens {
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0usize;
        while start < chars.len() {
            let abs = at + start;
            // próximo ponto de corte dentro deste pedaço
            let mut end = chars.len();
            for cut in [sel.map(|s| s.0), sel.map(|s| s.1), cursor_col].into_iter().flatten() {
                if cut > abs && cut < at + chars.len() {
                    end = end.min(cut - at);
                }
            }
            let caret = !caret_done && cursor_col == Some(abs);
            caret_done |= caret;
            out.push(Seg { text: chars[start..end].iter().collect(), tok: *tok, sel: sel.is_some_and(|(a, b)| abs >= a && abs < b), caret_before: caret });
            start = end;
        }
        at += chars.len();
    }
    // cursor depois do último caractere (ou numa linha vazia)
    if !caret_done && cursor_col.is_some_and(|c| c >= at) {
        out.push(Seg { text: String::new(), tok: Tok::Plain, sel: false, caret_before: true });
    }
    out
}

/// Posição do texto sob o ponto (x, y) dado em pixels a partir do canto do texto; arredonda para o caractere mais
/// próximo e fica dentro do texto.
pub fn pos_at(buf: &TextBuffer, scroll: usize, x: f32, y: f32, char_w: f32) -> Pos {
    let line = (scroll + (y / LINE_H).floor().max(0.0) as usize).min(buf.line_count().saturating_sub(1));
    let col = ((x / char_w).round().max(0.0) as usize).min(buf.line(line).chars().count());
    Pos::new(line, col)
}

/// Ajusta a rolagem para o cursor ficar à vista.
pub fn scroll_to_cursor(scroll: usize, cursor_line: usize, line_count: usize) -> usize {
    let max = line_count.saturating_sub(VISIBLE_LINES);
    let mut s = scroll.min(max);
    if cursor_line < s {
        s = cursor_line;
    } else if cursor_line >= s + VISIBLE_LINES {
        s = cursor_line + 1 - VISIBLE_LINES;
    }
    s.min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(t: &str) -> TextBuffer {
        TextBuffer::new(t)
    }

    fn press(b: &mut TextBuffer, key: &str) -> Outcome {
        handle_key(b, key, None, false, false, false)
    }

    fn type_text(b: &mut TextBuffer, t: &str) {
        for c in t.chars() {
            let s = c.to_string();
            handle_key(b, &s, Some(&s), false, false, false);
        }
    }

    #[test]
    fn typing_edits_in_the_middle_not_only_at_the_end() {
        let mut b = buf("SELECT 1");
        b.move_doc_start(false);
        for _ in 0..6 {
            press(&mut b, "right");
        }
        type_text(&mut b, " *,");
        assert_eq!(b.text(), "SELECT *, 1");
        press(&mut b, "backspace");
        assert_eq!(b.text(), "SELECT * 1");
        press(&mut b, "delete");
        assert_eq!(b.text(), "SELECT *1");
        press(&mut b, "enter");
        assert_eq!(b.text(), "SELECT *\n1");
        assert_eq!(b.cursor(), Pos::new(1, 0));
    }

    #[test]
    fn shift_arrows_select_and_typing_replaces_the_selection() {
        let mut b = buf("DELETE FROM t");
        b.move_doc_start(false);
        for _ in 0..6 {
            handle_key(&mut b, "right", None, false, true, false);
        }
        assert_eq!(b.selected_text().as_deref(), Some("DELETE"));
        type_text(&mut b, "SELECT *");
        assert_eq!(b.text(), "SELECT * FROM t");
        assert!(b.selection().is_none());
    }

    #[test]
    fn select_all_copy_cut_paste_and_undo() {
        let mut b = buf("a\nb");
        assert_eq!(handle_key(&mut b, "a", Some("a"), true, false, false), Outcome::Handled);
        assert_eq!(handle_key(&mut b, "c", Some("c"), true, false, false), Outcome::Copy("a\nb".into()));
        assert_eq!(b.text(), "a\nb");
        assert_eq!(handle_key(&mut b, "x", Some("x"), true, false, false), Outcome::Cut("a\nb".into()));
        assert_eq!(b.text(), "");
        assert_eq!(handle_key(&mut b, "z", Some("z"), true, false, false), Outcome::Handled);
        assert_eq!(b.text(), "a\nb");
        handle_key(&mut b, "y", Some("y"), true, false, false);
        assert_eq!(b.text(), "");
        assert_eq!(handle_key(&mut b, "v", Some("v"), true, false, false), Outcome::Paste);
        // copiar sem seleção não faz nada e não é erro
        assert_eq!(handle_key(&mut b, "c", Some("c"), true, false, false), Outcome::Handled);
    }

    #[test]
    fn run_blur_and_global_shortcuts() {
        let mut b = buf("x");
        assert_eq!(handle_key(&mut b, "enter", None, true, false, false), Outcome::Run);
        assert_eq!(press(&mut b, "escape"), Outcome::Blur);
        // Ctrl ou Alt com outra tecla não é do editor: segue para os atalhos do app
        assert_eq!(handle_key(&mut b, "t", Some("t"), true, false, false), Outcome::Ignored);
        assert_eq!(handle_key(&mut b, "1", Some("1"), false, false, true), Outcome::Ignored);
        assert_eq!(press(&mut b, "f5"), Outcome::Ignored);
        assert_eq!(b.text(), "x");
    }

    #[test]
    fn word_and_line_movement() {
        let mut b = buf("SELECT nome FROM clientes");
        b.move_doc_end(false);
        handle_key(&mut b, "left", None, true, false, false);
        assert_eq!(b.cursor(), Pos::new(0, 17));
        handle_key(&mut b, "home", None, false, true, false);
        assert_eq!(b.selected_text().as_deref(), Some("SELECT nome FROM "));
        press(&mut b, "end");
        assert_eq!(b.cursor(), Pos::new(0, 25));
        handle_key(&mut b, "backspace", None, true, false, false);
        assert_eq!(b.text(), "SELECT nome FROM ");
        let mut m = buf("um\ndois\ntres");
        handle_key(&mut m, "home", None, true, false, false);
        assert_eq!(m.cursor(), Pos::new(0, 0));
        handle_key(&mut m, "end", None, true, false, false);
        assert_eq!(m.cursor(), Pos::new(2, 4));
    }

    #[test]
    fn tab_inserts_two_spaces() {
        let mut b = buf("");
        press(&mut b, "tab");
        type_text(&mut b, "x");
        assert_eq!(b.text(), "  x");
    }

    #[test]
    fn selection_is_reported_per_line() {
        let mut b = buf("abc\ndef\nghi");
        b.move_to(Pos::new(0, 1), false);
        b.move_to(Pos::new(2, 2), true);
        assert_eq!(selection_on_line(&b, 0), Some((1, 3)));
        assert_eq!(selection_on_line(&b, 1), Some((0, 3)));
        assert_eq!(selection_on_line(&b, 2), Some((0, 2)));
        assert_eq!(selection_on_line(&b, 3), None);
        let mut one = buf("abcdef");
        one.move_to(Pos::new(0, 2), false);
        one.move_to(Pos::new(0, 4), true);
        assert_eq!(selection_on_line(&one, 0), Some((2, 4)));
        assert_eq!(selection_on_line(&buf("sem seleção"), 0), None);
    }

    #[test]
    fn segments_split_at_selection_and_caret_without_losing_text() {
        let tokens = vec![("SELECT".to_string(), Tok::Keyword), (" nome ".to_string(), Tok::Plain), ("FROM".to_string(), Tok::Keyword)];
        let line = "SELECT nome FROM";
        for cursor in [None, Some(0), Some(3), Some(6), Some(8), Some(16)] {
            for sel in [None, Some((2, 9)), Some((0, 16)), Some((6, 7))] {
                let segs = line_segments(&tokens, cursor, sel);
                assert_eq!(segs.iter().map(|s| s.text.as_str()).collect::<String>(), line, "cursor {cursor:?} sel {sel:?}");
                assert_eq!(segs.iter().filter(|s| s.caret_before).count(), usize::from(cursor.is_some()), "{cursor:?}");
                let selected: String = segs.iter().filter(|s| s.sel).map(|s| s.text.as_str()).collect();
                let expect: String = sel.map(|(a, b)| line.chars().skip(a).take(b - a).collect()).unwrap_or_default();
                assert_eq!(selected, expect, "cursor {cursor:?} sel {sel:?}");
            }
        }
        // cursor no fim da linha e numa linha vazia
        assert!(line_segments(&tokens, Some(16), None).last().unwrap().caret_before);
        assert_eq!(line_segments(&[], Some(0), None), vec![Seg { text: String::new(), tok: Tok::Plain, sel: false, caret_before: true }]);
        // as cores se mantêm
        let segs = line_segments(&tokens, None, Some((2, 9)));
        assert!(segs.iter().any(|s| s.tok == Tok::Keyword && s.sel && s.text == "LECT"));
    }

    #[test]
    fn clicks_map_to_the_nearest_character_and_stay_inside_the_text() {
        let b = buf("abcdef\nxy");
        assert_eq!(pos_at(&b, 0, 0.0, 0.0, 7.0), Pos::new(0, 0));
        assert_eq!(pos_at(&b, 0, 15.0, 5.0, 7.0), Pos::new(0, 2));
        assert_eq!(pos_at(&b, 0, 3.0, 0.0, 7.0), Pos::new(0, 0)); // 3/7 arredonda para 0
        assert_eq!(pos_at(&b, 0, 4.0, 0.0, 7.0), Pos::new(0, 1));
        assert_eq!(pos_at(&b, 0, 500.0, 25.0, 7.0), Pos::new(1, 2)); // depois do fim: no fim da linha
        assert_eq!(pos_at(&b, 0, 5.0, 900.0, 7.0), Pos::new(1, 1)); // abaixo do texto: última linha
        assert_eq!(pos_at(&b, 1, 5.0, 0.0, 7.0), Pos::new(1, 1)); // rolado: a primeira linha visível é a 1
        assert_eq!(pos_at(&b, 0, -9.0, -9.0, 7.0), Pos::new(0, 0));
    }

    #[test]
    fn scrolling_keeps_the_cursor_in_view() {
        assert_eq!(scroll_to_cursor(0, 3, 10), 0);
        assert_eq!(scroll_to_cursor(0, 9, 10), 3); // 10 linhas, 7 visíveis: rola 3
        assert_eq!(scroll_to_cursor(3, 1, 10), 1);
        assert_eq!(scroll_to_cursor(5, 4, 3), 0); // texto curto: sem rolagem
        assert_eq!(scroll_to_cursor(0, 0, 0), 0);
    }
}
