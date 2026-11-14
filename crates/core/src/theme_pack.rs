//! Theme packs: the palette in a `theme.json` and the folder it lives in.
//! Reading files is the app's job; everything here is pure and tested.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A colour as written in `theme.json`: `#RRGGBB` or `#RRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba { pub r: u8, pub g: u8, pub b: u8, pub a: u8 }

/// One mode's colours. A missing key keeps the built-in colour.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Palette {
    pub background: Option<Rgba>,
    pub surface: Option<Rgba>,
    pub text: Option<Rgba>,
    pub text_caption: Option<Rgba>,
    pub line: Option<Rgba>,
    pub accent: Option<Rgba>,
    pub selection: Option<Rgba>,
    pub hover: Option<Rgba>,
    pub added: Option<Rgba>,
    pub changed: Option<Rgba>,
    pub danger: Option<Rgba>,
}

#[derive(Clone, Debug, Default)]
pub struct ThemePack {
    pub name: String,
    pub author: String,
    pub version: String,
    pub light: Palette,
    pub dark: Palette,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawFile { name: String, author: String, version: String, light: RawPalette, dark: RawPalette }

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawPalette {
    background: Option<String>, surface: Option<String>, text: Option<String>, text_caption: Option<String>,
    line: Option<String>, accent: Option<String>, selection: Option<String>, hover: Option<String>,
    added: Option<String>, changed: Option<String>, danger: Option<String>,
}

impl RawPalette {
    fn palette(self) -> Palette {
        Palette {
            background: self.background.as_deref().and_then(parse_color),
            surface: self.surface.as_deref().and_then(parse_color),
            text: self.text.as_deref().and_then(parse_color),
            text_caption: self.text_caption.as_deref().and_then(parse_color),
            line: self.line.as_deref().and_then(parse_color),
            accent: self.accent.as_deref().and_then(parse_color),
            selection: self.selection.as_deref().and_then(parse_color),
            hover: self.hover.as_deref().and_then(parse_color),
            added: self.added.as_deref().and_then(parse_color),
            changed: self.changed.as_deref().and_then(parse_color),
            danger: self.danger.as_deref().and_then(parse_color),
        }
    }
}

/// `#RRGGBB` or `#RRGGBBAA`, the `#` optional.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let h = s.trim().trim_start_matches('#');
    if !matches!(h.len(), 6 | 8) || !h.bytes().all(|b| b.is_ascii_hexdigit()) { return None }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(Rgba { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: if h.len() == 8 { byte(6)? } else { 255 } })
}

/// The pack in `theme.json` text. `Err` names what is wrong, for a toast.
pub fn parse(json: &str) -> Result<ThemePack, String> {
    let raw: RawFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if raw.light.background.is_none() && raw.light.surface.is_none() && raw.dark.background.is_none() {
        return Err("theme.json has no colours".into())
    }
    Ok(ThemePack { name: raw.name, author: raw.author, version: raw.version, light: raw.light.palette(), dark: raw.dark.palette() })
}

/// Reads `<dir>/theme.json`. The folder is returned as given, for the assets.
pub fn load(dir: &Path) -> Result<ThemePack, String> {
    let path: PathBuf = dir.join("theme.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text)
}

/// The theme folders inside `dir`: each subfolder with a readable `theme.json`,
/// as (name, folder), sorted by name. A pack without a name uses its folder name.
pub fn list(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() { continue }
        let Ok(pack) = load(&folder) else { continue };
        let name = if pack.name.trim().is_empty() {
            folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            pack.name
        };
        out.push((name, folder));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACK: &str = r##"{
      "name": "奶油猫咪工作室", "author": "x", "version": "1.0.0",
      "light": { "background": "#F7F1E7", "surface": "#FFFCF6", "text": "#44352D", "line": "#44352D24", "accent": "#A34F26" },
      "dark": { "background": "#241F1C", "surface": "#302823" }
    }"##;

    #[test]
    fn reads_colours() {
        let p = parse(PACK).unwrap();
        assert_eq!(p.name, "奶油猫咪工作室");
        assert_eq!(p.light.background, Some(Rgba { r: 0xF7, g: 0xF1, b: 0xE7, a: 0xFF }));
        assert_eq!(p.light.line, Some(Rgba { r: 0x44, g: 0x35, b: 0x2D, a: 0x24 }));
        assert_eq!(p.dark.background, Some(Rgba { r: 0x24, g: 0x1F, b: 0x1C, a: 0xFF }));
        assert!(p.dark.text.is_none());
    }

    #[test]
    fn rejects_rubbish() {
        assert!(parse("not json").is_err());
        assert!(parse("{\"light\":{}}").is_err());
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("zzzzzz"), None);
        assert_eq!(parse_color("F7F1E7").map(|c| c.a), Some(255));
    }
}
