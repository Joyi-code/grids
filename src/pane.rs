//! One pane of the grid: header (number, folder, back / forward / up, star),
//! breadcrumb address bar, optional name filter, column header and the
//! virtualised file list. Folder reads run on a background thread; the
//! folder shown is watched and reloads when it changes.

use crate::input::{InputEvent, TextInput};
use crate::state::{self, tr};
use crate::theme::{self, Theme};
use crate::menu::{Cmd, ContextMenu as MenuView, Glyph, Item, MenuEvent};
use crate::thumbs;
use crate::ops::{self, Op};
use grids_core::journal::OpKind;
use crate::ui::{TipExt, icon, ui_text};
use futures::StreamExt;
use gpui::{prelude::FluentBuilder, *};
use grids_core::format;
use grids_core::listing::{self, Entry};
use grids_core::paths;
use grids_core::session::{Columns, PaneSave, ViewMode};
use std::collections::{HashMap, HashSet};
use grids_core::{PaneState, SortKey};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

actions!(
    pane,
    [
        Open, GoUp, GoBack, GoForward, Rename, Recycle, DeletePermanently, CopyFiles, CutFiles, PasteFiles, Refresh, NewFolder,
        EditAddress, FilterNames, ToggleHidden, SelectAll, CursorUp, CursorDown, PageUp, PageDown, First, Last, SelectUp,
        SelectDown, SelectPageUp, SelectPageDown, SelectFirst, SelectLast, MoveUp, MoveDown, Escape, ContextMenu,
        CopyPath, Properties, CursorLeft, CursorRight, SelectLeft, SelectRight, ViewHuge, ViewLarge, ViewMedium, ViewList,
        ViewDetails, AddToShelf
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Pane");
    cx.bind_keys([
        KeyBinding::new("enter", Open, c),
        KeyBinding::new("backspace", GoUp, c),
        KeyBinding::new("alt-up", GoUp, c),
        KeyBinding::new("alt-left", GoBack, c),
        KeyBinding::new("alt-right", GoForward, c),
        KeyBinding::new("f2", Rename, c),
        KeyBinding::new("delete", Recycle, c),
        KeyBinding::new("shift-delete", DeletePermanently, c),
        KeyBinding::new("secondary-c", CopyFiles, c),
        KeyBinding::new("secondary-x", CutFiles, c),
        KeyBinding::new("secondary-v", PasteFiles, c),
        KeyBinding::new("f5", Refresh, c),
        KeyBinding::new("f7", NewFolder, c),
        KeyBinding::new("secondary-shift-n", NewFolder, c),
        KeyBinding::new("secondary-l", EditAddress, c),
        KeyBinding::new("alt-d", EditAddress, c),
        KeyBinding::new("secondary-f", FilterNames, c),
        KeyBinding::new("secondary-h", ToggleHidden, c),
        KeyBinding::new("secondary-a", SelectAll, c),
        KeyBinding::new("up", CursorUp, c),
        KeyBinding::new("down", CursorDown, c),
        KeyBinding::new("pageup", PageUp, c),
        KeyBinding::new("pagedown", PageDown, c),
        KeyBinding::new("home", First, c),
        KeyBinding::new("end", Last, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("shift-pageup", SelectPageUp, c),
        KeyBinding::new("shift-pagedown", SelectPageDown, c),
        KeyBinding::new("shift-home", SelectFirst, c),
        KeyBinding::new("shift-end", SelectLast, c),
        KeyBinding::new("secondary-up", MoveUp, c),
        KeyBinding::new("secondary-down", MoveDown, c),
        KeyBinding::new("escape", Escape, c),
        KeyBinding::new("shift-f10", ContextMenu, c),
        KeyBinding::new("menu", ContextMenu, c),
        KeyBinding::new("secondary-shift-c", CopyPath, c),
        KeyBinding::new("secondary-shift-a", AddToShelf, c),
        KeyBinding::new("alt-enter", Properties, c),
        KeyBinding::new("left", CursorLeft, c),
        KeyBinding::new("right", CursorRight, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        // Explorer's view keys: 1 extra large … 4 list (small icons), 5 details.
        KeyBinding::new("secondary-shift-1", ViewHuge, c),
        // Shifted digits arrive as symbols (US layout) on Windows and Linux.
        KeyBinding::new("secondary-!", ViewHuge, c),
        KeyBinding::new("secondary-shift-2", ViewLarge, c),
        KeyBinding::new("secondary-@", ViewLarge, c),
        KeyBinding::new("secondary-shift-3", ViewMedium, c),
        KeyBinding::new("secondary-#", ViewMedium, c),
        KeyBinding::new("secondary-shift-4", ViewList, c),
        KeyBinding::new("secondary-$", ViewList, c),
        KeyBinding::new("secondary-shift-5", ViewDetails, c),
        KeyBinding::new("secondary-%", ViewDetails, c),
    ]);
}

/// Rows match quiet-md's sidebar rows: 30 high, radius 9 pills inset 8.
pub const ROW_H: f32 = 30.;
const ROW_R: f32 = 8.;
const HEAD_H: f32 = 40.;
const FIELD_H: f32 = 30.;
const COLS_H: f32 = 28.;
const ICON: f32 = 18.;
/// Left padding of rows and the column header (the row inset plus its padding).
const ROW_X: f32 = 8.;
const CELL_PAD: f32 = 8.;
const NAME_MIN: f32 = 120.;
/// How long a changed entry stays highlighted.
const FRESH: Duration = Duration::from_secs(4);
/// The wash holds at full strength this long, then fades for the rest of `FRESH`.
const FRESH_HOLD: Duration = Duration::from_secs(2);
/// More changes than this at once are only counted, not highlighted.
const FRESH_MAX: usize = 500;

/// What the workspace hears from a pane.
pub enum PaneEvent {
    /// The pane was clicked or focused: make it the active one.
    Activated,
    /// Something worth saving changed (path, sort, columns, hidden).
    Changed,
    /// Files changed in these folders (after a copy, move, delete …).
    FsChanged(Vec<PathBuf>),
    /// A message for the status bar.
    Toast(String),
    /// Show this folder in the next pane (menu: "Open in next pane").
    OpenInNext(PathBuf),
    /// Delete these (to the recycle bin unless `permanent`) once the user
    /// confirms in the workspace's dialog.
    AskDelete { paths: Vec<PathBuf>, permanent: bool },
    /// Double-click on the title bar: maximize this pane or restore the grid.
    ToggleMaximize,
    /// Esc with nothing else to close while maximized.
    RestoreGrid,
    /// An edit was refused in a read-only folder (the locked root).
    Locked(PathBuf),
    /// Open the note editor for this folder.
    EditNote(PathBuf),
    /// The watcher saw entries change in this folder (the Today view rescans).
    DiskChanged(PathBuf),
    /// The pane went to another folder (linked panes follow).
    Moved(PathBuf),
    /// A listing came in (compare marks are worked out again).
    Listed,
    /// A link-group command from the title menu.
    Link(LinkCmd),
    /// Check who holds `paths`; run `then` once nobody does (None: just
    /// show the result, the menu's 查看占用).
    Lockers { paths: Vec<PathBuf>, then: Option<crate::workspace::LockersThen> },
}

/// What the title menu asks of the link groups.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkCmd {
    /// Link this pane with pane N (0-based).
    With(usize),
    Unlink,
    Compare(bool),
    /// Create the missing counterpart folder and go there.
    CreateMissing,
}

/// How a compared entry differs from the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// No entry of that name over there.
    Only,
    /// Same name, other size or date.
    Differs,
}

/// The pane's place in a link group, set by the workspace.
#[derive(Clone, Default)]
pub struct LinkView {
    /// Group number (colour).
    pub group: usize,
    /// Inside its base folder; false: paused (hollow dot).
    pub live: bool,
    pub compare: bool,
    /// The group has exactly two panes.
    pub can_compare: bool,
    /// The counterpart folder the group wanted to show is missing.
    pub missing: Option<PathBuf>,
    pub marks: Rc<HashMap<String, Mark>>,
}

/// Files dragged from a pane.
#[derive(Clone)]
pub struct DraggedFiles {
    pub paths: Vec<PathBuf>,
    pub from: PathBuf,
    pub label: SharedString,
}

impl Render for DraggedFiles {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let n = self.paths.len();
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(5.))
            .rounded(px(8.))
            .bg(theme::raised(Theme::bg_body()))
            .border_1()
            .border_color(Theme::line_border())
            .shadow(theme::shadow_menu())
            .font_family(theme::UI_FONT)
            .text_size(px(theme::UI_SMALL))
            .text_color(Theme::text())
            .child(icon(if n > 1 { "copy" } else { "file" }, 14., Theme::icon()))
            .child(self.label.clone())
    }
}

/// A column divider being dragged.
#[derive(Clone)]
struct ColResize {
    pane: EntityId,
    col: usize,
}

impl Render for ColResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { Empty }
}

struct RenameState {
    path: PathBuf,
    input: Entity<TextInput>,
    _subs: Vec<Subscription>,
}

/// A rubber-band selection dragged out over the list's empty space.
struct Marquee {
    start: Point<Pixels>,
    cur: Point<Pixels>,
    /// Entries selected before the box started (Ctrl+drag adds to them).
    base: Vec<usize>,
}


pub struct PaneView {
    /// Position in the grid (shown as number + 1).
    pub index: usize,
    pub active: bool,
    pub state: PaneState,
    pub columns: Columns,
    pub view: ViewMode,
    /// Items per row in the icon and list views (1 in details), from the
    /// last layout; the arrow keys and Page Up / Down step by it.
    grid_cols: usize,
    /// Ctrl+wheel travel not yet turned into a view step.
    wheel: Rc<Cell<f32>>,
    pub focus: FocusHandle,
    scroll: UniformListScrollHandle,
    list_bounds: Rc<Cell<Bounds<Pixels>>>,
    header_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Width the breadcrumbs had last frame, to fold them from the left.
    crumbs_w: Rc<Cell<f32>>,
    address: Option<(Entity<TextInput>, Vec<Subscription>)>,
    filter: Option<(Entity<TextInput>, Vec<Subscription>)>,
    rename: Option<RenameState>,
    menu: Option<(Entity<MenuView>, Subscription)>,
    load_seq: u64,
    pub loading: bool,
    _load: Option<Task<()>>,
    _watcher: Option<notify::RecommendedWatcher>,
    _watch_task: Option<Task<()>>,
    type_buf: (String, Instant),
    /// Names to select once the next reload is in (pasted, dropped files).
    pending_select: Option<Vec<String>>,
    /// Entries changed by a live reload, with when they were seen; drawn
    /// with an accent wash fading out over `FRESH`.
    fresh: HashMap<String, Instant>,
    /// Changes counted since the user last touched the pane (+N badge).
    pub unseen: usize,
    /// The last listing and its folder, to diff the next live reload against.
    prev: Vec<Entry>,
    listed: Option<PathBuf>,
    /// `Some(n)`: this pane fills the grid of `n` panes.
    pub maximized: Option<usize>,
    /// A first click on the breadcrumbs waits briefly for a second one
    /// (double-click maximizes) before opening the address field.
    addr_click: Option<Task<()>>,
    /// Rubber-band selection in progress (press on empty space, drag).
    marquee: Option<Marquee>,
    /// The row the current press started on, so a release over another row
    /// (a box selection, say) doesn't collapse the selection.
    press_row: Option<usize>,
    /// Link group membership (None: not linked).
    pub link: Option<LinkView>,
    /// Panes in the grid, for the title menu.
    pub pane_count: usize,
}

impl EventEmitter<PaneEvent> for PaneView {}

impl Focusable for PaneView {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() }
}

impl PaneView {
    pub fn new(index: usize, save: &PaneSave, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut state = PaneState::new(save.path.clone());
        state.sort = save.sort;
        state.ascending = save.ascending;
        state.show_hidden = save.show_hidden;
        let focus = cx.focus_handle();
        cx.on_focus_in(&focus, window, |_, _, cx| cx.emit(PaneEvent::Activated)).detach();
        let mut me = Self {
            index,
            active: false,
            state,
            columns: save.columns.clone(),
            view: save.view,
            grid_cols: 1,
            wheel: Rc::default(),
            focus,
            scroll: UniformListScrollHandle::new(),
            list_bounds: Rc::default(),
            crumbs_w: Rc::default(),
            header_bounds: Rc::default(),
            address: None,
            filter: None,
            rename: None,
            menu: None,
            load_seq: 0,
            loading: false,
            _load: None,
            _watcher: None,
            _watch_task: None,
            type_buf: (String::new(), Instant::now()),
            pending_select: None,
            fresh: HashMap::new(),
            unseen: 0,
            prev: Vec::new(),
            listed: None,
            maximized: None,
            addr_click: None,
            marquee: None,
            press_row: None,
            link: None,
            pane_count: 1,
        };
        me.load(None, cx);
        me
    }

    pub fn save(&self) -> PaneSave {
        PaneSave { path: self.state.path.clone(), sort: self.state.sort, ascending: self.state.ascending, show_hidden: self.state.show_hidden, columns: self.columns.clone(), view: self.view }
    }

    // ---- loading

    /// Reads the folder again on a background thread; `select` picks an
    /// entry by name once it is in.
    pub fn load(&mut self, select: Option<String>, cx: &mut Context<Self>) { self.ack_changes(); self.load_with(select, false, cx) }

    /// The user touched the pane (a click, a key, a reload): the +N badge
    /// starts again from zero.
    fn ack_changes(&mut self) {
        if self.unseen > 0 { self.unseen = 0 }
    }

