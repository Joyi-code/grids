//! Platform layer for Grids.
//!
//! One API for the app; `#[cfg(windows)]` implementations use the Windows
//! Shell (ShellExecuteW, IFileOperation, IContextMenu, CF_HDROP ...), other
//! platforms fall back to `std` or do nothing so the app builds and runs on
//! Linux for screenshots and debugging.
//!
//! Threading rules for the app:
//! - `copy`, `move_`, `recycle`, `delete` block and may show the system
//!   progress / confirmation dialogs on Windows: call them from a background
//!   thread (`cx.background_spawn`). COM is initialised per thread here.
//! - `show_context_menu` / `context_menu` / `background_menu` run a modal menu
//!   loop on the calling thread, which must be the window's (UI) thread. Call
//!   them from a foreground task *outside* any `cx.update` borrow, e.g.
//!   `cx.spawn(async move |cx| { let r = grids_shell::context_menu(..); cx.update(..) })`,
//!   the same way GPUI runs its own file dialogs; a GPUI repaint arriving
//!   during the menu loop must not find the app already borrowed.
//! - `file_icon` is cached and cheap after the first call per extension; it
//!   is safe from any thread.

// Platform code runs on worker threads and inside window procedures: a panic
// there aborts the app, so failures are values, never unwraps.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use image::RgbaImage;

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as imp;

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
use other as imp;

mod zip;

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

/// Where a shortcut (.lnk, or a symlink elsewhere) points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// An existing folder (local or UNC).
    Folder(PathBuf),
    /// An existing file.
    File(PathBuf),
    /// A non-empty target path that does not exist (or cannot be reached).
    Missing(PathBuf),
    /// No file system target: an item-ID-only shortcut (Control Panel,
    /// `shell:` locations, ...).
    Other,
}

/// A program offered under "打开方式" (Open with) for a file type.
#[derive(Clone, Debug)]
pub struct Handler {
    /// Stable key to pass back to [`open_with`] (the handler's name as the
    /// shell reports it: an exe path, or an app ID for Store apps).
    pub id: String,
    /// Display name, e.g. "记事本".
    pub name: String,
    /// The program's icon, about 32 px; `None` when it could not be read.
    pub icon: Option<Arc<RgbaImage>>,
}

/// A file type Explorer offers under "新建" (New).
#[derive(Clone, Debug)]
pub struct NewItem {
    /// Extension with the dot, e.g. ".docx".
    pub ext: String,
    /// Localized type name, e.g. "Microsoft Word 文档" / "文本文档".
    pub name: String,
    /// The type's icon (32 px list); `None` off Windows.
    pub icon: Option<Arc<RgbaImage>>,
}

/// What the user did in a native context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Closed without choosing anything.
    Dismissed,
    /// A command ran (its canonical verb when the handler names one, such as
    /// `delete`, `cut`, `copy`, `paste`, `properties`; empty otherwise).
    /// The caller should refresh the folder.
    Invoked(String),
    /// "Rename" was chosen: the shell can only rename inside its own view, so
    /// nothing ran and the app should start its inline rename (F2).
    Rename,
}

/// Opens a file or folder with its default program (ShellExecuteW "open";
/// files without an association get the "Open with" dialog).
pub fn open_with_system(path: &Path) -> io::Result<()> {
    imp::open_with_system(path)
}

/// Reads where a shortcut points without resolving it: no link tracking, no
/// search, no UI, so it is safe on any thread and never re-enters the app.
/// On Windows `.lnk` files (env vars expanded); `Err` when the file is not a
/// shortcut. Elsewhere symlinks; any other file gives `Other`.
/// Checking whether the target exists may wait on the network for a UNC
/// target: call it from a background thread.
pub fn resolve_link(lnk: &Path) -> io::Result<LinkTarget> {
    imp::resolve_link(lnk)
}

