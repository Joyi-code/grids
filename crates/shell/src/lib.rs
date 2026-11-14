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

/// Opens a URL with the system's default handler (normally the browser).
pub fn open_url(url: &str) -> io::Result<()> {
    imp::open_url(url)
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

/// One item a file operation actually processed, for the operation journal.
///
/// Items the user skipped in a conflict dialog, and items that failed, are
/// not reported; children inside a copied / moved folder are not reported
/// separately (the folder is one item).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpItem {
    /// The path before the operation (for `create_new_tracked`: the new file).
    pub src: PathBuf,
    /// Where the item ended up: the real copy / move / rename target,
    /// including an automatic "名称 - 副本 (2)" name; for a recycle, the
    /// item inside the recycle bin (Windows: `X:\$Recycle.Bin\<SID>\$R…`)
    /// when the shell reports it; `None` for a permanent delete.
    pub dst: Option<PathBuf>,
    /// The item is a folder.
    pub is_dir: bool,
}

/// Moves to the recycle bin (IFileOperation, FOF_ALLOWUNDO). Elsewhere the
/// freedesktop trash (`~/.local/share/Trash`) or `~/.Trash` on macOS.
/// `Err(Interrupted)` when the user cancelled.
pub fn recycle(paths: &[PathBuf]) -> io::Result<()> {
    recycle_tracked(paths, false).map(drop)
}

/// Deletes permanently (Windows asks for confirmation as Explorer's
/// Shift+Del does). `Err(Interrupted)` when the user cancelled.
pub fn delete(paths: &[PathBuf]) -> io::Result<()> {
    delete_tracked(paths, false).map(drop)
}

/// [`recycle`] after Grids' own confirmation: FOF_NOCONFIRMATION and no nuke
/// warning (word the confirmation with [`recycle_would_nuke`]). Progress and
/// conflict UI stay native.
pub fn recycle_confirmed(paths: &[PathBuf]) -> io::Result<()> {
    recycle_tracked(paths, true).map(drop)
}

/// [`delete`] after Grids' own confirmation (FOF_NOCONFIRMATION).
pub fn delete_confirmed(paths: &[PathBuf]) -> io::Result<()> {
    delete_tracked(paths, true).map(drop)
}

/// Copies `paths` into the folder `dest` (system progress dialog on Windows).
/// A copy into the folder an item already sits in gets a free name
/// ("名称 - 副本.txt", "名称 - 副本 (2).txt"). `Err(Interrupted)` when the
/// user cancelled.
pub fn copy(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    copy_tracked(paths, dest).map(drop)
}

/// Moves `paths` into the folder `dest`. Items already in `dest` are left
/// alone. `Err(Interrupted)` when the user cancelled.
pub fn move_(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    move_tracked(paths, dest).map(drop)
}

/// [`recycle`], returning what was recycled. When the user cancels part-way
/// the items already done are returned as `Ok`; `Err(Interrupted)` only when
/// nothing was done.
///
/// `confirmed`: Grids already asked (its own dialog, worded by
/// [`recycle_would_nuke`]), so the shell adds FOF_NOCONFIRMATION and no
/// "delete permanently?" nuke warning. Progress and conflict UI remain.
pub fn recycle_tracked(paths: &[PathBuf], confirmed: bool) -> io::Result<Vec<OpItem>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    imp::recycle(paths, confirmed)
}

/// [`delete`], returning what was deleted (`dst` is `None`). Partial
/// cancellation and `confirmed` as in [`recycle_tracked`].
pub fn delete_tracked(paths: &[PathBuf], confirmed: bool) -> io::Result<Vec<OpItem>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    imp::delete(paths, confirmed)
}

/// Best effort, cheap enough for the UI thread on local drives (network
/// drives answer `true` without being queried): whether recycling `paths`
/// would delete them permanently because their drive has no Recycle Bin.
/// Use it to word Grids' confirmation; files too big for the bin are not
/// detected. Always `false` off Windows (the trash fails instead).
pub fn recycle_would_nuke(paths: &[PathBuf]) -> bool {
    !paths.is_empty() && imp::recycle_would_nuke(paths)
}

