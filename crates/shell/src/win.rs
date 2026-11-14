//! Windows implementation: Windows Shell through the `windows` crate (0.61,
//! the version GPUI 0.2.2 uses). Ported in spirit from the original Grids
//! (efww/grids, MIT), re-written for a GPUI-drawn app.

use crate::{
    Drive, KnownFolder, KnownFolderKind, LinkTarget, MenuAction, OpItem, RgbaImage,
    classify_target, parent_is,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_CANCELLED, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA,
    ERROR_NO_ASSOCIATION, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, GlobalFree, HANDLE, HGLOBAL, HWND,
    LPARAM, LRESULT, POINT, RPC_E_CHANGED_MODE, S_OK, SIZE, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, GetDriveTypeW,
    GetLogicalDrives, GetLongPathNameW, GetVolumeInformationW, GetVolumePathNameW,
    WIN32_FIND_DATAW,
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
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::System::Ole::{CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, KEY_READ, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RRF_NOEXPAND,
    RRF_RT_ANY, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmEndSession, RmGetList, RmRegisterResources,
    RmStartSession,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ASSOC_FILTER, ASSOC_FILTER_NONE, ASSOC_FILTER_RECOMMENDED, ASSOCF_NOTRUNCATE, ASSOCSTR,
    ASSOCSTR_EXECUTABLE, ASSOCSTR_FRIENDLYDOCNAME, AssocQueryStringW, BHID_DataObject,
    BHID_SFUIObject, CFSTR_PREFERREDDROPEFFECT, CMF_CANRENAME, CMF_EXTENDEDVERBS, CMF_NORMAL,
    CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DROPFILES, DragQueryFileW,
    FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR,
    FOF_RENAMEONCOLLISION, FOF_WANTNUKEWARNING, FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE,
    FOLDERID_CommonTemplates, FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads,
    FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Templates, FileOperation, GCS_VERBW, HDROP,
    IAssocHandler, IContextMenu, IContextMenu2, IContextMenu3, IFileOperation, ILFree,
    IShellFolder, IShellItem, IShellItemArray, IShellLinkW, KF_FLAG_DEFAULT,
    OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT, OPENASINFO, SEE_MASK_FLAG_NO_UI,
    SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC, SHAssocEnumHandlers, SHCNE_CREATE, SHCNF_PATHW,
    SHChangeNotify, SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists,
    SHDefExtractIconW, SHELLEXECUTEINFOW, SHFILEINFOW, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES,
    SHGetDesktopFolder, SHGetFileInfoW, SHGetImageList, SHGetKnownFolderPath, SHIL_EXTRALARGE,
    SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL, SHLoadIndirectString, SHMultiFileProperties, SHOP_FILEPATH,
    SHObjectProperties, SHOpenFolderAndSelectItems, SHOpenWithDialog, SHParseDisplayName,
    SHQUERYRBINFO, SHQueryRecycleBinW, SLGP_RAWPATH, ShellExecuteExW, ShellLink,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_RecycleBinFolder, IEnumShellItems, IFileOperationProgressSink,
    IFileOperationProgressSink_Impl, IShellItemImageFactory, SHGetKnownFolderItem,
    SIGDN_FILESYSPATH, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
    EnumWindows, GW_OWNER, GetIconInfo, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId,
    HICON, HMENU, ICONINFO, IsIconic, IsWindowVisible, PostMessageW, RegisterClassExW, SW_RESTORE,
    SW_SHOWNORMAL, SetForegroundWindow, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WINDOW_EX_STYLE, WM_CLOSE, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM,
    WM_MENUCHAR, WM_MENUSELECT, WM_NULL, WNDCLASSEXW, WS_POPUP,
};
use windows::core::{
    BOOL, GUID, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, PSTR, PWSTR, Ref, implement, w,
};

const CMIC_MASK_UNICODE: u32 = 0x4000;
const CMIC_MASK_FLAG_NO_UI: u32 = 0x0400;
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

/// Opens a URL with its default handler, normally the browser. No download and
/// no parsing happens here: the shell hands the string to the registered
/// protocol handler.
pub fn open_url(url: &str) -> io::Result<()> {
    ensure_com();
    shell_execute(PCWSTR::null(), &HSTRING::from(url), &HSTRING::new(), None, SEE_MASK_FLAG_NO_UI).map_err(win_err)
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

/// `p` with 8.3 short components expanded ("C:\Users\RUNNER~1" ->
/// "C:\Users\runneradmin"), the form the shell reports. Works for a path
/// that no longer exists by expanding its nearest existing parent.
fn long_path(p: &Path) -> PathBuf {
    let w = wide(p);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetLongPathNameW(&w, Some(&mut buf)) } as usize;
    if n > 0 && n < buf.len() {
        return PathBuf::from(from_wide(&buf));
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) if parent != p => long_path(parent).join(name),
        _ => p.to_path_buf(),
    }
}

/// File-system path of a shell item; `None` for virtual items.
fn fs_path(item: &IShellItem) -> Option<PathBuf> {
    unsafe {
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.map(PathBuf::from)
    }
}