/// Writes a shortcut at `lnk` pointing at `target` (a .lnk on Windows, a
/// symlink elsewhere).
pub fn create_link(lnk: &Path, target: &Path) -> io::Result<()> {
    imp::create_link(lnk, target)
}

/// Shows the file in the system file manager, selected.
pub fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    imp::reveal_in_explorer(path)
}

/// The system icon for a file, cached by extension (by path for `.exe`,
/// `.lnk`, `.ico` and other types whose icon lives in the file, and for
/// drive roots). `size` picks the nearest system list (16, 32, 48, 256 px);
/// the image comes back at the size the system drew it, which can be larger
/// on high-DPI screens. `None` when the platform has no icons; the UI then
/// draws its own type-coloured icon.
pub fn file_icon(path: &Path, is_dir: bool, size: u32) -> Option<Arc<RgbaImage>> {
    imp::file_icon(path, is_dir, size)
}

/// Pops up the native context menu for `paths` at `screen_pos` (physical
/// screen pixels) and runs the chosen verb. `hwnd` is the raw window handle
/// (0 when unknown, see [`hwnd_of`]). Returns true when a command was invoked
/// (or Rename was picked), so the caller can refresh. No-op elsewhere.
/// See [`context_menu`] to tell Rename apart.
pub fn show_context_menu(hwnd: isize, paths: &[PathBuf], screen_pos: (i32, i32)) -> bool {
    context_menu(hwnd, paths, screen_pos) != MenuAction::Dismissed
}

/// Like [`show_context_menu`], but says what was picked. All `paths` should
/// share one parent folder (as a selection in one pane does).
pub fn context_menu(hwnd: isize, paths: &[PathBuf], screen_pos: (i32, i32)) -> MenuAction {
    if paths.is_empty() {
        return MenuAction::Dismissed;
    }
    imp::context_menu(hwnd, paths, screen_pos)
}

/// The folder background menu (right click on empty space): New, Paste,
/// Properties and the shell extensions registered for backgrounds.
pub fn background_menu(hwnd: isize, folder: &Path, screen_pos: (i32, i32)) -> MenuAction {
    imp::background_menu(hwnd, folder, screen_pos)
}

/// Moves to the recycle bin (IFileOperation, FOF_ALLOWUNDO). Elsewhere the
/// freedesktop trash (`~/.local/share/Trash`) or `~/.Trash` on macOS.
/// `Err(Interrupted)` when the user cancelled.
pub fn recycle(paths: &[PathBuf]) -> io::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    imp::recycle(paths)
}

/// Deletes permanently (Windows asks for confirmation as Explorer's
/// Shift+Del does). `Err(Interrupted)` when the user cancelled.
pub fn delete(paths: &[PathBuf]) -> io::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    imp::delete(paths)
}

/// Copies `paths` into the folder `dest` (system progress dialog on Windows).
/// A copy into the folder an item already sits in gets a free name
/// ("名称 - 副本.txt", "名称 - 副本 (2).txt"). `Err(Interrupted)` when the
/// user cancelled.
pub fn copy(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    imp::copy(paths, dest)
}

/// Moves `paths` into the folder `dest`. Items already in `dest` are left
/// alone. `Err(Interrupted)` when the user cancelled.
pub fn move_(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    let paths: Vec<PathBuf> = paths
        .iter()
        .filter(|p| !parent_is(p, dest))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Ok(());
    }
    imp::move_(&paths, dest)
}

/// Renames `path` to `new_name` in the same folder; returns the new path.
/// Never overwrites: an existing other item gives `AlreadyExists`. A change
/// of letter case only ("a.txt" -> "A.txt") is allowed.
pub fn rename(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    check_name(new_name)?;
    let to = path.with_file_name(new_name);
    if to == path {
        return Ok(to);
    }
    let case_only = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().to_lowercase() == new_name.to_lowercase());
    if !case_only && std::fs::symlink_metadata(&to).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", to.display()),
        ));
    }
    std::fs::rename(path, &to)?;
    Ok(to)
}

