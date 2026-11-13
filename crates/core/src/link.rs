//! Path comparison the way the file system sees it (case-insensitive on
//! Windows) and the relative paths behind linked panes: a pane linked to
//! another follows its navigation relative to each pane's base folder.

use std::path::{Component, Path, PathBuf};

/// Paths compare case-insensitively here (Windows / NTFS semantics).
pub const CASE_INSENSITIVE: bool = cfg!(windows);

/// The normalised components of `p`: `/` and `\` alike on Windows, `.`
/// dropped, trailing separators ignored, folded to lower case when `ci`.
fn parts(p: &Path, ci: bool) -> Vec<String> {
    let s = p.to_string_lossy();
    let s = if cfg!(windows) { s.replace('/', "\\") } else { s.into_owned() };
    Path::new(&s)
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| {
            let t = c.as_os_str().to_string_lossy();
            if ci { t.to_lowercase() } else { t.into_owned() }
        })
        .collect()
}

/// `a` and `b` name the same path (no file-system access).
pub fn same_path(a: &Path, b: &Path) -> bool {
    same_path_ci(a, b, CASE_INSENSITIVE)
}

pub(crate) fn same_path_ci(a: &Path, b: &Path, ci: bool) -> bool {
    parts(a, ci) == parts(b, ci)
}

/// `inner` is `outer` or somewhere below it (component-wise: `C:\ab` is not
/// inside `C:\a`).
pub fn is_within(inner: &Path, outer: &Path) -> bool {
    is_within_ci(inner, outer, CASE_INSENSITIVE)
}

pub(crate) fn is_within_ci(inner: &Path, outer: &Path, ci: bool) -> bool {
    relative_ci(outer, inner, ci).is_some()
}

/// `path` relative to `base` ("" when equal); `None` when `path` is not
/// inside `base`. The result keeps `path`'s own spelling.
pub fn relative(base: &Path, path: &Path) -> Option<PathBuf> {
    relative_ci(base, path, CASE_INSENSITIVE)
}

pub(crate) fn relative_ci(base: &Path, path: &Path, ci: bool) -> Option<PathBuf> {
    let b = parts(base, ci);
    let folded = parts(path, ci);
    if folded.len() < b.len() || folded.get(..b.len())? != b.as_slice() {
        return None;
    }
    let original = parts(path, false);
    Some(original.get(b.len()..)?.iter().collect())
}

/// Where a linked pane should go: `other_base` plus the path of `current`
/// relative to `my_base`. `None` when `current` has left `my_base` (the link
/// pauses until it comes back).
pub fn linked_target(my_base: &Path, current: &Path, other_base: &Path) -> Option<PathBuf> {
    linked_target_ci(my_base, current, other_base, CASE_INSENSITIVE)
}

pub(crate) fn linked_target_ci(my_base: &Path, current: &Path, other_base: &Path, ci: bool) -> Option<PathBuf> {
    let rel = relative_ci(my_base, current, ci)?;
    Some(if rel.as_os_str().is_empty() { other_base.to_path_buf() } else { other_base.join(rel) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_case_rules() {
        let p = Path::new;
        assert_eq!(relative_ci(p("/a/b"), p("/a/b/c/d"), false), Some(PathBuf::from("c/d")));
        assert_eq!(relative_ci(p("/a/b/"), p("/a/b"), false), Some(PathBuf::new()));
        assert_eq!(relative_ci(p("/a/b"), p("/a/bc"), false), None);
        assert_eq!(relative_ci(p("/a/b"), p("/a"), false), None);
        assert_eq!(relative_ci(p("/a/B"), p("/a/b/c"), false), None);
        // case-insensitive: the tail keeps its own spelling
        assert_eq!(relative_ci(p("/a/B"), p("/A/b/Cc"), true), Some(PathBuf::from("Cc")));
        assert!(same_path_ci(p("/x/Y/"), p("/X/./y"), true));
        assert!(!same_path_ci(p("/x/Y"), p("/x/y"), false));
        assert!(is_within_ci(p("/x/y/z"), p("/X"), true));
        assert!(!is_within_ci(p("/xy"), p("/x"), true));
    }

    #[test]
    fn linked_navigation() {
        let p = Path::new;
        assert_eq!(linked_target_ci(p("/l/proj"), p("/l/proj/src/ui"), p("/r/proj2"), false), Some(PathBuf::from("/r/proj2/src/ui")));
        assert_eq!(linked_target_ci(p("/l/proj"), p("/l/proj"), p("/r/proj2"), false), Some(PathBuf::from("/r/proj2")));
        assert_eq!(linked_target_ci(p("/l/proj"), p("/l"), p("/r/proj2"), false), None);
        assert_eq!(linked_target_ci(p("/l/Proj"), p("/L/proj/Src"), p("/r"), true), Some(PathBuf::from("/r/Src")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths() {
        let p = Path::new;
        assert_eq!(relative(p(r"C:\Users\Me"), p(r"c:/users/me/Docs\A")), Some(PathBuf::from(r"Docs\A")));
        assert!(same_path(p(r"C:\a\"), p(r"c:\A")));
        assert!(is_within(p(r"D:\x\y"), p(r"d:\")));
        assert!(!is_within(p(r"D:\x"), p(r"C:\")));
        assert_eq!(linked_target(p(r"C:\a"), p(r"C:\A\b"), p(r"E:\z")), Some(PathBuf::from(r"E:\z\b")));
    }
}
