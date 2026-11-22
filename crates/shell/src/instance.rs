//! One Grids at a time: a second launch asks the running one to come forward
//! and then leaves, so the taskbar keeps a single button for the program.
//! Windows only; elsewhere every launch is its own instance.

/// True when this process is the one Grids that may run. A false answer means
/// another instance is already running: it has been asked to show itself, and
/// this launch should end.
#[cfg(windows)]
pub fn claim() -> bool {
    let name = wide("Grids.SingleInstance.v1");
    // SAFETY: a named mutex and a plain Win32 window lookup; every result is
    // checked. The handle is kept open (never closed) so the name lives as
    // long as this process does.
    let already = unsafe {
        let _mutex = CreateMutexW(None, false, PCWSTR(name.as_ptr()));
        GetLastError() == ERROR_ALREADY_EXISTS
    };
    if already {
        signal();
    }
    !already
}

/// Every launch is a new instance elsewhere.
#[cfg(not(windows))]
pub fn claim() -> bool {
    true
}

/// Waits until process `pid` has exited. The theme relaunch helper uses this:
/// the new copy starts only once the old one has let go of the single-instance
/// name and the config file.
#[cfg(windows)]
pub fn wait_for_exit(pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    // SAFETY: a handle to another process, waited on and closed; failures are
    // ignored (the caller only wants to know "the old one is gone").
    unsafe {
        if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(process, INFINITE);
            let _ = CloseHandle(process);
        }
    }
}

/// Nothing to wait for elsewhere.
#[cfg(not(windows))]
pub fn wait_for_exit(_pid: u32) {}

/// The window message a second launch posts to the running instance.
#[cfg(windows)]
const INSTANCE_MSG: u32 = WM_APP + 2;

/// Waits for the running instance's hidden window, then asks it to show.
#[cfg(windows)]
fn signal() {
    let class = wide("GridsInstanceWindow");
    // The first instance creates its window while starting: give it a moment.
    for _ in 0..30 {
        if let Ok(hwnd) = unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) } {
            unsafe {
                let _ = PostMessageW(Some(hwnd), INSTANCE_MSG, WPARAM(0), LPARAM(0));
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// The first instance: keeps a hidden window of its own, so a later launch can
/// ask it to come forward. `on_show` runs on that window's thread.
#[cfg(windows)]
pub fn watch(on_show: impl Fn() + Send + 'static) -> bool {
    std::thread::Builder::new()
        .name("grids-instance".into())
        .spawn(move || unsafe { run(Box::new(on_show)) })
        .is_ok()
}

/// Nothing to watch for elsewhere.
#[cfg(not(windows))]
pub fn watch(_on_show: impl Fn() + Send + 'static) -> bool {
    false
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// What the window procedure needs: the callback for a later launch.
#[cfg(windows)]
struct State {
    on_show: Box<dyn Fn() + Send>,
}

#[cfg(windows)]
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::System::Threading::CreateMutexW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GWLP_USERDATA,
    GetMessageW, GetWindowLongPtrW, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    SetWindowLongPtrW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROY,
    WM_NCCREATE, WNDCLASSW,
};
#[cfg(windows)]
use windows::core::PCWSTR;

#[cfg(windows)]
unsafe fn run(on_show: Box<dyn Fn() + Send>) {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = wide("GridsInstanceWindow");
        let wndclass = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wndclass);
        let state = Box::into_raw(Box::new(State { on_show }));
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
        if created.is_err() {
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
        match msg {
            INSTANCE_MSG => {
                ((*state).on_show)();
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, msg, wp, lp),
        }
    }
}
