//! The right-click menu, drawn with GPUI: a neutral panel (hairline, soft
//! shadow, radius 8), 30pt rows with the glyph on the left and the shortcut
//! on the right, hairline separators, submenus that open on hover or →.
//! Positions are worked out here so the panels flip to stay in the window.

use crate::theme::{self, Theme};
use crate::ui::icon;
use gpui::{prelude::FluentBuilder, *};
use grids_core::SortKey;
use std::path::PathBuf;
use std::sync::Arc;

actions!(menu, [MenuUp, MenuDown, MenuLeft, MenuRight, MenuConfirm, MenuCancel]);

/// The menu's keys live in its own context, which sits inside the pane's,
/// so they win over the pane's arrow / Enter / Esc bindings.
pub fn bind_keys(cx: &mut App) {
    let c = Some("Menu");
    cx.bind_keys([
        KeyBinding::new("up", MenuUp, c),
        KeyBinding::new("down", MenuDown, c),
        KeyBinding::new("left", MenuLeft, c),
        KeyBinding::new("right", MenuRight, c),
        KeyBinding::new("enter", MenuConfirm, c),
        KeyBinding::new("space", MenuConfirm, c),
        KeyBinding::new("escape", MenuCancel, c),
    ]);
}

const ROW_H: f32 = 30.;
const SEP_H: f32 = 9.;
const PAD: f32 = 4.;
const RADIUS: f32 = 8.;
const ROW_R: f32 = 6.;
const MARGIN: f32 = 8.;

/// What a menu row does; the pane carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Open,
    OpenWith(String),
    OpenWithOther,
    OpenInNextPane,
    Cut,
    Copy,
    Paste,
    PasteInto(PathBuf),
    CopyPath,
    Shelve,
    Rename,
    Delete,
    Zip,
    ToggleFavourite(PathBuf),
    Reveal,
    Properties,
    FolderProperties,
    More,
    Sort(SortKey),
    Ascending(bool),
    ToggleHidden,
    Refresh,
    NewFolder,
    NewFile(String),
    Terminal,
    CopyFolderPath,
    View(grids_core::ViewMode),
}

#[derive(Clone)]
pub enum Glyph {
    Icon(&'static str),
    Image(Arc<RenderImage>),
    Check(bool),
}

#[derive(Clone)]
pub enum Item {
    Row { label: SharedString, glyph: Glyph, keys: &'static str, enabled: bool, cmd: Option<Cmd>, sub: Option<Vec<Item>> },
    Sep,
    /// Greyed text (loading, empty).
    Note(SharedString),
}

impl Item {
    pub fn row(label: impl Into<SharedString>, glyph: &'static str, keys: &'static str, cmd: Cmd) -> Item {
        Item::Row { label: label.into(), glyph: Glyph::Icon(glyph), keys, enabled: true, cmd: Some(cmd), sub: None }
    }
    pub fn check(label: impl Into<SharedString>, on: bool, keys: &'static str, cmd: Cmd) -> Item {
        Item::Row { label: label.into(), glyph: Glyph::Check(on), keys, enabled: true, cmd: Some(cmd), sub: None }
    }
    pub fn sub(label: impl Into<SharedString>, glyph: &'static str, items: Vec<Item>) -> Item {
        Item::Row { label: label.into(), glyph: Glyph::Icon(glyph), keys: "", enabled: true, cmd: None, sub: Some(items) }
    }
    pub fn enabled(mut self, on: bool) -> Item {
        if let Item::Row { enabled, .. } = &mut self { *enabled = on }
        self
    }
    fn selectable(&self) -> bool { matches!(self, Item::Row { enabled: true, .. }) }
    fn submenu(&self) -> Option<&Vec<Item>> { if let Item::Row { sub: Some(s), enabled: true, .. } = self { Some(s) } else { None } }
    fn height(&self) -> f32 { match self { Item::Sep => SEP_H, _ => ROW_H } }
}

pub enum MenuEvent {
    Pick(Cmd),
    Dismiss,
}

pub struct ContextMenu {
    items: Vec<Item>,
    /// Where it was asked for, window coordinates.
    pos: Point<Pixels>,
    width: f32,
    /// The highlighted row per open level; a highlighted row with a submenu
    /// opens the next level.
    sel: Vec<usize>,
    pub focus: FocusHandle,
}

impl EventEmitter<MenuEvent> for ContextMenu {}

impl ContextMenu {
    pub fn new(items: Vec<Item>, pos: Point<Pixels>, width: f32, cx: &mut Context<Self>) -> Self {
        Self { items, pos, width, sel: vec![], focus: cx.focus_handle() }
    }

