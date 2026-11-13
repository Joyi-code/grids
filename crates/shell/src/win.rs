//! Windows implementation: Windows Shell through the `windows` crate (0.61,
//! the version GPUI 0.2.2 uses). Ported in spirit from the original Grids
//! (efww/grids, MIT), re-written for a GPUI-drawn app.

use crate::{
    Drive, KnownFolder, KnownFolderKind, LinkTarget, MenuAction, RgbaImage, classify_target,
    parent_is,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{
    ERROR_CANCELLED, ERROR_NO_ASSOCIATION, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, GlobalFree, HANDLE,
    HGLOBAL, HWND, LPARAM, LRESULT, POINT, RPC_E_CHANGED_MODE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, GetDriveTypeW,
    GetLogicalDrives, GetVolumeInformationW, GetVolumePathNameW, WIN32_FIND_DATAW,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IDataObject, IPersistFile,
    STGM_READ,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Diagnostics::Debug::{
    SEM_FAILCRITICALERRORS, SetThreadErrorMode, THREAD_ERROR_MODE,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, KEY_READ, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RRF_NOEXPAND,
    RRF_RT_ANY, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ASSOC_FILTER, ASSOC_FILTER_NONE, ASSOC_FILTER_RECOMMENDED, ASSOCF_NOTRUNCATE, ASSOCSTR,
    ASSOCSTR_EXECUTABLE, ASSOCSTR_FRIENDLYDOCNAME, AssocQueryStringW, BHID_DataObject,
    BHID_SFUIObject, CFSTR_PREFERREDDROPEFFECT, CMF_CANRENAME, CMF_EXTENDEDVERBS, CMF_NORMAL,
    CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DROPFILES, DragQueryFileW,
    FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMMKDIR, FOF_RENAMEONCOLLISION,
    FOF_WANTNUKEWARNING, FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE, FOLDERID_CommonTemplates,
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_Profile,
    FOLDERID_Templates, FileOperation, GCS_VERBW, HDROP, IAssocHandler, IContextMenu,
    IContextMenu2, IContextMenu3, IFileOperation, ILFree, IShellFolder, IShellItem,
    IShellItemArray, IShellLinkW, KF_FLAG_DEFAULT, OAIF_ALLOW_REGISTRATION, OAIF_EXEC,
    OAIF_REGISTER_EXT, OPENASINFO, SEE_MASK_FLAG_NO_UI, SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC,
    SHAssocEnumHandlers, SHCNE_CREATE, SHCNF_PATHW, SHChangeNotify, SHCreateItemFromParsingName,
    SHCreateShellItemArrayFromIDLists, SHDefExtractIconW, SHELLEXECUTEINFOW, SHFILEINFOW,
    SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES, SHGetDesktopFolder, SHGetFileInfoW,
    SHGetImageList, SHGetKnownFolderPath, SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL,
    SHLoadIndirectString, SHMultiFileProperties, SHOP_FILEPATH, SHObjectProperties,
    SHOpenFolderAndSelectItems, SHOpenWithDialog, SHParseDisplayName, SLGP_RAWPATH,
    ShellExecuteExW, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
    GetIconInfo, HICON, HMENU, ICONINFO, PostMessageW, RegisterClassExW, SW_SHOWNORMAL,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WINDOW_EX_STYLE, WM_DRAWITEM,
    WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR, WM_MENUSELECT, WM_NULL, WNDCLASSEXW, WS_POPUP,
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

/// A NUL-terminated (or full) UTF-16 buffer as a String.
fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(buf.get(..n).unwrap_or_default())
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
        ensure_com();
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

/// One ShellExecuteExW call. `verb` null = the default verb, as Explorer's
/// double click. NOASYNC: done when it returns, so a worker thread may exit
/// right after. With a PIDL the shell invokes the item's own context menu
/// (INVOKEIDLIST), which is what Explorer does for .lnk, .url and friends.
fn shell_execute(
    verb: PCWSTR,
    file: &HSTRING,
    dir: &HSTRING,
    pidl: Option<&Pidl>,
    mask: u32,
) -> windows::core::Result<()> {
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | mask,
        lpVerb: verb,
        lpFile: PCWSTR(file.as_ptr()),
        lpDirectory: if dir.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(dir.as_ptr())
        },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if let Some(p) = pidl {
        sei.fMask |= SEE_MASK_INVOKEIDLIST;
        sei.lpIDList = p.0 as *mut _;
    }
    unsafe { ShellExecuteExW(&mut sei) }
}

// some handlers report the old ShellExecute code 31 (SE_ERR_NOASSOC) through GetLastError
const SE_ERR_NOASSOC_HRESULT: windows::core::HRESULT =
    windows::core::HRESULT(0x8007_001F_u32 as i32);

fn no_association(e: &windows::core::Error) -> bool {
    e.code() == ERROR_NO_ASSOCIATION.to_hresult() || e.code() == SE_ERR_NOASSOC_HRESULT
}

/// `openas`: on "no program for this type", show the Open-with dialog.
fn open_default(path: &Path, openas: bool) -> io::Result<()> {
    ensure_com();
    let file = wide(path);
    let dir = path.parent().map(wide).unwrap_or_default();
    let pidl = Pidl::parse(path).ok(); // an unreachable UNC path may have none: lpFile alone then
    let first = shell_execute(
        PCWSTR::null(),
        &file,
        &dir,
        pidl.as_ref(),
        SEE_MASK_FLAG_NO_UI,
    );
    let result = match first {
        Err(e) if openas && no_association(&e) => {
            shell_execute(w!("openas"), &file, &dir, pidl.as_ref(), 0)
        }
        other => other,
    };
    result.map_err(|e| {
        let why = e.message();
        let why = if why.trim().is_empty() {
            format!("{:#010x}", e.code().0)
        } else {
            why.trim().to_string()
        };
        io::Error::other(format!("{}: {why}", path.display()))
    })
}

/// Opens like Explorer's double click / Enter: the default verb through the
/// item's PIDL, in its folder, without UI on the first try (a broken
/// shortcut or unreachable target comes back as an error with the system's
/// reason); only "no program for this type" falls back to the Open-with
/// dialog. Blocks; safe on any thread (STA COM per thread).
pub fn open_with_system(path: &Path) -> io::Result<()> {
    open_default(path, true)
}

// ---------- shortcuts ----------

fn expand_env(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let src = HSTRING::from(s);
    let need = unsafe { ExpandEnvironmentStringsW(&src, None) } as usize;
    if need == 0 {
        return s.to_string();
    }
    let mut buf = vec![0u16; need + 1];
    let got = unsafe { ExpandEnvironmentStringsW(&src, Some(&mut buf)) } as usize;
    if got == 0 || got > buf.len() {
        return s.to_string();
    }
    from_wide(&buf)
}

/// Reads a .lnk without ever resolving it (no link tracking, no search, no
/// UI): IShellLinkW + IPersistFile::Load, GetPath(SLGP_RAWPATH), env vars
/// expanded. `Err` when the file is not a shortcut.
pub fn resolve_link(lnk: &Path) -> io::Result<LinkTarget> {
    ensure_com();
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(win_err)?;
        let file: IPersistFile = link.cast().map_err(win_err)?;
        file.Load(&wide(lnk), STGM_READ).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {e}", lnk.display()),
            )
        })?;
        let mut buf = vec![0u16; 32_768];
        let mut fd = WIN32_FIND_DATAW::default();
        // S_FALSE (no file system path) is Ok with an empty buffer
        if link
            .GetPath(&mut buf, &mut fd, SLGP_RAWPATH.0 as u32)
            .is_err()
        {
            return Ok(LinkTarget::Other);
        }
        let raw = from_wide(&buf);
        Ok(classify_target(&expand_env(raw.trim())))
    }
}

