//! Windows implementation: Windows Shell through the `windows` crate (0.61,
//! the version GPUI 0.2.2 uses). Ported in spirit from the original Grids
//! (efww/grids, MIT), re-written for a GPUI-drawn app.

use crate::{Drive, KnownFolder, KnownFolderKind, MenuAction, RgbaImage, parent_is};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{
    GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RPC_E_CHANGED_MODE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, GetDriveTypeW,
    GetLogicalDrives, GetVolumeInformationW, GetVolumePathNameW,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SetThreadErrorMode, THREAD_ERROR_MODE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CFSTR_PREFERREDDROPEFFECT, CMF_CANRENAME, CMF_EXTENDEDVERBS, CMF_NORMAL,
    CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DROPFILES, DragQueryFileW,
    FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMMKDIR, FOF_RENAMEONCOLLISION,
    FOF_WANTNUKEWARNING, FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE, FOLDERID_Desktop,
    FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_Profile, FileOperation,
    GCS_VERBW, HDROP, IContextMenu, IContextMenu2, IContextMenu3, IFileOperation, ILFree,
    IShellFolder, IShellItem, IShellItemArray, KF_FLAG_DEFAULT, SHCreateItemFromParsingName,
    SHCreateShellItemArrayFromIDLists, SHFILEINFOW, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES,
    SHGetDesktopFolder, SHGetFileInfoW, SHGetImageList, SHGetKnownFolderPath, SHIL_EXTRALARGE,
    SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL, SHOpenFolderAndSelectItems, SHParseDisplayName,
    ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, GWLP_USERDATA, GetIconInfo, GetWindowLongPtrW, HICON, HMENU, ICONINFO,
    PostMessageW, RegisterClassExW, SW_SHOWNORMAL, SetWindowLongPtrW, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WINDOW_EX_STYLE, WM_DRAWITEM, WM_INITMENUPOPUP,
    WM_MEASUREITEM, WM_MENUCHAR, WM_MENUSELECT, WM_NCCREATE, WM_NULL, WNDCLASSEXW, WS_POPUP,
};
use windows::core::{GUID, HSTRING, Interface, PCSTR, PCWSTR, PSTR, PWSTR, w};

const CMIC_MASK_UNICODE: u32 = 0x4000;
const CMIC_MASK_SHIFT_DOWN: u32 = 0x1000_0000;
const CMIC_MASK_CONTROL_DOWN: u32 = 0x4000_0000;
const FIRST_CMD: u32 = 1;
const LAST_CMD: u32 = 0x7fff;

// ---------- COM ----------

struct Com(bool);

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// STA COM on this thread for as long as the thread lives. A thread already
/// in another apartment (RPC_E_CHANGED_MODE) is used as it is.
fn ensure_com() {
    thread_local! {
        static COM: Com = {
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            Com(hr.is_ok() && hr != RPC_E_CHANGED_MODE)
        };
    }
    COM.with(|_| {});
}

fn wide(p: &Path) -> HSTRING {
    HSTRING::from(p.as_os_str())
}

fn win_err(e: windows::core::Error) -> io::Error {
    io::Error::other(e.to_string())
}

/// An absolute PIDL, freed on drop.
struct Pidl(*mut ITEMIDLIST);

impl Pidl {
    fn parse(p: &Path) -> io::Result<Pidl> {
        let mut pidl = std::ptr::null_mut();
        unsafe { SHParseDisplayName(&wide(p), None, &mut pidl, 0, None) }
            .map_err(|e| io::Error::other(format!("{}: {e}", p.display())))?;
        Ok(Pidl(pidl))
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { ILFree(Some(self.0)) };
        }
    }
}

// ---------- open / reveal ----------

pub fn open_with_system(path: &Path) -> io::Result<()> {
    ensure_com();
    let file = wide(path);
    let dir = path.parent().map(wide).unwrap_or_default();
    let run = |verb: PCWSTR| unsafe { ShellExecuteW(None, verb, &file, PCWSTR::null(), &dir, SW_SHOWNORMAL) }.0 as isize;
    let mut r = run(w!("open"));
    if r == 31 {
        // SE_ERR_NOASSOC: no default program, ask the user which one
        r = run(w!("openas"));
    }
    if r <= 32 {
        Err(io::Error::other(format!(
            "ShellExecuteW failed ({r}) for {}",
            path.display()
        )))
    } else {
        Ok(())
    }
}

