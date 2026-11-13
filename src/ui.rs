//! Small shared UI pieces: icons, tooltips, popover shells, menu rows,
//! title bar behaviour, window controls and single-line text with ellipsis.
//! Adapted from quiet-md (MIT, see THIRD_PARTY.md).

use crate::theme::{self, Theme};
use gpui::{prelude::FluentBuilder, *};
use std::cell::RefCell;
use std::rc::Rc;

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

/// Popover shell shared by the slash menu, block menu and toolbar dropdowns:
/// the light panel (macOS 26 menu material, near-white), hairline,
/// radius 12, soft drop.
pub fn popover() -> Div {
    div()
        .bg(theme::raised(Theme::popover_bg()))
        .border_1()
        .border_color(Theme::line_border())
        .rounded(px(MENU_RADIUS))
        .shadow(theme::shadow_menu())
        .font_family(theme::UI_FONT)
        .text_color(Theme::text())
}

/// Menus and popovers round at 12 (the native menu panel); their
/// rows at 6, 5pt in from the edge.
pub const MENU_RADIUS: f32 = 12.;
pub const ROW_RADIUS: f32 = 6.;

/// A menu row: 32 tall, 13pt label, 16pt glyph; hovering lays the quiet
/// fill the editor's menus use, and text and glyph keep their colours.
pub const MENU_ROW_H: f32 = 32.;
/// How far a menu row sits in from the menu's sides.
pub const MENU_INSET: f32 = 5.;

pub fn menu_row(id: impl Into<ElementId>, group: SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .group(group)
        .flex_none()
        .h(px(MENU_ROW_H))
        .mx(px(MENU_INSET))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(ROW_RADIUS))
        .cursor_pointer()
        .text_size(px(13.))
        .line_height(px(20.))
        .text_color(Theme::text())
        .hover(|s| s.bg(Theme::fill_hover()))
}

/// Tooltip bubble: the dark rounded tip (12pt white on near-black, radius 7, arrow).
pub struct Tooltip {
    /// One or more lines.
    label: SharedString,
    /// Window bounds of the trigger, when it was registered through [`TipExt::tip`].
    anchor: Option<Bounds<Pixels>>,
    /// Where GPUI anchors the tooltip root (mouse + 1 at show time).
    origin: Point<Pixels>,
    side: TipSide,
}

/// Preferred side: chrome shows tooltips below, document controls above,
/// menu items to the right. Flipped when there is no room.
#[derive(Clone, Copy, PartialEq)]
pub enum TipSide {
    Below,
    Above,
    Right,
}