/// Programs recommended for opening `path` (SHAssocEnumHandlers,
/// ASSOC_FILTER_RECOMMENDED), the current default first, de-duplicated.
/// Empty on failure and off Windows. Reads the registry and program icons:
/// call from a background thread.
pub fn open_with_handlers(path: &Path) -> Vec<Handler> {
    imp::open_with_handlers(path)
}

/// Opens `path` with the handler whose [`Handler::id`] is `handler_id`
/// (IAssocHandler::Invoke on the file's data object). `Err` with the
/// system's reason; `NotFound` when no such handler exists for the type.
/// Background thread.
pub fn open_with(path: &Path, handler_id: &str) -> io::Result<()> {
    imp::open_with(path, handler_id)
}

/// Explorer's "选择其他应用" (Choose another app) dialog for `path`
/// (SHOpenWithDialog: the chosen program opens the file, and "always use"
/// can make it the default). Modal: call on the UI thread outside any
/// `cx.update` borrow, like the context menus. `Interrupted` when the user
/// cancels; `Unsupported` off Windows.
pub fn open_with_dialog(hwnd: isize, path: &Path) -> io::Result<()> {
    imp::open_with_dialog(hwnd, path)
}

/// The system Properties sheet for one item (SHObjectProperties) or several
/// (SHMultiFileProperties, items from one folder). Non-modal: returns once
/// the sheet is up. UI thread, outside any borrow. `Unsupported` off Windows.
pub fn properties(hwnd: isize, paths: &[PathBuf]) -> io::Result<()> {
    imp::properties(hwnd, paths)
}

/// File types under Explorer's "新建" menu (`HKCR\.ext[\ProgID]\ShellNew`
/// with NullFile, FileName or Data; Command / Handler entries are left
/// out), sorted by name; `.txt` is always there. Folders are not included.
/// Cached for a short while; first call reads the registry, so call from a
/// background thread. Off Windows just `.txt`.
pub fn new_items() -> Vec<NewItem> {
    imp::new_items()
}

/// Creates a new file of type `ext` (".docx") in `dir` from its ShellNew
/// entry (NullFile: empty; Data: those bytes; FileName: copy of the
/// template) named like Explorer's ("新建 文本文档.txt", then
/// "新建 文本文档 (2).txt"), never overwriting. Returns the new path.
pub fn create_new(dir: &Path, ext: &str) -> io::Result<PathBuf> {
    imp::create_new(dir, ext)
}

/// Opens a terminal in `dir`: Windows Terminal (`wt -d`) when installed,
/// else PowerShell, else cmd. Elsewhere `$TERMINAL`, then
/// x-terminal-emulator and common terminals. Returns once started.
pub fn open_terminal(dir: &Path) -> io::Result<()> {
    imp::open_terminal(dir)
}

/// Zips `paths` (files and folders from one folder) into `dest_zip`, which
/// must not exist yet. Built in (deflate, UTF-8 names, no ZIP64: over 4 GB
/// is refused); symlinks and junctions inside folders are skipped. Blocks
/// until done: background thread. A failed archive is removed.
pub fn compress_zip(paths: &[PathBuf], dest_zip: &Path) -> io::Result<()> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nothing to compress",
        ));
    }
    if std::fs::symlink_metadata(dest_zip).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", dest_zip.display()),
        ));
    }
    let parent = paths
        .first()
        .and_then(|p| p.parent())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent folder"))?;
    if !paths.iter().all(|p| parent_is(p, parent)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "items to compress must come from one folder",
        ));
    }
    let items: Vec<(String, &Path)> = paths
        .iter()
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p.as_path())))
        .collect();
    let dest = if dest_zip.is_absolute() {
        dest_zip.to_path_buf()
    } else {
        std::env::current_dir()?.join(dest_zip)
    };
    let result = zip::write(&dest, &items);
    if result.is_err() {
        let _ = std::fs::remove_file(&dest); // no half-written archive (create_new: never someone else's)
    }
    result
}