/// [`copy`], returning each copy's real path (automatic "副本" names
/// included; items skipped in the conflict dialog are absent). Partial
/// cancellation as in [`recycle_tracked`].
pub fn copy_tracked(paths: &[PathBuf], dest: &Path) -> io::Result<Vec<OpItem>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    imp::copy(paths, dest)
}

/// [`move_`], returning where each item went. Partial cancellation as in
/// [`recycle_tracked`].
pub fn move_tracked(paths: &[PathBuf], dest: &Path) -> io::Result<Vec<OpItem>> {
    let paths: Vec<PathBuf> = paths
        .iter()
        .filter(|p| !parent_is(p, dest))
        .cloned()
        .collect();
    if paths.is_empty() {
        return Ok(Vec::new());
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

/// [`rename`] as one [`OpItem`]; empty when the name did not change.
pub fn rename_tracked(path: &Path, new_name: &str) -> io::Result<Vec<OpItem>> {
    let is_dir = path.is_dir();
    let to = rename(path, new_name)?;
    if to == path {
        return Ok(Vec::new());
    }
    Ok(vec![OpItem {
        src: path.to_path_buf(),
        dst: Some(to),
        is_dir,
    }])
}

/// [`create_new`] as one [`OpItem`] (`src` and `dst` both the new file).
pub fn create_new_tracked(dir: &Path, ext: &str) -> io::Result<Vec<OpItem>> {
    let path = create_new(dir, ext)?;
    Ok(vec![OpItem {
        src: path.clone(),
        dst: Some(path),
        is_dir: false,
    }])
}

/// Puts back an item recycled from `original`: the recycle-bin entry with
/// that original path whose deletion time is nearest `deleted_near` (within
/// 10 seconds). Windows walks the Recycle Bin (FOLDERID_RecycleBinFolder)
/// and runs its "undelete" verb; elsewhere the freedesktop trash.
/// Returns the restored path (= `original`). Errors: `NotFound` when no
/// entry matches, `AlreadyExists` when something now sits at `original`.
pub fn restore_from_recycle_bin(
    original: &Path,
    deleted_near: std::time::SystemTime,
) -> io::Result<PathBuf> {
    if std::fs::symlink_metadata(original).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", original.display()),
        ));
    }
    imp::restore_from_recycle_bin(original, deleted_near)
}

/// Seconds between two times, either order.
fn secs_apart(a: std::time::SystemTime, b: std::time::SystemTime) -> f64 {
    match a.duration_since(b) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => e.duration().as_secs_f64(),
    }
}

/// Picks the candidate (path, deletion time) whose original matches and
/// whose time is nearest `near`, within 10 s.
fn nearest_deleted<T>(
    candidates: impl IntoIterator<Item = (T, PathBuf, std::time::SystemTime)>,
    original: &Path,
    near: std::time::SystemTime,
) -> Option<T> {
    candidates
        .into_iter()
        .filter(|(_, orig, _)| same_path(orig, original))
        .map(|(t, _, when)| (t, secs_apart(when, near)))
        .filter(|(_, d)| *d <= 10.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| t)
}

/// What kind of program holds a file (Restart Manager's RM_APP_TYPE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    /// A program with a main window.
    Window,
    /// A program with windows, none of them its main one.
    OtherWindow,
    /// A Windows service.
    Service,
    /// Windows Explorer.
    Explorer,
    /// A console program.
    Console,
    /// A process Windows needs (closing it would restart / crash Windows).
    Critical,
    /// Anything else (or off Windows).
    Unknown,
}

/// A process that has a file open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locker {
    pub pid: u32,
    /// Friendly name ("Microsoft Word"), else the program's file name.
    pub name: String,
    pub kind: AppKind,
    /// Its main top-level window (HWND), when it has a visible one.
    pub hwnd: Option<isize>,
    /// Full path of the program, when Grids may query it (for its icon).
    pub exe: Option<PathBuf>,
    /// Service short name for [`AppKind::Service`].
    pub service: Option<String>,
    /// Grids could open it for termination. `false`: it runs elevated or as
    /// another user / the system; the UI should say administrator rights
    /// are needed and offer no end-process button.
    pub can_terminate: bool,
}

