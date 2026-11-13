//! 历史: the operation journal as a panel (newest first) and Ctrl+Z.
//! Any record can be undone, in any order; `plan_undo` checks the files
//! first and says why when it can't. Undoing runs on a background thread
//! and is itself journalled (`mark_undone`). Only what Grids did is here:
//! changes made in Explorer or other programs are not recorded.

use super::{Confirm, DialogCancel, Workspace};
use crate::ops;
use crate::state::{self, tr};
use crate::theme::{self, Theme};
use crate::ui::{icon, ui_text};
use gpui::{prelude::FluentBuilder, *};
use grids_core::journal::{item_with_stat, plan_undo, OpKind, RealFs, Record, RecordItem, UndoStep};
use grids_shell::OpItem;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

actions!(history, [HistoryClose]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", HistoryClose, Some("History"))]);
}

/// At most this many records are listed.
const MAX_ROWS: usize = 500;

#[derive(Clone)]
struct Row {
    id: u64,
    time_ms: i64,
    kind: OpKind,
    summary: SharedString,
    undone: bool,
}

pub struct HistoryPanel {
    focus: FocusHandle,
    /// None while the journal loads.
    rows: Option<Rc<Vec<Row>>>,
    /// The record being undone.
    busy: Option<u64>,
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// "复制「a.txt」到 文档" style one-liners.
fn summary(r: &Record, en: bool) -> String {
    let n = r.items.len();
    let first = r.items.first();
    let what = match (n, first) {
        (1, Some(it)) => if en { format!("“{}”", name_of(&it.src)) } else { format!("「{}」", name_of(&it.src)) },
        _ => if en { format!("{n} items") } else { format!(" {n} 项") },
    };
    let dest = first.and_then(|it| it.dst.as_deref()).and_then(Path::parent).map(name_of).unwrap_or_default();
    let new_name = first.and_then(|it| it.dst.as_deref()).map(name_of).unwrap_or_default();
    match (r.kind, en) {
        (OpKind::Copy, false) => format!("复制{what}到「{dest}」"),
        (OpKind::Copy, true) => format!("Copied {what} to “{dest}”"),
        (OpKind::Move, false) => format!("移动{what}到「{dest}」"),
        (OpKind::Move, true) => format!("Moved {what} to “{dest}”"),
        (OpKind::Rename, false) => format!("将{what}重命名为「{new_name}」"),
        (OpKind::Rename, true) => format!("Renamed {what} to “{new_name}”"),
        (OpKind::Recycle, false) => format!("将{what}移入回收站"),
        (OpKind::Recycle, true) => format!("Moved {what} to the Recycle Bin"),
        (OpKind::Delete, false) => format!("永久删除{what}"),
        (OpKind::Delete, true) => format!("Permanently deleted {what}"),
        (OpKind::New, false) => format!("新建{what}"),
        (OpKind::New, true) => format!("Created {what}"),
        (OpKind::PasteFile, false) => format!("粘贴为文件{what}"),
        (OpKind::PasteFile, true) => format!("Pasted {what} as a file"),
        (OpKind::Undo, false) => "撤销".into(),
        (OpKind::Undo, true) => "Undo".into(),
    }
}

fn kind_icon(k: OpKind) -> &'static str {
    match k {
        OpKind::Copy => "copy",
        OpKind::Move => "folder-input",
        OpKind::Rename => "text-cursor-input",
        OpKind::Recycle | OpKind::Delete => "trash-2",
        OpKind::New => "plus",
        OpKind::PasteFile => "clipboard-paste",
        OpKind::Undo => "undo-2",
    }
}

/// What one undo did, for the toast and the refresh.
struct Outcome {
    /// Moved back / renamed back.
    items: Vec<OpItem>,
    recycled: Vec<OpItem>,
    restored: Vec<PathBuf>,
    failed: Vec<String>,
    dirs: Vec<PathBuf>,
}

