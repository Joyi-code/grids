//! Platform layer for Grids.
//!
//! One API for the app; `#[cfg(windows)]` implementations use the Windows
//! Shell (ShellExecuteW, IFileOperation, IContextMenu, CF_HDROP ...), other
//! platforms fall back to `std` or do nothing so the app builds and runs on
//! Linux for screenshots and debugging.
//!
//! Blocking calls (`copy`, `move_`, `recycle`, `delete`) may show a system
//! progress dialog on Windows; the app calls them from a background thread.
//! Owned by the shell/CI agent; this file is the agreed skeleton.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use image::RgbaImage;

/// A mounted drive / volume root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    /// Root path, e.g. `C:\` or `/`.
    pub path: PathBuf,
    /// Volume label, may be empty.
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KnownFolderKind {
    Home,
    Desktop,
    Downloads,
    Documents,
    Pictures,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownFolder {
    pub kind: KnownFolderKind,
    pub path: PathBuf,
}

/// Opens a file or folder with its default program (ShellExecuteW "open").
pub fn open_with_system(path: &Path) -> io::Result<()> {
    let _ = path;
    Ok(())
}

/// Shows the file in the system file manager, selected.
pub fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    let _ = path;
    Ok(())
}

/// The system icon for a file, cached by extension. `None` when the platform
/// has no icons; the UI then draws its own type-coloured icon.
pub fn file_icon(path: &Path, is_dir: bool, size: u32) -> Option<Arc<RgbaImage>> {
    let _ = (path, is_dir, size);
    None
}

/// Pops up the native context menu for `paths` at `screen_pos` (physical
/// screen pixels) and runs the chosen verb. `hwnd` is the raw window handle
/// (0 when unknown). Returns true when a command was invoked, so the caller
/// can refresh. No-op elsewhere.
pub fn show_context_menu(hwnd: isize, paths: &[PathBuf], screen_pos: (i32, i32)) -> bool {
    let _ = (hwnd, paths, screen_pos);
    false
}

/// Moves to the recycle bin (IFileOperation, FOF_ALLOWUNDO).
pub fn recycle(paths: &[PathBuf]) -> io::Result<()> {
    delete(paths)
}

/// Deletes permanently.
pub fn delete(paths: &[PathBuf]) -> io::Result<()> {
    for p in paths {
        if p.is_dir() {
            std::fs::remove_dir_all(p)?;
        } else {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}

/// Copies `paths` into the folder `dest` (system progress dialog on Windows).
pub fn copy(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    let _ = (paths, dest);
    Err(io::Error::other("todo"))
}

/// Moves `paths` into the folder `dest`.
pub fn move_(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    let _ = (paths, dest);
    Err(io::Error::other("todo"))
}

/// Renames `path` to `new_name` in the same folder; returns the new path.
pub fn rename(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    let to = path.with_file_name(new_name);
    std::fs::rename(path, &to)?;
    Ok(to)
}

/// Drives / volume roots. `["/"]` on non-Windows.
pub fn drives() -> Vec<Drive> {
    vec![Drive { path: PathBuf::from("/"), label: String::new() }]
}

/// Desktop, Downloads, Documents, Pictures (and Home), those that exist.
pub fn known_folders() -> Vec<KnownFolder> {
    Vec::new()
}

/// Puts files on the system clipboard (CF_HDROP + Preferred DropEffect).
pub fn clipboard_set_files(paths: &[PathBuf], cut: bool) -> io::Result<()> {
    let _ = (paths, cut);
    Ok(())
}

/// Files on the system clipboard and whether they were cut.
pub fn clipboard_get_files() -> Option<(Vec<PathBuf>, bool)> {
    None
}

/// True when both paths are on the same volume (drag = move, else copy).
pub fn same_volume(a: &Path, b: &Path) -> bool {
    let _ = (a, b);
    true
}
