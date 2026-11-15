//! A pane in its own small window. The window has no bar of its own: the pane
//! header carries a pin and a restore icon in place of the detach button,
//! because a floating window is one pane and needs no second row of chrome.

use crate::pane::{FloatCtl, PaneView};
use crate::theme::Theme;
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::*;
use grids_core::session::PaneSave;
use std::cell::Cell;
use std::rc::Rc;

pub struct FloatPane {
    pane: Entity<PaneView>,
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
        Self { pane }
    }
}

impl Render for FloatPane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().bg(Theme::window()).child(self.pane.clone())
    }
}
