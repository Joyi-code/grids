//! App-wide state shared by the workspace and its panes: the settings file,
//! the file clipboard fallback, the local UTC offset and the icon cache.

use grids_core::format::Lang;
use grids_core::Config;
use gpui::{App, Global, RenderImage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Shared {
    pub config: Config,
    pub config_path: PathBuf,
    /// Files copied or cut in this app when the system clipboard cannot
    /// carry them (non-Windows).
    pub clipboard: Option<(Vec<PathBuf>, bool)>,
    /// Seconds east of UTC, for the date column.
    pub utc_offset: i64,
    /// None while loading on a background thread, then the icon (or None: fallback).
    icons: HashMap<String, Option<Option<Arc<RenderImage>>>>,
    new_items: Option<Arc<Vec<NewItem>>>,
}

impl Global for Shared {}

impl Shared {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        let utc_offset = chrono::Local::now().offset().local_minus_utc() as i64;
        Self { config, config_path, clipboard: None, utc_offset, icons: HashMap::new(), new_items: None }
    }
}

pub fn shared(cx: &App) -> &Shared { cx.global::<Shared>() }
pub fn lang(cx: &App) -> Lang { cx.global::<Shared>().config.lang }
pub fn en(cx: &App) -> bool { lang(cx) == Lang::En }
/// The read-only folder covering `path` (itself or an ancestor), if any.
pub fn locked_root(cx: &App, path: &Path) -> Option<PathBuf> {
    grids_core::locked::locked_root(&shared(cx).config.locked, path).cloned()
}
/// The first read-only folder covering any of `paths`.
pub fn locked_any(cx: &App, paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find_map(|p| locked_root(cx, p))
}
/// The string for the interface language.
pub fn tr(cx: &App, zh: &'static str, en: &'static str) -> &'static str { if self::en(cx) { en } else { zh } }

/// Puts files on the clipboard: the system's, and ours as a fallback.
pub fn set_clipboard(paths: Vec<PathBuf>, cut: bool, cx: &mut App) {
    let _ = grids_shell::clipboard_set_files(&paths, cut);
    cx.global_mut::<Shared>().clipboard = Some((paths, cut));
}

/// Files to paste: the system clipboard first (copied in Explorer too).
pub fn get_clipboard(cx: &App) -> Option<(Vec<PathBuf>, bool)> {
    grids_shell::clipboard_get_files().filter(|(p, _)| !p.is_empty()).or_else(|| shared(cx).clipboard.clone())
}

/// The system icon for `path`, cached by extension (by path for programs,
/// shortcuts and icons, which carry their own). Loaded on a background thread:
/// shell icon lookups can touch the disk or the network, so the first frames
/// draw the fallback. None: draw the fallback.
/// GPUI wants BGRA frames; the shell hands out RGBA.
pub fn render_image(rgba: &image::RgbaImage) -> Arc<RenderImage> {
    let mut bgra = rgba.clone();
    for p in bgra.pixels_mut() { p.0.swap(0, 2) }
    Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(bgra)]))
}

/// A "New" menu entry (ShellNew templates), with its icon ready to draw.
pub struct NewItem {
    pub ext: String,
    pub name: String,
    pub icon: Option<Arc<RenderImage>>,
}

/// The "New" templates. The first call starts reading them from the
/// registry on a background thread and returns an empty list until then.
pub fn new_items(cx: &mut App) -> Arc<Vec<NewItem>> {
    let s = cx.global_mut::<Shared>();
    if let Some(v) = &s.new_items { return v.clone() }
    s.new_items = Some(Arc::new(Vec::new()));
    cx.spawn(async move |cx| {
        let list = cx.background_executor().spawn(async move {
            std::panic::catch_unwind(grids_shell::new_items).unwrap_or_default().into_iter()
                .map(|n| NewItem { ext: n.ext, name: n.name, icon: n.icon.map(|i| render_image(&i)) }).collect::<Vec<_>>()
        }).await;
        let _ = cx.update(|cx| cx.global_mut::<Shared>().new_items = Some(Arc::new(list)));
    }).detach();
    Arc::new(Vec::new())
}

pub fn file_icon(path: &Path, is_dir: bool, ext: &str, cx: &mut App) -> Option<Arc<RenderImage>> {
    if !cfg!(windows) { return None }
    let key = if is_dir { "\0dir".to_string() } else if matches!(ext, "exe" | "lnk" | "ico" | "url" | "appref-ms") { path.to_string_lossy().into_owned() } else { ext.to_string() };
    let s = cx.global_mut::<Shared>();
    if let Some(v) = s.icons.get(&key) { return v.clone().flatten() }
    s.icons.insert(key.clone(), None);
    let path = path.to_path_buf();
    cx.spawn(async move |cx| {
        let img = cx.background_executor().spawn(async move {
            std::panic::catch_unwind(|| grids_shell::file_icon(&path, is_dir, 32)).ok().flatten().map(|rgba| render_image(&rgba))
        }).await;
        let _ = cx.update(|cx| {
            cx.global_mut::<Shared>().icons.insert(key, Some(img));
            cx.refresh_windows();
        });
    }).detach();
    None
}
