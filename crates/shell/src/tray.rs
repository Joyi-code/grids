//! The notification-area icon: closing the main window hides it there
//! instead of ending the program. Windows only; elsewhere [`install`] says no
//! and the app keeps quitting when its window closes.

/// What the user picked in the notification area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Show the app's window again.
    Show,
    /// End the program.
    Quit,
}

/// Puts this app's icon in the notification area, on a thread that runs the
/// icon's own message loop. `on_event` is called on that thread. Returns false
/// when Windows would not take the icon (or off Windows), so the caller can
/// fall back to quitting when its window closes.
#[cfg(windows)]
pub fn install(
    tooltip: &str,
    show_label: &str,
    quit_label: &str,
    on_event: impl Fn(TrayEvent) + Send + 'static,
) -> bool {
    let (ready, took) = std::sync::mpsc::sync_channel(1);
    let (tooltip, show, quit) = (wide(tooltip), wide(show_label), wide(quit_label));
    let spawned = std::thread::Builder::new()
        .name("grids-tray".into())
        .spawn(move || unsafe { run(tooltip, show, quit, Box::new(on_event), ready) })
        .is_ok();
    if !spawned {
        return false;
    }
    took.recv_timeout(std::time::Duration::from_secs(2))
        .unwrap_or(false)
}

/// No notification area elsewhere.
#[cfg(not(windows))]
pub fn install(
    _tooltip: &str,
    _show_label: &str,
    _quit_label: &str,
    _on_event: impl Fn(TrayEvent) + Send + 'static,
) -> bool {
    false
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW, GetSystemMetrics,
    GetWindowLongPtrW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON,
    SetForegroundWindow, SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROY, WM_LBUTTONUP, WM_NCCREATE,
    WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
#[cfg(windows)]
use windows::core::PCWSTR;

/// The tray callback message (a `WM_APP` message of our own).
#[cfg(windows)]
const CALLBACK_MSG: u32 = WM_APP + 1;

/// `TaskbarCreated`: sent back to the icon's window when Explorer restarts, so
/// the icon can be added again.
#[cfg(windows)]
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// What the window procedure needs: the menu labels and the callback.
#[cfg(windows)]
struct State {
    on_event: Box<dyn Fn(TrayEvent) + Send>,
    show: Vec<u16>,
    quit: Vec<u16>,
    window: HWND,
    icon: NOTIFYICONDATAW,
}

/// Creates the hidden window, adds the icon, then runs its message loop. The
/// thread lives as long as the process does.
#[cfg(windows)]
unsafe fn run(
    tooltip: Vec<u16>,
    show: Vec<u16>,
    quit: Vec<u16>,
    on_event: Box<dyn Fn(TrayEvent) + Send>,
    ready: std::sync::mpsc::SyncSender<bool>,
) {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            let _ = ready.send(false);
            return;
        };
        let class = wide("GridsTrayWindow");
        let wndclass = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wndclass);
        let state = Box::into_raw(Box::new(State {
            on_event,
            show,
            quit,
            window: HWND::default(),
            icon: NOTIFYICONDATAW::default(),
        }));
        // `state` lives in the window's USERDATA until the process ends.
        let created = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            PCWSTR(class.as_ptr()),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            Some(state as *const core::ffi::c_void),
        );
        let Ok(window) = created else {
            let _ = ready.send(false);
            return;
        };
        (*state).window = window;
        let taskbar_created = wide("TaskbarCreated");
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(PCWSTR(taskbar_created.as_ptr())),
            Ordering::Relaxed,
        );
        let size = GetSystemMetrics(SM_CXSMICON).max(16);
        let mut icon = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: CALLBACK_MSG,
            ..Default::default()
        };
        // Resource 1 is the app icon, as on the windows themselves.
        if let Ok(handle) = LoadImageW(
            Some(module.into()),
            PCWSTR(1 as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        ) {
            icon.hIcon = HICON(handle.0);
            icon.uFlags = icon.uFlags | NIF_ICON;
        }
        for (slot, ch) in icon.szTip.iter_mut().zip(tooltip.iter().take(127)) {
            *slot = *ch;
        }
        (*state).icon = icon;
        let added = Shell_NotifyIconW(NIM_ADD, &(*state).icon).as_bool();
        let _ = ready.send(added);
        if !added {
            return;
        }
        let mut msg = MSG::default();
        loop {
            let got = GetMessageW(&mut msg, None, 0, 0);
            if got.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn wndproc(window: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
            return DefWindowProcW(window, msg, wp, lp);
        }
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(window, msg, wp, lp);
        }
        let state = &mut *state;
        let taskbar_created = TASKBAR_CREATED.load(Ordering::Relaxed);
        match msg {
            CALLBACK_MSG => {
                // The low word of lParam is the mouse message, as the shell sends it.
                match (lp.0 as u32) & 0xFFFF {
                    WM_LBUTTONUP => (state.on_event)(TrayEvent::Show),
                    WM_RBUTTONUP => menu(state),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = Shell_NotifyIconW(NIM_DELETE, &state.icon);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ if taskbar_created != 0 && msg == taskbar_created => {
                // Explorer restarted: its notification area lost our icon.
                let _ = Shell_NotifyIconW(NIM_ADD, &state.icon);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, msg, wp, lp),
        }
    }
}

/// The icon's context menu: show the window, or quit.
#[cfg(windows)]
unsafe fn menu(state: &mut State) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let _ = AppendMenuW(menu, MF_STRING, 1, PCWSTR(state.show.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, 2, PCWSTR(state.quit.as_ptr()));
        let mut at = POINT::default();
        if GetCursorPos(&mut at).is_ok() {
            // The window must be foreground, or the menu will not close when
            // the next click lands elsewhere.
            let _ = SetForegroundWindow(state.window);
            let picked = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                at.x,
                at.y,
                Some(0),
                state.window,
                None,
            )
            .0;
            let _ = PostMessageW(Some(state.window), WM_NULL, WPARAM(0), LPARAM(0));
            match picked {
                1 => (state.on_event)(TrayEvent::Show),
                2 => (state.on_event)(TrayEvent::Quit),
                _ => {}
            }
        }
        let _ = DestroyMenu(menu);
    }
}
