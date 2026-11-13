//! The window: title bar, sidebar, the grid of panes with its splitters, the
//! layout picker and the status bar. Owns the panes and saves the session.

use crate::input::{InputEvent, TextInput};
use crate::pane::{PaneEvent, PaneView};
use crate::state::{self, Shared, tr};
use crate::theme::{self, Theme};
use crate::ui::{HEADER_H, TipExt, TitleBarExt, header_icon, icon, popover, ui_text, window_controls};
use gpui::{prelude::FluentBuilder, *};
use grids_core::format;
use grids_core::layout::{self, Layout};
use grids_core::session::{Appearance, PaneSave, Session};
use grids_shell::KnownFolderKind;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

actions!(
    workspace,
    [NextPane, PrevPane, Pane1, Pane2, Pane3, Pane4, Pane5, Pane6, Pane7, Pane8, Pane9, ToggleMaximize, ToggleSidebar, PickLayout, CycleTheme, CloseAbout, SwapLeft, SwapRight, SwapUp, SwapDown, DialogCancel, DialogAccept, DialogNext]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("ctrl-tab", NextPane, c),
        KeyBinding::new("ctrl-shift-tab", PrevPane, c),
        KeyBinding::new("secondary-1", Pane1, c),
        KeyBinding::new("secondary-2", Pane2, c),
        KeyBinding::new("secondary-3", Pane3, c),
        KeyBinding::new("secondary-4", Pane4, c),
        KeyBinding::new("secondary-5", Pane5, c),
        KeyBinding::new("secondary-6", Pane6, c),
        KeyBinding::new("secondary-7", Pane7, c),
        KeyBinding::new("secondary-8", Pane8, c),
        KeyBinding::new("secondary-9", Pane9, c),
        KeyBinding::new("secondary-shift-m", ToggleMaximize, c),
        KeyBinding::new("secondary-b", ToggleSidebar, c),
        KeyBinding::new("secondary-shift-g", PickLayout, c),
        KeyBinding::new("escape", CloseAbout, Some("About")),
        // Dialogs: Esc cancels; Enter / Space press the focused button (Cancel
        // when the dialog opens); Tab and the arrows move between buttons.
        KeyBinding::new("escape", DialogCancel, Some("Dialog")),
        KeyBinding::new("enter", DialogAccept, Some("Dialog")),
        KeyBinding::new("space", DialogAccept, Some("Dialog")),
        KeyBinding::new("tab", DialogNext, Some("Dialog")),
        KeyBinding::new("shift-tab", DialogNext, Some("Dialog")),
        KeyBinding::new("left", DialogNext, Some("Dialog")),
        KeyBinding::new("right", DialogNext, Some("Dialog")),
        KeyBinding::new("ctrl-alt-shift-left", SwapLeft, c),
        KeyBinding::new("ctrl-alt-shift-right", SwapRight, c),
        KeyBinding::new("ctrl-alt-shift-up", SwapUp, c),
        KeyBinding::new("ctrl-alt-shift-down", SwapDown, c),
    ]);
}

/// Splitter grab area either side of the 1px hairline.
const GRAB: f32 = 4.;
const STATUS_H: f32 = 28.;
const SIDEBAR_MIN: f32 = 180.;
const SIDEBAR_MAX: f32 = 420.;
/// quiet-md's sidebar geometry: pills inset 14 from the edge, 30 high with
/// radius 9 on a 31 pitch, 18pt glyphs in a 20 box, 9 before the label;
/// section headers 34 (6 above a 28 label), groups 16 apart.
const SB_X: f32 = 14.;
const SB_ROW: f32 = 30.;
const SB_R: f32 = 9.;
const SB_ICON: f32 = 18.;
const SB_SECTION: f32 = 34.;

/// A grid splitter being dragged: the divider after column `i` (`col` None)
/// or after pane `i` of column `col`.
#[derive(Clone)]
struct SplitDrag {
    col: Option<usize>,
    i: usize,
}

impl Render for SplitDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { Empty }
}

#[derive(Clone)]
struct SidebarDrag;

impl Render for SidebarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { Empty }
}

/// A confirmation dialog (GPUI-drawn, like the menus): title, message, the
/// destructive button's label and what it does.
struct Confirm {
    title: SharedString,
    body: SharedString,
    ok: SharedString,
    action: Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>,
    /// The focused button: 0 Cancel (the default), 1 the action.
    focus: u8,
}