pub fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    ensure_com();
    let pidl = Pidl::parse(path)?;
    // no child list: Explorer opens the parent and selects this item
    unsafe { SHOpenFolderAndSelectItems(pidl.0, None, 0) }.map_err(win_err)
}

// ---------- icons ----------

/// Types whose icon is stored in (or depends on) the file itself.
const PER_FILE: &[&str] = &[
    "exe",
    "lnk",
    "ico",
    "cur",
    "ani",
    "url",
    "msi",
    "scr",
    "appref-ms",
    "cpl",
    "msc",
];

static ICONS: OnceLock<Mutex<HashMap<(String, u32), Option<Arc<RgbaImage>>>>> = OnceLock::new();

fn icon_list_for(size: u32) -> (u32, u32) {
    match size {
        0..=16 => (SHIL_SMALL, 16),
        17..=32 => (SHIL_LARGE, 32),
        33..=48 => (SHIL_EXTRALARGE, 48),
        _ => (SHIL_JUMBO, 256),
    }
}

pub fn file_icon(path: &Path, is_dir: bool, size: u32) -> Option<Arc<RgbaImage>> {
    let (list, bucket) = icon_list_for(size);
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_root = path.parent().is_none();
    // generic: looked up by attributes only, no disk access
    let generic = !is_root
        && if is_dir {
            true
        } else {
            !PER_FILE.contains(&ext.as_str())
        };
    let key = match (generic, is_dir) {
        (true, true) => "<dir>".to_string(),
        (true, false) => format!(".{ext}"),
        (false, _) => path.to_string_lossy().to_lowercase(),
    };
    let cache = ICONS.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok()?.get(&(key.clone(), bucket)) {
        return hit.clone();
    }
    ensure_com();
    let img = unsafe { load_icon(path, is_dir, generic, &ext, list) }.map(Arc::new);
    if let Ok(mut c) = cache.lock() {
        c.insert((key, bucket), img.clone());
    }
    img
}

unsafe fn load_icon(
    path: &Path,
    is_dir: bool,
    generic: bool,
    ext: &str,
    list: u32,
) -> Option<RgbaImage> {
    unsafe {
        let mut sfi = SHFILEINFOW::default();
        let (name, attrs, flags) = if generic {
            let attrs = if is_dir {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            };
            let name = if is_dir {
                HSTRING::from("folder")
            } else {
                HSTRING::from(format!("file.{ext}"))
            };
            (name, attrs, SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES)
        } else {
            (wide(path), FILE_FLAGS_AND_ATTRIBUTES(0), SHGFI_SYSICONINDEX)
        };
        if SHGetFileInfoW(
            &name,
            attrs,
            Some(&mut sfi),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        ) == 0
        {
            return None;
        }
        let images: IImageList = SHGetImageList(list as i32).ok()?;
        let hicon = images.GetIcon(sfi.iIcon, ILD_TRANSPARENT.0).ok()?;
        let img = hicon_to_rgba(hicon);
        let _ = DestroyIcon(hicon);
        img
    }
}

unsafe fn bitmap_bgra(
    dc: windows::Win32::Graphics::Gdi::HDC,
    bmp: HBITMAP,
    w: i32,
    h: i32,
) -> Option<Vec<u8>> {
    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let lines = unsafe {
        GetDIBits(
            dc,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as _),
            &mut bi,
            DIB_RGB_COLORS,
        )
    };
    (lines == h).then_some(buf)
}