/// Writes a shortcut `lnk` pointing at `target`.
pub fn create_link(lnk: &Path, target: &Path) -> io::Result<()> {
    ensure_com();
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(win_err)?;
        link.SetPath(&wide(target)).map_err(win_err)?;
        if let Some(dir) = target.parent() {
            let _ = link.SetWorkingDirectory(&wide(dir));
        }
        let file: IPersistFile = link.cast().map_err(win_err)?;
        file.Save(&wide(lnk), true).map_err(win_err)
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
        if hicon.is_invalid() {
            return None;
        }
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
                    p[3] = if m.get(i * 4).copied().unwrap_or(1) == 0 {
                        255
                    } else {
                        0
                    };
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
        if matches!(
            msg,
            WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR | WM_MENUSELECT
        ) {
            // nothing here may panic: an unwind out of a window procedure aborts
            let cm = ACTIVE_MENU
                .try_with(|m| m.try_borrow().ok().and_then(|b| b.clone()))
                .ok()
                .flatten();
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
    from_wide(&buf)
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
        let _ = ACTIVE_MENU.try_with(|m| m.try_borrow_mut().map(|mut b| *b = Some(cm.clone())));
        let picked = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            pos.0,
            pos.1,
            host.0,
            None,
        )
        .0 as u32;
        let _ = ACTIVE_MENU.try_with(|m| m.try_borrow_mut().map(|mut b| *b = None));
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
    ensure_com();
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
    recycle_with(paths, FILEOPERATION_FLAGS(0))
}