/// Whether a Post*Item result means the item really went through. Skips,
/// "keep the existing one" and failures report other codes.
fn item_done(hr: HRESULT) -> bool {
    // COPYENGINE_S_* (winerror.h, FACILITY_SHELL 0x27)
    const DONT_PROCESS_CHILDREN: i32 = 0x0027_0008; // folder handled as one unit
    const MERGE: i32 = 0x0027_0006; // merged into an existing folder
    const KEEP_BOTH: i32 = 0x0027_000C;
    const COLLISION_RESOLVED: i32 = 0x0027_000E;
    hr == S_OK
        || matches!(
            hr.0,
            DONT_PROCESS_CHILDREN | MERGE | KEEP_BOTH | COLLISION_RESOLVED
        )
}

/// Collects what an IFileOperation actually did (IFileOperationProgressSink).
#[implement(IFileOperationProgressSink)]
struct OpSink {
    /// (source, result, sure): `sure` false for a delete the shell answered
    /// with another success code; it counts only if the source is gone after.
    done: std::rc::Rc<RefCell<Vec<(PathBuf, Option<PathBuf>, bool)>>>,
}

impl OpSink {
    fn record(
        &self,
        src: Ref<'_, IShellItem>,
        hr: HRESULT,
        new: Ref<'_, IShellItem>,
        need_new: bool,
    ) {
        let src = src.as_ref().and_then(fs_path);
        let dst = new.as_ref().and_then(fs_path);
        #[cfg(test)]
        println!("  sink: hr={:#010x} src={src:?} new={dst:?}", hr.0 as u32);
        // a folder delete reports COPYENGINE_S_NOT_HANDLED for the folder
        // itself before deleting its contents
        let sure = item_done(hr);
        if !sure && (need_new || hr.is_err()) {
            return;
        }
        let Some(src) = src else {
            return;
        };
        if need_new && dst.is_none() {
            return;
        }
        self.done.borrow_mut().push((src, dst, sure));
    }
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for OpSink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _: HRESULT) -> windows::core::Result<()> {
        Ok(())
    }
    fn PreRenameItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostRenameItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: &PCWSTR,
        hr: HRESULT,
        new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(item, hr, new, true);
        Ok(())
    }
    fn PreMoveItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        hr: HRESULT,
        new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(item, hr, new, true);
        Ok(())
    }
    fn PreCopyItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        hr: HRESULT,
        new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(item, hr, new, true);
        Ok(())
    }
    fn PreDeleteItem(&self, _: u32, _: Ref<'_, IShellItem>) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        item: Ref<'_, IShellItem>,
        hr: HRESULT,
        new: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(item, hr, new, false);
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Ref<'_, IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        _: HRESULT,
        _: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Runs one IFileOperation over `paths`; no owner window, so its progress and
/// conflict dialogs belong to the calling (worker) thread, as in the
/// original Grids. Returns the top-level items done (children of a folder
/// are reported by the shell too; they are dropped here).
fn file_op(
    paths: &[PathBuf],
    flags: FILEOPERATION_FLAGS,
    add: impl FnOnce(&IFileOperation) -> io::Result<()>,
) -> io::Result<Vec<OpItem>> {
    ensure_com();
    let is_dir: Vec<bool> = paths.iter().map(|p| p.is_dir()).collect();
    let long: Vec<PathBuf> = paths.iter().map(|p| long_path(p)).collect();
    let log = std::rc::Rc::new(RefCell::new(Vec::new()));
    let sink: IFileOperationProgressSink = OpSink { done: log.clone() }.into();
    let (done, aborted) = unsafe {
        let op: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(win_err)?;
        op.SetOperationFlags(flags).map_err(win_err)?;
        let cookie = op.Advise(&sink).map_err(win_err)?;
        let added = add(&op);
        let done = added.and_then(|()| op.PerformOperations().map_err(win_err));
        let _ = op.Unadvise(cookie);
        (
            done,
            op.GetAnyOperationsAborted().is_ok_and(|b| b.as_bool()),
        )
    };
    let mut items = Vec::new();
    drop(sink);
    let collected = log.take();
    let mut seen = vec![false; paths.len()];
    for (src, dst, sure) in collected {
        let Some(i) = long.iter().position(|p| crate::same_path(p, &src)) else {
            continue;
        };
        if !sure && std::fs::symlink_metadata(&src).is_ok() {
            continue;
        }
        match seen.get_mut(i) {
            Some(s) if !*s => *s = true,
            _ => continue,
        }
        if let Some(orig) = paths.get(i) {
            items.push(OpItem {
                src: orig.clone(),
                dst,
                is_dir: is_dir.get(i).copied().unwrap_or(false),
            });
        }
    }
    if items.is_empty() {
        if aborted {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        done?;
    }
    Ok(items)
}

pub fn recycle(paths: &[PathBuf], confirmed: bool) -> io::Result<Vec<OpItem>> {
    recycle_with(paths, confirmed_flags(confirmed))
}

pub fn delete(paths: &[PathBuf], confirmed: bool) -> io::Result<Vec<OpItem>> {
    delete_with(paths, confirmed_flags(confirmed))
}

/// After Grids' own confirmation the shell asks nothing more (conflict and
/// progress UI stay); `recycle_with` then also drops the nuke warning.
fn confirmed_flags(confirmed: bool) -> FILEOPERATION_FLAGS {
    if confirmed {
        FOF_NOCONFIRMATION
    } else {
        FILEOPERATION_FLAGS(0)
    }
}

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_CDROM: u32 = 5;
const DRIVE_RAMDISK: u32 = 6;

/// Best effort: whether recycling `paths` would delete permanently because
/// their drive has no Recycle Bin (network, removable, optical, RAM disks,
/// or a volume whose bin can't be queried). Items too large for the bin are
/// not detected.
pub fn recycle_would_nuke(paths: &[PathBuf]) -> bool {
    ensure_com();
    let mut roots: Vec<String> = Vec::new();
    for p in paths {
        let Some(root) = volume_of(p) else {
            return true;
        };
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    let mut old = THREAD_ERROR_MODE(0);
    let _ = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut old)) };
    let nuke = roots.iter().any(|root| {
        let w = HSTRING::from(root.as_str());
        let kind = unsafe { GetDriveTypeW(&w) };
        if root.starts_with("\\\\")
            || matches!(
                kind,
                DRIVE_REMOTE | DRIVE_REMOVABLE | DRIVE_CDROM | DRIVE_RAMDISK
            )
        {
            return true;
        }
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            ..Default::default()
        };
        unsafe { SHQueryRecycleBinW(&w, &mut info) }.is_err()
    });
    let _ = unsafe { SetThreadErrorMode(old, None) };
    nuke
}

