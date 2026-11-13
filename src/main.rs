#![cfg_attr(windows, windows_subsystem = "windows")]
//! Grids: a multi-pane file manager drawn with GPUI.

mod app_icon;
mod assets;
mod input;
mod menu;
mod pane;
mod state;
mod theme;
mod ui;
mod workspace;

use gpui::*;
use grids_core::format::Lang;
use grids_core::Config;

fn main() {
    install_crash_log();
    env_logger_lite();
    let path = Config::default_path();
    let mut config = Config::load(&path);
    // Simplified Chinese by default; GRIDS_LANG=en switches to English.
    if let Some(l) = std::env::var_os("GRIDS_LANG") { config.lang = if l == "en" { Lang::En } else { Lang::Zh } }
    let (w, h) = config.window.unwrap_or((1360., 860.));
    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
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
                    title: Some("Grids".into()),
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
            "==== {} Grids {} ({} {})\nthread '{}' panicked at {place}:\n{msg}\n\n{bt}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            thread.name().unwrap_or("?"),
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
