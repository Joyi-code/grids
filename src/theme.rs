//! Design tokens: palette, type, shapes and shadows. Taken from quiet-md
//! (MIT, see THIRD_PARTY.md): the same light look with a mirrored dark mode,
//! a sunken sidebar on the window surface and white content planes.

use gpui::{BoxShadow, Hsla, Rgba, WindowBackgroundAppearance, point, px, rgb};
use std::sync::atomic::{AtomicBool, Ordering};

// ---- dark mode
//
// Every colour here is designed for the light look. In dark mode the
// palette helpers (`c`, `ca`, `mix`, `ta`) pass through `adapt`, which
// mirrors lightness: the white page turns dark gray, near-black text
// near-white, while vivid colours (the accent) keep their hue.
// `rgbaf` stays literal for shadows and scrims.

// ---- window glass (as quiet-md)

/// Whether the window sits on blurred glass (acrylic). Windows 10 and 11
/// 21H2 drag acrylic windows with a lag, so there it stays opaque; Linux
/// compositors vary, so it is opaque there too.
pub fn glass() -> bool {
    #[cfg(target_os = "windows")] {
        static GLASS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *GLASS.get_or_init(|| std::env::var_os("GRIDS_OPAQUE").is_none() && windows_build().is_some_and(|b| b >= 22621))
    }
    #[cfg(target_os = "linux")] { false }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))] { true }
}

#[cfg(target_os = "windows")]
fn windows_build() -> Option<u32> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::w;
    let mut buf = [0u16; 32];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"), w!("CurrentBuildNumber"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    if r != ERROR_SUCCESS { return None }
    let n = (len as usize / 2).min(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').trim().parse().ok()
}

/// Blurred where `glass`, else opaque.
pub fn window_background() -> WindowBackgroundAppearance {
    if glass() { WindowBackgroundAppearance::Blurred } else { WindowBackgroundAppearance::Opaque }
}

static DARK: AtomicBool = AtomicBool::new(false);

static PACK: std::sync::OnceLock<grids_core::theme_pack::ThemePack> = std::sync::OnceLock::new();
static PACK_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

include!(concat!(env!("OUT_DIR"), "/bundled_theme.rs"));

/// Writes each bundled theme to `<dir>/themes/<pack>` when that pack is not
/// there yet, so a single exe carries its themes. Existing packs are left alone.
pub fn install_bundled(dir: &std::path::Path) -> bool {
    let themes = dir.join("themes");
    let mut wrote = false;
    // Every file that is missing gets written: a half-copied pack repairs
    // itself, and nothing that already exists is ever overwritten.
    for (rel, bytes) in THEME_FILES {
        let p = themes.join(rel);
        if p.is_file() { continue }
        if let Some(parent) = p.parent() { let _ = std::fs::create_dir_all(parent); }
        if std::fs::write(&p, bytes).is_ok() { wrote = true }
    }
    wrote
}

/// Applies a theme pack for this run; call before the window opens.
pub fn set_pack(pack: grids_core::theme_pack::ThemePack, dir: std::path::PathBuf) {
    let _ = PACK_DIR.set(dir);
    let _ = PACK.set(pack);
}

/// The active pack's folder.
pub fn dir() -> Option<&'static std::path::Path> { PACK_DIR.get().map(|p| p.as_path()) }

/// Whether the pack ships `rel` (e.g. `types/folder.svg`).
pub fn has(rel: &str) -> bool { dir().is_some_and(|d| d.join(rel).is_file()) }

