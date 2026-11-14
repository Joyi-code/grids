//! The window: title bar, sidebar, the grid of panes with its splitters, the
//! layout picker and the status bar. Owns the panes and saves the session.

use crate::input::{InputEvent, TextInput};
use crate::ops::{self, Op};
use crate::pane::{DraggedFiles, PaneEvent, PaneView};
use crate::state::{self, Shared, tr};
use crate::theme::{self, Theme};
use crate::ui::{HEADER_H, TipExt, TitleBarExt, header_icon, icon, popover, ui_text, window_controls};
use gpui::{prelude::FluentBuilder, *};
use grids_core::format;
use grids_core::layout::{self, Layout};
use grids_core::session::{Appearance, PaneSave, Session};
use grids_core::version;
use grids_shell::KnownFolderKind;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

mod link;
mod lockers;
pub use lockers::Then as LockersThen;
mod palette;
mod today;
mod history;

actions!(
    workspace,
    [NextPane, PrevPane, Pane1, Pane2, Pane3, Pane4, Pane5, Pane6, Pane7, Pane8, Pane9, ToggleMaximize, ToggleSidebar, PickLayout, CycleTheme, CloseAbout, SwapLeft, SwapRight, SwapUp, SwapDown, DialogCancel, DialogAccept, DialogNext, CommandPalette, OpenToday, UndoLast, OpenHistory]
);

pub fn bind_keys(cx: &mut App) {
    palette::bind_keys(cx);
    today::bind_keys(cx);
    history::bind_keys(cx);
    let c = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("secondary-k", CommandPalette, c),
        KeyBinding::new("secondary-z", UndoLast, c),
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
const SHELF_H: f32 = 40.;
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
    /// Names listed under the body (at most five) and the rest's count line.
    names: Vec<SharedString>,
    more: Option<SharedString>,
    /// The action button is red (deletes); otherwise the accent colour.
    danger: bool,
    action: Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>,
    /// The focused button: 0 Cancel (the default), 1 the action.
    focus: u8,
}

/// The folder-note editor card.
struct NoteEdit {
    folder: PathBuf,
    input: Entity<TextInput>,
    had: bool,
    _subs: Vec<Subscription>,
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
    /// The read-only folder behind the current toast: it offers 解锁.
    toast_unlock: Option<PathBuf>,
    save_task: Option<Task<()>>,
    session_input: Option<(Entity<TextInput>, Vec<Subscription>)>,
    drives: Vec<grids_shell::Drive>,
    known: Vec<grids_shell::KnownFolder>,
    focus: FocusHandle,
    /// Shelf entries as last seen on disk: Some(is_dir), None missing.
    /// Checked on a background thread after changes.
    shelf_stat: HashMap<PathBuf, Option<bool>>,
    _shelf_check: Option<Task<()>>,
    /// A shelf copy / move is running.
    shelf_busy: bool,
    note_edit: Option<NoteEdit>,
    palette: Option<palette::Palette>,
    today: Option<today::Today>,
    lockers: Option<lockers::LockersDlg>,
    history: Option<history::HistoryPanel>,
    /// What the update check found; drives the sidebar pill and the popover.
    update: UpdateState,
    /// Theme packs found in `themes/` beside the settings, plus the active one.
    themes: Vec<(String, PathBuf)>,
    _subs: Vec<Subscription>,
}

/// Where the update check stands.
#[derive(Clone, PartialEq)]
enum UpdateState {
    Idle,
    Checking,
    Current,
    Newer(version::Release),
    Downloading,
    Failed,
}

/// The one address the app asks about versions, and where its releases live.
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/Joyi-code/grids/releases/latest";
const PROJECT_PAGE: &str = "https://github.com/Joyi-code/grids";

fn home_dir() -> PathBuf {
    grids_shell::known_folders().into_iter().find(|k| k.kind == KnownFolderKind::Home).map(|k| k.path).unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }))
}

/// The pack's window backdrop, when it ships one.
fn theme_backdrop() -> Option<Img> {
    let name = if theme::dark() && theme::has("background-dark.png") { "background-dark.png" } else if theme::has("background.png") { "background.png" } else { return None };
    Some(img(SharedString::from(format!("theme/{name}"))).absolute().inset_0().object_fit(ObjectFit::Cover))
}

/// The pack's little sidebar illustration, above the version label.
fn sidebar_foot() -> Option<Img> {
    let name = if theme::dark() && theme::has("sidebar-foot-dark.png") { "sidebar-foot-dark.png" } else if theme::has("sidebar-foot.png") { "sidebar-foot.png" } else { return None };
    Some(img(SharedString::from(format!("theme/{name}"))).w(px(150.)).h(px(75.)).object_fit(ObjectFit::Contain).mb(px(2.)))
}

