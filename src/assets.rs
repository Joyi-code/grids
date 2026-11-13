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
    "panel-left-close", "pencil", "plus", "presentation", "refresh-cw", "save", "scissors", "search", "square", "star",
    "star-fill", "sun", "sun-moon", "trash-2", "x",
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(FILES.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}
