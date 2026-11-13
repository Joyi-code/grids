//! Path helpers: breadcrumbs, typed paths, display names, fresh names.

use std::path::{Component, Path, PathBuf};

/// The breadcrumb of `path`: (label, path up to it) per segment, the root
/// first ("C:" for `C:\`, "/" on Unix).
pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut acc = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => {
                acc.push(p.as_os_str());
                out.push((p.as_os_str().to_string_lossy().trim_end_matches('\\').to_string(), acc.clone()));
            }
            Component::RootDir => {
                acc.push(c.as_os_str());
                match out.last_mut() {
                    Some(last) => last.1 = acc.clone(),
                    None => out.push(("/".into(), acc.clone())),
                }
            }
            Component::Normal(s) => {
                acc.push(s);
                out.push((s.to_string_lossy().into_owned(), acc.clone()));
            }
            Component::CurDir | Component::ParentDir => acc.push(c.as_os_str()),
        }
    }
    out
}

/// The folder's own name, or the drive / root for a root ("C:", "/").
pub fn display_name(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => breadcrumbs(path).last().map(|b| b.0.clone()).unwrap_or_else(|| path.to_string_lossy().into_owned()),
    }
}

/// A path typed in the address bar: quotes trimmed, `~` expanded, relative
/// paths taken from `base`, `C:` read as `C:\`.
pub fn parse_typed(text: &str, base: &Path, home: Option<&Path>) -> PathBuf {
    let t = text.trim().trim_matches('"').trim();
    let mut p = if t == "~" || t.starts_with("~/") || t.starts_with("~\\") {
        match home {
            Some(h) => h.join(t[1..].trim_start_matches(['/', '\\'])),
            None => PathBuf::from(t),
        }
    } else {
        PathBuf::from(t)
    };
    if t.len() == 2 && t.as_bytes()[1] == b':' && t.as_bytes()[0].is_ascii_alphabetic() {
        p = PathBuf::from(format!("{t}\\"));
    }
    if p.is_relative() && !(t.len() >= 2 && t.as_bytes()[1] == b':') { base.join(p) } else { p }
}

/// `base`, or "base (2)", "base (3)" … : the first name not yet in `dir`.
/// An extension stays at the end ("新建文本文档 (2).txt").
pub fn unique_name(dir: &Path, base: &str) -> String {
    if !dir.join(base).exists() { return base.to_string() }
    let (stem, ext) = match base.rfind('.') {
        Some(i) if i > 0 => (&base[..i], &base[i..]),
        _ => (base, ""),
    };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|n| !dir.join(n).exists()).unwrap()
}

/// Whether `name` is usable as a file name (no separators or reserved characters).
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n != "." && n != ".." && !n.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crumbs_unix() {
        let b = breadcrumbs(Path::new("/home/user/文档"));
        let labels: Vec<_> = b.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(labels, ["/", "home", "user", "文档"]);
        assert_eq!(b[1].1, PathBuf::from("/home"));
        assert_eq!(display_name(Path::new("/")), "/");
    }
    #[test]
    fn typed() {
        let base = Path::new("/base");
        assert_eq!(parse_typed(" \"/tmp\" ", base, None), PathBuf::from("/tmp"));
        assert_eq!(parse_typed("sub", base, None), PathBuf::from("/base/sub"));
        assert_eq!(parse_typed("~/x", base, Some(Path::new("/h"))), PathBuf::from("/h/x"));
    }
    #[test]
    fn fresh_names() {
        let dir = std::env::temp_dir().join(format!("grids-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("新建文件夹")).unwrap();
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        assert_eq!(unique_name(&dir, "新建文件夹"), "新建文件夹 (2)");
        assert_eq!(unique_name(&dir, "a.txt"), "a (2).txt");
        assert_eq!(unique_name(&dir, "b"), "b");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(valid_name("报告.docx") && !valid_name("a/b") && !valid_name(" "));
    }
}