    /// `live`: a reload for changes on disk, whose new and modified entries
    /// are highlighted (never on entering a folder or a manual refresh).
    fn load_with(&mut self, select: Option<String>, live: bool, cx: &mut Context<Self>) {
        self.load_seq += 1;
        let seq = self.load_seq;
        let path = self.state.path.clone();
        self.loading = true;
        let changed_dir = self._watcher.is_none() || self.state.total() == 0;
        if changed_dir { self.watch(cx) }
        self._load = Some(cx.spawn(async move |this, cx| {
            let p = path.clone();
            let res = cx.background_executor().spawn(async move { listing::read_dir(&p) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.load_seq != seq { return }
                this.loading = false;
                match res {
                    Ok(entries) => {
                        if this.listed.as_deref() != Some(path.as_path()) {
                            // A folder entered successfully: the palette's recent list.
                            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
                            grids_core::frecency::visit(&mut cx.global_mut::<state::Shared>().config.recent, &path, now);
                        }
                        if this.note_changes(&path, live, &entries) { cx.emit(PaneEvent::DiskChanged(path.clone())) }
                        this.state.set_entries(entries, select.as_deref());
                        let mut jump = select.is_some();
                        if let Some(names) = this.pending_select.take() { this.state.select_names(&names); jump = true }
                        if let Some(r) = this.state.cursor.filter(|_| jump) { this.reveal(r, ScrollStrategy::Center) }
                        cx.emit(PaneEvent::Listed);
                    }
                    Err(e) => this.state.set_error(error_text(&e, state::en(cx))),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Diffs a fresh listing of `path` against the previous one: changed
    /// names start fading highlights (up to `FRESH_MAX` of them; more, as
    /// when unpacking an archive, only count) and add to the +N badge.
    /// The badge keeps counting until the user touches the pane.
    /// True when a live reload found changes.
    fn note_changes(&mut self, path: &Path, live: bool, entries: &[Entry]) -> bool {
        let same = self.listed.as_deref() == Some(path);
        if !same { self.fresh.clear() }
        let mut changed = false;
        if live && same {
            let ch = grids_core::diff::diff_entries(&self.prev, entries);
            if !ch.is_empty() {
                changed = true;
                self.unseen += ch.len();
                if ch.len() <= FRESH_MAX {
                    let now = Instant::now();
                    for n in ch.added.into_iter().chain(ch.modified) { self.fresh.insert(n, now); }
                }
            }
        }
        self.prev = entries.to_vec();
        self.listed = Some(path.to_path_buf());
        changed
    }

    /// The accent wash over a changed entry: 30 % held for `FRESH_HOLD`,
    /// then fading to 0 by `FRESH`.
    fn fresh_wash(&self, name: &str) -> Option<Hsla> {
        let secs = self.fresh.get(name)?.elapsed().as_secs_f32();
        let t = ((secs - FRESH_HOLD.as_secs_f32()) / (FRESH.as_secs_f32() - FRESH_HOLD.as_secs_f32())).clamp(0., 1.);
        (t < 1.).then(|| Theme::primary().alpha(0.30 * (1. - t)))
    }

    /// Watches the current folder; changes reload it after a short pause.
    fn watch(&mut self, cx: &mut Context<Self>) {
        use notify::Watcher;
        self._watcher = None;
        self._watch_task = None;
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res && !matches!(ev.kind, notify::EventKind::Access(_)) { let _ = tx.unbounded_send(()); }
        });
        let Ok(mut watcher) = watcher else { return };
        if watcher.watch(&self.state.path, notify::RecursiveMode::NonRecursive).is_err() { return }
        self._watcher = Some(watcher);
        self._watch_task = Some(cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                while rx.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.load_with(None, true, cx)).is_err() { break }
            }
        }));
    }

    // ---- navigation

    pub fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.close_editors();
        if self.state.navigate(path) { self.after_move(None, cx) } else { self.load(None, cx) }
    }

    /// Shows `file`'s folder with `file` selected (Today: 在原格子定位).
    pub fn reveal_path(&mut self, file: &Path, cx: &mut Context<Self>) {
        let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else { return };
        let names = vec![name.to_string_lossy().into_owned()];
        self.close_editors();
        if grids_core::link::same_path(dir, &self.state.path) && !self.loading {
            self.state.select_names(&names);
            if let Some(r) = self.state.cursor { self.reveal(r, ScrollStrategy::Center) }
            cx.notify();
        } else {
            self.pending_select = Some(names);
            self.navigate(dir.to_path_buf(), cx);
        }
    }

    fn after_move(&mut self, select: Option<String>, cx: &mut Context<Self>) {
        self._watcher = None;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.load(select, cx);
        cx.emit(PaneEvent::Changed);
        cx.emit(PaneEvent::Moved(self.state.path.clone()));
    }

    /// The last listing (all entries, unfiltered) and its folder.
    pub fn listing(&self) -> Option<(&Path, &[Entry])> { self.listed.as_deref().map(|p| (p, self.prev.as_slice())) }

    /// Compare tint for an entry.
    fn mark_wash(&self, name: &str) -> Option<Hsla> {
        let l = self.link.as_ref().filter(|l| l.compare)?;
        Some(match l.marks.get(name)? { Mark::Only => Theme::added().alpha(0.16), Mark::Differs => Theme::changed().alpha(0.18) })
    }

    fn close_editors(&mut self) {
        self.address = None;
        self.filter = None;
        self.rename = None;
        self.menu = None;
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        self.close_editors();
        if let Some(name) = self.state.up() { self.after_move(Some(name), cx) }
    }
    fn go_back(&mut self, cx: &mut Context<Self>) {
        self.close_editors();
        if self.state.back() { self.after_move(None, cx) }
    }
    fn go_forward(&mut self, cx: &mut Context<Self>) {
        self.close_editors();
        if self.state.forward() { self.after_move(None, cx) }
    }