/// Runs `steps` (background thread). A move back into an occupied place
/// takes a free name (the user confirmed that). Nothing here is journalled
/// on its own: the caller writes one Undo record.
fn run_steps(steps: &[UndoStep]) -> Outcome {
    let mut out = Outcome { items: vec![], recycled: vec![], restored: vec![], failed: vec![], dirs: vec![] };
    let dir = |out: &mut Outcome, p: &Path| if let Some(d) = p.parent() && !out.dirs.iter().any(|x| x == d) { out.dirs.push(d.to_path_buf()) };
    for s in steps {
        let res: std::io::Result<()> = match s {
            UndoStep::Recycle { path, .. } => {
                dir(&mut out, path);
                grids_shell::recycle_tracked(std::slice::from_ref(path), true).map(|done| out.recycled.extend(done))
            }
            UndoStep::MoveBack { from, to, occupied } => {
                dir(&mut out, from);
                dir(&mut out, to);
                match to.parent() {
                    None => Err(std::io::Error::other("no parent")),
                    Some(parent) => grids_shell::move_tracked(std::slice::from_ref(from), parent).map(|mut done| {
                        // a free name was picked although the place is free again: take the old name
                        if !occupied && let Some(it) = done.first_mut() && let Some(d) = it.dst.clone() && d != *to && std::fs::symlink_metadata(to).is_err()
                            && let Ok(r) = grids_shell::rename(&d, &name_of(to)) { it.dst = Some(r) }
                        out.items.extend(done);
                    }),
                }
            }
            UndoStep::Rename { from, to } => {
                dir(&mut out, from);
                grids_shell::rename_tracked(from, &name_of(to)).map(|done| out.items.extend(done))
            }
            UndoStep::Restore { original, deleted_ms } => {
                dir(&mut out, original);
                let when = std::time::UNIX_EPOCH + Duration::from_millis((*deleted_ms).max(0) as u64);
                grids_shell::restore_from_recycle_bin(original, when).map(|p| out.restored.push(p))
            }
            UndoStep::Skip { path, why } => { out.failed.push(format!("{}：{why}", name_of(path))); Ok(()) }
        };
        if let Err(e) = res && e.kind() != std::io::ErrorKind::Interrupted {
            let p = match s { UndoStep::Recycle { path, .. } => path, UndoStep::MoveBack { from, .. } | UndoStep::Rename { from, .. } => from, UndoStep::Restore { original, .. } => original, UndoStep::Skip { path, .. } => path };
            out.failed.push(format!("{}：{e}", name_of(p)));
        }
    }
    out
}