unsafe fn hicon_to_rgba(hicon: HICON) -> Option<RgbaImage> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(hicon, &mut ii).ok()?;
        let free = |b: HBITMAP| {
            if !b.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
        };
        let out = (|| {
            if ii.hbmColor.is_invalid() {
                return None; // monochrome icon: not worth drawing
            }
            let mut bm = BITMAP::default();
            if GetObjectW(
                HGDIOBJ(ii.hbmColor.0),
                size_of::<BITMAP>() as i32,
                Some(&mut bm as *mut _ as _),
            ) == 0
            {
                return None;
            }
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
                return None;
            }
            let dc = CreateCompatibleDC(None);
            let color = bitmap_bgra(dc, ii.hbmColor, w, h);
            // old icons carry no alpha: take it from the AND mask (0 = opaque)
            let mask = match &color {
                Some(c)
                    if c.as_chunks::<4>().0.iter().all(|p| p[3] == 0)
                        && !ii.hbmMask.is_invalid() =>
                {
                    bitmap_bgra(dc, ii.hbmMask, w, h)
                }
                _ => None,
            };
            let _ = DeleteDC(dc);
            let mut px = color?;
            for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                p.swap(0, 2); // BGRA -> RGBA
                if let Some(m) = &mask {
                    p[3] = if m[i * 4] == 0 { 255 } else { 0 };
                }
            }
            RgbaImage::from_raw(w as u32, h as u32, px)
        })();
        free(ii.hbmColor);
        free(ii.hbmMask);
        out
    }
}

// ---------- context menus ----------

thread_local! {
    /// The menu being tracked, for the host window's owner-drawn submenus ("Send to", "Open with").
    static ACTIVE_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        if matches!(
            msg,
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR | WM_MENUSELECT
        ) {
            let cm = ACTIVE_MENU.with(|m| m.borrow().clone());
            if let Some(cm) = cm {
                if let Ok(cm3) = cm.cast::<IContextMenu3>() {
                    let mut res = LRESULT(0);
                    if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut res)).is_ok() {
                        return res;
                    }
                } else if let Ok(cm2) = cm.cast::<IContextMenu2>()
                    && cm2.HandleMenuMsg(msg, wp, lp).is_ok()
                {
                    return LRESULT(0);
                }
            }
        }
        let _ = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

/// A hidden popup owned by the app window: it owns the tracked menu and
/// forwards the menu messages GPUI's window procedure would drop.
struct Host(HWND);

impl Host {
    fn new(owner: HWND) -> Option<Host> {
        unsafe {
            let inst = GetModuleHandleW(None).ok()?;
            let class = w!("GridsShellMenuHost");
            static REGISTERED: OnceLock<bool> = OnceLock::new();
            REGISTERED.get_or_init(|| {
                let wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(host_proc),
                    hInstance: inst.into(),
                    lpszClassName: class,
                    ..Default::default()
                };
                RegisterClassExW(&wc) != 0
            });
            let owner = (!owner.is_invalid()).then_some(owner);
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                owner,
                None,
                Some(inst.into()),
                None,
            )
            .ok()
            .map(Host)
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

fn key_down(vk: i32) -> bool {
    unsafe { GetKeyState(vk) < 0 }
}

fn verb_of(cm: &IContextMenu, offset: u32) -> String {
    let mut buf = [0u16; 128];
    let ok = unsafe {
        cm.GetCommandString(
            offset as usize,
            GCS_VERBW,
            None,
            PSTR(buf.as_mut_ptr() as *mut u8),
            buf.len() as u32,
        )
    }
    .is_ok();
    if !ok {
        return String::new();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

unsafe fn track(hwnd: isize, cm: IContextMenu, pos: (i32, i32), can_rename: bool) -> MenuAction {
    unsafe {
        let owner = HWND(hwnd as *mut _);
        let Ok(menu) = CreatePopupMenu() else {
            return MenuAction::Dismissed;
        };
        struct MenuGuard(HMENU);
        impl Drop for MenuGuard {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyMenu(self.0);
                }
            }
        }
        let _menu_guard = MenuGuard(menu);
        let mut flags = CMF_NORMAL;
        if can_rename {
            flags |= CMF_CANRENAME;
        }
        if key_down(VK_SHIFT.0 as i32) {
            flags |= CMF_EXTENDEDVERBS; // Shift+right click: "Copy as path", "Open PowerShell here" ...
        }
        if cm
            .QueryContextMenu(menu, 0, FIRST_CMD, LAST_CMD, flags)
            .is_err()
        {
            return MenuAction::Dismissed;
        }
        let Some(host) = Host::new(owner) else {
            return MenuAction::Dismissed;
        };
        ACTIVE_MENU.with(|m| *m.borrow_mut() = Some(cm.clone()));
        let picked = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            pos.0,
            pos.1,
            host.0,
            None,
        )
        .0 as u32;
        ACTIVE_MENU.with(|m| *m.borrow_mut() = None);
        let _ = PostMessageW(Some(host.0), WM_NULL, WPARAM(0), LPARAM(0));
        if picked < FIRST_CMD {
            return MenuAction::Dismissed;
        }
        let offset = picked - FIRST_CMD;
        let verb = verb_of(&cm, offset);
        if verb.eq_ignore_ascii_case("rename") {
            return MenuAction::Rename;
        }
        let mut mask = CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE;
        if key_down(VK_SHIFT.0 as i32) {
            mask |= CMIC_MASK_SHIFT_DOWN;
        }
        if key_down(VK_CONTROL.0 as i32) {
            mask |= CMIC_MASK_CONTROL_DOWN;
        }
        let ci = CMINVOKECOMMANDINFOEX {
            cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: mask,
            hwnd: owner,
            // MAKEINTRESOURCE(offset): the command by its offset
            lpVerb: PCSTR(offset as usize as *const u8),
            lpVerbW: PCWSTR(offset as usize as *const u16),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: POINT { x: pos.0, y: pos.1 },
            ..Default::default()
        };
        match cm.InvokeCommand(&ci as *const _ as *const CMINVOKECOMMANDINFO) {
            Ok(()) => MenuAction::Invoked(verb),
            Err(_) => MenuAction::Dismissed,
        }
    }
}