    /// Replaces the submenu of the row labelled `label` (filled in later).
    pub fn set_submenu(&mut self, label: &str, items: Vec<Item>, cx: &mut Context<Self>) {
        fn walk(list: &mut [Item], label: &str, items: &mut Option<Vec<Item>>) {
            for it in list {
                if let Item::Row { label: l, sub: Some(s), .. } = it {
                    if l.as_ref() == label { if let Some(v) = items.take() { *s = v } return }
                    walk(s, label, items);
                }
            }
        }
        walk(&mut self.items, label, &mut Some(items));
        // A highlight past the new end would point nowhere.
        self.sel.truncate(1);
        cx.notify();
    }

    fn level(&self, depth: usize) -> Option<&Vec<Item>> {
        let mut list = &self.items;
        for &i in self.sel.iter().take(depth) { list = list.get(i)?.submenu()? }
        Some(list)
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.sel.is_empty() { self.sel.push(usize::MAX) }
        let depth = self.sel.len() - 1;
        let Some(list) = self.level(depth) else { return };
        let n = list.len() as isize;
        if n == 0 { return }
        let start = self.sel[depth];
        let mut i = if start == usize::MAX { if delta > 0 { -1 } else { n } } else { start as isize };
        for _ in 0..n {
            i = (i + delta).rem_euclid(n);
            if list[i as usize].selectable() { break }
        }
        if list.get(i as usize).is_some_and(Item::selectable) { self.sel[depth] = i as usize }
        cx.notify();
    }

    fn current(&self) -> Option<&Item> {
        let depth = self.sel.len().checked_sub(1)?;
        self.level(depth)?.get(*self.sel.get(depth)?)
    }

    fn enter_sub(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(sub) = self.current().and_then(Item::submenu) else { return false };
        if let Some(first) = sub.iter().position(Item::selectable) { self.sel.push(first) }
        cx.notify();
        true
    }

    fn activate(&mut self, cx: &mut Context<Self>) {
        if self.enter_sub(cx) { return }
        if let Some(Item::Row { cmd: Some(cmd), enabled: true, .. }) = self.current() { cx.emit(MenuEvent::Pick(cmd.clone())) }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        if self.sel.len() > 1 { self.sel.pop(); cx.notify() }
    }

    fn panel_height(list: &[Item]) -> f32 { list.iter().map(Item::height).sum::<f32>() + PAD * 2. + 2. }

