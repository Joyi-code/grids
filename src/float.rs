//! A pane in its own small window. The window draws its own caption in the
//! app's colours (the system title bar is hidden), and the pane header carries
//! a pin and a restore icon in place of the detach button. The window also
//! answers the pane's events the way the grid does: a delete asks first, and
//! messages show in a small line at the bottom.

use crate::confirm::{self, Card};
use crate::input::{InputEvent, TextInput};
use crate::pane::{FloatCtl, PaneEvent, PaneView};
use crate::state::{self, tr};
use crate::theme::Theme;
use crate::ui::{HEADER_H, TitleBarExt, TipExt, ui_text, window_controls};
use crate::workspace::{DialogAccept, DialogCancel, DialogNext, UndoLast, Workspace, history};
use gpui::prelude::*;
use gpui::*;
use grids_core::journal::{Record, UndoStep};
use grids_core::session::PaneSave;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

/// Ctrl+Z in a floating window: the grid's shortcut only lives there.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-z", UndoLast, Some("Float"))]);
}

/// A delete or an undo waiting for its answer.
struct Pending {
    card: Card,
    action: PendingAction,
    /// The focused button: 0 Cancel (the default), 1 the action.
    focus: u8,
}

enum PendingAction {
    Delete { paths: Vec<PathBuf>, permanent: bool },
    Undo { rec: Record, steps: Vec<UndoStep> },
}

/// The folder-note editor: an input, like the grid's card.
struct NoteEdit {
    folder: PathBuf,
    input: Entity<TextInput>,
    had: bool,
    _subs: Vec<Subscription>,
}

pub struct FloatPane {
    pane: Entity<PaneView>,
    /// The grid this window was detached from; it keeps the session saved and
    /// its other panes in step with what happens here.
    workspace: Entity<Workspace>,
    /// Keeps the caption's folder name in step with the pane.
    _pane: Subscription,
    /// What the pane asks of its window (deletes, messages, refreshes).
    _events: Subscription,
    dialog_focus: FocusHandle,
    pending: Option<Pending>,
    note: Option<NoteEdit>,
    toast: Option<(SharedString, u64)>,
    toast_seq: u64,
}

impl FloatPane {
    pub fn new(
        save: PaneSave,
        index: usize,
        workspace: Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pinned = Rc::new(Cell::new(false));
        let pane = cx.new(|cx| {
            let mut pane = PaneView::new(index, &save, window, cx);
            // The only pane of its window: selected rows take the active look.
            pane.active = true;
            pane.set_float(FloatCtl { pinned, workspace: workspace.clone(), index });
            pane
        });
        let sub = cx.observe(&pane, |_, _, cx| cx.notify());
        let events = cx.subscribe_in(&pane, window, |this, _, ev: &PaneEvent, window, cx| this.on_pane_event(ev, window, cx));
        Self {
            pane,
            workspace,
            _pane: sub,
            _events: events,
            dialog_focus: cx.focus_handle(),
            pending: None,
            note: None,
            toast: None,
            toast_seq: 0,
        }
    }

