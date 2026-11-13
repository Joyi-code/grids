//! One pane of the grid: header (number, folder, back / forward / up, star),
//! breadcrumb address bar, optional name filter, column header and the
//! virtualised file list. Folder reads run on a background thread; the
//! folder shown is watched and reloads when it changes.

use crate::input::{InputEvent, TextInput};
use crate::state::{self, tr};
use crate::theme::{self, Theme};
use crate::menu::{Cmd, ContextMenu as MenuView, Glyph, Item, MenuEvent};
use crate::ui::{TipExt, icon, ui_text};
use futures::StreamExt;
use gpui::{prelude::FluentBuilder, *};
use grids_core::format;
use grids_core::listing::{self, Entry};
use grids_core::paths;
use grids_core::session::{Columns, PaneSave};
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
        CopyPath, Properties
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
        KeyBinding::new("alt-enter", Properties, c),
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


pub struct PaneView {
    /// Position in the grid (shown as number + 1).
    pub index: usize,
    pub active: bool,
    pub state: PaneState,
    pub columns: Columns,
    pub focus: FocusHandle,
    scroll: UniformListScrollHandle,
    list_bounds: Rc<Cell<Bounds<Pixels>>>,
    header_bounds: Rc<Cell<Bounds<Pixels>>>,
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
            focus,
            scroll: UniformListScrollHandle::new(),
            list_bounds: Rc::default(),
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
        };
        me.load(None, cx);
        me
    }

    pub fn save(&self) -> PaneSave {
        PaneSave { path: self.state.path.clone(), sort: self.state.sort, ascending: self.state.ascending, show_hidden: self.state.show_hidden, columns: self.columns.clone() }
    }

    // ---- loading

    /// Reads the folder again on a background thread; `select` picks an
    /// entry by name once it is in.
    pub fn load(&mut self, select: Option<String>, cx: &mut Context<Self>) {
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
                        this.state.set_entries(entries, select.as_deref());
                        let mut jump = select.is_some();
                        if let Some(names) = this.pending_select.take() { this.state.select_names(&names); jump = true }
                        if let Some(r) = this.state.cursor.filter(|_| jump) { this.scroll.scroll_to_item(r, ScrollStrategy::Center) }
                    }
                    Err(e) => this.state.set_error(error_text(&e, state::en(cx))),
                }
                cx.notify();
            });
        }));
        cx.notify();
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
                if this.update(cx, |this, cx| this.load(None, cx)).is_err() { break }
            }
        }));
    }

    // ---- navigation

    pub fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.close_editors();
        if self.state.navigate(path) { self.after_move(None, cx) } else { self.load(None, cx) }
    }

    fn after_move(&mut self, select: Option<String>, cx: &mut Context<Self>) {
        self._watcher = None;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.load(select, cx);
        cx.emit(PaneEvent::Changed);
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

    fn page(&self) -> isize { ((f32::from(self.list_bounds.get().size.height) / ROW_H).floor() as isize - 1).max(1) }

    fn move_cursor(&mut self, delta: isize, to_end: bool, shift: bool, ctrl: bool, cx: &mut Context<Self>) {
        self.state.move_cursor(delta, to_end, shift, ctrl);
        if let Some(r) = self.state.cursor { self.scroll.scroll_to_item(r, ScrollStrategy::Top) }
        cx.notify();
    }

    fn type_to_select(&mut self, ev: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) {
        // Only when the list itself has focus, not one of the pane's text fields.
        if !self.focus.is_focused(window) { return }
        let k = &ev.keystroke;
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform || self.editing() { return }
        let Some(ch) = k.key_char.as_ref().filter(|c| c.chars().all(|c| !c.is_control()) && !c.is_empty()) else { return };
        if self.type_buf.1.elapsed() > Duration::from_millis(900) { self.type_buf.0.clear() }
        self.type_buf.0.push_str(ch);
        self.type_buf.1 = Instant::now();
        if let Some(r) = self.state.find_prefix(&self.type_buf.0) {
            self.state.click(Some(r), false, false);
            self.scroll.scroll_to_item(r, ScrollStrategy::Top);
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

    fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.state.cursor_entry().cloned().or_else(|| self.state.selection().first().map(|e| (*e).clone())) else { return };
        self.state.select_name(&e.name);
        if let Some(r) = self.state.cursor { self.scroll.scroll_to_item(r, ScrollStrategy::Top) }
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
        match grids_shell::rename(&r.path, &name) {
            Ok(_) => {
                self.load(Some(name), cx);
                cx.emit(PaneEvent::FsChanged(vec![self.state.path.clone()]));
            }
            Err(e) => cx.emit(PaneEvent::Toast(format!("{}{e}", tr(cx, "重命名失败：", "Rename failed: ")))),
        }
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = tr(cx, "新建文件夹", "New folder");
        let name = paths::unique_name(&self.state.path, base);
        match std::fs::create_dir(self.state.path.join(&name)) {
            Ok(()) => {
                // Add the folder now so it can be renamed at once; the reload confirms it.
                if let Ok(list) = listing::read_dir(&self.state.path) {
                    self.state.set_entries(list, Some(&name));
                    self.start_rename(window, cx);
                }
            }
            Err(e) => cx.emit(PaneEvent::Toast(format!("{}{e}", tr(cx, "无法新建文件夹：", "Cannot create folder: ")))),
        }
    }

    // ---- file operations

    fn copy_files(&mut self, cut: bool, cx: &mut Context<Self>) {
        let t = self.state.targets();
        if t.is_empty() { return }
        let n = t.len();
        state::set_clipboard(t, cut, cx);
        let msg = if state::en(cx) { format!("{} {n} item(s)", if cut { "Cut" } else { "Copied" }) } else { format!("已{} {n} 项", if cut { "剪切" } else { "复制" }) };
        cx.emit(PaneEvent::Toast(msg));
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        let dest = self.state.path.clone();
        self.paste_into(dest, cx)
    }

    fn paste_into(&mut self, dest: PathBuf, cx: &mut Context<Self>) {
        let Some((paths, cut)) = state::get_clipboard(cx) else { return };
        let names: Vec<String> = paths.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        if cut { cx.global_mut::<state::Shared>().clipboard = None }
        self.run_op(if cut { Op::Move } else { Op::Copy }, paths, Some(dest), names, cx);
    }

    /// Runs a copy / move / delete on a background thread (Windows shows its
    /// own progress and confirmation), then refreshes the folders involved.
    fn run_op(&mut self, op: Op, paths: Vec<PathBuf>, dest: Option<PathBuf>, select: Vec<String>, cx: &mut Context<Self>) {
        if paths.is_empty() { return }
        let mut dirs: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
        if let Some(d) = &dest { dirs.push(d.clone()) }
        dirs.sort();
        dirs.dedup();
        let en = state::en(cx);
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move {
                match (op, dest) {
                    (Op::Copy, Some(d)) => grids_shell::copy(&paths, &d),
                    (Op::Move, Some(d)) => grids_shell::move_(&paths, &d),
                    (Op::Recycle, _) => grids_shell::recycle(&paths),
                    (Op::Delete, _) => grids_shell::delete(&paths),
                    _ => Ok(()),
                }
            }).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = res && e.kind() != std::io::ErrorKind::Interrupted { cx.emit(PaneEvent::Toast(format!("{}{e}", if en { "Operation failed: " } else { "操作失败：" }))) }
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
            self.load(None, cx);
        }
    }

    fn drop_files(&mut self, d: &DraggedFiles, into: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let dest = into.unwrap_or_else(|| self.state.path.clone());
        if d.from == dest || d.paths.iter().any(|p| p == &dest || dest.starts_with(p)) { return }
        let copy = window.modifiers().control || !d.paths.first().is_some_and(|p| grids_shell::same_volume(p, &dest));
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

    // ---- context menu

    /// Opens the GPUI menu for the selection (`row`) or the folder background.
    fn context_menu(&mut self, row: Option<usize>, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = row && !self.state.is_selected(r) { self.state.click(Some(r), false, false) }
        if row.is_none() { self.state.clear_selection() }
        let en = state::en(cx);
        let t = move |zh: &'static str, e: &'static str| -> SharedString { (if en { e } else { zh }).into() };
        let can_paste = state::get_clipboard(cx).is_some();
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
            v.push(Item::row(t("剪切", "Cut"), "scissors", "Ctrl+X", Cmd::Cut));
            v.push(Item::row(t("复制", "Copy"), "copy", "Ctrl+C", Cmd::Copy));
            if let Some(dir) = &folder { v.push(Item::row(t("粘贴到此文件夹", "Paste into folder"), "clipboard-paste", "", Cmd::PasteInto(dir.clone())).enabled(can_paste)) }
            v.push(Item::row(t("复制路径", "Copy as path"), "link", "Ctrl+Shift+C", Cmd::CopyPath));
            v.push(Item::row(t("重命名", "Rename"), "pencil", "F2", Cmd::Rename).enabled(one));
            v.push(Item::row(t("删除", "Delete"), "trash-2", "Delete", Cmd::Delete));
            v.push(Item::Sep);
            v.push(Item::row(t("压缩为 ZIP 文件", "Compress to ZIP file"), "file-archive", "", Cmd::Zip));
            if let Some(dir) = &folder {
                let fav = state::shared(cx).config.is_favourite(dir);
                v.push(Item::row(if fav { t("从收藏夹移除", "Remove from favourites") } else { t("添加到收藏夹", "Add to favourites") }, if fav { "star-off" } else { "star" }, "", Cmd::ToggleFavourite(dir.clone())));
            }
            v.push(Item::row(t("在资源管理器中显示", "Show in Explorer"), "folder-search", "", Cmd::Reveal));
            v.push(Item::row(t("属性", "Properties"), "info", "Alt+Enter", Cmd::Properties).enabled(cfg!(windows)));
            v.push(Item::Sep);
            v.push(more);
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
            let view = vec![
                Item::sub(t("排序方式", "Sort by"), "arrow-up-down", sort_items),
                Item::check(t("显示隐藏的项目", "Show hidden items"), st.show_hidden, "Ctrl+H", Cmd::ToggleHidden),
            ];
            let mut new_items = vec![Item::row(t("文件夹", "Folder"), "folder", "Ctrl+Shift+N", Cmd::NewFolder), Item::Sep];
            for n in state::new_items(cx).iter() {
                let glyph = n.icon.clone().map_or(Glyph::Icon("file"), Glyph::Image);
                new_items.push(Item::Row { label: n.name.clone().into(), glyph, keys: "", enabled: true, cmd: Some(Cmd::NewFile(n.ext.clone())), sub: None });
            }
            vec![
                Item::sub(t("查看", "View"), "eye", view),
                Item::row(t("刷新", "Refresh"), "refresh-cw", "F5", Cmd::Refresh),
                Item::Sep,
                Item::row(t("粘贴", "Paste"), "clipboard-paste", "Ctrl+V", Cmd::Paste).enabled(can_paste),
                Item::sub(t("新建", "New"), "plus", new_items),
                Item::Sep,
                Item::row(t("在终端中打开", "Open in Terminal"), "terminal", "", Cmd::Terminal),
                Item::row(t("复制文件夹路径", "Copy folder path"), "link", "", Cmd::CopyFolderPath),
                Item::row(t("属性", "Properties"), "info", "", Cmd::FolderProperties).enabled(cfg!(windows)),
                Item::Sep,
                more,
            ]
        };
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
            Cmd::Paste => self.paste(cx),
            Cmd::PasteInto(d) => self.paste_into(d, cx),
            Cmd::CopyPath => self.copy_paths(cx),
            Cmd::Rename => self.start_rename(window, cx),
            Cmd::Delete => self.run_op(Op::Recycle, targets, None, vec![], cx),
            Cmd::Zip => if let Some(p) = first {
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
            Cmd::NewFolder => self.new_folder(window, cx),
            Cmd::NewFile(ext) => match grids_shell::create_new(&dir, &ext) {
                Ok(p) => {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if let Ok(list) = listing::read_dir(&dir) {
                        self.state.set_entries(list, Some(&name));
                        self.start_rename(window, cx);
                    }
                }
                Err(e) => cx.emit(PaneEvent::Toast(format!("{}{e}", if en { "Cannot create file: " } else { "无法新建文件：" }))),
            },
            Cmd::Terminal => bg(self, Box::new(move || grids_shell::open_terminal(&dir)), cx),
            Cmd::CopyFolderPath => {
                cx.write_to_clipboard(ClipboardItem::new_string(dir.to_string_lossy().into_owned()));
                cx.emit(PaneEvent::Toast((if en { "Copied the folder path" } else { "已复制文件夹路径" }).into()));
            }
        }
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

#[derive(Clone, Copy)]
enum Op {
    Copy,
    Move,
    Recycle,
    Delete,
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
            let mut row = div().flex().flex_none().items_center();
            for (i, (label, path)) in crumbs.into_iter().enumerate() {
                let last = i + 1 == n;
                row = row.child(
                    div().id(("crumb", i)).flex_none().h(px(24.)).px(px(4.)).rounded(px(6.)).flex().items_center().whitespace_nowrap().cursor_pointer()
                        .map(|d| if last { d.text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()) } else { d.text_size(px(13.)).text_color(Theme::text_caption()) })
                        .hover(|s| s.bg(Theme::fill_hover()).text_color(Theme::text()))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); this.navigate(path.clone(), cx) })),
                );
                if !last { row = row.child(div().mx(px(1.)).flex_none().child(icon("chevron-right", 12., Theme::icon_muted()))) }
            }
            // Long paths keep their end (the folder) in view.
            div().id("crumbs").flex_1().min_w_0().h(px(FIELD_H)).flex().items_center().justify_end().overflow_hidden().cursor_text()
                .child(row).child(div().flex_1().min_w(px(12.)))
                .on_click(cx.listener(|this, _, window, cx| this.edit_address(window, cx)))
                .into_any_element()
        };
        let quiet = !self.active;
        let btn = |id: &'static str, glyph: &'static str, enabled: bool| {
            div().id(id).size(px(26.)).rounded(px(7.)).flex_none().flex().items_center().justify_center()
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed())))
                .child(icon(glyph, 16., if enabled { Theme::icon() } else { Theme::text_disabled() }))
        };
        div()
            .h(px(HEAD_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(12.))
            .pr(px(8.))
            .child(badge)
            .child(path_area)
            .child(
                div().flex().flex_none().items_center().gap(px(1.))
                    .when(quiet, |d| d.opacity(0.).group_hover("pane", |s| s.opacity(1.)))
                    .child(btn("back", "arrow-left", st.can_back()).tip(t("后退", "Back"), Some("Alt+←")).on_click(cx.listener(|this, _, _, cx| this.go_back(cx))))
                    .child(btn("fwd", "arrow-right", st.can_forward()).tip(t("前进", "Forward"), Some("Alt+→")).on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))))
                    .child(btn("up", "arrow-up", st.can_up()).tip(t("上一级", "Up"), Some("Alt+↑")).on_click(cx.listener(|this, _, _, cx| this.go_up(cx))))
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

    fn render_row(&self, r: usize, widths: [f32; 4], focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(e) = self.state.row(r) else { return div().into_any_element() };
        let lang = state::lang(cx);
        let offset = state::shared(cx).utc_offset;
        let selected = self.state.is_selected(r);
        let cursor = self.state.cursor == Some(r) && focused;
        let kind = e.kind();
        let renaming = self.rename.as_ref().filter(|rn| rn.path == e.path);
        let sys_icon = state::file_icon(&e.path, e.is_dir, &e.ext, cx);
        let glyph = match sys_icon {
            Some(image) => img(image).size(px(ICON)).flex_none().into_any_element(),
            None => icon(theme::kind_icon(kind), ICON, theme::kind_color(kind)).into_any_element(),
        };
        let name_cell = div().flex_none().w(px(widths[0])).h_full().pl(px(CELL_PAD - 2.)).pr(px(CELL_PAD)).flex().items_center().gap(px(9.)).child(glyph).map(|d| match renaming {
            Some(rn) => d.child(div().flex_1().min_w_0().h(px(24.)).px(px(4.)).rounded(px(5.)).bg(Theme::bg_body()).border_1().border_color(Theme::field_edge()).flex().items_center().child(rn.input.clone())),
            None => d.child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(if e.hidden { Theme::text_caption() } else { Theme::text() }).child(ui_text(e.name.clone()).hit(filter_hit(&e.name, &self.state.filter), Theme::primary()))),
        });
        let cell = |w: f32, text: String, right: bool| div().flex_none().w(px(w)).px(px(CELL_PAD)).overflow_hidden().whitespace_nowrap().text_ellipsis().when(right, |d| d.flex().justify_end()).child(ui_text(text));
        let size = if e.is_dir { String::new() } else { format::size(e.size, lang) };
        let path = e.path.clone();
        let is_dir = e.is_dir;
        let drag = if selected { self.state.selected_paths() } else { vec![e.path.clone()] };
        let label: SharedString = if drag.len() > 1 { format!("{} {}", drag.len(), if lang == format::Lang::En { "items" } else { "项" }).into() } else { e.name.clone().into() };
        let payload = DraggedFiles { paths: drag, from: self.state.path.clone(), label };
        div()
            .id(("row", r))
            .h(px(ROW_H))
            .mx(px(ROW_X))
            .rounded(px(ROW_R))
            .flex()
            .items_center()
            .text_color(Theme::text_caption())
            .map(|d| if selected { d.bg(if focused && self.active { Theme::sidebar_selected() } else { Theme::sidebar_hover() }) } else { d.hover(|s| s.bg(Theme::sidebar_hover())) })
            .when(cursor && !selected, |d| d.border_1().border_color(Theme::line_border()))
            .child(name_cell)
            .when(widths[1] > 0., |d| d.child(cell(widths[1], short_date(format::date(e.modified, offset), widths[1]), false)))
            .when(widths[2] > 0., |d| d.child(cell(widths[2], format::type_name(e, lang), false)))
            .when(widths[3] > 0., |d| d.child(cell(widths[3], size, true)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.menu = None;
                window.focus(&this.focus);
                let m = ev.modifiers;
                if ev.click_count >= 2 && !m.control && !m.shift { return this.open_row(r, cx) }
                this.state.press(r, m.control || m.platform, m.shift);
                cx.notify();
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(move |this, ev: &MouseUpEvent, _, cx| {
                let m = ev.modifiers;
                if !m.control && !m.shift && !cx.has_active_drag() && this.state.selected_count() > 1 && this.state.is_selected(r) {
                    this.state.click(Some(r), false, false);
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                this.context_menu(Some(r), ev.position, window, cx);
            }))
            .on_drag(payload, |d, _, _, cx| cx.new(|_| d.clone()))
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
}

impl Render for PaneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let avail = f32::from(self.list_bounds.get().size.width);
        let widths = self.fit_columns(if avail <= 1. { 600. } else { avail });
        let n = self.state.len();
        let en = state::en(cx);
        let _ = window;
        let list = uniform_list("files", n, cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
            let focused = this.focus.contains_focused(window, cx) || this.menu.is_some() || this.rename.is_some();
            range.map(|r| this.render_row(r, widths, focused, cx)).collect()
        }))
        .track_scroll(self.scroll.clone())
        .size_full()
        .pb(px(6.));
        let list_bounds = self.list_bounds.clone();
        let empty_msg: Option<String> = if let Some(err) = &self.state.error { Some(err.clone()) } else if n == 0 && !self.loading {
            Some(if !self.state.filter.is_empty() { if en { "No matching items." } else { "没有匹配的项目。" } } else if en { "This folder is empty." } else { "此文件夹为空。" }.to_string())
        } else { None };
        let body = div()
            .id("list-body")
            .relative()
            .flex_1()
            .min_h_0()
            .pt(px(2.))
            .child(canvas(move |b, _, _| list_bounds.set(b), |_, _, _, _| {}).absolute().size_full())
            .child(list)
            .when_some(empty_msg, |d, msg| d.child(div().absolute().top(px(24.)).left_0().right_0().px(px(12.)).flex().justify_center().text_color(Theme::text_caption()).child(msg)))
            .drag_over::<DraggedFiles>(|s, _, _, _| s.bg(Theme::drop_target()))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(Theme::drop_target()))
            .on_drop(cx.listener(|this, d: &DraggedFiles, window, cx| this.drop_files(d, None, window, cx)))
            .on_drop(cx.listener(|this, d: &ExternalPaths, _, cx| this.drop_external(d, None, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                this.menu = None;
                window.focus(&this.focus);
                this.state.click(None, ev.modifiers.control, ev.modifiers.shift);
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.context_menu(None, ev.position, window, cx)));

        let pane_id = cx.entity_id();
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
            .on_action(cx.listener(|this, _: &Recycle, _, cx| { let t = this.state.targets(); this.run_op(Op::Recycle, t, None, vec![], cx) }))
            .on_action(cx.listener(|this, _: &DeletePermanently, _, cx| { let t = this.state.targets(); this.run_op(Op::Delete, t, None, vec![], cx) }))
            .on_action(cx.listener(|this, _: &CopyFiles, _, cx| this.copy_files(false, cx)))
            .on_action(cx.listener(|this, _: &CutFiles, _, cx| this.copy_files(true, cx)))
            .on_action(cx.listener(|this, _: &PasteFiles, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.load(None, cx)))
            .on_action(cx.listener(|this, _: &NewFolder, window, cx| this.new_folder(window, cx)))
            .on_action(cx.listener(|this, _: &EditAddress, window, cx| this.edit_address(window, cx)))
            .on_action(cx.listener(|this, _: &FilterNames, window, cx| this.start_filter(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| this.toggle_hidden(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| { this.state.select_all(); cx.notify() }))
            .on_action(cx.listener(|this, _: &CursorUp, _, cx| this.move_cursor(-1, false, false, false, cx)))
            .on_action(cx.listener(|this, _: &CursorDown, _, cx| this.move_cursor(1, false, false, false, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| { let p = this.page(); this.move_cursor(-p, false, false, false, cx) }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| { let p = this.page(); this.move_cursor(p, false, false, false, cx) }))
            .on_action(cx.listener(|this, _: &First, _, cx| this.move_cursor(-1, true, false, false, cx)))
            .on_action(cx.listener(|this, _: &Last, _, cx| this.move_cursor(1, true, false, false, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.move_cursor(-1, false, true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.move_cursor(1, false, true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| { let p = this.page(); this.move_cursor(-p, false, true, false, cx) }))
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| { let p = this.page(); this.move_cursor(p, false, true, false, cx) }))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.move_cursor(-1, true, true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.move_cursor(1, true, true, false, cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_cursor(-1, false, false, true, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_cursor(1, false, false, true, cx)))
            .on_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.menu.take().is_some() { return cx.notify() }
                if this.filter.is_some() { return this.end_filter(window, cx) }
                this.state.clear_selection();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyPath, _, cx| this.copy_paths(cx)))
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
            .children(self.render_filter(cx))
            .child(self.render_columns(widths, cx))
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
