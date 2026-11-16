//! The confirmation card used before destructive actions (delete, remove a
//! session): the grid shows it over the whole window, a floating pane over its
//! own pane. Both share this look.

use crate::theme::{self, Theme};
use crate::ui::ui_text;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::path::PathBuf;

/// What a confirmation says.
pub struct Card {
    pub title: SharedString,
    pub body: SharedString,
    /// Label of the action button.
    pub ok: SharedString,
    /// Names listed under the body (at most five) and the rest's count line.
    pub names: Vec<SharedString>,
    pub more: Option<SharedString>,
    /// The action button is red (deletes); otherwise the accent colour.
    pub danger: bool,
}

/// How a dialog button looks.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// Cancel: a neutral fill.
    Plain,
    /// The safe action: the accent colour.
    Accent,
    /// A delete: red.
    Danger,
}

/// One button of the card; the focused one carries the ring.
pub fn button(id: &'static str, label: SharedString, kind: Kind, focused: bool) -> Stateful<Div> {
    div().id(id).h(px(30.)).min_w(px(76.)).px(px(14.)).rounded(px(7.)).flex().items_center().justify_center().cursor_pointer()
        .border_2().border_color(if focused { Theme::focus_ring() } else { gpui::transparent_black() })
        .map(|d| match kind {
            Kind::Danger => d.bg(Theme::danger()).text_color(gpui::white()).font_weight(FontWeight::MEDIUM).hover(|s| s.bg(Theme::danger_hover())),
            Kind::Accent => d.bg(Theme::primary()).text_color(gpui::white()).font_weight(FontWeight::MEDIUM).hover(|s| s.bg(Theme::primary()).opacity(0.9)),
            Kind::Plain => d.bg(Theme::fill_hover_strong()).text_color(Theme::text()).hover(|s| s.bg(Theme::fill_pressed())),
        })
        .child(label)
}

/// The card itself: title, body, the names it acts on, and a row of buttons.
/// The caller adds `.on_action(...)` for Esc / Enter / Tab and wraps it in a
/// scrim.
pub fn card(text: &Card, buttons: Div, dialog_focus: &FocusHandle) -> Stateful<Div> {
    div().id("confirm")
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
        .track_focus(dialog_focus)
        .key_context("Dialog")
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text.title.clone()))
        // plain text: it wraps (UiText is one line)
        .child(div().mt(px(8.)).line_height(px(20.)).text_color(Theme::text_caption()).child(text.body.clone()))
        .when(!text.names.is_empty(), |d| {
            d.child(
                div().mt(px(8.)).px(px(10.)).py(px(6.)).rounded(px(6.)).bg(Theme::fill_hover()).flex().flex_col()
                    .text_size(px(12.)).line_height(px(19.)).text_color(Theme::text())
                    .children(text.names.iter().map(|n| div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(n.clone()))))
                    .children(text.more.clone().map(|m| div().text_color(Theme::text_caption()).child(m))),
            )
        })
        .child(div().mt(px(20.)).flex().justify_end().gap(px(8.)).child(buttons))
}

/// The dimmed backdrop: a click outside the card cancels it.
pub fn scrim(inner: impl IntoElement) -> Stateful<Div> {
    div().id("confirm-scrim").absolute().inset_0().flex().items_center().justify_center().bg(gpui::black().opacity(0.18)).occlude()
        .child(inner)
}

/// Fade a scrim in.
pub fn fade(inner: impl Styled + IntoElement + 'static) -> AnyElement {
    deferred(
        inner.with_animation("confirm-fade", Animation::new(std::time::Duration::from_millis(110)), |d, t| d.opacity(t)),
    )
    .with_priority(5)
    .into_any_element()
}

/// The card for deleting `paths`: recycling says so, Shift+Delete warns, and
/// the names being deleted are listed (five, then a count).
pub fn delete_card(en: bool, paths: &[PathBuf], permanent: bool) -> Card {
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
    Card {
        title: if permanent { if en { "Delete permanently" } else { "永久删除" } } else { if en { "Move to Recycle Bin" } else { "移入回收站" } }.into(),
        body: body.into(),
        ok: if permanent { if en { "Delete permanently" } else { "永久删除" } } else { if en { "Delete" } else { "删除" } }.into(),
        names,
        more,
        danger: true,
    }
}