const TIP_GAP: f32 = 8.;
const TIP_ARROW: f32 = 5.;
const TIP_PAD: f32 = 12.;
const TIP_LINE: f32 = 18.;
const TIP_BG: u32 = 0x2a2c2f;
/// Tooltips stay dark in both looks (a step lighter on the dark page).
fn tip_bg() -> Hsla { if theme::dark() { theme::rgbaf(0x3a, 0x3b, 0x3d, 1.) } else { theme::rgbaf((TIP_BG >> 16) as u8, (TIP_BG >> 8) as u8, TIP_BG as u8, 1.) } }

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let lines: Vec<SharedString> = self.label.split('\n').map(|l| SharedString::from(l.to_string())).collect();
        let bubble = div()
            .px(px(10.))
            .py(px(6.))
            .rounded(px(7.))
            .bg(tip_bg())
            .text_color(gpui::white())
            .text_size(px(12.))
            .line_height(px(TIP_LINE))
            .font_family(theme::UI_FONT)
            .whitespace_nowrap()
            .flex()
            .flex_col()
            .children(lines.iter().map(|l| ui_text(l.clone())));
        let Some(anchor) = self.anchor else {
            return div().pl(px(8.)).pt(px(10.)).child(bubble).into_any_element();
        };
        // 10px from the trigger with an arrow pointing at it, on the preferred
        // side unless it doesn't fit, and kept inside the window.
        let mut style = window.text_style();
        style.font_family = theme::UI_FONT.into();
        let text_w = lines
            .iter()
            .map(|l| {
                let runs = ui_text(l.clone()).runs(&style);
                window.text_system().shape_line(l.clone(), px(12.), &runs, None).width
            })
            .fold(px(0.), |a, b| a.max(b));
        let w = text_w + px(20.);
        let h = px(TIP_PAD + TIP_LINE * lines.len() as f32);
        let vp = window.viewport_size();
        let gap = px(TIP_GAP);
        let s = px(TIP_ARROW);
        // (bubble origin, arrow base centre, arrow direction)
        let (x, y, arrow): (Pixels, Pixels, Box<dyn Fn(&mut PathBuilder)>) = if self.side == TipSide::Right
            && (anchor.right() + gap + w <= vp.width || anchor.left() - gap - w < px(0.))
        {
            let x = anchor.right() + gap;
            let cy = anchor.center().y;
            let y = (cy - h / 2.).min(vp.height - h).max(px(0.));
            let tip_y = cy.max(y + px(10.)).min(y + h - px(10.));
            (x, y, Box::new(move |p: &mut PathBuilder| {
                p.move_to(point(x, tip_y - s));
                p.line_to(point(x - s * 0.88, tip_y - px(0.8)));
                p.line_to(point(x - s * 0.88, tip_y + px(0.8)));
                p.line_to(point(x, tip_y + s));
            }))
        } else if self.side == TipSide::Right {
            let x = anchor.left() - gap - w;
            let cy = anchor.center().y;
            let y = (cy - h / 2.).min(vp.height - h).max(px(0.));
            let tip_y = cy.max(y + px(10.)).min(y + h - px(10.));
            let bx = x + w;
            (x, y, Box::new(move |p: &mut PathBuilder| {
                p.move_to(point(bx, tip_y - s));
                p.line_to(point(bx + s * 0.88, tip_y - px(0.8)));
                p.line_to(point(bx + s * 0.88, tip_y + px(0.8)));
                p.line_to(point(bx, tip_y + s));
            }))
        } else {
            let cx = anchor.center().x;
            let x = (cx - w / 2.).min(vp.width - w).max(px(0.));
            let fits_below = anchor.bottom() + gap + h <= vp.height;
            let fits_above = anchor.top() - gap - h >= px(0.);
            let below = if self.side == TipSide::Above { !fits_above } else { fits_below };
            let y = if below { anchor.bottom() + gap } else { anchor.top() - gap - h };
            let tip_x = cx.max(x + px(10.)).min(x + w - px(10.));
            let (base, apex) = if below { (y, y - s) } else { (y + h, y + h + s) };
            (x, y, Box::new(move |p: &mut PathBuilder| {
                p.move_to(point(tip_x - s, base));
                p.line_to(point(tip_x - px(0.8), apex + (base - apex) * 0.12));
                p.line_to(point(tip_x + px(0.8), apex + (base - apex) * 0.12));
                p.line_to(point(tip_x + s, base));
            }))
        };
        let o = self.origin;
        let arrow = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let mut p = PathBuilder::fill();
                arrow(&mut p);
                p.close();
                if let Ok(path) = p.build() {
                    window.paint_path(path, tip_bg());
                }
            },
        )
        .absolute()
        .size(px(1.));
        div()
            .size(px(1.))
            .child(div().absolute().left(x - o.x).top(y - o.y).child(bubble))
            .child(arrow)
            .into_any_element()
    }
}

/// Tooltip anchored on the element rather than the mouse.
pub trait TipExt: Sized {
    /// Window chrome: below, shortcut in full-width parens ("展开面板（⌘ + ]）").
    fn tip(self, text: &str, keys: Option<&str>) -> Self {
        let label = match keys {
            Some(k) => format!("{text}（{k}）"),
            None => text.to_string(),
        };
        self.tip_label(label, TipSide::Below)
    }
    fn tip_label(self, label: String, side: TipSide) -> Self;
}

/// Window chrome above the page's top line works like a native title bar: a
/// press drags the window, a double press zooms it (per the system setting).
/// Controls on it opt out with `no_drag`, so double-clicking them doesn't zoom.
pub trait TitleBarExt: InteractiveElement + Sized {
    fn title_bar(self) -> Self {
        // Keep the workspace's focus handler from preventing the native caption action.
        if cfg!(target_os = "windows") { return self.occlude().window_control_area(WindowControlArea::Drag); }
        self.on_mouse_down(MouseButton::Left, |ev, window, _| match ev.click_count {
            1 => window.start_window_move(),
            2 => window.titlebar_double_click(),
            _ => {}
        })
    }
    fn no_drag(self) -> Self {
        let element = if cfg!(target_os = "windows") { self.occlude() } else { self };
        element.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    }
}

impl<E: InteractiveElement> TitleBarExt for E {}

