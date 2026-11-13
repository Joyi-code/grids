//! Assets compiled into the binary: Lucide icons (ISC, see THIRD_PARTY.md).

use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8])),*]
    };
}

static FILES: &[(&str, &[u8])] = icons![
    "app-window", "arrow-down-narrow-wide", "arrow-left", "arrow-right", "arrow-up", "arrow-up-down", "arrow-up-narrow-wide",
    "check", "chevron-down", "chevron-left", "chevron-right", "chevron-up", "clipboard-paste", "copy", "download", "ellipsis",
    "external-link", "eye", "eye-off", "file", "file-archive", "file-audio", "file-code", "file-cog", "file-image",
    "file-spreadsheet", "file-text", "file-video", "filter", "folder", "folder-fill", "folder-open", "folder-plus", "hard-drive",
    "house", "image", "layers", "layout-grid", "maximize-2", "minimize-2", "minus", "monitor", "moon", "panel-left",
    "panel-left-close", "panel-left-open", "pencil", "plus", "presentation", "refresh-cw", "save", "scissors", "search", "square", "star",
    "star-fill", "sun", "sun-moon", "trash-2", "x", "star-off", "folder-search", "square-split-horizontal", "link", "info",
    "terminal", "list", "layout-list", "grid-2x2", "lock", "lock-open", "inbox", "folder-input", "sticky-note",
    "history", "text-cursor-input", "list-checks", "table-2", "arrow-right-left",
];

/// The app icon, drawn beside "Grids" in the sidebar (48px, shown at 20).
static BRAND: (&str, &[u8]) = ("icon/grids-48.png", include_bytes!("../assets/icon/grids-48.png"));

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if path == BRAND.0 { return Ok(Some(Cow::Borrowed(BRAND.1))) }
        Ok(FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(FILES.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::FILES;

    /// Every `icon("…")` / `header_icon(id, "…", …)` in the sources names a bundled file;
    /// a missing one draws nothing (the hidden sidebar toggle was one).
    #[test]
    fn icons_used_are_bundled() {
        let srcs = [include_str!("pane.rs"), include_str!("workspace.rs"), include_str!("ui.rs"), include_str!("menu.rs"), include_str!("workspace/palette.rs")];
        let mut missing = vec![];
        for src in srcs {
            for (pat, skip) in [("icon(\"", 0), ("header_icon(", 1), ("glyph: \"", 0)] {
                for (i, _) in src.match_indices(pat) {
                    if pat == "icon(\"" && src[..i].ends_with("header_") { continue }
                    let rest = &src[i + pat.len()..];
                    let name = if skip == 0 { rest.split('"').next() } else { rest.split('"').nth(3).filter(|_| rest.split('"').nth(2).is_some_and(|s| s.trim() == ",")) };
                    if let Some(n) = name && !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                        && !FILES.iter().any(|(p, _)| *p == format!("icons/{n}.svg")) { missing.push(n.to_string()) }
                }
            }
        }
        assert!(missing.is_empty(), "icons not bundled: {missing:?}");
    }
}
