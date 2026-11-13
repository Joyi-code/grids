//! The Today view: a virtual pane over the grid listing files modified
//! today (or in the last 3 / 7 days) under the folders the panes show,
//! newest first. The scan runs on a background thread (grids_core::today)
//! and repeats 1 s after a pane's watcher reports changes.

use super::*;
use grids_core::today::{self as scan, TodayItem, TodayScan};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

actions!(today, [TodayUp, TodayDown, TodayOpen, TodayLocate, TodayCopy, TodayShelve, TodayRecycle, TodayClose, TodayFirst, TodayLast]);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Today");
    cx.bind_keys([
        KeyBinding::new("up", TodayUp, c),
        KeyBinding::new("down", TodayDown, c),
        KeyBinding::new("home", TodayFirst, c),
        KeyBinding::new("end", TodayLast, c),
        KeyBinding::new("enter", TodayOpen, c),
        KeyBinding::new("secondary-enter", TodayLocate, c),
        KeyBinding::new("secondary-c", TodayCopy, c),
        KeyBinding::new("secondary-shift-a", TodayShelve, c),
        KeyBinding::new("delete", TodayRecycle, c),
        KeyBinding::new("escape", TodayClose, c),
    ]);
}

const HEAD_H: f32 = 40.;
const COLS_H: f32 = 28.;
const ROW_H: f32 = 30.;
const DEBOUNCE: Duration = Duration::from_secs(1);

pub(super) struct Today {
    /// 1, 3 or 7 days.
    days: u32,
    result: Option<Arc<TodayScan>>,
    scanning: bool,
    cancel: Arc<AtomicBool>,
    _scan: Option<Task<()>>,
    _debounce: Option<Task<()>>,
    /// The panes' folders when the scan started: (pane index, folder).
    roots: Vec<(usize, PathBuf)>,
    sel: Option<usize>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

fn since(days: u32) -> std::time::SystemTime {
    use chrono::{Local, TimeZone};
    let day = Local::now().date_naive() - chrono::Days::new(u64::from(days.max(1) - 1));
    let midnight = day.and_hms_opt(0, 0, 0).and_then(|t| Local.from_local_datetime(&t).earliest());
    midnight.map_or(std::time::UNIX_EPOCH, std::time::SystemTime::from)
}

fn secs(t: std::time::SystemTime) -> i64 { t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) }

impl Workspace {
    pub(super) fn open_today(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.today.is_none() {
            self.today = Some(Today { days: 1, result: None, scanning: false, cancel: Arc::default(), _scan: None, _debounce: None, roots: vec![], sel: None, scroll: UniformListScrollHandle::new(), focus: cx.focus_handle() });
            self.rescan_today(cx);
        }
        if let Some(t) = &self.today { window.focus(&t.focus) }
        cx.notify();
    }

    pub(super) fn close_today(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_today();
        self.refocus(window, cx);
    }

    /// Closes the view, stopping a running scan.
    pub(super) fn drop_today(&mut self) {
        if let Some(t) = self.today.take() { t.cancel.store(true, Ordering::Relaxed) }
    }

    fn today_roots(&self, cx: &App) -> Vec<(usize, PathBuf)> {
        let mut out: Vec<(usize, PathBuf)> = Vec::new();
        for (i, p) in self.panes.iter().enumerate() {
            let path = p.read(cx).state.path.clone();
            if !out.iter().any(|(_, q)| grids_core::link::same_path(q, &path)) { out.push((i, path)) }
        }
        out
    }

