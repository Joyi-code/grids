//! Window-level features the floating pane and the notification-area icon
//! need: always-on-top and hiding / showing a window by HWND. Ignored off
//! Windows.

/// Pins `hwnd` above other windows, or releases it.
#[cfg(windows)]
pub fn set_always_on_top(hwnd: isize, on: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };
    if hwnd == 0 { return }
    // SAFETY: plain Win32 call on a window this process owns.
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as _),
            Some(if on { HWND_TOPMOST } else { HWND_NOTOPMOST }),
            0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
pub fn set_always_on_top(_hwnd: isize, _on: bool) {}

/// Shows `hwnd` again after [`hide`] (or restores it from minimised) and gives
/// it the foreground: a click on the notification-area icon, or a second
/// launch, is the user's intent to come back.
#[cfg(windows)]
pub fn show(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
    };
    if hwnd == 0 { return }
    let hwnd = HWND(hwnd as _);
    // SAFETY: plain Win32 calls on a window this process owns.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

#[cfg(not(windows))]
pub fn show(_hwnd: isize) {}

/// Hides `hwnd` without closing it: the program keeps running and its
/// notification-area icon brings the window back.
#[cfg(windows)]
pub fn hide(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
    if hwnd == 0 { return }
    // SAFETY: plain Win32 call on a window this process owns.
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
    }
}

#[cfg(not(windows))]
pub fn hide(_hwnd: isize) {}