pub fn copy(paths: &[PathBuf], dest: &Path) -> io::Result<Vec<OpItem>> {
    copy_with(paths, dest, FILEOPERATION_FLAGS(0))
}

pub fn move_(paths: &[PathBuf], dest: &Path) -> io::Result<Vec<OpItem>> {
    move_with(paths, dest, FILEOPERATION_FLAGS(0))
}

// `extra`: FOF_NO_UI in the tests, so no dialog can wait for a click on the CI runner.

fn recycle_with(paths: &[PathBuf], extra: FILEOPERATION_FLAGS) -> io::Result<Vec<OpItem>> {
    let arr = items(paths)?;
    // confirmed in Grids (which warned about permanent deletion itself): no nuke prompt
    let nuke_warning = if extra.0 & FOF_NOCONFIRMATION.0 != 0 {
        FILEOPERATION_FLAGS(0)
    } else {
        FOF_WANTNUKEWARNING
    };
    let flags = FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | nuke_warning | FOFX_ADDUNDORECORD | extra;
    file_op(paths, flags, |op| {
        unsafe { op.DeleteItems(&arr) }.map_err(win_err)
    })
}

fn delete_with(paths: &[PathBuf], extra: FILEOPERATION_FLAGS) -> io::Result<Vec<OpItem>> {
    let arr = items(paths)?;
    let mut done = file_op(paths, extra, |op| {
        unsafe { op.DeleteItems(&arr) }.map_err(win_err)
    })?;
    for it in &mut done {
        it.dst = None;
    }
    Ok(done)
}

fn copy_with(
    paths: &[PathBuf],
    dest: &Path,
    extra: FILEOPERATION_FLAGS,
) -> io::Result<Vec<OpItem>> {
    // into the folder they already sit in: Explorer's "名称 - 副本" names; else its conflict dialog
    let same = paths.iter().all(|p| parent_is(p, dest));
    let mut flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD | extra;
    if same {
        flags |= FOF_RENAMEONCOLLISION;
    }
    let (arr, to) = (items(paths)?, item(dest)?);
    file_op(paths, flags, |op| {
        unsafe { op.CopyItems(&arr, &to) }.map_err(win_err)
    })
}

fn move_with(
    paths: &[PathBuf],
    dest: &Path,
    extra: FILEOPERATION_FLAGS,
) -> io::Result<Vec<OpItem>> {
    let (arr, to) = (items(paths)?, item(dest)?);
    file_op(
        paths,
        FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD | extra,
        |op| unsafe { op.MoveItems(&arr, &to) }.map_err(win_err),
    )
}

// ---------- recycle bin ----------

const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// Original path and deletion time from a `$I…` file (format 1: Vista-8.1,
/// fixed 260-char path; format 2: Windows 10+, length-prefixed path).
fn parse_recycle_info(b: &[u8]) -> Option<(PathBuf, std::time::SystemTime)> {
    let u64_at =
        |i: usize| -> Option<u64> { Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?)) };
    let version = u64_at(0)?;
    let filetime = u64_at(16)?;
    let units: Vec<u16> = match version {
        1 => b.get(24..24 + 520)?,
        2 => {
            let n = u32::from_le_bytes(b.get(24..28)?.try_into().ok()?) as usize;
            b.get(28..28 + n.checked_mul(2)?)?
        }
        _ => return None,
    }
    .as_chunks::<2>()
    .0
    .iter()
    .map(|c| u16::from_le_bytes(*c))
    .collect();
    let path = from_wide(&units);
    if path.is_empty() {
        return None;
    }
    let when = std::time::UNIX_EPOCH
        + std::time::Duration::from_nanos(
            filetime
                .checked_sub(FILETIME_UNIX_EPOCH)?
                .checked_mul(100)?,
        );
    Some((PathBuf::from(path), when))
}

/// `$Rxxxxxx.ext` -> its `$Ixxxxxx.ext` sibling.
fn recycle_info_path(r: &Path) -> Option<PathBuf> {
    let name = r.file_name()?.to_str()?;
    let rest = name.strip_prefix("$R")?;
    Some(r.with_file_name(format!("$I{rest}")))
}

