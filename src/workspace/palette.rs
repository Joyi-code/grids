//! The command palette (Ctrl+K): a 560px card at the top of the window that
//! finds commands, the panes' folders, favourites, sessions, drives and
//! recent folders by fuzzy match. Enter runs the pick in the active pane,
//! Ctrl+Enter in the next one. GPUI-drawn with the menus' tokens.

use super::*;
use crate::pane;
use grids_core::{frecency, fuzzy};

actions!(palette, [PaletteUp, PaletteDown, PaletteOpenNext, PalettePageUp, PalettePageDown]);

pub fn bind_keys(cx: &mut App) {
    let c = Some("Palette");
    cx.bind_keys([
        KeyBinding::new("up", PaletteUp, c),
        KeyBinding::new("down", PaletteDown, c),
        KeyBinding::new("pageup", PalettePageUp, c),
        KeyBinding::new("pagedown", PalettePageDown, c),
        KeyBinding::new("secondary-enter", PaletteOpenNext, c),
    ]);
}

const WIDTH: f32 = 560.;
const ROW: f32 = 34.;
const ROWS: usize = 12;
/// Results kept for a query (the list scrolls).
const MAX_RESULTS: usize = 100;
/// Recent folders listed before anything is typed.
const RECENT_IDLE: usize = 8;

#[derive(Clone)]
pub(super) enum Act {
    Action(Rc<dyn Action>),
    Folder(PathBuf),
    Session(String),
    Today,
    SaveSession,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pane(usize),
    Recent,
    Favourite,
    Place,
    Drive,
    Session,
    Command,
}

#[derive(Clone)]
struct Item {
    kind: Kind,
    glyph: &'static str,
    label: String,
    /// Matched too (the full path, or the other language's command name), not highlighted.
    alt: String,
    /// Shown on the right: the shortcut of a command, the path of a folder.
    detail: String,
    act: Act,
    /// Frecency of a recent folder (ranking bonus).
    boost: f64,
    /// Listed before anything is typed.
    idle: bool,
}