pub fn context_menu(hwnd: isize, paths: &[PathBuf], pos: (i32, i32)) -> MenuAction {
    ensure_com();
    let Ok(pidls) = paths
        .iter()
        .map(|p| Pidl::parse(p))
        .collect::<io::Result<Vec<_>>>()
    else {
        return MenuAction::Dismissed;
    };
    let raw: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| p.0 as *const _).collect();
    unsafe {
        let Ok(items) = SHCreateShellItemArrayFromIDLists(&raw) else {
            return MenuAction::Dismissed;
        };
        let cm: Result<IContextMenu, _> = items.BindToHandler(None, &BHID_SFUIObject);
        match cm {
            Ok(cm) => track(hwnd, cm, pos, true),
            Err(_) => MenuAction::Dismissed,
        }
    }
}

pub fn background_menu(hwnd: isize, folder: &Path, pos: (i32, i32)) -> MenuAction {
    ensure_com();
    let Ok(pidl) = Pidl::parse(folder) else {
        return MenuAction::Dismissed;
    };
    unsafe {
        let Ok(desktop) = SHGetDesktopFolder() else {
            return MenuAction::Dismissed;
        };
        let sf: Result<IShellFolder, _> = desktop.BindToObject(pidl.0, None);
        let Ok(sf) = sf else {
            return MenuAction::Dismissed;
        };
        match sf.CreateViewObject::<IContextMenu>(HWND(hwnd as *mut _)) {
            Ok(cm) => track(hwnd, cm, pos, false),
            Err(_) => MenuAction::Dismissed,
        }
    }
}

// ---------- file operations ----------

fn item(p: &Path) -> io::Result<IShellItem> {
    unsafe { SHCreateItemFromParsingName(&wide(p), None) }
        .map_err(|e| io::Error::other(format!("{}: {e}", p.display())))
}

fn items(paths: &[PathBuf]) -> io::Result<IShellItemArray> {
    let pidls = paths
        .iter()
        .map(|p| Pidl::parse(p))
        .collect::<io::Result<Vec<_>>>()?;
    let raw: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| p.0 as *const _).collect();
    unsafe { SHCreateShellItemArrayFromIDLists(&raw) }.map_err(win_err)
}

/// Runs one IFileOperation; no owner window, so its progress and conflict
/// dialogs belong to the calling (worker) thread, as in the original Grids.
fn file_op(
    flags: FILEOPERATION_FLAGS,
    add: impl FnOnce(&IFileOperation) -> io::Result<()>,
) -> io::Result<()> {
    ensure_com();
    unsafe {
        let op: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(win_err)?;
        op.SetOperationFlags(flags).map_err(win_err)?;
        add(&op)?;
        let done = op.PerformOperations();
        if op.GetAnyOperationsAborted().is_ok_and(|b| b.as_bool()) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        done.map_err(win_err)
    }
}