    /// Opens the cursor's / selected entries: a folder is entered, files open
    /// in their default programs, shortcuts to folders are entered here.
    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let sel: Vec<Entry> = { let s = self.state.selection(); if s.is_empty() { self.state.cursor_entry().into_iter().cloned().collect() } else { s.into_iter().cloned().collect() } };
        if let [one] = sel.as_slice() && one.is_dir {
            return self.navigate(one.path.clone(), cx);
        }
        let files: Vec<Entry> = sel.into_iter().filter(|e| !e.is_dir).take(32).collect();
        self.open_files(files, cx)
    }

    fn open_row(&mut self, r: usize, cx: &mut Context<Self>) {
        let Some(e) = self.state.row(r).cloned() else { return };
        if e.is_dir { self.navigate(e.path, cx) } else { self.open_files(vec![e], cx) }
    }

    /// Opens files one after another off the UI thread, each with its type's
    /// default program: the shell may run its own message loop (shortcut
    /// resolution, prompts), which must not re-enter GPUI mid-update. A
    /// shortcut is resolved first; a single one to a folder opens in this pane.
    fn open_files(&mut self, files: Vec<Entry>, cx: &mut Context<Self>) {
        if files.is_empty() { return }
        let en = state::en(cx);
        let enter = files.len() == 1;
        cx.spawn(async move |this, cx| {
            let plans = cx.background_executor().spawn(async move {
                files.into_iter().map(|e| {
                    let link = (e.ext == "lnk").then(|| grids_shell::resolve_link(&e.path));
                    let plan = open_plan(&e.path, link, enter);
                    if let OpenPlan::System(p) = &plan && let Err(err) = grids_shell::open_with_system(p) {
                        return OpenPlan::Failed(format!("{}{}: {err}", if en { "Cannot open " } else { "无法打开 " }, e.name));
                    }
                    plan
                }).collect::<Vec<_>>()
            }).await;
            let _ = this.update(cx, |this, cx| for plan in plans { match plan {
                OpenPlan::Enter(dir) => this.navigate(dir, cx),
                OpenPlan::Missing(t) => cx.emit(PaneEvent::Toast(format!("{}{}", if en { "Shortcut target not found: " } else { "快捷方式的目标不存在：" }, t.display()))),
                OpenPlan::Failed(msg) => cx.emit(PaneEvent::Toast(msg)),
                OpenPlan::System(_) => {}
            } });
        }).detach();
    }

    // ---- keyboard

    fn page(&self) -> isize {
        let h = f32::from(self.list_bounds.get().size.height);
        let row_h = grid_metrics(self.view, f32::from(self.list_bounds.get().size.width)).map_or(ROW_H, |g| g.cell_h);
        ((h / row_h).floor() as isize - 1).max(1) * self.grid_cols as isize
    }

    /// Scrolls item `r` into view (its row, in the grid views).
    fn reveal(&self, r: usize, strategy: ScrollStrategy) {
        self.scroll.scroll_to_item(r / self.grid_cols.max(1), strategy)
    }

    fn move_cursor(&mut self, delta: isize, to_end: bool, shift: bool, ctrl: bool, cx: &mut Context<Self>) {
        // A locked folder has nothing to select, by mouse or by keyboard.
        if self.here_locked(cx) { return }
        self.ack_changes();
        self.state.move_cursor(delta, to_end, shift, ctrl);
        if let Some(r) = self.state.cursor { self.reveal(r, ScrollStrategy::Top) }
        cx.notify();
    }

    /// Up / Down: a whole row of items in the grid views.
    fn move_rows(&mut self, rows: isize, shift: bool, ctrl: bool, cx: &mut Context<Self>) {
        let step = rows * self.grid_cols.max(1) as isize;
        // Not past the ends in a grid: stay in the column, as Explorer does.
        if self.grid_cols > 1 && let Some(c) = self.state.cursor {
            let to = c as isize + step;
            if to < 0 || to >= self.state.len() as isize { return }
        }
        self.move_cursor(step, false, shift, ctrl, cx)
    }

    /// Left / Right: the next item in the grid views; nothing in details.
    fn move_side(&mut self, delta: isize, shift: bool, cx: &mut Context<Self>) {
        if self.view == ViewMode::Details { return }
        self.move_cursor(delta, false, shift, false, cx)
    }

    pub fn set_view(&mut self, view: ViewMode, cx: &mut Context<Self>) {
        if self.view == view { return }
        self.view = view;
        if let Some(r) = self.state.cursor {
            self.grid_cols = grid_metrics(view, f32::from(self.list_bounds.get().size.width)).map_or(1, |g| g.cols);
            self.reveal(r, ScrollStrategy::Center);
        }
        cx.emit(PaneEvent::Changed);
        cx.notify();
    }

    fn type_to_select(&mut self, ev: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        // Only when the list itself has focus, not one of the pane's text fields.
        if !self.focus.is_focused(window) || self.here_locked(cx) { return }
        let k = &ev.keystroke;
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform || self.editing() { return }
        let Some(ch) = k.key_char.as_ref().filter(|c| c.chars().all(|c| !c.is_control()) && !c.is_empty()) else { return };
        if self.type_buf.1.elapsed() > Duration::from_millis(900) { self.type_buf.0.clear() }
        self.type_buf.0.push_str(ch);
        self.type_buf.1 = Instant::now();
        if let Some(r) = self.state.find_prefix(&self.type_buf.0) {
            self.state.click(Some(r), false, false);
            self.reveal(r, ScrollStrategy::Top);
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn editing(&self) -> bool { self.address.is_some() || self.rename.is_some() }

    // ---- address bar

    pub fn edit_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let text = self.state.path.to_string_lossy().into_owned();
        let input = cx.new(|cx| TextInput::new(&text, "", cx));
        let subs = vec![
            cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Submit => {
                    let text = input.read(cx).text().to_string();
                    this.address = None;
                    this.go_typed(&text, cx);
                    window.focus(&this.focus);
                }
                InputEvent::Cancel => {
                    this.address = None;
                    window.focus(&this.focus);
                    cx.notify();
                }
                InputEvent::Changed => {}
            }),
            cx.on_focus_out(&input.focus_handle(cx), window, |this, _, _, cx| {
                this.address = None;
                cx.notify();
            }),
        ];
        input.read(cx).focus(window);
        self.address = Some((input, subs));
        cx.notify();
    }

    fn go_typed(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.trim().is_empty() { return cx.notify() }
        let home = grids_shell::known_folders().into_iter().find(|k| k.kind == grids_shell::KnownFolderKind::Home).map(|k| k.path);
        let p = paths::parse_typed(text, &self.state.path, home.as_deref());
        if p.is_dir() {
            self.navigate(p, cx);
        } else if p.is_file() {
            if let Some(parent) = p.parent() { self.navigate(parent.to_path_buf(), cx) }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            self.open_files(vec![Entry { name, path: p, is_dir: false, size: 0, modified: None, hidden: false, ext }], cx);
        } else {
            cx.emit(PaneEvent::Toast(format!("{}{}", tr(cx, "找不到路径：", "Path not found: "), p.display())));
            cx.notify();
        }
    }

    // ---- filter

    pub fn start_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((input, _)) = &self.filter { input.read(cx).focus(window); return }
        let input = cx.new(|cx| TextInput::new(&self.state.filter, tr(cx, "按名称筛选", "Filter by name"), cx));
        let subs = vec![cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => {
                let t = input.read(cx).text().to_string();
                this.state.set_filter(&t);
                this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
            InputEvent::Submit => window.focus(&this.focus),
            InputEvent::Cancel => this.end_filter(window, cx),
        })];
        input.read(cx).focus(window);
        self.filter = Some((input, subs));
        cx.notify();
    }

    fn end_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter = None;
        self.state.set_filter("");
        window.focus(&self.focus);
        cx.notify();
    }

    // ---- rename and new folder

    /// Refuses an edit touching a read-only folder: true when `paths` are
    /// in one (the workspace shows why, with an unlock button).
    fn refuse_locked(&self, paths: &[PathBuf], cx: &mut Context<Self>) -> bool {
        let Some(root) = state::locked_any(cx, paths) else { return false };
        cx.emit(PaneEvent::Locked(root));
        true
    }

    /// This pane's folder is read-only.
    fn here_locked(&self, cx: &App) -> bool { state::locked_root(cx, &self.state.path).is_some() }

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.state.cursor_entry().cloned().or_else(|| self.state.selection().first().map(|e| (*e).clone())) else { return };
        if self.refuse_locked(std::slice::from_ref(&e.path), cx) { return }
        self.state.select_name(&e.name);
        if let Some(r) = self.state.cursor { self.reveal(r, ScrollStrategy::Top) }
        let input = cx.new(|cx| {
            let mut t = TextInput::new(&e.name, "", cx);
            let stem = if e.is_dir { e.name.len() } else { e.name.rfind('.').filter(|&i| i > 0).unwrap_or(e.name.len()) };
            t.select_range(0..stem, cx);
            t
        });
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Submit => { this.commit_rename(cx); window.focus(&this.focus) }
                InputEvent::Cancel => { this.rename = None; window.focus(&this.focus); cx.notify() }
                InputEvent::Changed => {}
            }),
            cx.on_focus_out(&input.focus_handle(cx), window, |this, _, _, cx| this.commit_rename(cx)),
        ];
        input.read(cx).focus(window);
        self.rename = Some(RenameState { path: e.path, input, _subs: subs });
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.rename.take() else { return };
        let name = r.input.read(cx).text().trim().to_string();
        let old = r.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        cx.notify();
        if name == old || name.is_empty() { return }
        if !paths::valid_name(&name) {
            cx.emit(PaneEvent::Toast(tr(cx, "文件名不能包含下列字符：\\ / : * ? \" < > |", "A file name can't contain \\ / : * ? \" < > |").into()));
            return;
        }
        let me = cx.entity().downgrade();
        let (p2, n2) = (r.path.clone(), name.clone());
        let then: crate::workspace::LockersThen = Rc::new(move |_, cx| { let (p, n) = (p2.clone(), n2.clone()); let _ = me.update(cx, |this, cx| this.rename_now(p, n, cx)); });
        cx.emit(PaneEvent::Lockers { paths: vec![r.path.clone()], then: Some(then) });
    }

    fn rename_now(&mut self, path: PathBuf, name: String, cx: &mut Context<Self>) {
        let (pane, en) = (Some(self.index), state::en(cx));
        let to = name.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move {
                let r = grids_shell::rename_tracked(&path, &to);
                if let Ok(items) = &r { ops::log(OpKind::Rename, pane, items) }
                r
            }).await;
            let _ = this.update(cx, |this, cx| match res {
                Ok(items) => {
                    if ops::follow_moves(&items, cx) { cx.emit(PaneEvent::Changed) }
                    this.load(Some(name), cx);
                    cx.emit(PaneEvent::FsChanged(vec![this.state.path.clone()]));
                }
                Err(e) => cx.emit(PaneEvent::Toast(format!("{}{e}", if en { "Rename failed: " } else { "重命名失败：" }))),
            });
        }).detach();
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.refuse_locked(std::slice::from_ref(&self.state.path), cx) { return }
        let base = tr(cx, "新建文件夹", "New folder").to_string();
        let fail = tr(cx, "无法新建文件夹：", "Cannot create folder: ");
        self.create_then_rename(OpKind::New, fail, Box::new(move |dir| {
            let path = dir.join(paths::unique_name(dir, &base));
            std::fs::create_dir(&path)?;
            Ok(vec![grids_shell::OpItem { src: path.clone(), dst: Some(path), is_dir: true }])
        }), window, cx);
    }

    /// Creates something in this folder on a background thread (and
    /// journals it), then selects it with its name ready to edit.
    fn create_then_rename(&mut self, kind: OpKind, fail: &'static str, make: Box<dyn FnOnce(&Path) -> std::io::Result<Vec<grids_shell::OpItem>> + Send>, window: &mut Window, cx: &mut Context<Self>) {
        let (dir, pane) = (self.state.path.clone(), Some(self.index));
        cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_executor().spawn({
                let dir = dir.clone();
                async move {
                    let items = make(&dir)?;
                    ops::log(kind, pane, &items);
                    let name = items.first().and_then(|i| i.src.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    Ok::<_, std::io::Error>((name, listing::read_dir(&dir)))
                }
            }).await;
            let _ = this.update_in(cx, |this, window, cx| match res {
                Ok((name, list)) => {
                    cx.emit(PaneEvent::FsChanged(vec![dir.clone()]));
                    // Add it now so it can be renamed at once; the reload confirms it.
                    if this.state.path == dir && let Ok(list) = list {
                        this.state.set_entries(list, Some(&name));
                        this.start_rename(window, cx);
                    }
                }
                Err(e) => cx.emit(PaneEvent::Toast(format!("{fail}{e}"))),
            });
        }).detach();
    }

    // ---- file operations

    fn copy_files(&mut self, cut: bool, cx: &mut Context<Self>) {
        let t = self.state.targets();
        if t.is_empty() || (cut && self.refuse_locked(&t, cx)) { return }
        let n = t.len();
        state::set_clipboard(t, cut, cx);
        let msg = if state::en(cx) { format!("{} {n} item(s)", if cut { "Cut" } else { "Copied" }) } else { format!("已{} {n} 项", if cut { "剪切" } else { "复制" }) };
        cx.emit(PaneEvent::Toast(msg));
    }

    pub fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dest = self.state.path.clone();
        if self.refuse_locked(std::slice::from_ref(&dest), cx) { return }
        if state::get_clipboard(cx).is_some() { return self.paste_into(dest, cx) }
        self.paste_as_file(window, cx)
    }

    fn paste_into(&mut self, dest: PathBuf, cx: &mut Context<Self>) {
        if self.refuse_locked(std::slice::from_ref(&dest), cx) { return }
        let Some((paths, cut)) = state::get_clipboard(cx) else { return };
        let names: Vec<String> = paths.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        if cut { cx.global_mut::<state::Shared>().clipboard = None }
        self.run_op(if cut { Op::Move } else { Op::Copy }, paths, Some(dest), names, cx);
    }

    /// Ctrl+V with no files on the clipboard: an image becomes
    /// 截图 <date time>.png, text becomes 文本 <date time>.txt (UTF-8), then
    /// the new file is selected with its name ready to edit.
    fn paste_as_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        // Off Windows the shell can't read the clipboard: GPUI's copy instead.
        let fallback: Option<Clip> = if cfg!(windows) { None } else {
            cx.read_from_clipboard().and_then(|item| item.entries().iter().find_map(|e| match e {
                ClipboardEntry::Image(img) if img.format == ImageFormat::Png => Some(Clip::Png(img.bytes.clone())),
                ClipboardEntry::String(s) if !s.text().is_empty() => Some(Clip::Text(s.text().to_string())),
                _ => None,
            }))
        };
        let fail = if en { "Paste failed: " } else { "粘贴失败：" };
        self.create_then_rename(OpKind::PasteFile, fail, Box::new(move |dir| {
            let clip = grids_shell::clipboard_image().map(Clip::Image)
                .or_else(|| grids_shell::clipboard_text().map(Clip::Text))
                .or(fallback)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, if en { "nothing to paste (no files, image or text on the clipboard)" } else { "剪贴板里没有文件、图片或文字" }))?;
            let (name, bytes) = match clip {
                Clip::Image(img) => {
                    let mut out = std::io::Cursor::new(Vec::new());
                    img.write_to(&mut out, image::ImageFormat::Png).map_err(std::io::Error::other)?;
                    (ops::stamp_name(if en { "Screenshot" } else { "截图" }, "png"), out.into_inner())
                }
                Clip::Png(b) => (ops::stamp_name(if en { "Screenshot" } else { "截图" }, "png"), b),
                Clip::Text(t) => (ops::stamp_name(if en { "Text" } else { "文本" }, "txt"), t.into_bytes()),
            };
            let path = dir.join(paths::unique_name(dir, &name));
            // create_new: never overwrite something that appeared meanwhile
            std::io::Write::write_all(&mut std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?, &bytes)?;
            Ok(vec![grids_shell::OpItem { src: path.clone(), dst: Some(path), is_dir: false }])
        }), window, cx);
    }

    /// Runs a copy / move / delete on a background thread (Windows shows its
    /// own progress and confirmation), then refreshes the folders involved.
    /// Delete / Shift+Delete: the workspace asks first (our dialog, never
    /// the system's), then calls `delete_confirmed`.
    fn ask_delete(&mut self, permanent: bool, cx: &mut Context<Self>) {
        let paths = self.state.targets();
        // A read-only folder gets the notice, never the dialog.
        if paths.is_empty() || self.refuse_locked(&paths, cx) { return }
        cx.emit(PaneEvent::AskDelete { paths, permanent })
    }

    /// Runs a delete the user has confirmed.
    pub fn delete_confirmed(&mut self, paths: Vec<PathBuf>, permanent: bool, cx: &mut Context<Self>) {
        self.run_op(if permanent { Op::Delete } else { Op::Recycle }, paths, None, vec![], cx)
    }

    fn run_op(&mut self, op: Op, paths: Vec<PathBuf>, dest: Option<PathBuf>, select: Vec<String>, cx: &mut Context<Self>) {
        if paths.is_empty() { return }
        // Last line of defence for every route here (keys, menus, drops).
        // A move out of a read-only area is refused; a copy is fine.
        if !matches!(op, Op::Copy) && self.refuse_locked(&paths, cx) { return }
        if let Some(d) = &dest && self.refuse_locked(std::slice::from_ref(d), cx) { return }
        if op == Op::Move {
            // Moves wait until nothing holds the files.
            let me = cx.entity().downgrade();
            let p2 = paths.clone();
            let then: crate::workspace::LockersThen = Rc::new(move |_, cx| {
                let (paths, dest, select) = (p2.clone(), dest.clone(), select.clone());
                let _ = me.update(cx, |this, cx| this.run_op_now(op, paths, dest, select, cx));
            });
            return cx.emit(PaneEvent::Lockers { paths, then: Some(then) });
        }
        self.run_op_now(op, paths, dest, select, cx)
    }

    fn run_op_now(&mut self, op: Op, paths: Vec<PathBuf>, dest: Option<PathBuf>, select: Vec<String>, cx: &mut Context<Self>) {
        let mut dirs: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
        if let Some(d) = &dest { dirs.push(d.clone()) }
        dirs.sort();
        dirs.dedup();
        let en = state::en(cx);
        let pane = Some(self.index);
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { ops::run(op, &paths, dest.as_deref(), pane) }).await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(items) => if ops::after_op(op, &items, cx) { cx.emit(PaneEvent::Changed) },
                    Err(e) => if e.kind() != std::io::ErrorKind::Interrupted { cx.emit(PaneEvent::Toast(format!("{}{e}", if en { "Operation failed: " } else { "操作失败：" }))) },
                }
                if !select.is_empty() && dirs.contains(&this.state.path) {
                    this.pending_select = Some(select);
                }
                cx.emit(PaneEvent::FsChanged(dirs));
            });
        }).detach();
    }

    /// Reloads when showing one of `dirs` (selecting freshly pasted names).
    pub fn refresh_if_showing(&mut self, dirs: &[PathBuf], cx: &mut Context<Self>) {
        if dirs.iter().any(|d| d == &self.state.path) {
            self.load_with(None, true, cx);
        }
    }

    fn drop_files(&mut self, d: &DraggedFiles, into: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let dest = into.unwrap_or_else(|| self.state.path.clone());
        if d.from == dest || d.paths.iter().any(|p| p == &dest || dest.starts_with(p)) { return }
        // Out of a read-only folder a drag only copies, and says so.
        let same_vol = d.paths.first().is_some_and(|p| grids_shell::same_volume(p, &dest));
        let locked = state::locked_any(cx, &d.paths).is_some();
        let copy = window.modifiers().control || locked || !same_vol;
        if locked && same_vol && !window.modifiers().control {
            cx.emit(PaneEvent::Toast(tr(cx, "只读文件夹：已复制，未移动", "Read-only folder: copied, not moved").into()));
        }
        let names = d.paths.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        window.focus(&self.focus);
        self.run_op(if copy { Op::Copy } else { Op::Move }, d.paths.clone(), Some(dest), names, cx);
    }

    fn drop_external(&mut self, paths: &ExternalPaths, into: Option<PathBuf>, cx: &mut Context<Self>) {
        let dest = into.unwrap_or_else(|| self.state.path.clone());
        let list: Vec<PathBuf> = paths.paths().iter().filter(|p| p.parent() != Some(dest.as_path())).cloned().collect();
        let names = list.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        self.run_op(Op::Copy, list, Some(dest), names, cx);
    }

    // ---- rubber-band selection

    /// Tracks the pointer while a box is dragged out over the list.
    fn marquee_track(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let weak_up = weak.clone();
        canvas(|_, _, _| {}, move |_, _, window, _| {
            let w = weak.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture { return }
                let _ = w.update(cx, |this, cx| this.marquee_to(ev.position, cx));
            });
            let w = weak_up.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture { return }
                let _ = w.update(cx, |this, cx| if this.marquee.take().is_some() { cx.notify() });
            });
        }).absolute().size_full()
    }

    fn marquee_to(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let (start, base) = match &mut self.marquee {
            Some(m) => {
                if m.cur == pos { return }
                m.cur = pos;
                (m.start, m.base.clone())
            }
            None => return,
        };
        let rows = self.marquee_rows(start, pos);
        self.state.select_rows(&rows, &base);
        cx.notify();
    }

    /// Rows whose item rectangle meets the box.
    fn marquee_rows(&self, a: Point<Pixels>, b: Point<Pixels>) -> Vec<usize> {
        let n = self.state.len();
        let bounds = self.list_bounds.get();
        let avail = f32::from(bounds.size.width);
        if n == 0 || avail <= 1. { return Vec::new() }
        let (ax, ay, bx, by) = (f32::from(a.x), f32::from(a.y), f32::from(b.x), f32::from(b.y));
        let (x0, x1) = (ax.min(bx), ax.max(bx));
        let (y0, y1) = (ay.min(by), ay.max(by));
        let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let scroll = f32::from(self.scroll.0.borrow().base_handle.offset().y);
        let mut out = Vec::new();
        match grid_metrics(self.view, avail) {
            None => {
                let first = ((y0 - top + scroll) / ROW_H).floor().max(0.) as usize;
                let last = ((y1 - top + scroll) / ROW_H).floor().max(0.) as usize;
                for r in first..=last.min(n - 1) {
                    let itop = top + r as f32 * ROW_H - scroll;
                    if itop + ROW_H >= y0 && itop <= y1 { out.push(r) }
                }
            }
            Some(g) => {
                let first = ((y0 - top + scroll) / g.cell_h).floor().max(0.) as usize;
                let last = ((y1 - top + scroll) / g.cell_h).floor().max(0.) as usize;
                for row in first..=last {
                    let itop = top + row as f32 * g.cell_h - scroll;
                    if itop + g.cell_h < y0 || itop > y1 { continue }
                    for c in 0..g.cols {
                        let i = row * g.cols + c;
                        if i >= n { break }
                        let ileft = left + ROW_X + c as f32 * g.cell_w;
                        if ileft + g.cell_w >= x0 && ileft <= x1 { out.push(i) }
                    }
                }
            }
        }
        out
    }

    // ---- context menu

    /// Opens the GPUI menu for the selection (`row`) or the folder background.
    fn context_menu(&mut self, row: Option<usize>, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = row && !self.state.is_selected(r) { self.state.click(Some(r), false, false) }
        if row.is_none() { self.state.clear_selection() }
        let en = state::en(cx);
        let t = move |zh: &'static str, e: &'static str| -> SharedString { (if en { e } else { zh }).into() };
        // Read-only: no edits, and no system menu (it would get round that).
        let ro = self.here_locked(cx);
        let can_paste = state::get_clipboard(cx).is_some() && !ro;
        let more = Item::row(t("显示更多选项", "Show more options"), "ellipsis", "", Cmd::More).enabled(cfg!(windows));
        let open_with_label = t("打开方式", "Open with");
        let mut open_with_for = None;
        let items = if row.is_some() {
            let sel: Vec<Entry> = self.state.selection().into_iter().cloned().collect();
            let one = sel.len() == 1;
            let folder = sel.first().filter(|e| one && e.is_dir).map(|e| e.path.clone());
            let file = sel.first().filter(|e| one && !e.is_dir).map(|e| e.path.clone());
            let mut v = vec![Item::row(t("打开", "Open"), if folder.is_some() { "folder-open" } else { "external-link" }, "Enter", Cmd::Open)];
            if let Some(f) = &file {
                open_with_for = Some(f.clone());
                v.push(Item::sub(open_with_label.clone(), "app-window", vec![Item::Note(t("正在读取…", "Loading…"))]));
            }
            if folder.is_some() { v.push(Item::row(t("在下一格中打开", "Open in next pane"), "square-split-horizontal", "", Cmd::OpenInNextPane)) }
            v.push(Item::Sep);
            v.push(Item::row(t("剪切", "Cut"), "scissors", "Ctrl+X", Cmd::Cut).enabled(!ro));
            v.push(Item::row(t("复制", "Copy"), "copy", "Ctrl+C", Cmd::Copy));
            if let Some(dir) = &folder { v.push(Item::row(t("粘贴到此文件夹", "Paste into folder"), "clipboard-paste", "", Cmd::PasteInto(dir.clone())).enabled(can_paste)) }
            v.push(Item::row(t("复制路径", "Copy as path"), "link", "Ctrl+Shift+C", Cmd::CopyPath));
            v.push(Item::row(t("添加到暂存架", "Add to shelf"), "inbox", "Ctrl+Shift+A", Cmd::Shelve));
            v.push(Item::row(t("重命名", "Rename"), "pencil", "F2", Cmd::Rename).enabled(one && !ro));
            v.push(Item::row(t("删除", "Delete"), "trash-2", "Delete", Cmd::Delete).enabled(!ro));
            v.push(Item::Sep);
            v.push(Item::row(t("压缩为 ZIP 文件", "Compress to ZIP file"), "file-archive", "", Cmd::Zip).enabled(!ro));
            if let Some(dir) = &folder {
                let has = grids_core::notes::get(&state::shared(cx).config.notes, dir).is_some();
                v.push(Item::row(if has { t("编辑备注", "Edit note") } else { t("添加备注", "Add note") }, "sticky-note", "", Cmd::EditNote(dir.clone())));
                let fav = state::shared(cx).config.is_favourite(dir);
                v.push(Item::row(if fav { t("从收藏夹移除", "Remove from favourites") } else { t("添加到收藏夹", "Add to favourites") }, if fav { "star-off" } else { "star" }, "", Cmd::ToggleFavourite(dir.clone())));
            }
            v.push(Item::row(t("在资源管理器中显示", "Show in Explorer"), "folder-search", "", Cmd::Reveal));
            v.push(Item::row(t("查看占用", "Who's using it"), "file-lock", "", Cmd::Lockers));
            v.push(Item::row(t("属性", "Properties"), "info", "Alt+Enter", Cmd::Properties).enabled(cfg!(windows)));
            if !ro { v.push(Item::Sep); v.push(more) }
            v
        } else {
            let st = &self.state;
            let sort = |label: SharedString, key: SortKey| Item::check(label, st.sort == key, "", Cmd::Sort(key));
            let sort_items = vec![
                sort(t("名称", "Name"), SortKey::Name),
                sort(t("修改日期", "Date modified"), SortKey::Modified),
                sort(t("类型", "Type"), SortKey::Type),
                sort(t("大小", "Size"), SortKey::Size),
                Item::Sep,
                Item::check(t("递增", "Ascending"), st.ascending, "", Cmd::Ascending(true)),
                Item::check(t("递减", "Descending"), !st.ascending, "", Cmd::Ascending(false)),
            ];
            let mut view = view_items(self.view, en);
            view.extend([
                Item::Sep,
                Item::sub(t("排序方式", "Sort by"), "arrow-up-down", sort_items),
                Item::check(t("显示隐藏的项目", "Show hidden items"), st.show_hidden, "Ctrl+H", Cmd::ToggleHidden),
            ]);
            let mut new_items = vec![Item::row(t("文件夹", "Folder"), "folder", "Ctrl+Shift+N", Cmd::NewFolder), Item::Sep];
            for n in state::new_items(cx).iter() {
                let glyph = n.icon.clone().map_or(Glyph::Icon("file"), Glyph::Image);
                new_items.push(Item::Row { label: n.name.clone().into(), glyph, keys: "", enabled: true, cmd: Some(Cmd::NewFile(n.ext.clone())), sub: None });
            }
            vec![
                Item::sub(t("查看", "View"), "eye", view),
                Item::row(t("刷新", "Refresh"), "refresh-cw", "F5", Cmd::Refresh),
                Item::Sep,
                // No files on the clipboard: Ctrl+V saves an image or text as a file.
                if can_paste || ro { Item::row(t("粘贴", "Paste"), "clipboard-paste", "Ctrl+V", Cmd::Paste).enabled(can_paste) }
                else { Item::row(t("粘贴为文件", "Paste as file"), "clipboard-paste", "Ctrl+V", Cmd::Paste) },
                Item::sub(t("新建", "New"), "plus", if ro { vec![Item::Note(t("只读文件夹", "Read-only folder"))] } else { new_items }),
                Item::Sep,
                Item::row(t("在终端中打开", "Open in Terminal"), "terminal", "", Cmd::Terminal),
                Item::row(t("复制文件夹路径", "Copy folder path"), "link", "", Cmd::CopyFolderPath),
                {
                    let has = grids_core::notes::get(&state::shared(cx).config.notes, &self.state.path).is_some();
                    Item::row(if has { t("编辑此文件夹的备注", "Edit this folder's note") } else { t("为此文件夹添加备注", "Add a note to this folder") }, "sticky-note", "", Cmd::EditNote(self.state.path.clone()))
                },
                Item::row(t("属性", "Properties"), "info", "", Cmd::FolderProperties).enabled(cfg!(windows)),
                Item::Sep,
                more,
            ]
        };
        let items = if ro && row.is_none() { let mut v = items; v.truncate(v.len() - 2); v } else { items };
        let menu = cx.new(|cx| MenuView::new(items, pos, 264., cx));
        let sub = cx.subscribe_in(&menu, window, move |this, _, ev: &MenuEvent, window, cx| {
            let pos = pos;
            this.menu = None;
            window.focus(&this.focus);
            if let MenuEvent::Pick(cmd) = ev { this.run_cmd(cmd.clone(), row, pos, window, cx) }
            cx.notify();
        });
        // After the click's own handling: GPUI focuses the clicked pane as the event bubbles.
        let mf = menu.read(cx).focus.clone();
        window.defer(cx, move |window, _| mf.focus(window));
        // The handlers come from the shell, which may read the registry for a while.
        if let Some(path) = open_with_for {
            let m = menu.downgrade();
            let label = open_with_label.to_string();
            cx.spawn(async move |_, cx| {
                let handlers = cx.background_executor().spawn(async move { grids_shell::open_with_handlers(&path) }).await;
                let _ = m.update(cx, |m, cx| {
                    let mut v: Vec<Item> = handlers.into_iter().map(|h| Item::Row {
                        label: h.name.into(),
                        glyph: h.icon.map_or(Glyph::Icon("app-window"), |i| Glyph::Image(state::render_image(&i))),
                        keys: "", enabled: true, cmd: Some(Cmd::OpenWith(h.id)), sub: None,
                    }).collect();
                    if !v.is_empty() { v.push(Item::Sep) }
                    v.push(Item::row(if en { "Choose another app" } else { "选择其他应用" }, "layout-grid", "", Cmd::OpenWithOther));
                    m.set_submenu(&label, v, cx);
                });
            }).detach();
        }
        self.menu = Some((menu, sub));
        cx.notify();
    }

    /// The view button's menu, under the button.
    fn view_menu(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let items = view_items(self.view, state::en(cx));
        let menu = cx.new(|cx| MenuView::new(items, pos, 220., cx));
        let sub = cx.subscribe_in(&menu, window, move |this, _, ev: &MenuEvent, window, cx| {
            this.menu = None;
            window.focus(&this.focus);
            if let MenuEvent::Pick(cmd) = ev { this.run_cmd(cmd.clone(), None, pos, window, cx) }
            cx.notify();
        });
        let mf = menu.read(cx).focus.clone();
        window.defer(cx, move |window, _| mf.focus(window));
        self.menu = Some((menu, sub));
        cx.notify();
    }

    /// The title menu: link with another pane, compare, unlink.
    fn link_menu(&mut self, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        let t = move |zh: &'static str, e: &'static str| -> SharedString { (if en { e } else { zh }).into() };
        let others: Vec<Item> = (0..self.pane_count).filter(|&j| j != self.index)
            .map(|j| Item::row(if en { format!("Pane {}", j + 1) } else { format!("格子 {}", j + 1) }, "layout-grid", "", Cmd::Link(LinkCmd::With(j)))).collect();
        let mut items = vec![Item::sub(t("联动到", "Link with"), "link", others).enabled(self.pane_count > 1)];
        if let Some(l) = &self.link {
            items.push(Item::check(t("对比两侧差异", "Compare both sides"), l.compare, "", Cmd::Link(LinkCmd::Compare(!l.compare))).enabled(l.can_compare));
            items.push(Item::Sep);
            items.push(Item::row(t("取消联动", "Unlink"), "unlink", "", Cmd::Link(LinkCmd::Unlink)));
        }
        let menu = cx.new(|cx| MenuView::new(items, pos, 200., cx));
        let sub = cx.subscribe_in(&menu, window, move |this, _, ev: &MenuEvent, window, cx| {
            this.menu = None;
            window.focus(&this.focus);
            if let MenuEvent::Pick(cmd) = ev { this.run_cmd(cmd.clone(), None, pos, window, cx) }
            cx.notify();
        });
        let mf = menu.read(cx).focus.clone();
        window.defer(cx, move |window, _| mf.focus(window));
        self.menu = Some((menu, sub));
        cx.notify();
    }

    /// Carries out a menu command.
    fn run_cmd(&mut self, cmd: Cmd, row: Option<usize>, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        let hwnd = crate::ui::hwnd(window);
        let toast = move |this: &mut Self, r: std::io::Result<()>, cx: &mut Context<Self>| if let Err(e) = r && e.kind() != std::io::ErrorKind::Interrupted { let _ = this; cx.emit(PaneEvent::Toast(e.to_string())) };
        let bg = |this: &mut Self, f: Box<dyn FnOnce() -> std::io::Result<()> + Send>, cx: &mut Context<Self>| {
            let _ = this;
            cx.spawn(async move |this, cx| {
                let r = cx.background_executor().spawn(async move { f() }).await;
                let _ = this.update(cx, |this, cx| toast(this, r, cx));
            }).detach();
        };
        let targets = self.state.targets();
        let first = targets.first().cloned();
        let dir = self.state.path.clone();
        match cmd {
            Cmd::Open => self.open_selected(cx),
            Cmd::OpenWith(id) => if let Some(p) = first { bg(self, Box::new(move || grids_shell::open_with(&p, &id)), cx) },
            Cmd::OpenWithOther => if let Some(p) = first {
                cx.spawn_in(window, async move |this, cx| { let r = grids_shell::open_with_dialog(hwnd, &p); let _ = this.update(cx, |this, cx| toast(this, r, cx)); }).detach();
            },
            Cmd::OpenInNextPane => if let Some(p) = first { cx.emit(PaneEvent::OpenInNext(p)) },
            Cmd::Cut => self.copy_files(true, cx),
            Cmd::Copy => self.copy_files(false, cx),
            Cmd::Paste => self.paste(window, cx),
            Cmd::PasteInto(d) => self.paste_into(d, cx),
            Cmd::CopyPath => self.copy_paths(cx),
            Cmd::Shelve => self.shelve(cx),
            Cmd::EditNote(p) => cx.emit(PaneEvent::EditNote(p)),
            Cmd::Link(c) => cx.emit(PaneEvent::Link(c)),
            Cmd::Lockers => if !targets.is_empty() { cx.emit(PaneEvent::Lockers { paths: targets.clone(), then: None }) },
            Cmd::Rename => self.start_rename(window, cx),
            Cmd::Delete => self.ask_delete(false, cx),
            Cmd::Zip => if let Some(p) = first && !self.refuse_locked(std::slice::from_ref(&dir), cx) {
                let stem = if targets.len() == 1 { p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() } else { dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into()) };
                let name = paths::unique_name(&dir, &format!("{stem}.zip"));
                let dest = dir.join(&name);
                let dirs = vec![dir.clone()];
                self.pending_select = Some(vec![name]);
                cx.spawn(async move |this, cx| {
                    let r = cx.background_executor().spawn(async move { grids_shell::compress_zip(&targets, &dest) }).await;
                    let _ = this.update(cx, |this, cx| { toast(this, r, cx); cx.emit(PaneEvent::FsChanged(dirs)) });
                }).detach();
            },
            Cmd::ToggleFavourite(p) => {
                cx.global_mut::<state::Shared>().config.toggle_favourite(&p);
                cx.emit(PaneEvent::Changed);
            }
            Cmd::Reveal => if let Some(p) = first { bg(self, Box::new(move || grids_shell::reveal_in_explorer(&p)), cx) },
            Cmd::Properties => if !targets.is_empty() {
                cx.spawn_in(window, async move |this, cx| { let r = grids_shell::properties(hwnd, &targets); let _ = this.update(cx, |this, cx| toast(this, r, cx)); }).detach();
            },
            Cmd::FolderProperties => {
                cx.spawn_in(window, async move |this, cx| { let r = grids_shell::properties(hwnd, &[dir]); let _ = this.update(cx, |this, cx| toast(this, r, cx)); }).detach();
            }
            Cmd::More => self.native_menu(row.is_some(), pos, window, cx),
            Cmd::Sort(k) => { if self.state.sort != k { self.sort_by(k, cx) } }
            Cmd::Ascending(up) => { if self.state.ascending != up { let k = self.state.sort; self.sort_by(k, cx) } }
            Cmd::ToggleHidden => self.toggle_hidden(cx),
            Cmd::Refresh => self.load(None, cx),
            Cmd::View(v) => self.set_view(v, cx),
            Cmd::NewFolder => self.new_folder(window, cx),
            Cmd::NewFile(_) if self.refuse_locked(std::slice::from_ref(&dir), cx) => {}
            Cmd::NewFile(ext) => {
                let fail = if en { "Cannot create file: " } else { "无法新建文件：" };
                self.create_then_rename(OpKind::New, fail, Box::new(move |dir| grids_shell::create_new_tracked(dir, &ext)), window, cx);
            }
            Cmd::Terminal => bg(self, Box::new(move || grids_shell::open_terminal(&dir)), cx),
            Cmd::CopyFolderPath => {
                cx.write_to_clipboard(ClipboardItem::new_string(dir.to_string_lossy().into_owned()));
                cx.emit(PaneEvent::Toast((if en { "Copied the folder path" } else { "已复制文件夹路径" }).into()));
            }
        }
    }

    /// Puts the selection on the shelf (allowed in read-only folders too).
    fn shelve(&mut self, cx: &mut Context<Self>) {
        let t = self.state.targets();
        if t.is_empty() { return }
        let added = state::shelve(cx, &t);
        cx.emit(PaneEvent::Changed);
        let msg = match (state::en(cx), added) {
            (true, 0) => "Already on the shelf".to_string(),
            (true, n) => format!("Added {n} item(s) to the shelf"),
            (false, 0) => "已在暂存架上".to_string(),
            (false, n) => format!("已加入暂存架 {n} 项"),
        };
        cx.emit(PaneEvent::Toast(msg));
    }

    /// "Copy as path": quoted paths, one per line, as Explorer does.
    fn copy_paths(&mut self, cx: &mut Context<Self>) {
        let t = self.state.targets();
        if t.is_empty() { return }
        let text = t.iter().map(|p| format!("\"{}\"", p.display())).collect::<Vec<_>>().join("\r\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.emit(PaneEvent::Toast(if state::en(cx) { format!("Copied {} path(s)", t.len()) } else { format!("已复制 {} 个路径", t.len()) }));
    }

    /// The system's own menu (third-party entries such as WPS or 7-Zip).
    fn native_menu(&mut self, on_items: bool, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.state.targets();
        let folder = self.state.path.clone();
        let hwnd = crate::ui::hwnd(window);
        let scale = window.scale_factor();
        let origin = window.bounds().origin;
        let screen = ((f32::from(origin.x + pos.x) * scale) as i32, (f32::from(origin.y + pos.y) * scale) as i32);
        // The menu runs a modal loop: call it outside any app borrow.
        cx.spawn_in(window, async move |this, cx| {
            let action = if on_items && !targets.is_empty() { grids_shell::context_menu(hwnd, &targets, screen) } else { grids_shell::background_menu(hwnd, &folder, screen) };
            let _ = this.update_in(cx, |this, window, cx| match action {
                grids_shell::MenuAction::Dismissed => {}
                grids_shell::MenuAction::Rename => this.start_rename(window, cx),
                grids_shell::MenuAction::Invoked(_) => this.load(None, cx),
            });
        }).detach();
    }

    fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        let on = !self.state.show_hidden;
        self.state.set_show_hidden(on);
        cx.emit(PaneEvent::Changed);
        cx.notify();
    }

    fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.state.sort_by(key);
        cx.emit(PaneEvent::Changed);
        cx.notify();
    }

    /// The drawn column widths for a list `avail` wide: the set widths,
    /// shrunk in turn (name to a comfortable width, type, date, size, then
    /// name to its minimum) so they fit; a column that would get too narrow
    /// is hidden (width 0).
    fn fit_columns(&self, avail: f32) -> [f32; 4] {
        let c = &self.columns;
        let mut w = [c.name, c.modified, c.kind, c.size];
        let room = avail - ROW_X * 2. - 2.;
        let floors: [(usize, f32); 5] = [(0, 160.), (2, 104.), (1, 96.), (3, 64.), (0, NAME_MIN)];
        for (i, floor) in floors {
            let over = w.iter().sum::<f32>() - room;
            if over <= 0. { break }
            w[i] = (w[i] - over).max(floor.min(w[i]));
        }
        // Still too wide: drop type, then date, then size.
        for i in [2, 1, 3] {
            if w.iter().sum::<f32>() - room <= 0. { break }
            w[i] = 0.;
        }
        // Spare room goes to the name column.
        let spare = room - w.iter().sum::<f32>();
        if spare > 0. && w[0] < c.name { w[0] = (w[0] + spare).min(c.name) }
        w
    }
}

