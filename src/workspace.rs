//! The window: title bar, sidebar, the grid of panes with its splitters, the
//! layout picker and the status bar. Owns the panes and saves the session.

use crate::input::{InputEvent, TextInput};
use crate::pane::{PaneEvent, PaneView};
use crate::state::{self, Shared, tr};
use crate::theme::{self, Theme};
use crate::ui::{HEADER_H, TipExt, TitleBarExt, icon, icon_button, popover, ui_text, window_controls};
use gpui::{prelude::FluentBuilder, *};
use grids_core::format;
use grids_core::layout::Layout;
use grids_core::session::{Appearance, PaneSave, Session};
use grids_shell::KnownFolderKind;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

actions!(
    workspace,
    [NextPane, PrevPane, Pane1, Pane2, Pane3, Pane4, Pane5, Pane6, Pane7, Pane8, Pane9, ToggleMaximize, ToggleSidebar, PickLayout, CycleTheme]
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
    ]);
}

const GAP: f32 = 6.;
const STATUS_H: f32 = 26.;
const PRESETS: [usize; 7] = [1, 2, 4, 6, 9, 12, 16];

/// A grid splitter being dragged: column (`vertical`) or row divider `i`.
#[derive(Clone)]
struct SplitDrag {
    vertical: bool,
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

pub struct Workspace {
    panes: Vec<Entity<PaneView>>,
    active: usize,
    maximized: Option<usize>,
    layout: Layout,
    picker: bool,
    picker_hover: Option<(usize, usize)>,
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
            picker_hover: None,
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
        let sub = cx.subscribe_in(&pane, window, |this, pane, ev: &PaneEvent, _, cx| match ev {
            PaneEvent::Activated => {
                if let Some(i) = this.panes.iter().position(|p| p == pane) && i != this.active { this.set_active_only(i, cx) }
            }
            PaneEvent::Changed => this.schedule_save(cx),
            PaneEvent::FsChanged(dirs) => {
                for p in this.panes.clone() { p.update(cx, |p, cx| p.refresh_if_showing(dirs, cx)) }
            }
            PaneEvent::Toast(msg) => this.show_toast(msg.clone(), cx),
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

    fn apply_layout(&mut self, new: Layout, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = false;
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

    fn render_title(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let en = state::en(cx);
        let cfg = &state::shared(cx).config;
        let sidebar_open = cfg.sidebar_open;
        let session = cfg.session_name.clone();
        let appearance = cfg.appearance;
        let l = &self.layout;
        let layout_label = format!("{} × {}", l.rows, l.cols);
        let theme_icon = match appearance { Appearance::System => "sun-moon", Appearance::Light => "sun", Appearance::Dark => "moon" };
        let theme_tip = match appearance {
            Appearance::System => tr(cx, "外观：跟随系统", "Appearance: system"),
            Appearance::Light => tr(cx, "外观：浅色", "Appearance: light"),
            Appearance::Dark => tr(cx, "外观：深色", "Appearance: dark"),
        };
        div()
            .id("title-bar")
            .h(px(HEADER_H))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(8.))
            .title_bar()
            .child(
                icon_button("toggle-sidebar", if sidebar_open { "panel-left-close" } else { "panel-left" }, 30., 18., true)
                    .no_drag()
                    .tip(if sidebar_open { tr(cx, "隐藏侧栏", "Hide sidebar") } else { tr(cx, "显示侧栏", "Show sidebar") }, Some("Ctrl+B"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(
                div().ml(px(8.)).flex().items_center().gap(px(8.)).min_w_0()
                    .child(icon("layout-grid", 16., Theme::primary()))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child("Grids"))
                    .when_some(session, |d, s| d.child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(Theme::text_caption()).child(ui_text(s)))),
            )
            .child(div().flex_1())
            .child(
                div().id("layout-btn").relative().no_drag().h(px(30.)).px(px(10.)).rounded(px(8.)).flex().items_center().gap(px(6.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed()))
                    .when(self.picker, |d| d.bg(Theme::fill_hover_strong()))
                    .child(icon("layout-grid", 16., Theme::icon()))
                    .child(div().whitespace_nowrap().child(format!("{} {layout_label}", tr(cx, "布局", "Layout"))))
                    .child(icon("chevron-down", 14., Theme::icon_muted()))
                    .child({ let b = self.layout_btn.clone(); canvas(move |bounds, _, _| b.set(bounds), |_, _, _, _| {}).absolute().size_full() })
                    .on_click(cx.listener(|this, _, _, cx| { this.picker = !this.picker; this.picker_hover = None; cx.notify() })),
            )
            .child(
                icon_button("maximize-pane", if self.maximized.is_some() { "minimize-2" } else { "maximize-2" }, 30., 16., true)
                    .no_drag().ml(px(2.))
                    .tip(if self.maximized.is_some() { tr(cx, "还原宫格", "Restore grid") } else { tr(cx, "最大化当前窗格", "Maximize pane") }, Some("Ctrl+Shift+M"))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_maximize(window, cx))),
            )
            .child(icon_button("theme", theme_icon, 30., 16., true).no_drag().ml(px(2.)).tip(theme_tip, None).on_click(cx.listener(|this, _, window, cx| this.cycle_theme(window, cx))))
            .children(window_controls(window, en))
            .when(window_controls(window, en).is_none(), |d| d.pr(px(8.)))
    }

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

    fn render_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.picker { return None }
        let btn = self.layout_btn.get();
        let en = state::en(cx);
        let tile = |n: usize, cur: &Layout| {
            let l = Layout::preset(n);
            let on = cur.rows == l.rows && cur.cols == l.cols;
            let mut grid = div().w(px(40.)).h(px(28.)).flex().flex_col().gap(px(2.));
            for _ in 0..l.rows {
                let mut row = div().flex_1().flex().gap(px(2.));
                for _ in 0..l.cols { row = row.child(div().flex_1().rounded(px(2.)).bg(if on { Theme::primary() } else { Theme::icon_muted() }).opacity(if on { 0.9 } else { 0.55 })) }
                grid = grid.child(row);
            }
            div().id(("preset", n)).w(px(68.)).py(px(8.)).rounded(px(8.)).flex().flex_col().items_center().gap(px(5.)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover()))
                .when(on, |d| d.bg(Theme::row_selected()))
                .child(grid)
                .child(div().text_size(px(theme::UI_SMALL)).text_color(if on { Theme::text() } else { Theme::text_caption() }).child(if en { format!("{n} pane{}", if n > 1 { "s" } else { "" }) } else { format!("{n} 宫格") }))
        };
        let cur = self.layout.clone();
        let mut presets = div().flex().flex_wrap().gap(px(4.)).w(px(292.));
        for n in PRESETS {
            presets = presets.child(tile(n, &cur).on_click(cx.listener(move |this, _, window, cx| this.apply_layout(Layout::preset(n), window, cx))));
        }
        let hover = self.picker_hover;
        let mut custom = div().id("custom-grid").flex().flex_col().gap(px(3.));
        for r in 0..4 {
            let mut row = div().flex().gap(px(3.));
            for c in 0..4 {
                let lit = hover.is_some_and(|(hr, hc)| r <= hr && c <= hc);
                let cur_lit = hover.is_none() && r < cur.rows && c < cur.cols;
                row = row.child(
                    div().id(("cell", r * 4 + c)).size(px(22.)).rounded(px(4.)).border_1().cursor_pointer()
                        .border_color(if lit || cur_lit { Theme::primary() } else { Theme::line_border() })
                        .bg(if lit { Theme::row_selected() } else if cur_lit { Theme::row_selected_inactive() } else { Theme::bg_body() })
                        .on_hover(cx.listener(move |this, on: &bool, _, cx| { if *on { this.picker_hover = Some((r, c)); cx.notify() } }))
                        .on_click(cx.listener(move |this, _, window, cx| this.apply_layout(Layout::new(r + 1, c + 1), window, cx))),
                );
            }
            custom = custom.child(row);
        }
        let (hr, hc) = hover.map_or((cur.rows, cur.cols), |(r, c)| (r + 1, c + 1));
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
            .child(section(tr(cx, "自定义（行 × 列）", "Custom (rows × columns)")))
            .child(
                div().flex().items_center().gap(px(16.)).px(px(4.))
                    .child(custom.on_hover(cx.listener(|this, on: &bool, _, cx| if !*on { this.picker_hover = None; cx.notify() })))
                    .child(div().flex().flex_col().gap(px(4.))
                        .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child(format!("{hr} × {hc}")))
                        .child(div().text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(if en { format!("{} panes", hr * hc) } else { format!("共 {} 个窗格", hr * hc) }))),
            )
            .child(div().pt(px(10.)).px(px(4.)).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(tr(cx, "切换布局时保留已有窗格的路径", "Existing panes keep their folders")));
        Some(
            deferred(
                anchored().anchor(Corner::TopRight).position(btn.top_right() + point(px(0.), btn.size.height + px(6.))).snap_to_window_with_margin(px(8.))
                    .child(div().on_mouse_down_out(cx.listener(|this, _, _, cx| { this.picker = false; cx.notify() })).child(panel)),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cfg = &state::shared(cx).config;
        let width = cfg.sidebar_width.clamp(160., 360.);
        let current = self.active_pane().map(|p| p.read(cx).state.path.clone());
        let en = state::en(cx);
        let section = |t: &'static str| div().h(px(28.)).px(px(10.)).pt(px(8.)).flex().items_center().text_size(px(theme::UI_SMALL)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(t);
        let item = |id: SharedString, glyph: &'static str, color: Hsla, label: String, path: &PathBuf| {
            let on = current.as_ref() == Some(path);
            div().id(id).group("side-item").h(px(28.)).mx(px(6.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(8.)).cursor_pointer()
                .text_color(Theme::sidebar_text())
                .map(|d| if on { d.bg(Theme::sidebar_selected()) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
                .child(icon(glyph, 16., color))
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
            col = col.child(item(format!("known-{i}").into(), glyph, Theme::primary(), label.into(), &k.path).on_click(cx.listener(move |this, _, window, cx| this.go_to(p.clone(), window, cx))));
        }

        col = col.child(section(tr(cx, "收藏夹", "Favourites")));
        if cfg.favourites.is_empty() {
            col = col.child(div().mx(px(6.)).px(px(8.)).py(px(4.)).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(tr(cx, "点击窗格标题栏的星标收藏文件夹", "Star a pane's folder to add it here")));
        }
        for (i, f) in cfg.favourites.iter().enumerate() {
            let p = f.clone();
            let rm = f.clone();
            col = col.child(
                item(format!("fav-{i}").into(), "star-fill", Theme::star(), grids_core::paths::display_name(f), f)
                    .tip(&f.to_string_lossy(), None)
                    .on_click(cx.listener(move |this, _, window, cx| this.go_to(p.clone(), window, cx)))
                    .child(
                        div().id(("fav-x", i)).invisible().group_hover("side-item", |s| s.visible()).size(px(18.)).rounded(px(4.)).flex().items_center().justify_center().hover(|s| s.bg(Theme::fill_hover_strong()))
                            .child(icon("x", 12., Theme::icon_muted()))
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

        col = col.child(section(tr(cx, "此电脑", "This PC")));
        for (i, d) in self.drives.iter().enumerate() {
            let p = d.path.clone();
            let root = d.path.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
            let label = if d.label.is_empty() { if root.is_empty() { "/".to_string() } else { root } } else if root.is_empty() { d.label.clone() } else { format!("{} ({root})", d.label) };
            col = col.child(item(format!("drive-{i}").into(), "hard-drive", Theme::icon(), label, &d.path).on_click(cx.listener(move |this, _, window, cx| this.go_to(p.clone(), window, cx))));
        }

        col = col.child(
            div().flex().items_center().pr(px(8.))
                .child(div().flex_1().child(section(tr(cx, "会话", "Sessions"))))
                .child(icon_button("save-session", "plus", 22., 14., true).mt(px(8.)).tip(tr(cx, "保存当前会话", "Save current session"), None).on_click(cx.listener(|this, _, window, cx| this.start_save_session(window, cx)))),
        );
        if let Some((input, _)) = &self.session_input {
            col = col.child(div().mx(px(6.)).h(px(28.)).px(px(8.)).rounded(px(6.)).flex().items_center().bg(Theme::bg_body()).border_1().border_color(Theme::field_edge()).child(div().flex_1().min_w_0().child(input.clone())));
        }
        if cfg.sessions.is_empty() && self.session_input.is_none() {
            col = col.child(div().mx(px(6.)).px(px(8.)).py(px(4.)).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(tr(cx, "点 + 把当前布局和路径存为会话", "Click + to save the layout and folders")));
        }
        let current_name = cfg.session_name.clone();
        for (i, s) in cfg.sessions.iter().enumerate() {
            let name = s.name.clone();
            let del = s.name.clone();
            let session = s.session.clone();
            let on = current_name.as_deref() == Some(&s.name);
            let n = s.session.layout.count();
            col = col.child(
                div().id(("session", i)).group("side-item").h(px(28.)).mx(px(6.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(8.)).cursor_pointer()
                    .text_color(Theme::sidebar_text())
                    .map(|d| if on { d.bg(Theme::sidebar_selected()) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
                    .child(icon("layers", 16., Theme::icon()))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(name.clone())))
                    .child(div().group_hover("side-item", |s| s.invisible()).text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(if en { format!("{n}") } else { format!("{n} 格") }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.global_mut::<Shared>().config.session_name = Some(name.clone());
                        this.load_session(session.clone(), window, cx);
                        this.schedule_save(cx);
                    }))
                    .child(
                        div().id(("session-x", i)).absolute().right(px(6.)).invisible().group_hover("side-item", |s| s.visible()).size(px(18.)).rounded(px(4.)).flex().items_center().justify_center().hover(|s| s.bg(Theme::fill_hover_strong()))
                            .child(icon("x", 12., Theme::icon_muted()))
                            .tip(tr(cx, "删除会话", "Delete session"), None)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                cx.global_mut::<Shared>().config.delete_session(&del);
                                this.schedule_save(cx);
                                cx.notify();
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
            .child(col)
            .child(
                div().id("sidebar-resize").absolute().right(px(-3.)).top_0().h_full().w(px(6.)).cursor(CursorStyle::ResizeColumn)
                    .on_drag(SidebarDrag, |d, _, _, cx| cx.new(|_| d.clone())),
            )
    }

    fn render_grid(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.grid_bounds.clone();
        let mut grid = div().relative().flex_1().min_w_0().min_h_0().flex().flex_col()
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full());
        if let Some(m) = self.maximized.and_then(|m| self.panes.get(m)) {
            return grid.child(div().flex_1().min_h_0().child(m.clone()));
        }
        let l = &self.layout;
        for r in 0..l.rows {
            let mut row = div().flex().min_h_0().flex_basis(px(0.)).map(|mut d| { d.style().flex_grow = Some(l.row_weights[r]); d });
            for c in 0..l.cols {
                let i = l.index(r, c);
                let Some(p) = self.panes.get(i) else { continue };
                row = row.child(div().min_w_0().min_h_0().flex_basis(px(0.)).map(|mut d| { d.style().flex_grow = Some(l.col_weights[c]); d }).child(p.clone()));
                if c + 1 < l.cols { row = row.child(self.splitter(true, c, cx)) }
            }
            grid = grid.child(row);
            if r + 1 < l.rows { grid = grid.child(self.splitter(false, r, cx)) }
        }
        grid
    }

    fn splitter(&self, vertical: bool, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("split-{}-{i}-{}", if vertical { "v" } else { "h" }, cx.entity_id())))
            .flex_none()
            .map(|d| if vertical { d.w(px(GAP)).h_full().cursor(CursorStyle::ResizeColumn) } else { d.h(px(GAP)).w_full().cursor(CursorStyle::ResizeRow) })
            .flex()
            .items_center()
            .justify_center()
            .group("splitter")
            .child(div().map(|d| if vertical { d.w(px(2.)).h(px(28.)) } else { d.h(px(2.)).w(px(28.)) }).rounded(px(1.)).invisible().group_hover("splitter", |s| s.visible()).bg(Theme::split_active()))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.click_count() >= 2 {
                    if vertical { this.layout.equalize_cols() } else { this.layout.equalize_rows() }
                    this.schedule_save(cx);
                    cx.notify();
                }
            }))
            .on_drag(SplitDrag { vertical, i }, |d, _, _, cx| cx.new(|_| d.clone()))
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
        div()
            .h(px(STATUS_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(12.))
            .text_size(px(theme::UI_SMALL))
            .text_color(Theme::text_caption())
            .child(div().flex_none().whitespace_nowrap().child(pane_no))
            .child(div().flex_none().whitespace_nowrap().child(left))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(Theme::text()).children(self.toast.as_ref().map(|t| ui_text(t.0.clone()))))
            .child(div().flex_none().whitespace_nowrap().child("Ctrl+Tab / Ctrl+1…9"))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_open = state::shared(cx).config.sidebar_open;
        let decorations = window.window_decorations();
        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
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
            .on_action(cx.listener(|this, _: &PickLayout, _, cx| { this.picker = !this.picker; cx.notify() }))
            .on_action(cx.listener(|this, _: &CycleTheme, window, cx| this.cycle_theme(window, cx)))
            .on_drag_move::<SplitDrag>(cx.listener(|this, ev: &DragMoveEvent<SplitDrag>, _, cx| {
                let d = ev.drag(cx).clone();
                let b = this.grid_bounds.get();
                let pos = ev.event.position;
                if d.vertical {
                    let w = f32::from(b.size.width).max(1.);
                    let at = (f32::from(pos.x - b.origin.x) / w).clamp(0., 1.);
                    let delta = at - this.layout.col_split(d.i);
                    this.layout.drag_col(d.i, delta);
                } else {
                    let h = f32::from(b.size.height).max(1.);
                    let at = (f32::from(pos.y - b.origin.y) / h).clamp(0., 1.);
                    let delta = at - this.layout.row_split(d.i);
                    this.layout.drag_row(d.i, delta);
                }
                this.schedule_save(cx);
                cx.notify();
            }))
            .on_drag_move::<SidebarDrag>(cx.listener(|this, ev: &DragMoveEvent<SidebarDrag>, _, cx| {
                let x = f32::from(ev.event.position.x);
                cx.global_mut::<Shared>().config.sidebar_width = x.clamp(160., 360.);
                this.schedule_save(cx);
                cx.notify();
            }))
            .child(self.render_title(window, cx))
            .child(
                div().flex_1().min_h_0().flex().px(px(GAP)).when(sidebar_open, |d| d.pl(px(0.)))
                    .when(sidebar_open, |d| d.child(self.render_sidebar(cx)))
                    .child(self.render_grid(cx)),
            )
            .child(self.render_status(cx))
            .children(self.render_picker(cx))
    }
}
