//! What changed between two listings of the same folder (live highlight).

use crate::listing::Entry;
use std::collections::HashMap;

/// Names added or modified since the previous listing, in the new
/// listing's order. Removed entries are not tracked (nothing to highlight).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub added: Vec<String>,
    pub modified: Vec<String>,
}

impl Changes {
    pub fn len(&self) -> usize {
        self.added.len() + self.modified.len()
    }
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty()
    }
}

/// O(n): an entry is added when its name is new, modified when its kind,
/// size or modification time differ.
pub fn diff_entries(old: &[Entry], new: &[Entry]) -> Changes {
    let before: HashMap<&str, &Entry> = old.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut out = Changes::default();
    for e in new {
        match before.get(e.name.as_str()) {
            None => out.added.push(e.name.clone()),
            Some(o) if o.is_dir != e.is_dir || o.size != e.size || o.modified != e.modified => out.modified.push(e.name.clone()),
            Some(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn e(name: &str, size: u64, modified: i64) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from(name),
            is_dir: false,
            size,
            modified: Some(modified),
            hidden: false,
            ext: String::new(),
        }
    }

    #[test]
    fn added_and_modified() {
        let old = vec![e("a", 1, 10), e("b", 2, 20), e("gone", 3, 30), e("same", 4, 40)];
        let mut dir = e("b", 2, 20);
        dir.is_dir = true;
        let new = vec![e("same", 4, 40), e("a", 1, 11), e("new", 5, 50), e("c", 0, 0), dir];
        let c = diff_entries(&old, &new);
        assert_eq!(c.added, vec!["new".to_string(), "c".to_string()]);
        assert_eq!(c.modified, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(c.len(), 4);
        assert!(diff_entries(&new, &new).is_empty());
        assert_eq!(diff_entries(&[], &old).added.len(), 4);
        assert!(diff_entries(&old, &[]).is_empty());
        let mut bigger = e("same", 5, 40);
        bigger.size = 5;
        assert_eq!(diff_entries(&old, &[bigger]).modified, vec!["same".to_string()]);
    }
}