/// A colour from the active pack for the current mode.
fn packed(f: impl Fn(&grids_core::theme_pack::Palette) -> Option<grids_core::theme_pack::Rgba>) -> Option<Hsla> {
    let p = PACK.get()?;
    let pal = if dark() { &p.dark } else { &p.light };
    f(pal).map(|c| rgbaf(c.r, c.g, c.b, c.a as f32 / 255.))
}
thread_local! {
    static ADAPTED: std::cell::RefCell<std::collections::HashMap<[u32; 4], Hsla>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

pub fn dark() -> bool { DARK.load(Ordering::Relaxed) }
/// Switch the look; true when it changed.
pub fn set_dark(on: bool) -> bool { DARK.swap(on, Ordering::Relaxed) != on }
/// The colour to draw for light-look colour `c`: itself, or in dark mode its dark counterpart.
pub fn adapt(c: Hsla) -> Hsla {
    if !dark() { return c }
    let key = [c.h.to_bits(), c.s.to_bits(), c.l.to_bits(), c.a.to_bits()];
    if let Some(d) = ADAPTED.with(|m| m.borrow().get(&key).copied()) { return d }
    let d = to_dark(c);
    ADAPTED.with(|m| m.borrow_mut().insert(key, d));
    d
}

/// Lightness mirrored in OKLab: white → L 0.23 (the page, #1c1c1c), the
/// text (L 0.25) → L 0.91, black → white. Chroma and hue are kept; vivid
/// colours (chroma ≥ 0.14) keep their own lightness within 0.62–0.86, pale
/// ones follow the mirror, with a smooth blend between.
fn to_dark(c: Hsla) -> Hsla {
    let Rgba { r, g, b, a } = Rgba::from(c);
    let [ll, oa, ob] = oklab(r, g, b);
    let chroma = (oa * oa + ob * ob).sqrt();
    let mirror = (1.137 - 0.907 * ll).clamp(0., 1.);
    let t = ((chroma - 0.04) / 0.10).clamp(0., 1.);
    let t = t * t * (3. - 2. * t);
    let l2 = mirror + (ll.clamp(0.62, 0.86) - mirror) * t;
    srgb(l2, oa, ob, a)
}

// sRGB ⇄ OKLab with Björn Ottosson's matrices, kept digit for digit.
#[allow(clippy::excessive_precision)]
fn oklab(r: f32, g: f32, b: f32) -> [f32; 3] {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s, 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s]
}
/// OKLab → linear sRGB (unclamped: out of [0, 1] means out of gamut).
#[allow(clippy::excessive_precision)]
fn lrgb(l: f32, oa: f32, ob: f32) -> [f32; 3] {
    let l_ = l + 0.3963377774 * oa + 0.2158037573 * ob;
    let m_ = l - 0.1055613458 * oa - 0.0638541728 * ob;
    let s_ = l - 0.0894841775 * oa - 1.2914855480 * ob;
    let (l_, m_, s_) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_, -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_, -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_]
}
fn srgb(l: f32, oa: f32, ob: f32, a: f32) -> Hsla {
    let [r, g, b] = lrgb(l, oa, ob);
    let enc = |v: f32| { let v = v.clamp(0., 1.); if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1. / 2.4) - 0.055 } };
    Rgba { r: enc(r), g: enc(g), b: enc(b), a }.into()
}

pub fn c(hex: u32) -> Hsla {
    adapt(rgb(hex).into())
}

/// A surface above the page (menus, dialogs, toasts, buttons, the picked
/// segment): `light` in the light look; in dark mode a step lighter than the
/// page, as raised surfaces are.
pub fn raised(light: Hsla) -> Hsla { if dark() { rgbaf(0x2c, 0x2c, 0x2e, 1.) } else { light } }

/// A literal colour, never adapted (shadows, scrims, glyphs over images).
pub fn rgbaf(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a }.into()
}

/// `rgbaf` for the page and chrome: adapted in dark mode.
pub fn ta(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    adapt(rgbaf(r, g, b, a))
}

