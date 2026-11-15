//! A pane in its own small window. The window draws its own caption in the
//! app's colours (the system title bar is hidden), and the pane header carries
//! a pin and a restore icon in place of the detach button.

use crate::pane::{FloatCtl, PaneView};
use crate::state;
use crate::theme::Theme;
use crate::ui::{HEADER_H, TitleBarExt, TipExt, ui_text, window_controls};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::*;
use grids_core::session::PaneSave;
use std::cell::Cell;
use std::rc::Rc;

pub struct FloatPane {
    pane: Entity<PaneView>,
    /// Keeps the caption's folder name in step with the pane.
    _pane: Subscription,
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
            pane.set_float(FloatCtl { pinned, workspace, index });
            pane
        });
        let sub = cx.observe(&pane, |_, _, cx| cx.notify());
        Self { pane, _pane: sub }
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
}

impl Render for FloatPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().bg(Theme::window())
            .child(self.bar(window, cx))
            .child(div().flex_1().min_h_0().child(self.pane.clone()))
    }
}