/// A preview of `path` whose longer side is `size_px` (aspect kept, never
/// cropped). Windows: the shell's thumbnail (IShellItemImageFactory, the same
/// previews Explorer shows, for images, videos, PDFs, Office files ...), else
/// the type's icon at that size, so it is `Some` for almost any existing
/// file. Elsewhere: png / jpeg / gif / webp / bmp decoded here, `None` for
/// other files. Blocks: call from a background thread. Gives up after about
/// 8 seconds (an offline share, a slow codec) and returns `None`. Not cached:
/// cache in the app by path + modified time + size. Never shows UI.
pub fn thumbnail(path: &Path, size_px: u32) -> Option<RgbaImage> {
    imp::thumbnail(path, size_px)
}

/// Drives / volume roots. `/` plus removable mounts on Linux.
pub fn drives() -> Vec<Drive> {
    imp::drives()
}

/// Home, Desktop, Downloads, Documents, Pictures, those that exist.
pub fn known_folders() -> Vec<KnownFolder> {
    imp::known_folders()
}

/// Puts files on the system clipboard (CF_HDROP + Preferred DropEffect), so
/// Explorer can paste them. In-process only on other platforms.
pub fn clipboard_set_files(paths: &[PathBuf], cut: bool) -> io::Result<()> {
    imp::clipboard_set_files(paths, cut)
}

/// Files on the system clipboard (copied here or in Explorer) and whether
/// they were cut. `None` when the clipboard holds no files.
pub fn clipboard_get_files() -> Option<(Vec<PathBuf>, bool)> {
    imp::clipboard_get_files()
}

/// True when both paths are on the same volume (drag = move, else copy).
pub fn same_volume(a: &Path, b: &Path) -> bool {
    imp::same_volume(a, b)
}

/// The native window handle of a GPUI window (HWND on Windows), for
/// [`show_context_menu`]. GPUI's `Window` implements `HasWindowHandle`.
/// 0 when there is none (other platforms).
pub fn hwnd_of(window: &impl raw_window_handle::HasWindowHandle) -> isize {
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(raw_window_handle::RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

// ---------- shared helpers ----------

/// A name that is not taken: "a.txt", then "a - 副本.txt", "a - 副本 (2).txt", ...
/// Folders (`is_dir`) keep dots in their names.
pub fn free_copy_name(name: &str, is_dir: bool, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !is_dir && !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let first = format!("{stem} - 副本{ext}");
    if !taken(&first) {
        return first;
    }
    (2..100_000)
        .map(|n| format!("{stem} - 副本 ({n}){ext}"))
        .find(|n| !taken(n))
        .unwrap_or(first)
}

/// A file name the app may create: not empty, no separators or characters
/// Windows refuses, not `.` / `..`, no trailing dot or space.
pub fn check_name(name: &str) -> io::Result<()> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with([' ', '.'])
        || name.chars().any(|c| {
            matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 32
        });
    if bad {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid file name: {name:?}"),
        ));
    }
    Ok(())
}

/// Scales `img` down so its longer side is at most `size` (never up).
#[cfg_attr(not(windows), allow(dead_code))]
fn fit_within(img: RgbaImage, size: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    if w <= size && h <= size {
        return img;
    }
    let (nw, nh) = if w >= h {
        (
            size,
            ((u64::from(h) * u64::from(size) + u64::from(w) / 2) / u64::from(w)).max(1) as u32,
        )
    } else {
        (
            ((u64::from(w) * u64::from(size) + u64::from(h) / 2) / u64::from(h)).max(1) as u32,
            size,
        )
    };
    image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
}