/// Result of [`lockers`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lockers {
    pub list: Vec<Locker>,
    /// A folder held more than [`LOCKERS_MAX_FILES`] files; the rest were
    /// not checked.
    pub truncated: bool,
}

/// Files of a folder checked by [`lockers`] at most.
pub const LOCKERS_MAX_FILES: usize = 2000;

/// Which processes have any of `paths` open (Restart Manager: RmStartSession,
/// RmRegisterResources, RmGetList). A folder is checked through the files
/// inside it, recursively, up to [`LOCKERS_MAX_FILES`]. Grids itself is
/// listed too when it holds one (`pid == std::process::id()`). A single file takes a few milliseconds; folders can take
/// longer: call from a background thread. Linux reads `/proc/*/fd`; other
/// platforms return an empty list.
pub fn lockers(paths: &[PathBuf]) -> io::Result<Lockers> {
    let (files, truncated) = lock_candidates(paths, LOCKERS_MAX_FILES);
    if files.is_empty() {
        return Ok(Lockers {
            list: Vec::new(),
            truncated,
        });
    }
    let mut list = imp::lockers(&files)?;
    list.sort_by_key(|l| l.pid);
    list.dedup_by_key(|l| l.pid);
    Ok(Lockers { list, truncated })
}

/// Files to check: `paths` themselves, folders expanded to the files under
/// them (depth first), at most `max`; `true` when some were left out.
fn lock_candidates(paths: &[PathBuf], max: usize) -> (Vec<PathBuf>, bool) {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = paths.iter().rev().cloned().collect();
    while let Some(p) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&p) {
                let mut kids: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                kids.sort();
                stack.extend(kids.into_iter().rev());
            }
        } else {
            if out.len() >= max {
                return (out, true);
            }
            out.push(p);
        }
    }
    (out, false)
}

/// Brings `pid`'s main window to the front, restoring it when minimised.
/// `NotFound` when it has no visible top-level window. UI thread is fine.
pub fn activate_process_window(pid: u32) -> io::Result<()> {
    imp::activate_process_window(pid)
}

/// Asks `pid` to close: WM_CLOSE to its visible top-level windows, so it can
/// offer to save. Returns at once; poll [`lockers`] to see whether it let
/// go. `NotFound` when it has no window (console programs, services).
pub fn close_process(pid: u32) -> io::Result<()> {
    imp::close_process(pid)
}

/// Ends `pid` at once (TerminateProcess); unsaved work is lost.
/// `PermissionDenied` when Grids lacks the rights (elevated / system
/// process); `InvalidInput` for Grids itself.
pub fn terminate_process(pid: u32) -> io::Result<()> {
    if pid == std::process::id() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to end Grids itself",
        ));
    }
    imp::terminate_process(pid)
}

/// The image on the clipboard: the registered "PNG" format first (browsers,
/// Office, Snipping Tool put it there with alpha), else CF_DIBV5, else
/// CF_DIB (BI_RGB / BI_BITFIELDS, 1-32 bpp, bottom-up or top-down).
/// `None` when there is none or it can't be read; always `None` off
/// Windows. Opens the clipboard briefly: any thread.
pub fn clipboard_image() -> Option<RgbaImage> {
    imp::clipboard_image()
}

/// Text on the clipboard (CF_UNICODETEXT), `None` when there is none or it
/// is empty. Always `None` off Windows.
pub fn clipboard_text() -> Option<String> {
    imp::clipboard_text()
}

/// Largest clipboard image accepted (pixels), so a bogus header can't make
/// us allocate gigabytes.
#[cfg_attr(not(windows), allow(dead_code))]
const MAX_CLIP_PIXELS: u64 = 100_000_000;