impl TipExt for Stateful<Div> {
    fn tip_label(self, label: String, side: TipSide) -> Self {
        let bounds = Rc::new(std::cell::Cell::new(None::<Bounds<Pixels>>));
        let record = bounds.clone();
        let label = SharedString::from(label);
        self.child(canvas(move |b, _, _| record.set(Some(b)), |_, _, _, _| {}).absolute().inset_0()).tooltip(move |window, cx| {
            let origin = window.mouse_position() + point(px(1.), px(1.));
            let (label, anchor) = (label.clone(), bounds.get());
            cx.new(|_| Tooltip { label, anchor, origin, side }).into()
        })
    }
}

/// Full-width CJK punctuation (、。〈〉《》「」『』【】〔〕（）：，… in U+3001–301B / U+FF01–FF65).
pub fn is_cjk_punct(ch: char) -> bool {
    matches!(ch, '\u{3001}'..='\u{3003}' | '\u{3008}'..='\u{3011}' | '\u{3014}'..='\u{301B}'
        | '\u{FF01}'..='\u{FF0F}' | '\u{FF1A}'..='\u{FF20}' | '\u{FF3B}'..='\u{FF40}' | '\u{FF5B}'..='\u{FF65}')
}

/// Font for full-width punctuation. When CoreText falls back from the
/// system UI font it squeezes these (an opening paren after a CJK
/// character loses ~5px). Browsers draw them full width with PingFang SC.
pub const CJK_PUNCT_FONT: &str = "PingFang SC";

/// Single-line UI text that keeps full-width punctuation at full width,
/// with an optional colored range (search hits). Inherits the parent text
/// style; honours `text_ellipsis()` against the final width (StyledText
/// caches its first, unconstrained measurement when nowrap).
pub struct UiText {
    text: SharedString,
    hit: Option<(std::ops::Range<usize>, Hsla)>,
}

pub fn ui_text(text: impl Into<SharedString>) -> UiText {
    UiText { text: text.into(), hit: None }
}

pub struct UiTextLayout {
    runs: Vec<TextRun>,
    font_size: Pixels,
    line_height: Pixels,
    ellipsis: Option<SharedString>,
    full: Rc<RefCell<Option<ShapedLine>>>,
}

impl UiText {
    pub fn hit(mut self, range: Option<std::ops::Range<usize>>, color: Hsla) -> Self {
        self.hit = range.map(|r| (r, color));
        self
    }

    fn runs(&self, style: &TextStyle) -> Vec<TextRun> {
        match &self.hit {
            Some((r, c)) => ui_runs(&self.text, style, std::slice::from_ref(r), *c),
            None => ui_runs(&self.text, style, &[], style.color),
        }
    }
}

/// Text runs that keep full-width punctuation at full width (see
/// [`CJK_PUNCT_FONT`]), with `hits` drawn in `hit_color`.
pub fn ui_runs(text: &str, style: &TextStyle, hits: &[std::ops::Range<usize>], hit_color: Hsla) -> Vec<TextRun> {
    let mut cuts = vec![0, text.len()];
    for (i, ch) in text.char_indices() {
        if is_cjk_punct(ch) {
            cuts.extend([i, i + ch.len_utf8()]);
        }
    }
    for r in hits {
        cuts.extend([r.start.min(text.len()), r.end.min(text.len())]);
    }
    cuts.sort();
    cuts.dedup();
    cuts.windows(2)
        .map(|w| {
            let mut run = style.to_run(w[1] - w[0]);
            if text[w[0]..w[1]].chars().all(is_cjk_punct) {
                run.font.family = CJK_PUNCT_FONT.into();
            }
            if hits.iter().any(|r| r.start <= w[0] && w[1] <= r.end) {
                run.color = hit_color;
            }
            run
        })
        .collect()
}

/// Longest prefix of `line` that fits `width` together with `suffix`.
fn truncated(line: &ShapedLine, runs: &[TextRun], suffix: &str, font_size: Pixels, width: Pixels, window: &mut Window) -> ShapedLine {
    let text = line.text.clone();
    let mut suffix_runs: Vec<TextRun> = runs.last().cloned().into_iter().collect();
    if let Some(r) = suffix_runs.first_mut() {
        r.len = suffix.len();
        r.font.family = theme::UI_FONT.into();
    }
    let suffix_w = window.text_system().shape_line(suffix.to_string().into(), font_size, &suffix_runs, None).width;
    let mut cut = 0;
    for (i, ch) in text.char_indices() {
        let end = i + ch.len_utf8();
        if line.x_for_index(end) + suffix_w > width {
            break;
        }
        cut = end;
    }
    let mut out = vec![];
    let mut at = 0;
    for r in runs {
        if at >= cut {
            break;
        }
        let mut r = r.clone();
        r.len = r.len.min(cut - at);
        at += r.len;
        out.push(r);
    }
    out.extend(suffix_runs);
    let s: SharedString = format!("{}{}", &text[..cut], suffix).into();
    window.text_system().shape_line(s, font_size, &out, None)
}