const CHEVRON_W: f32 = 14.;
const FOLD_W: f32 = 22. + CHEVRON_W;

/// Rough width of a breadcrumb chip (label + padding): CJK about 1em,
/// other text about 0.62em, rounded up so a fold happens before a cut.
fn crumb_width(label: &str, size: f32) -> f32 {
    let em: f32 = label.chars().map(|c| if (c as u32) >= 0x2E80 { 1.0 } else if c.is_ascii_uppercase() || "mwMW".contains(c) { 0.72 } else { 0.58 }).sum();
    em * size + 8. + 2.
}

/// Clipboard content pasted as a new file.
enum Clip {
    Image(image::RgbaImage),
    Png(Vec<u8>),
    Text(String),
}

/// The sticky-note mark beside a folder that has a note; hover shows it.
fn note_mark(e: &Entry, r: usize, cx: &App) -> Option<AnyElement> {
    if !e.is_dir { return None }
    let notes = &state::shared(cx).config.notes;
    if notes.is_empty() { return None }
    let text = grids_core::notes::get(notes, &e.path)?;
    Some(div().id(("note", r)).flex_none().ml(px(-4.)).size(px(18.)).flex().items_center().justify_center()
        .child(icon("sticky-note", 13., Theme::star()))
        .tip_label(crate::ui::wrap_tip(text, 56), crate::ui::TipSide::Below)
        .into_any_element())
}

