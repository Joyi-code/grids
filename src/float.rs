//! A pane in its own small window: pinned above other windows if asked,
//! translucent through a slider, and one click hands its folder back to the
//! pane it came from.

use crate::pane::PaneView;
use crate::theme::Theme;
use crate::ui;
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::*;
use grids_core::session::PaneSave;
use std::cell::Cell;
use std::rc::Rc;

pub struct FloatPane {
    pane: Entity<PaneView>,
    workspace: Entity<Workspace>,
    /// Pane the window was detached from, so a restore lands in the same place.
    index: usize,
    pinned: bool,
    alpha: u8,
    hwnd: isize,
    dragging: bool,
    track: Rc<Cell<Bounds<Pixels>>>,
}

impl FloatPane {
    pub fn new(
        save: PaneSave,
        index: usize,
        workspace: Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pane = cx.new(|cx| PaneView::new(index, &save, window, cx));
        Self { pane, workspace, index, pinned: false, alpha: 100, hwnd: 0, dragging: false, track: Rc::new(Cell::new(Bounds::default())) }
    }

    fn apply(&self) {
        if self.hwnd == 0 { return }
        grids_shell::window::set_always_on_top(self.hwnd, self.pinned);
        grids_shell::window::set_alpha(self.hwnd, self.alpha);
    }

    fn alpha_at(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let b = self.track.get();
        let w = f32::from(b.size.width);
        if w <= 0. { return }
        let t = (f32::from(x) - f32::from(b.origin.x)) / w;
        let pct = (30. + t.clamp(0., 1.) * 70.).round() as u8;
        if pct != self.alpha {
            self.alpha = pct;
            self.apply();
            cx.notify();
        }
    }

    /// Hands the window's current folder back to the main window: the pane it
    /// came from takes its path, sort, view and columns, then this closes.
    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let save = {
            let p = self.pane.read(cx);
            PaneSave {
                path: p.state.path.clone(),
                sort: p.state.sort,
                ascending: p.state.ascending,
                show_hidden: p.state.show_hidden,
                columns: p.columns.clone(),
                view: p.view,
            }
        };
        let ws = self.workspace.clone();
        let index = self.index;
        ws.update(cx, |ws, cx| ws.adopt_float(index, save, cx));
        window.remove_window();
    }

    fn bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let alpha = self.alpha;
        let t = self.track.clone();
        let fill = ((alpha as f32 - 30.) / 70.).clamp(0., 1.);
        div().flex_none().h(px(28.)).px(px(6.)).flex().items_center().gap(px(8.))
            .bg(Theme::window()).border_b_1().border_color(Theme::line_divider())
            .child(
                div().id("float-pin").h(px(24.)).px(px(9.)).rounded(px(6.)).flex().items_center().cursor_pointer()
                    .when(self.pinned, |d| d.bg(Theme::primary().alpha(0.14)).text_color(Theme::primary()))
                    .when(!self.pinned, |d| d.text_color(Theme::text_caption()).hover(|s| s.bg(Theme::fill_hover())))
                    .text_size(px(12.))
                    .child(if self.pinned { "已置顶" } else { "置顶" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pinned = !this.pinned;
                        this.apply();
                        cx.notify();
                    })),
            )
            .child(
                div().id("float-alpha").relative().h(px(18.)).w(px(96.)).rounded(px(9.)).bg(Theme::fill_hover())
                    .child(div().absolute().left_0().top_0().h_full().rounded(px(8.)).w(px(96. * fill)).bg(Theme::primary().alpha(0.30)))
                    .child(div().absolute().top(px(2.)).size(px(14.)).rounded_full().bg(Theme::primary()).left(px((96. * fill - 14.).max(0.))))
                    .child(canvas(move |b, _, _| t.set(b), |_, _, _, _| {}).absolute().size_full())
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.dragging = true;
                        this.alpha_at(ev.position.x, cx);
                    }))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                        if this.dragging && ev.pressed_button == Some(MouseButton::Left) { this.alpha_at(ev.position.x, cx) }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| { this.dragging = false; cx.notify() })),
            )
            .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(format!("{alpha}%")))
            .child(div().flex_1())
            .child(
                div().id("float-restore").h(px(24.)).px(px(10.)).rounded(px(6.)).flex().items_center().gap(px(4.))
                    .text_size(px(12.)).text_color(Theme::primary()).cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                    .child("恢复到主界面")
                    .on_click(cx.listener(|this, _, window, cx| this.restore(window, cx))),
            )
            .child(
                div().id("float-close").size(px(24.)).rounded(px(6.)).flex().items_center().justify_center()
                    .text_color(Theme::text_caption()).cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                    .child(ui::icon("x", 14., Theme::icon()))
                    .on_click(cx.listener(|_, _, window, _| window.remove_window())),
            )
    }
}

impl Render for FloatPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hwnd = ui::hwnd(window);
        if hwnd != 0 && hwnd != self.hwnd {
            self.hwnd = hwnd;
            self.apply();
        }
        div().size_full().flex().flex_col().bg(Theme::window())
            .child(self.bar(cx))
            .child(div().flex_1().min_h_0().child(self.pane.clone()))
    }
}