pub(super) struct Palette {
    input: Entity<TextInput>,
    items: Vec<Item>,
    /// (item index, highlighted byte ranges of the label)
    results: Vec<(usize, Vec<std::ops::Range<usize>>)>,
    sel: usize,
    scroll: ScrollHandle,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

/// A command the palette lists: its names, glyph and action.
struct Command {
    zh: &'static str,
    en: &'static str,
    glyph: &'static str,
    make: fn() -> Act,
}

fn act<A: Action + Default>() -> Act { Act::Action(Rc::new(A::default())) }

const COMMANDS: &[Command] = &[
    Command { zh: "今日视图", en: "Today", glyph: "calendar-clock", make: || Act::Today },
    Command { zh: "新建文件夹", en: "New folder", glyph: "folder-plus", make: act::<pane::NewFolder> },
    Command { zh: "刷新", en: "Refresh", glyph: "refresh-cw", make: act::<pane::Refresh> },
    Command { zh: "上一级", en: "Up one level", glyph: "arrow-up", make: act::<pane::GoUp> },
    Command { zh: "后退", en: "Back", glyph: "arrow-left", make: act::<pane::GoBack> },
    Command { zh: "前进", en: "Forward", glyph: "arrow-right", make: act::<pane::GoForward> },
    Command { zh: "编辑地址", en: "Edit address", glyph: "text-cursor-input", make: act::<pane::EditAddress> },
    Command { zh: "按名称筛选", en: "Filter by name", glyph: "search", make: act::<pane::FilterNames> },
    Command { zh: "显示或隐藏隐藏项目", en: "Show or hide hidden items", glyph: "eye", make: act::<pane::ToggleHidden> },
    Command { zh: "全选", en: "Select all", glyph: "list-checks", make: act::<pane::SelectAll> },
    Command { zh: "粘贴", en: "Paste", glyph: "clipboard-paste", make: act::<pane::PasteFiles> },
    Command { zh: "复制路径", en: "Copy path", glyph: "link", make: act::<pane::CopyPath> },
    Command { zh: "添加到暂存架", en: "Add to shelf", glyph: "inbox", make: act::<pane::AddToShelf> },
    Command { zh: "重命名", en: "Rename", glyph: "pencil", make: act::<pane::Rename> },
    Command { zh: "移入回收站", en: "Move to Recycle Bin", glyph: "trash-2", make: act::<pane::Recycle> },
    Command { zh: "属性", en: "Properties", glyph: "info", make: act::<pane::Properties> },
    Command { zh: "视图：超大图标", en: "View: huge icons", glyph: "layout-grid", make: act::<pane::ViewHuge> },
    Command { zh: "视图：大图标", en: "View: large icons", glyph: "layout-grid", make: act::<pane::ViewLarge> },
    Command { zh: "视图：中图标", en: "View: medium icons", glyph: "layout-grid", make: act::<pane::ViewMedium> },
    Command { zh: "视图：列表", en: "View: list", glyph: "list", make: act::<pane::ViewList> },
    Command { zh: "视图：详细信息", en: "View: details", glyph: "table-2", make: act::<pane::ViewDetails> },
    Command { zh: "最大化或还原当前格子", en: "Maximize or restore pane", glyph: "maximize-2", make: act::<ToggleMaximize> },
    Command { zh: "下一个格子", en: "Next pane", glyph: "arrow-right-left", make: act::<NextPane> },
    Command { zh: "选择布局", en: "Choose layout", glyph: "layout-grid", make: act::<PickLayout> },
    Command { zh: "显示或收起侧栏", en: "Show or hide sidebar", glyph: "panel-left", make: act::<ToggleSidebar> },
    Command { zh: "切换外观", en: "Switch appearance", glyph: "sun-moon", make: act::<CycleTheme> },
    Command { zh: "保存当前会话", en: "Save current session", glyph: "layers", make: || Act::SaveSession },
    Command { zh: "与左侧格子交换", en: "Swap with pane on the left", glyph: "arrow-left", make: act::<SwapLeft> },
    Command { zh: "与右侧格子交换", en: "Swap with pane on the right", glyph: "arrow-right", make: act::<SwapRight> },
];

/// "secondary-shift-n" as the user reads it: Ctrl+Shift+N.
fn keys_text(b: &KeyBinding) -> String {
    b.keystrokes().iter().map(|k| {
        let m = k.modifiers();
        let mut s = String::new();
        if m.control || m.platform { s += if cfg!(target_os = "macos") && m.platform { "Cmd+" } else { "Ctrl+" } }
        if m.alt { s += "Alt+" }
        if m.shift { s += "Shift+" }
        let key = k.key();
        s += &match key {
            "enter" => "Enter".into(), "escape" => "Esc".into(), "delete" => "Del".into(), "backspace" => "Backspace".into(),
            "tab" => "Tab".into(), "space" => "Space".into(), "up" => "↑".into(), "down" => "↓".into(), "left" => "←".into(), "right" => "→".into(),
            "pageup" => "PgUp".into(), "pagedown" => "PgDn".into(), "home" => "Home".into(), "end" => "End".into(),
            k => k.to_uppercase(),
        };
        s
    }).collect::<Vec<_>>().join(" ")
}

fn now_secs() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) }

/// Char positions → byte ranges of `s` (consecutive chars merged).
fn byte_ranges(s: &str, positions: &[usize]) -> Vec<std::ops::Range<usize>> {
    let idx: Vec<(usize, char)> = s.char_indices().collect();
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for &p in positions {
        let Some(&(b, ch)) = idx.get(p) else { continue };
        let r = b..b + ch.len_utf8();
        match out.last_mut() { Some(l) if l.end == r.start => l.end = r.end, _ => out.push(r) }
    }
    out
}