impl Workspace {
    pub(super) fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.is_some() { return self.close_history(window, cx) }
        let focus = cx.focus_handle();
        focus.focus(window);
        self.history = Some(HistoryPanel { focus, rows: None, busy: None });
        self.reload_history(cx);
        cx.notify();
    }

    fn close_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.history = None;
        self.refocus(window, cx);
        cx.notify();
    }

    /// Rereads the journal (the first read hits the disk: background).
    fn reload_history(&mut self, cx: &mut Context<Self>) {
        if self.history.is_none() { return }
        let en = state::en(cx);
        cx.spawn(async move |this, cx| {
            let rows = cx.background_executor().spawn(async move {
                let Ok(j) = ops::journal().lock() else { return Vec::new() };
                j.history().take(MAX_ROWS).map(|r| Row { id: r.id, time_ms: r.time_ms, kind: r.kind, summary: summary(r, en).into(), undone: r.undone }).collect::<Vec<_>>()
            }).await;
            let _ = this.update(cx, |this, cx| if let Some(h) = &mut this.history { h.rows = Some(Rc::new(rows)); cx.notify() });
        }).detach();
    }

    /// Ctrl+Z: the newest record not undone yet. Text fields keep their own
    /// Ctrl+Z (bound in the TextInput context), dialogs block it.
    pub(super) fn undo_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirm.is_some() || self.lockers.is_some() { return }
        cx.spawn_in(window, async move |this, cx| {
            let id = cx.background_executor().spawn(async move { ops::journal().lock().ok().and_then(|j| j.last_undoable().map(|r| r.id)) }).await;
            let _ = this.update_in(cx, |this, window, cx| match id {
                Some(id) => this.undo_record(id, window, cx),
                None => this.show_toast(tr(cx, "没有可撤销的操作", "Nothing to undo").into(), cx),
            });
        }).detach();
    }

    /// Plans the undo of `id` off the UI thread, confirms the risky cases,
    /// then runs it.
    pub(super) fn undo_record(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.as_ref().is_some_and(|h| h.busy.is_some()) { return }
        let en = state::en(cx);
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx.background_executor().spawn(async move {
                let rec = ops::journal().lock().ok().and_then(|j| j.get(id).cloned());
                let Some(rec) = rec else { return Err(if en { "The record is gone".to_string() } else { "记录已不存在".to_string() }) };
                plan_undo(&rec, &RealFs).map(|steps| (rec, steps)).map_err(|why| why.to_string())
            }).await;
            let _ = this.update_in(cx, |this, window, cx| match plan {
                Err(why) => this.show_toast(if en { format!("Can't undo: {why}") } else { format!("无法撤销：{why}") }, cx),
                Ok((rec, steps)) => this.confirm_undo(rec, steps, window, cx),
            });
        }).detach();
    }

    fn confirm_undo(&mut self, rec: Record, steps: Vec<UndoStep>, window: &mut Window, cx: &mut Context<Self>) {
        let en = state::en(cx);
        let changed: Vec<PathBuf> = steps.iter().filter_map(|s| match s { UndoStep::Recycle { path, changed: true } => Some(path.clone()), _ => None }).collect();
        let occupied: Vec<PathBuf> = steps.iter().filter_map(|s| match s { UndoStep::MoveBack { to, occupied: true, .. } => Some(to.clone()), _ => None }).collect();
        if changed.is_empty() && occupied.is_empty() { return self.run_undo(rec, steps, cx) }
        let one = |v: &[PathBuf]| v.first().map(|p| name_of(p)).unwrap_or_default();
        let (title, body, ok) = if !occupied.is_empty() {
            let f = one(&occupied);
            (tr(cx, "原位置已有同名项", "Original place is taken"),
             if en { format!("Something named “{f}” is back in the original folder. Put the item back under a new name?") } else { format!("原文件夹里已经有「{f}」。改用新名称放回？") },
             tr(cx, "改名放回", "Put back renamed"))
        } else {
            let f = one(&changed);
            (tr(cx, "文件已被修改", "File was changed"),
             if en { format!("“{f}” was changed after it was created. Undoing moves it to the Recycle Bin with the changes.") } else { format!("「{f}」创建后又被修改过，撤销会把它连同修改一起移入回收站。") },
             tr(cx, "仍然撤销", "Undo anyway"))
        };
        let names: Vec<SharedString> = if changed.len() + occupied.len() > 1 { occupied.iter().chain(changed.iter()).take(5).map(|p| name_of(p).into()).collect() } else { vec![] };
        let steps = Rc::new(steps);
        let rec = Rc::new(rec);
        self.confirm = Some(Confirm {
            title: title.into(),
            body: body.into(),
            ok: ok.into(),
            names,
            more: None,
            danger: !changed.is_empty(),
            action: Rc::new(move |this, _, cx| this.run_undo((*rec).clone(), (*steps).clone(), cx)),
            focus: 0,
        });
        self.dialog_focus.focus(window);
        cx.notify();
    }

    fn run_undo(&mut self, rec: Record, steps: Vec<UndoStep>, cx: &mut Context<Self>) {
        let en = state::en(cx);
        if let Some(h) = &mut self.history { h.busy = Some(rec.id) }
        cx.notify();
        let label = summary(&rec, en);
        cx.spawn(async move |this, cx| {
            let (id, pane) = (rec.id, rec.pane);
            let out = cx.background_executor().spawn(async move {
                let out = run_steps(&steps);
                let mut items: Vec<RecordItem> = out.items.iter().chain(&out.recycled).map(|i| item_with_stat(i.src.clone(), i.dst.clone(), i.is_dir, &RealFs)).collect();
                items.extend(out.restored.iter().map(|p| item_with_stat(p.clone(), Some(p.clone()), p.is_dir(), &RealFs)));
                if !items.is_empty() && let Ok(mut j) = ops::journal().lock() { let _ = j.mark_undone(id, pane, items); }
                out
            }).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(h) = &mut this.history { h.busy = None }
                let a = ops::follow_moves(&out.items, cx);
                if ops::after_op(ops::Op::Recycle, &out.recycled, cx) || a { this.schedule_save(cx) }
                this.fs_changed(&out.dirs, cx);
                let msg = match out.failed.first() {
                    None => if en { format!("Undone: {label}") } else { format!("已撤销：{label}") },
                    Some(f) if out.failed.len() == 1 => if en { format!("Partly undone. {f}") } else { format!("部分未撤销。{f}") },
                    Some(f) => if en { format!("Partly undone ({} failed). {f}", out.failed.len()) } else { format!("部分未撤销（{} 项失败）。{f}", out.failed.len()) },
                };
                this.show_toast(msg, cx);
                this.reload_history(cx);
            });
        }).detach();
    }

    pub(super) fn render_history(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let h = self.history.as_ref()?;
        let en = state::en(cx);
        let offset = state::shared(cx).utc_offset;
        let view = cx.entity();
        let busy = h.busy;
        let list = match &h.rows {
            None => div().h(px(120.)).flex().items_center().justify_center().text_color(Theme::text_caption()).child(tr(cx, "正在读取…", "Loading…")).into_any_element(),
            Some(rows) if rows.is_empty() => div().h(px(160.)).flex().flex_col().gap(px(8.)).items_center().justify_center().text_color(Theme::text_caption())
                .child(icon("history", 28., Theme::icon_muted()))
                .child(tr(cx, "还没有操作记录", "No operations yet")).into_any_element(),
            Some(rows) => {
                let rows = rows.clone();
                let n = rows.len();
                let now_day = { let s = secs_now(); let (y, m, d, _, _) = grids_core::format::civil(s, offset); (y, m, d) };
                uniform_list("history-rows", n, move |range, _, _| {
                    range.filter_map(|i| {
                        let r = rows.get(i)?;
                        let s = r.time_ms.div_euclid(1000);
                        let (y, m, d, hh, mi) = grids_core::format::civil(s, offset);
                        let when = if (y, m, d) == now_day { format!("{hh:02}:{mi:02}") } else if y == now_day.0 { format!("{m}/{d} {hh:02}:{mi:02}") } else { format!("{y}/{m}/{d}") };
                        let dim = r.undone || !r.kind.undoable();
                        let id = r.id;
                        let right = if busy == Some(id) {
                            div().text_color(Theme::text_caption()).child(if en { "Undoing…" } else { "撤销中…" }).into_any_element()
                        } else if r.undone {
                            div().text_color(Theme::text_caption()).child(if en { "Undone" } else { "已撤销" }).into_any_element()
                        } else if !r.kind.undoable() {
                            div().text_color(Theme::text_caption()).child(if en { "Can't undo" } else { "无法撤销" }).into_any_element()
                        } else {
                            div().id(("h-undo", id as usize)).h(px(24.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(4.)).cursor_pointer()
                                .bg(Theme::fill_hover()).hover(|s| s.bg(Theme::fill_hover_strong())).text_color(Theme::text())
                                .child(icon("undo-2", 13., Theme::icon()))
                                .child(if en { "Undo" } else { "撤销" })
                                .on_click(super::today::window_listener(&view, move |this, window, cx| this.undo_record(id, window, cx)))
                                .into_any_element()
                        };
                        Some(div().id(("h-row", i)).w_full().h(px(36.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(10.)).hover(|s| s.bg(Theme::fill_hover()))
                            .child(icon(kind_icon(r.kind), 15., if dim { Theme::icon_muted() } else { Theme::icon() }))
                            .child(div().flex_none().w(px(70.)).text_size(px(12.)).text_color(Theme::text_caption()).child(when))
                            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()
                                .text_color(if dim { Theme::text_caption() } else { Theme::text() })
                                .when(r.undone, |d| d.line_through())
                                .child(ui_text(r.summary.clone())))
                            .child(div().flex_none().text_size(px(12.)).child(right))
                            .into_any_element())
                    }).collect()
                }).w_full().h(px((n.min(10) as f32) * 36.)).into_any_element()
            }
        };
        let card = div().id("history").w(px(560.)).p(px(20.)).pb(px(16.)).flex().flex_col()
            .bg(theme::raised(Theme::popover_bg())).border_1().border_color(Theme::line_border()).rounded(px(8.)).shadow(theme::shadow_menu())
            .font_family(theme::UI_FONT).text_size(px(13.)).text_color(Theme::text())
            .occlude()
            .track_focus(&h.focus)
            .key_context("History")
            .on_action(cx.listener(|this, _: &HistoryClose, window, cx| this.close_history(window, cx)))
            .on_action(cx.listener(|this, _: &DialogCancel, window, cx| this.close_history(window, cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().flex().items_center().gap(px(8.))
                .child(icon("history", 17., Theme::icon()))
                .child(div().flex_1().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(tr(cx, "操作历史", "History")))
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(tr(cx, "Ctrl+Z 撤销最近一步", "Ctrl+Z undoes the latest")))
                .child(div().id("history-x").size(px(26.)).rounded(px(6.)).flex().items_center().justify_center().cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                    .child(icon("x", 15., Theme::icon()))
                    .on_click(cx.listener(|this, _, window, cx| this.close_history(window, cx)))))
            .child(div().mt(px(6.)).mb(px(10.)).text_size(px(12.)).line_height(px(18.)).text_color(Theme::text_caption())
                .child(tr(cx, "只记录在 Grids 里做的操作。资源管理器或其他程序里的改动不在这里，也无法撤销。",
                    "Only operations done in Grids are listed; changes made in Explorer or other programs are not recorded and can't be undone here.")))
            .child(list);
        Some(
            deferred(
                div().id("history-scrim").absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(0.18)).occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_history(window, cx)))
                    .child(card)
                    .with_animation("history-fade", Animation::new(Duration::from_millis(110)), |d, t| d.opacity(t)),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

fn secs_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
