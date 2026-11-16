//! The folder-note editor: a small card, shown over the grid or over a
//! floating pane. The caller builds the buttons (remove / cancel / save).

use crate::input::TextInput;
use crate::theme::{self, Theme};
use crate::ui::{icon, ui_text};
use gpui::prelude::FluentBuilder;
use gpui::*;

/// What the card shows around the input.
pub struct Note {
    /// The folder's name, under the title.
    pub name: SharedString,
    /// "Add note" or "Edit note".
    pub title: SharedString,
    pub input: Entity<TextInput>,
    /// Where the note is kept; the same in both windows.
    pub hint: SharedString,
    /// Characters typed and the limit.
    pub count: usize,
    pub max: usize,
}

pub fn card(note: &Note, buttons: Div) -> Stateful<Div> {
    div().id("note-card")
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
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(note.title.clone())))
        .child(div().mt(px(6.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(Theme::text_caption()).child(ui_text(note.name.clone())))
        .child(div().mt(px(12.)).h(px(32.)).px(px(8.)).rounded(px(7.)).flex().items_center().bg(Theme::bg_body()).border_1().border_color(Theme::field_edge())
            .child(div().flex_1().min_w_0().child(note.input.clone())))
        .child(div().mt(px(6.)).flex().items_center().gap(px(8.)).text_size(px(11.)).text_color(Theme::text_placeholder())
            .child(div().flex_1().min_w_0().child(note.hint.clone()))
            .child(div().flex_none().map(|d| if note.count > note.max { d.text_color(Theme::danger()) } else { d }).child(format!("{} / {}", note.count, note.max))))
        .child(div().mt(px(16.)).flex().items_center().gap(px(8.)).child(buttons))
}
