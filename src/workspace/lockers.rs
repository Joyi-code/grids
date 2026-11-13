//! "Who has this file open": checked before deletes, moves and renames (and
//! from the menu, 查看占用). The Restart Manager query runs on a background
//! thread; the card appears at once only when something holds the files,
//! or after 300 ms of checking (big folders). Per process: switch to its
//! window, ask it to close, or end it (confirmed, red). Elevated or other
//! users' processes get no end button: administrator rights are needed.

use super::{DialogCancel, DialogAccept, Workspace};
use crate::state::{self, tr};
use crate::theme::{self, Theme};
use crate::ui::{icon, ui_text, TipExt};
use gpui::{prelude::FluentBuilder, *};
use grids_shell::{AppKind, Locker, Lockers};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// What runs once nothing holds the files any more.
pub type Then = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct LockersDlg {
    paths: Vec<PathBuf>,
    /// None while checking.
    list: Option<Lockers>,
    then: Option<Then>,
    /// Shown (an immediate result or 300 ms of checking).
    visible: bool,
    /// The process whose 强制结束 waits for a second click.
    ask_kill: Option<u32>,
    seq: u64,
    note: Option<SharedString>,
}

fn kind_text(k: AppKind, en: bool) -> &'static str {
    match (k, en) {
        (AppKind::Window | AppKind::OtherWindow, false) => "应用程序",
        (AppKind::Window | AppKind::OtherWindow, true) => "App",
        (AppKind::Service, false) => "服务",
        (AppKind::Service, true) => "Service",
        (AppKind::Explorer, false) => "资源管理器",
        (AppKind::Explorer, true) => "Explorer",
        (AppKind::Console, false) => "控制台程序",
        (AppKind::Console, true) => "Console",
        (AppKind::Critical, false) => "系统关键进程",
        (AppKind::Critical, true) => "System process",
        (AppKind::Unknown, false) => "进程",
        (AppKind::Unknown, true) => "Process",
    }
}

/// Debug builds only, for screenshots off Windows: `GRIDS_DEMO_LOCKERS` =
/// `name|pid|kind(w,s,c,u)|can_end(0/1)|service;…`.
#[cfg(debug_assertions)]
fn demo_lockers() -> Option<Lockers> {
    let v = std::env::var("GRIDS_DEMO_LOCKERS").ok()?;
    let list = v.split(';').filter_map(|r| {
        let f: Vec<&str> = r.split('|').collect();
        let kind = match *f.get(2)? { "w" => AppKind::Window, "s" => AppKind::Service, "c" => AppKind::Console, _ => AppKind::Unknown };
        Some(Locker { name: f[0].into(), pid: f.get(1)?.parse().ok()?, kind, hwnd: (kind == AppKind::Window).then_some(1), exe: None,
            service: f.get(4).filter(|s| !s.is_empty()).map(|s| s.to_string()), can_terminate: f.get(3) == Some(&"1") })
    }).collect();
    Some(Lockers { list, truncated: false })
}
#[cfg(not(debug_assertions))]
fn demo_lockers() -> Option<Lockers> { None }