/// Creates `dir/<base><ext>`, or "<base> (2)<ext>", "(3)", ... when taken,
/// atomically (create_new), writes `contents` and returns the path.
fn create_numbered(dir: &Path, base: &str, ext: &str, contents: &[u8]) -> io::Result<PathBuf> {
    use std::io::Write;
    check_name(&format!("{base}{ext}"))?;
    for n in 1..10_000u32 {
        let name = if n == 1 {
            format!("{base}{ext}")
        } else {
            format!("{base} ({n}){ext}")
        };
        let path = dir.join(&name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(contents) {
                    drop(f);
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free name"))
}

/// ".docx" from ".docx", "docx" or ".DOCX"; rejects empty / path-like input.
fn norm_ext(ext: &str) -> io::Result<String> {
    let e = ext.trim().trim_start_matches('.').to_lowercase();
    if e.is_empty() || check_name(&e).is_err() || e.contains('.') && e.ends_with('.') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid extension: {ext:?}"),
        ));
    }
    Ok(format!(".{e}"))
}

/// "新建 {type}" for a type name.
fn new_base_name(type_name: &str) -> String {
    format!("新建 {type_name}")
}

/// Folder / File / Missing for a target path; Other when it is empty.
fn classify_target(target: &str) -> LinkTarget {
    if target.is_empty() {
        return LinkTarget::Other;
    }
    let path = PathBuf::from(target);
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => LinkTarget::Folder(path),
        Ok(_) => LinkTarget::File(path),
        Err(_) => LinkTarget::Missing(path),
    }
}

fn parent_is(p: &Path, dir: &Path) -> bool {
    p.parent().is_some_and(|parent| same_path(parent, dir))
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let norm = |p: &Path| {
            p.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .replace('/', "\\")
                .to_lowercase()
        };
        norm(a) == norm(b)
    } else {
        a.components().eq(b.components())
    }
}

/// Is `inner` the same as or inside `outer`?
#[cfg_attr(windows, allow(dead_code))]
fn is_within(inner: &Path, outer: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(inner).starts_with(canon(outer))
}