/// Every entry in the Recycle Bin (all drives) as (item, original, deleted).
fn recycle_bin_entries() -> io::Result<Vec<(IShellItem, PathBuf, std::time::SystemTime)>> {
    ensure_com();
    let mut out = Vec::new();
    unsafe {
        let bin: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)
                .map_err(win_err)?;
        let en: IEnumShellItems = bin.BindToHandler(None, &BHID_EnumItems).map_err(win_err)?;
        loop {
            let mut got = [None];
            let mut n = 0u32;
            if en.Next(&mut got, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let [Some(it)] = got else { break };
            let Some(info) = fs_path(&it).as_deref().and_then(recycle_info_path) else {
                continue;
            };
            if let Some((orig, when)) = std::fs::read(&info)
                .ok()
                .and_then(|b| parse_recycle_info(&b))
            {
                out.push((it, orig, when));
            }
        }
    }
    Ok(out)
}

pub fn restore_from_recycle_bin(
    original: &Path,
    near: std::time::SystemTime,
) -> io::Result<PathBuf> {
    let entries = recycle_bin_entries()?;
    let Some(it) = crate::nearest_deleted(entries, &long_path(original), near) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: not in the Recycle Bin", original.display()),
        ));
    };
    let r_path = fs_path(&it);
    unsafe {
        let cm: IContextMenu = it.BindToHandler(None, &BHID_SFUIObject).map_err(win_err)?;
        let menu = CreatePopupMenu().map_err(win_err)?;
        let _ = cm.QueryContextMenu(menu, 0, FIRST_CMD, LAST_CMD, CMF_NORMAL);
        let info = CMINVOKECOMMANDINFO {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFO>() as u32,
            fMask: CMIC_MASK_FLAG_NO_UI,
            lpVerb: PCSTR(c"undelete".as_ptr().cast()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        let invoked = cm.InvokeCommand(&info);
        let _ = DestroyMenu(menu);
        // the verb may finish on another thread: wait a little for the file
        for _ in 0..50 {
            if std::fs::symlink_metadata(original).is_ok() {
                return Ok(original.to_path_buf());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        invoked.map_err(win_err)?;
    }
    // the verb ran but nothing came back: put the $R item back by hand
    let r = r_path.ok_or_else(|| io::Error::other("Recycle Bin item has no path"))?;
    std::fs::rename(&r, original)?;
    if let Some(i) = recycle_info_path(&r) {
        let _ = std::fs::remove_file(i);
    }
    let w = wide(original);
    unsafe { SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(w.as_ptr() as _), None) };
    Ok(original.to_path_buf())
}

// ---------- who holds a file (Restart Manager) ----------

/// Ends a Restart Manager session on drop.
struct RmSession(u32);

impl Drop for RmSession {
    fn drop(&mut self) {
        unsafe {
            let _ = RmEndSession(self.0);
        }
    }
}

fn rm_err(code: WIN32_ERROR) -> io::Error {
    io::Error::from_raw_os_error(code.0 as i32)
}

pub fn lockers(files: &[PathBuf]) -> io::Result<Vec<crate::Locker>> {
    let mut handle = 0u32;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    let rc = unsafe { RmStartSession(&mut handle, None, PWSTR(key.as_mut_ptr())) };
    if rc != ERROR_SUCCESS {
        return Err(rm_err(rc));
    }
    let session = RmSession(handle);
    let names: Vec<HSTRING> = files.iter().map(|f| wide(f)).collect();
    // RmRegisterResources copies the names; register in batches to keep each call small
    for chunk in names.chunks(256) {
        let ptrs: Vec<PCWSTR> = chunk.iter().map(|h| PCWSTR(h.as_ptr())).collect();
        let rc = unsafe { RmRegisterResources(session.0, Some(&ptrs), None, None) };
        if rc != ERROR_SUCCESS {
            return Err(rm_err(rc));
        }
    }
    let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
    for _ in 0..5 {
        let (mut needed, mut count, mut reasons) = (0u32, infos.len() as u32, 0u32);
        let ptr = if infos.is_empty() {
            None
        } else {
            Some(infos.as_mut_ptr())
        };
        let rc = unsafe { RmGetList(session.0, &mut needed, &mut count, ptr, &mut reasons) };
        if rc == ERROR_SUCCESS {
            infos.truncate(count as usize);
            break;
        }
        if rc != ERROR_MORE_DATA {
            return Err(rm_err(rc));
        }
        // the list can grow between calls: retry with room to spare
        infos = vec![RM_PROCESS_INFO::default(); needed as usize + 4];
    }
    drop(session);
    let windows = top_windows();
    Ok(infos
        .iter()
        .map(|i| {
            let pid = i.Process.dwProcessId;
            // RM_APP_TYPE values (RmMainWindow = 1 ... RmCritical = 1000)
            let kind = match i.ApplicationType.0 {
                1 => crate::AppKind::Window,
                2 => crate::AppKind::OtherWindow,
                3 => crate::AppKind::Service,
                4 => crate::AppKind::Explorer,
                5 => crate::AppKind::Console,
                1000 => crate::AppKind::Critical,
                _ => crate::AppKind::Unknown,
            };
            let (exe, can_terminate) = process_info(pid);
            let mut name = from_wide(&i.strAppName);
            if name.is_empty() {
                name = exe
                    .as_ref()
                    .and_then(|e| e.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("PID {pid}"));
            }
            let service = Some(from_wide(&i.strServiceShortName)).filter(|s| !s.is_empty());
            crate::Locker {
                pid,
                name,
                kind,
                hwnd: main_window(&windows, pid).map(|h| h.0 as isize),
                exe,
                service,
                can_terminate,
            }
        })
        .collect())
}

/// Closes a process handle on drop.
struct ProcHandle(HANDLE);

impl Drop for ProcHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// (program path, may terminate) for `pid`.
fn process_info(pid: u32) -> (Option<PathBuf>, bool) {
    let exe = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .ok()
        .map(ProcHandle)
        .and_then(|h| {
            let mut buf = vec![0u16; 1024];
            let mut n = buf.len() as u32;
            unsafe {
                QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n)
            }
            .ok()?;
            Some(PathBuf::from(String::from_utf16_lossy(
                buf.get(..n as usize)?,
            )))
        });
    let can_terminate = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }
        .ok()
        .map(ProcHandle)
        .is_some();
    (exe, can_terminate)
}

