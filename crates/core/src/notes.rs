//! Folder notes: kept in grids-gpui.json under a normalised key, never in
//! the folder itself. Moves and renames made in Grids carry the notes along.

use std::collections::BTreeMap;
use std::path::Path;

/// Longest note, in characters.
pub const MAX_NOTE_CHARS: usize = 500;

/// The key of a folder's note: the path in lower case, `\` as separator on
/// Windows, no trailing separator (except a root such as `c:\` or `/`).
pub fn note_key(path: &Path) -> String {
    let mut s = path.to_string_lossy().to_lowercase();
    if cfg!(windows) {
        s = s.replace('/', "\\");
    }
    let sep = std::path::MAIN_SEPARATOR;
    while s.len() > 1 && s.ends_with(sep) && !(cfg!(windows) && s.len() == 3 && s.as_bytes().get(1) == Some(&b':')) {
        s.pop();
    }
    if cfg!(windows) && s.len() == 2 && s.as_bytes().get(1) == Some(&b':') {
        s.push('\\');
    }
    s
}

/// `text` trimmed and cut to [`MAX_NOTE_CHARS`] characters.
pub fn clamp_note(text: &str) -> String {
    text.trim().chars().take(MAX_NOTE_CHARS).collect()
}

/// The note for `folder`.
pub fn get<'a>(notes: &'a BTreeMap<String, String>, folder: &Path) -> Option<&'a str> {
    notes.get(&note_key(folder)).map(String::as_str)
}

/// Sets (or, for blank text, removes) the note for `folder`.
pub fn set(notes: &mut BTreeMap<String, String>, folder: &Path, text: &str) {
    let text = clamp_note(text);
    if text.is_empty() {
        notes.remove(&note_key(folder));
    } else {
        notes.insert(note_key(folder), text);
    }
}

/// After `from` was moved / renamed to `to`: re-keys its note and the notes
/// of every folder below it. Returns how many notes moved. A note already
/// at the destination key is replaced.
pub fn rewrite_moved(notes: &mut BTreeMap<String, String>, from: &Path, to: &Path) -> usize {
    rewrite_keys(notes, &note_key(from), &note_key(to))
}

fn rewrite_keys(notes: &mut BTreeMap<String, String>, from: &str, to: &str) -> usize {
    if from == to {
        return 0;
    }
    let sep = std::path::MAIN_SEPARATOR;
    let prefix = if from.ends_with(sep) { from.to_string() } else { format!("{from}{sep}") };
    let moving: Vec<String> = notes.keys().filter(|k| k.as_str() == from || k.starts_with(&prefix)).cloned().collect();
    for k in &moving {
        if let Some(v) = notes.remove(k) {
            let tail = &k[from.len()..];
            let tail = if to.ends_with(sep) { tail.trim_start_matches(sep) } else { tail };
            notes.insert(format!("{to}{tail}"), v);
        }
    }
    moving.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_text() {
        #[cfg(not(windows))]
        {
            assert_eq!(note_key(Path::new("/Home/Me/")), "/home/me");
            assert_eq!(note_key(Path::new("/")), "/");
        }
        #[cfg(windows)]
        {
            assert_eq!(note_key(Path::new(r"C:\Users\Me\")), r"c:\users\me");
            assert_eq!(note_key(Path::new("D:/A/b")), r"d:\a\b");
            assert_eq!(note_key(Path::new(r"E:\")), r"e:\");
            assert_eq!(note_key(Path::new("E:")), r"e:\");
        }
        assert_eq!(clamp_note(&"字".repeat(600)).chars().count(), MAX_NOTE_CHARS);
        assert_eq!(clamp_note("  hi \n"), "hi");
        let mut n = BTreeMap::new();
        let dir = std::env::temp_dir().join("Proj");
        set(&mut n, &dir, " 发布前别动 ");
        assert_eq!(get(&n, &std::env::temp_dir().join("proj")), Some("发布前别动"));
        set(&mut n, &dir, "   ");
        assert!(n.is_empty());
    }

    #[test]
    fn moves_carry_sub_notes() {
        let s = std::path::MAIN_SEPARATOR;
        let k = |p: &str| p.replace('/', &s.to_string());
        let mut n = BTreeMap::new();
        n.insert(k("/a/b"), "b".to_string());
        n.insert(k("/a/b/c"), "c".to_string());
        n.insert(k("/a/bc"), "other".to_string());
        n.insert(k("/z"), "z".to_string());
        assert_eq!(rewrite_keys(&mut n, &k("/a/b"), &k("/x/y")), 2);
        assert_eq!(n.get(&k("/x/y")).map(String::as_str), Some("b"));
        assert_eq!(n.get(&k("/x/y/c")).map(String::as_str), Some("c"));
        assert_eq!(n.get(&k("/a/bc")).map(String::as_str), Some("other"));
        assert!(!n.contains_key(&k("/a/b")));
        // into a root
        assert_eq!(rewrite_keys(&mut n, &k("/x/y"), &k("/")), 2);
        assert_eq!(n.get(&k("/c")).map(String::as_str), Some("c"));
        assert_eq!(n.get(&k("/")).map(String::as_str), Some("b"));
        assert_eq!(rewrite_keys(&mut n, &k("/nothing"), &k("/else")), 0);
    }
}