pub fn delete(paths: &[PathBuf]) -> io::Result<()> {
    delete_with(paths, FILEOPERATION_FLAGS(0))
}

pub fn copy(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    copy_with(paths, dest, FILEOPERATION_FLAGS(0))
}

pub fn move_(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    move_with(paths, dest, FILEOPERATION_FLAGS(0))
}

// `extra`: FOF_NO_UI in the tests, so no dialog can wait for a click on the CI runner.

fn recycle_with(paths: &[PathBuf], extra: FILEOPERATION_FLAGS) -> io::Result<()> {
    let arr = items(paths)?;
    let flags =
        FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOF_WANTNUKEWARNING | FOFX_ADDUNDORECORD | extra;
    file_op(flags, |op| unsafe { op.DeleteItems(&arr) }.map_err(win_err))
}

fn delete_with(paths: &[PathBuf], extra: FILEOPERATION_FLAGS) -> io::Result<()> {
    let arr = items(paths)?;
    file_op(extra, |op| unsafe { op.DeleteItems(&arr) }.map_err(win_err))
}

fn copy_with(paths: &[PathBuf], dest: &Path, extra: FILEOPERATION_FLAGS) -> io::Result<()> {
    // into the folder they already sit in: Explorer's "名称 - 副本" names; else its conflict dialog
    let same = paths.iter().all(|p| parent_is(p, dest));
    let mut flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD | extra;
    if same {
        flags |= FOF_RENAMEONCOLLISION;
    }
    let (arr, to) = (items(paths)?, item(dest)?);
    file_op(flags, |op| {
        unsafe { op.CopyItems(&arr, &to) }.map_err(win_err)
    })
}

fn move_with(paths: &[PathBuf], dest: &Path, extra: FILEOPERATION_FLAGS) -> io::Result<()> {
    let (arr, to) = (items(paths)?, item(dest)?);
    file_op(
        FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD | extra,
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
                    label = from_wide(&buf);
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
                let len = (DragQueryFileW(drop, k, Some(&mut buf)) as usize).min(buf.len());
                PathBuf::from(from_wide(buf.get(..len).unwrap_or_default()))
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
    Some(from_wide(&buf).to_lowercase())
}

pub fn same_volume(a: &Path, b: &Path) -> bool {
    match (volume_of(a), volume_of(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

// ---------- open with ----------

/// A CoTaskMem string as a String, freed.
unsafe fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as _)) };
    s
}

/// ".txt" for a path; None without an extension.
fn dot_ext(path: &Path) -> Option<String> {
    let e = path.extension()?.to_string_lossy().to_lowercase();
    (!e.is_empty()).then(|| format!(".{e}"))
}