/// Visible, unowned top-level windows as (hwnd, pid, has title).
fn top_windows() -> Vec<(HWND, u32, bool)> {
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        // SAFETY: lp is the &mut Vec passed to EnumWindows below, alive for the call
        let out = unsafe { &mut *(lp.0 as *mut Vec<(HWND, u32, bool)>) };
        unsafe {
            if IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).is_err() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                out.push((hwnd, pid, GetWindowTextLengthW(hwnd) > 0));
            }
        }
        BOOL(1)
    }
    let mut out: Vec<(HWND, u32, bool)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// The window to show for `pid`: the first titled one, else any.
fn main_window(windows: &[(HWND, u32, bool)], pid: u32) -> Option<HWND> {
    let mine = || windows.iter().filter(move |w| w.1 == pid);
    mine().find(|w| w.2).or_else(|| mine().next()).map(|w| w.0)
}

fn no_window(pid: u32) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("process {pid} has no window"),
    )
}

pub fn activate_process_window(pid: u32) -> io::Result<()> {
    let hwnd = main_window(&top_windows(), pid).ok_or_else(|| no_window(pid))?;
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if !SetForegroundWindow(hwnd).as_bool() {
            return Err(io::Error::other(format!(
                "Windows did not let process {pid} come to the front"
            )));
        }
    }
    Ok(())
}

pub fn close_process(pid: u32) -> io::Result<()> {
    let windows: Vec<HWND> = top_windows()
        .into_iter()
        .filter(|w| w.1 == pid)
        .map(|w| w.0)
        .collect();
    if windows.is_empty() {
        return Err(no_window(pid));
    }
    for hwnd in windows {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    Ok(())
}

pub fn terminate_process(pid: u32) -> io::Result<()> {
    let h = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }.map_err(|e| {
        if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("no rights to end process {pid}"),
            )
        } else if e.code() == ERROR_INVALID_PARAMETER.to_hresult() {
            io::Error::new(io::ErrorKind::NotFound, format!("no process {pid}"))
        } else {
            win_err(e)
        }
    })?;
    let h = ProcHandle(h);
    unsafe { TerminateProcess(h.0, 1) }.map_err(|e| {
        if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("no rights to end process {pid}"),
            )
        } else {
            win_err(e)
        }
    })
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

/// One thread of Grids at a time: with no owner window, a second thread's
/// OpenClipboard can succeed too, and its EmptyClipboard would free the
/// memory the first one is reading.
static CLIPBOARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Opens the clipboard, closed again when the guard drops. The clipboard is
/// shared: another program may hold it for a moment.
fn open_clipboard() -> io::Result<ClipboardGuard> {
    let lock = CLIPBOARD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(ClipboardGuard(lock));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Err(io::Error::other(
        "the clipboard is in use by another program",
    ))
}

struct ClipboardGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

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
    let _guard = open_clipboard()?;
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
        let _guard = open_clipboard().ok()?;
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

/// The bytes of clipboard format `fmt` (the clipboard must be open).
unsafe fn clipboard_bytes(fmt: u32) -> Option<Vec<u8>> {
    unsafe {
        let h = GetClipboardData(fmt).ok()?;
        let g = HGLOBAL(h.0);
        let size = GlobalSize(g);
        let p = GlobalLock(g) as *const u8;
        if p.is_null() {
            return None;
        }
        let out = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(g);
        Some(out)
    }
}

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_DIBV5: u32 = 17;

pub fn clipboard_image() -> Option<RgbaImage> {
    unsafe {
        let png = RegisterClipboardFormatW(w!("PNG"));
        let fmts = [png, CF_DIBV5, CF_DIB];
        if !fmts
            .iter()
            .any(|&f| f != 0 && IsClipboardFormatAvailable(f).is_ok())
        {
            return None;
        }
        let _guard = open_clipboard().ok()?;
        if png != 0
            && let Some(img) = clipboard_bytes(png)
                .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
        {
            return Some(img.into_rgba8());
        }
        [CF_DIBV5, CF_DIB]
            .into_iter()
            .find_map(|f| clipboard_bytes(f).and_then(|b| crate::decode_dib(&b)))
    }
}

