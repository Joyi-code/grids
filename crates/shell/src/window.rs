//! Window-level features the floating pane needs: always-on-top and
//! translucency, addressed by HWND. Both are ignored off Windows.

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

/// Sets the window's opacity to `percent` (clamped to 30..=100) through a
/// layered window, so the content stays clickable while it is translucent.
#[cfg(windows)]
pub fn set_alpha(hwnd: isize, percent: u8) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
        WS_EX_LAYERED,
    };
    if hwnd == 0 { return }
    let hwnd = HWND(hwnd as _);
    let alpha = (percent.clamp(30, 100) as u32 * 255 / 100) as u8;
    // SAFETY: plain Win32 calls on a window this process owns.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex & (WS_EX_LAYERED.0 as isize) == 0 {
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        }
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
    }
}

#[cfg(not(windows))]
pub fn set_alpha(_hwnd: isize, _percent: u8) {}