    pub(super) fn rescan_today(&mut self, cx: &mut Context<Self>) {
        let roots = self.today_roots(cx);
        let Some(t) = &mut self.today else { return };
        t.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        t.cancel = cancel.clone();
        t.scanning = true;
        t.roots = roots.clone();
        let cut = since(t.days);
        let paths: Vec<PathBuf> = roots.into_iter().map(|(_, p)| p).collect();
        t._scan = Some(cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { scan::scan_recent(&paths, cut, &cancel) }).await;
            let _ = this.update(cx, |this, cx| {
                let Some(t) = &mut this.today else { return };
                // keep the selected file selected across rescans
                let keep = t.sel.and_then(|i| t.result.as_ref()?.items.get(i).map(|it| it.path.clone()));
                t.sel = keep.and_then(|p| res.items.iter().position(|it| it.path == p));
                t.result = Some(Arc::new(res));
                t.scanning = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// A pane's watcher saw changes in `dir`: rescan 1 s later if it is
    /// under one of the scanned folders.
    pub(super) fn today_disk_changed(&mut self, dir: &std::path::Path, cx: &mut Context<Self>) {
        let Some(t) = &mut self.today else { return };
        if !t.roots.iter().any(|(_, r)| grids_core::link::same_path(r, dir) || grids_core::link::is_within(dir, r)) { return }
        t._debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.rescan_today(cx));
        }));
    }

    fn set_today_days(&mut self, days: u32, cx: &mut Context<Self>) {
        let Some(t) = &mut self.today else { return };
        if t.days == days { return }
        t.days = days;
        t.sel = None;
        t.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.rescan_today(cx);
    }

    fn today_item(&self, i: Option<usize>) -> Option<TodayItem> {
        let t = self.today.as_ref()?;
        t.result.as_ref()?.items.get(i.or(t.sel)?).cloned()
    }

    fn move_today(&mut self, delta: isize, to_end: bool, cx: &mut Context<Self>) {
        let Some(t) = &mut self.today else { return };
        let n = t.result.as_ref().map_or(0, |r| r.items.len());
        if n == 0 { return }
        let s = match (t.sel, to_end) {
            (_, true) => if delta < 0 { 0 } else { n - 1 },
            (None, _) => 0,
            (Some(s), _) => (s as isize + delta).clamp(0, n as isize - 1) as usize,
        };
        t.sel = Some(s);
        t.scroll.scroll_to_item(s, ScrollStrategy::Top);
        cx.notify();
    }

    fn today_open(&mut self, i: Option<usize>, cx: &mut Context<Self>) {
        let Some(it) = self.today_item(i) else { return };
        let en = state::en(cx);
        cx.spawn(async move |this, cx| {
            let p = it.path.clone();
            let r = cx.background_executor().spawn(async move { grids_shell::open_with_system(&p) }).await;
            if let Err(e) = r {
                let name = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let _ = this.update(cx, |this, cx| this.show_toast(format!("{}{name}: {e}", if en { "Cannot open " } else { "无法打开 " }), cx));
            }
        }).detach();
    }

    /// Back to the grid: the pane that showed the file's folder tree shows
    /// its folder with the file selected.
    fn today_locate(&mut self, i: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(it) = self.today_item(i) else { return };
        let pane = self.today.as_ref().and_then(|t| t.roots.iter().find(|(_, r)| grids_core::link::same_path(r, &it.root)).map(|(i, _)| *i)).unwrap_or(self.active);
        self.close_today(window, cx);
        if let Some(p) = self.panes.get(pane).cloned() {
            p.update(cx, |p, cx| p.reveal_path(&it.path, cx));
            self.set_active(pane, window, cx);
        }
    }

    fn today_copy(&mut self, i: Option<usize>, cx: &mut Context<Self>) {
        let Some(it) = self.today_item(i) else { return };
        let name = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        state::set_clipboard(vec![it.path], false, cx);
        let msg = if state::en(cx) { format!("Copied “{name}”") } else { format!("已复制「{name}」") };
        self.show_toast(msg, cx);
    }

    fn today_shelve(&mut self, i: Option<usize>, cx: &mut Context<Self>) {
        let Some(it) = self.today_item(i) else { return };
        let added = state::shelve(cx, &[it.path]);
        self.schedule_save(cx);
        self.recheck_shelf(cx);
        let msg = match (added, state::en(cx)) {
            (0, false) => "已在暂存架上".to_string(), (0, true) => "Already on the shelf".to_string(),
            (_, false) => "已添加到暂存架".to_string(), (_, true) => "Added to the shelf".to_string(),
        };
        self.show_toast(msg, cx);
        cx.notify();
    }

    fn today_recycle(&mut self, i: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(it) = self.today_item(i) else { return };
        if let Some(root) = state::locked_root(cx, &it.path) { return self.show_locked(root, cx) }
        let en = state::en(cx);
        let name = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let path = it.path.clone();
        self.confirm = Some(Confirm {
            title: tr(cx, "移入回收站", "Move to Recycle Bin").into(),
            body: if en { format!("Move “{name}” to the Recycle Bin?") } else { format!("将「{name}」移入回收站？") }.into(),
            ok: tr(cx, "删除", "Delete").into(),
            names: vec![],
            more: None,
            action: Rc::new(move |this, _, cx| {
                let paths = vec![path.clone()];
                let dirs: Vec<PathBuf> = path.parent().map(PathBuf::from).into_iter().collect();
                let idx = Some(this.active);
                cx.spawn(async move |this, cx| {
                    let p = paths.clone();
                    let res = cx.background_executor().spawn(async move { ops::run(Op::Recycle, &p, None, idx) }).await;
                    let _ = this.update(cx, |this, cx| {
                        match res {
                            Ok(items) => { ops::after_op(Op::Recycle, &items, cx); this.recheck_shelf(cx) }
                            Err(e) => if e.kind() != std::io::ErrorKind::Interrupted { this.show_toast(format!("{}{e}", if en { "Operation failed: " } else { "操作失败：" }), cx) },
                        }
                        for p in this.panes.clone() { p.update(cx, |p, cx| p.refresh_if_showing(&dirs, cx)) }
                        this.rescan_today(cx);
                    });
                }).detach();
            }),
            danger: true,
            focus: 0,
        });
        self.dialog_focus.focus(window);
        cx.notify();
    }

    pub(super) fn render_today(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.today.as_ref()?;
        let en = state::en(cx);
        let lang = state::lang(cx);
        let offset = state::shared(cx).utc_offset;
        let tt = move |zh: &'static str, e: &'static str| -> &'static str { if en { e } else { zh } };
        let items = t.result.clone();
        let n = items.as_ref().map_or(0, |r| r.items.len());
        let truncated = items.as_ref().is_some_and(|r| r.truncated);
        let roots = t.roots.clone();
        let sel = t.sel;
        let days = t.days;

        let seg = |id: &'static str, label: &'static str, d: u32| {
            let on = days == d;
            div().id(id).h(px(24.)).px(px(10.)).rounded(px(6.)).flex().items_center().cursor_pointer().text_size(px(12.))
                .map(|x| if on { x.bg(theme::raised(Theme::popover_bg())).text_color(Theme::text()).shadow(theme::shadow_menu()) } else { x.text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::text())) })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_today_days(d, cx)))
        };
        let status = if t.scanning && items.is_none() {
            tt("正在扫描…", "Scanning…").to_string()
        } else if en {
            format!("{n} file{} · {} folder{} scanned", if n == 1 { "" } else { "s" }, roots.len(), if roots.len() == 1 { "" } else { "s" })
        } else {
            format!("{n} 个文件 · 扫描 {} 个文件夹", roots.len())
        };
        let header = div().h(px(HEAD_H)).flex_none().flex().items_center().gap(px(10.)).pl(px(14.)).pr(px(8.))
            .child(icon("calendar-clock", 17., Theme::primary()))
            .child(div().flex_none().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(tt("今日", "Today")))
            .child(div().ml(px(6.)).p(px(2.)).rounded(px(8.)).bg(Theme::fill_hover()).flex().items_center().gap(px(2.))
                .child(seg("today-1", tt("今天", "Today"), 1))
                .child(seg("today-3", tt("近 3 天", "3 days"), 3))
                .child(seg("today-7", tt("近 7 天", "7 days"), 7)))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(Theme::text_caption()).child(status))
            .when(truncated, |d| d.child(div().flex_none().text_size(px(12.)).text_color(Theme::star()).child(tt("已达 2 万个文件上限，结果不完整", "Hit the 20,000-file limit; results incomplete"))))
            .child(header_icon("today-refresh", "refresh-cw", 28., 15.).tip(tt("重新扫描", "Rescan"), None).on_click(cx.listener(|this, _, _, cx| this.rescan_today(cx))))
            .child(header_icon("today-close", "x", 28., 16.).tip(tt("关闭今日视图", "Close Today"), Some("Esc")).on_click(cx.listener(|this, _, window, cx| this.close_today(window, cx))));

        // name | folder | modified | size | actions
        let cols = div().h(px(COLS_H)).flex_none().flex().items_center().px(px(16.)).gap(px(12.)).text_size(px(12.)).text_color(Theme::text_caption())
            .child(div().flex_1().min_w(px(160.)).pl(px(26.)).child(tt("名称", "Name")))
            .child(div().w(px(240.)).flex_none().child(tt("所在文件夹", "Folder")))
            .child(div().w(px(130.)).flex_none().child(tt("修改时间", "Modified")))
            .child(div().w(px(80.)).flex_none().flex().justify_end().child(tt("大小", "Size")))
            .child(div().w(px(112.)).flex_none());

        let body = if n == 0 {
            let (title, sub) = if t.scanning { (tt("正在扫描…", "Scanning…"), "") } else {
                (match days { 1 => tt("今天还没有修改过的文件", "Nothing modified today"), 3 => tt("近 3 天没有修改过的文件", "Nothing modified in 3 days"), _ => tt("近 7 天没有修改过的文件", "Nothing modified in 7 days") },
                 tt("扫描范围：各格子当前显示的文件夹，向下 3 层，跳过隐藏和系统文件", "Scope: the folders the panes show, 3 levels deep, hidden and system files skipped"))
            };
            div().flex_1().flex().flex_col().items_center().justify_center().gap(px(6.))
                .child(icon("calendar-clock", 28., Theme::icon_muted()))
                .child(div().text_size(px(14.)).text_color(Theme::text()).child(title))
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(sub))
                .into_any_element()
        } else {
            let view = cx.entity();
            let items = items.clone().unwrap_or_default();
            uniform_list("today-rows", n, move |range, _window, cx| {
                let today_day = { let (y, m, d, _, _) = format::civil(secs(std::time::SystemTime::now()), offset); (y, m, d) };
                range.filter_map(|i| {
                    let it = items.items.get(i)?;
                    let name: SharedString = it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().into();
                    let ext = grids_core::listing::ext_of(&name);
                    let kind = grids_core::FileKind::of_ext(&ext);
                    let glyph = match state::file_icon(&it.path, false, &ext, cx) {
                        Some(image) => img(image).size(px(16.)).flex_none().into_any_element(),
                        None => icon(theme::kind_icon(kind), 16., theme::kind_color(kind)).into_any_element(),
                    };
                    let root_name = grids_core::paths::display_name(&it.root);
                    let rel = it.path.parent().and_then(|p| p.strip_prefix(&it.root).ok()).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                    let folder = if rel.is_empty() { root_name } else { format!("{root_name}/{rel}") };
                    let s = secs(it.modified);
                    let (y, m, d, h, mi) = format::civil(s, offset);
                    let when = if (y, m, d) == today_day { if en { format!("Today {h:02}:{mi:02}") } else { format!("今天 {h:02}:{mi:02}") } } else { format::date(Some(s), offset) };
                    let on = sel == Some(i);
                    let act = |id: &'static str, glyph: &'static str, tip: &'static str, keys: Option<&'static str>| {
                        div().id((id, i)).size(px(24.)).rounded(px(6.)).flex().items_center().justify_center().cursor_pointer()
                            .hover(|s| s.bg(Theme::fill_pressed()))
                            .child(icon(glyph, 14., Theme::icon()))
                            .tip(tip, keys)
                    };
                    Some(div().w_full().px(px(8.)).child(
                        div().id(("today-row", i)).group("today-row").w_full().h(px(ROW_H)).px(px(8.)).rounded(px(8.)).flex().items_center().gap(px(12.)).cursor_default()
                            .map(|d| if on { d.bg(Theme::row_selected()) } else { d.hover(|s| s.bg(Theme::fill_hover())) })
                            .child(div().flex_1().min_w(px(160.)).overflow_hidden().flex().items_center().gap(px(10.))
                                .child(glyph)
                                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(name))))
                            .child(div().w(px(240.)).flex_none().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(Theme::text_caption()).child(ui_text(folder)))
                            .child(div().w(px(130.)).flex_none().whitespace_nowrap().text_color(Theme::text_caption()).child(when))
                            .child(div().w(px(80.)).flex_none().flex().justify_end().whitespace_nowrap().text_color(Theme::text_caption()).child(format::size(it.size, lang)))
                            .child(div().w(px(112.)).flex_none().flex().justify_end().gap(px(4.))
                                .when(!on, |d| d.invisible().group_hover("today-row", |s| s.visible()))
                                .child(act("today-locate", "locate", tt("在原格子中定位", "Show in its pane"), Some("Ctrl+Enter"))
                                    .on_click(window_listener(&view, move |this, window, cx| this.today_locate(Some(i), window, cx))))
                                .child(act("today-shelve", "inbox", tt("添加到暂存架", "Add to shelf"), Some("Ctrl+Shift+A"))
                                    .on_click(window_listener(&view, move |this, _, cx| this.today_shelve(Some(i), cx))))
                                .child(act("today-copy", "copy", tt("复制", "Copy"), Some("Ctrl+C"))
                                    .on_click(window_listener(&view, move |this, _, cx| this.today_copy(Some(i), cx))))
                                .child(act("today-recycle", "trash-2", tt("移入回收站", "Move to Recycle Bin"), Some("Del"))
                                    .on_click(window_listener(&view, move |this, window, cx| this.today_recycle(Some(i), window, cx)))))
                            .on_click(window_listener_ev(&view, move |this, ev: &ClickEvent, window, cx| {
                                if let Some(t) = &mut this.today { t.sel = Some(i); window.focus(&t.focus) }
                                if ev.click_count() >= 2 { this.today_open(Some(i), cx) }
                                cx.notify();
                            })),
                    ).into_any_element())
                }).collect()
            })
            .track_scroll(t.scroll.clone())
            .flex_1()
            .min_h_0()
            .pb(px(6.))
            .into_any_element()
        };

        Some(
            div().id("today").key_context("Today").track_focus(&t.focus).flex_1().min_w_0().min_h_0().flex().flex_col().bg(Theme::bg_body()).text_size(px(13.))
                .on_action(cx.listener(|this, _: &TodayUp, _, cx| this.move_today(-1, false, cx)))
                .on_action(cx.listener(|this, _: &TodayDown, _, cx| this.move_today(1, false, cx)))
                .on_action(cx.listener(|this, _: &TodayFirst, _, cx| this.move_today(-1, true, cx)))
                .on_action(cx.listener(|this, _: &TodayLast, _, cx| this.move_today(1, true, cx)))
                .on_action(cx.listener(|this, _: &TodayOpen, _, cx| this.today_open(None, cx)))
                .on_action(cx.listener(|this, _: &TodayLocate, window, cx| this.today_locate(None, window, cx)))
                .on_action(cx.listener(|this, _: &TodayCopy, _, cx| this.today_copy(None, cx)))
                .on_action(cx.listener(|this, _: &TodayShelve, _, cx| this.today_shelve(None, cx)))
                .on_action(cx.listener(|this, _: &TodayRecycle, window, cx| this.today_recycle(None, window, cx)))
                .on_action(cx.listener(|this, _: &TodayClose, window, cx| this.close_today(window, cx)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, _| if let Some(t) = &this.today { window.focus(&t.focus) }))
                .child(header)
                .child(div().h(px(1.)).flex_none().bg(Theme::line_divider()))
                .child(cols)
                .child(body)
                .into_any_element(),
        )
    }
}

/// A click handler on the workspace from inside a `uniform_list` row
/// (which only has `&mut App`).
pub(super) fn window_listener(view: &Entity<Workspace>, f: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let view = view.downgrade();
    move |_, window, cx| {
        cx.stop_propagation();
        let _ = view.update(cx, |this, cx| f(this, window, cx));
    }
}

fn window_listener_ev(view: &Entity<Workspace>, f: impl Fn(&mut Workspace, &ClickEvent, &mut Window, &mut Context<Workspace>) + 'static) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let view = view.downgrade();
    move |ev, window, cx| { let _ = view.update(cx, |this, cx| f(this, ev, window, cx)); }
}