/// Where the filter text matches in `name` (case-insensitive), for highlighting.
fn filter_hit(name: &str, filter: &str) -> Option<std::ops::Range<usize>> {
    if filter.is_empty() { return None }
    let lower = name.to_lowercase();
    if lower.len() != name.len() { return None }
    let f = filter.to_lowercase();
    let at = lower.find(&f)?;
    Some(at..at + f.len())
}

/// The date alone (no time) when the column is too narrow for both.
/// The grid views' geometry for a list `avail` px wide: items per row and
/// each cell's size, and the picture's edge. None in the details view.
#[derive(Clone, Copy)]
struct Grid {
    cols: usize,
    cell_w: f32,
    cell_h: f32,
    icon: f32,
}

/// The theme pack's PNG icon for a file type, when it ships one.
fn type_img(kind: grids_core::FileKind, dark: bool) -> Option<SharedString> {
    use grids_core::FileKind::*;
    let name = match kind {
        Folder => "folder",
        Image => "file-image",
        Video => "file-video",
        Audio => "file-audio",
        Archive => "file-archive",
        Code => "file-code",
        Text | Word | Pdf => "file-text",
        Sheet => "file-spreadsheet",
        Slides => "presentation",
        App => "app-window",
        Other => "file",
    };
    let rel = if dark { format!("types/{name}-dark.png") } else { format!("types/{name}.png") };
    theme::has(&rel).then(|| SharedString::from(rel))
}

fn grid_metrics(view: ViewMode, avail: f32) -> Option<Grid> {
    // (picture edge, narrowest cell, cell height)
    let (icon, min_w, cell_h) = match view {
        ViewMode::Details => return None,
        ViewMode::List => (ICON, 220., ROW_H),
        ViewMode::Medium => (48., 112., 48. + 58.),
        ViewMode::Large => (96., 132., 96. + 60.),
        ViewMode::Huge => (192., 224., 192. + 62.),
    };
    let inner = (avail - 2. * ROW_X).max(min_w);
    let cols = ((inner / min_w).floor() as usize).max(1);
    Some(Grid { cols, cell_w: inner / cols as f32, cell_h, icon })
}

fn view_glyph(v: ViewMode) -> &'static str {
    match v {
        ViewMode::Details => "layout-list",
        ViewMode::List => "list",
        ViewMode::Medium => "layout-grid",
        ViewMode::Large => "grid-2x2",
        ViewMode::Huge => "image",
    }
}

/// The view choices, as in Explorer's View menu (largest first).
fn view_items(cur: ViewMode, en: bool) -> Vec<Item> {
    ViewMode::ALL.iter().enumerate().map(|(i, &v)| {
        let label = match (v, en) {
            (ViewMode::Huge, false) => "超大图标",
            (ViewMode::Large, false) => "大图标",
            (ViewMode::Medium, false) => "中图标",
            (ViewMode::List, false) => "列表",
            (ViewMode::Details, false) => "详细信息",
            (ViewMode::Huge, true) => "Extra large icons",
            (ViewMode::Large, true) => "Large icons",
            (ViewMode::Medium, true) => "Medium icons",
            (ViewMode::List, true) => "List",
            (ViewMode::Details, true) => "Details",
        };
        const KEYS: [&str; 5] = ["Ctrl+Shift+1", "Ctrl+Shift+2", "Ctrl+Shift+3", "Ctrl+Shift+4", "Ctrl+Shift+5"];
        Item::check(label, v == cur, KEYS[i], Cmd::View(v))
    }).collect()
}

fn short_date(full: String, width: f32) -> String {
    if width >= 124. { return full }
    full.split(' ').next().unwrap_or_default().to_string()
}

fn error_text(e: &std::io::Error, en: bool) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => (if en { "This folder no longer exists." } else { "此文件夹不存在。" }).into(),
        std::io::ErrorKind::PermissionDenied => (if en { "Access denied." } else { "拒绝访问。" }).into(),
        _ => e.to_string(),
    }
}

// ---------------------------------------------------------------- rendering

