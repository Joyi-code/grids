//! Settings and sessions, saved as one JSON file next to the executable
//! (`grids-gpui.json`, portable). Unknown or missing fields fall back to
//! defaults so older and newer files both load.

use crate::format::Lang;
use crate::layout::Layout;
use crate::pane::SortKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "grids-gpui.json";

/// Column widths of a pane's list, in px (the name column takes the rest).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    pub name: f32,
    pub modified: f32,
    pub kind: f32,
    pub size: f32,
}

impl Default for Columns {
    fn default() -> Self { Self { name: 240., modified: 132., kind: 132., size: 84. } }
}

impl Columns {
    pub const MIN: f32 = 40.;
    pub fn clamped(mut self) -> Self {
        for w in [&mut self.name, &mut self.modified, &mut self.kind, &mut self.size] {
            if !w.is_finite() { *w = 100. }
            *w = w.clamp(Self::MIN, 1200.);
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneSave {
    pub path: PathBuf,
    pub sort: SortKey,
    pub ascending: bool,
    pub show_hidden: bool,
    pub columns: Columns,
    pub view: ViewMode,
}

impl Default for PaneSave {
    fn default() -> Self {
        Self { path: PathBuf::new(), sort: SortKey::Name, ascending: true, show_hidden: false, columns: Columns::default(), view: ViewMode::Details }
    }
}

/// How a pane shows its folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    Details,
    List,
    Medium,
    Large,
    Huge,
}

impl ViewMode {
    /// In menu / Ctrl+Shift+1..5 order.
    pub const ALL: [ViewMode; 5] = [ViewMode::Huge, ViewMode::Large, ViewMode::Medium, ViewMode::List, ViewMode::Details];

    /// The thumbnail edge in logical px for the icon views.
    pub fn icon_px(self) -> Option<f32> {
        match self {
            ViewMode::Medium => Some(48.),
            ViewMode::Large => Some(96.),
            ViewMode::Huge => Some(192.),
            _ => None,
        }
    }

    /// One step bigger (`up`) or smaller, for Ctrl+wheel; stops at the ends.
    pub fn step(self, up: bool) -> ViewMode {
        let i = Self::ALL.iter().position(|&v| v == self).unwrap_or(4);
        let j = if up { i.saturating_sub(1) } else { (i + 1).min(4) };
        Self::ALL[j]
    }
}

/// At most this many link groups per session.
pub const MAX_LINK_GROUPS: usize = 4;

/// One pane in a link group, with the folder it was in when linked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LinkMember {
    /// Index into [`Session::panes`].
    pub pane: usize,
    pub base: PathBuf,
}

/// Panes that navigate together (relative to each one's `base`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LinkGroup {
    pub members: Vec<LinkMember>,
    /// Compare mode (only meaningful with exactly two members).
    pub compare: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    pub layout: Layout,
    pub panes: Vec<PaneSave>,
    pub active: usize,
    /// Link groups; group `i` has colour `i`. A pane is in at most one.
    pub links: Vec<LinkGroup>,
}

impl Session {
    /// Panes for every cell of the layout: missing ones start at `home`.
    pub fn normalized(mut self, home: &Path) -> Self {
        self.layout = self.layout.normalized();
        let n = self.layout.count();
        self.panes.truncate(n);
        while self.panes.len() < n {
            self.panes.push(PaneSave { path: home.to_path_buf(), ..Default::default() });
        }
        if let Some(rows) = self.layout.legacy_rows.take() {
            // An old rows × cols session numbered its panes row by row; now
            // they go down each column. Keep every folder in its cell.
            let cols = n / rows;
            let old = std::mem::take(&mut self.panes);
            let mut active = 0;
            for c in 0..cols {
                for r in 0..rows {
                    let j = r * cols + c;
                    if j == self.active { active = self.panes.len() }
                    self.panes.push(old[j].clone());
                }
            }
            self.active = active;
        }
        for p in &mut self.panes {
            if p.path.as_os_str().is_empty() { p.path = home.to_path_buf(); }
            p.columns = p.columns.clone().clamped();
        }
        if self.active >= n { self.active = 0; }
        self.links = normalized_links(std::mem::take(&mut self.links), n);
        self
    }
}