pub fn recycle(paths: &[PathBuf]) -> io::Result<()> {
    let arr = items(paths)?;
    file_op(
        FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOF_WANTNUKEWARNING | FOFX_ADDUNDORECORD,
        |op| unsafe { op.DeleteItems(&arr) }.map_err(win_err),
    )
}

pub fn delete(paths: &[PathBuf]) -> io::Result<()> {
    let arr = items(paths)?;
    file_op(FILEOPERATION_FLAGS(0), |op| {
        unsafe { op.DeleteItems(&arr) }.map_err(win_err)
    })
}

pub fn copy(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    // into the folder they already sit in: Explorer's "名称 - 副本" names; else its conflict dialog
    let same = paths.iter().all(|p| parent_is(p, dest));
    let mut flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD;
    if same {
        flags |= FOF_RENAMEONCOLLISION;
    }
    let (arr, to) = (items(paths)?, item(dest)?);
    file_op(flags, |op| {
        unsafe { op.CopyItems(&arr, &to) }.map_err(win_err)
    })
}

pub fn move_(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    let (arr, to) = (items(paths)?, item(dest)?);
    file_op(
        FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD,
        |op| unsafe { op.MoveItems(&arr, &to) }.map_err(win_err),
    )
}

// ---------- drives and known folders ----------

const DRIVE_REMOTE: u32 = 4;

pub fn drives() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    let mut old = THREAD_ERROR_MODE(0);
    // no "insert a disk" box for an empty card reader or DVD drive
    let _ = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut old)) };
    let out = (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| {
            let root = format!("{}:\\", (b'A' + i) as char);
            let wroot = HSTRING::from(root.as_str());
            let mut label = String::new();
            // a network drive can hang for seconds when offline: no label lookup there
            if unsafe { GetDriveTypeW(&wroot) } != DRIVE_REMOTE {
                let mut buf = [0u16; 261];
                if unsafe { GetVolumeInformationW(&wroot, Some(&mut buf), None, None, None, None) }
                    .is_ok()
                {
                    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
                    label = String::from_utf16_lossy(&buf[..n]);
                }
            }
            Drive {
                path: PathBuf::from(root),
                label,
            }
        })
        .collect();
    let _ = unsafe { SetThreadErrorMode(old, None) };
    out
}

fn known_path(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p: PWSTR = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.map(PathBuf::from)
    }
}

pub fn known_folders() -> Vec<KnownFolder> {
    ensure_com();
    [
        (KnownFolderKind::Home, FOLDERID_Profile),
        (KnownFolderKind::Desktop, FOLDERID_Desktop),
        (KnownFolderKind::Downloads, FOLDERID_Downloads),
        (KnownFolderKind::Documents, FOLDERID_Documents),
        (KnownFolderKind::Pictures, FOLDERID_Pictures),
    ]
    .into_iter()
    .filter_map(|(kind, id)| {
        known_path(&id)
            .filter(|p| p.is_dir())
            .map(|path| KnownFolder { kind, path })
    })
    .collect()
}

// ---------- clipboard ----------

/// The clipboard is shared: another program may hold it for a moment.
fn open_clipboard() -> io::Result<()> {
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Err(io::Error::other(
        "the clipboard is in use by another program",
    ))
}

struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

unsafe fn global_with(bytes: &[u8]) -> io::Result<HGLOBAL> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(win_err)?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err(io::Error::other("GlobalLock failed"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

unsafe fn set_data(format: u32, bytes: &[u8]) -> io::Result<()> {
    unsafe {
        let h = global_with(bytes)?;
        if let Err(e) = SetClipboardData(format, Some(HANDLE(h.0))) {
            let _ = GlobalFree(Some(h)); // owned by the system only after success
            return Err(win_err(e));
        }
        Ok(())
    }
}

/// DROPFILES header + double-NUL-terminated wide path list.
fn hdrop_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        fWide: true.into(),
        ..Default::default()
    };
    let mut out = unsafe {
        std::slice::from_raw_parts(
            &header as *const DROPFILES as *const u8,
            size_of::<DROPFILES>(),
        )
    }
    .to_vec();
    for p in paths {
        for u in p.as_os_str().to_string_lossy().encode_utf16().chain([0]) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out.extend_from_slice(&[0, 0]);
    out
}

pub fn clipboard_set_files(paths: &[PathBuf], cut: bool) -> io::Result<()> {
    open_clipboard()?;
    let _guard = ClipboardGuard;
    unsafe {
        EmptyClipboard().map_err(win_err)?;
        if paths.is_empty() {
            return Ok(());
        }
        set_data(CF_HDROP.0 as u32, &hdrop_bytes(paths))?;
        // Explorer's own values: cut = move, copy = copy | link
        let effect = if cut {
            DROPEFFECT_MOVE.0
        } else {
            DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0
        };
        set_data(
            RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT),
            &effect.to_le_bytes(),
        )
    }
}

