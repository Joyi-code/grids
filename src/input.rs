//! Single-line text field (address bar, rename, filter, session names).
//! Adapted from GPUI's `examples/input.rs` (Apache-2.0): IME, selection,
//! clipboard; adds Submit / Cancel events and horizontal scrolling.

use crate::theme::Theme;
use gpui::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(text_input, [Backspace, Delete, Left, Right, SelectLeft, SelectRight, SelectAll, Home, End, SelectHome, SelectEnd, Paste, Cut, Copy, Submit, Cancel, KeepUndo]);

pub fn bind_keys(cx: &mut App) {
    let c = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("secondary-a", SelectAll, c),
        KeyBinding::new("secondary-v", Paste, c),
        KeyBinding::new("secondary-c", Copy, c),
        KeyBinding::new("secondary-x", Cut, c),
        KeyBinding::new("enter", Submit, c),
        KeyBinding::new("escape", Cancel, c),
        // Ctrl+Z while typing must not undo a file operation (the
        // workspace binding): the field swallows it.
        KeyBinding::new("secondary-z", KeepUndo, c),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Changed,
    Submit,
    Cancel,
}

pub struct TextInput {
    focus: FocusHandle,
    pub content: String,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
    /// How far the text is scrolled left to keep the caret in view.
    scroll_x: Pixels,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() }
}