fn assoc_string(kind: ASSOCSTR, assoc: &str) -> Option<String> {
    let src = HSTRING::from(assoc);
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let hr = unsafe {
        AssocQueryStringW(
            ASSOCF_NOTRUNCATE,
            kind,
            &src,
            PCWSTR::null(),
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    };
    let s = from_wide(&buf);
    (hr.is_ok() && !s.trim().is_empty()).then(|| s.trim().to_string())
}

fn assoc_handlers(ext: &str, filter: ASSOC_FILTER) -> Vec<IAssocHandler> {
    ensure_com();
    let Ok(list) = (unsafe { SHAssocEnumHandlers(&HSTRING::from(ext), filter) }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // a broken handler registration must not loop forever
    for _ in 0..256 {
        let mut one = [None];
        let mut got = 0u32;
        if unsafe { list.Next(&mut one, Some(&mut got)) }.is_err() || got == 0 {
            break;
        }
        if let [Some(h)] = one {
            out.push(h);
        }
    }
    out
}

static HANDLER_ICONS: OnceLock<Mutex<HashMap<String, Option<Arc<RgbaImage>>>>> = OnceLock::new();

/// About 32 px: a resource icon (`file,index`) or, for Store apps, the
/// package image an `@{...}` indirect string points at.
fn handler_icon(h: &IAssocHandler) -> Option<Arc<RgbaImage>> {
    let mut loc = PWSTR::null();
    let mut index = 0i32;
    unsafe { h.GetIconLocation(&mut loc, &mut index) }.ok()?;
    let loc = unsafe { take_pwstr(loc) };
    if loc.trim().is_empty() {
        return None;
    }
    let key = format!("{loc},{index}");
    let cache = HANDLER_ICONS.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok()?.get(&key) {
        return hit.clone();
    }
    let img = load_handler_icon(&loc, index).map(Arc::new);
    if let Ok(mut c) = cache.lock() {
        c.insert(key, img.clone());
    }
    img
}

fn load_handler_icon(loc: &str, index: i32) -> Option<RgbaImage> {
    if loc.starts_with('@') {
        let file = indirect_string(loc)?;
        let img = image::open(&file).ok()?.to_rgba8();
        if img.width() > 48 || img.height() > 48 {
            return Some(image::imageops::resize(
                &img,
                32,
                32,
                image::imageops::FilterType::Triangle,
            ));
        }
        return Some(img);
    }
    let file = HSTRING::from(expand_env(loc.trim_matches('"')));
    let mut large = HICON::default();
    let hr = unsafe { SHDefExtractIconW(&file, index, 0, Some(&mut large), None, 32) };
    if hr.is_err() || large.is_invalid() {
        return None;
    }
    let img = unsafe { hicon_to_rgba(large) };
    let _ = unsafe { DestroyIcon(large) };
    img
}

/// "@dll,-123" / "@{package?ms-resource://...}" resolved; plain text as is.
fn indirect_string(s: &str) -> Option<String> {
    if !s.starts_with('@') {
        return Some(s.to_string());
    }
    let mut buf = vec![0u16; 4096];
    unsafe { SHLoadIndirectString(&HSTRING::from(s), &mut buf, None) }.ok()?;
    let out = from_wide(&buf);
    (!out.trim().is_empty()).then(|| out.trim().to_string())
}

pub fn open_with_handlers(path: &Path) -> Vec<crate::Handler> {
    let Some(ext) = dot_ext(path) else {
        return Vec::new();
    };
    let default = assoc_string(ASSOCSTR_EXECUTABLE, &ext).map(|s| s.to_lowercase());
    let mut out: Vec<crate::Handler> = Vec::new();
    for h in assoc_handlers(&ext, ASSOC_FILTER_RECOMMENDED) {
        let id = unsafe { h.GetName() }
            .map(|p| unsafe { take_pwstr(p) })
            .unwrap_or_default();
        if id.is_empty() || out.iter().any(|o| o.id.eq_ignore_ascii_case(&id)) {
            continue;
        }
        let name = unsafe { h.GetUIName() }
            .map(|p| unsafe { take_pwstr(p) })
            .ok()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| {
                Path::new(&id)
                    .file_stem()
                    .map_or_else(|| id.clone(), |s| s.to_string_lossy().into_owned())
            });
        let icon = handler_icon(&h);
        out.push(crate::Handler { id, name, icon });
    }
    // the shell usually lists the default first already; make sure
    if let Some(d) = default
        && let Some(i) = out.iter().position(|h| h.id.to_lowercase() == d)
    {
        let h = out.remove(i);
        out.insert(0, h);
    }
    out
}

fn data_object(paths: &[PathBuf]) -> io::Result<IDataObject> {
    let arr = items(paths)?;
    unsafe { arr.BindToHandler(None, &BHID_DataObject) }.map_err(win_err)
}

pub fn open_with(path: &Path, handler_id: &str) -> io::Result<()> {
    let ext = dot_ext(path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: no file type", path.display()),
        )
    })?;
    let handler = [ASSOC_FILTER_RECOMMENDED, ASSOC_FILTER_NONE]
        .into_iter()
        .flat_map(|f| assoc_handlers(&ext, f))
        .find(|h| {
            unsafe { h.GetName() }
                .map(|p| unsafe { take_pwstr(p) })
                .is_ok_and(|n| n.eq_ignore_ascii_case(handler_id))
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{handler_id}: not a program for {ext}"),
            )
        })?;
    let data = data_object(&[path.to_path_buf()])?;
    unsafe { handler.Invoke(&data) }
        .map_err(|e| io::Error::other(format!("{}: {}", path.display(), e.message().trim())))
}

const CANCELLED: windows::core::HRESULT = ERROR_CANCELLED.to_hresult();

pub fn open_with_dialog(hwnd: isize, path: &Path) -> io::Result<()> {
    ensure_com();
    let file = wide(path);
    let info = OPENASINFO {
        pcszFile: PCWSTR(file.as_ptr()),
        pcszClass: PCWSTR::null(),
        // as Explorer's "选择其他应用": open with the choice, offer "always use this app"
        oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC,
    };
    let owner = (hwnd != 0).then_some(HWND(hwnd as *mut _));
    match unsafe { SHOpenWithDialog(owner, &info) } {
        Ok(()) => Ok(()),
        Err(e) if e.code() == CANCELLED => {
            Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
        }
        Err(e) => Err(io::Error::other(format!(
            "{}: {}",
            path.display(),
            e.message().trim()
        ))),
    }
}