pub struct Workspace {
    panes: Vec<Entity<PaneView>>,
    active: usize,
    maximized: Option<usize>,
    layout: Layout,
    picker: bool,
    /// The About dialog: None when closed, else the tab (0 licence, 1 third-party notices).
    about: Option<u8>,
    about_focus: FocusHandle,
    /// The version popover (from the version label at the bottom left).
    version_pop: bool,
    version_btn: Rc<Cell<Bounds<Pixels>>>,
    confirm: Option<Confirm>,
    dialog_focus: FocusHandle,
    /// The picker's "by column" editor: panes per column (0 = no column),
    /// and whether the mouse is held down on it.
    col_draft: [usize; 4],
    col_dragging: bool,
    grid_bounds: Rc<Cell<Bounds<Pixels>>>,
    layout_btn: Rc<Cell<Bounds<Pixels>>>,
    toast: Option<(String, u64)>,
    toast_seq: u64,
    save_task: Option<Task<()>>,
    session_input: Option<(Entity<TextInput>, Vec<Subscription>)>,
    drives: Vec<grids_shell::Drive>,
    known: Vec<grids_shell::KnownFolder>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

fn home_dir() -> PathBuf {
    grids_shell::known_folders().into_iter().find(|k| k.kind == KnownFolderKind::Home).map(|k| k.path).unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }))
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = state::shared(cx).config.current.clone().normalized(&home_dir());
        let mut me = Self {
            panes: vec![],
            active: session.active,
            maximized: None,
            layout: session.layout.clone(),
            picker: false,
            about: None,
            about_focus: cx.focus_handle(),
            version_pop: false,
            version_btn: Rc::default(),
            confirm: None,
            dialog_focus: cx.focus_handle(),
            col_draft: [0; 4],
            col_dragging: false,
            grid_bounds: Rc::default(),
            layout_btn: Rc::default(),
            toast: None,
            toast_seq: 0,
            save_task: None,
            session_input: None,
            drives: grids_shell::drives(),
            known: grids_shell::known_folders(),
            focus: cx.focus_handle(),
            _subs: vec![],
        };
        me.panes = session.panes.iter().enumerate().map(|(i, p)| me.make_pane(i, p, window, cx)).collect();
        me.set_active(me.active, window, cx);
        me._subs.push(cx.observe_window_appearance(window, |this, window, cx| { this.apply_theme(window, cx) }));
        me.apply_theme(window, cx);
        // Read the "New" templates now so the first right-click has them.
        let _ = state::new_items(cx);
        // Drives come and go (USB sticks): look again every few seconds.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            let drives = cx.background_executor().spawn(async { grids_shell::drives() }).await;
            if this.update(cx, |this, cx| if this.drives != drives { this.drives = drives; cx.notify() }).is_err() { break }
        }).detach();
        me
    }

    fn make_pane(&mut self, i: usize, save: &PaneSave, window: &mut Window, cx: &mut Context<Self>) -> Entity<PaneView> {
        let pane = cx.new(|cx| PaneView::new(i, save, window, cx));
        let sub = cx.subscribe_in(&pane, window, |this, pane, ev: &PaneEvent, window, cx| match ev {
            PaneEvent::Activated => {
                if let Some(i) = this.panes.iter().position(|p| p == pane) && i != this.active { this.set_active_only(i, cx) }
            }
            PaneEvent::Changed => this.schedule_save(cx),
            PaneEvent::FsChanged(dirs) => {
                for p in this.panes.clone() { p.update(cx, |p, cx| p.refresh_if_showing(dirs, cx)) }
            }
            PaneEvent::Toast(msg) => this.show_toast(msg.clone(), cx),
            PaneEvent::OpenInNext(path) => {
                let n = this.panes.len();
                let Some(i) = this.panes.iter().position(|p| p == pane) else { return };
                if n < 2 { pane.update(cx, |p, cx| p.navigate(path.clone(), cx)); return }
                let j = (i + 1) % n;
                this.panes[j].clone().update(cx, |p, cx| p.navigate(path.clone(), cx));
                this.set_active(j, window, cx);
            }
        });
        let _ = window;
        self._subs.push(sub);
        pane
    }

    fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dark = match state::shared(cx).config.appearance {
            Appearance::System => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
            Appearance::Light => false,
            Appearance::Dark => true,
        };
        if theme::set_dark(dark) {
            for p in self.panes.clone() { p.update(cx, |_, cx| cx.notify()) }
            window.refresh();
        }
        cx.notify();
    }

    fn set_active_only(&mut self, i: usize, cx: &mut Context<Self>) {
        self.active = i.min(self.panes.len().saturating_sub(1));
        for (j, p) in self.panes.iter().enumerate() {
            let on = j == self.active;
            p.update(cx, |p, cx| if p.active != on { p.active = on; cx.notify() });
        }
        if self.maximized.is_some() { self.maximized = Some(self.active) }
        self.schedule_save(cx);
        cx.notify();
    }

    fn set_active(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.set_active_only(i, cx);
        if let Some(p) = self.panes.get(self.active) { window.focus(&p.read(cx).focus) }
    }

    fn active_pane(&self) -> Option<&Entity<PaneView>> { self.panes.get(self.active) }

    fn show_toast(&mut self, msg: String, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some((msg, seq));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |this, cx| if this.toast.as_ref().is_some_and(|t| t.1 == seq) { this.toast = None; cx.notify() });
        }).detach();
    }

    // ---- sessions and saving

    fn session(&self, cx: &App) -> Session {
        Session { layout: self.layout.clone(), panes: self.panes.iter().map(|p| p.read(cx).save()).collect(), active: self.active }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(600)).await;
            let _ = this.update(cx, |this, cx| this.save_now(cx));
        }));
    }

    pub fn save_now(&mut self, cx: &mut App) {
        let session = self.session(cx);
        let s = cx.global_mut::<Shared>();
        s.config.current = session;
        if let Err(e) = s.config.save(&s.config_path) { log::warn!("saving settings: {e}") }
    }

    fn load_session(&mut self, session: Session, window: &mut Window, cx: &mut Context<Self>) {
        let session = session.normalized(&home_dir());
        self.layout = session.layout.clone();
        self.maximized = None;
        self.panes.clear();
        self._subs.retain(|_| false);
        self._subs.push(cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)));
        self.panes = session.panes.iter().enumerate().map(|(i, p)| self.make_pane(i, p, window, cx)).collect();
        self.set_active(session.active, window, cx);
    }

    fn start_save_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = &state::shared(cx).config;
        let name = cfg.session_name.clone().unwrap_or_else(|| cfg.fresh_session_name(tr(cx, "会话", "Session")));
        let input = cx.new(|cx| {
            let mut t = TextInput::new(&name, tr(cx, "会话名称", "Session name"), cx);
            t.select_range(0..name.len(), cx);
            t
        });
        let subs = vec![cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit => {
                let name = input.read(cx).text().trim().to_string();
                this.session_input = None;
                if !name.is_empty() {
                    this.save_now(cx);
                    cx.global_mut::<Shared>().config.save_session(&name);
                    this.save_now(cx);
                    let msg = if state::en(cx) { format!("Saved session \u{201c}{name}\u{201d}") } else { format!("已保存会话\u{201c}{name}\u{201d}") };
                    this.show_toast(msg, cx);
                }
                this.set_active(this.active, window, cx);
            }
            InputEvent::Cancel => { this.session_input = None; this.set_active(this.active, window, cx) }
            InputEvent::Changed => {}
        })];
        input.read(cx).focus(window);
        self.session_input = Some((input, subs));
        cx.notify();
    }

    // ---- layout

    fn toggle_picker(&mut self, cx: &mut Context<Self>) {
        self.picker = !self.picker;
        self.col_draft = [0; 4];
        for (d, &n) in self.col_draft.iter_mut().zip(&self.layout.cols) { *d = n }
        self.col_dragging = false;
        cx.notify();
    }

    /// Swaps the active pane with its neighbour; folder, history, selection
    /// and view move along, the cells stay.
    fn swap_pane(&mut self, dx: i32, dy: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(j) = self.layout.neighbour(self.active, dx, dy) else { return };
        let i = self.active;
        self.panes.swap(i, j);
        for k in [i, j] { self.panes[k].update(cx, |p, cx| { p.index = k; cx.notify() }) }
        self.maximized = None;
        self.set_active(j, window, cx);
        self.schedule_save(cx);
    }

    fn apply_layout(&mut self, new: Layout, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = false;
        self.apply_layout_keep_picker(new, window, cx);
    }

    fn apply_layout_keep_picker(&mut self, new: Layout, window: &mut Window, cx: &mut Context<Self>) {
        if new.cols == self.layout.cols { cx.notify(); return }
        let carry = new.carry_from(&self.layout);
        let old = std::mem::take(&mut self.panes);
        let base = old.get(self.active).map(|p| p.read(cx).save()).unwrap_or_default();
        let mut active = 0;
        let mut panes = Vec::with_capacity(new.count());
        for (i, from) in carry.into_iter().enumerate() {
            match from.and_then(|j| old.get(j).map(|p| (j, p.clone()))) {
                Some((j, p)) => {
                    if j == self.active { active = i }
                    p.update(cx, |p, cx| { p.index = i; cx.notify() });
                    panes.push(p);
                }
                None => panes.push(self.make_pane(i, &PaneSave { path: base.path.clone(), ..Default::default() }, window, cx)),
            }
        }
        self.panes = panes;
        self.layout = new;
        self.maximized = None;
        self.set_active(active, window, cx);
        self.schedule_save(cx);
    }

    fn cycle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let c = &mut cx.global_mut::<Shared>().config;
        c.appearance = match c.appearance { Appearance::System => Appearance::Light, Appearance::Light => Appearance::Dark, Appearance::Dark => Appearance::System };
        self.apply_theme(window, cx);
        self.schedule_save(cx);
    }

    fn go_to(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.active_pane().cloned() {
            p.update(cx, |p, cx| p.navigate(path, cx));
            window.focus(&p.read(cx).focus);
        }
    }

    // ---- rendering

    /// The top of the white plane, as quiet-md's Home: the title (session
    /// name), a filter field for the active pane in the middle, and icon
    /// buttons at the top right before the window controls.
    fn render_topbar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let en = state::en(cx);
        let cfg = &state::shared(cx).config;
        let sidebar_open = cfg.sidebar_open;
        let title = cfg.session_name.clone().unwrap_or_else(|| "Grids".into());
        let appearance = cfg.appearance;
        let l = &self.layout;
        let theme_icon = match appearance { Appearance::System => "sun-moon", Appearance::Light => "sun", Appearance::Dark => "moon" };
        let theme_tip = match appearance {
            Appearance::System => tr(cx, "外观：跟随系统", "Appearance: system"),
            Appearance::Light => tr(cx, "外观：浅色", "Appearance: light"),
            Appearance::Dark => tr(cx, "外观：深色", "Appearance: dark"),
        };
        let controls = window_controls(window, en);
        let has_controls = controls.is_some();
        // The filter field, centred in the plane when there is room.
        let plane_w = f32::from(window.viewport_size().width) - if sidebar_open { self.sidebar_w(cx) } else { 0. };
        let search_w = (plane_w - 560.).min(360.);
        let search = (search_w >= 160.).then(|| {
            div().id("header-search").absolute().left(px((plane_w - search_w) / 2.)).top(px(8.))
                .w(px(search_w)).h(px(32.)).no_drag().flex().items_center().gap(px(8.)).px(px(10.))
                .rounded(px(9.)).bg(Theme::fill_hover())
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
                .child(icon("search", 15., Theme::icon()))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.)).text_color(Theme::text_caption()).child(if en { format!("Filter pane {}…", self.active + 1) } else { format!("筛选窗格 {}…", self.active + 1) }))
                .when(search_w >= 220., |d| d.child(div().flex_none().text_size(px(11.)).text_color(Theme::text_placeholder()).child("Ctrl F")))
                .tip(tr(cx, "按名称筛选当前窗格", "Filter the active pane by name"), Some("Ctrl+F"))
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(p) = this.active_pane().cloned() { p.update(cx, |p, cx| p.start_filter(window, cx)) }
                }))
        });
        div()
            .id("top-bar")
            .relative()
            .h(px(HEADER_H))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(if sidebar_open { 22. } else { 12. }))
            .title_bar()
            .when(!sidebar_open, |d| d.child(
                header_icon("open-sidebar", "panel-left-open", 28., 17.).mr(px(10.))
                    .tip(tr(cx, "展开侧栏", "Show sidebar"), Some("Ctrl+B"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            ))
            .child(div().flex_none().max_w(px(260.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(ui_text(title)))
            .child(div().flex_1())
            .children(search)
            .child(
                div().id("layout-btn").relative().no_drag().h(px(32.)).px(px(8.)).rounded(px(8.)).flex().flex_none().items_center().gap(px(6.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed()))
                    .when(self.picker, |d| d.bg(Theme::fill_hover_strong()))
                    .child(icon("layout-grid", 18., Theme::icon()))
                    .child(div().whitespace_nowrap().text_size(px(13.)).child(match l.even() { Some((r, c)) => format!("{r} × {c}"), None => l.label() }))
                    .child({ let b = self.layout_btn.clone(); canvas(move |bounds, _, _| b.set(bounds), |_, _, _, _| {}).absolute().size_full() })
                    .tip(tr(cx, "布局", "Layout"), Some("Ctrl+Shift+G"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_picker(cx))),
            )
            .child(
                header_icon("maximize-pane", if self.maximized.is_some() { "minimize-2" } else { "maximize-2" }, 32., 17.).ml(px(2.))
                    .tip(if self.maximized.is_some() { tr(cx, "还原宫格", "Restore grid") } else { tr(cx, "最大化当前窗格", "Maximize pane") }, Some("Ctrl+Shift+M"))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_maximize(window, cx))),
            )
            .child(header_icon("theme", theme_icon, 32., 17.).ml(px(2.)).tip(theme_tip, None).on_click(cx.listener(|this, _, window, cx| this.cycle_theme(window, cx))))
            .children(controls)
            .when(!has_controls, |d| d.pr(px(8.)))
    }

    fn sidebar_w(&self, cx: &App) -> f32 { state::shared(cx).config.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX) }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        let c = &mut cx.global_mut::<Shared>().config;
        c.sidebar_open = !c.sidebar_open;
        self.schedule_save(cx);
        cx.notify();
    }

    fn toggle_maximize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.maximized = if self.maximized.is_some() { None } else { Some(self.active) };
        self.set_active(self.active, window, cx);
    }

    fn close_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.about = None;
        let p = self.panes.get(self.active).cloned();
        if let Some(p) = p { p.update(cx, |p, _| p.focus.clone()).focus(window) }
        cx.notify();
    }

    /// About: name, version, and the licence texts compiled into the exe
    /// (MIT attribution travels with grids.exe alone).
    fn render_about(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        const LICENSE: &str = include_str!("../LICENSE");
        const THIRD_PARTY: &str = include_str!("../THIRD_PARTY.md");
        let tab = self.about?;
        let tab_btn = |id: &'static str, label: &'static str, n: u8| {
            div().id(id).h(px(28.)).px(px(10.)).rounded(px(7.)).flex().items_center().cursor_pointer()
                .text_color(if tab == n { Theme::text() } else { Theme::text_caption() })
                .when(tab == n, |d| d.bg(Theme::fill_hover_strong()))
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| { this.about = Some(n); cx.notify() }))
        };
        let text = if tab == 0 { LICENSE } else { THIRD_PARTY };
        let panel = popover()
            .id("about")
            .w(px(640.))
            // A definite height so the text area (flex_1) scrolls instead of
            // growing to its content.
            .h((window.viewport_size().height * 0.85).min(px(640.)))
            .flex()
            .flex_col()
            .text_size(px(theme::UI_SIZE))
            .occlude()
            .track_focus(&self.about_focus)
            .key_context("About")
            .on_action(cx.listener(|this, _: &CloseAbout, window, cx| this.close_about(window, cx)))
            .child(
                div().flex().items_center().gap(px(12.)).p(px(20.)).pb(px(12.))
                    .child(img("icon/grids-48.png").size(px(40.)).flex_none())
                    .child(div().flex_1().flex().flex_col().gap(px(2.))
                        .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(format!("Grids {}", env!("CARGO_PKG_VERSION"))))
                        .child(div().text_color(Theme::text_caption()).child(tr(cx, "多窗格文件管理器 · MIT 许可证", "Multi-pane file manager · MIT licence"))))
                    .child(header_icon("about-close", "x", 28., 16.).tip(tr(cx, "关闭", "Close"), Some("Esc")).on_click(cx.listener(|this, _, window, cx| this.close_about(window, cx)))),
            )
            .child(div().flex().gap(px(4.)).px(px(16.)).pb(px(8.))
                .child(tab_btn("about-license", tr(cx, "许可证", "Licence"), 0))
                .child(tab_btn("about-third", tr(cx, "第三方声明", "Third-party notices"), 1)))
            .child(div().h(px(1.)).bg(Theme::line_divider()))
            .child(
                div().id(("about-text", tab as usize)).flex_1().min_h(px(120.)).overflow_y_scroll().px(px(20.)).py(px(12.))
                    .font_family(theme::MONO_FONT).text_size(px(12.)).line_height(px(18.)).text_color(Theme::text())
                    .children(text.lines().map(|l| div().min_h(px(18.)).child(SharedString::from(l.to_string())))),
            );
        Some(
            deferred(
                div().id("about-scrim").absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(0.18)).occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_about(window, cx)))
                    .child(panel),
            )
            .with_priority(4)
            .into_any_element(),
        )
    }

    // ---- version popover and confirmation dialogs

    /// "v0.8.0" at the bottom left (the sidebar's foot, or the status bar's
    /// start while the sidebar is hidden); opens the version popover.
    fn version_label(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let b = self.version_btn.clone();
        div().id("version").relative().h(px(24.)).px(px(8.)).rounded(px(6.)).flex().flex_none().items_center().cursor_pointer()
            .text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).whitespace_nowrap()
            .hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed()))
            .when(self.version_pop, |d| d.bg(Theme::fill_hover_strong()).text_color(Theme::text()))
            .child(concat!("v", env!("CARGO_PKG_VERSION")))
            .child(canvas(move |bounds, _, _| b.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .on_click(cx.listener(|this, _, window, cx| {
                this.version_pop = !this.version_pop;
                if this.version_pop { this.dialog_focus.focus(window) } else { this.refocus(window, cx) }
                cx.notify();
            }))
    }

    /// Focus back to the active pane after a popover or dialog closes.
    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let p = self.panes.get(self.active).cloned();
        if let Some(p) = p { p.update(cx, |p, _| p.focus.clone()).focus(window) }
        cx.notify();
    }

    fn close_version_pop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.version_pop = false;
        self.refocus(window, cx);
    }

    /// The version popover: version and build, the licences, the project
    /// address as plain text (Grids never goes online).
    fn render_version_pop(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.version_pop { return None }
        let btn = self.version_btn.get();
        let vh = window.viewport_size().height;
        let row = |label: &'static str, value: String| {
            div().h(px(24.)).flex().items_center().gap(px(12.))
                .child(div().w(px(64.)).flex_none().text_color(Theme::text_caption()).child(label))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(value))
        };
        let platform = format!("{} {}", match std::env::consts::OS { "windows" => "Windows", "linux" => "Linux", "macos" => "macOS", o => o }, std::env::consts::ARCH);
        let panel = div()
            .id("version-pop")
            .absolute()
            .left(btn.left())
            .bottom(vh - btn.top() + px(6.))
            .w(px(288.))
            .py(px(4.))
            .flex()
            .flex_col()
            .bg(theme::raised(Theme::popover_bg()))
            .border_1()
            .border_color(Theme::line_border())
            .rounded(px(8.))
            .shadow(theme::shadow_menu())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .text_color(Theme::text())
            .occlude()
            .track_focus(&self.dialog_focus)
            .key_context("Dialog")
            .on_action(cx.listener(|this, _: &DialogCancel, window, cx| this.close_version_pop(window, cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div().flex().items_center().gap(px(10.)).px(px(14.)).pt(px(10.)).pb(px(8.))
                    .child(img("icon/grids-48.png").size(px(32.)).flex_none())
                    .child(div().flex().flex_col()
                        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Grids"))
                        .child(div().text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(tr(cx, "多窗格文件管理器", "Multi-pane file manager")))),
            )
            .child(
                div().px(px(14.)).pb(px(6.)).text_size(px(theme::UI_SMALL)).flex().flex_col()
                    .child(row(tr(cx, "版本", "Version"), env!("CARGO_PKG_VERSION").to_string()))
                    .child(row(tr(cx, "提交", "Commit"), env!("GRIDS_GIT_HASH").to_string()))
                    .child(row(tr(cx, "构建日期", "Built"), env!("GRIDS_BUILD_DATE").to_string()))
                    .child(row(tr(cx, "平台", "Platform"), platform))
                    .child(row(tr(cx, "项目", "Project"), "github.com/Joyi-code/grids".to_string())),
            )
            .child(div().h(px(9.)).flex().items_center().mx(px(12.)).child(div().h(px(1.)).flex_1().bg(Theme::line_divider())))
            .child(
                div().id("version-licences").h(px(30.)).mx(px(4.)).pl(px(8.)).pr(px(10.)).rounded(px(6.)).flex().items_center().cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover_strong())).active(|s| s.bg(Theme::fill_pressed()))
                    .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().mr(px(8.)).child(icon("file-text", 16., Theme::icon())))
                    .child(div().flex_1().child(tr(cx, "许可证与第三方声明", "Licence and third-party notices")))
                    .child(icon("chevron-right", 14., Theme::icon_muted()))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.version_pop = false;
                        this.about = Some(0);
                        this.about_focus.focus(window);
                        cx.notify();
                    })),
            )
            .child(div().px(px(14.)).pt(px(4.)).pb(px(8.)).text_size(px(11.)).text_color(Theme::text_placeholder()).child(tr(cx, "Grids 完全离线运行，不连接网络。", "Grids runs fully offline and never connects to the network.")));
        Some(
            deferred(
                div().id("version-pop-scrim").absolute().inset_0().occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_version_pop(window, cx)))
                    .child(panel),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn ask_delete_session(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        let body = if en { format!("Delete the session “{name}”? This can't be undone.") } else { format!("确定删除会话「{name}」？此操作无法撤销。") };
        self.confirm = Some(Confirm {
            title: tr(cx, "删除会话", "Delete session").into(),
            body: body.into(),
            ok: tr(cx, "删除", "Delete").into(),
            action: Rc::new(move |this, _, cx| {
                cx.global_mut::<Shared>().config.delete_session(&name);
                this.schedule_save(cx);
            }),
            focus: 0,
        });
        self.dialog_focus.focus(window);
        cx.notify();
    }

    fn close_confirm(&mut self, accept: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.confirm.take() else { return };
        if accept { (c.action)(self, window, cx) }
        self.refocus(window, cx);
    }

    /// The confirmation card: centred, 360 wide, on a dim scrim (a click on
    /// it cancels); the menus' surface, hairline, radius and shadow.
    fn render_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.confirm.as_ref()?;
        let button = |id: &'static str, label: SharedString, danger: bool, focused: bool| {
            div().id(id).h(px(30.)).min_w(px(76.)).px(px(14.)).rounded(px(7.)).flex().items_center().justify_center().cursor_pointer()
                .border_2().border_color(if focused { Theme::focus_ring() } else { gpui::transparent_black() })
                .map(|d| if danger {
                    d.bg(Theme::danger()).text_color(gpui::white()).font_weight(FontWeight::MEDIUM).hover(|s| s.bg(Theme::danger_hover()))
                } else {
                    d.bg(Theme::fill_hover_strong()).text_color(Theme::text()).hover(|s| s.bg(Theme::fill_pressed()))
                })
                .child(label)
        };
        let card = div()
            .id("confirm")
            .w(px(360.))
            .p(px(20.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .bg(theme::raised(Theme::popover_bg()))
            .border_1()
            .border_color(Theme::line_border())
            .rounded(px(8.))
            .shadow(theme::shadow_menu())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .text_color(Theme::text())
            .occlude()
            .track_focus(&self.dialog_focus)
            .key_context("Dialog")
            .on_action(cx.listener(|this, _: &DialogCancel, window, cx| this.close_confirm(false, window, cx)))
            .on_action(cx.listener(|this, _: &DialogAccept, window, cx| {
                let accept = this.confirm.as_ref().is_some_and(|c| c.focus == 1);
                this.close_confirm(accept, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DialogNext, _, cx| {
                if let Some(c) = &mut this.confirm { c.focus ^= 1 }
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(c.title.clone()))
            .child(div().mt(px(8.)).line_height(px(20.)).text_color(Theme::text_caption()).child(ui_text(c.body.clone())))
            .child(
                div().mt(px(20.)).flex().justify_end().gap(px(8.))
                    .child(button("confirm-cancel", tr(cx, "取消", "Cancel").into(), false, c.focus == 0)
                        .on_click(cx.listener(|this, _, window, cx| this.close_confirm(false, window, cx))))
                    .child(button("confirm-ok", c.ok.clone(), true, c.focus == 1)
                        .on_click(cx.listener(|this, _, window, cx| this.close_confirm(true, window, cx)))),
            );
        Some(
            deferred(
                div().id("confirm-scrim").absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(0.18)).occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_confirm(false, window, cx)))
                    .child(card)
                    .with_animation("confirm-fade", Animation::new(Duration::from_millis(110)), |d, t| d.opacity(t)),
            )
            .with_priority(5)
            .into_any_element(),
        )
    }

    fn render_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.picker { return None }
        let btn = self.layout_btn.get();
        let en = state::en(cx);
        let count_label = |cols: &[usize]| {
            let n: usize = cols.iter().sum();
            if en { format!("{} · {n} pane{}", layout::label(cols), if n > 1 { "s" } else { "" }) } else { format!("{} · {n} 格", layout::label(cols)) }
        };
        let cur = self.layout.clone();
        let mut presets = div().flex().flex_wrap().gap(px(4.)).w(px(292.));
        for (k, p) in layout::PRESETS.iter().enumerate() {
            let on = cur.same_shape(p);
            let n: usize = p.iter().sum();
            let even = p.iter().all(|&x| x == p[0]);
            let label = if even { if en { format!("{n} pane{}", if n > 1 { "s" } else { "" }) } else { format!("{n} 宫格") } } else { layout::label(p) };
            presets = presets.child(
                div().id(("preset", k)).w(px(68.)).py(px(8.)).rounded(px(8.)).flex().flex_col().items_center().gap(px(5.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover()))
                    .when(on, |d| d.bg(Theme::row_selected()))
                    .child(mini_layout(p, on).w(px(40.)).h(px(28.)))
                    .child(div().text_size(px(theme::UI_SMALL)).text_color(if on { Theme::text() } else { Theme::text_caption() }).child(label))
                    .on_click(cx.listener(move |this, _, window, cx| this.apply_layout(Layout::columns(p), window, cx))),
            );
        }
        // By column: four small bars of four cells; click or press and drag
        // across them to set each column's panes (the strip under a bar is 0).
        let draft = self.col_draft;
        let mut bars = div().id("col-editor").flex().gap(px(6.)).items_end()
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| this.finish_col_drag(window, cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, window, cx| this.finish_col_drag(window, cx)));
        for c in 0..4 {
            let mut bar = div().flex().flex_col().items_center().gap(px(3.));
            for level in (0..=4usize).rev() {
                let lit = level > 0 && level <= draft[c];
                let cell = div().id(("colcell", c * 5 + level)).w(px(30.)).cursor_pointer().rounded(px(3.))
                    .map(|d| if level == 0 {
                        d.h(px(8.)).bg(if draft[c] == 0 { Theme::primary().opacity(0.35) } else { Theme::fill_hover_strong() })
                    } else {
                        d.h(px(15.)).border_1()
                            .border_color(if lit { Theme::primary() } else { Theme::line_border() })
                            .bg(if lit { Theme::primary().opacity(0.8) } else { Theme::bg_body() })
                    })
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        let d = &mut this.col_draft[c];
                        // Pressing the top lit cell again takes one off.
                        *d = if *d == level && level > 0 { level - 1 } else { level };
                        this.col_dragging = true;
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                        if *on && this.col_dragging && this.col_draft[c] != level { this.col_draft[c] = level; cx.notify() }
                    }));
                bar = bar.child(cell);
            }
            bar = bar.child(div().text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(draft[c].to_string()));
            bars = bars.child(bar);
        }
        let shape = Layout::columns(&draft).cols;
        let section = |t: &'static str| div().px(px(4.)).pb(px(6.)).text_size(px(theme::UI_SMALL)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(t);
        let panel = popover()
            .p(px(12.))
            .flex()
            .flex_col()
            .text_size(px(theme::UI_SIZE))
            .occlude()
            .child(section(tr(cx, "预设", "Presets")))
            .child(presets)
            .child(div().my(px(10.)).h(px(1.)).bg(Theme::line_divider()))
            .child(section(tr(cx, "按列自定义", "By column")))
            .child(
                div().flex().items_center().gap(px(18.)).px(px(4.))
                    .child(bars)
                    .child(div().flex().flex_col().gap(px(8.))
                        .child(mini_layout(&shape, true).w(px(84.)).h(px(56.)))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(count_label(&shape)))),
            )
            .child(div().pt(px(10.)).px(px(4.)).w(px(292.)).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption())
                .child(tr(cx, "点击或按住拖动设置每列格数（0 = 无此列）；已有窗格保留路径", "Click or drag to set panes per column (0 = no column); existing panes keep their folders")));
        Some(
            deferred(
                anchored().anchor(Corner::TopRight).position(btn.top_right() + point(px(0.), btn.size.height + px(6.))).snap_to_window_with_margin(px(8.))
                    .child(div().on_mouse_down_out(cx.listener(|this, _, _, cx| { this.picker = false; cx.notify() })).child(panel)),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn finish_col_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.col_dragging { return }
        self.col_dragging = false;
        if self.col_draft.iter().all(|&n| n == 0) { self.col_draft[0] = 1 }
        self.apply_layout_keep_picker(Layout::columns(&self.col_draft), window, cx);
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cfg = &state::shared(cx).config;
        let width = cfg.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        let current = self.active_pane().map(|p| p.read(cx).state.path.clone());
        let en = state::en(cx);
        let section = |t: &'static str| div().h(px(SB_SECTION)).pt(px(6.)).px(px(SB_X + 8.)).flex().items_center().text_size(px(theme::UI_SIZE)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::sidebar_section()).child(t);
        let item = |id: SharedString, glyph: &'static str, color: Hsla, label: String, path: &PathBuf| {
            let on = current.as_ref() == Some(path);
            div().id(id).group("side-item").relative().h(px(SB_ROW)).mx(px(SB_X)).mb(px(1.)).pl(px(6.)).pr(px(4.)).rounded(px(SB_R)).flex().items_center().cursor_pointer()
                .text_color(Theme::sidebar_text())
                .map(|d| if on { d.bg(Theme::sidebar_selected()) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
                .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().mr(px(9.)).child(icon(glyph, SB_ICON, color)))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(label)))
        };
        let mut col = div().id("sidebar-scroll").flex().flex_col().pb(px(10.)).overflow_y_scroll().flex_1().min_h_0();

        col = col.child(section(tr(cx, "快速访问", "Quick access")));
        for (i, k) in self.known.iter().enumerate() {
            let (glyph, label) = match k.kind {
                KnownFolderKind::Home => ("house", tr(cx, "主文件夹", "Home")),
                KnownFolderKind::Desktop => ("monitor", tr(cx, "桌面", "Desktop")),
                KnownFolderKind::Downloads => ("download", tr(cx, "下载", "Downloads")),
                KnownFolderKind::Documents => ("file-text", tr(cx, "文档", "Documents")),
                KnownFolderKind::Pictures => ("image", tr(cx, "图片", "Pictures")),
            };
            let p = k.path.clone();
            col = col.child(item(format!("known-{i}").into(), glyph, Theme::icon(), label.into(), &k.path).on_click(cx.listener(move |this, _, window, cx| this.go_to(p.clone(), window, cx))));
        }

        col = col.child(div().h(px(16. - 6.))).child(section(tr(cx, "收藏夹", "Favourites")));
        if cfg.favourites.is_empty() {
            col = col.child(div().mx(px(SB_X)).px(px(8.)).py(px(4.)).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(tr(cx, "点击窗格标题栏的星标收藏文件夹", "Star a pane's folder to add it here")));
        }
        for (i, f) in cfg.favourites.iter().enumerate() {
            let p = f.clone();
            let rm = f.clone();
            col = col.child(
                item(format!("fav-{i}").into(), "folder", Theme::icon(), grids_core::paths::display_name(f), f)
                    .tip(&f.to_string_lossy(), None)
                    .on_click(cx.listener(move |this, _, window, cx| this.go_to(p.clone(), window, cx)))
                    .child(
                        div().id(("fav-x", i)).invisible().group_hover("side-item", |s| s.visible()).size(px(22.)).rounded(px(6.)).flex().items_center().justify_center().hover(|s| s.bg(Theme::fill_hover_strong()))
                            .child(icon("x", 14., Theme::icon()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                cx.global_mut::<Shared>().config.toggle_favourite(&rm);
                                for p in this.panes.clone() { p.update(cx, |_, cx| cx.notify()) }
                                this.schedule_save(cx);
                                cx.notify();
                            })),
                    ),
            );
        }

        col = col.child(div().h(px(16. - 6.))).child(section(tr(cx, "此电脑", "This PC")));
        for (i, d) in self.drives.iter().enumerate() {
            let p = d.path.clone();
            let root = d.path.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
            let label = if d.label.is_empty() { if root.is_empty() { "/".to_string() } else { root } } else if root.is_empty() { d.label.clone() } else { format!("{} ({root})", d.label) };
            col = col.child(item(format!("drive-{i}").into(), "hard-drive", Theme::icon(), label, &d.path).on_click(cx.listener(move |this, _, window, cx| this.go_to(p.clone(), window, cx))));
        }

        col = col.child(
            div().mt(px(16. - 6.)).flex().items_center().pr(px(SB_X + 4.))
                .child(div().flex_1().child(section(tr(cx, "会话", "Sessions"))))
                .child(header_icon("save-session", "plus", 22., 16.).rounded(px(6.)).mt(px(6.)).tip(tr(cx, "保存当前会话", "Save current session"), None).on_click(cx.listener(|this, _, window, cx| this.start_save_session(window, cx)))),
        );
        if let Some((input, _)) = &self.session_input {
            col = col.child(div().mx(px(SB_X)).h(px(SB_ROW)).px(px(8.)).rounded(px(SB_R)).flex().items_center().bg(Theme::bg_body()).border_1().border_color(Theme::field_edge()).child(div().flex_1().min_w_0().child(input.clone())));
        }
        if cfg.sessions.is_empty() && self.session_input.is_none() {
            col = col.child(div().mx(px(SB_X)).px(px(8.)).py(px(4.)).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(tr(cx, "点 + 把当前布局和路径存为会话", "Click + to save the layout and folders")));
        }
        let current_name = cfg.session_name.clone();
        for (i, s) in cfg.sessions.iter().enumerate() {
            let name = s.name.clone();
            let del = s.name.clone();
            let session = s.session.clone();
            let on = current_name.as_deref() == Some(&s.name);
            let n = s.session.layout.count();
            col = col.child(
                div().id(("session", i)).group("side-item").h(px(SB_ROW)).mx(px(SB_X)).mb(px(1.)).pl(px(6.)).pr(px(8.)).rounded(px(SB_R)).flex().items_center().cursor_pointer()
                    .text_color(Theme::sidebar_text())
                    .map(|d| if on { d.bg(Theme::sidebar_selected()) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
                    .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().mr(px(9.)).child(icon("layers", SB_ICON, Theme::icon())))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(name.clone())))
                    .child(div().group_hover("side-item", |s| s.invisible()).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(if en { format!("{n}") } else { format!("{n} 格") }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.global_mut::<Shared>().config.session_name = Some(name.clone());
                        this.load_session(session.clone(), window, cx);
                        this.schedule_save(cx);
                    }))
                    .child(
                        div().id(("session-x", i)).absolute().right(px(4.)).invisible().group_hover("side-item", |s| s.visible()).size(px(22.)).rounded(px(6.)).flex().items_center().justify_center().hover(|s| s.bg(Theme::fill_hover_strong()))
                            .child(icon("x", 14., Theme::icon()))
                            .tip(tr(cx, "删除会话", "Delete session"), None)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.ask_delete_session(del.clone(), window, cx);
                            })),
                    )
                    .relative(),
            );
        }

        div()
            .relative()
            .w(px(width))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .text_size(px(theme::UI_SIZE))
            .child(
                // The sidebar's own header row: the toggle, level with the plane's title.
                div().h(px(HEADER_H)).flex_none().flex().items_center().pl(px(16.)).title_bar()
                    .child(header_icon("toggle-sidebar", "panel-left-close", 28., 17.).tip(tr(cx, "收起侧栏", "Hide sidebar"), Some("Ctrl+B")).on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))))
                    .child(div().ml(px(8.)).flex().items_center().gap(px(6.)).text_color(Theme::sidebar_text()).child(img("icon/grids-48.png").size(px(20.)).flex_none()).child(div().font_weight(FontWeight::SEMIBOLD).child("Grids"))),
            )
            .child(col)
            .child(div().flex_none().h(px(40.)).px(px(SB_X)).flex().items_center().child(self.version_label(cx)))
            .child(
                div().id("sidebar-resize").absolute().right(px(-3.)).top_0().h_full().w(px(6.)).cursor(CursorStyle::ResizeColumn)
                    .on_drag(SidebarDrag, |d, _, _, cx| cx.new(|_| d.clone())),
            )
    }

    /// The panes edge to edge on the white plane, 1px hairlines between them;
    /// the splitters' grab areas lie over the hairlines, drawn after the panes.
    fn render_grid(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.grid_bounds.clone();
        let mut grid = div().relative().flex_1().min_w_0().min_h_0().flex()
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full());
        if let Some(m) = self.maximized.and_then(|m| self.panes.get(m)) {
            return grid.child(div().flex_1().min_w_0().min_h_0().child(m.clone()));
        }
        let l = &self.layout;
        let line = || div().flex_none().bg(Theme::line_divider());
        for (c, &n) in l.cols.iter().enumerate() {
            let mut col = div().flex().flex_col().min_w_0().min_h_0().flex_basis(px(0.)).map(|mut d| { d.style().flex_grow = Some(l.col_weights[c]); d });
            for r in 0..n {
                let Some(p) = self.panes.get(l.index(c, r)) else { continue };
                col = col.child(div().min_w_0().min_h_0().flex_basis(px(0.)).map(|mut d| { d.style().flex_grow = Some(l.row_weights[c][r]); d }).child(p.clone()));
                if r + 1 < n { col = col.child(line().h(px(1.)).w_full()) }
            }
            grid = grid.child(col);
            if c + 1 < l.cols.len() { grid = grid.child(line().w(px(1.)).h_full()) }
        }
        let b = self.grid_bounds.get();
        let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
        if w > 1. {
            for i in 0..l.cols.len() - 1 { grid = grid.child(self.splitter(None, i, l.col_split(i) * w, (0., h), cx)) }
            for (c, &n) in l.cols.iter().enumerate() {
                let x0 = if c == 0 { 0. } else { l.col_split(c - 1) * w + GRAB + 1. };
                let x1 = if c + 1 == l.cols.len() { w } else { l.col_split(c) * w - GRAB };
                for i in 0..n - 1 { grid = grid.child(self.splitter(Some(c), i, l.row_split(c, i) * h, (x0, x1 - x0), cx)) }
            }
        }
        grid
    }

    /// The grab area over a hairline: a column divider at `at` px from the
    /// grid's left spanning `span` = (top, height), or a divider inside column
    /// `col` at `at` px from the top spanning (left, width).
    fn splitter(&self, col: Option<usize>, i: usize, at: f32, span: (f32, f32), cx: &mut Context<Self>) -> impl IntoElement {
        let vertical = col.is_none();
        div()
            .id(SharedString::from(match col { None => format!("split-v-{i}"), Some(c) => format!("split-h-{c}-{i}") }))
            .absolute()
            .map(|d| if vertical {
                d.top(px(span.0)).h(px(span.1)).left(px(at - GRAB)).w(px(GRAB * 2. + 1.)).cursor(CursorStyle::ResizeColumn)
            } else {
                d.left(px(span.0)).w(px(span.1)).top(px(at - GRAB)).h(px(GRAB * 2. + 1.)).cursor(CursorStyle::ResizeRow)
            })
            .flex()
            .justify_center()
            .items_center()
            .group("splitter")
            .child(div().map(|d| if vertical { d.w(px(1.)).h_full() } else { d.h(px(1.)).w_full() }).invisible().group_hover("splitter", |s| s.visible()).bg(Theme::split_active()))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.click_count() >= 2 {
                    match col { None => this.layout.equalize_cols(), Some(c) => this.layout.equalize_rows(c) }
                    this.schedule_save(cx);
                    cx.notify();
                }
            }))
            .on_drag(SplitDrag { col, i }, |d, _, _, cx| cx.new(|_| d.clone()))
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = state::lang(cx);
        let en = lang == format::Lang::En;
        let mut left = String::new();
        if let Some(p) = self.active_pane() {
            let st = &p.read(cx).state;
            left = if en { format!("{} items", st.len()) } else { format!("{} 个项目", st.len()) };
            let n = st.selected_count();
            if n > 0 {
                let size = st.selected_size();
                left += &if en { format!("    {n} selected") } else { format!("    已选择 {n} 项") };
                if size > 0 { left += &format!("  {}", format::size(size, lang)) }
            }
            if st.show_hidden { left += if en { "    Showing hidden items" } else { "    显示隐藏项目" } }
        }
        let pane_no = if en { format!("Pane {}", self.active + 1) } else { format!("窗格 {}", self.active + 1) };
        let sidebar_open = state::shared(cx).config.sidebar_open;
        div()
            .h(px(STATUS_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.))
            .pl(px(if sidebar_open { 22. } else { 8. }))
            .pr(px(16.))
            .border_t_1()
            .border_color(Theme::line_divider())
            .text_size(px(theme::UI_SMALL))
            .text_color(Theme::text_caption())
            .when(!sidebar_open, |d| d.child(self.version_label(cx)))
            .child(div().flex_none().whitespace_nowrap().child(pane_no))
            .child(div().flex_none().whitespace_nowrap().child(left))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(Theme::text()).children(self.toast.as_ref().map(|t| ui_text(t.0.clone()))))
            .child(div().flex_none().whitespace_nowrap().child("Ctrl+Tab / Ctrl+1…9"))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in crate::thumbs::take_evicted(cx) { let _ = window.drop_image(image); }
        let sidebar_open = state::shared(cx).config.sidebar_open;
        let decorations = window.window_decorations();
        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(Theme::window())
            .font_family(theme::UI_FONT)
            .text_size(px(theme::UI_SIZE))
            .text_color(Theme::text())
            .when(matches!(decorations, Decorations::Client { .. }) && cfg!(target_os = "linux"), |d| d.rounded(px(10.)).border_1().border_color(Theme::line_border()))
            .on_action(cx.listener(|this, _: &NextPane, window, cx| { let n = this.panes.len(); this.set_active((this.active + 1) % n.max(1), window, cx) }))
            .on_action(cx.listener(|this, _: &PrevPane, window, cx| { let n = this.panes.len().max(1); this.set_active((this.active + n - 1) % n, window, cx) }))
            .on_action(cx.listener(|this, _: &Pane1, w, cx| this.set_active(0, w, cx)))
            .on_action(cx.listener(|this, _: &Pane2, w, cx| this.set_active(1, w, cx)))
            .on_action(cx.listener(|this, _: &Pane3, w, cx| this.set_active(2, w, cx)))
            .on_action(cx.listener(|this, _: &Pane4, w, cx| this.set_active(3, w, cx)))
            .on_action(cx.listener(|this, _: &Pane5, w, cx| this.set_active(4, w, cx)))
            .on_action(cx.listener(|this, _: &Pane6, w, cx| this.set_active(5, w, cx)))
            .on_action(cx.listener(|this, _: &Pane7, w, cx| this.set_active(6, w, cx)))
            .on_action(cx.listener(|this, _: &Pane8, w, cx| this.set_active(7, w, cx)))
            .on_action(cx.listener(|this, _: &Pane9, w, cx| this.set_active(8, w, cx)))
            .on_action(cx.listener(|this, _: &ToggleMaximize, window, cx| this.toggle_maximize(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &PickLayout, _, cx| this.toggle_picker(cx)))
            .on_action(cx.listener(|this, _: &SwapLeft, w, cx| this.swap_pane(-1, 0, w, cx)))
            .on_action(cx.listener(|this, _: &SwapRight, w, cx| this.swap_pane(1, 0, w, cx)))
            .on_action(cx.listener(|this, _: &SwapUp, w, cx| this.swap_pane(0, -1, w, cx)))
            .on_action(cx.listener(|this, _: &SwapDown, w, cx| this.swap_pane(0, 1, w, cx)))
            .on_action(cx.listener(|this, _: &CycleTheme, window, cx| this.cycle_theme(window, cx)))
            .on_drag_move::<SplitDrag>(cx.listener(|this, ev: &DragMoveEvent<SplitDrag>, _, cx| {
                let d = ev.drag(cx).clone();
                let b = this.grid_bounds.get();
                let pos = ev.event.position;
                match d.col {
                    None => {
                        let w = f32::from(b.size.width).max(1.);
                        let at = (f32::from(pos.x - b.origin.x) / w).clamp(0., 1.);
                        let delta = at - this.layout.col_split(d.i);
                        this.layout.drag_col(d.i, delta);
                    }
                    Some(c) => {
                        let h = f32::from(b.size.height).max(1.);
                        let at = (f32::from(pos.y - b.origin.y) / h).clamp(0., 1.);
                        let delta = at - this.layout.row_split(c, d.i);
                        this.layout.drag_row(c, d.i, delta);
                    }
                }
                this.schedule_save(cx);
                cx.notify();
            }))
            .on_drag_move::<SidebarDrag>(cx.listener(|this, ev: &DragMoveEvent<SidebarDrag>, _, cx| {
                let x = f32::from(ev.event.position.x);
                cx.global_mut::<Shared>().config.sidebar_width = x.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                this.schedule_save(cx);
                cx.notify();
            }))
            .when(sidebar_open, |d| d.child(self.render_sidebar(cx)))
            .child(
                // The white plane, flush with the window's top, right and bottom.
                div().flex_1().min_w_0().h_full().flex().flex_col().bg(Theme::bg_body())
                    .child(self.render_topbar(window, cx))
                    .child(self.render_grid(cx))
                    .child(self.render_status(cx)),
            )
            .children(self.render_picker(cx))
            .children(self.render_version_pop(window, cx))
            .children(self.render_about(window, cx))
            .children(self.render_confirm(cx))
    }
}

/// A small picture of a layout: one bar of cells per column.
fn mini_layout(cols: &[usize], on: bool) -> Div {
    let mut d = div().flex().gap(px(2.));
    for &n in cols {
        let mut col = div().flex_1().flex().flex_col().gap(px(2.));
        for _ in 0..n {
            col = col.child(div().flex_1().rounded(px(2.)).bg(if on { Theme::primary() } else { Theme::icon_muted() }).opacity(if on { 0.9 } else { 0.55 }));
        }
        d = d.child(col);
    }
    d
}
