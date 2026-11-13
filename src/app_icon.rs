//! The window's own icons (title bar, taskbar, Alt+Tab), loaded from the exe
//! resource at the window's DPI. GPUI's window class only carries one
//! default-sized icon, which Windows scales down blurrily for the small one.

/// Sets the big and small icons of `hwnd` from icon resource 1. Failures are
/// ignored: the class icon stays.
#[cfg(windows)]
pub fn apply(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON, SendMessageW, WM_SETICON};
    use windows::core::PCWSTR;
    if hwnd == 0 { return }
    let hwnd = HWND(hwnd as _);
    // SAFETY: plain Win32 calls on our own window and module; every result is checked.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let dpi = match GetDpiForWindow(hwnd) { 0 => 96, d => d };
        for (kind, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let size = GetSystemMetricsForDpi(metric, dpi);
            if let Ok(h) = LoadImageW(Some(module.into()), PCWSTR(1 as _), IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(kind as usize)), Some(LPARAM(h.0 as isize)));
            }
        }
    }
}

#[cfg(not(windows))]
pub fn apply(_hwnd: isize) {}