pub fn clipboard_text() -> Option<String> {
    unsafe {
        IsClipboardFormatAvailable(CF_UNICODETEXT).ok()?;
        let _guard = open_clipboard().ok()?;
        let b = clipboard_bytes(CF_UNICODETEXT)?;
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Some(from_wide(&units)).filter(|t| !t.is_empty())
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

// ---------- thumbnails ----------

type ThumbJob = (PathBuf, u32, std::sync::mpsc::Sender<Option<RgbaImage>>);

/// How long a caller waits for one thumbnail (an offline share or a codec
/// chewing on a huge video must not hold the app's worker forever).
const THUMB_WAIT: std::time::Duration = std::time::Duration::from_secs(8);
const THUMB_WORKERS: usize = 4;

/// A few STA threads that own all IShellItemImageFactory calls. A call stuck
/// in a thumbnail handler only ties up its worker; the caller gives up after
/// THUMB_WAIT.
fn thumb_queue() -> Option<&'static Mutex<std::sync::mpsc::Sender<ThumbJob>>> {
    static QUEUE: OnceLock<Option<Mutex<std::sync::mpsc::Sender<ThumbJob>>>> = OnceLock::new();
    QUEUE
        .get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<ThumbJob>();
            let rx = Arc::new(Mutex::new(rx));
            let mut started = 0;
            for i in 0..THUMB_WORKERS {
                let rx = rx.clone();
                let ok = std::thread::Builder::new()
                    .name(format!("grids-thumb-{i}"))
                    .spawn(move || {
                        ensure_com();
                        loop {
                            let job = match rx.lock() {
                                Ok(r) => r.recv(),
                                Err(_) => return,
                            };
                            let Ok((path, size, reply)) = job else { return };
                            // the caller may have given up already: nothing to do then
                            let _ = reply.send(thumbnail_now(&path, size));
                        }
                    })
                    .is_ok();
                started += usize::from(ok);
            }
            (started > 0).then(|| Mutex::new(tx))
        })
        .as_ref()
}

pub fn thumbnail(path: &Path, size: u32) -> Option<RgbaImage> {
    let size = size.clamp(16, 1024);
    let (reply, answer) = std::sync::mpsc::channel();
    thumb_queue()?
        .lock()
        .ok()?
        .send((path.to_path_buf(), size, reply))
        .ok()?;
    answer.recv_timeout(THUMB_WAIT).ok().flatten()
}

fn thumbnail_now(path: &Path, size: u32) -> Option<RgbaImage> {
    ensure_com();
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&wide(path), None) }.ok()?;
    let want = SIZE {
        cx: size as i32,
        cy: size as i32,
    };
    // a real preview when a thumbnail handler has one, else the type's icon at this size
    let bmp = unsafe { factory.GetImage(want, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK) }
        .or_else(|_| unsafe { factory.GetImage(want, SIIGBF_ICONONLY) })
        .ok()?;
    let img = unsafe { hbitmap_to_rgba(bmp) };
    let _ = unsafe { DeleteObject(HGDIOBJ(bmp.0)) };
    let img = img?;
    Some(crate::fit_within(img, size))
}

