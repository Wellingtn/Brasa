//! Ícones por tipo de arquivo. Usa glifos da Nerd Font quando ela está instalada;
//! sem ela, cai para um selo de texto com a extensão.

use gpui::{div, prelude::*, px, rgb, svg, Div, FontWeight, SharedString};
use ox_files::EntryKind;

pub struct Icon {
    /// SVG embutido (ícones do Warp); tem prioridade sobre o glifo
    pub svg: Option<&'static str>,
    pub glyph: char,
    pub color: u32,
    /// texto usado quando não há Nerd Font
    pub fallback: String,
}

fn icon(glyph: char, color: u32, fallback: &str) -> Icon {
    Icon { svg: None, glyph, color, fallback: fallback.to_string() }
}

fn svg_icon(path: &'static str, color: u32) -> Icon {
    Icon { svg: Some(path), glyph: ' ', color, fallback: String::new() }
}

/// Ícone SVG do Warp por nome (ex.: "copy", "terminal"), já tingido.
pub fn wicon(name: &str, size: f32, color: u32) -> gpui::Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).flex_none().size(px(size)).text_color(rgb(color))
}

// glifos Nerd Font (v3)
pub const FOLDER: char = '\u{f07b}';
pub const FOLDER_OPEN: char = '\u{f07c}';
pub const TERMINAL: char = '\u{f489}';
pub const BRANCH: char = '\u{e725}';
const FILE_TEXT: char = '\u{f15c}';
const FILE_GENERIC: char = '\u{f15b}';
const FILE_CODE: char = '\u{f1c9}';
const LINK: char = '\u{f0c1}';
pub const CHEVRON_RIGHT: char = '\u{f054}';
pub const CHEVRON_DOWN: char = '\u{f078}';

const COLOR_FOLDER: u32 = 0x7aa2f7;
const COLOR_TEXT: u32 = 0x9aa0aa;

pub fn folder_icon(expanded: bool) -> Icon {
    let _ = (FOLDER_OPEN, FOLDER);
    svg_icon(if expanded { "icons/folder.svg" } else { "icons/folder-closed.svg" }, COLOR_FOLDER)
}