/// Recursive std copy of `src` to the not-yet-existing `to` (fallback path).
#[cfg_attr(windows, allow(dead_code))]
fn copy_tree(src: &Path, to: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            return std::os::unix::fs::symlink(std::fs::read_link(src)?, to);
        }
        #[cfg(not(unix))]
        {
            std::fs::copy(src, to)?;
            return Ok(());
        }
    }
    if meta.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        std::fs::set_permissions(to, meta.permissions())?;
        Ok(())
    } else {
        std::fs::copy(src, to).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_files_numbered() {
        let d = std::env::temp_dir().join(format!("grids_new_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(
            new_items()
                .iter()
                .any(|i| i.ext == ".txt" && !i.name.is_empty())
        );
        let a = create_new(&d, ".txt").unwrap();
        let b = create_new(&d, "txt").unwrap();
        assert!(a.is_file() && b.is_file() && a != b, "{a:?} {b:?}");
        let (na, nb) = (
            a.file_name().unwrap().to_string_lossy().into_owned(),
            b.file_name().unwrap().to_string_lossy().into_owned(),
        );
        assert!(na.ends_with(".txt") && !na.contains("(2)"), "{na}");
        assert_eq!(nb, na.replace(".txt", " (2).txt"));
        assert!(create_new(&d, "").is_err());
        assert!(create_new(&d, "../x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_readable() {
        // for an outside reader: GRIDS_ZIP_KEEP=dir keeps the archive there
        let d = std::env::temp_dir().join(format!("grids_zip2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("文件夹 1").join("空")).unwrap();
        std::fs::write(d.join("报告.txt"), "你好 Grids\n".repeat(1000)).unwrap();
        std::fs::write(
            d.join("文件夹 1").join("b.bin"),
            (0..=255u8).collect::<Vec<_>>(),
        )
        .unwrap();
        std::fs::write(d.join("empty.txt"), "").unwrap();
        let out = d.join("out.zip");
        compress_zip(
            &[d.join("报告.txt"), d.join("文件夹 1"), d.join("empty.txt")],
            &out,
        )
        .unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        assert!(bytes.len() < 2000, "not compressed: {}", bytes.len());
        if let Some(keep) = std::env::var_os("GRIDS_ZIP_KEEP") {
            std::fs::copy(&out, std::path::Path::new(&keep).join("grids_test.zip")).unwrap();
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_arguments_checked() {
        let d = std::env::temp_dir().join(format!("grids_zip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::write(d.join("sub").join("b.txt"), "b").unwrap();
        assert!(compress_zip(&[], &d.join("x.zip")).is_err());
        // items from two folders
        assert!(
            compress_zip(
                &[d.join("a.txt"), d.join("sub").join("b.txt")],
                &d.join("x.zip")
            )
            .is_err()
        );
        std::fs::write(d.join("taken.zip"), "z").unwrap();
        assert_eq!(
            compress_zip(&[d.join("a.txt")], &d.join("taken.zip"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        // the real thing; the content is checked by zip_readable below
        match compress_zip(&[d.join("a.txt"), d.join("sub")], &d.join("out.zip")) {
            Ok(()) => {
                let bytes = std::fs::read(d.join("out.zip")).unwrap();
                assert!(bytes.starts_with(b"PK"), "not a zip");
            }
            Err(e) => panic!("{e}"),
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_names() {
        assert_eq!(free_copy_name("a.txt", false, |_| false), "a.txt");
        assert_eq!(
            free_copy_name("a.txt", false, |n| n == "a.txt"),
            "a - 副本.txt"
        );
        let taken = ["a.txt", "a - 副本.txt", "a - 副本 (2).txt"];
        assert_eq!(
            free_copy_name("a.txt", false, |n| taken.contains(&n)),
            "a - 副本 (3).txt"
        );
        assert_eq!(free_copy_name("v1.2", true, |n| n == "v1.2"), "v1.2 - 副本");
        assert_eq!(
            free_copy_name(".bashrc", false, |n| n == ".bashrc"),
            ".bashrc - 副本"
        );
        assert_eq!(
            free_copy_name("noext", false, |n| n == "noext"),
            "noext - 副本"
        );
    }

    #[test]
    fn names() {
        assert!(check_name("报告.docx").is_ok());
        for bad in ["", ".", "..", "a/b", "a\\b", "a:b", "x?", "end.", "end "] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grids_shell_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rename_never_overwrites() {
        let d = scratch("rename");
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::write(d.join("b.txt"), "b").unwrap();
        assert_eq!(
            rename(&d.join("a.txt"), "b.txt").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let to = rename(&d.join("a.txt"), "c.txt").unwrap();
        assert_eq!(std::fs::read_to_string(to).unwrap(), "a");
        assert!(rename(&d.join("c.txt"), "x/y").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_tree_recurses() {
        let d = scratch("tree");
        std::fs::create_dir_all(d.join("src/sub")).unwrap();
        std::fs::write(d.join("src/sub/f.txt"), "x").unwrap();
        copy_tree(&d.join("src"), &d.join("dst")).unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("dst/sub/f.txt")).unwrap(),
            "x"
        );
        assert!(is_within(&d.join("src/sub"), &d.join("src")));
        assert!(!is_within(&d.join("dst"), &d.join("src")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn move_skips_items_already_there() {
        let d = scratch("move_same");
        std::fs::write(d.join("a.txt"), "a").unwrap();
        move_(&[d.join("a.txt")], &d).unwrap();
        assert!(d.join("a.txt").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn known_folders_have_home() {
        if std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).is_some() {
            assert!(
                known_folders()
                    .iter()
                    .any(|f| f.kind == KnownFolderKind::Home)
            );
        }
    }

    #[test]
    fn drives_not_empty() {
        assert!(!drives().is_empty());
    }
}
