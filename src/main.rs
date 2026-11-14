#![cfg_attr(windows, windows_subsystem = "windows")]
//! Grids: a multi-pane file manager drawn with GPUI.

mod app_icon;
mod assets;
mod input;
mod menu;
mod ops;
mod pane;
mod state;
mod theme;
mod thumbs;
mod ui;
mod workspace;

use gpui::*;
use grids_core::format::Lang;
use grids_core::Config;
use std::path::Path;

fn main() {
    install_crash_log();
    env_logger_lite();
    let path = Config::default_path();
    let mut config = Config::load(&path);
    ops::init(path.parent().map(Path::to_path_buf).unwrap_or_default());
    // Simplified Chinese by default; GRIDS_LANG=en switches to English.
    if let Some(l) = std::env::var_os("GRIDS_LANG") { config.lang = if l == "en" { Lang::En } else { Lang::Zh } }
    let (w, h) = config.window.unwrap_or((1360., 860.));
    // The product name the interface and the taskbar show, in its language.
    let brand = if config.lang == Lang::En { "PaneBoard" } else { "文件看板" };
    // A theme pack: GRIDS_THEME wins over the settings file.
    let theme_dir = std::env::var_os("GRIDS_THEME").map(std::path::PathBuf::from).or_else(|| config.theme.clone());
    if let Some(dir) = &theme_dir {
        match grids_core::theme_pack::load(dir) {
            Ok(pack) => theme::set_pack(pack),
            Err(e) => log::warn!("theme: {e}"),
        }
    }
    Application::new().with_assets(assets::Assets::new(theme_dir)).run(move |cx: &mut App| {
        input::bind_keys(cx);
        pane::bind_keys(cx);
        menu::bind_keys(cx);
        workspace::bind_keys(cx);
        cx.set_global(state::Shared::new(config, path));
        let bounds = Bounds::centered(None, size(px(w.max(640.)), px(h.max(420.))), cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("grids".into()),
                window_decorations: if cfg!(target_os = "linux") { Some(WindowDecorations::Client) } else { None },
                titlebar: Some(TitlebarOptions {
                    title: Some(brand.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(14.), px(ui::HEADER_H / 2. - 7.))),
                }),
                window_min_size: Some(size(px(640.), px(420.))),
                window_background: crate::theme::window_background(),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| workspace::Workspace::new(window, cx)),
        );
        if let Ok(handle) = window {
            let _ = handle.update(cx, |_, window, cx| {
                app_icon::apply(ui::hwnd(window));
                let ws = cx.entity();
                window.on_window_should_close(cx, move |window, cx| {
                    let b = window.bounds().size;
                    ws.update(cx, |ws, cx| {
                        if !window.is_maximized() { cx.global_mut::<state::Shared>().config.window = Some((f32::from(b.width), f32::from(b.height))) }
                        ws.save_now(cx);
                    });
                    true
                });
            });
        }
        cx.on_window_closed(|cx| if cx.windows().is_empty() { cx.quit() }).detach();
        cx.activate(true);
    });
}

fn max_level() -> log::Level {
    if std::env::var("GRIDS_LOG").is_ok_and(|v| v == "info") { log::Level::Info } else { log::Level::Warn }
}

/// Logs warnings to stderr (debug builds) without pulling in a logger crate.
fn env_logger_lite() {
    struct L;
    impl log::Log for L {
        fn enabled(&self, m: &log::Metadata) -> bool { m.level() <= max_level() }
        fn log(&self, r: &log::Record) { if self.enabled(r.metadata()) { eprintln!("[{}] {}", r.level(), r.args()) } }
        fn flush(&self) {}
    }
    static LOGGER: L = L;
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(max_level().to_level_filter()));
}

/// Writes panics (message, place, thread, backtrace) to grids-crash.log next
/// to the executable, then lets the default hook run.
/// Return addresses as offsets into grids.exe, so a crash from a build
/// without symbols on the user's machine can be resolved later with the
/// matching grids.pdb (CI's symbols artifact), e.g. in WinDbg: `ln grids+OFFSET`.
#[cfg(windows)]
fn raw_frames() -> String {
    use windows::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let mut frames = [std::ptr::null_mut(); 62];
    // SAFETY: fills our own buffer with this thread's return addresses.
    let n = unsafe { RtlCaptureStackBackTrace(0, &mut frames, None) } as usize;
    // SAFETY: the exe's own module handle is its load address.
    let base = unsafe { GetModuleHandleW(None) }.map(|m| m.0 as usize).unwrap_or(0);
    let mut out = format!("grids.exe base 0x{base:x}\n");
    for (i, f) in frames[..n.min(frames.len())].iter().enumerate() {
        let a = *f as usize;
        if base != 0 && a >= base && a - base < 0x1000_0000 { out += &format!("  #{i:<2} grids+0x{:x}\n", a - base) } else { out += &format!("  #{i:<2} 0x{a:x}\n") }
    }
    out + "\n"
}

#[cfg(not(windows))]
fn raw_frames() -> String { String::new() }

fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let place = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let text = format!(
            "==== {} Grids {} ({} {})\nthread '{}' panicked at {place}:\n{msg}\n\n{}{bt}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            thread.name().unwrap_or("?"),
            raw_frames(),
        );
        if let Some(path) = crash_log_path() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) { let _ = f.write_all(text.as_bytes()); }
        }
        default(info);
    }));
}

fn crash_log_path() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("grids-crash.log"))
}