/// Theme packs in `themes/` beside the settings file, plus the active one when
/// it lives somewhere else.
fn theme_packs(cx: &App) -> Vec<(String, PathBuf)> {
    let shared = state::shared(cx);
    let mut out = shared.config_path.parent().map(|d| grids_core::theme_pack::list(&d.join("themes"))).unwrap_or_default();
    if let Some(dir) = &shared.config.theme && !out.iter().any(|(_, p)| p == dir) {
        let name = grids_core::theme_pack::load(dir).map(|p| p.name).unwrap_or_else(|_| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        out.push((name, dir.clone()));
    }
    out
}

/// The newest release tag when it beats this build: `Ok(None)` means up to
/// date, `Err` that the request could not be made (offline, no curl, a
/// rate-limited GitHub). Only the GET leaves the machine.
fn fetch_latest() -> Result<Option<version::Release>, ()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("curl.exe")
            .args(["-sS", "--fail", "--max-time", "8", "-H", "User-Agent: PaneBoard-update-check", LATEST_RELEASE_API])
            .stdin(std::process::Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: no console flashes
            .output()
            .map_err(|_| ())?;
        if !out.status.success() { return Err(()) }
        let body = String::from_utf8_lossy(&out.stdout);
        let release = version::parse_release(&body).ok_or(())?;
        Ok(version::is_newer(&release.version, env!("CARGO_PKG_VERSION")).then_some(release))
    }
    #[cfg(not(windows))]
    { Err(()) }
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
            toast_unlock: None,
            save_task: None,
            session_input: None,
            drives: grids_shell::drives(),
            known: grids_shell::known_folders(),
            focus: cx.focus_handle(),
            shelf_stat: HashMap::new(),
            _shelf_check: None,
            shelf_busy: false,
            note_edit: None,
            palette: None,
            today: None,
            lockers: None,
            history: None,
            update: UpdateState::Idle,
            themes: theme_packs(cx),
            _subs: vec![],
        };
        me.panes = session.panes.iter().enumerate().map(|(i, p)| me.make_pane(i, p, window, cx)).collect();
        me.set_active(me.active, window, cx);
        cx.global_mut::<Shared>().config.current.links = session.links.clone();
        me.sync_links(cx);
        me.recheck_shelf(cx);
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
        if state::shared(cx).config.check_updates { me.check_update(cx) }
        me
    }

    fn make_pane(&mut self, i: usize, save: &PaneSave, window: &mut Window, cx: &mut Context<Self>) -> Entity<PaneView> {
        let pane = cx.new(|cx| PaneView::new(i, save, window, cx));
        let sub = cx.subscribe_in(&pane, window, |this, pane, ev: &PaneEvent, window, cx| match ev {
            PaneEvent::Activated => {
                if let Some(i) = this.panes.iter().position(|p| p == pane) && i != this.active { this.set_active_only(i, cx) }
            }
            PaneEvent::Changed => { this.schedule_save(cx); this.recheck_shelf(cx) }
            PaneEvent::FsChanged(dirs) => this.fs_changed(dirs, cx),
            PaneEvent::DiskChanged(dir) => this.today_disk_changed(dir, cx),
            PaneEvent::Moved(path) => if let Some(i) = this.panes.iter().position(|p| p == pane) { this.link_moved(i, path, cx) },
            PaneEvent::Listed => if !state::shared(cx).config.current.links.is_empty() { this.sync_links(cx) },
            PaneEvent::Lockers { paths, then } => this.check_lockers(paths.clone(), then.clone(), window, cx),
            PaneEvent::Link(c) => if let Some(i) = this.panes.iter().position(|p| p == pane) { this.link_cmd(i, c, cx) },
            PaneEvent::Toast(msg) => this.show_toast(msg.clone(), cx),
            PaneEvent::AskDelete { paths, permanent } => this.ask_delete_files(pane.clone(), paths.clone(), *permanent, window, cx),
            PaneEvent::ToggleMaximize => {
                if let Some(i) = this.panes.iter().position(|p| p == pane) { this.active = i }
                this.toggle_maximize(window, cx)
            }
            PaneEvent::RestoreGrid => if this.maximized.is_some() { this.toggle_maximize(window, cx) },
            PaneEvent::Locked(root) => this.show_locked(root.clone(), cx),
            PaneEvent::EditNote(folder) => this.edit_note(folder.clone(), window, cx),
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
            // Focusing a pane clears its +N badge.
            p.update(cx, |p, cx| if p.active != on || (on && p.unseen > 0) { p.active = on; if on { p.unseen = 0 } cx.notify() });
        }
        if self.maximized.is_some() { self.maximized = Some(self.active) }
        self.sync_max(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn set_active(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.set_active_only(i, cx);
        if let Some(p) = self.panes.get(self.active) { window.focus(&p.read(cx).focus) }
    }

    fn active_pane(&self) -> Option<&Entity<PaneView>> { self.panes.get(self.active) }

    /// After an operation changed `dirs`: panes showing them, the shelf and
    /// the Today view catch up.
    fn fs_changed(&mut self, dirs: &[PathBuf], cx: &mut Context<Self>) {
        for p in self.panes.clone() { p.update(cx, |p, cx| p.refresh_if_showing(dirs, cx)) }
        self.recheck_shelf(cx);
        for d in dirs { self.today_disk_changed(d, cx) }
    }

    fn show_toast(&mut self, msg: String, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some((msg, seq));
        self.toast_unlock = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |this, cx| if this.toast.as_ref().is_some_and(|t| t.1 == seq) { this.toast = None; cx.notify() });
        }).detach();
    }

    /// An edit was refused in a read-only folder: say so, offer to unlock.
    fn show_locked(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let msg = if state::en(cx) { format!("This folder is read-only (inside {} only)", state::brand(cx)) } else { format!("此文件夹已设为只读（仅在{}内生效）", state::brand(cx)) };
        self.show_toast(msg, cx);
        self.toast_unlock = Some(root);
    }

    fn unlock(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let locked = &mut cx.global_mut::<Shared>().config.locked;
        locked.retain(|l| l != &root);
        self.toast = None;
        self.toast_unlock = None;
        self.schedule_save(cx);
        window.refresh();
    }

    // ---- sessions and saving

    fn session(&self, cx: &App) -> Session {
        Session { layout: self.layout.clone(), panes: self.panes.iter().map(|p| p.read(cx).save()).collect(), active: self.active, ..state::shared(cx).config.current.clone() }
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
        // The Today view covers the grid: leave it, or the switch looks dead.
        self.leave_today(window, cx);
        self.layout = session.layout.clone();
        self.maximized = None;
        self.sync_max(cx);
        self.panes.clear();
        self._subs.retain(|_| false);
        self._subs.push(cx.observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)));
        self.panes = session.panes.iter().enumerate().map(|(i, p)| self.make_pane(i, p, window, cx)).collect();
        cx.global_mut::<Shared>().config.current.links = session.links.clone();
        self.sync_links(cx);
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
        self.remap_links(|k| Some(if k == i { j } else if k == j { i } else { k }), cx);
        self.maximized = None;
        self.sync_max(cx);
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
        let back: Vec<Option<usize>> = carry.clone();
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
        self.remap_links(|old| back.iter().position(|&f| f == Some(old)), cx);
        self.maximized = None;
        self.sync_max(cx);
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
        // The Today view covers the grid: leave it, or the click looks dead.
        self.leave_today(window, cx);
        if let Some(p) = self.active_pane().cloned() {
            p.update(cx, |p, cx| p.navigate(path, cx));
            window.focus(&p.read(cx).focus);
        }
        cx.notify();
    }

    // ---- rendering

    /// The top of the white plane, as quiet-md's Home: the title (session
    /// name), a filter field for the active pane in the middle, and icon
    /// buttons at the top right before the window controls.
    fn render_topbar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let en = state::en(cx);
        let cfg = &state::shared(cx).config;
        let sidebar_open = cfg.sidebar_open;
        let title = cfg.session_name.clone().unwrap_or_else(|| state::brand(cx).into());
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

    /// Tells each pane whether it fills the grid (its title bar shows it).
    fn sync_max(&mut self, cx: &mut Context<Self>) {
        let n = self.panes.len();
        for (j, p) in self.panes.iter().enumerate() {
            let m = (self.maximized == Some(j)).then_some(n);
            p.update(cx, |p, cx| if p.maximized != m { p.maximized = m; cx.notify() });
        }
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
        const LICENSE_ZH: &str = include_str!("../LICENSE.zh.md");
        const THIRD_PARTY: &str = include_str!("../THIRD_PARTY.md");
        const THIRD_PARTY_ZH: &str = include_str!("../THIRD_PARTY.zh.md");
        let tab = self.about?;
        let tab_btn = |id: &'static str, label: &'static str, n: u8| {
            div().id(id).h(px(28.)).px(px(10.)).rounded(px(7.)).flex().items_center().cursor_pointer()
                .text_color(if tab == n { Theme::text() } else { Theme::text_caption() })
                .when(tab == n, |d| d.bg(Theme::fill_hover_strong()))
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| { this.about = Some(n); cx.notify() }))
        };
        let en = state::en(cx);
        let text = match (tab, en) { (0, true) => LICENSE, (0, false) => LICENSE_ZH, (_, true) => THIRD_PARTY, (_, false) => THIRD_PARTY_ZH };
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
                        .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(format!("{} {}", state::brand(cx), env!("CARGO_PKG_VERSION"))))
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

    /// Asks GitHub for the newest release tag, once. The request is made by the
    /// system's curl.exe, so grids.exe links no networking code, and the only
    /// thing sent is that GET: no files, no folder names, no counters.
    fn check_update(&mut self, cx: &mut Context<Self>) {
        if self.update == UpdateState::Checking { return }
        self.update = UpdateState::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            // Let the window settle first: the check is not urgent.
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let found = cx.background_executor().spawn(async { fetch_latest() }).await;
            let _ = this.update(cx, |this, cx| {
                this.update = match found {
                    Ok(Some(v)) => UpdateState::Newer(v),
                    Ok(None) => UpdateState::Current,
                    Err(()) => UpdateState::Failed,
                };
                cx.notify();
            });
        }).detach();
    }

    /// "新版本 0.9.1" above the version label: one click downloads and swaps it in.
    fn update_pill(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let label = match &self.update {
            UpdateState::Newer(r) => if state::en(cx) { format!("Update to {}", r.version) } else { format!("更新到 {}", r.version) },
            UpdateState::Downloading => if state::en(cx) { "Downloading…".to_string() } else { "正在下载…".to_string() },
            _ => return None,
        };
        let busy = self.update == UpdateState::Downloading;
        Some(
            div().id("update-pill").h(px(28.)).px(px(8.)).rounded(px(7.)).flex().items_center().gap(px(6.)).cursor_pointer()
                .bg(Theme::primary().alpha(0.12)).text_color(Theme::primary()).text_size(px(12.)).font_weight(FontWeight::MEDIUM)
                .hover(|s| s.bg(Theme::primary().alpha(0.2)))
                .child(icon("download", 14., Theme::primary()))
                .child(label)
                .tip(tr(cx, "下载并重启到新版本", "Download and restart into the new version"), None)
                .when(!busy, |d| d.on_click(cx.listener(|this, _, _, cx| this.start_update(cx)))),
        )
    }

    /// The version popover's "updates" row: a status, clickable when a check
    /// or a download makes sense.
    fn update_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let en = state::en(cx);
        let (text, link) = match &self.update {
            UpdateState::Idle => (if en { "Check for updates".to_string() } else { "检查更新".to_string() }, false),
            UpdateState::Checking => (if en { "Checking…".to_string() } else { "正在检查…".to_string() }, false),
            UpdateState::Current => (if en { "Up to date".to_string() } else { "已是最新版本".to_string() }, false),
            UpdateState::Newer(r) => (if en { format!("Update to {}", r.version) } else { format!("更新到 {}", r.version) }, true),
            UpdateState::Downloading => (if en { "Downloading…".to_string() } else { "正在下载…".to_string() }, false),
            UpdateState::Failed => (if en { "Check failed, click to retry".to_string() } else { "检查失败，点击重试".to_string() }, true),
        };
        let retry = self.update == UpdateState::Idle || self.update == UpdateState::Failed;
        let hot = link || retry;
        div().id("update-row").h(px(24.)).flex().items_center().gap(px(12.))
            .when(hot, |d| d.cursor_pointer())
            .child(div().w(px(64.)).flex_none().text_color(Theme::text_caption()).child(tr(cx, "更新", "Updates")))
            .child(
                div().flex_1().min_w_0().flex().items_center().gap(px(6.))
                    .text_color(if link { Theme::primary() } else { Theme::text() })
                    .when(link, |d| d.child(icon("download", 13., Theme::primary())))
                    .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(text)),
            )
            .when(link, |d| d.on_click(cx.listener(|this, _, _, cx| this.start_update(cx))))
            .when(retry, |d| d.on_click(cx.listener(|this, _, _, cx| this.check_update(cx))))
    }

    /// Whether to look for a newer release at startup.
    fn check_updates_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let on = state::shared(cx).config.check_updates;
        div().id("check-updates").h(px(24.)).flex().items_center().gap(px(12.)).cursor_pointer()
            .child(div().w(px(64.)).flex_none().text_color(Theme::text_caption()).child(tr(cx, "启动时检查", "Check at startup")))
            .child(
                div().flex_1().flex().items_center().gap(px(6.)).text_color(if on { Theme::text() } else { Theme::text_caption() })
                    .child(icon(if on { "check" } else { "minus" }, 13., if on { Theme::primary() } else { Theme::icon_muted() }))
                    .child(if on { tr(cx, "开启", "On") } else { tr(cx, "关闭", "Off") }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                let on = state::shared(cx).config.check_updates;
                cx.global_mut::<Shared>().config.check_updates = !on;
                this.schedule_save(cx);
                if !on { this.update = UpdateState::Idle }
                cx.notify();
            }))
    }

    /// One row per theme pack, the built-in look first.
    fn theme_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = state::shared(cx).config.theme.clone();
        let mut rows = vec![self.theme_row(tr(cx, "默认", "Default").to_string(), None, current.is_none(), cx)];
        for (name, path) in self.themes.clone() {
            let on = current.as_ref() == Some(&path);
            rows.push(self.theme_row(name, Some(path), on, cx));
        }
        rows
    }

    fn theme_row(&self, name: String, path: Option<PathBuf>, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = SharedString::from(format!("theme-{name}"));
        div().id(id).h(px(26.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(8.)).cursor_pointer()
            .hover(|s| s.bg(Theme::fill_hover()))
            .when(selected, |d| d.bg(Theme::fill_hover_strong()))
            .child(icon(if selected { "check" } else { "square" }, 13., if selected { Theme::primary() } else { Theme::icon_muted() }))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(name))
            .on_click(cx.listener(move |this, _, _, cx| this.apply_theme_pack(path.clone(), cx)))
            .into_any_element()
    }

    /// The theme pack supplies a palette for each of these.
    fn appearance_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cur = state::shared(cx).config.appearance;
        let mut row = div().flex().gap(px(4.));
        for (a, zh, en) in [(Appearance::System, "跟随系统", "System"), (Appearance::Light, "浅色", "Light"), (Appearance::Dark, "深色", "Dark")] {
            row = row.child(self.appearance_btn(a, tr(cx, zh, en), cur, cx));
        }
        row
    }

    fn appearance_btn(&self, a: Appearance, label: &'static str, cur: Appearance, cx: &mut Context<Self>) -> AnyElement {
        let id = SharedString::from(format!("appearance-{label}"));
        div().id(id).h(px(24.)).px(px(10.)).rounded(px(6.)).flex().items_center().cursor_pointer().text_size(px(12.))
            .when(cur == a, |d| d.bg(Theme::primary().alpha(0.14)).text_color(Theme::primary()))
            .when(cur != a, |d| d.text_color(Theme::text_caption()).hover(|s| s.bg(Theme::fill_hover())))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.set_appearance(a, window, cx)))
            .into_any_element()
    }

    fn set_appearance(&mut self, a: Appearance, window: &mut Window, cx: &mut Context<Self>) {
        cx.global_mut::<Shared>().config.appearance = a;
        self.apply_theme(window, cx);
        self.schedule_save(cx);
    }

    /// Switches the theme pack and restarts: the icons are chosen at start-up.
    fn apply_theme_pack(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if state::shared(cx).config.theme == path { return }
        cx.global_mut::<Shared>().config.theme = path;
        self.save_now(cx);
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).env_remove("GRIDS_THEME").spawn();
        }
        cx.quit();
    }

    /// Downloads the newer release, checks its hash, then hands the swap to a
    /// copy of this exe and quits, so the window is gone before it happens.
    fn start_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Newer(release) = self.update.clone() else { return };
        let dir = state::shared(cx).config_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        self.update = UpdateState::Downloading;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (rel, dir) = (release.clone(), dir.clone());
            let dest = dir.join(format!("grids-update-{}.exe", rel.version));
            let (dest2, rel2) = (dest.clone(), rel.clone());
            let ok = cx.background_executor().spawn(async move {
                crate::update::download(&rel2.url, &dest2) && crate::update::verify(&dest2, &rel2.digest)
            }).await;
            let _ = this.update(cx, |this, cx| {
                if !ok {
                    this.update = UpdateState::Newer(release);
                    this.show_toast(tr(cx, "下载或校验失败，请稍后重试", "Download or check failed, try again later").into(), cx);
                    return;
                }
                let exe = std::env::current_exe().unwrap_or_default();
                if crate::update::start_helper(&dest, &exe) { cx.quit() } else {
                    this.update = UpdateState::Newer(release);
                    this.show_toast(tr(cx, "无法启动更新程序", "Could not start the updater").into(), cx);
                }
            });
        }).detach();
    }

    /// "v0.9.0" at the bottom left (the sidebar's foot, or the status bar's
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
            .child({
                // A theme pack's banner replaces the icon-and-name header.
                let banner = if theme::dark() && theme::has("about-dark.png") { Some("theme/about-dark.png") }
                    else if theme::has("about.png") { Some("theme/about.png") } else { None };
                match banner {
                    Some(name) => div().px(px(10.)).pt(px(10.))
                        .child(img(name).w_full().h(px(58.)).object_fit(ObjectFit::Contain))
                        .into_any_element(),
                    None => div().flex().items_center().gap(px(10.)).px(px(14.)).pt(px(10.)).pb(px(8.))
                        .child(img("icon/grids-48.png").size(px(32.)).flex_none())
                        .child(div().flex().flex_col()
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(state::brand(cx)))
                            .child(div().text_size(px(theme::UI_SMALL)).text_color(Theme::text_caption()).child(tr(cx, "多窗格文件管理器", "Multi-pane file manager"))))
                        .into_any_element(),
                }
            })
            .child(
                div().px(px(14.)).pb(px(4.)).text_size(px(theme::UI_SMALL)).flex().flex_col()
                    .child(row(tr(cx, "版本", "Version"), env!("CARGO_PKG_VERSION").to_string())),
            )
            .child(div().px(px(14.)).pt(px(4.)).pb(px(2.)).text_size(px(theme::UI_SMALL)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(tr(cx, "主题", "Theme")))
            .child(div().px(px(8.)).flex().flex_col().gap(px(1.)).children(self.theme_rows(cx)))
            .child(div().px(px(14.)).pt(px(8.)).pb(px(2.)).text_size(px(theme::UI_SMALL)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(tr(cx, "外观", "Appearance")))
            .child(div().px(px(14.)).pb(px(6.)).child(self.appearance_row(cx)))
            .child(
                div().px(px(14.)).pb(px(6.)).text_size(px(theme::UI_SMALL)).flex().flex_col()
                    .child(self.update_row(cx))
                    .child(self.check_updates_toggle(cx)),
            )
            .child(div().h(px(9.)).flex().items_center().mx(px(12.)).child(div().h(px(1.)).flex_1().bg(Theme::line_divider())))
            .child(
                div().id("version-project").h(px(30.)).mx(px(4.)).pl(px(8.)).pr(px(10.)).rounded(px(6.)).flex().items_center().cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover_strong())).active(|s| s.bg(Theme::fill_pressed()))
                    .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().mr(px(8.)).child(icon("github", 15., Theme::icon())))
                    .child(div().flex_1().child(tr(cx, "项目主页与更新说明", "Project page and release notes")))
                    .child(icon("chevron-right", 14., Theme::icon_muted()))
                    .on_click(|_, _, _| { let _ = grids_shell::open_url(PROJECT_PAGE); }),
            )
            .child(div().px(px(14.)).pt(px(6.)).pb(px(8.)).text_size(px(11.)).text_color(Theme::text_placeholder())
                .child(tr(cx, "启动时只查询一次 GitHub 上的最新版本号，不上传任何文件或数据。", "Asks GitHub for the latest version once at startup; uploads no files or data.")));
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
            names: vec![],
            more: None,
            action: Rc::new(move |this, _, cx| {
                cx.global_mut::<Shared>().config.delete_session(&name);
                this.schedule_save(cx);
            }),
            danger: true,
            focus: 0,
        });
        self.dialog_focus.focus(window);
        cx.notify();
    }

    /// Delete / Shift+Delete in a pane: our confirmation, then the delete
    /// without any system prompt. A drive without a recycle bin gets the
    /// permanent wording (that check runs off the UI thread).
    /// Deletes wait until nothing holds the files (see lockers.rs).
    fn ask_delete_files(&mut self, pane: Entity<PaneView>, paths: Vec<PathBuf>, permanent: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ws = cx.entity().downgrade();
        let p2 = paths.clone();
        let then: lockers::Then = Rc::new(move |window, cx| {
            let (pane, paths) = (pane.clone(), p2.clone());
            let _ = ws.update(cx, |ws, cx| ws.ask_delete_unlocked(pane, paths, permanent, window, cx));
        });
        self.check_lockers(paths, Some(then), window, cx);
    }

    fn ask_delete_unlocked(&mut self, pane: Entity<PaneView>, paths: Vec<PathBuf>, permanent: bool, window: &mut Window, cx: &mut Context<Self>) {
        if permanent { return self.show_delete_confirm(pane, paths, true, window, cx) }
        let probe = paths.clone();
        cx.spawn_in(window, async move |this, cx| {
            let nuke = cx.background_executor().spawn(async move { grids_shell::recycle_would_nuke(&probe) }).await;
            let _ = this.update_in(cx, |this, window, cx| this.show_delete_confirm(pane, paths, nuke, window, cx));
        }).detach();
    }

    fn show_delete_confirm(&mut self, pane: Entity<PaneView>, paths: Vec<PathBuf>, permanent: bool, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        let n = paths.len();
        let name = |p: &PathBuf| p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |s| s.to_string_lossy().into_owned());
        let one = (n == 1).then(|| name(&paths[0]));
        let body = match (permanent, &one, en) {
            (false, Some(f), false) => format!("将「{f}」移入回收站？"),
            (false, None, false) => format!("将 {n} 项移入回收站？"),
            (true, Some(f), false) => format!("永久删除「{f}」？删除后无法从回收站恢复。"),
            (true, None, false) => format!("永久删除 {n} 项？删除后无法从回收站恢复。"),
            (false, Some(f), true) => format!("Move “{f}” to the Recycle Bin?"),
            (false, None, true) => format!("Move {n} items to the Recycle Bin?"),
            (true, Some(f), true) => format!("Permanently delete “{f}”? It can't be restored from the Recycle Bin."),
            (true, None, true) => format!("Permanently delete {n} items? They can't be restored from the Recycle Bin."),
        };
        let names: Vec<SharedString> = if n > 1 { paths.iter().take(5).map(|p| name(p).into()).collect() } else { vec![] };
        let more = (n > 5).then(|| SharedString::from(if en { format!("and {} more", n - 5) } else { format!("等 {n} 项") }));
        self.confirm = Some(Confirm {
            title: if permanent { tr(cx, "永久删除", "Delete permanently") } else { tr(cx, "移入回收站", "Move to Recycle Bin") }.into(),
            body: body.into(),
            ok: if permanent { tr(cx, "永久删除", "Delete permanently") } else { tr(cx, "删除", "Delete") }.into(),
            names,
            more,
            action: Rc::new(move |_, _, cx| {
                let paths = paths.clone();
                pane.update(cx, |p, cx| p.delete_confirmed(paths, permanent, cx));
            }),
            danger: true,
            focus: 0,
        });
        self.version_pop = false;
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
                .when(!danger && id == "confirm-ok", |d| d.bg(Theme::primary()).text_color(gpui::white()).font_weight(FontWeight::MEDIUM).hover(|s| s.bg(Theme::primary()).opacity(0.9)))
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
            // plain text: it wraps (UiText is one line)
            .child(div().mt(px(8.)).line_height(px(20.)).text_color(Theme::text_caption()).child(c.body.clone()))
            .when(!c.names.is_empty(), |d| d.child(
                div().mt(px(8.)).px(px(10.)).py(px(6.)).rounded(px(6.)).bg(Theme::fill_hover()).flex().flex_col()
                    .text_size(px(12.)).line_height(px(19.)).text_color(Theme::text())
                    .children(c.names.iter().map(|n| div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(n.clone()))))
                    .children(c.more.clone().map(|m| div().text_color(Theme::text_caption()).child(m))),
            ))
            .child(
                div().mt(px(20.)).flex().justify_end().gap(px(8.))
                    .child(button("confirm-cancel", tr(cx, "取消", "Cancel").into(), false, c.focus == 0)
                        .on_click(cx.listener(|this, _, window, cx| this.close_confirm(false, window, cx))))
                    .child(button("confirm-ok", c.ok.clone(), c.danger, c.focus == 1)
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

        let today_on = self.today.is_some();
        col = col.child(div().h(px(6.))).child(
            div().id("side-today").h(px(SB_ROW)).mx(px(SB_X)).mb(px(1.)).pl(px(6.)).pr(px(4.)).rounded(px(SB_R)).flex().items_center().cursor_pointer()
                .text_color(Theme::sidebar_text())
                .map(|d| if today_on { d.bg(Theme::sidebar_selected()) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
                .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().mr(px(9.)).child(icon("calendar-clock", SB_ICON, Theme::primary())))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(tr(cx, "今日", "Today")))
                .on_click(cx.listener(|this, _, window, cx| if this.today.is_some() { this.close_today(window, cx) } else { this.open_today(window, cx) })),
        );
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
                    .child(div().ml(px(8.)).flex().items_center().gap(px(6.)).text_color(Theme::sidebar_text()).child(img("icon/grids-48.png").size(px(20.)).flex_none()).child(div().font_weight(FontWeight::SEMIBOLD).child(state::brand(cx)))),
            )
            .child(col)
            .child(
                div().flex_none().px(px(SB_X)).pb(px(4.)).flex().flex_col().gap(px(6.))
                    .children(sidebar_foot())
                    .children(self.update_pill(cx))
                    .child(self.version_label(cx)),
            )
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
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
            // The pack's backdrop stays under the panes: its left edge would
            // otherwise draw a line down the sidebar.
            .children(theme_backdrop());
        if let Some(t) = self.render_today(cx) { return grid.child(t) }
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

    // ---- folder notes

    fn edit_note(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let old = grids_core::notes::get(&state::shared(cx).config.notes, &folder).map(str::to_string);
        let text = old.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut t = TextInput::new(&text, tr(cx, "写点什么，例如：报销单据，月底前整理", "e.g. receipts, sort by month end"), cx);
            t.select_range(text.len()..text.len(), cx);
            t
        });
        let subs = vec![cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Submit => this.close_note(Some(false), window, cx),
            InputEvent::Cancel => this.close_note(None, window, cx),
            InputEvent::Changed => cx.notify(),
        })];
        input.read(cx).focus(window);
        self.note_edit = Some(NoteEdit { folder, input, had: old.is_some(), _subs: subs });
        cx.notify();
    }

    /// None cancels; Some(false) saves the text; Some(true) removes the note.
    fn close_note(&mut self, how: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = self.note_edit.take() else { return };
        if let Some(remove) = how {
            let text = if remove { String::new() } else { ed.input.read(cx).text().to_string() };
            let notes = &mut cx.global_mut::<Shared>().config.notes;
            let before = grids_core::notes::get(notes, &ed.folder).map(str::to_string);
            grids_core::notes::set(notes, &ed.folder, &text);
            let after = grids_core::notes::get(notes, &ed.folder).map(str::to_string);
            if before != after {
                self.schedule_save(cx);
                for p in self.panes.clone() { p.update(cx, |_, cx| cx.notify()) }
                let msg = match (after.is_some(), state::en(cx)) {
                    (true, false) => "已保存备注", (true, true) => "Note saved",
                    (false, false) => "已删除备注", (false, true) => "Note removed",
                };
                self.show_toast(msg.into(), cx);
            }
        }
        self.set_active(self.active, window, cx);
        cx.notify();
    }

    fn render_note_edit(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ed = self.note_edit.as_ref()?;
        let en = state::en(cx);
        let t = move |zh: &'static str, e: &'static str| -> SharedString { (if en { e } else { zh }).into() };
        let n = ed.input.read(cx).text().trim().chars().count();
        let max = grids_core::notes::MAX_NOTE_CHARS;
        let name = ed.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| ed.folder.to_string_lossy().into_owned());
        let button = |id: &'static str, label: SharedString, primary: bool| {
            div().id(id).h(px(30.)).min_w(px(76.)).px(px(14.)).rounded(px(7.)).flex().items_center().justify_center().cursor_pointer()
                .map(|d| if primary {
                    d.bg(Theme::primary()).text_color(gpui::white()).font_weight(FontWeight::MEDIUM).hover(|s| s.opacity(0.9))
                } else {
                    d.bg(Theme::fill_hover_strong()).text_color(Theme::text()).hover(|s| s.bg(Theme::fill_pressed()))
                })
                .child(label)
        };
        let card = div()
            .id("note-card")
            .w(px(400.))
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
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().flex().items_center().gap(px(8.))
                .child(icon("sticky-note", 16., Theme::star()))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(if ed.had { t("编辑备注", "Edit note") } else { t("添加备注", "Add note") })))
            .child(div().mt(px(6.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(Theme::text_caption()).child(ui_text(name)))
            .child(div().mt(px(12.)).h(px(32.)).px(px(8.)).rounded(px(7.)).flex().items_center().bg(Theme::bg_body()).border_1().border_color(Theme::field_edge())
                .child(div().flex_1().min_w_0().child(ed.input.clone())))
            .child(div().mt(px(6.)).flex().items_center().gap(px(8.)).text_size(px(11.)).text_color(Theme::text_placeholder())
                .child(div().flex_1().min_w_0().child(t("仅保存在 Grids 中，在其他程序里改名会丢失备注", "Kept in Grids only; renaming elsewhere loses it")))
                .child(div().flex_none().map(|d| if n > max { d.text_color(Theme::danger()) } else { d }).child(format!("{n} / {max}"))))
            .child(
                div().mt(px(16.)).flex().items_center().gap(px(8.))
                    .when(ed.had, |d| d.child(div().id("note-remove").h(px(30.)).px(px(10.)).rounded(px(7.)).flex().items_center().cursor_pointer()
                        .text_color(Theme::danger()).hover(|s| s.bg(Theme::fill_hover()))
                        .child(t("删除备注", "Remove note"))
                        .on_click(cx.listener(|this, _, window, cx| this.close_note(Some(true), window, cx)))))
                    .child(div().flex_1())
                    .child(button("note-cancel", t("取消", "Cancel"), false).on_click(cx.listener(|this, _, window, cx| this.close_note(None, window, cx))))
                    .child(button("note-save", t("保存", "Save"), true).on_click(cx.listener(|this, _, window, cx| this.close_note(Some(false), window, cx)))),
            );
        Some(
            deferred(
                div().id("note-scrim").absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(0.18)).occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_note(None, window, cx)))
                    .child(card)
                    .with_animation("note-fade", Animation::new(Duration::from_millis(110)), |d, t| d.opacity(t)),
            )
            .with_priority(5)
            .into_any_element(),
        )
    }

    // ---- the shelf

    /// Looks at the shelf entries on disk (background), to grey out the
    /// missing ones and pick folder / file icons.
    fn recheck_shelf(&mut self, cx: &mut Context<Self>) {
        let list = state::shared(cx).config.shelf.clone();
        if list.is_empty() {
            if !self.shelf_stat.is_empty() { self.shelf_stat.clear(); cx.notify() }
            return;
        }
        self._shelf_check = Some(cx.spawn(async move |this, cx| {
            let stat = cx.background_executor().spawn(async move {
                list.into_iter().map(|p| { let s = std::fs::metadata(&p).ok().map(|m| m.is_dir()); (p, s) }).collect::<HashMap<_, _>>()
            }).await;
            let _ = this.update(cx, |this, cx| if this.shelf_stat != stat { this.shelf_stat = stat; cx.notify() });
        }));
    }

    fn set_shelf(&mut self, f: impl FnOnce(&mut Vec<PathBuf>), cx: &mut Context<Self>) {
        f(&mut cx.global_mut::<Shared>().config.shelf);
        self.schedule_save(cx);
        self.recheck_shelf(cx);
        cx.notify();
    }

    /// The shelf's present entries, copied or moved into the active pane's
    /// folder in one journalled operation; the shelf empties on success.
    fn shelf_to_pane(&mut self, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        if op == Op::Move {
            let paths: Vec<PathBuf> = state::shared(cx).config.shelf.clone();
            let ws = cx.entity().downgrade();
            let then: lockers::Then = Rc::new(move |_, cx| { let _ = ws.update(cx, |ws, cx| ws.shelf_run(op, cx)); });
            return self.check_lockers(paths, Some(then), window, cx);
        }
        self.shelf_run(op, cx)
    }

    fn shelf_run(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.shelf_busy { return }
        let Some(pane) = self.active_pane().cloned() else { return };
        let dest = pane.read(cx).state.path.clone();
        if let Some(root) = state::locked_root(cx, &dest) { return self.show_locked(root, cx) }
        let paths: Vec<PathBuf> = state::shared(cx).config.shelf.iter().filter(|p| self.shelf_stat.get(*p).is_some_and(Option::is_some)).cloned().collect();
        if paths.is_empty() { return }
        if op == Op::Move && let Some(root) = state::locked_any(cx, &paths) { return self.show_locked(root, cx) }
        let mut dirs: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(PathBuf::from)).collect();
        dirs.push(dest.clone());
        dirs.sort();
        dirs.dedup();
        let idx = Some(self.active);
        let en = state::en(cx);
        self.shelf_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { ops::run(op, &paths, Some(&dest), idx) }).await;
            let _ = this.update(cx, |this, cx| {
                this.shelf_busy = false;
                match res {
                    Ok(items) => {
                        ops::after_op(op, &items, cx);
                        let n = items.len();
                        this.set_shelf(|s| s.clear(), cx);
                        this.show_toast(match (en, op) {
                            (true, Op::Move) => format!("Moved {n} item(s) from the shelf"),
                            (true, _) => format!("Copied {n} item(s) from the shelf"),
                            (false, Op::Move) => format!("已从暂存架移动 {n} 项"),
                            (false, _) => format!("已从暂存架复制 {n} 项"),
                        }, cx);
                    }
                    Err(e) => if e.kind() != std::io::ErrorKind::Interrupted { this.show_toast(format!("{}{e}", if en { "Operation failed: " } else { "操作失败：" }), cx) },
                }
                for p in this.panes.clone() { p.update(cx, |p, cx| p.refresh_if_showing(&dirs, cx)) }
            });
        }).detach();
    }

    fn render_shelf(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shelf = state::shared(cx).config.shelf.clone();
        // While files are dragged, an empty shelf shows up as a drop target.
        let dragging = cx.has_active_drag();
        if shelf.is_empty() && !dragging { return None }
        let en = state::en(cx);
        let t = move |zh: &'static str, e: &'static str| if en { e } else { zh };
        let dest = self.active_pane().map(|p| p.read(cx).state.path.clone());
        let dest_locked = dest.as_ref().is_some_and(|d| state::locked_root(cx, d).is_some());
        let present = shelf.iter().filter(|p| self.shelf_stat.get(*p).is_some_and(Option::is_some)).count();
        let src_locked = state::locked_any(cx, &shelf).is_some();
        let can_copy = present > 0 && !dest_locked && !self.shelf_busy;
        let can_move = can_copy && !src_locked;
        let mut cards = div().id("shelf-cards").flex_1().min_w_0().h_full().flex().items_center().gap(px(6.)).overflow_x_scroll();
        for (i, p) in shelf.iter().enumerate() {
            let name: SharedString = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned()).into();
            let stat = self.shelf_stat.get(p).copied();
            let missing = matches!(stat, Some(None));
            let is_dir = matches!(stat, Some(Some(true)));
            let ext = grids_core::listing::ext_of(&name);
            let kind = if is_dir { grids_core::FileKind::Folder } else { grids_core::FileKind::of_ext(&ext) };
            let glyph = match (!missing).then(|| state::file_icon(p, is_dir, &ext, cx)).flatten() {
                Some(image) => img(image).size(px(16.)).flex_none().into_any_element(),
                None => icon(theme::kind_icon(kind), 16., if missing { Theme::text_disabled() } else { theme::kind_color(kind) }).into_any_element(),
            };
            let tip = if missing { format!("{}{}", p.display(), t("（已不存在）", " (no longer there)")) } else { p.display().to_string() };
            let remove = p.clone();
            let drag = DraggedFiles { paths: vec![p.clone()], from: p.parent().map(PathBuf::from).unwrap_or_default(), label: name.clone() };
            cards = cards.child(
                div().id(("shelf-card", i)).group("shelf-card").flex_none().max_w(px(200.)).h(px(28.)).pl(px(8.)).pr(px(4.)).rounded(px(7.)).flex().items_center().gap(px(6.))
                    .bg(Theme::fill_hover()).border_1().border_color(Theme::line_border())
                    .hover(|s| s.bg(Theme::fill_hover_strong()))
                    .when(!missing, |d| d.cursor_grab().on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone())))
                    .child(glyph)
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.))
                        .map(|d| if missing { d.text_color(Theme::text_disabled()).line_through() } else { d.text_color(Theme::text()) })
                        .child(ui_text(name)))
                    .child(div().id(("shelf-x", i)).flex_none().size(px(18.)).rounded(px(5.)).flex().items_center().justify_center().cursor_pointer()
                        .hover(|s| s.bg(Theme::fill_pressed()))
                        .child(icon("x", 12., Theme::icon_muted()))
                        .tip(t("从暂存架移除", "Remove from shelf"), None)
                        .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); let r = remove.clone(); this.set_shelf(move |s| s.retain(|q| q != &r), cx) })))
                    .tip_label(tip, crate::ui::TipSide::Above),
            );
        }
        if shelf.is_empty() {
            cards = cards.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(t("拖到这里加入暂存架", "Drop here to add to the shelf")));
        }
        let action = |id: &'static str, glyph: &'static str, label: &'static str, enabled: bool| {
            div().id(id).flex_none().h(px(26.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(5.)).text_size(px(12.)).whitespace_nowrap()
                .map(|d| if enabled {
                    d.cursor_pointer().text_color(Theme::text()).hover(|s| s.bg(Theme::fill_hover_strong())).active(|s| s.bg(Theme::fill_pressed()))
                } else { d.text_color(Theme::text_disabled()) })
                .child(icon(glyph, 14., if enabled { Theme::icon() } else { Theme::text_disabled() }))
                .child(label)
        };
        let blocked: Option<&'static str> = if dest_locked { Some(t("当前格子是只读文件夹", "The active pane's folder is read-only")) } else { None };
        let move_tip = blocked.or(src_locked.then(|| t("暂存架上有只读文件夹里的项目，只能复制", "Some items are in a read-only folder: copy only")));
        Some(
            div().id("shelf").h(px(SHELF_H)).flex_none().flex().items_center().gap(px(8.)).pl(px(12.)).pr(px(10.))
                .border_t_1().border_color(Theme::line_divider())
                .map(|d| if dragging { d.bg(Theme::drop_target()) } else { d })
                .on_drop(cx.listener(|this, d: &DraggedFiles, _, cx| { let p = d.paths.clone(); this.set_shelf(move |s| { for x in p { if !s.contains(&x) { s.push(x) } } }, cx) }))
                .on_drop(cx.listener(|this, d: &ExternalPaths, _, cx| { let p = d.paths().to_vec(); this.set_shelf(move |s| { for x in p { if !s.contains(&x) { s.push(x) } } }, cx) }))
                .child(div().flex_none().flex().items_center().gap(px(5.)).text_size(px(12.)).text_color(Theme::text_caption())
                    .child(icon("inbox", 15., Theme::icon_muted()))
                    .child(if shelf.is_empty() { t("暂存架", "Shelf").to_string() } else if en { format!("Shelf · {}", shelf.len()) } else { format!("暂存架 · {}", shelf.len()) }))
                .child(cards)
                .child(action("shelf-copy", "copy", t("复制到当前格子", "Copy here"), can_copy)
                    .when_some(blocked, |d, why| d.tip(why, None))
                    .when(can_copy, |d| d.on_click(cx.listener(|this, _, window, cx| this.shelf_to_pane(Op::Copy, window, cx)))))
                .child(action("shelf-move", "folder-input", t("移动到当前格子", "Move here"), can_move)
                    .when_some(move_tip, |d, why| d.tip(why, None))
                    .when(can_move, |d| d.on_click(cx.listener(|this, _, window, cx| this.shelf_to_pane(Op::Move, window, cx)))))
                .child(action("shelf-clear", "trash-2", t("清空", "Clear"), !shelf.is_empty())
                    .when(!shelf.is_empty(), |d| d.tip(t("清空暂存架（不删除文件）", "Empty the shelf (files stay)"), None).on_click(cx.listener(|this, _, _, cx| this.set_shelf(|s| s.clear(), cx)))))
                .into_any_element(),
        )
    }

    fn render_status(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        // The shortcut hint goes first when the bar gets narrow.
        let bar_w = f32::from(window.viewport_size().width) - if sidebar_open { state::shared(cx).config.sidebar_width } else { 0. };
        let hint = bar_w >= 820.;
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
            .child(div().flex_1().min_w_0().flex().items_center().gap(px(8.))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(Theme::text()).children(self.toast.as_ref().map(|t| ui_text(t.0.clone()))))
                .children(self.toast_unlock.clone().map(|root| {
                    div().id("toast-unlock").flex_none().h(px(22.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(4.)).cursor_pointer()
                        .bg(Theme::fill_hover()).hover(|s| s.bg(Theme::fill_hover_strong())).text_color(Theme::text())
                        .child(icon("lock-open", 12., Theme::icon()))
                        .child(tr(cx, "解锁", "Unlock"))
                        .on_click(cx.listener(move |this, _, window, cx| this.unlock(root.clone(), window, cx)))
                })))
            .child(div().id("status-history").flex_none().h(px(22.)).px(px(6.)).rounded(px(6.)).flex().items_center().gap(px(4.)).cursor_pointer()
                .when(self.history.is_some(), |d| d.bg(Theme::fill_hover_strong()).text_color(Theme::text()))
                .hover(|s| s.bg(Theme::fill_hover()).text_color(Theme::text()))
                .child(icon("history", 13., Theme::icon()))
                .child(tr(cx, "历史", "History"))
                .tip(tr(cx, "操作历史与撤销", "History and undo"), Some("Ctrl+Z"))
                .on_click(cx.listener(|this, _, window, cx| this.open_history(window, cx))))
            .when(hint, |d| d.child(div().flex_none().whitespace_nowrap().child("Ctrl+Tab / Ctrl+1…9")))
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
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| this.open_palette(window, cx)))
            .on_action(cx.listener(|this, _: &OpenToday, window, cx| this.open_today(window, cx)))
            .on_action(cx.listener(|this, _: &UndoLast, window, cx| this.undo_last(window, cx)))
            .on_action(cx.listener(|this, _: &OpenHistory, window, cx| this.open_history(window, cx)))
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
                    .children(self.render_shelf(cx))
                    .child(self.render_status(window, cx)),
            )
            .children(self.render_picker(cx))
            .children(self.render_version_pop(window, cx))
            .children(self.render_about(window, cx))
            .children(self.render_history(cx))
            .children(self.render_confirm(cx))
            .children(self.render_lockers(cx))
            .children(self.render_note_edit(cx))
            .children(self.render_palette(cx))
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