impl Workspace {
    fn palette_items(&self, window: &Window, cx: &App) -> Vec<Item> {
        let en = state::en(cx);
        let cfg = &state::shared(cx).config;
        let mut items: Vec<Item> = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut folder = |items: &mut Vec<Item>, kind: Kind, glyph: &'static str, label: String, path: &PathBuf, boost: f64, idle: bool| {
            if seen.iter().any(|s| grids_core::link::same_path(s, path)) { return }
            seen.push(path.clone());
            let detail = path.to_string_lossy().into_owned();
            items.push(Item { kind, glyph, label, alt: detail.clone(), detail, act: Act::Folder(path.clone()), boost, idle });
        };
        for (i, p) in self.panes.iter().enumerate() {
            let path = p.read(cx).state.path.clone();
            folder(&mut items, Kind::Pane(i), "folder-open", grids_core::paths::display_name(&path), &path, 0., true);
        }
        let now = now_secs();
        let ranked = frecency::ranked(&cfg.recent, now);
        for (k, r) in ranked.iter().enumerate() {
            folder(&mut items, Kind::Recent, "history", grids_core::paths::display_name(&r.path), &r.path, frecency::score(r, now), k < RECENT_IDLE);
        }
        for f in &cfg.favourites { folder(&mut items, Kind::Favourite, "star", grids_core::paths::display_name(f), f, 0., true) }
        for s in &cfg.sessions {
            let n = s.session.layout.count();
            items.push(Item { kind: Kind::Session, glyph: "layers", label: s.name.clone(), alt: String::new(), detail: if en { format!("{n} panes") } else { format!("{n} 格") }, act: Act::Session(s.name.clone()), boost: 0., idle: true });
        }
        let pane_focus = self.active_pane().map(|p| p.read(cx).focus.clone());
        for c in COMMANDS {
            let a = (c.make)();
            let detail = match (&a, &pane_focus) {
                (Act::Action(a), Some(f)) => window.bindings_for_action_in(&**a, f).first().map(keys_text).unwrap_or_default(),
                _ => String::new(),
            };
            let (label, alt) = if en { (c.en, c.zh) } else { (c.zh, c.en) };
            items.push(Item { kind: Kind::Command, glyph: c.glyph, label: label.into(), alt: alt.into(), detail, act: a, boost: 0., idle: true });
        }
        for k in &self.known {
            let label = match k.kind {
                KnownFolderKind::Home => tr(cx, "主文件夹", "Home"),
                KnownFolderKind::Desktop => tr(cx, "桌面", "Desktop"),
                KnownFolderKind::Downloads => tr(cx, "下载", "Downloads"),
                KnownFolderKind::Documents => tr(cx, "文档", "Documents"),
                KnownFolderKind::Pictures => tr(cx, "图片", "Pictures"),
            };
            folder(&mut items, Kind::Place, "folder", label.into(), &k.path, 0., false);
        }
        for d in &self.drives {
            let root = d.path.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
            let label = if d.label.is_empty() { if root.is_empty() { "/".to_string() } else { root } } else { d.label.clone() };
            folder(&mut items, Kind::Drive, "hard-drive", label, &d.path, 0., false);
        }
        items
    }

    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() { return self.close_palette(window, cx) }
        self.picker = false;
        self.version_pop = false;
        let input = cx.new(|cx| TextInput::new("", tr(cx, "搜索命令、文件夹、会话…", "Search commands, folders, sessions…"), cx));
        let subs = vec![cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::Changed => { this.filter_palette(cx); cx.notify() }
            InputEvent::Submit => this.run_palette(false, window, cx),
            InputEvent::Cancel => this.close_palette(window, cx),
        })];
        input.read(cx).focus(window);
        let items = self.palette_items(window, cx);
        self.palette = Some(Palette { input, items, results: vec![], sel: 0, scroll: ScrollHandle::new(), focus: cx.focus_handle(), _subs: subs });
        self.filter_palette(cx);
        cx.notify();
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() { self.refocus(window, cx) }
    }

    fn filter_palette(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &mut self.palette else { return };
        let q = p.input.read(cx).text().trim().to_string();
        if q.is_empty() {
            p.results = p.items.iter().enumerate().filter(|(_, it)| it.idle).map(|(i, _)| (i, vec![])).collect();
        } else {
            let mut scored: Vec<(i32, usize, Vec<std::ops::Range<usize>>)> = p.items.iter().enumerate().filter_map(|(i, it)| {
                let main = fuzzy::fuzzy_match(&q, &it.label).map(|m| (m.score, byte_ranges(&it.label, &m.positions)));
                let alt = (!it.alt.is_empty()).then(|| fuzzy::fuzzy_match(&q, &it.alt)).flatten().map(|m| (m.score - 8, vec![]));
                let (score, hits) = match (main, alt) {
                    (Some(a), Some(b)) => if a.0 >= b.0 { a } else { b },
                    (Some(a), None) | (None, Some(a)) => a,
                    (None, None) => return None,
                };
                let bonus = (it.boost.ln_1p() * 6.).min(20.) as i32 + if matches!(it.kind, Kind::Pane(_)) { 4 } else { 0 };
                Some((score + bonus, i, hits))
            }).collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            scored.truncate(MAX_RESULTS);
            p.results = scored.into_iter().map(|(_, i, h)| (i, h)).collect();
        }
        p.sel = 0;
        p.scroll.scroll_to_item(0);
    }

    fn move_palette(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(p) = &mut self.palette else { return };
        let n = p.results.len();
        if n == 0 { return }
        let s = p.sel as isize + delta;
        // single steps wrap; pages stop at the ends
        p.sel = if delta.abs() == 1 { s.rem_euclid(n as isize) as usize } else { s.clamp(0, n as isize - 1) as usize };
        p.scroll.scroll_to_item(p.sel);
        cx.notify();
    }

    /// Runs the selected result in the active pane (`next`: the pane after it).
    fn run_palette(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.palette.take() else { return };
        let Some(item) = p.results.get(p.sel).and_then(|(i, _)| p.items.get(*i)).cloned() else { return self.refocus(window, cx) };
        let n = self.panes.len();
        if next && n > 1 {
            let j = (self.active + 1) % n;
            self.set_active(j, window, cx);
        } else {
            self.refocus(window, cx);
        }
        match item.act {
            Act::Folder(path) => {
                if self.today.is_some() { self.close_today(window, cx) }
                self.go_to(path, window, cx)
            }
            Act::Session(name) => {
                let session = state::shared(cx).config.sessions.iter().find(|s| s.name == name).map(|s| s.session.clone());
                if let Some(session) = session {
                    cx.global_mut::<Shared>().config.session_name = Some(name);
                    self.load_session(session, window, cx);
                    self.schedule_save(cx);
                }
            }
            Act::Today => self.open_today(window, cx),
            Act::SaveSession => {
                if !state::shared(cx).config.sidebar_open { self.toggle_sidebar(cx) }
                self.start_save_session(window, cx)
            }
            Act::Action(a) => {
                if self.today.is_some() { self.close_today(window, cx) }
                window.dispatch_action(a.boxed_clone(), cx)
            }
        }
        cx.notify();
    }

    pub(super) fn render_palette(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.palette.as_ref()?;
        let en = state::en(cx);
        let kind_text = |k: Kind| -> String {
            match (k, en) {
                (Kind::Pane(i), false) => format!("格子 {}", i + 1),
                (Kind::Pane(i), true) => format!("Pane {}", i + 1),
                (Kind::Recent, false) => "最近".into(), (Kind::Recent, true) => "Recent".into(),
                (Kind::Favourite, false) => "收藏".into(), (Kind::Favourite, true) => "Favourite".into(),
                (Kind::Place, false) => "位置".into(), (Kind::Place, true) => "Place".into(),
                (Kind::Drive, false) => "磁盘".into(), (Kind::Drive, true) => "Drive".into(),
                (Kind::Session, false) => "会话".into(), (Kind::Session, true) => "Session".into(),
                (Kind::Command, false) => "命令".into(), (Kind::Command, true) => "Command".into(),
            }
        };
        let mut list = div().id("palette-list").flex().flex_col().px(px(5.)).py(px(4.)).max_h(px(ROW * ROWS as f32 + 8.)).overflow_y_scroll().track_scroll(&p.scroll);
        for (row, (i, hits)) in p.results.iter().enumerate() {
            let Some(it) = p.items.get(*i) else { continue };
            let on = row == p.sel;
            let is_cmd = it.kind == Kind::Command;
            list = list.child(
                div().id(("palette-row", row)).h(px(ROW)).flex_none().px(px(10.)).rounded(px(6.)).flex().items_center().gap(px(10.)).cursor_pointer()
                    .when(on, |d| d.bg(Theme::fill_hover_strong()))
                    .when(!on, |d| d.hover(|s| s.bg(Theme::fill_hover())))
                    .child(icon(it.glyph, 16., if it.kind == Kind::Favourite { Theme::star() } else { Theme::icon() }))
                    .child(div().flex_none().max_w(px(if is_cmd { 300. } else { 220. })).overflow_hidden().whitespace_nowrap().text_ellipsis()
                        .child(ui_text(it.label.clone()).hits(hits.clone(), Theme::primary())))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(Theme::text_caption())
                        .when(!is_cmd, |d| d.child(ui_text(it.detail.clone()))))
                    .when(is_cmd && !it.detail.is_empty(), |d| d.child(div().flex_none().px(px(6.)).h(px(20.)).rounded(px(5.)).flex().items_center().bg(Theme::fill_hover()).text_size(px(11.)).text_color(Theme::text_caption()).child(it.detail.clone())))
                    .child(div().flex_none().w(px(56.)).flex().justify_end().text_size(px(11.)).text_color(Theme::text_placeholder()).child(kind_text(it.kind)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        if let Some(p) = &mut this.palette { p.sel = row }
                        this.run_palette(ev.modifiers().control, window, cx)
                    })),
            );
        }
        let empty = p.results.is_empty();
        let card = div()
            .id("palette")
            .w(px(WIDTH))
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
            .track_focus(&p.focus)
            .key_context("Palette")
            .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.move_palette(-1, cx)))
            .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.move_palette(1, cx)))
            .on_action(cx.listener(|this, _: &PalettePageUp, _, cx| this.move_palette(-(ROWS as isize - 1), cx)))
            .on_action(cx.listener(|this, _: &PalettePageDown, _, cx| this.move_palette(ROWS as isize - 1, cx)))
            .on_action(cx.listener(|this, _: &PaletteOpenNext, window, cx| this.run_palette(true, window, cx)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().h(px(46.)).flex().items_center().gap(px(10.)).px(px(16.))
                .child(icon("search", 17., Theme::icon_muted()))
                .child(div().flex_1().min_w_0().text_size(px(14.)).child(p.input.clone()))
                .child(div().flex_none().text_size(px(11.)).text_color(Theme::text_placeholder()).child("Esc")))
            .child(div().h(px(1.)).bg(Theme::line_divider()))
            .map(|d| if empty {
                d.child(div().h(px(64.)).flex().items_center().justify_center().text_color(Theme::text_caption()).child(tr(cx, "没有匹配的结果", "No matches")))
            } else { d.child(list) })
            .child(div().h(px(1.)).bg(Theme::line_divider()))
            .child(div().h(px(30.)).flex().items_center().gap(px(14.)).px(px(16.)).text_size(px(11.)).text_color(Theme::text_placeholder())
                .child(tr(cx, "↑↓ 选择", "↑↓ select"))
                .child(tr(cx, "Enter 在当前格子执行", "Enter run in this pane"))
                .child(tr(cx, "Ctrl+Enter 在下一个格子打开", "Ctrl+Enter open in next pane")));
        Some(
            deferred(
                div().id("palette-scrim").absolute().inset_0().flex().flex_col().items_center().pt(px(72.)).bg(gpui::black().opacity(0.12)).occlude()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_palette(window, cx)))
                    .child(card)
                    .with_animation("palette-fade", Animation::new(Duration::from_millis(90)), |d, t| d.opacity(t)),
            )
            .with_priority(5)
            .into_any_element(),
        )
    }
}