/// A 32-bit shell bitmap as straight-alpha RGBA, top row first. Shell
/// thumbnails are premultiplied (PARGB) when they carry alpha; photos often
/// come with alpha 0 everywhere, which means opaque.
unsafe fn hbitmap_to_rgba(bmp: HBITMAP) -> Option<RgbaImage> {
    let mut bm = BITMAP::default();
    if unsafe {
        GetObjectW(
            HGDIOBJ(bmp.0),
            size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as _),
        )
    } == 0
    {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
        return None;
    }
    let dc = unsafe { CreateCompatibleDC(None) };
    // negative height in bitmap_bgra: rows come back top-down whatever the source order
    let px = unsafe { bitmap_bgra(dc, bmp, w, h) };
    let _ = unsafe { DeleteDC(dc) };
    let mut px = px?;
    let chunks = px.as_chunks_mut::<4>().0;
    let no_alpha = chunks.iter().all(|p| p[3] == 0);
    for p in chunks.iter_mut() {
        p.swap(0, 2); // BGRA -> RGBA
        if no_alpha {
            p[3] = 255;
        } else if p[3] != 0 && p[3] != 255 {
            let a = u32::from(p[3]);
            for c in p.iter_mut().take(3) {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(w as u32, h as u32, px)
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
    fn tracked_ops_report_real_paths() {
        use windows::Win32::UI::Shell::FOF_NO_UI;
        let d = std::env::temp_dir().join(format!("grids_win_tracked_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("to")).unwrap();
        std::fs::create_dir_all(d.join("dir")).unwrap();
        std::fs::write(d.join("dir").join("inner.txt"), "i").unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        // copy into its own folder: the shell renames; dst must be the real copy
        let done = copy_with(&[d.join("a.txt")], &d, FOF_NO_UI).unwrap();
        println!("copy conflict -> {done:?}");
        assert_eq!(done.len(), 1);
        let dst = done[0].dst.clone().unwrap();
        assert_eq!(done[0].src, d.join("a.txt"));
        assert!(
            !crate::same_path(&dst, &long_path(&d.join("a.txt"))),
            "{dst:?}"
        );
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "a");
        // a folder is one item, its children are not reported
        let done = copy_with(&[d.join("dir"), d.join("a.txt")], &d.join("to"), FOF_NO_UI).unwrap();
        println!("copy folder -> {done:?}");
        assert_eq!(done.len(), 2);
        assert!(done[0].is_dir && !done[1].is_dir);
        assert!(done[0].dst.as_ref().unwrap().join("inner.txt").is_file());
        // move
        let done = move_with(&[d.join("to").join("a.txt")], &d.join("dir"), FOF_NO_UI).unwrap();
        println!("move -> {done:?}");
        assert!(
            !crate::recycle_would_nuke(&[d.join("a.txt")]),
            "temp dir has a Recycle Bin"
        );
        assert!(!crate::recycle_would_nuke(&[]));
        assert_eq!(done.len(), 1, "move reported nothing");
        assert!(crate::same_path(
            done[0].dst.as_ref().unwrap(),
            &long_path(&d.join("dir").join("a.txt"))
        ));
        // recycle, then put it back from the Recycle Bin
        let victim = d.join("victim.txt");
        std::fs::write(&victim, "precious").unwrap();
        let when = std::time::SystemTime::now();
        let done = recycle_with(std::slice::from_ref(&victim), FOF_NO_UI).unwrap();
        println!("recycle -> {done:?}");
        assert_eq!(done.len(), 1);
        assert!(!victim.exists());
        let back = crate::restore_from_recycle_bin(&victim, when).unwrap();
        assert_eq!(std::fs::read_to_string(&back).unwrap(), "precious");
        assert!(matches!(
            crate::restore_from_recycle_bin(&victim, when)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        ));
        // permanent delete: dst None
        let done = delete_with(&[d.join("to")], FOF_NO_UI).unwrap();
        assert_eq!(
            done,
            vec![OpItem {
                src: d.join("to"),
                dst: None,
                is_dir: true
            }]
        );
        // nothing to restore
        let gone = d.join("never-recycled.txt");
        assert!(matches!(
            crate::restore_from_recycle_bin(&gone, when)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recycle_info_formats() {
        let path: Vec<u16> = r"C:\x\y.txt".encode_utf16().chain([0]).collect();
        let ft: u64 = FILETIME_UNIX_EPOCH + 1_700_000_000 * 10_000_000;
        let mut v2 = Vec::new();
        v2.extend(2u64.to_le_bytes());
        v2.extend(5u64.to_le_bytes());
        v2.extend(ft.to_le_bytes());
        v2.extend((path.len() as u32).to_le_bytes());
        path.iter().for_each(|c| v2.extend(c.to_le_bytes()));
        let want = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(
            parse_recycle_info(&v2),
            Some((PathBuf::from(r"C:\x\y.txt"), want))
        );
        let mut v1 = Vec::new();
        v1.extend(1u64.to_le_bytes());
        v1.extend(5u64.to_le_bytes());
        v1.extend(ft.to_le_bytes());
        let mut fixed = path.clone();
        fixed.resize(260, 0);
        fixed.iter().for_each(|c| v1.extend(c.to_le_bytes()));
        assert_eq!(
            parse_recycle_info(&v1),
            Some((PathBuf::from(r"C:\x\y.txt"), want))
        );
        assert_eq!(parse_recycle_info(&v2[..20]), None);
        assert_eq!(
            recycle_info_path(Path::new(r"C:\$Recycle.Bin\S-1\$RAB12.txt")),
            Some(PathBuf::from(r"C:\$Recycle.Bin\S-1\$IAB12.txt"))
        );
    }

    #[test]
    fn processes_close_and_terminate() {
        use std::os::windows::process::CommandExt;
        // a windowless console child: no window to close or show, but it can be ended
        let mut child = std::process::Command::new("PING.EXE")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .unwrap();
        let pid = child.id();
        let (exe, can) = process_info(pid);
        println!("ping: exe {exe:?}, can_terminate {can}");
        assert!(can);
        assert!(
            exe.unwrap()
                .to_string_lossy()
                .to_lowercase()
                .ends_with("ping.exe")
        );
        assert_eq!(
            crate::close_process(pid).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            crate::activate_process_window(pid).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        crate::terminate_process(pid).unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(1));
        // the System process: no rights (or, unelevated, not even visible)
        let e = crate::terminate_process(4).unwrap_err();
        println!("terminate System -> {e}");
        assert!(matches!(
            e.kind(),
            io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound
        ));
        assert!(!process_info(4).1);
        // top-level windows enumerate without trouble
        println!("visible top-level windows: {}", top_windows().len());
    }

    /// Puts `data` on the clipboard as format `fmt` (test helper).
    fn put_clipboard(items: &[(u32, &[u8])]) -> bool {
        unsafe {
            let Ok(_guard) = open_clipboard() else {
                return false;
            };
            let _ = EmptyClipboard();
            for (fmt, data) in items {
                let g = GlobalAlloc(GMEM_MOVEABLE, data.len()).unwrap();
                let p = GlobalLock(g) as *mut u8;
                std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
                let _ = GlobalUnlock(g);
                SetClipboardData(*fmt, Some(HANDLE(g.0))).unwrap();
            }
            true
        }
    }

    /// The clipboard tests set and read back the one system clipboard.
    static CLIPBOARD_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn clipboard_image_and_text() {
        let _one = CLIPBOARD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        // a 2x1 top-down 32-bpp DIB with no alpha: red, green
        let mut dib = Vec::new();
        dib.extend(40u32.to_le_bytes());
        dib.extend(2i32.to_le_bytes());
        dib.extend((-1i32).to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend(32u16.to_le_bytes());
        dib.resize(40, 0);
        dib.extend([0, 0, 255, 0, 0, 255, 0, 0]);
        if !put_clipboard(&[(CF_DIB, &dib)]) {
            println!("clipboard busy on this runner: skipped");
            return;
        }
        let img = crate::clipboard_image().expect("DIB on the clipboard");
        assert_eq!(img.dimensions(), (2, 1));
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0, 255]);
        assert_eq!(crate::clipboard_text(), None);
        // PNG wins over the DIB
        let mut png = Vec::new();
        let src = RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 100]));
        src.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let png_fmt = unsafe { RegisterClipboardFormatW(w!("PNG")) };
        assert!(put_clipboard(&[(CF_DIB, &dib), (png_fmt, &png)]));
        assert_eq!(crate::clipboard_image().unwrap(), src);
        // text
        let text: Vec<u8> = "截图 hello\r\n"
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(put_clipboard(&[(CF_UNICODETEXT, &text)]));
        assert_eq!(crate::clipboard_text().as_deref(), Some("截图 hello\r\n"));
        assert_eq!(crate::clipboard_image(), None);
        assert!(put_clipboard(&[]));
        assert_eq!(crate::clipboard_text(), None);
    }

    #[test]
    fn clipboard_round_trip() {
        let _one = CLIPBOARD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
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

    #[test]
    fn thumbnails() {
        let d = std::env::temp_dir().join(format!("grids_win_thumb_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // 400x200 gradient, written as PNG; the JPEG is made by Windows itself (WIC via PowerShell)
        let img = RgbaImage::from_fn(400, 200, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        let png = d.join("图 片 1.png");
        img.save(&png).unwrap();
        let jpg = d.join("photo.jpg");
        let st = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Add-Type -AssemblyName System.Drawing; \
                 $i = [System.Drawing.Image]::FromFile($env:SRC); \
                 $i.Save($env:DST, [System.Drawing.Imaging.ImageFormat]::Jpeg); $i.Dispose()",
            ])
            .env("SRC", &png)
            .env("DST", &jpg)
            .status()
            .unwrap();
        assert!(st.success() && jpg.is_file());
        let txt = d.join("a.txt");
        std::fs::write(&txt, "x").unwrap();

        for p in [&png, &jpg] {
            let t = thumbnail(p, 128).unwrap_or_else(|| panic!("no thumbnail for {p:?}"));
            println!("{}: {}x{}", p.display(), t.width(), t.height());
            assert_eq!(t.width().max(t.height()), 128, "{:?}", t.dimensions());
            assert!(t.width() > t.height(), "aspect lost: {:?}", t.dimensions());
            assert!(t.pixels().any(|p| p[3] > 0), "fully transparent");
            // the gradient survives: not a generic icon
            let (l, r) = (
                t.get_pixel(2, t.height() / 2),
                t.get_pixel(60, t.height() / 2),
            );
            assert!(r[0] > l[0], "no gradient: {l:?} {r:?}");
        }
        // the 256 step must be the jumbo icon, not a 32 px one blown up: count distinct 2x2 blocks
        let big = thumbnail(&txt, 256).expect("jumbo icon for .txt");
        let mut blocky = 0usize;
        let mut total = 0usize;
        for y in (0..big.height().saturating_sub(1)).step_by(2) {
            for x in (0..big.width().saturating_sub(1)).step_by(2) {
                let q = [
                    big.get_pixel(x, y),
                    big.get_pixel(x + 1, y),
                    big.get_pixel(x, y + 1),
                    big.get_pixel(x + 1, y + 1),
                ];
                if q[0][3] > 0 {
                    total += 1;
                    blocky += usize::from(q.iter().all(|p| p == &q[0]));
                }
            }
        }
        println!(
            "txt 256: {}x{}, {blocky}/{total} flat 2x2 blocks",
            big.width(),
            big.height()
        );
        assert_eq!(big.width().max(big.height()), 256);
        let t = thumbnail(&txt, 64).expect("icon fallback for .txt");
        println!("txt: {}x{}", t.width(), t.height());
        assert!(t.width() <= 64 && t.height() <= 64 && t.width() >= 16);
        assert!(t.pixels().any(|p| p[3] > 0));
        assert!(thumbnail(&d.join("missing.png"), 64).is_none());

        // many at once from several threads: no panic, all answered
        let workers: Vec<_> = (0..8)
            .map(|k| {
                let (png, txt) = (png.clone(), txt.clone());
                std::thread::spawn(move || {
                    let p = if k % 2 == 0 { png } else { txt };
                    thumbnail(&p, [32, 96, 256, 48][k % 4]).is_some()
                })
            })
            .collect();
        for w in workers {
            assert!(w.join().expect("thumbnail panicked"));
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