    fn panel(&self, depth: usize, list: &[Item], width: f32, cx: &mut Context<Self>) -> Div {
        let sel = self.sel.get(depth).copied();
        let mut col = div()
            .w(px(width))
            .py(px(PAD))
            .flex()
            .flex_col()
            .bg(theme::raised(Theme::popover_bg()))
            .border_1()
            .border_color(Theme::line_border())
            .rounded(px(RADIUS))
            .shadow(theme::shadow_menu())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .text_color(Theme::text())
            .occlude();
        for (i, it) in list.iter().enumerate() {
            col = col.child(match it {
                Item::Sep => div().h(px(SEP_H)).flex().items_center().mx(px(PAD + 8.)).child(div().h(px(1.)).flex_1().bg(Theme::line_divider())).into_any_element(),
                Item::Note(t) => div().h(px(ROW_H)).mx(px(PAD)).px(px(10.)).flex().items_center().text_color(Theme::text_caption()).child(t.clone()).into_any_element(),
                Item::Row { label, glyph, keys, enabled, sub, .. } => {
                    let on = sel == Some(i);
                    let enabled = *enabled;
                    let has_sub = sub.is_some();
                    let fg = if enabled { Theme::text() } else { Theme::text_disabled() };
                    let glyph_el: AnyElement = match glyph {
                        Glyph::Icon(g) => icon(g, 16., if enabled { Theme::icon() } else { Theme::text_disabled() }).into_any_element(),
                        Glyph::Image(img_) => img(img_.clone()).size(px(16.)).into_any_element(),
                        Glyph::Check(c) => if *c { icon("check", 16., Theme::icon()).into_any_element() } else { div().into_any_element() },
                    };
                    div()
                        .id(("menu-row", depth * 1000 + i))
                        .h(px(ROW_H))
                        .mx(px(PAD))
                        .pl(px(8.))
                        .pr(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(ROW_R))
                        .text_color(fg)
                        .when(enabled, |d| d.cursor_pointer())
                        .when(on && enabled, |d| d.bg(Theme::fill_hover_strong()))
                        .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().mr(px(8.)).child(glyph_el))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label.clone()))
                        .when(!keys.is_empty(), |d| d.child(div().flex_none().ml(px(16.)).text_size(px(12.)).text_color(Theme::text_placeholder()).child(*keys)))
                        .when(has_sub, |d| d.child(div().flex_none().ml(px(8.)).child(icon("chevron-right", 14., Theme::icon_muted()))))
                        .when(enabled, |d| d
                            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                if this.sel.len() == depth + 1 && this.sel[depth] == i { return }
                                this.sel.truncate(depth);
                                this.sel.push(i);
                                cx.notify();
                            }))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.sel.truncate(depth);
                                this.sel.push(i);
                                this.activate(cx);
                            })))
                        .into_any_element()
                }
            });
        }
        col
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        // Root panel: right and down from the pointer, flipped left / up when it would leave the window.
        let w0 = self.width;
        let h0 = Self::panel_height(&self.items);
        let (px0, py0) = (f32::from(self.pos.x), f32::from(self.pos.y));
        let x0 = if px0 + w0 > vw - MARGIN { (px0 - w0).max(MARGIN) } else { px0 };
        let y0 = if py0 + h0 > vh - MARGIN { (py0 - h0).max(MARGIN).min((vh - MARGIN - h0).max(MARGIN)) } else { py0 };
        let mut overlay = div()
            .id("menu-overlay")
            .track_focus(&self.focus)
            .key_context("Menu")
            .w(px(vw))
            .h(px(vh))
            .occlude()
            .on_action(cx.listener(|this, _: &MenuDown, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &MenuUp, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &MenuRight, _, cx| { this.enter_sub(cx); }))
            .on_action(cx.listener(|this, _: &MenuLeft, _, cx| this.back(cx)))
            .on_action(cx.listener(|this, _: &MenuConfirm, _, cx| this.activate(cx)))
            .on_action(cx.listener(|this, _: &MenuCancel, _, cx| if this.sel.len() > 1 { this.back(cx) } else { cx.emit(MenuEvent::Dismiss) }))
            .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, cx| cx.emit(MenuEvent::Dismiss)))
            .on_mouse_down(MouseButton::Right, cx.listener(|_, _, _, cx| cx.emit(MenuEvent::Dismiss)));
        let items = self.items.clone();
        overlay = overlay.child(self.panel(0, &items, w0, cx).absolute().left(px(x0)).top(px(y0)));
        // Open submenus, each beside its row; to the left when the right has no room.
        let (mut x, mut y, mut w, mut list) = (x0, y0, w0, items);
        for depth in 0..self.sel.len() {
            let Some(&i) = self.sel.get(depth) else { break };
            let Some(sub) = list.get(i).and_then(Item::submenu).cloned() else { break };
            let row_top = y + 1. + PAD + list[..i].iter().map(Item::height).sum::<f32>();
            let sw = 240.;
            let sh = Self::panel_height(&sub);
            let sx = if x + w - 2. + sw > vw - MARGIN { (x - sw + 2.).max(MARGIN) } else { x + w - 2. };
            let sy = (row_top - PAD - 1.).min(vh - MARGIN - sh).max(MARGIN);
            overlay = overlay.child(self.panel(depth + 1, &sub, sw, cx).absolute().left(px(sx)).top(px(sy)));
            (x, y, w, list) = (sx, sy, sw, sub);
        }
        overlay
    }
}