/// Drops members whose pane is gone, a pane's later memberships, groups left
/// with fewer than two panes, compare on groups not of two, and groups past
/// [`MAX_LINK_GROUPS`].
fn normalized_links(groups: Vec<LinkGroup>, panes: usize) -> Vec<LinkGroup> {
    let mut used = vec![false; panes];
    let mut out = Vec::new();
    for mut g in groups {
        g.members.retain(|m| match used.get_mut(m.pane) {
            Some(u) if !*u && !m.base.as_os_str().is_empty() => { *u = true; true }
            _ => false,
        });
        if g.members.len() < 2 {
            for m in &g.members { used[m.pane] = false; }
            continue;
        }
        if out.len() == MAX_LINK_GROUPS {
            for m in &g.members { used[m.pane] = false; }
            continue;
        }
        g.compare &= g.members.len() == 2;
        out.push(g);
    }
    out
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedSession {
    pub name: String,
    pub session: Session,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub lang: Lang,
    pub appearance: Appearance,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    pub favourites: Vec<PathBuf>,
    /// What the window shows; saved on every change.
    pub current: Session,
    /// Sessions saved by name.
    pub sessions: Vec<NamedSession>,
    /// The saved session last loaded or saved, if any.
    pub session_name: Option<String>,
    /// Window size in logical px.
    pub window: Option<(f32, f32)>,
    /// Locked folders (see [`crate::locked`]).
    pub locked: Vec<PathBuf>,
    /// The shelf: files collected for a later copy / move.
    pub shelf: Vec<PathBuf>,
    /// Folder notes keyed by [`crate::notes::note_key`].
    pub notes: BTreeMap<String, String>,
    /// Visited folders for frecency (see [`crate::frecency`]).
    pub recent: Vec<crate::frecency::Recent>,
}

impl Default for Config {
    fn default() -> Self {
        Self { version: 1, lang: Lang::Zh, appearance: Appearance::System, sidebar_open: true, sidebar_width: 220., favourites: vec![], current: Session::default(), sessions: vec![], session_name: None, window: None, locked: vec![], shelf: vec![], notes: BTreeMap::new(), recent: vec![] }
    }
}

impl Config {
    /// The file next to the executable; `GRIDS_CONFIG` overrides it.
    pub fn default_path() -> PathBuf {
        if let Some(p) = std::env::var_os("GRIDS_CONFIG") { return PathBuf::from(p) }
        std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(FILE_NAME))).unwrap_or_else(|| PathBuf::from(FILE_NAME))
    }

    /// Reads `path`; a missing or unreadable file gives the defaults.
    pub fn load(path: &Path) -> Config {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Writes atomically (temp file, then rename).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    pub fn is_favourite(&self, p: &Path) -> bool { self.favourites.iter().any(|f| f == p) }

    /// Adds or removes `p`; true when it is now a favourite.
    pub fn toggle_favourite(&mut self, p: &Path) -> bool {
        if let Some(i) = self.favourites.iter().position(|f| f == p) {
            self.favourites.remove(i);
            false
        } else {
            self.favourites.push(p.to_path_buf());
            true
        }
    }

    /// Saves the current session under `name` (replacing one of that name).
    pub fn save_session(&mut self, name: &str) {
        let s = NamedSession { name: name.to_string(), session: self.current.clone() };
        match self.sessions.iter_mut().find(|n| n.name == name) {
            Some(n) => *n = s,
            None => self.sessions.push(s),
        }
        self.session_name = Some(name.to_string());
    }

    pub fn delete_session(&mut self, name: &str) {
        self.sessions.retain(|n| n.name != name);
        if self.session_name.as_deref() == Some(name) { self.session_name = None; }
    }

    /// A name not yet used: "会话 1", "会话 2" …
    pub fn fresh_session_name(&self, base: &str) -> String {
        (1..).map(|n| format!("{base} {n}")).find(|n| !self.sessions.iter().any(|s| &s.name == n)).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("grids-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let mut c = Config::default();
        c.current = Session { layout: Layout::new(3, 3), panes: vec![], active: 4, ..Default::default() }.normalized(Path::new("/home"));
        c.current.panes[2].sort = SortKey::Size;
        c.current.panes[2].columns.name = 300.;
        c.toggle_favourite(Path::new("/tmp"));
        c.save_session("工作");
        c.save(&path).unwrap();
        let back = Config::load(&path);
        assert_eq!(back, c);
        assert_eq!(back.current.panes.len(), 9);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tolerant_load() {
        let c: Config = serde_json::from_str(r#"{"sidebar_open": false, "future_field": 1, "current": {"layout": {"rows": 2, "cols": 9, "row_weights": [], "col_weights": []}}}"#).unwrap();
        assert!(!c.sidebar_open);
        let s = c.current.normalized(Path::new("/h"));
        assert_eq!((s.layout.cols.clone(), s.panes.len()), (vec![2; 4], 8));
        assert_eq!(s.panes[7].path, PathBuf::from("/h"));
        assert_eq!(Config::load(Path::new("/nonexistent/x.json")), Config::default());
    }

    #[test]
    fn v09_fields_default_and_links_normalize() {
        let c: Config = serde_json::from_str(r#"{"favourites": ["/f"]}"#).unwrap();
        assert!(c.locked.is_empty() && c.shelf.is_empty() && c.notes.is_empty() && c.recent.is_empty());
        assert!(c.current.links.is_empty());
        let m = |pane: usize, base: &str| LinkMember { pane, base: base.into() };
        let g = |members: Vec<LinkMember>, compare: bool| LinkGroup { members, compare };
        let mut s = Session { layout: Layout::default(), ..Default::default() };
        let n = s.layout.count();
        assert!(n >= 2);
        s.links = vec![
            g(vec![m(0, "/a"), m(1, "/b")], true),
            g(vec![m(1, "/x"), m(99, "/y")], false), // pane 1 taken, 99 gone: dropped
            g(vec![m(0, "/a")], false),
        ];
        let s = s.normalized(Path::new("/h"));
        assert_eq!(s.links, vec![g(vec![m(0, "/a"), m(1, "/b")], true)]);
        let mut c = Config::default();
        c.locked.push("/l".into());
        c.notes.insert("/n".into(), "hi".into());
        c.recent.push(crate::frecency::Recent { path: "/r".into(), count: 2, last: 5 });
        c.current = s;
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn migrates_row_major_panes() {
        let c: Config = serde_json::from_str(r#"{"current": {"layout": {"rows": 2, "cols": 3, "row_weights": [], "col_weights": []},
            "panes": [{"path": "/a"}, {"path": "/b"}, {"path": "/c"}, {"path": "/d"}, {"path": "/e"}, {"path": "/f"}], "active": 1}}"#).unwrap();
        let s = c.current.normalized(Path::new("/h"));
        let paths: Vec<_> = s.panes.iter().map(|p| p.path.to_string_lossy().into_owned()).collect();
        // a b c / d e f → columns a d, b e, c f
        assert_eq!(paths, ["/a", "/d", "/b", "/e", "/c", "/f"]);
        assert_eq!(s.active, 2);
        assert_eq!(s.clone().normalized(Path::new("/h")), s);
    }

    #[test]
    fn favourites_and_sessions() {
        let mut c = Config::default();
        assert!(c.toggle_favourite(Path::new("/a")));
        assert!(c.is_favourite(Path::new("/a")));
        assert!(!c.toggle_favourite(Path::new("/a")));
        c.save_session("A");
        c.save_session("A");
        assert_eq!(c.sessions.len(), 1);
        assert_eq!(c.fresh_session_name("会话"), "会话 1");
        c.delete_session("A");
        assert!(c.sessions.is_empty() && c.session_name.is_none());
    }
}