pub fn clipboard_get_files() -> Option<(Vec<PathBuf>, bool)> {
    unsafe {
        IsClipboardFormatAvailable(CF_HDROP.0 as u32).ok()?;
        open_clipboard().ok()?;
        let _guard = ClipboardGuard;
        let h = GetClipboardData(CF_HDROP.0 as u32).ok()?;
        let drop = HDROP(h.0);
        let n = DragQueryFileW(drop, u32::MAX, None);
        let paths: Vec<PathBuf> = (0..n)
            .map(|k| {
                let mut buf = vec![0u16; DragQueryFileW(drop, k, None) as usize + 1];
                let len = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
                PathBuf::from(String::from_utf16_lossy(&buf[..len]))
            })
            .collect();
        let mut cut = false;
        if let Ok(h) = GetClipboardData(RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT)) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u32;
            if !p.is_null() {
                cut = std::ptr::read_unaligned(p) & DROPEFFECT_MOVE.0 != 0;
                let _ = GlobalUnlock(g);
            }
        }
        (!paths.is_empty()).then_some((paths, cut))
    }
}

// ---------- volumes ----------

fn volume_of(p: &Path) -> Option<String> {
    // the nearest existing ancestor: a drop target may not exist yet
    let existing = p.ancestors().find(|q| q.exists())?;
    let mut buf = [0u16; 261];
    unsafe { GetVolumePathNameW(&wide(existing), &mut buf) }.ok()?;
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..n]).to_lowercase())
}

pub fn same_volume(a: &Path, b: &Path) -> bool {
    match (volume_of(a), volume_of(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdrop_layout() {
        let b = hdrop_bytes(&[PathBuf::from(r"C:\a"), PathBuf::from(r"D:\文")]);
        let head = size_of::<DROPFILES>();
        assert_eq!(
            u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize,
            head
        );
        let chars: Vec<u16> = b[head..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let expect: Vec<u16> = "C:\\a\0D:\\文\0\0".encode_utf16().collect();
        assert_eq!(chars, expect);
    }

    #[test]
    fn icon_buckets() {
        assert_eq!(icon_list_for(16).1, 16);
        assert_eq!(icon_list_for(20).1, 32);
        assert_eq!(icon_list_for(48).1, 48);
        assert_eq!(icon_list_for(64).1, 256);
    }

    #[test]
    fn system_icons_and_folders() {
        // runs on the Windows CI runner: the shell answers without a window
        assert!(file_icon(Path::new("x.txt"), false, 16).is_some());
        assert!(file_icon(Path::new(r"C:\Windows"), true, 32).is_some());
        assert!(
            known_folders()
                .iter()
                .any(|f| f.kind == KnownFolderKind::Home)
        );
        assert!(drives().iter().any(|d| d.path == Path::new(r"C:\")));
    }

    #[test]
    fn file_ops_round_trip() {
        let d = std::env::temp_dir().join(format!("grids_win_ops_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("to")).unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        copy(&[d.join("a.txt")], &d.join("to")).unwrap();
        assert!(d.join("to").join("a.txt").is_file());
        copy(&[d.join("a.txt")], &d).unwrap(); // same folder: renamed copy
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 3);
        std::fs::create_dir(d.join("sub")).unwrap();
        move_(&[d.join("a.txt")], &d.join("sub")).unwrap();
        assert!(d.join("sub").join("a.txt").is_file() && !d.join("a.txt").exists());
        delete(&[d.join("to")]).unwrap();
        assert!(!d.join("to").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn clipboard_round_trip() {
        let files = vec![PathBuf::from(r"C:\Windows\notepad.exe")];
        if clipboard_set_files(&files, true).is_ok() {
            assert_eq!(clipboard_get_files(), Some((files, true)));
        }
    }
}