    /// The caption's row: the folder's name, a drag area, and the window's
    /// minimise / maximise / close buttons, as on the main window.
    fn bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let en = state::en(cx);
        let path = self.pane.read(cx).state.path.clone();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        div().id("float-bar").h(px(HEADER_H)).flex_none().flex().items_center().pl(px(14.)).bg(Theme::window())
            .title_bar()
            .child(
                div().id("float-title").flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()
                    .text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text())
                    .child(ui_text(name))
                    .tip(&path.display().to_string(), None),
            )
            .children(window_controls(window, en))
    }

    /// What the pane would have said to the grid, handled here.
    fn on_pane_event(&mut self, ev: &PaneEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            PaneEvent::AskDelete { paths, permanent } => self.ask_delete(paths.clone(), *permanent, window, cx),
            PaneEvent::Toast(msg) => self.say(msg.clone(), cx),
            PaneEvent::Locked(_) => {
                let msg = if state::en(cx) {
                    format!("This folder is read-only (inside {} only)", state::brand(cx))
                } else {
                    format!("此文件夹已设为只读（仅在{}内生效）", state::brand(cx))
                };
                self.say(msg, cx);
            }
            // Sort, columns, favourites: the grid saves the session.
            PaneEvent::Changed => self.workspace.update(cx, |ws, cx| ws.note_changed(cx)),
            // An operation just changed these folders: reload this pane when it
            // shows one, and let the grid refresh its own panes.
            PaneEvent::FsChanged(dirs) => {
                let pane = self.pane.clone();
                pane.update(cx, |p, cx| p.refresh_if_showing(dirs, cx));
                self.workspace.update(cx, |ws, cx| ws.files_changed(dirs, cx));
            }
            // Every move asks who holds the files first. The grid shows its
            // dialog; here the check runs quietly and the move goes ahead, the
            // shell reporting a file that is really stuck.
            PaneEvent::Lockers { paths, then } => self.lockers(paths.clone(), then.clone(), window, cx),
            // The folder note editor is a card here too.
            PaneEvent::EditNote(folder) => self.edit_note(folder.clone(), window, cx),
            // The rest belongs to the grid: link groups, Today, the next pane,
            // maximising.
            _ => {}
        }
    }

    /// `then` Some: a move waits for this check. None: the menu's 查看占用,
    /// answered here with a line instead of the grid's list dialog.
    fn lockers(&mut self, paths: Vec<PathBuf>, then: Option<crate::workspace::LockersThen>, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        cx.spawn_in(window, async move |this, cx| {
            let probe = paths.clone();
            let found = cx.background_executor().spawn(async move {
                grids_shell::lockers(&probe).map(|l| l.list.into_iter().map(|w| w.name).collect::<Vec<_>>()).unwrap_or_default()
            }).await;
            let _ = this.update_in(cx, |this, window, cx| match then {
                Some(then) => {
                    if !found.is_empty() {
                        let msg = if en { format!("In use by {}: trying anyway", found.join(", ")) } else { format!("被占用（{}），仍然继续", found.join("、")) };
                        this.say(msg, cx);
                    }
                    // `then` updates the pane; run it after this update ends.
                    window.defer(cx, move |window, cx| then(window, cx));
                }
                None => {
                    let msg = if found.is_empty() {
                        if en { "Nothing holds these files".to_string() } else { "没有程序占用这些文件".to_string() }
                    } else if en {
                        format!("In use by {}", found.join(", "))
                    } else {
                        format!("被占用：{}", found.join("、"))
                    };
                    this.say(msg, cx);
                }
            });
        }).detach();
    }

    fn say(&mut self, msg: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some((msg.into(), seq));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(5)).await;
            let _ = this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|t| t.1 == seq) {
                    this.toast = None;
                    cx.notify();
                }
            });
        }).detach();
    }

    fn ask_delete(&mut self, paths: Vec<PathBuf>, permanent: bool, window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() || self.note.is_some() {
            return;
        }
        let card = confirm::delete_card(state::en(cx), &paths, permanent);
        self.pending = Some(Pending { card, action: PendingAction::Delete { paths, permanent }, focus: 0 });
        self.dialog_focus.focus(window);
        cx.notify();
    }

    fn close_confirm(&mut self, accept: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if accept {
            match pending.action {
                PendingAction::Delete { paths, permanent } => {
                    let pane = self.pane.clone();
                    pane.update(cx, |p, cx| p.delete_confirmed(paths, permanent, cx));
                }
                PendingAction::Undo { rec, steps } => self.run_undo(rec, steps, cx),
            }
        }
        let focus = self.pane.read(cx).focus.clone();
        window.focus(&focus);
        cx.notify();
    }

    // ---- undo (the grid's Ctrl+Z, without its history panel)

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() || self.note.is_some() {
            return;
        }
        let en = state::en(cx);
        cx.spawn_in(window, async move |this, cx| {
            let next = cx.background_executor().spawn(async move { history::next_undo() }).await;
            let _ = this.update_in(cx, |this, window, cx| match next {
                history::NextUndo::None => this.say(if en { "Nothing to undo" } else { "没有可撤销的操作" }, cx),
                history::NextUndo::Failed(why) => this.say(if en { format!("Can't undo: {why}") } else { format!("无法撤销：{why}") }, cx),
                history::NextUndo::Ready(rec, steps) => this.undo_ready(rec, steps, window, cx),
            });
        }).detach();
    }

    /// Asks first for the two risky undos, as the grid's history panel does.
    fn undo_ready(&mut self, rec: Record, steps: Vec<UndoStep>, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        let changed: Vec<PathBuf> = steps.iter().filter_map(|s| match s { UndoStep::Recycle { path, changed: true } => Some(path.clone()), _ => None }).collect();
        let occupied: Vec<PathBuf> = steps.iter().filter_map(|s| match s { UndoStep::MoveBack { to, occupied: true, .. } => Some(to.clone()), _ => None }).collect();
        if changed.is_empty() && occupied.is_empty() {
            return self.run_undo(rec, steps, cx);
        }
        let name = |p: &PathBuf| p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
        let one = |v: &[PathBuf]| v.first().map(name).unwrap_or_default();
        let (title, body, ok) = if !occupied.is_empty() {
            (
                if en { "Original place is taken".to_string() } else { "原位置已有同名项".to_string() },
                if en {
                    format!("Something named “{}” is back in the original folder. Put the item back under a new name?", one(&occupied))
                } else {
                    format!("原文件夹里已经有「{}」。改用新名称放回？", one(&occupied))
                },
                if en { "Put back renamed".to_string() } else { "改名放回".to_string() },
            )
        } else {
            (
                if en { "File was changed".to_string() } else { "文件已被修改".to_string() },
                if en {
                    format!("“{}” was changed after it was created. Undoing moves it to the Recycle Bin with the changes.", one(&changed))
                } else {
                    format!("「{}」创建后又被修改过，撤销会把它连同修改一起移入回收站。", one(&changed))
                },
                if en { "Undo anyway".to_string() } else { "仍然撤销".to_string() },
            )
        };
        let names: Vec<SharedString> = if changed.len() + occupied.len() > 1 { occupied.iter().chain(changed.iter()).take(5).map(|p| name(p).into()).collect() } else { vec![] };
        let card = Card { title: title.into(), body: body.into(), ok: ok.into(), names, more: None, danger: !changed.is_empty() };
        self.pending = Some(Pending { card, action: PendingAction::Undo { rec, steps }, focus: 0 });
        self.dialog_focus.focus(window);
        cx.notify();
    }

    fn run_undo(&mut self, rec: Record, steps: Vec<UndoStep>, cx: &mut Context<Self>) {
        let en = state::en(cx);
        let label = history::summary(&rec, en);
        cx.spawn(async move |this, cx| {
            let out = cx.background_executor().spawn(async move { history::apply_undo(&rec, &steps) }).await;
            let _ = this.update(cx, |this, cx| {
                let dirs = out.dirs.clone();
                let pane = this.pane.clone();
                pane.update(cx, |p, cx| p.refresh_if_showing(&dirs, cx));
                this.workspace.update(cx, |ws, cx| ws.files_changed(&dirs, cx));
                let msg = match out.failed.first() {
                    None => if en { format!("Undone: {label}") } else { format!("已撤销：{label}") },
                    Some(f) if out.failed.len() == 1 => if en { format!("Partly undone. {f}") } else { format!("部分未撤销。{f}") },
                    Some(f) => if en { format!("Partly undone ({} failed). {f}", out.failed.len()) } else { format!("部分未撤销（{} 项失败）。{f}", out.failed.len()) },
                };
                this.say(msg, cx);
            });
        }).detach();
    }

    // ---- the folder note editor

    fn edit_note(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
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
        self.note = Some(NoteEdit { folder, input, had: old.is_some(), _subs: subs });
        cx.notify();
    }

    /// None cancels; Some(false) saves the text; Some(true) removes the note.
    fn close_note(&mut self, how: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = self.note.take() else {
            return;
        };
        if let Some(remove) = how {
            let text = if remove { String::new() } else { ed.input.read(cx).text().to_string() };
            let notes = &mut cx.global_mut::<state::Shared>().config.notes;
            let before = grids_core::notes::get(notes, &ed.folder).map(str::to_string);
            grids_core::notes::set(notes, &ed.folder, &text);
            let after = grids_core::notes::get(notes, &ed.folder).map(str::to_string);
            if before != after {
                self.workspace.update(cx, |ws, cx| ws.note_saved(cx));
                let pane = self.pane.clone();
                pane.update(cx, |_, cx| cx.notify());
                let msg = match (after.is_some(), state::en(cx)) {
                    (true, false) => "已保存备注",
                    (true, true) => "Note saved",
                    (false, false) => "已删除备注",
                    (false, true) => "Note removed",
                };
                self.say(msg, cx);
            }
        }
        let focus = self.pane.read(cx).focus.clone();
        window.focus(&focus);
        cx.notify();
    }

    fn note_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ed = self.note.as_ref()?;
        let en = state::en(cx);
        let t = move |zh: &'static str, e: &'static str| -> SharedString { (if en { e } else { zh }).into() };
        let n = ed.input.read(cx).text().trim().chars().count();
        let name = ed.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| ed.folder.to_string_lossy().into_owned());
        let buttons = div().flex().items_center().gap(px(8.))
            .when(ed.had, |d| d.child(div().id("note-remove").h(px(30.)).px(px(10.)).rounded(px(7.)).flex().items_center().cursor_pointer()
                .text_color(Theme::danger()).hover(|s| s.bg(Theme::fill_hover()))
                .child(t("删除备注", "Remove note"))
                .on_click(cx.listener(|this, _, window, cx| this.close_note(Some(true), window, cx)))))
            .child(div().flex_1())
            .child(confirm::button("float-note-cancel", t("取消", "Cancel"), confirm::Kind::Plain, false)
                .on_click(cx.listener(|this, _, window, cx| this.close_note(None, window, cx))))
            .child(confirm::button("float-note-save", t("保存", "Save"), confirm::Kind::Accent, false)
                .on_click(cx.listener(|this, _, window, cx| this.close_note(Some(false), window, cx))));
        let note = crate::note_card::Note {
            name: name.into(),
            title: if ed.had { t("编辑备注", "Edit note") } else { t("添加备注", "Add note") },
            input: ed.input.clone(),
            hint: t("仅保存在 Grids 中，在其他程序里改名会丢失备注", "Kept in Grids only; renaming elsewhere loses it"),
            count: n,
            max: grids_core::notes::MAX_NOTE_CHARS,
        };
        let card = crate::note_card::card(&note, buttons);
        let scrim = confirm::scrim(card)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_note(None, window, cx)));
        Some(confirm::fade(scrim))
    }

    fn confirm_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pending = self.pending.as_ref()?;
        let kind = if pending.card.danger { confirm::Kind::Danger } else { confirm::Kind::Accent };
        let buttons = div().flex().items_center().gap(px(8.))
            .child(confirm::button("float-confirm-cancel", tr(cx, "取消", "Cancel").into(), confirm::Kind::Plain, pending.focus == 0)
                .on_click(cx.listener(|this, _, window, cx| this.close_confirm(false, window, cx))))
            .child(confirm::button("float-confirm-ok", pending.card.ok.clone(), kind, pending.focus == 1)
                .on_click(cx.listener(|this, _, window, cx| this.close_confirm(true, window, cx))));
        let card = confirm::card(&pending.card, buttons, &self.dialog_focus)
            .on_action(cx.listener(|this, _: &DialogCancel, window, cx| this.close_confirm(false, window, cx)))
            .on_action(cx.listener(|this, _: &DialogAccept, window, cx| {
                let accept = this.pending.as_ref().is_some_and(|p| p.focus == 1);
                this.close_confirm(accept, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DialogNext, _, cx| {
                if let Some(p) = &mut this.pending { p.focus ^= 1 }
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let scrim = confirm::scrim(card)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_confirm(false, window, cx)));
        Some(confirm::fade(scrim))
    }

    fn toast_line(&self) -> Option<AnyElement> {
        let (msg, _) = self.toast.as_ref()?;
        Some(
            div().absolute().bottom(px(14.)).left_0().right_0().flex().justify_center()
                .child(
                    div().max_w(px(560.)).px(px(12.)).py(px(6.)).rounded(px(7.))
                        .bg(Theme::fill_hover_strong()).text_size(px(12.)).text_color(Theme::text())
                        .child(msg.clone()),
                )
                .into_any_element(),
        )
    }
}

impl Render for FloatPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().relative().size_full().flex().flex_col().bg(Theme::window())
            .key_context("Float")
            .on_action(cx.listener(|this, _: &UndoLast, window, cx| this.undo(window, cx)))
            .child(self.bar(window, cx))
            .child(div().flex_1().min_h_0().child(self.pane.clone()))
            .children(self.toast_line())
            .children(self.note_card(cx))
            .children(self.confirm_card(cx))
    }
}