impl PaneView {
    /// One light row: the pane number, the path as breadcrumbs (the folder
    /// itself in the title weight), and buttons that stay faint until the
    /// pane is hovered or active. A click on the empty part edits the path.
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let st = &self.state;
        let fav = state::shared(cx).config.is_favourite(&st.path);
        let en = state::en(cx);
        let t = move |zh: &'static str, e: &'static str| if en { e } else { zh };
        let badge = div()
            .flex_none()
            .min_w(px(18.))
            .h(px(18.))
            .px(px(4.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .map(|d| if self.active { d.bg(Theme::primary()).text_color(gpui::white()) } else { d.text_color(Theme::text_placeholder()) })
            .child((self.index + 1).to_string());
        let path_area: AnyElement = if let Some((input, _)) = &self.address {
            div().flex_1().min_w_0().h(px(FIELD_H)).px(px(8.)).rounded(px(8.)).flex().items_center().bg(Theme::bg_body()).border_1().border_color(Theme::field_edge()).child(div().flex_1().min_w_0().child(input.clone())).into_any_element()
        } else {
            let crumbs = paths::breadcrumbs(&st.path);
            let n = crumbs.len();
            // Fold leading crumbs into "…" until the rest fits; never cut a
            // label in half. The last crumb (the folder) always stays.
            let avail = self.crumbs_w.get();
            let widths: Vec<f32> = crumbs.iter().enumerate().map(|(i, (l, _))| crumb_width(l, if i + 1 == n { 14. } else { 13. }) + if i + 1 == n { 0. } else { CHEVRON_W }).collect();
            let mut skip = 0;
            if avail > 0. {
                let mut total: f32 = widths.iter().sum();
                while skip + 1 < n && total > avail - 8. {
                    total -= widths[skip];
                    if skip == 0 { total += FOLD_W }
                    skip += 1;
                }
            }
            let mut row = div().flex().min_w_0().items_center();
            if skip > 0 {
                let parent = crumbs[skip - 1].1.clone();
                let hidden: String = crumbs[..skip].iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(" › ");
                row = row.child(
                    div().id("crumb-fold").flex_none().h(px(24.)).px(px(4.)).rounded(px(6.)).flex().items_center().cursor_pointer()
                        .text_size(px(13.)).text_color(Theme::text_caption())
                        .hover(|s| s.bg(Theme::fill_hover()).text_color(Theme::text()))
                        .child("…")
                        .tip(&hidden, None)
                        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| { cx.stop_propagation(); if ev.click_count() < 2 { this.navigate(parent.clone(), cx) } })),
                ).child(div().mx(px(1.)).flex_none().child(icon("chevron-right", 12., Theme::icon_muted())));
            }
            for (i, (label, path)) in crumbs.into_iter().enumerate().skip(skip) {
                let last = i + 1 == n;
                row = row.child(
                    div().id(("crumb", i)).h(px(24.)).px(px(4.)).rounded(px(6.)).flex().items_center().whitespace_nowrap().cursor_pointer()
                        .map(|d| if last { d.min_w_0().overflow_hidden().text_ellipsis().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()) } else { d.flex_none().text_size(px(13.)).text_color(Theme::text_caption()) })
                        .hover(|s| s.bg(Theme::fill_hover()).text_color(Theme::text()))
                        .child(label)
                        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| { cx.stop_propagation(); if ev.click_count() < 2 { this.navigate(path.clone(), cx) } })),
                );
                if !last { row = row.child(div().mx(px(1.)).flex_none().child(icon("chevron-right", 12., Theme::icon_muted()))) }
            }
            let crumbs_w = self.crumbs_w.clone();
            // Long paths keep their end (the folder) in view.
            div().id("crumbs").relative().flex_1().min_w_0().h(px(FIELD_H)).flex().items_center().overflow_hidden().cursor_text()
                .child(canvas(move |b, window, _| {
                    let w = f32::from(b.size.width);
                    if (crumbs_w.replace(w) - w).abs() > 0.5 { window.request_animation_frame() }
                }, |_, _, _, _| {}).absolute().size_full())
                .child(row).child(div().flex_1().min_w(px(12.)))
                .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if ev.click_count() >= 2 {
                        this.addr_click = None;
                        return cx.emit(PaneEvent::ToggleMaximize);
                    }
                    // Wait out a possible second click before editing.
                    this.addr_click = Some(cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(260)).await;
                        let _ = this.update_in(cx, |this, window, cx| { if this.addr_click.take().is_some() { this.edit_address(window, cx) } });
                    }));
                }))
                .into_any_element()
        };
        let quiet = !self.active;
        let btn = |id: &'static str, glyph: &'static str, enabled: bool| {
            div().id(id).size(px(26.)).rounded(px(7.)).flex_none().flex().items_center().justify_center()
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed())))
                .child(icon(glyph, 16., if enabled { Theme::icon() } else { Theme::text_disabled() }))
        };
        // Changes since the user last touched this pane.
        let seen_tip = t("新增或修改的文件数，点一下格子即清零", "Added or modified since you last touched this pane; click to clear");
        let unseen = (self.unseen > 0).then(|| {
            div().id("unseen").flex_none().h(px(18.)).px(px(6.)).rounded(px(9.)).flex().items_center()
                .bg(Theme::primary().alpha(0.14)).text_color(Theme::primary())
                .text_size(px(11.)).font_weight(FontWeight::SEMIBOLD)
                .child(format!("+{}", self.unseen.min(9999)))
                .tip(seen_tip, None)
        });
        let max_tag = self.maximized.map(|n| {
            div().flex_none().h(px(20.)).px(px(7.)).rounded(px(6.)).flex().items_center().gap(px(4.))
                .bg(Theme::fill_hover()).text_size(px(11.)).text_color(Theme::text_caption()).whitespace_nowrap()
                .child(icon("maximize-2", 11., Theme::icon_muted()))
                .child(if en { format!("Pane {} of {n}", self.index + 1) } else { format!("第 {} 格 / 共 {n} 格", self.index + 1) })
        });
        // Read-only: the lock stays in view; unlocked it shows on hover.
        let lock_root = state::locked_root(cx, &st.path);
        let brand = state::brand(cx);
        let lock_tip: SharedString = match &lock_root {
            Some(r) if r == &st.path => if en { format!("Read-only folder (inside {brand} only) · click to unlock") } else { format!("只读文件夹（仅在{brand}内生效）· 点击解锁") }.into(),
            Some(r) => if en { format!("Read-only: locked at “{}” · click to unlock", r.display()) } else { format!("只读：由「{}」锁定 · 点击解锁", r.display()) }.into(),
            None => if en { format!("Make read-only (inside {brand} only)") } else { format!("设为只读（仅在{brand}内生效）") }.into(),
        };
        let locked = lock_root.is_some();
        let lock_btn = div().id("lock").size(px(26.)).rounded(px(7.)).flex_none().flex().items_center().justify_center().cursor_pointer()
            .hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed()))
            .child(if locked { icon("lock", 15., Theme::primary()) } else { icon("lock-open", 15., Theme::icon()) })
            .tip(&lock_tip, None)
            .on_click(cx.listener(move |this, _, window, cx| {
                let folder = state::locked_root(cx, &this.state.path).unwrap_or_else(|| this.state.path.clone());
                grids_core::locked::toggle(&mut cx.global_mut::<state::Shared>().config.locked, &folder);
                cx.emit(PaneEvent::Changed);
                // Other panes may show folders under it.
                window.refresh();
            }));
        let (lock_out, lock_in) = if locked { (Some(lock_btn), None) } else { (None, Some(lock_btn)) };
        // Link group: a dot in the group's colour, hollow while paused.
        let link_dot = self.link.as_ref().map(|l| {
            let c = Theme::link(l.group);
            let tip: SharedString = match (l.live, l.compare) {
                (false, _) => t("联动已暂停：已离开基准文件夹，回到其中自动恢复", "Link paused: outside the base folder; resumes inside it").into(),
                (true, true) => t("联动中 · 对比差异（绿色：仅此侧有，琥珀色：有差别）", "Linked · comparing (green: only here, amber: differs)").into(),
                (true, false) => if en { format!("Linked (group {})", l.group + 1) } else { format!("联动中（第 {} 组）", l.group + 1) }.into(),
            };
            div().id("link-dot").flex_none().size(px(18.)).flex().items_center().justify_center().cursor_pointer()
                .map(|d| if l.compare && l.live { d.child(icon("git-compare", 14., c)) } else { d.child(div().size(px(9.)).rounded_full().border(px(1.5)).border_color(c).when(l.live, |d| d.bg(c))) })
                .tip(&tip, None)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, ev: &ClickEvent, window, cx| { cx.stop_propagation(); this.link_menu(ev.position(), window, cx) }))
        });
        div()
            .id("pane-head")
            .h(px(HEAD_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(12.))
            .pr(px(8.))
            .on_click(cx.listener(|_, ev: &ClickEvent, _, cx| if ev.click_count() == 2 { cx.emit(PaneEvent::ToggleMaximize) }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| { cx.stop_propagation(); cx.emit(PaneEvent::Activated); this.link_menu(ev.position, window, cx) }))
            .child(badge)
            .children(link_dot)
            .children(max_tag)
            .children(unseen)
            .child(path_area)
            .children(lock_out)
            .child(
                div().flex().flex_none().items_center().gap(px(1.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(quiet, |d| d.opacity(0.).group_hover("pane", |s| s.opacity(1.)))
                    .children(lock_in)
                    .child(btn("back", "arrow-left", st.can_back()).tip(t("后退", "Back"), Some("Alt+←")).on_click(cx.listener(|this, _, _, cx| this.go_back(cx))))
                    .child(btn("fwd", "arrow-right", st.can_forward()).tip(t("前进", "Forward"), Some("Alt+→")).on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))))
                    .child(btn("up", "arrow-up", st.can_up()).tip(t("上一级", "Up"), Some("Alt+↑")).on_click(cx.listener(|this, _, _, cx| this.go_up(cx))))
                    .child(btn("view", view_glyph(self.view), true).tip(t("视图", "View"), Some("Ctrl+Shift+1…5")).on_click(cx.listener(|this, ev: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        let at = ev.position();
                        this.view_menu(point(at.x - px(180.), at.y + px(14.)), window, cx)
                    })))
                    .child(
                        div().id("fav").size(px(26.)).rounded(px(7.)).flex_none().flex().items_center().justify_center().cursor_pointer()
                            .hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed()))
                            .child(if fav { icon("star-fill", 16., Theme::star()) } else { icon("star", 16., Theme::icon()) })
                            .tip(if fav { t("取消收藏", "Remove from favourites") } else { t("收藏此文件夹", "Add to favourites") }, None)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let p = this.state.path.clone();
                                cx.global_mut::<state::Shared>().config.toggle_favourite(&p);
                                cx.emit(PaneEvent::Changed);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// One grey line under the title with the folder's note; click to edit.
    fn render_note(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let notes = &state::shared(cx).config.notes;
        if notes.is_empty() { return None }
        let text = grids_core::notes::get(notes, &self.state.path)?;
        let line: SharedString = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("  ").into();
        let tip = format!("{}\n— {}", crate::ui::wrap_tip(text, 56), tr(cx, "点击编辑", "Click to edit"));
        let path = self.state.path.clone();
        Some(
            div().id("pane-note").flex_none().h(px(24.)).mx(px(8.)).mb(px(2.)).px(px(6.)).rounded(px(6.)).flex().items_center().gap(px(6.)).cursor_pointer()
                .text_size(px(12.)).text_color(Theme::text_caption())
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(icon("sticky-note", 13., Theme::star()))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(line)))
                .tip_label(tip, crate::ui::TipSide::Below)
                .on_click(cx.listener(move |_, _, _, cx| { cx.stop_propagation(); cx.emit(PaneEvent::EditNote(path.clone())) })),
        )
    }

    fn render_filter(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (input, _) = self.filter.as_ref()?;
        Some(
            div().h(px(FIELD_H)).flex_none().mx(px(10.)).mb(px(4.)).px(px(10.)).gap(px(8.)).rounded(px(9.)).flex().items_center()
                .bg(Theme::fill_hover())
                .child(icon("search", 15., Theme::icon()))
                .child(div().flex_1().min_w_0().child(input.clone()))
                .child(div().id("filter-x").size(px(22.)).rounded(px(6.)).flex().items_center().justify_center().cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
                    .child(icon("x", 14., Theme::icon()))
                    .on_click(cx.listener(|this, _, window, cx| this.end_filter(window, cx)))),
        )
    }

    fn render_columns(&self, widths: [f32; 4], cx: &mut Context<Self>) -> impl IntoElement {
        let id = cx.entity_id();
        let en = state::en(cx);
        let cols: [(SortKey, &str, f32, bool); 4] = [
            (SortKey::Name, if en { "Name" } else { "名称" }, widths[0], false),
            (SortKey::Modified, if en { "Date modified" } else { "修改日期" }, widths[1], false),
            (SortKey::Type, if en { "Type" } else { "类型" }, widths[2], false),
            (SortKey::Size, if en { "Size" } else { "大小" }, widths[3], true),
        ];
        let mut row = div().relative().h(px(COLS_H)).flex_none().flex().items_center().px(px(ROW_X)).text_size(px(theme::UI_SMALL)).text_color(Theme::list_header());
        row = row.group("cols");
        let header_bounds = self.header_bounds.clone();
        row = row.child(canvas(move |b, _, _| header_bounds.set(b), |_, _, _, _| {}).absolute().size_full());
        for (i, (key, label, w, right)) in cols.into_iter().enumerate() {
            if w <= 0. { continue }
            let sorted = self.state.sort == key;
            let arrow = if self.state.ascending { "arrow-up-narrow-wide" } else { "arrow-down-narrow-wide" };
            row = row.child(
                div().id(("col", i)).relative().flex_none().w(px(w)).h(px(24.)).px(px(CELL_PAD)).rounded(px(6.)).flex().items_center().gap(px(4.)).cursor_pointer()
                    .when(right, |d| d.justify_end())
                    .when(i == 0, |d| d.pl(px(CELL_PAD + 2.)))
                    .hover(|s| s.bg(Theme::fill_hover()).text_color(Theme::text()))
                    .when(sorted, |d| d.text_color(Theme::text_caption()))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label.to_string()))
                    .when(sorted, |d| d.child(icon(arrow, 12., Theme::text_caption())))
                    .on_click(cx.listener(move |this, _, _, cx| this.sort_by(key, cx)))
                    .child(
                        div().id(("col-resize", i)).absolute().right(px(-4.)).top_0().h_full().w(px(8.)).flex().justify_center().cursor(CursorStyle::ResizeColumn)
                            .child(div().w(px(1.)).h(px(12.)).mt(px(6.)).invisible().group_hover("cols", |s| s.visible()).bg(Theme::line_border()))
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_drag(ColResize { pane: id, col: i }, |d, _, _, cx| cx.new(|_| d.clone())),
                    ),
            );
        }
        row
    }

    /// What every item does with the mouse, in any view: select, open,
    /// context menu, drag, and folders take drops.
    fn item_events(&self, el: Stateful<Div>, r: usize, e: &Entry, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let lang = state::lang(cx);
        let path = e.path.clone();
        let is_dir = e.is_dir;
        // Read-only folders open and copy, but their items never start a drag.
        let draggable = state::locked_root(cx, &self.state.path).is_none();
        let drag = if selected { self.state.selected_paths() } else { vec![e.path.clone()] };
        let label: SharedString = if drag.len() > 1 { format!("{} {}", drag.len(), if lang == format::Lang::En { "items" } else { "项" }).into() } else { e.name.clone().into() };
        let payload = DraggedFiles { paths: drag, from: self.state.path.clone(), label };
        el
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.menu = None;
                // Touching the pane accepts the changes counted so far.
                this.ack_changes();
                this.press_row = Some(r);
                window.focus(&this.focus);
                let m = ev.modifiers;
                if ev.click_count >= 2 && !m.control && !m.shift { return this.open_row(r, cx) }
                this.state.press(r, m.control || m.platform, m.shift);
                cx.notify();
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(move |this, ev: &MouseUpEvent, _, cx| {
                let m = ev.modifiers;
                let pressed = this.press_row.take() == Some(r);
                if pressed && !m.control && !m.shift && !cx.has_active_drag() && this.state.selected_count() > 1 && this.state.is_selected(r) {
                    this.state.click(Some(r), false, false);
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.context_menu(Some(r), ev.position, window, cx);
            }))
            .when(draggable, |d| d.on_drag(payload, |p, _, _, cx| cx.new(|_| p.clone())))
            .when(is_dir, |d| {
                let p1 = path.clone();
                let p2 = path.clone();
                d.drag_over::<DraggedFiles>(|s, _, _, _| s.bg(Theme::drop_target()).border_1().border_color(Theme::split_active()))
                    .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(Theme::drop_target()).border_1().border_color(Theme::split_active()))
                    .on_drop(cx.listener(move |this, d: &DraggedFiles, window, cx| this.drop_files(d, Some(p1.clone()), window, cx)))
                    .on_drop(cx.listener(move |this, d: &ExternalPaths, _, cx| this.drop_external(d, Some(p2.clone()), cx)))
            })
            .into_any_element()
    }

    /// One item of the list and icon views: a cell `g.cell_w` wide with the
    /// picture over (icons) or beside (list) the name.
    fn render_item(&self, r: usize, g: Grid, focused: bool, scale: f32, keys: &mut HashSet<thumbs::Key>, cx: &mut Context<Self>) -> AnyElement {
        let Some(e) = self.state.row(r) else { return div().w(px(g.cell_w)).into_any_element() };
        let selected = self.state.is_selected(r);
        let cursor = self.state.cursor == Some(r) && focused;
        let kind = e.kind();
        let renaming = self.rename.as_ref().filter(|rn| rn.path == e.path);
        let name_color = if e.hidden { Theme::text_caption() } else { Theme::text() };
        let wash = self.fresh_wash(&e.name).or_else(|| self.mark_wash(&e.name)).filter(|_| !selected);
        let hl = |d: Div| -> Div {
            d.map(|d| if selected { d.bg(if focused && self.active { Theme::sidebar_selected() } else { Theme::sidebar_hover() }) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
                .when_some(wash, |d, c| d.bg(c))
                .when(cursor && !selected, |d| d.border_1().border_color(Theme::line_border()))
        };
        let rename_box = |rn: &RenameState| div().w_full().min_w_0().h(px(24.)).px(px(4.)).rounded(px(5.)).bg(Theme::bg_body()).border_1().border_color(Theme::field_edge()).flex().items_center().child(rn.input.clone());
        if g.icon <= ICON {
            // List: small icon and one line of name.
            let glyph = match type_img(kind, theme::dark()) {
                Some(path) => img(path).size(px(ICON)).object_fit(ObjectFit::Contain).flex_none().into_any_element(),
                None => match state::file_icon(&e.path, e.is_dir, &e.ext, cx) {
                    Some(image) => img(image).size(px(ICON)).flex_none().into_any_element(),
                    None => icon(theme::kind_icon(kind), ICON, theme::kind_color(kind)).into_any_element(),
                },
            };
            let inner = hl(div().size_full().rounded(px(ROW_R)).pl(px(CELL_PAD - 2.)).pr(px(CELL_PAD)).flex().items_center().gap(px(9.)))
                .child(glyph)
                .child(match renaming {
                    Some(rn) => rename_box(rn).flex_1().into_any_element(),
                    None => div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(name_color)
                        .child(ui_text(e.name.clone()).hit(filter_hit(&e.name, &self.state.filter), Theme::primary())).into_any_element(),
                })
                .children(note_mark(e, r, cx));
            let cell = div().id(("row", r)).flex_none().w(px(g.cell_w)).h(px(g.cell_h)).px(px(2.)).child(inner);
            return self.item_events(cell, r, e, selected, cx);
        }
        let edge = ((g.icon * scale).round() as u32).min(256);
        let key: thumbs::Key = (e.path.clone(), e.modified, edge);
        let thumb = thumbs::get(&key, cx).flatten();
        keys.insert(key);
        let picture = match thumb {
            Some(image) => img(image).size(px(g.icon)).object_fit(ObjectFit::Contain).into_any_element(),
            None => div().size(px(g.icon)).flex().items_center().justify_center()
                .child(icon(theme::kind_icon(kind), (g.icon * 0.72).round(), theme::kind_color(kind))).into_any_element(),
        };
        let name = match renaming {
            Some(rn) => rename_box(rn).into_any_element(),
            None => {
                // Two centred lines at most, cut with an ellipsis.
                let hl: Vec<_> = filter_hit(&e.name, &self.state.filter).map(|r| (r, HighlightStyle { color: Some(Theme::primary()), ..Default::default() })).into_iter().collect();
                div().w_full().px(px(4.)).text_center().line_clamp(2).text_ellipsis().line_height(px(18.)).text_color(name_color)
                    .child(StyledText::new(e.name.clone()).with_highlights(hl)).into_any_element()
            }
        };
        let mark = note_mark(e, r, cx).map(|m| div().absolute().top(px(2.)).right(px(-6.)).child(m));
        let inner = hl(div().size_full().rounded(px(8.)).pt(px(8.)).px(px(4.)).flex().flex_col().items_center().gap(px(6.)))
            .child(div().flex_none().relative().child(picture).children(mark))
            .child(name);
        let cell = div().id(("row", r)).flex_none().w(px(g.cell_w)).h(px(g.cell_h)).p(px(2.)).child(inner);
        self.item_events(cell, r, e, selected, cx)
    }

    fn render_row(&self, r: usize, widths: [f32; 4], focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(e) = self.state.row(r) else { return div().into_any_element() };
        let lang = state::lang(cx);
        let offset = state::shared(cx).utc_offset;
        let selected = self.state.is_selected(r);
        let cursor = self.state.cursor == Some(r) && focused;
        let kind = e.kind();
        let renaming = self.rename.as_ref().filter(|rn| rn.path == e.path);
        let sys_icon = state::file_icon(&e.path, e.is_dir, &e.ext, cx);
        let glyph = match type_img(kind, theme::dark()) {
            Some(path) => img(path).size(px(ICON)).object_fit(ObjectFit::Contain).flex_none().into_any_element(),
            None => match sys_icon {
                Some(image) => img(image).size(px(ICON)).flex_none().into_any_element(),
                None => icon(theme::kind_icon(kind), ICON, theme::kind_color(kind)).into_any_element(),
            },
        };
        let name_cell = div().flex_none().w(px(widths[0])).h_full().pl(px(CELL_PAD - 2.)).pr(px(CELL_PAD)).flex().items_center().gap(px(9.)).child(glyph).map(|d| match renaming {
            Some(rn) => d.child(div().flex_1().min_w_0().h(px(24.)).px(px(4.)).rounded(px(5.)).bg(Theme::bg_body()).border_1().border_color(Theme::field_edge()).flex().items_center().child(rn.input.clone())),
            None => d.child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(if e.hidden { Theme::text_caption() } else { Theme::text() }).child(ui_text(e.name.clone()).hit(filter_hit(&e.name, &self.state.filter), Theme::primary())))
                .children(note_mark(e, r, cx)),
        });
        let cell = |w: f32, text: String, right: bool| div().flex_none().w(px(w)).px(px(CELL_PAD)).overflow_hidden().whitespace_nowrap().text_ellipsis().when(right, |d| d.flex().justify_end()).child(ui_text(text));
        let size = if e.is_dir { String::new() } else { format::size(e.size, lang) };
        let wash = self.fresh_wash(&e.name).or_else(|| self.mark_wash(&e.name));
        let row = div()
            .id(("row", r))
            .h(px(ROW_H))
            .mx(px(ROW_X))
            .rounded(px(ROW_R))
            .flex()
            .items_center()
            .text_color(Theme::text_caption())
            .map(|d| if selected { d.bg(if focused && self.active { Theme::sidebar_selected() } else { Theme::sidebar_hover() }) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
            .when_some(wash.filter(|_| !selected), |d, c| d.bg(c))
            .when(cursor && !selected, |d| d.border_1().border_color(Theme::line_border()))
            .child(name_cell)
            .when(widths[1] > 0., |d| d.child(cell(widths[1], short_date(format::date(e.modified, offset), widths[1]), false)))
            .when(widths[2] > 0., |d| d.child(cell(widths[2], format::type_name(e, lang), false)))
            .when(widths[3] > 0., |d| d.child(cell(widths[3], size, true)));
        self.item_events(row, r, e, selected, cx)
            .into_any_element()
    }
}

impl PaneView {
    /// Ctrl+wheel over the list steps the view (up: bigger), caught before
    /// the list scrolls.
    fn ctrl_wheel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let acc = self.wheel.clone();
        canvas(|_, _, _| {}, move |bounds, _, window, _| {
            window.on_mouse_event(move |ev: &ScrollWheelEvent, phase, _, cx| {
                if phase != DispatchPhase::Capture || !ev.modifiers.secondary() || !bounds.contains(&ev.position) { return }
                cx.stop_propagation();
                let dy = match ev.delta { ScrollDelta::Lines(l) => l.y * 40., ScrollDelta::Pixels(p) => f32::from(p.y) };
                let a = acc.get() + dy;
                if a.abs() < 40. { return acc.set(a) }
                acc.set(0.);
                let _ = weak.update(cx, |this, cx| { let v = this.view.step(a > 0.); this.set_view(v, cx) });
            });
        }).absolute().size_full()
    }
}

impl Render for PaneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let avail = f32::from(self.list_bounds.get().size.width);
        let avail = if avail <= 1. { 600. } else { avail };
        let widths = self.fit_columns(avail);
        // Fading highlights draw frame by frame, only while there are any.
        if !self.fresh.is_empty() {
            self.fresh.retain(|_, t| t.elapsed() < FRESH);
            if !self.fresh.is_empty() { window.request_animation_frame() }
        }
        let n = self.state.len();
        let en = state::en(cx);
        let grid = grid_metrics(self.view, avail);
        self.grid_cols = grid.map_or(1, |g| g.cols);
        let scale = window.scale_factor();
        let pane_id = cx.entity_id();
        let rows = grid.map_or(n, |g| n.div_ceil(g.cols));
        // A locked folder is drawn under a veil; its items never start a drag.
        let mut list = uniform_list("files", rows, cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
            let focused = this.focus.contains_focused(window, cx) || this.menu.is_some() || this.rename.is_some();
            let Some(g) = grid else {
                thumbs::set_visible(pane_id, HashSet::new(), cx);
                return range.map(|r| this.render_row(r, widths, focused, cx)).collect();
            };
            let mut keys = HashSet::new();
            let out = range.map(|row| {
                let items = (row * g.cols..((row + 1) * g.cols).min(n)).map(|r| this.render_item(r, g, focused, scale, &mut keys, cx)).collect::<Vec<_>>();
                div().flex().px(px(ROW_X)).children(items).into_any_element()
            }).collect();
            thumbs::set_visible(pane_id, keys, cx);
            out
        }))
        .track_scroll(self.scroll.clone())
        .size_full()
        .pb(px(6.));
        // A locked folder is drawn under a veil; its items never start a drag.
        let locked = state::locked_root(cx, &self.state.path);
        if locked.is_some() { list = list.opacity(0.5) }
        let list_bounds = self.list_bounds.clone();
        let empty: Option<AnyElement> = if let Some(err) = &self.state.error {
            Some(div().absolute().top(px(24.)).left_0().right_0().px(px(12.)).flex().justify_center().text_color(Theme::text_caption()).child(err.clone()).into_any_element())
        } else if n == 0 && !self.loading {
            let msg = if !self.state.filter.is_empty() { if en { "No matching items." } else { "没有匹配的项目。" } } else if en { "This folder is empty." } else { "此文件夹为空。" }.to_string();
            let pick = |light: &'static str, dark: &'static str| -> Option<&'static str> {
                if theme::dark() && theme::has(dark) { Some(dark) } else if theme::has(light) { Some(light) } else { None }
            };
            let art = if self.state.filter.is_empty() { pick("empty-2x.png", "empty-dark-2x.png") } else { pick("empty-search.png", "empty-search-dark.png") }
                .map(|n| SharedString::from(format!("theme/{n}")))
                .map(|n| img(n).w(px(180.)).h(px(120.)).object_fit(ObjectFit::Contain).into_any_element());
            Some(div().absolute().inset_0().flex().flex_col().items_center().justify_center().gap(px(10.)).px(px(12.))
                .children(art)
                .child(div().text_color(Theme::text_caption()).child(msg)).into_any_element())
        } else { None };
        let missing = self.link.as_ref().and_then(|l| l.missing.clone()).map(|m| {
            let art = [("missing.png", "missing-dark.png")]
                .iter()
                .find(|(light, dark)| theme::has(light) && (!theme::dark() || theme::has(dark)))
                .map(|(light, dark)| if theme::dark() { *dark } else { *light });
            div().absolute().inset_0().bg(Theme::bg_body()).flex().flex_col().items_center().justify_center().gap(px(10.)).px(px(16.))
                .children(art.map(|n| img(SharedString::from(format!("theme/{n}"))).w(px(180.)).h(px(120.)).object_fit(ObjectFit::Contain)))
                .child(icon("folder-x", 28., Theme::icon_muted()))
                .child(div().text_color(Theme::text()).child(if en { "The matching folder doesn't exist" } else { "对应文件夹不存在" }))
                .child(div().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(Theme::text_caption()).child(ui_text(m.display().to_string())))
                .child(div().id("create-missing").h(px(30.)).px(px(14.)).rounded(px(8.)).flex().items_center().gap(px(6.)).cursor_pointer()
                    .bg(Theme::primary()).text_color(gpui::white()).text_size(px(13.))
                    .hover(|s| s.opacity(0.9))
                    .child(icon("folder-plus", 14., gpui::white()))
                    .child(if en { "Create folder" } else { "创建文件夹" })
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::Link(LinkCmd::CreateMissing)))))
        });
        // A locked folder takes no clicks at all: nothing to select, nothing to
        // drag, nothing to drop onto.
        let brand = state::brand(cx);
        let lock_veil = locked.as_ref().map(|root| {
            let tip: SharedString = if root == &self.state.path {
                if en { format!("Locked folder (inside {brand} only) · unlock with the lock button in the title bar").into() } else { format!("文件夹已锁定（仅在{brand}内生效）· 点标题栏的锁图标解锁").into() }
            } else if en { format!("Locked at “{}” · unlock with the lock button in the title bar", root.display()).into() } else { format!("由「{}」锁定 · 点标题栏的锁图标解锁", root.display()).into() };
            let root2 = root.clone();
            div().id("lock-veil").absolute().inset_0().occlude()
                .bg(Theme::bg_body().alpha(0.30))
                .flex().flex_col().items_center().justify_center().gap(px(8.))
                .child(icon("lock", 26., Theme::primary()))
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text())
                    .child(if en { "This folder is locked" } else { "此文件夹已锁定" }))
                .child(div().text_size(px(12.)).text_color(Theme::text_caption())
                    .child(if en { "Unlock it to select, drag or copy" } else { "解锁后才能选中、拖动或复制" }))
                .tip(&tip, None)
                .on_drop(cx.listener(move |_, _: &DraggedFiles, _, cx| cx.emit(PaneEvent::Locked(root2.clone()))))
        });
        // The box being dragged out, in the body's coordinates.
        let marquee_box = self.marquee.as_ref().map(|m| {
            let bounds = self.list_bounds.get();
            let (ax, ay, bx, by) = (f32::from(m.start.x), f32::from(m.start.y), f32::from(m.cur.x), f32::from(m.cur.y));
            let (x0, x1) = (ax.min(bx), ax.max(bx));
            let (y0, y1) = (ay.min(by), ay.max(by));
            div().absolute()
                .left(px(x0 - f32::from(bounds.origin.x)))
                .top(px(y0 - f32::from(bounds.origin.y)))
                .w(px((x1 - x0).max(1.))).h(px((y1 - y0).max(1.)))
                .border_1().border_color(Theme::primary()).bg(Theme::primary().alpha(0.12)).rounded(px(2.))
        });
        let body = div()
            .id("list-body")
            .relative()
            .flex_1()
            .min_h_0()
            .pt(px(2.))
            // Columns are fitted to last frame's width: when it changes (the
            // pane was resized), lay out again or the header and rows stay
            // fitted to the old width, cut at the pane's edge.
            .child(canvas(move |b, window, _| {
                let old = list_bounds.replace(b);
                if (old.size.width - b.size.width).abs() > px(0.5) { window.request_animation_frame() }
            }, |_, _, _, _| {}).absolute().size_full())
            .child(self.ctrl_wheel(cx))
            .child(self.marquee_track(cx))
            .child(list)
            .children(marquee_box)
            .children(lock_veil)
            .children(empty)
            .children(missing)
            .drag_over::<DraggedFiles>(|s, _, _, _| s.bg(Theme::drop_target()))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(Theme::drop_target()))
            .on_drop(cx.listener(|this, d: &DraggedFiles, window, cx| this.drop_files(d, None, window, cx)))
            .on_drop(cx.listener(|this, d: &ExternalPaths, _, cx| this.drop_external(d, None, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                this.menu = None;
                this.ack_changes();
                window.focus(&this.focus);
                this.state.click(None, ev.modifiers.control, ev.modifiers.shift);
                // Pressing empty space starts a rubber-band selection.
                this.press_row = None;
                let base = if ev.modifiers.control { this.state.selected_indices() } else { Vec::new() };
                this.marquee = Some(Marquee { start: ev.position, cur: ev.position, base });
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.context_menu(None, ev.position, window, cx)));

        div()
            .id("pane")
            .key_context("Pane")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .group("pane")
            .bg(Theme::bg_body())
            .font_family(theme::UI_FONT)
            .text_size(px(theme::UI_SIZE))
            .text_color(Theme::text())
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(PaneEvent::Activated)))
            .on_mouse_down(MouseButton::Right, cx.listener(|_, _, _, cx| cx.emit(PaneEvent::Activated)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| this.type_to_select(ev, window, cx)))
            .on_action(cx.listener(|this, _: &Open, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &GoUp, _, cx| this.go_up(cx)))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|this, _: &Rename, window, cx| this.start_rename(window, cx)))
            .on_action(cx.listener(|this, _: &Recycle, _, cx| this.ask_delete(false, cx)))
            .on_action(cx.listener(|this, _: &DeletePermanently, _, cx| this.ask_delete(true, cx)))
            .on_action(cx.listener(|this, _: &CopyFiles, _, cx| this.copy_files(false, cx)))
            .on_action(cx.listener(|this, _: &CutFiles, _, cx| this.copy_files(true, cx)))
            .on_action(cx.listener(|this, _: &PasteFiles, window, cx| this.paste(window, cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.load(None, cx)))
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| this.new_folder(window, cx)))
            .on_action(cx.listener(|this, _: &EditAddress, window, cx| this.edit_address(window, cx)))
            .on_action(cx.listener(|this, _: &FilterNames, window, cx| this.start_filter(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| this.toggle_hidden(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| { if this.here_locked(cx) { return } this.state.select_all(); cx.notify() }))
            .on_action(cx.listener(|this, _: &CursorUp, _, cx| this.move_rows(-1, false, false, cx)))
            .on_action(cx.listener(|this, _: &CursorDown, _, cx| this.move_rows(1, false, false, cx)))
            .on_action(cx.listener(|this, _: &CursorLeft, _, cx| this.move_side(-1, false, cx)))
            .on_action(cx.listener(|this, _: &CursorRight, _, cx| this.move_side(1, false, cx)))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.move_side(-1, true, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.move_side(1, true, cx)))
            .on_action(cx.listener(|this, _: &ViewHuge, _, cx| this.set_view(ViewMode::Huge, cx)))
            .on_action(cx.listener(|this, _: &ViewLarge, _, cx| this.set_view(ViewMode::Large, cx)))
            .on_action(cx.listener(|this, _: &ViewMedium, _, cx| this.set_view(ViewMode::Medium, cx)))
            .on_action(cx.listener(|this, _: &ViewList, _, cx| this.set_view(ViewMode::List, cx)))
            .on_action(cx.listener(|this, _: &ViewDetails, _, cx| this.set_view(ViewMode::Details, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| { let p = this.page(); this.move_cursor(-p, false, false, false, cx) }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| { let p = this.page(); this.move_cursor(p, false, false, false, cx) }))
            .on_action(cx.listener(|this, _: &First, _, cx| this.move_cursor(-1, true, false, false, cx)))
            .on_action(cx.listener(|this, _: &Last, _, cx| this.move_cursor(1, true, false, false, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.move_rows(-1, true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.move_rows(1, true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| { let p = this.page(); this.move_cursor(-p, false, true, false, cx) }))
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| { let p = this.page(); this.move_cursor(p, false, true, false, cx) }))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.move_cursor(-1, true, true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.move_cursor(1, true, true, false, cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_rows(-1, false, true, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_rows(1, false, true, cx)))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.menu.take().is_some() { return cx.notify() }
                if this.filter.is_some() { return this.end_filter(window, cx) }
                if this.maximized.is_some() { return cx.emit(PaneEvent::RestoreGrid) }
                this.state.clear_selection();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyPath, _, cx| this.copy_paths(cx)))
            .on_action(cx.listener(|this, _: &AddToShelf, _, cx| this.shelve(cx)))
            .on_action(cx.listener(|this, _: &Properties, window, cx| this.run_cmd(Cmd::Properties, None, Point::default(), window, cx)))
            .on_action(cx.listener(|this, _: &ContextMenu, window, cx| {
                let b = this.list_bounds.get();
                let r = this.state.cursor.filter(|&r| this.state.is_selected(r));
                let y = r.map_or(px(40.), |r| px(((r as f32 + 1.) * ROW_H).min(f32::from(b.size.height) - 10.)));
                let pos = point(b.origin.x + px(40.), b.origin.y + y);
                this.context_menu(r, pos, window, cx)
            }))
            .on_drag_move::<ColResize>(cx.listener(move |this, ev: &DragMoveEvent<ColResize>, _, cx| {
                let d = ev.drag(cx);
                if d.pane != pane_id { return }
                let col = d.col;
                let hb = this.header_bounds.get();
                let avail = f32::from(this.list_bounds.get().size.width);
                let widths = this.fit_columns(avail);
                let c = &mut this.columns;
                let left: f32 = f32::from(hb.origin.x) + ROW_X + widths[..col].iter().sum::<f32>();
                let w = (f32::from(ev.event.position.x) - left).clamp(Columns::MIN, 1200.);
                match col { 0 => c.name = w, 1 => c.modified = w, 2 => c.kind = w, _ => c.size = w }
                cx.emit(PaneEvent::Changed);
                cx.notify();
            }))
            .child(self.render_header(cx))
            .children(self.render_note(cx))
            .children(self.render_filter(cx))
            .when(grid.is_none(), |d| d.child(self.render_columns(widths, cx)))
            .child(body)
            .children(self.menu.as_ref().map(|(m, _)| deferred(anchored().position(point(px(0.), px(0.))).child(m.clone())).with_priority(5)))
    }
}