impl TextInput {
    pub fn new(text: &str, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: text.to_string(),
            placeholder: placeholder.into(),
            selected: 0..text.len(),
            reversed: false,
            marked: None,
            last_layout: None,
            last_bounds: None,
            selecting: false,
            scroll_x: px(0.),
        }
    }

    pub fn text(&self) -> &str { &self.content }

    /// Selects `range` (bytes), e.g. a file name without its extension.
    pub fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let r = range.start.min(self.content.len())..range.end.min(self.content.len());
        self.selected = r;
        self.reversed = false;
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window) { window.focus(&self.focus) }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() { self.move_to(self.previous_boundary(self.cursor()), cx) } else { self.move_to(self.selected.start, cx) }
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() { self.move_to(self.next_boundary(self.selected.end), cx) } else { self.move_to(self.selected.end, cx) }
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) { self.select_to(self.previous_boundary(self.cursor()), cx) }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) { self.select_to(self.next_boundary(self.cursor()), cx) }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) { self.move_to(0, cx) }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) { self.move_to(self.content.len(), cx) }
    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) { self.select_to(0, cx) }
    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) { self.select_to(self.content.len(), cx) }
    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() { self.select_to(self.previous_boundary(self.cursor()), cx) }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() { self.select_to(self.next_boundary(self.cursor()), cx) }
        self.replace_text_in_range(None, "", window, cx)
    }
    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) { cx.emit(InputEvent::Submit) }
    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) { cx.emit(InputEvent::Cancel) }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.focus(&self.focus);
        if ev.click_count >= 2 {
            self.move_to(0, cx);
            self.select_to(self.content.len(), cx);
            return;
        }
        self.selecting = true;
        if ev.modifiers.shift { self.select_to(self.index_for(ev.position), cx) } else { self.move_to(self.index_for(ev.position), cx) }
    }
    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) { self.selecting = false }
    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting { self.select_to(self.index_for(ev.position), cx) }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace(['\r', '\n'], " "), window, cx);
        }
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() { cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected.clone()].to_string())) }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected.clone()].to_string()));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        cx.notify()
    }
    fn cursor(&self) -> usize { if self.reversed { self.selected.start } else { self.selected.end } }

    fn index_for(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() { return 0 }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref()) else { return 0 };
        if position.y < bounds.top() { return 0 }
        if position.y > bounds.bottom() { return self.content.len() }
        line.closest_index_for_x(position.x - bounds.left() + self.scroll_x)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed { self.selected.start = offset } else { self.selected.end = offset };
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let (mut u8o, mut u16c) = (0, 0);
        for ch in self.content.chars() {
            if u16c >= offset { break }
            u16c += ch.len_utf16();
            u8o += ch.len_utf8();
        }
        u8o
    }
    fn offset_to_utf16(&self, offset: usize) -> usize {
        let (mut u16o, mut u8c) = (0, 0);
        for ch in self.content.chars() {
            if u8c >= offset { break }
            u8c += ch.len_utf8();
            u16o += ch.len_utf16();
        }
        u16o
    }
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> { self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end) }
    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> { self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end) }
    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }
    fn next_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(&mut self, range_utf16: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected), reversed: self.reversed })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> { self.marked.as_ref().map(|r| self.range_to_utf16(r)) }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) { self.marked = None }
    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.content = self.content[..range.start].to_owned() + new_text + &self.content[range.end..];
        self.selected = range.start + new_text.len()..range.start + new_text.len();
        self.marked.take();
        cx.emit(InputEvent::Changed);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, new_sel: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16.as_ref().map(|r| self.range_from_utf16(r)).or(self.marked.clone()).unwrap_or(self.selected.clone());
        self.content = self.content[..range.start].to_owned() + new_text + &self.content[range.end..];
        self.marked = if new_text.is_empty() { None } else { Some(range.start..range.start + new_text.len()) };
        self.selected = new_sel.as_ref().map(|r| self.range_from_utf16(r)).map(|n| n.start + range.start..n.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        cx.notify();
    }
    fn bounds_for_range(&mut self, range_utf16: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let x0 = bounds.left() - self.scroll_x;
        Some(Bounds::from_corners(point(x0 + line.x_for_index(range.start), bounds.top()), point(x0 + line.x_for_index(range.end), bounds.bottom())))
    }
    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let local = self.last_bounds?.localize(&p)?;
        let line = self.last_layout.as_ref()?;
        let i = line.index_for_x(local.x + self.scroll_x)?;
        Some(self.offset_to_utf16(i))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll_x: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> PrepaintState {
        let input = self.input.read(cx);
        let content: SharedString = input.content.clone().into();
        let selected = input.selected.clone();
        let cursor = input.cursor();
        let style = window.text_style();
        let (display, color) = if content.is_empty() { (input.placeholder.clone(), Theme::text_placeholder()) } else { (content, style.color) };
        let run = TextRun { len: display.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs = if let Some(m) = input.marked.as_ref().filter(|_| !input.content.is_empty()) {
            vec![
                TextRun { len: m.start, ..run.clone() },
                TextRun { len: m.end - m.start, underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.), wavy: false }), ..run.clone() },
                TextRun { len: display.len() - m.end, ..run },
            ].into_iter().filter(|r| r.len > 0).collect()
        } else {
            vec![run]
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(display, font_size, &runs, None);
        let caret_x = if input.content.is_empty() { px(0.) } else { line.x_for_index(cursor) };
        // Keep the caret inside the field.
        let w = bounds.size.width - px(2.);
        let mut sx = input.scroll_x;
        if caret_x - sx > w { sx = caret_x - w }
        if caret_x < sx { sx = caret_x }
        if line.width - sx < w { sx = (line.width - w).max(px(0.)) }
        let x0 = bounds.left() - sx;
        let (selection, cursor) = if selected.is_empty() {
            (None, Some(fill(Bounds::new(point(x0 + caret_x, bounds.top() + px(1.)), size(px(1.5), bounds.size.height - px(2.))), Theme::caret())))
        } else {
            (Some(fill(Bounds::from_corners(point(x0 + line.x_for_index(selected.start), bounds.top()), point(x0 + line.x_for_index(selected.end), bounds.bottom())), Theme::selection())), None)
        };
        PrepaintState { line: Some(line), cursor, selection, scroll_x: sx }
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), state: &mut PrepaintState, window: &mut Window, cx: &mut App) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let sx = state.scroll_x;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(sel) = state.selection.take() { window.paint_quad(sel) }
            if let Some(line) = state.line.as_ref() {
                let _ = line.paint(point(bounds.left() - sx, bounds.top()), window.line_height(), window, cx);
            }
            if focus.is_focused(window) && let Some(c) = state.cursor.take() { window.paint_quad(c) }
        });
        let line = state.line.take();
        self.input.update(cx, |input, _| {
            input.last_layout = line;
            input.last_bounds = Some(bounds);
            input.scroll_x = sx;
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .key_context("TextInput")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(|_: &KeepUndo, _, _| {})
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(TextElement { input: cx.entity() })
    }
}