pub fn properties(hwnd: isize, paths: &[PathBuf]) -> io::Result<()> {
    ensure_com();
    match paths {
        [] => Err(io::Error::new(io::ErrorKind::InvalidInput, "no items")),
        [one] => {
            let owner = (hwnd != 0).then_some(HWND(hwnd as *mut _));
            let ok =
                unsafe { SHObjectProperties(owner, SHOP_FILEPATH, &wide(one), PCWSTR::null()) };
            if ok.as_bool() {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "{}: {}",
                    one.display(),
                    io::Error::last_os_error()
                )))
            }
        }
        many => {
            let data = data_object(many)?;
            unsafe { SHMultiFileProperties(&data, 0) }.map_err(win_err)
        }
    }
}

// ---------- new items ----------

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn open_key(sub: &str) -> Option<Key> {
    let mut k = HKEY::default();
    let r = unsafe {
        RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            &HSTRING::from(sub),
            None,
            KEY_READ,
            &mut k,
        )
    };
    (r == ERROR_SUCCESS).then_some(Key(k))
}

/// A value's type and bytes; `name` None = the key's default value.
fn reg_value(key: &Key, name: Option<&str>) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let name = name.map(HSTRING::from);
    let pname = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    let flags = RRF_RT_ANY | RRF_NOEXPAND;
    let mut ty = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let r = unsafe {
        RegGetValueW(
            key.0,
            PCWSTR::null(),
            pname,
            flags,
            Some(&mut ty),
            None,
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let mut buf = vec![0u8; size as usize + 2];
    let mut size = buf.len() as u32;
    let r = unsafe {
        RegGetValueW(
            key.0,
            PCWSTR::null(),
            pname,
            flags,
            Some(&mut ty),
            Some(buf.as_mut_ptr() as _),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    buf.truncate(size as usize);
    Some((ty, buf))
}

fn reg_string(key: &Key, name: Option<&str>) -> Option<String> {
    let (ty, bytes) = reg_value(key, name)?;
    if ty != REG_SZ && ty != REG_EXPAND_SZ {
        return None;
    }
    let wide: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Some(from_wide(&wide))
}

enum Template {
    Empty,
    Bytes(Vec<u8>),
    File(String),
}

struct ShellNew {
    template: Template,
    /// ItemName: the base name for new files ("新建 文本文档"), localized.
    item_name: Option<String>,
}

/// `HKCR\.ext\ShellNew` or `HKCR\.ext\<ProgID>\ShellNew`, when it makes a
/// file from NullFile / Data / FileName (Command, Handler and Directory
/// entries run code or make folders: left out).
fn shell_new(ext: &str) -> Option<ShellNew> {
    let progid = open_key(ext).and_then(|k| reg_string(&k, None));
    let mut places = vec![format!("{ext}\\ShellNew")];
    if let Some(p) = progid.filter(|p| !p.is_empty() && !p.contains(['\\', '/'])) {
        places.push(format!("{ext}\\{p}\\ShellNew"));
    }
    for place in places {
        let Some(k) = open_key(&place) else { continue };
        if ["Command", "Handler", "Directory"]
            .iter()
            .any(|v| reg_value(&k, Some(v)).is_some())
        {
            continue;
        }
        let template = if reg_value(&k, Some("NullFile")).is_some() {
            Template::Empty
        } else if let Some(f) = reg_string(&k, Some("FileName")).filter(|f| !f.trim().is_empty()) {
            Template::File(f)
        } else if let Some((ty, bytes)) = reg_value(&k, Some("Data")) {
            if ty == REG_SZ || ty == REG_EXPAND_SZ {
                let wide: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                Template::Bytes(from_wide(&wide).into_bytes())
            } else {
                Template::Bytes(bytes)
            }
        } else {
            continue;
        };
        let item_name = reg_string(&k, Some("ItemName"))
            .and_then(|s| indirect_string(&expand_env(&s)))
            .filter(|s| crate::check_name(s).is_ok());
        return Some(ShellNew {
            template,
            item_name,
        });
    }
    None
}

fn type_name(ext: &str) -> String {
    assoc_string(ASSOCSTR_FRIENDLYDOCNAME, ext)
        .unwrap_or_else(|| format!("{} 文件", ext.trim_start_matches('.').to_uppercase()))
}

fn class_extensions() -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = vec![0u16; 512];
    for i in 0..200_000u32 {
        let mut len = buf.len() as u32;
        let r = unsafe {
            RegEnumKeyExW(
                HKEY_CLASSES_ROOT,
                i,
                Some(PWSTR(buf.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if r == ERROR_NO_MORE_ITEMS {
            break;
        }
        if r != ERROR_SUCCESS {
            continue;
        }
        let name = String::from_utf16_lossy(buf.get(..len as usize).unwrap_or_default());
        if name.len() > 1 && name.starts_with('.') {
            out.push(name.to_lowercase());
        }
    }
    out
}

static NEW_ITEMS: Mutex<Option<(std::time::Instant, Vec<crate::NewItem>)>> = Mutex::new(None);

pub fn new_items() -> Vec<crate::NewItem> {
    if let Ok(c) = NEW_ITEMS.lock()
        && let Some((at, items)) = c.as_ref()
        && at.elapsed() < std::time::Duration::from_secs(30)
    {
        return items.clone();
    }
    ensure_com();
    let mut items: Vec<crate::NewItem> = class_extensions()
        .into_iter()
        .filter(|ext| ext != ".lnk" && crate::norm_ext(ext).is_ok() && shell_new(ext).is_some())
        .map(|ext| crate::NewItem {
            name: type_name(&ext),
            icon: file_icon(Path::new(&format!("x{ext}")), false, 32),
            ext,
        })
        .collect();
    if !items.iter().any(|i| i.ext == ".txt") {
        items.push(crate::NewItem {
            name: assoc_string(ASSOCSTR_FRIENDLYDOCNAME, ".txt")
                .unwrap_or_else(|| "文本文档".into()),
            icon: file_icon(Path::new("x.txt"), false, 32),
            ext: ".txt".into(),
        });
    }
    items.sort_by_cached_key(|i| (i.name.to_lowercase(), i.ext.clone()));
    items.dedup_by(|a, b| a.ext == b.ext);
    if let Ok(mut c) = NEW_ITEMS.lock() {
        *c = Some((std::time::Instant::now(), items.clone()));
    }
    items
}

/// FileName templates: absolute, or in the user's / common Templates folder
/// or %SystemRoot%\ShellNew, as Explorer looks them up.
fn find_template(name: &str) -> Option<PathBuf> {
    let name = expand_env(name.trim_matches('"'));
    let p = PathBuf::from(&name);
    if p.is_absolute() {
        return p.is_file().then_some(p);
    }
    let windir = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("ShellNew"));
    [
        known_path(&FOLDERID_Templates),
        known_path(&FOLDERID_CommonTemplates),
        windir,
    ]
    .into_iter()
    .flatten()
    .map(|d| d.join(&name))
    .find(|f| f.is_file())
}

pub fn create_new(dir: &Path, ext: &str) -> io::Result<PathBuf> {
    ensure_com();
    let ext = crate::norm_ext(ext)?;
    let entry = shell_new(&ext);
    let base = entry
        .as_ref()
        .and_then(|e| e.item_name.clone())
        .unwrap_or_else(|| crate::new_base_name(&type_name(&ext)));
    // a missing FileName template gives an empty file, which Office and most programs open as new
    let contents = match entry.map(|e| e.template) {
        Some(Template::Bytes(b)) => b,
        Some(Template::File(f)) => match find_template(&f) {
            Some(t) => std::fs::read(t)?,
            None => Vec::new(),
        },
        Some(Template::Empty) | None => Vec::new(),
    };
    let path = crate::create_numbered(dir, &base, &ext, &contents)?;
    let w = wide(&path);
    unsafe { SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(w.as_ptr() as _), None) };
    Ok(path)
}

// ---------- terminal ----------

const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Windows Terminal's app execution alias (a reparse point: exists() can't see it).
fn windows_terminal() -> Option<PathBuf> {
    let alias = std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join(r"Microsoft\WindowsApps\wt.exe"));
    let on_path = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("wt.exe"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    alias
        .into_iter()
        .chain(on_path)
        .find(|p| std::fs::symlink_metadata(p).is_ok())
}

pub fn open_terminal(dir: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: not a folder", dir.display()),
        ));
    }
    let quiet = |c: &mut Command| {
        c.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    };
    if let Some(wt) = windows_terminal() {
        // wt splits its command line at ';'; a trailing '\' would escape the closing quote
        let mut d = dir.to_string_lossy().replace(';', "\\;");
        if d.ends_with('\\') && !d.ends_with(":\\") {
            d.pop();
        }
        let mut c = Command::new(wt);
        c.arg("-d").arg(d).current_dir(dir);
        quiet(&mut c);
        if c.spawn().is_ok() {
            return Ok(());
        }
    }
    let system = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let ps = system.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut c = Command::new(&ps);
    c.arg("-NoExit")
        .current_dir(dir)
        .creation_flags(CREATE_NEW_CONSOLE);
    match c.spawn() {
        Ok(_) => Ok(()),
        Err(_) => Command::new(system.join(r"System32\cmd.exe"))
            .arg("/K")
            .current_dir(dir)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ()),
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
        use windows::Win32::UI::Shell::FOF_NO_UI;
        let d = std::env::temp_dir().join(format!("grids_win_ops_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("to")).unwrap();
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        copy_with(&[d.join("a.txt")], &d.join("to"), FOF_NO_UI).unwrap();
        assert!(d.join("to").join("a.txt").is_file());
        copy_with(&[d.join("a.txt")], &d, FOF_NO_UI).unwrap(); // same folder: renamed copy
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 4);
        move_with(&[d.join("a.txt")], &d.join("sub"), FOF_NO_UI).unwrap();
        assert!(d.join("sub").join("a.txt").is_file() && !d.join("a.txt").exists());
        delete_with(&[d.join("to")], FOF_NO_UI).unwrap();
        assert!(!d.join("to").exists());
        recycle_with(&[d.join("sub")], FOF_NO_UI).unwrap();
        assert!(!d.join("sub").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn clipboard_round_trip() {
        let files = vec![PathBuf::from(r"C:\Windows\notepad.exe")];
        if clipboard_set_files(&files, true).is_ok() {
            assert_eq!(clipboard_get_files(), Some((files, true)));
        }
    }

    /// Runs `f` on its own thread; fails the test instead of hanging the job.
    fn within<T: Send + 'static>(
        secs: u64,
        what: &str,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(secs))
            .unwrap_or_else(|_| panic!("{what}: no answer within {secs} s"))
    }

    fn same_file(a: &Path, b: &Path) -> bool {
        std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
    }

    #[test]
    fn shortcuts_resolve_without_ui() {
        let d = std::env::temp_dir().join(format!("grids_win_lnk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("目标 文件夹")).unwrap();
        std::fs::write(d.join("报告 1.txt"), "x").unwrap();
        std::fs::write(d.join("gone.txt"), "x").unwrap();
        create_link(&d.join("dir.lnk"), &d.join("目标 文件夹")).unwrap();
        create_link(&d.join("file.lnk"), &d.join("报告 1.txt")).unwrap();
        create_link(&d.join("missing.lnk"), &d.join("gone.txt")).unwrap();
        std::fs::remove_file(d.join("gone.txt")).unwrap();
        std::fs::write(d.join("fake.lnk"), "not a shortcut").unwrap();

        match resolve_link(&d.join("dir.lnk")).unwrap() {
            LinkTarget::Folder(p) => assert!(same_file(&p, &d.join("目标 文件夹")), "{p:?}"),
            other => panic!("dir.lnk: {other:?}"),
        }
        match resolve_link(&d.join("file.lnk")).unwrap() {
            LinkTarget::File(p) => assert!(same_file(&p, &d.join("报告 1.txt")), "{p:?}"),
            other => panic!("file.lnk: {other:?}"),
        }
        match resolve_link(&d.join("missing.lnk")).unwrap() {
            LinkTarget::Missing(p) => assert_eq!(p.file_name().unwrap(), "gone.txt"),
            other => panic!("missing.lnk: {other:?}"),
        }
        assert!(!matches!(
            resolve_link(&d.join("fake.lnk")),
            Ok(LinkTarget::Folder(_) | LinkTarget::File(_) | LinkTarget::Missing(_))
        ));

        let unc = Path::new(r"\\localhost\C$\Windows");
        if unc.is_dir() {
            create_link(&d.join("unc.lnk"), unc).unwrap();
            match resolve_link(&d.join("unc.lnk")).unwrap() {
                LinkTarget::Folder(p) => assert!(
                    p.to_string_lossy()
                        .to_lowercase()
                        .starts_with(r"\\localhost\"),
                    "{p:?}"
                ),
                other => panic!("unc.lnk: {other:?}"),
            }
        } else {
            eprintln!("skipping UNC shortcut: {} not reachable", unc.display());
        }

        // icons for every kind of shortcut, from several threads at once: no panic
        let lnks: Vec<PathBuf> = ["dir.lnk", "file.lnk", "missing.lnk", "fake.lnk", "unc.lnk"]
            .iter()
            .map(|n| d.join(n))
            .collect();
        let workers: Vec<_> = (0..4)
            .map(|k| {
                let lnks = lnks.clone();
                std::thread::spawn(move || {
                    for p in &lnks {
                        let _ = file_icon(p, false, [16, 32, 48, 256][k]);
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().expect("file_icon panicked");
        }

        // a shortcut to nothing opens with an error, not a dialog
        let missing = d.join("missing.lnk");
        let r = within(60, "open missing.lnk", move || {
            open_default(&missing, false)
        });
        assert!(r.is_err(), "missing target opened: {r:?}");
        eprintln!("missing.lnk: {}", r.unwrap_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn open_like_explorer() {
        let d = std::env::temp_dir().join(format!("grids_win_open_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let txt = d.join("打开 测试 1.txt");
        std::fs::write(&txt, "Grids").unwrap();
        let r = within(60, "open .txt", move || open_with_system(&txt));
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", "notepad.exe", "/F"])
            .output();
        assert!(r.is_ok(), "{r:?}");
        // no extension: no program, and with no Open-with dialog allowed it must just come back
        let bare = d.join("无扩展名");
        std::fs::write(&bare, "x").unwrap();
        let r = within(60, "open file without extension", move || {
            open_default(&bare, false)
        });
        eprintln!("no extension: {r:?}");
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", "OpenWith.exe", "/F"])
            .output();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn open_with_and_new_menu() {
        let handlers = within(60, "open_with_handlers", || {
            open_with_handlers(Path::new(r"C:\x.txt"))
                .into_iter()
                .map(|h| (h.id, h.name, h.icon.map(|i| i.dimensions())))
                .collect::<Vec<_>>()
        });
        println!("handlers for .txt: {handlers:?}");
        assert!(!handlers.is_empty(), "no handlers for .txt");
        assert!(open_with_handlers(Path::new(r"C:\no_extension")).is_empty());
        assert!(open_with_handlers(Path::new(r"C:\x.grids-unknown-type")).len() < 100);

        let items = within(60, "new_items", new_items);
        println!(
            "new items: {:?}",
            items
                .iter()
                .map(|i| (&i.ext, &i.name, i.icon.is_some()))
                .collect::<Vec<_>>()
        );
        assert!(items.iter().any(|i| i.ext == ".txt"));
        assert!(
            items
                .iter()
                .all(|i| i.ext.starts_with('.') && !i.name.is_empty())
        );

        let d = std::env::temp_dir().join(format!("grids_win_new_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let a = create_new(&d, ".txt").unwrap();
        let b = create_new(&d, ".txt").unwrap();
        println!("created {a:?} then {b:?}");
        assert!(
            b.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(" (2).txt")
        );
        for i in &items {
            if i.ext != ".txt" {
                let p = create_new(&d, &i.ext);
                println!("new {}: {p:?}", i.ext);
                assert!(p.is_ok_and(|p| p.is_file()));
            }
        }

        // open with an explicit handler (Notepad on the runner), then close it
        if let Some((id, _, _)) = handlers
            .iter()
            .find(|h| h.0.to_lowercase().contains("notepad"))
        {
            let (a, id) = (a.clone(), id.clone());
            let r = within(60, "open_with notepad", move || open_with(&a, &id));
            std::thread::sleep(std::time::Duration::from_secs(2));
            let _ = std::process::Command::new("taskkill")
                .args(["/IM", "notepad.exe", "/F"])
                .output();
            assert!(r.is_ok(), "{r:?}");
        }
        assert_eq!(
            open_with(&a, r"C:\no\such.exe").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_opens_in_windows() {
        let d = std::env::temp_dir().join(format!("grids_win_zip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("资料 夹").join("内层")).unwrap();
        std::fs::write(d.join("资料 夹").join("内层").join("说明.txt"), "你好").unwrap();
        std::fs::write(d.join("报告 1.txt"), "x".repeat(10_000)).unwrap();
        let zip = d.join("归档.zip");
        crate::compress_zip(&[d.join("资料 夹"), d.join("报告 1.txt")], &zip).unwrap();
        let out = d.join("out");
        let st = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Expand-Archive -LiteralPath $env:GRIDS_ZIP -DestinationPath $env:GRIDS_OUT",
            ])
            .env("GRIDS_ZIP", &zip)
            .env("GRIDS_OUT", &out)
            .status()
            .unwrap();
        assert!(st.success());
        assert_eq!(
            std::fs::read_to_string(out.join("资料 夹").join("内层").join("说明.txt")).unwrap(),
            "你好"
        );
        assert_eq!(std::fs::read(out.join("报告 1.txt")).unwrap().len(), 10_000);
        let _ = std::fs::remove_dir_all(&d);
    }
}