/// What opening a file amounts to once a shortcut is resolved.
#[derive(Debug, PartialEq)]
enum OpenPlan {
    /// Show this folder in the pane.
    Enter(PathBuf),
    /// Hand this path to the system (a shortcut itself, so Windows applies its arguments).
    System(PathBuf),
    /// A shortcut whose target is gone.
    Missing(PathBuf),
    Failed(String),
}

fn open_plan(path: &Path, link: Option<std::io::Result<grids_shell::LinkTarget>>, enter: bool) -> OpenPlan {
    use grids_shell::LinkTarget;
    match link {
        Some(Ok(LinkTarget::Folder(dir))) if enter => OpenPlan::Enter(dir),
        Some(Ok(LinkTarget::Missing(t))) => OpenPlan::Missing(t),
        _ => OpenPlan::System(path.to_path_buf()),
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenPlan, open_plan};
    use grids_shell::LinkTarget;
    use std::path::Path;

    #[test]
    fn shortcut_plans() {
        let lnk = Path::new("a.lnk");
        assert_eq!(open_plan(lnk, Some(Ok(LinkTarget::Folder("d".into()))), true), OpenPlan::Enter("d".into()));
        assert_eq!(open_plan(lnk, Some(Ok(LinkTarget::Folder("d".into()))), false), OpenPlan::System(lnk.into()));
        assert_eq!(open_plan(lnk, Some(Ok(LinkTarget::File("f".into()))), true), OpenPlan::System(lnk.into()));
        assert_eq!(open_plan(lnk, Some(Ok(LinkTarget::Missing("m".into()))), true), OpenPlan::Missing("m".into()));
        assert_eq!(open_plan(lnk, Some(Ok(LinkTarget::Other)), true), OpenPlan::System(lnk.into()));
        assert_eq!(open_plan(lnk, Some(Err(std::io::Error::other("bad"))), true), OpenPlan::System(lnk.into()));
        assert_eq!(open_plan(Path::new("x.txt"), None, true), OpenPlan::System("x.txt".into()));
    }
}



