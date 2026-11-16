//! A pane in its own small window. The window draws its own caption in the
//! app's colours (the system title bar is hidden), and the pane header carries
//! a pin and a restore icon in place of the detach button. The window also
//! answers the pane's events the way the grid does: a delete asks first, and
//! messages show in a small line at the bottom.

use crate::confirm::{self, Card};
use crate::pane::{FloatCtl, PaneEvent, PaneView};
use crate::state::{self, tr};
use crate::theme::Theme;
use crate::ui::{HEADER_H, TitleBarExt, TipExt, ui_text, window_controls};
use crate::workspace::{DialogAccept, DialogCancel, DialogNext, Workspace};
use gpui::prelude::*;
use gpui::*;
use grids_core::session::PaneSave;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

/// A delete waiting for its answer.
struct Pending {
    card: Card,
    paths: Vec<PathBuf>,
    permanent: bool,
    /// The focused button: 0 Cancel (the default), 1 the action.
    focus: u8,
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
            // The grid's features have no meaning in one pane: link groups,
            // Today, the next pane, maximising, the note editor.
            _ => {}
        }
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
        if paths.is_empty() {
            return;
        }
        let card = confirm::delete_card(state::en(cx), &paths, permanent);
        self.pending = Some(Pending { card, paths, permanent, focus: 0 });
        self.dialog_focus.focus(window);
        cx.notify();
    }

    fn close_confirm(&mut self, accept: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if accept {
            let pane = self.pane.clone();
            pane.update(cx, |p, cx| p.delete_confirmed(pending.paths, pending.permanent, cx));
        }
        let focus = self.pane.read(cx).focus.clone();
        window.focus(&focus);
        cx.notify();
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
            .child(self.bar(window, cx))
            .child(div().flex_1().min_h_0().child(self.pane.clone()))
            .children(self.toast_line())
            .children(self.confirm_card(cx))
    }
}