/// `hex` at opacity `a`.
pub fn ca(hex: u32, a: f32) -> Hsla {
    ta((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, a)
}


/// The palette on the ramp names the code was written against
/// (`N*` neutrals, `B*` accent, `R*` / `O*` reds and oranges), so every
/// `craft::N300` still means "border", `N600` "secondary text" and so on.
/// Neutrals are measured greys; `B*` is built around the accent
/// (#3A85F7); `R*` / `O*` are Tailwind v4 ramps.
#[allow(dead_code)]
pub mod craft {
    pub const N00: u32 = 0xffffff;
    /// Code block fill.
    pub const N50: u32 = 0xfbfbfb;
    /// Popover / quick-entry body.
    pub const N100: u32 = 0xf7f7f7;
    /// Inactive chrome.
    pub const N200: u32 = 0xf1f2f3;
    /// Code block border, card hairlines (opaque on white).
    pub const N300: u32 = 0xeaebeb;
    /// Selected sidebar row, opaque.
    pub const N350: u32 = 0xdedfe0;
    pub const N400: u32 = 0xc9cacb;
    /// Quaternary text, inactive glyphs.
    pub const N500: u32 = 0x9c9fa0;
    /// Secondary text (dates, meta).
    pub const N600: u32 = 0x737477;
    pub const N650: u32 = 0x5d6064;
    /// Sidebar rows (text at 85% on the sidebar).
    pub const N700: u32 = 0x3f4143;
    /// Toolbar glyphs.
    pub const N800: u32 = 0x303335;
    /// Text (`.page` color).
    pub const N900: u32 = 0x1f2225;
    pub const N950: u32 = 0x141517;
    pub const N1000: u32 = 0x000000;
    pub const B50: u32 = 0xf0f5fe;
    pub const B100: u32 = 0xe3edfd;
    pub const B200: u32 = 0xc6dafc;
    pub const B300: u32 = 0x9cc0fa;
    pub const B350: u32 = 0x80aef9;
    pub const B400: u32 = 0x5d99f8;
    /// The accent: caret, primary button, menu highlight.
    pub const B500: u32 = 0x3a85f7;
    pub const B600: u32 = 0x2f74e0;
    pub const B700: u32 = 0x2660c0;
    pub const B800: u32 = 0x1d4b96;
    pub const B900: u32 = 0x14356a;
    pub const R50: u32 = 0xfef2f2;
    pub const R100: u32 = 0xffe2e2;
    pub const R200: u32 = 0xffc9c9;
    pub const R300: u32 = 0xffa2a2;
    pub const R350: u32 = 0xff6467;
    pub const R400: u32 = 0xfb2c36;
    pub const R500: u32 = 0xe7000b;
    pub const R600: u32 = 0xc10007;
    pub const R700: u32 = 0x9f0712;
    pub const R800: u32 = 0x82181a;
    pub const R900: u32 = 0x460809;
    pub const O50: u32 = 0xfff7ed;
    pub const O100: u32 = 0xffedd4;
    pub const O200: u32 = 0xffd6a8;
    pub const O300: u32 = 0xffb86a;
    pub const O350: u32 = 0xff8904;
    pub const O400: u32 = 0xff6900;
    pub const O500: u32 = 0xf54900;
    pub const O600: u32 = 0xca3500;
    pub const O700: u32 = 0x9f2d00;
    pub const O800: u32 = 0x7e2a0c;
    pub const O900: u32 = 0x441306;

    /// The secondary accents some controls use (dialog buttons, countdown,
    /// focus rings, success): one accent family and a green.
    pub mod old {
        pub const B50: u32 = 0xf0f5fe;
        pub const B100: u32 = 0xe3edfd;
        pub const B200: u32 = 0xc6dafc;
        pub const B300: u32 = 0x9cc0fa;
        pub const B400: u32 = 0x5d99f8;
        pub const B500: u32 = 0x3a85f7;
        pub const B600: u32 = 0x2f74e0;
        pub const B700: u32 = 0x2660c0;
        pub const G500: u32 = 0x00c950;
        pub const O500: u32 = 0xfe9a00;
    }

    /// Window surfaces (measured on the key window).
    /// The sunken surface: the sidebar material, here the whole window
    /// behind the note.
    pub const WINDOW: u32 = 0xf9fafb;
    /// The chrome between floating panels.
    pub const CHROME: u32 = 0xe6e7e8;
}

/// Shadows stay black; on the dark page they need more weight to show.
fn shadow(a: f32, y: f32, blur: f32, spread: f32) -> BoxShadow {
    let a = if dark() { (a * 3.).min(0.6) } else { a };
    BoxShadow { color: rgbaf(0, 0, 0, a), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(spread) }
}

/// Menus and dropdowns: macOS 26 menu panel, soft and wide.
pub fn shadow_menu() -> Vec<BoxShadow> {
    vec![shadow(0.05, 1., 3., 0.), shadow(0.10, 8., 24., 0.)]
}






/// The UI font. Chinese Windows draws its UI in Microsoft YaHei UI.
#[cfg(target_os = "windows")]
pub const UI_FONT: &str = "Microsoft YaHei UI";
#[cfg(target_os = "macos")]
pub const UI_FONT: &str = ".SystemUIFont";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const UI_FONT: &str = "Noto Sans CJK SC";

/// Monospace, for licence texts.
#[cfg(target_os = "windows")]
pub const MONO_FONT: &str = "Consolas";
#[cfg(target_os = "macos")]
pub const MONO_FONT: &str = "Menlo";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const MONO_FONT: &str = "DejaVu Sans Mono";

/// Interface text size (quiet-md's default 界面字号) and the smaller one.
pub const UI_SIZE: f32 = 13.;
pub const UI_SMALL: f32 = 12.;

pub struct Theme;

impl Theme {
    pub fn text() -> Hsla { packed(|p| p.text).unwrap_or_else(|| c(craft::N900)) }
    /// Secondary text: dates, meta, captions of controls.
    pub fn text_caption() -> Hsla { packed(|p| p.text_caption).unwrap_or_else(|| c(craft::N600)) }
    pub fn text_placeholder() -> Hsla { c(craft::N500) }
    pub fn text_disabled() -> Hsla { ca(craft::N900, 0.3) }
    pub fn primary() -> Hsla { packed(|p| p.accent).unwrap_or_else(|| c(craft::B500)) }
    /// Content planes: white.
    pub fn bg_body() -> Hsla { packed(|p| p.surface).unwrap_or_else(|| if dark() { rgb(0x151517).into() } else { c(craft::N00) }) }
    pub fn line_divider() -> Hsla { packed(|p| p.line).map(|c| c.opacity(0.7)).unwrap_or_else(|| ca(craft::N1000, 0.06)) }
    pub fn line_border() -> Hsla { packed(|p| p.line).unwrap_or_else(|| ca(craft::N1000, 0.08)) }
    pub fn fill_hover() -> Hsla { ca(craft::N1000, 0.045) }
    pub fn fill_hover_strong() -> Hsla { ca(craft::N1000, 0.06) }
    pub fn fill_pressed() -> Hsla { ca(craft::N1000, 0.09) }
    /// Selected rows: the accent, light.
    pub fn row_selected() -> Hsla { ca(craft::B500, 0.14) }
    /// Selected rows in a pane without focus.
    /// Text selection in fields.
    pub fn selection() -> Hsla { ca(craft::B500, 0.24) }
    pub fn caret() -> Hsla { c(craft::B500) }
    pub fn star() -> Hsla { c(0xf5a524) }
    /// The window surface: the sidebar material behind the content.
    pub fn window() -> Hsla { packed(|p| p.background).unwrap_or_else(|| if dark() { rgb(0x1b1b1c).into() } else { c(craft::WINDOW) }) }
    /// Sidebar section titles.
    pub fn sidebar_section() -> Hsla { c(0x333739) }
    pub fn sidebar_text() -> Hsla { ca(craft::N900, 0.85) }
    pub fn sidebar_hover() -> Hsla { packed(|p| p.hover).unwrap_or_else(|| ca(craft::N1000, 0.035)) }
    pub fn sidebar_selected() -> Hsla { packed(|p| p.selection).unwrap_or_else(|| ca(craft::N1000, 0.05)) }
    /// Monochrome icons.
    pub fn icon() -> Hsla { if dark() { ca(craft::N900, 0.6) } else { rgb(0x191919).into() } }
    pub fn icon_muted() -> Hsla { c(craft::N500) }
    /// Menus / popovers body.
    pub fn popover_bg() -> Hsla { c(0xfafafa) }
    /// Column headers of lists: tertiary text (#88898B measured in quiet-md).
    pub fn list_header() -> Hsla { c(0x88898b) }
    pub fn field_edge() -> Hsla { ca(craft::B500, 0.6) }
    pub fn split_active() -> Hsla { ca(craft::B500, 0.5) }
    pub fn drop_target() -> Hsla { ca(craft::B500, 0.10) }
    /// Destructive actions (the delete button): a firm red, softer and less
    /// saturated on the dark page.
    pub fn danger() -> Hsla { packed(|p| p.danger).unwrap_or_else(|| if dark() { rgb(0xc4574f).into() } else { rgb(0xd92d20).into() }) }
    pub fn danger_hover() -> Hsla { if dark() { rgb(0xb24c45).into() } else { rgb(0xc0261a).into() } }
    /// Compare: only on this side (green) / same name, differs (amber).
    pub fn added() -> Hsla { packed(|p| p.added).unwrap_or_else(|| if dark() { rgb(0x4fb477).into() } else { rgb(0x16a34a).into() }) }
    pub fn changed() -> Hsla { packed(|p| p.changed).unwrap_or_else(|| if dark() { rgb(0xd9a441).into() } else { rgb(0xd97706).into() }) }
    /// Link-group dot colours (blue, violet, teal, rose).
    pub fn link(i: usize) -> Hsla {
        let lt = [0x2f6fed, 0x8b5cf6, 0x0d9488, 0xe11d74];
        let dk = [0x5b8ff5, 0xa58bf7, 0x2bb3a6, 0xee5a97];
        rgb(if dark() { dk[i % 4] } else { lt[i % 4] }).into()
    }
    /// The keyboard focus ring on dialog buttons.
    pub fn focus_ring() -> Hsla { ca(craft::B500, 0.55) }
}

// ---- file type colours (fallback icons when the system gives none)

pub fn kind_color(k: grids_core::FileKind) -> Hsla {
    use grids_core::FileKind::*;
    match k {
        Folder => c(0xf2b33d),
        Image => c(0x2fa36b),
        Video => c(0xd9476b),
        Audio => c(0x9b59d0),
        Archive => c(0xb07a3b),
        Code => c(0x3a85f7),
        Text => c(craft::N600),
        Word => c(0x2b6cd8),
        Sheet => c(0x1f9d55),
        Slides => c(0xe0662f),
        Pdf => c(0xe04848),
        App => c(0x5d6064),
        Other => c(craft::N500),
    }
}

pub fn kind_icon(k: grids_core::FileKind) -> &'static str {
    use grids_core::FileKind::*;
    match k {
        Folder => "folder-fill",
        Image => "file-image",
        Video => "file-video",
        Audio => "file-audio",
        Archive => "file-archive",
        Code => "file-code",
        Text | Word | Pdf => "file-text",
        Sheet => "file-spreadsheet",
        Slides => "presentation",
        App => "app-window",
        Other => "file",
    }
}





