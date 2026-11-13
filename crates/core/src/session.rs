//! Settings and sessions, saved as one JSON file next to the executable
//! (`grids-gpui.json`, portable). Unknown or missing fields fall back to
//! defaults so older and newer files both load.

use crate::format::Lang;
use crate::layout::Layout;
use crate::pane::SortKey;
use serde::{Deserialize, Serialize};
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
}

impl Default for PaneSave {
    fn default() -> Self { Self { path: PathBuf::new(), sort: SortKey::Name, ascending: true, show_hidden: false, columns: Columns::default() } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Session {
    pub layout: Layout,
    pub panes: Vec<PaneSave>,
    pub active: usize,
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
        for p in &mut self.panes {
            if p.path.as_os_str().is_empty() { p.path = home.to_path_buf(); }
            p.columns = p.columns.clone().clamped();
        }
        if self.active >= n { self.active = 0; }
        self
    }
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
}

impl Default for Config {
    fn default() -> Self {
        Self { version: 1, lang: Lang::Zh, appearance: Appearance::System, sidebar_open: true, sidebar_width: 220., favourites: vec![], current: Session::default(), sessions: vec![], session_name: None, window: None }
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
        c.current = Session { layout: Layout::new(3, 3), panes: vec![], active: 4 }.normalized(Path::new("/home"));
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
        assert_eq!((s.layout.rows, s.layout.cols, s.panes.len()), (2, 4, 8));
        assert_eq!(s.panes[7].path, PathBuf::from("/h"));
        assert_eq!(Config::load(Path::new("/nonexistent/x.json")), Config::default());
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