impl IntoElement for UiText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for UiText {
    type RequestLayoutState = UiTextLayout;
    type PrepaintState = Option<ShapedLine>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, UiTextLayout) {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style.line_height.to_pixels(font_size.into(), window.rem_size());
        let runs = if self.text.is_empty() { vec![] } else { self.runs(&style) };
        let ellipsis = style.text_overflow.as_ref().map(|TextOverflow::Truncate(s)| s.clone());
        let full = Rc::new(RefCell::new(None));
        let (text, r, f, clip) = (self.text.clone(), runs.clone(), full.clone(), ellipsis.is_some());
        let id = window.request_measured_layout(Style::default(), move |known, avail, window, _| {
            let mut slot = f.borrow_mut();
            let line = slot.get_or_insert_with(|| window.text_system().shape_line(text.clone(), font_size, &r, None));
            let mut w = line.width.ceil();
            if clip {
                let limit = known.width.or(match avail.width {
                    AvailableSpace::Definite(x) => Some(x),
                    _ => None,
                });
                if let Some(l) = limit {
                    w = w.min(l);
                }
            }
            size(w, line_height)
        });
        (id, UiTextLayout { runs, font_size, line_height, ellipsis, full })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut UiTextLayout,
        window: &mut Window,
        _: &mut App,
    ) -> Option<ShapedLine> {
        let full = layout.full.borrow().clone()?;
        match &layout.ellipsis {
            Some(suffix) if full.width > bounds.size.width + px(0.5) => {
                Some(truncated(&full, &layout.runs, suffix, layout.font_size, bounds.size.width, window))
            }
            _ => Some(full),
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut UiTextLayout,
        line: &mut Option<ShapedLine>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(line) = line {
            let _ = line.paint(bounds.origin, layout.line_height, window, cx);
        }
    }
}


/// Title bar height and its buttons.
pub const HEADER_H: f32 = 44.;
pub const HEADER_BTN: f32 = 30.;

/// Minimise / maximise / close at the right of the title bar (Windows and
/// Linux, where the app draws its own caption). On Windows the buttons are
/// native hit-test areas, which keeps Snap Layouts.
pub fn window_controls(window: &Window, en: bool) -> Option<Div> {
    if !cfg!(any(target_os = "windows", target_os = "linux")) { return None; }
    let tr = |zh: &'static str, e: &'static str| if en { e } else { zh };
    let button = |id: &'static str, area: WindowControlArea, label: &'static str| {
        div().id(id).size(px(HEADER_BTN)).mx(px(4.)).rounded(px(8.)).flex_none().flex().items_center().justify_center().cursor_pointer()
            .occlude()
            .when(cfg!(target_os = "linux"), |d| d.no_drag())
            .when(cfg!(target_os = "windows"), |d| d.window_control_area(area))
            .when(cfg!(target_os = "linux"), |d| d.on_click(move |_, window, _| match area {
                WindowControlArea::Min => window.minimize_window(),
                WindowControlArea::Max => window.zoom_window(),
                WindowControlArea::Close => window.remove_window(),
                WindowControlArea::Drag => {},
            }))
            .hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed())).tip(label, None)
    };
    let maximized = window.is_maximized();
    Some(div().h(px(HEADER_H)).pr(px(6.)).flex().flex_none().items_center()
        .child(div().mx(px(8.)).w(px(1.)).h(px(18.)).bg(Theme::line_divider()))
        .child(button("window-minimize", WindowControlArea::Min, tr("最小化", "Minimize")).child(icon("minus", 17., Theme::icon())))
        .child(button("window-maximize", WindowControlArea::Max, if maximized { tr("还原", "Restore") } else { tr("最大化", "Maximize") })
            .child(icon(if maximized { "copy" } else { "square" }, if maximized { 15. } else { 14. }, Theme::icon())))
        .child(button("window-close", WindowControlArea::Close, tr("关闭", "Close")).child(icon("x", 17., Theme::icon()))))
}

/// A square icon button (title bar, pane headers).
pub fn icon_button(id: impl Into<ElementId>, name: &'static str, size: f32, glyph: f32, enabled: bool) -> Stateful<Div> {
    div().id(id).size(px(size)).rounded(px(6.)).flex_none().flex().items_center().justify_center()
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())).active(|s| s.bg(Theme::fill_pressed())))
        .child(icon(name, glyph, if enabled { Theme::icon() } else { Theme::text_disabled() }))
}
