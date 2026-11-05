//! Recursos embutidos no binário: ícones SVG (do Warp, AGPL-3.0) e as fontes Hack e Roboto.
//! Arquivo gerado a partir de `apps/brasa/assets/` — veja `NOTICE.md`.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

static FILES: &[(&str, &[u8])] = &[
    // logo da Brasa: a chama com o prompt e o nome (cópia de branding/brasa-logo-dark.png)
    ("brand/brasa-logo.png", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/brand/brasa-logo.png"))),
    ("icons/add.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/add.svg"))),
    ("icons/alert-triangle.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/alert-triangle.svg"))),
    ("icons/arrow-left.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/arrow-left.svg"))),
    ("icons/arrow-right.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/arrow-right.svg"))),
    ("icons/block-bulletedlist.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/block-bulletedlist.svg"))),
    ("icons/block-command.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/block-command.svg"))),
    ("icons/block-ordered-list.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/block-ordered-list.svg"))),
    ("icons/block-tasklist.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/block-tasklist.svg"))),
    ("icons/block-text.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/block-text.svg"))),
    ("icons/bookmark.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/bookmark.svg"))),
    ("icons/check.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/check.svg"))),
    ("icons/chevron-down.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/chevron-down.svg"))),
    ("icons/chevron-right.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/chevron-right.svg"))),
    ("icons/clock.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/clock.svg"))),
    ("icons/code-01.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/code-01.svg"))),
    ("icons/copy.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/copy.svg"))),
    ("icons/dots-horizontal.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/dots-horizontal.svg"))),
    ("icons/dots-vertical.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/dots-vertical.svg"))),
    ("icons/edit-bold.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/edit-bold.svg"))),
    ("icons/edit-code.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/edit-code.svg"))),
    ("icons/edit-italic.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/edit-italic.svg"))),
    ("icons/edit-strikethrough.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/edit-strikethrough.svg"))),
    ("icons/eye.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/eye.svg"))),
    ("icons/file-code-02.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/file-code-02.svg"))),
    ("icons/file.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/file.svg"))),
    ("icons/filter-funnel.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/filter-funnel.svg"))),
    ("icons/folder-closed.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/folder-closed.svg"))),
    ("icons/folder.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/folder.svg"))),
    ("icons/git-branch-02.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/git-branch-02.svg"))),
    ("icons/layout-left.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/layout-left.svg"))),
    ("icons/link-03.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/link-03.svg"))),
    ("icons/maximize-01.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/maximize-01.svg"))),
    ("icons/menu-01.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/menu-01.svg"))),
    ("icons/minimize-01.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/minimize-01.svg"))),
    ("icons/minus.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/minus.svg"))),
    ("icons/notebook.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/notebook.svg"))),
    ("icons/pencil-02.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/pencil-02.svg"))),
    ("icons/play-white.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/play-white.svg"))),
    ("icons/plus-circle.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/plus-circle.svg"))),
    ("icons/plus.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/plus.svg"))),
    ("icons/prompt.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/prompt.svg"))),
    ("icons/refresh.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/refresh.svg"))),
    ("icons/save-01.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/save-01.svg"))),
    ("icons/search.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/search.svg"))),
    ("icons/settings.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/settings.svg"))),
    ("icons/share-01.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/share-01.svg"))),
    ("icons/sort.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/sort.svg"))),
    ("icons/terminal-input.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/terminal-input.svg"))),
    ("icons/terminal.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/terminal.svg"))),
    ("icons/trash-02.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/trash-02.svg"))),
    ("icons/warp-drive.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/warp-drive.svg"))),
    ("icons/workflow.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/workflow.svg"))),
    ("icons/x-close.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/x-close.svg"))),
    ("file_type/angular.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/angular.svg"))),
    ("file_type/c.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/c.svg"))),
    ("file_type/cpp.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/cpp.svg"))),
    ("file_type/cython.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/cython.svg"))),
    ("file_type/flash.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/flash.svg"))),
    ("file_type/go.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/go.svg"))),
    ("file_type/javascript.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/javascript.svg"))),
    ("file_type/json.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/json.svg"))),
    ("file_type/kotlin.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/kotlin.svg"))),
    ("file_type/markdown.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/markdown.svg"))),
    ("file_type/mermaid.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/mermaid.svg"))),
    ("file_type/npm.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/npm.svg"))),
    ("file_type/perl.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/perl.svg"))),
    ("file_type/php.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/php.svg"))),
    ("file_type/python.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/python.svg"))),
    ("file_type/rust.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/rust.svg"))),
    ("file_type/sql.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/sql.svg"))),
    ("file_type/terraform.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/terraform.svg"))),
    ("file_type/typescript.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/typescript.svg"))),
    ("file_type/wasm.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/wasm.svg"))),
    ("file_type/zig.svg", include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/file_type/zig.svg"))),
];

static FONTS: &[&[u8]] = &[
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/hack/Hack-Bold.ttf")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/hack/Hack-BoldItalic.ttf")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/hack/Hack-Italic.ttf")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/hack/Hack-Regular.ttf")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/roboto/Roboto-Bold.ttf")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/roboto/Roboto-Italic.ttf")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/roboto/Roboto-Medium.ttf")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/roboto/Roboto-Regular.ttf")),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

/// Fontes para registrar no início (Hack = terminal e código; Roboto = interface e texto das notas).
pub fn fonts() -> Vec<Cow<'static, [u8]>> {
    FONTS.iter().map(|b| Cow::Borrowed(*b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icons_and_fonts_load() {
        assert!(Assets.load("icons/copy.svg").unwrap().is_some_and(|b| b.starts_with(b"<svg")));
        assert!(Assets.load("file_type/python.svg").unwrap().is_some());
        assert!(Assets.load("icons/nao-existe.svg").unwrap().is_none());
        assert!(Assets.list("file_type/").unwrap().len() >= 20);
        assert_eq!(fonts().len(), 8);
    }
}
