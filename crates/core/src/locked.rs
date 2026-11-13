//! Read-only folders: a locked folder protects itself and everything below
//! it, whichever pane shows it. Only enforced inside Grids.

use crate::link::{CASE_INSENSITIVE, is_within_ci, same_path_ci};
use std::path::{Path, PathBuf};

/// The locked folder that covers `path` (itself or the nearest ancestor in
/// `locked`), if any.
pub fn locked_root<'a>(locked: &'a [PathBuf], path: &Path) -> Option<&'a PathBuf> {
    locked_root_ci(locked, path, CASE_INSENSITIVE)
}

fn locked_root_ci<'a>(locked: &'a [PathBuf], path: &Path, ci: bool) -> Option<&'a PathBuf> {
    // the deepest match: unlocking it is what the "解锁" button should offer
    locked.iter().filter(|l| is_within_ci(path, l, ci)).max_by_key(|l| l.components().count())
}

/// `path` is in a read-only area.
pub fn is_locked(locked: &[PathBuf], path: &Path) -> bool {
    locked_root(locked, path).is_some()
}

/// Any of `paths` is in a read-only area (e.g. the sources of a move).
pub fn any_locked(locked: &[PathBuf], paths: &[PathBuf]) -> bool {
    paths.iter().any(|p| is_locked(locked, p))
}

/// Locks `folder` when it is not exactly locked yet, else unlocks it.
/// Returns the new state (`true` = locked).
pub fn toggle(locked: &mut Vec<PathBuf>, folder: &Path) -> bool {
    toggle_ci(locked, folder, CASE_INSENSITIVE)
}

fn toggle_ci(locked: &mut Vec<PathBuf>, folder: &Path, ci: bool) -> bool {
    let before = locked.len();
    locked.retain(|l| !same_path_ci(l, folder, ci));
    if locked.len() != before {
        return false;
    }
    locked.push(folder.to_path_buf());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_children_only() {
        let locked = vec![PathBuf::from("/data/photos"), PathBuf::from("/data/photos/2024")];
        let p = Path::new;
        assert_eq!(locked_root_ci(&locked, p("/data/photos/2024/x.jpg"), false), Some(&locked[1]));
        assert_eq!(locked_root_ci(&locked, p("/data/photos/a.jpg"), false), Some(&locked[0]));
        assert_eq!(locked_root_ci(&locked, p("/data/photos"), false), Some(&locked[0]));
        assert_eq!(locked_root_ci(&locked, p("/data/photos2"), false), None);
        assert_eq!(locked_root_ci(&locked, p("/data"), false), None);
        assert_eq!(locked_root_ci(&locked, p("/DATA/Photos/a"), false), None);
        assert_eq!(locked_root_ci(&locked, p("/DATA/Photos/a"), true), Some(&locked[0]));
    }

    #[test]
    fn toggling() {
        let mut l = Vec::new();
        assert!(toggle_ci(&mut l, Path::new("/a/B"), true));
        assert!(!toggle_ci(&mut l, Path::new("/A/b/"), true));
        assert!(l.is_empty());
        assert!(toggle_ci(&mut l, Path::new("/a"), false));
        assert!(any_locked(&l, &[PathBuf::from("/x"), PathBuf::from("/a/y")]));
        assert!(!any_locked(&l, &[PathBuf::from("/x")]));
    }
}