/// Ícone de uma entrada da árvore de arquivos.
pub fn file_icon(name: &str, kind: &EntryKind, expanded: bool) -> Icon {
    match kind {
        EntryKind::Dir => return folder_icon(expanded),
        EntryKind::Symlink => return icon(LINK, 0x56b6c2, "LNK"),
        EntryKind::File => {}
    }
    let lower = name.to_lowercase();
    // nomes especiais primeiro
    match lower.as_str() {
        "dockerfile" | "docker-compose.yml" | "docker-compose.yaml" | "compose.yaml" | "compose.yml" => return icon('\u{f308}', 0x2496ed, "DOCK"),
        ".gitignore" | ".gitattributes" | ".gitmodules" => return icon('\u{e702}', 0xf05033, "GIT"),
        "makefile" | "cmakelists.txt" => return icon('\u{f013}', 0x9aa0aa, "MAKE"),
        "cargo.toml" | "cargo.lock" => return icon('\u{e7a8}', 0xdea584, "RS"),
        "package.json" | "package-lock.json" => return Icon { svg: Some("file_type/npm.svg"), ..icon('\u{e74e}', 0xcb3837, "NPM") },
        _ if lower.starts_with("readme") => return icon('\u{e73e}', 0x519aba, "MD"),
        _ => {}
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let (glyph, color) = match ext {
        "py" | "pyw" | "pyi" => ('\u{e73c}', 0x5ba4cf),
        "java" | "jar" => ('\u{e738}', 0xe76f00),
        "kt" | "kts" => ('\u{e634}', 0xa97bff),
        "rs" => ('\u{e7a8}', 0xdea584),
        "js" | "mjs" | "cjs" => ('\u{e74e}', 0xf1e05a),
        "ts" => ('\u{e628}', 0x3178c6),
        "jsx" | "tsx" => ('\u{e7ba}', 0x61dafb),
        "html" | "htm" => ('\u{e736}', 0xe34c26),
        "css" => ('\u{e749}', 0x7a5ccf),
        "scss" | "sass" => ('\u{e603}', 0xcd6799),
        "json" | "jsonc" => ('\u{e60b}', 0xcbcb41),
        "md" | "markdown" => ('\u{e73e}', 0x519aba),
        "sh" | "bash" | "zsh" | "fish" => (TERMINAL, 0x89e051),
        "go" => ('\u{e627}', 0x00add8),
        "c" | "h" => ('\u{e61e}', 0x599eff),
        "cpp" | "cc" | "cxx" | "hpp" => ('\u{e61d}', 0xf34b7d),
        "php" => ('\u{e73d}', 0x8892bf),
        "rb" => ('\u{e791}', 0xcc342d),
        "swift" => ('\u{e755}', 0xf05138),
        "lua" => ('\u{e620}', 0x51a0cf),
        "vue" => ('\u{e6a0}', 0x41b883),
        "sql" | "db" | "sqlite" => ('\u{f1c0}', 0xdad8d8),
        "toml" | "ini" | "conf" | "cfg" | "env" => ('\u{f013}', 0x9aa0aa),
        "yml" | "yaml" | "xml" => (FILE_CODE, 0xcb7676),
        "lock" => ('\u{f023}', 0x8a8c96),
        "txt" | "log" | "rtf" => (FILE_TEXT, COLOR_TEXT),
        "csv" | "xls" | "xlsx" => ('\u{f1c3}', 0x57c785),
        "doc" | "docx" | "odt" => ('\u{f1c2}', 0x4f8cd6),
        "pdf" => ('\u{f1c1}', 0xf85149),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" => ('\u{f1c5}', 0xb392f0),
        "mp3" | "wav" | "flac" | "ogg" => ('\u{f1c7}', 0xe8825f),
        "mp4" | "mkv" | "mov" | "avi" | "webm" => ('\u{f1c8}', 0xe8825f),
        "zip" | "tar" | "gz" | "xz" | "7z" | "bz2" | "deb" | "rpm" => ('\u{f1c6}', 0xe8825f),
        _ => (FILE_GENERIC, 0x8a8c96),
    };
    let fallback = if ext.is_empty() { "FILE".to_string() } else { ext.chars().take(4).collect::<String>().to_uppercase() };
    let svg = match ext {
        "py" | "pyw" | "pyi" => Some("file_type/python.svg"),
        "pyx" => Some("file_type/cython.svg"),
        "rs" => Some("file_type/rust.svg"),
        "js" | "mjs" | "cjs" | "jsx" => Some("file_type/javascript.svg"),
        "ts" | "tsx" => Some("file_type/typescript.svg"),
        "json" | "jsonc" => Some("file_type/json.svg"),
        "md" | "markdown" => Some("file_type/markdown.svg"),
        "go" => Some("file_type/go.svg"),
        "c" | "h" => Some("file_type/c.svg"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("file_type/cpp.svg"),
        "php" => Some("file_type/php.svg"),
        "pl" | "pm" => Some("file_type/perl.svg"),
        "sql" => Some("file_type/sql.svg"),
        "kt" | "kts" => Some("file_type/kotlin.svg"),
        "zig" => Some("file_type/zig.svg"),
        "wasm" | "wat" => Some("file_type/wasm.svg"),
        "tf" | "tfvars" => Some("file_type/terraform.svg"),
        "mmd" => Some("file_type/mermaid.svg"),
        "sh" | "bash" | "zsh" | "fish" => Some("icons/terminal.svg"),
        _ => Some("icons/file.svg"),
    };
    Icon { svg, glyph, color, fallback }
}

/// Ícone na árvore de notas.
pub fn note_icon(is_dir: bool, expanded: bool) -> Icon {
    if is_dir {
        folder_icon(expanded)
    } else {
        svg_icon("icons/notebook.svg", 0x8fb8e8)
    }
}

/// Escolhe uma fonte com os glifos, se houver alguma Nerd Font instalada.
pub fn find_icon_font(families: &[String]) -> Option<SharedString> {
    families
        .iter()
        .find(|f| f.contains("Symbols Nerd Font"))
        .or_else(|| families.iter().find(|f| f.contains("Nerd Font")))
        .map(|f| SharedString::from(f.clone()))
}

/// Chevron fino de expandir/recolher (coluna fixa, para alinhar ícones de pastas e arquivos).
pub fn chevron_el(font: &Option<SharedString>, expanded: bool) -> Div {
    let base = div().flex_none().w(px(14.)).flex().items_center().justify_center().text_color(rgb(0x8a8c96));
    match font {
        Some(f) => base.font_family(f.clone()).text_size(px(10.)).child((if expanded { CHEVRON_DOWN } else { CHEVRON_RIGHT }).to_string()),
        None => base.text_size(px(11.)).child(if expanded { "▾" } else { "▸" }),
    }
}

/// Desenha o ícone (glifo da Nerd Font ou o selo de texto de reserva).
pub fn icon_el(font: &Option<SharedString>, ic: &Icon, size: f32) -> Div {
    if let Some(path) = ic.svg {
        return div().flex_none().size(px(size + 2.)).flex().items_center().justify_center().child(svg().path(path).size(px(size)).text_color(rgb(ic.color)));
    }
    match font {
        Some(f) => div().flex_none().min_w(px(size + 4.)).font_family(f.clone()).text_size(px(size)).line_height(px(size + 2.)).text_color(rgb(ic.color)).child(ic.glyph.to_string()),
        None => div().flex_none().text_size(px(size * 0.7)).font_weight(FontWeight::BOLD).text_color(rgb(ic.color)).child(ic.fallback.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(name: &str) -> char {
        file_icon(name, &EntryKind::File, false).glyph
    }

    #[test]
    fn language_icons_by_extension() {
        assert_eq!(glyph("main.py"), '\u{e73c}');
        assert_eq!(glyph("App.java"), '\u{e738}');
        assert_eq!(glyph("lib.rs"), '\u{e7a8}');
        assert_eq!(glyph("index.TS"), '\u{e628}'); // sem distinguir caixa
        assert_eq!(glyph("notas.txt"), FILE_TEXT);
        assert_eq!(glyph("foto.png"), '\u{f1c5}');
        assert_eq!(glyph("pacote.tar"), '\u{f1c6}');
        assert_eq!(glyph("semextensao"), FILE_GENERIC);
    }

    #[test]
    fn special_names_win_over_extension() {
        assert_eq!(glyph("Dockerfile"), '\u{f308}');
        assert_eq!(glyph(".gitignore"), '\u{e702}');
        assert_eq!(glyph("README.md"), '\u{e73e}');
        assert_eq!(glyph("Cargo.toml"), '\u{e7a8}'); // e não o ícone genérico de .toml
        assert_eq!(glyph("Makefile"), '\u{f013}');
    }

    #[test]
    fn folders_open_closed_and_symlinks() {
        assert_eq!(file_icon("src", &EntryKind::Dir, false).svg, Some("icons/folder-closed.svg"));
        assert_eq!(file_icon("src", &EntryKind::Dir, true).svg, Some("icons/folder.svg"));
        assert_eq!(file_icon("x", &EntryKind::Symlink, false).glyph, LINK);
        assert_eq!(note_icon(true, true).svg, Some("icons/folder.svg"));
    }

    #[test]
    fn fallback_badge_without_nerd_font() {
        assert_eq!(file_icon("a.rs", &EntryKind::File, false).fallback, "RS");
        assert_eq!(file_icon("Makefile", &EntryKind::File, false).fallback, "MAKE");
        assert_eq!(file_icon("x.jpeg", &EntryKind::File, false).fallback, "JPEG");
    }

    #[test]
    fn picks_symbols_font_first_then_any_nerd_font() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let pick = |v: &[&str]| find_icon_font(&l(v)).map(|f| f.to_string());
        assert_eq!(pick(&["DejaVu Sans", "JetBrainsMono Nerd Font", "Symbols Nerd Font Mono"]).as_deref(), Some("Symbols Nerd Font Mono"));
        assert_eq!(pick(&["DejaVu Sans", "JetBrainsMono Nerd Font"]).as_deref(), Some("JetBrainsMono Nerd Font"));
        assert_eq!(pick(&["DejaVu Sans"]), None);
    }
}
