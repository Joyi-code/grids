//! "Today" view: files modified since a cut-off under a few roots.
//!
//! [`scan_recent`] is a plain blocking function meant for a background
//! thread; it checks `cancel` between directories. Depth is limited to
//! [`MAX_DEPTH`] levels below each root and at most [`MAX_FILES`] files are
//! looked at in total, so a huge tree can't stall it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

/// Levels below a root that are listed (root's own files are level 1).
pub const MAX_DEPTH: usize = 3;
/// Files examined in total before giving up (`truncated`).
pub const MAX_FILES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodayItem {
    pub path: PathBuf,
    /// The root it was found under.
    pub root: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TodayScan {
    /// Newest first.
    pub items: Vec<TodayItem>,
    /// The file cap was hit or the scan was cancelled.
    pub truncated: bool,
}

#[cfg(windows)]
fn hidden_or_system(_name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    meta.file_attributes() & (HIDDEN | SYSTEM) != 0
}

#[cfg(not(windows))]
fn hidden_or_system(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

/// Midnight today in local time is the UI's business; this takes any
/// cut-off. Roots inside other roots are scanned once.
pub fn scan_recent(roots: &[PathBuf], since: SystemTime, cancel: &AtomicBool) -> TodayScan {
    let mut uniq: Vec<&PathBuf> = Vec::new();
    for r in roots {
        if !uniq.iter().any(|u| crate::link::same_path(u, r)) {
            uniq.push(r);
        }
    }
    // drop roots nested in another root
    let roots: Vec<&PathBuf> = uniq
        .iter()
        .copied()
        .filter(|r| !uniq.iter().any(|o| !crate::link::same_path(o, r) && crate::link::is_within(r, o)))
        .collect();

    let mut out = TodayScan::default();
    let mut seen = HashSet::new();
    let mut examined = 0usize;
    'roots: for root in roots {
        let mut stack: Vec<(PathBuf, usize)> = vec![(root.clone(), 1)];
        while let Some((dir, depth)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                out.truncated = true;
                break 'roots;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                // DirEntry::metadata doesn't follow symlinks
                let Ok(meta) = entry.metadata() else { continue };
                let name = entry.file_name();
                if hidden_or_system(&name.to_string_lossy(), &meta) || meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    if depth < MAX_DEPTH {
                        stack.push((entry.path(), depth + 1));
                    }
                    continue;
                }
                if examined >= MAX_FILES {
                    out.truncated = true;
                    break 'roots;
                }
                examined += 1;
                let Ok(modified) = meta.modified() else { continue };
                if modified < since {
                    continue;
                }
                let path = entry.path();
                if seen.insert(path.clone()) {
                    out.items.push(TodayItem {
                        path,
                        root: root.clone(),
                        size: meta.len(),
                        modified,
                    });
                }
            }
        }
    }
    out.items.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::path::Path;
    use std::time::Duration;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grids-today-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path, age: Duration) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"hi").unwrap();
        File::options().write(true).open(p).unwrap().set_modified(SystemTime::now() - age).unwrap();
    }

    #[cfg(windows)]
    fn hide(p: &Path) {
        // attrib is a system tool, not a script; only used by this test
        let _ = std::process::Command::new("attrib").arg("+h").arg(p).status();
    }

    #[test]
    fn finds_recent_files_within_depth() {
        let d = scratch("depth");
        let hour = Duration::from_secs(3600);
        touch(&d.join("new.txt"), Duration::ZERO);
        touch(&d.join("old.txt"), hour * 48);
        touch(&d.join("a/b/lvl3.txt"), hour);
        touch(&d.join("a/b/c/lvl4.txt"), Duration::ZERO);
        touch(&d.join(".hidden/x.txt"), Duration::ZERO);
        touch(&d.join(".dot.txt"), Duration::ZERO);
        #[cfg(windows)]
        {
            hide(&d.join(".hidden"));
            hide(&d.join(".dot.txt"));
        }
        let since = SystemTime::now() - hour * 24;
        let scan = scan_recent(&[d.clone(), d.join("a"), d.clone()], since, &AtomicBool::new(false));
        let names: Vec<_> = scan.items.iter().map(|i| i.path.strip_prefix(&d).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names, vec!["new.txt", "a/b/lvl3.txt"]);
        assert!(scan.items.iter().all(|i| i.root == d && i.size == 2));
        assert!(!scan.truncated);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn cancel_marks_truncated() {
        let d = scratch("cancel");
        touch(&d.join("x.txt"), Duration::ZERO);
        let scan = scan_recent(std::slice::from_ref(&d), SystemTime::UNIX_EPOCH, &AtomicBool::new(true));
        assert!(scan.truncated);
        assert!(scan.items.is_empty());
        let scan = scan_recent(&[d.join("missing")], SystemTime::UNIX_EPOCH, &AtomicBool::new(false));
        assert_eq!(scan, TodayScan::default());
        let _ = fs::remove_dir_all(&d);
    }
}
