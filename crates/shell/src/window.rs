//! Window-level features the floating pane needs: always-on-top, addressed by
//! HWND. Ignored off Windows.

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