/// Decodes a packed DIB (BITMAPINFOHEADER / V4 / V5 + optional masks +
/// palette + pixels), as CF_DIB and CF_DIBV5 hold it.
#[cfg_attr(not(windows), allow(dead_code))]
fn decode_dib(b: &[u8]) -> Option<RgbaImage> {
    let u32_at =
        |i: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    let u16_at =
        |i: usize| -> Option<u16> { Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?)) };
    let header = u32_at(0)? as usize;
    if header < 40 {
        return None;
    }
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bpp = u16_at(14)?;
    let compression = u32_at(16)?;
    let clr_used = u32_at(32)? as usize;
    if width <= 0 || height == 0 || height == i32::MIN {
        return None;
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if u64::from(w) * u64::from(h) > MAX_CLIP_PIXELS {
        return None;
    }
    let top_down = height < 0;
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    const BI_ALPHABITFIELDS: u32 = 6;
    let mut offset = header;
    // channel masks (r, g, b, a)
    let masks = match compression {
        BI_RGB => match bpp {
            16 => Some((0x7c00, 0x03e0, 0x001f, 0)),
            32 => Some((0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000)),
            _ => None,
        },
        BI_BITFIELDS | BI_ALPHABITFIELDS => {
            if bpp != 16 && bpp != 32 {
                return None;
            }
            let has_alpha_mask = compression == BI_ALPHABITFIELDS || header >= 56;
            let (base, after) = if header == 40 {
                (
                    40,
                    if compression == BI_ALPHABITFIELDS {
                        16
                    } else {
                        12
                    },
                )
            } else {
                (40, 0)
            };
            offset += after;
            let a = if has_alpha_mask {
                u32_at(base + 12).unwrap_or(0)
            } else {
                0
            };
            Some((u32_at(base)?, u32_at(base + 4)?, u32_at(base + 8)?, a))
        }
        _ => return None, // RLE, JPEG, PNG inside a DIB: not seen on the clipboard in practice
    };
    let palette: Vec<[u8; 4]> = if bpp <= 8 {
        let n = if clr_used == 0 {
            1usize << bpp
        } else {
            clr_used.min(256)
        };
        let pal = b.get(offset..offset + n * 4)?;
        offset += n * 4;
        pal.as_chunks::<4>()
            .0
            .iter()
            .map(|c| [c[2], c[1], c[0], 255])
            .collect()
    } else {
        // BI_RGB with 16-32 bpp may still carry a (useless) colour table
        offset += clr_used.min(256) * 4;
        Vec::new()
    };
    let stride = (w as usize * bpp as usize).div_ceil(32) * 4;
    let need = stride * h as usize;
    let mut pixels = b.get(offset..)?;
    if pixels.len() < need {
        return None;
    }
    // some writers repeat the three masks after a V4/V5 header
    if header > 40 && compression == BI_BITFIELDS && pixels.len() - need == 12 {
        pixels = pixels.get(12..)?;
    }
    let channel = |v: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        let x = (v & mask) >> shift;
        ((u64::from(x) * 255 + u64::from(max) / 2) / u64::from(max)) as u8
    };
    let mut img = RgbaImage::new(w, h);
    let mut any_alpha = false;
    for y in 0..h {
        let src_row = if top_down { y } else { h - 1 - y } as usize;
        let row = pixels.get(src_row * stride..src_row * stride + stride)?;
        for x in 0..w {
            let xi = x as usize;
            let px = match bpp {
                1 | 4 | 8 => {
                    let bit = xi * bpp as usize;
                    let byte = *row.get(bit / 8)?;
                    let idx = (byte >> (8 - bpp as usize - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                    palette.get(idx as usize).copied().unwrap_or([0, 0, 0, 255])
                }
                24 => {
                    let p = row.get(xi * 3..xi * 3 + 3)?;
                    [*p.get(2)?, *p.get(1)?, *p.first()?, 255]
                }
                16 | 32 => {
                    let (r, g, bl, a) = masks?;
                    let v = if bpp == 16 {
                        u32::from(u16::from_le_bytes(
                            row.get(xi * 2..xi * 2 + 2)?.try_into().ok()?,
                        ))
                    } else {
                        u32::from_le_bytes(row.get(xi * 4..xi * 4 + 4)?.try_into().ok()?)
                    };
                    let alpha = if a == 0 { 255 } else { channel(v, a) };
                    any_alpha |= a != 0 && alpha != 0;
                    [channel(v, r), channel(v, g), channel(v, bl), alpha]
                }
                _ => return None,
            };
            img.put_pixel(x, y, image::Rgba(px));
        }
    }
    // 32 bpp with an all-zero alpha channel means "no alpha" (most writers)
    if bpp == 32 && !any_alpha {
        for p in img.pixels_mut() {
            p.0[3] = 255;
        }
    }
    Some(img)
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

    /// A BITMAPINFOHEADER-family header of `size` bytes.
    fn dib_header(size: u32, w: i32, h: i32, bpp: u16, comp: u32, clr_used: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(size.to_le_bytes());
        v.extend(w.to_le_bytes());
        v.extend(h.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(bpp.to_le_bytes());
        v.extend(comp.to_le_bytes());
        v.extend(0u32.to_le_bytes()); // size image
        v.extend([0u8; 8]); // ppm
        v.extend(clr_used.to_le_bytes());
        v.extend(0u32.to_le_bytes()); // important
        v.resize(size as usize, 0);
        v
    }

    #[test]
    fn dib_24bpp_bottom_up_with_padding() {
        let mut b = dib_header(40, 2, 2, 24, 0, 0);
        // bottom row first: blue, green (+2 pad); then top row: red, white (+2 pad)
        b.extend([255, 0, 0, 0, 255, 0, 0, 0]);
        b.extend([0, 0, 255, 255, 255, 255, 0, 0]);
        let img = decode_dib(&b).unwrap();
        assert_eq!(img.dimensions(), (2, 2));
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(0, 1).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(1, 1).0, [0, 255, 0, 255]);
        b.truncate(b.len() - 1);
        assert!(decode_dib(&b).is_none());
    }

    #[test]
    fn dib_32bpp_alpha_rules() {
        // BI_RGB, top-down, alpha all zero: opaque
        let mut b = dib_header(40, 1, -2, 32, 0, 0);
        b.extend([10, 20, 30, 0, 40, 50, 60, 0]);
        let img = decode_dib(&b).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [30, 20, 10, 255]);
        assert_eq!(img.get_pixel(0, 1).0, [60, 50, 40, 255]);
        // V5 + BI_BITFIELDS with an alpha mask: alpha kept
        let mut b = dib_header(124, 1, 1, 32, 3, 0);
        b[40..44].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        b[44..48].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
        b[48..52].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        b[52..56].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        b.extend([1, 2, 3, 128]);
        assert_eq!(decode_dib(&b).unwrap().get_pixel(0, 0).0, [3, 2, 1, 128]);
        // the same with the masks repeated after the header
        let mut r = b[..124].to_vec();
        r.extend(0x00ff_0000u32.to_le_bytes());
        r.extend(0x0000_ff00u32.to_le_bytes());
        r.extend(0x0000_00ffu32.to_le_bytes());
        r.extend([1, 2, 3, 128]);
        assert_eq!(decode_dib(&r).unwrap().get_pixel(0, 0).0, [3, 2, 1, 128]);
        // RGBA order masks (BI_BITFIELDS in a plain 40-byte header: masks follow it)
        let mut b = dib_header(40, 1, 1, 32, 3, 0);
        b.extend(0x0000_00ffu32.to_le_bytes());
        b.extend(0x0000_ff00u32.to_le_bytes());
        b.extend(0x00ff_0000u32.to_le_bytes());
        b.extend([1, 2, 3, 99]);
        assert_eq!(decode_dib(&b).unwrap().get_pixel(0, 0).0, [1, 2, 3, 255]);
    }

    #[test]
    fn dib_16bpp_and_palettes() {
        // 565 bitfields: pure red, pure green
        let mut b = dib_header(40, 2, 1, 16, 3, 0);
        b.extend(0xf800u32.to_le_bytes());
        b.extend(0x07e0u32.to_le_bytes());
        b.extend(0x001fu32.to_le_bytes());
        b.extend(0xf800u16.to_le_bytes());
        b.extend(0x07e0u16.to_le_bytes());
        let img = decode_dib(&b).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0, 255]);
        // 555 BI_RGB default: blue
        let mut b = dib_header(40, 1, 1, 16, 0, 0);
        b.extend([0x1f, 0x00, 0, 0]);
        assert_eq!(decode_dib(&b).unwrap().get_pixel(0, 0).0, [0, 0, 255, 255]);
        // 8 bpp, 2-entry palette
        let mut b = dib_header(40, 3, 1, 8, 0, 2);
        b.extend([0, 0, 0, 0, 0, 0, 255, 0]); // black, red (BGRx)
        b.extend([1, 0, 1, 0]);
        let img = decode_dib(&b).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 0, 0, 255]);
        // 1 bpp, default 2-entry palette: 0b1010_0000
        let mut b = dib_header(40, 3, 1, 1, 0, 0);
        b.extend([0, 0, 0, 0, 255, 255, 255, 0]);
        b.extend([0b1010_0000, 0, 0, 0]);
        let img = decode_dib(&b).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 0, 0, 255]);
        assert_eq!(img.get_pixel(2, 0).0, [255, 255, 255, 255]);
        // nonsense
        assert!(decode_dib(&dib_header(40, 0, 1, 24, 0, 0)).is_none());
        assert!(decode_dib(&dib_header(40, 100_000, 100_000, 24, 0, 0)).is_none());
        assert!(decode_dib(&dib_header(40, 1, 1, 24, 1, 0)).is_none()); // RLE8
        assert!(decode_dib(&[1, 2, 3]).is_none());
    }

    #[test]
    fn lock_candidates_expand_and_cap() {
        let d = scratch("lockcand");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        for n in ["a", "b", "c"] {
            std::fs::write(d.join(n), n).unwrap();
        }
        std::fs::write(d.join("sub").join("z"), "z").unwrap();
        let (all, cut) = lock_candidates(std::slice::from_ref(&d), 10);
        assert_eq!(all.len(), 4);
        assert!(!cut);
        let (some, cut) = lock_candidates(std::slice::from_ref(&d), 2);
        assert_eq!(some, vec![d.join("a"), d.join("b")]);
        assert!(cut);
        let (none, cut) = lock_candidates(&[d.join("missing")], 2);
        assert!(none.is_empty() && !cut);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn lockers_find_this_process() {
        let d = scratch("lockers");
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("held.txt");
        std::fs::write(&f, "x").unwrap();
        std::fs::write(d.join("free.txt"), "y").unwrap();
        let me = std::process::id();
        assert!(
            !lockers(&[d.join("free.txt")])
                .unwrap()
                .list
                .iter()
                .any(|l| l.pid == me)
        );
        let held = std::fs::File::open(&f).unwrap();
        let by_file = lockers(std::slice::from_ref(&f)).unwrap();
        println!("lockers(file) = {by_file:?}");
        let mine = by_file
            .list
            .iter()
            .find(|l| l.pid == me)
            .expect("this process holds the file");
        assert!(!mine.name.is_empty());
        assert!(mine.can_terminate);
        let by_dir = lockers(std::slice::from_ref(&d)).unwrap();
        assert!(by_dir.list.iter().any(|l| l.pid == me) && !by_dir.truncated);
        drop(held);
        assert!(
            !lockers(std::slice::from_ref(&f))
                .unwrap()
                .list
                .iter()
                .any(|l| l.pid == me)
        );
        assert!(matches!(
            terminate_process(me).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nearest_deleted_picks_closest_within_10s() {
        use std::time::{Duration, UNIX_EPOCH};
        let t = |s: u64| UNIX_EPOCH + Duration::from_secs(s);
        let orig = if cfg!(windows) {
            PathBuf::from(r"C:\A\b.txt")
        } else {
            PathBuf::from("/a/b.txt")
        };
        let other = if cfg!(windows) {
            PathBuf::from(r"C:\A\c.txt")
        } else {
            PathBuf::from("/a/c.txt")
        };
        let cands = vec![
            (1, orig.clone(), t(1000)),
            (2, orig.clone(), t(1007)),
            (3, other.clone(), t(1009)),
            (4, orig.clone(), t(1030)),
        ];
        assert_eq!(nearest_deleted(cands.clone(), &orig, t(1009)), Some(2));
        assert_eq!(nearest_deleted(cands.clone(), &orig, t(995)), Some(1));
        assert_eq!(nearest_deleted(cands.clone(), &orig, t(1019)), None);
        assert_eq!(nearest_deleted(cands, &other, t(1000)), Some(3));
        if cfg!(windows) {
            let c = vec![(9, PathBuf::from(r"c:\a\B.TXT"), t(5))];
            assert_eq!(nearest_deleted(c, &orig, t(5)), Some(9));
        }
    }

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