impl Workspace {
    /// Runs `then` once nothing holds `paths`; otherwise shows who does.
    /// `then` None: the menu's 查看占用 (always shows the result).
    pub(super) fn check_lockers(&mut self, paths: Vec<PathBuf>, then: Option<Then>, window: &mut Window, cx: &mut Context<Self>) {
        let seq = self.lockers.as_ref().map_or(0, |d| d.seq) + 1;
        let show_now = then.is_none();
        self.lockers = Some(LockersDlg { paths, list: None, then, visible: show_now, ask_kill: None, seq, note: None });
        if show_now { self.dialog_focus.focus(window) }
        self.query_lockers(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(d) = &mut this.lockers && d.seq == seq && d.list.is_none() && !d.visible {
                    d.visible = true;
                    this.dialog_focus.focus(window);
                    cx.notify();
                }
            });
        }).detach();
        cx.notify();
    }

    fn query_lockers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = &mut self.lockers else { return };
        d.seq += 1;
        let seq = d.seq;
        let paths = d.paths.clone();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_executor().spawn(async move { demo_lockers().map_or_else(|| grids_shell::lockers(&paths), Ok) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(d) = &mut this.lockers else { return };
                if d.seq != seq { return }
                // A failed query must not block the operation.
                let res = res.map_err(|e| e.to_string());
                let empty = res.as_ref().map_or(true, |l| l.list.is_empty());
                if empty && let Some(then) = d.then.clone() {
                    this.lockers = None;
                    if this.confirm.is_none() { this.refocus(window, cx) }
                    // `then` may update the workspace; run it after this update ends.
                    window.defer(cx, move |window, cx| then(window, cx));
                    return;
                }
                match res {
                    Ok(l) => d.list = Some(l),
                    Err(e) => { d.list = Some(Lockers::default()); d.note = Some(e.into()) }
                }
                d.ask_kill = None;
                if !d.visible { d.visible = true; this.dialog_focus.focus(window) }
                cx.notify();
            });
        }).detach();
    }

    fn close_lockers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lockers.take().is_some() { self.refocus(window, cx) }
    }

    /// Looks again after the program had a moment to let go.
    fn requery_soon(&mut self, ms: u64, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(ms)).await;
            let _ = this.update_in(cx, |this, window, cx| this.query_lockers(window, cx));
        }).detach();
    }

    fn locker_action(&mut self, pid: u32, what: u8, window: &mut Window, cx: &mut Context<Self>) {
        let note = |e: std::io::Error, cx: &App| -> SharedString {
            match e.kind() {
                std::io::ErrorKind::PermissionDenied => tr(cx, "权限不足：需要以管理员身份运行", "Access denied: administrator rights needed").into(),
                std::io::ErrorKind::NotFound => tr(cx, "该程序没有可用的窗口", "That program has no window").into(),
                _ => e.to_string().into(),
            }
        };
        match what {
            0 => if let Err(e) = grids_shell::activate_process_window(pid) { let n = note(e, cx); if let Some(d) = &mut self.lockers { d.note = Some(n) } },
            1 => match grids_shell::close_process(pid) {
                Ok(()) => self.requery_soon(1500, window, cx),
                Err(e) => { let n = note(e, cx); if let Some(d) = &mut self.lockers { d.note = Some(n) } }
            },
            _ => {
                if let Some(d) = &mut self.lockers { d.ask_kill = None }
                cx.spawn_in(window, async move |this, cx| {
                    let r = cx.background_executor().spawn(async move { grids_shell::terminate_process(pid) }).await;
                    let _ = this.update_in(cx, |this, window, cx| match r {
                        Ok(()) => this.requery_soon(400, window, cx),
                        Err(e) => { let n = note(e, cx); if let Some(d) = &mut this.lockers { d.note = Some(n) } cx.notify() }
                    });
                }).detach();
            }
        }
        cx.notify();
    }

    fn render_locker_row(&self, i: usize, l: &Locker, ask_kill: bool, cx: &mut Context<Self>) -> AnyElement {
        let en = state::en(cx);
        let me = l.pid == std::process::id();
        let pid = l.pid;
        let small = |id: (&'static str, usize), glyph: &'static str, label: &'static str, danger: bool| {
            div().id(id).h(px(26.)).px(px(9.)).rounded(px(6.)).flex().flex_none().items_center().gap(px(5.)).cursor_pointer().text_size(px(12.))
                .map(|d| if danger { d.bg(Theme::danger()).text_color(gpui::white()).hover(|s| s.bg(Theme::danger_hover())) } else { d.bg(Theme::fill_hover_strong()).text_color(Theme::text()).hover(|s| s.bg(Theme::fill_pressed())) })
                .when(!glyph.is_empty(), |d| d.child(icon(glyph, 13., if danger { gpui::white() } else { Theme::icon() })))
                .child(label)
        };
        let glyph = match l.exe.as_ref().and_then(|p| state::file_icon(p, false, "exe", cx)) {
            Some(img_) => img(img_).size(px(24.)).flex_none().into_any_element(),
            None => icon("app-window", 22., Theme::icon_muted()).into_any_element(),
        };
        let caption = if me { tr(cx, "Grids（本程序）", "Grids (this app)").to_string() } else {
            format!("PID {} · {}{}", l.pid, kind_text(l.kind, en), l.service.as_ref().map(|s| format!(" · {s}")).unwrap_or_default())
        };
        let mut buttons = div().flex().flex_none().items_center().gap(px(6.));
        if ask_kill {
            buttons = buttons
                .child(div().text_size(px(12.)).text_color(Theme::danger()).child(tr(cx, "未保存的内容会丢失", "Unsaved work is lost")))
                .child(small(("lk-no", i), "", tr(cx, "取消", "Cancel"), false).on_click(cx.listener(|this, _, _, cx| { if let Some(d) = &mut this.lockers { d.ask_kill = None } cx.notify() })))
                .child(small(("lk-yes", i), "octagon-x", tr(cx, "强制结束", "End now"), true).on_click(cx.listener(move |this, _, window, cx| this.locker_action(pid, 2, window, cx))));
        } else if !me {
            let windowed = l.hwnd.is_some() || matches!(l.kind, AppKind::Window | AppKind::OtherWindow | AppKind::Explorer);
            if windowed {
                buttons = buttons
                    .child(small(("lk-go", i), "app-window", tr(cx, "切换到窗口", "Switch to"), false).on_click(cx.listener(move |this, _, window, cx| this.locker_action(pid, 0, window, cx))))
                    .child(small(("lk-close", i), "power", tr(cx, "关闭程序", "Close"), false).on_click(cx.listener(move |this, _, window, cx| this.locker_action(pid, 1, window, cx))));
            }
            if l.can_terminate && l.kind != AppKind::Critical {
                buttons = buttons.child(small(("lk-kill", i), "", tr(cx, "强制结束", "End"), false).text_color(Theme::danger())
                    .on_click(cx.listener(move |this, _, _, cx| { if let Some(d) = &mut this.lockers { d.ask_kill = Some(pid) } cx.notify() })));
            } else {
                buttons = buttons.child(div().id(("lk-admin", i)).flex().items_center().gap(px(4.)).text_size(px(12.)).text_color(Theme::text_caption())
                    .child(icon("shield-alert", 13., Theme::changed()))
                    .child(tr(cx, "需要管理员权限", "Needs administrator"))
                    .tip(tr(cx, "该进程以管理员或其他用户身份运行，Grids 无法结束它", "It runs elevated or as another user; Grids can't end it"), None));
            }
        }
        div().id(("locker", i)).h(px(48.)).px(px(10.)).rounded(px(7.)).flex().items_center().gap(px(10.))
            .hover(|s| s.bg(Theme::fill_hover()))
            .child(glyph)
            .child(div().flex_1().min_w_0().flex().flex_col()
                .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(l.name.clone())))
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(caption)))
            .child(buttons)
            .into_any_element()
    }

    pub(super) fn render_lockers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let d = self.lockers.as_ref().filter(|d| d.visible)?;
        let en = state::en(cx);
        let n = d.paths.len();
        let name = d.paths.first().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let what = if n == 1 { if en { format!("“{name}”") } else { format!("「{name}」") } } else if en { format!("{n} items") } else { format!("{n} 项") };
        let (title, body): (SharedString, String) = match &d.list {
            None => (tr(cx, "正在检查占用…", "Checking…").into(), if en { format!("Looking for programs that have {what} open.") } else { format!("正在查找打开了{what}的程序。") }),
            Some(l) if l.list.is_empty() => (tr(cx, "没有程序占用", "Not in use").into(), if en { format!("No program has {what} open.") } else { format!("没有程序打开{what}。") }),
            Some(l) => (tr(cx, "文件正被占用", "In use").into(), if en { format!("{} program(s) have {what} open. Close them, then retry.", l.list.len()) } else { format!("{} 个程序打开了{what}，关闭后点“重试”。", l.list.len()) }),
        };
        let truncated = d.list.as_ref().is_some_and(|l| l.truncated);
        let rows: Vec<AnyElement> = d.list.as_ref().map(|l| l.list.iter().enumerate().map(|(i, x)| self.render_locker_row(i, x, d.ask_kill == Some(x.pid), cx)).collect()).unwrap_or_default();
        let has_then = d.then.is_some();
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div().id(id).h(px(30.)).min_w(px(76.)).px(px(14.)).rounded(px(7.)).flex().items_center().justify_center().cursor_pointer()
                .map(|d| if primary { d.bg(Theme::primary()).text_color(gpui::white()).font_weight(FontWeight::MEDIUM).hover(|s| s.opacity(0.9)) } else { d.bg(Theme::fill_hover_strong()).text_color(Theme::text()).hover(|s| s.bg(Theme::fill_pressed())) })
                .child(label)
        };
        let card = div().id("lockers").w(px(520.)).p(px(20.)).pb(px(16.)).flex().flex_col()
            .bg(theme::raised(Theme::popover_bg())).border_1().border_color(Theme::line_border()).rounded(px(8.)).shadow(theme::shadow_menu())
            .font_family(theme::UI_FONT).text_size(px(13.)).text_color(Theme::text())
            .occlude()
            .track_focus(&self.dialog_focus)
            .key_context("Dialog")
            .on_action(cx.listener(|this, _: &DialogCancel, window, cx| this.close_lockers(window, cx)))
            .on_action(cx.listener(|this, _: &DialogAccept, window, cx| this.query_lockers(window, cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().flex().items_center().gap(px(8.))
                .child(icon(if d.list.is_none() { "refresh-cw" } else { "file-lock" }, 17., Theme::icon()))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title)))
            .child(div().mt(px(8.)).line_height(px(20.)).text_color(Theme::text_caption()).child(body))
            .when(!rows.is_empty(), |c| c.child(div().id("locker-list").mt(px(10.)).max_h(px(260.)).overflow_y_scroll().flex().flex_col().gap(px(2.)).children(rows)))
            .when(truncated, |c| c.child(div().mt(px(8.)).text_size(px(12.)).text_color(Theme::text_caption()).child(if en { format!("Only the first {} files were checked.", grids_shell::LOCKERS_MAX_FILES) } else { format!("文件较多，只检查了前 {} 个。", grids_shell::LOCKERS_MAX_FILES) })))
            .children(d.note.clone().map(|n| div().mt(px(8.)).text_size(px(12.)).text_color(Theme::danger()).child(n)))
            .child(div().mt(px(18.)).flex().justify_end().gap(px(8.))
                .child(button("lk-cancel", if has_then { tr(cx, "取消", "Cancel") } else { tr(cx, "关闭", "Close") }, false)
                    .on_click(cx.listener(|this, _, window, cx| this.close_lockers(window, cx))))
                .when(d.list.is_some(), |r| r.child(button("lk-retry", if has_then { tr(cx, "重试", "Retry") } else { tr(cx, "刷新", "Refresh") }, true)
                    .on_click(cx.listener(|this, _, window, cx| { if let Some(d) = &mut this.lockers { d.note = None } this.query_lockers(window, cx) })))));
        Some(
            deferred(
                div().id("lockers-scrim").absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(0.18)).occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_lockers(window, cx)))
                    .child(card)
                    .with_animation("lockers-fade", Animation::new(Duration::from_millis(110)), |d, t| d.opacity(t)),
            )
            .with_priority(5)
            .into_any_element(),
        )
    }
}
