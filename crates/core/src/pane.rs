//! One pane's state: where it is, where it has been, how it sorts and
//! filters, and what is selected. Loading happens elsewhere; this only
//! takes the entries once read.

use crate::listing::Entry;
use crate::natord;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Name,
    Modified,
    Type,
    Size,
}

/// How many folders the back history keeps.
const HISTORY: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct PaneState {
    pub path: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    pub sort: SortKey,
    pub ascending: bool,
    pub show_hidden: bool,
    /// Name filter (case-insensitive substring); empty shows all.
    pub filter: String,
    entries: Vec<Entry>,
    /// Rows shown: indices into `entries`, sorted and filtered.
    view: Vec<usize>,
    /// Selected entries (indices into `entries`).
    selected: HashSet<usize>,
    /// The row Shift extends from.
    anchor: Option<usize>,
    /// The keyboard cursor (a row).
    pub cursor: Option<usize>,
    /// Why the folder could not be read.
    pub error: Option<String>,
    /// Bumped whenever the rows change, so views can tell.
    pub generation: u64,
}

impl PaneState {
    pub fn new(path: PathBuf) -> Self {
        Self { path, ascending: true, ..Default::default() }
    }

    // ---- navigation

    /// Goes to `path` (recorded in history). False when already there.
    pub fn navigate(&mut self, path: PathBuf) -> bool {
        if path == self.path { return false }
        self.back.push(std::mem::replace(&mut self.path, path));
        if self.back.len() > HISTORY { self.back.remove(0); }
        self.forward.clear();
        self.reset_view();
        true
    }

    pub fn can_back(&self) -> bool { !self.back.is_empty() }
    pub fn can_forward(&self) -> bool { !self.forward.is_empty() }
    pub fn can_up(&self) -> bool { self.path.parent().is_some() }

    pub fn back(&mut self) -> bool {
        let Some(p) = self.back.pop() else { return false };
        self.forward.push(std::mem::replace(&mut self.path, p));
        self.reset_view();
        true
    }

    pub fn forward(&mut self) -> bool {
        let Some(p) = self.forward.pop() else { return false };
        self.back.push(std::mem::replace(&mut self.path, p));
        self.reset_view();
        true
    }

    /// Goes to the parent; the folder left is selected once the parent loads.
    /// Returns the name to select.
    pub fn up(&mut self) -> Option<String> {
        let parent = self.path.parent()?.to_path_buf();
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.navigate(parent);
        name
    }

    fn reset_view(&mut self) {
        self.entries.clear();
        self.view.clear();
        self.selected.clear();
        self.anchor = None;
        self.cursor = None;
        self.filter.clear();
        self.error = None;
        self.generation += 1;
    }

    // ---- entries

    /// Takes a fresh listing of the current folder. Selection, cursor and
    /// anchor follow their entries by name; `select` (a name) replaces the
    /// selection when given.
    pub fn set_entries(&mut self, entries: Vec<Entry>, select: Option<&str>) {
        let keep: HashSet<String> = self.selected.iter().filter_map(|&i| self.entries.get(i).map(|e| e.name.clone())).collect();
        let cursor_name = self.cursor_entry().map(|e| e.name.clone());
        let anchor_name = self.anchor.and_then(|r| self.view.get(r)).and_then(|&i| self.entries.get(i)).map(|e| e.name.clone());
        self.entries = entries;
        self.error = None;
        self.rebuild();
        self.selected = self.entries.iter().enumerate().filter(|(_, e)| keep.contains(&e.name)).map(|(i, _)| i).collect();
        self.cursor = cursor_name.and_then(|n| self.row_of(&n));
        self.anchor = anchor_name.and_then(|n| self.row_of(&n));
        if let Some(name) = select {
            self.select_name(name);
        }
    }

    pub fn set_error(&mut self, msg: String) {
        self.entries.clear();
        self.view.clear();
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.error = Some(msg);
        self.generation += 1;
    }

    /// Selects the entry named `name` alone and puts the cursor on it.
    pub fn select_name(&mut self, name: &str) -> bool {
        let Some(row) = self.row_of(name) else { return false };
        self.selected.clear();
        self.selected.insert(self.view[row]);
        self.cursor = Some(row);
        self.anchor = Some(row);
        true
    }

    /// Selects the entries named in `names` (after a paste).
    pub fn select_names(&mut self, names: &[String]) {
        let set: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
        self.selected = self.view.iter().copied().filter(|&i| set.contains(self.entries[i].name.as_str())).collect();
        self.cursor = self.view.iter().position(|&i| self.selected.contains(&i));
        self.anchor = self.cursor;
    }

    fn row_of(&self, name: &str) -> Option<usize> {
        self.view.iter().position(|&i| self.entries[i].name == name)
            .or_else(|| self.view.iter().position(|&i| self.entries[i].name.eq_ignore_ascii_case(name)))
    }

    /// Recomputes the shown rows after a sort, filter or hidden change.
    fn rebuild(&mut self) {
        let f = self.filter.to_lowercase();
        let mut view: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                (self.show_hidden || !e.hidden) && (f.is_empty() || e.name.to_lowercase().contains(&f))
            })
            .collect();
        let (es, key, asc) = (&self.entries, self.sort, self.ascending);
        view.sort_by(|&a, &b| compare(&es[a], &es[b], key, asc));
        self.view = view;
        let shown: HashSet<usize> = self.view.iter().copied().collect();
        self.selected.retain(|i| shown.contains(i));
        self.generation += 1;
    }

    /// Re-sorts and refilters, keeping selection and cursor on their entries.
    fn rebuild_keeping(&mut self) {
        let cursor = self.cursor.and_then(|r| self.view.get(r).copied());
        let anchor = self.anchor.and_then(|r| self.view.get(r).copied());
        self.rebuild();
        self.cursor = cursor.and_then(|i| self.view.iter().position(|&j| j == i));
        self.anchor = anchor.and_then(|i| self.view.iter().position(|&j| j == i));
    }

    /// A click on a column header: the same column flips direction, another
    /// sorts by it (names ascending, dates and sizes descending first).
    pub fn sort_by(&mut self, key: SortKey) {
        if self.sort == key {
            self.ascending = !self.ascending;
        } else {
            self.sort = key;
            self.ascending = matches!(key, SortKey::Name | SortKey::Type);
        }
        self.rebuild_keeping();
    }

    pub fn set_sort(&mut self, key: SortKey, ascending: bool) {
        self.sort = key;
        self.ascending = ascending;
        self.rebuild_keeping();
    }

    pub fn set_filter(&mut self, filter: &str) {
        if self.filter == filter { return }
        self.filter = filter.to_string();
        self.rebuild_keeping();
        if self.cursor.is_none() && !self.view.is_empty() { self.cursor = Some(0); }
    }

    pub fn set_show_hidden(&mut self, on: bool) {
        if self.show_hidden == on { return }
        self.show_hidden = on;
        self.rebuild_keeping();
    }

    // ---- rows

    pub fn len(&self) -> usize { self.view.len() }
    pub fn is_empty(&self) -> bool { self.view.is_empty() }
    pub fn total(&self) -> usize { self.entries.len() }
    pub fn row(&self, r: usize) -> Option<&Entry> { self.view.get(r).map(|&i| &self.entries[i]) }
    pub fn is_selected(&self, r: usize) -> bool { self.view.get(r).is_some_and(|i| self.selected.contains(i)) }
    pub fn cursor_entry(&self) -> Option<&Entry> { self.cursor.and_then(|r| self.row(r)) }

    /// Selected entries in row order.
    pub fn selection(&self) -> Vec<&Entry> {
        self.view.iter().filter(|i| self.selected.contains(i)).map(|&i| &self.entries[i]).collect()
    }
    pub fn selected_paths(&self) -> Vec<PathBuf> { self.selection().into_iter().map(|e| e.path.clone()).collect() }
    pub fn selected_count(&self) -> usize { self.selected.len() }
    /// Total size of the selected files (folders count 0).
    pub fn selected_size(&self) -> u64 { self.selected.iter().map(|&i| self.entries[i].size).sum() }

    /// What an action acts on: the selection, else the cursor's entry.
    pub fn targets(&self) -> Vec<PathBuf> {
        let s = self.selected_paths();
        if !s.is_empty() { return s }
        self.cursor_entry().map(|e| vec![e.path.clone()]).unwrap_or_default()
    }

    // ---- selection gestures

    /// A click on row `r` (None: on empty space) with the modifier keys.
    pub fn click(&mut self, r: Option<usize>, ctrl: bool, shift: bool) {
        let Some(r) = r.filter(|&r| r < self.view.len()) else {
            if !ctrl && !shift { self.selected.clear(); }
            return;
        };
        if shift {
            let a = self.anchor.unwrap_or(r).min(self.view.len() - 1);
            if !ctrl { self.selected.clear(); }
            let (lo, hi) = (a.min(r), a.max(r));
            self.selected.extend(self.view[lo..=hi].iter().copied());
            self.anchor = Some(a);
        } else if ctrl {
            let i = self.view[r];
            if !self.selected.remove(&i) { self.selected.insert(i); }
            self.anchor = Some(r);
        } else {
            self.selected.clear();
            self.selected.insert(self.view[r]);
            self.anchor = Some(r);
        }
        self.cursor = Some(r);
    }

    /// A press on a row that keeps an existing multi-selection when the row
    /// is part of it (so it can be dragged); otherwise like `click`.
    pub fn press(&mut self, r: usize, ctrl: bool, shift: bool) {
        if !ctrl && !shift && self.is_selected(r) {
            self.cursor = Some(r);
            return;
        }
        self.click(Some(r), ctrl, shift);
    }

    pub fn select_all(&mut self) {
        self.selected = self.view.iter().copied().collect();
    }

    /// Entry indices of the current selection.
    pub fn selected_indices(&self) -> Vec<usize> { self.selected.iter().copied().collect() }

    /// Rubber-band selection: the rows the box touches, plus `base` (entries
    /// selected before a Ctrl+drag started).
    pub fn select_rows(&mut self, rows: &[usize], base: &[usize]) {
        self.selected.clear();
        self.selected.extend(base.iter().copied());
        for &r in rows {
            if let Some(&i) = self.view.get(r) { self.selected.insert(i); }
        }
        if let Some(&r) = rows.first() {
            self.cursor = Some(r);
            self.anchor = Some(r);
        }
    }

    pub fn clear_selection(&mut self) { self.selected.clear() }

    /// Moves the cursor by `delta` rows (clamped); `to_end` jumps to the
    /// first / last row. Shift extends from the anchor, Ctrl moves only the cursor.
    pub fn move_cursor(&mut self, delta: isize, to_end: bool, shift: bool, ctrl: bool) {
        if self.view.is_empty() { return }
        let last = self.view.len() as isize - 1;
        let from = self.cursor.map_or(if delta < 0 { last + 1 } else { -1 }, |c| c as isize);
        let to = if to_end { if delta < 0 { 0 } else { last } } else { (from + delta).clamp(0, last) } as usize;
        if ctrl {
            self.cursor = Some(to);
        } else if shift {
            let a = self.anchor.or(self.cursor).unwrap_or(to);
            self.selected.clear();
            let (lo, hi) = (a.min(to), a.max(to));
            self.selected.extend(self.view[lo..=hi].iter().copied());
            self.anchor = Some(a);
            self.cursor = Some(to);
        } else {
            self.click(Some(to), false, false);
        }
    }

    /// The next row (after the cursor, wrapping) whose name starts with
    /// `prefix` (type-to-select).
    pub fn find_prefix(&self, prefix: &str) -> Option<usize> {
        if prefix.is_empty() || self.view.is_empty() { return None }
        let p = prefix.to_lowercase();
        let n = self.view.len();
        let start = if prefix.chars().count() > 1 { self.cursor.unwrap_or(0) } else { self.cursor.map_or(0, |c| c + 1) };
        (0..n).map(|k| (start + k) % n).find(|&r| self.entries[self.view[r]].name.to_lowercase().starts_with(&p))
    }

    /// The parent folder of the current one, if any.
    pub fn parent(&self) -> Option<&Path> { self.path.parent() }

    /// Paths back and forward (for saving history is not needed; tests).
    pub fn history(&self) -> (&[PathBuf], &[PathBuf]) { (&self.back, &self.forward) }
}

/// Folders first, then by `key`; ties by name. `asc` flips the key's order
/// (folders stay first).
pub fn compare(a: &Entry, b: &Entry, key: SortKey, asc: bool) -> Ordering {
    if a.is_dir != b.is_dir { return if a.is_dir { Ordering::Less } else { Ordering::Greater } }
    let by_name = || natord::compare(&a.name, &b.name);
    let o = match key {
        SortKey::Name => by_name(),
        SortKey::Modified => a.modified.cmp(&b.modified).then_with(by_name),
        SortKey::Type => a.ext.cmp(&b.ext).then_with(by_name),
        SortKey::Size => a.size.cmp(&b.size).then_with(by_name),
    };
    if asc { o } else { o.reverse() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, dir: bool, size: u64, m: i64) -> Entry {
        Entry { name: name.into(), path: PathBuf::from("/x").join(name), is_dir: dir, size, modified: Some(m), hidden: name.starts_with('.'), ext: crate::listing::ext_of(name) }
    }

    fn sample() -> PaneState {
        let mut p = PaneState::new("/x".into());
        p.set_entries(vec![e("b.txt", false, 10, 3), e("a10.txt", false, 30, 1), e("dir", true, 0, 2), e("a2.txt", false, 20, 5), e(".hidden", false, 1, 9)], None);
        p
    }

    fn names(p: &PaneState) -> Vec<&str> { (0..p.len()).map(|r| p.row(r).unwrap().name.as_str()).collect() }

    #[test]
    fn sorts_folders_first() {
        let mut p = sample();
        assert_eq!(names(&p), ["dir", "a2.txt", "a10.txt", "b.txt"]);
        p.sort_by(SortKey::Size);
        assert_eq!(names(&p), ["dir", "a10.txt", "a2.txt", "b.txt"]);
        p.sort_by(SortKey::Size);
        assert_eq!(names(&p), ["dir", "b.txt", "a2.txt", "a10.txt"]);
        p.set_sort(SortKey::Modified, false);
        assert_eq!(names(&p), ["dir", "a2.txt", "b.txt", "a10.txt"]);
    }

    #[test]
    fn hidden_and_filter() {
        let mut p = sample();
        p.set_show_hidden(true);
        assert_eq!(p.len(), 5);
        p.set_filter("A1");
        assert_eq!(names(&p), ["a10.txt"]);
        p.set_filter("");
        assert_eq!(p.len(), 5);
    }

    #[test]
    fn selection_gestures() {
        let mut p = sample();
        p.click(Some(1), false, false);
        p.click(Some(3), false, true);
        assert_eq!(p.selected_count(), 3);
        p.click(Some(2), true, false);
        assert_eq!(p.selected_count(), 2);
        assert_eq!(p.selection().iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a2.txt", "b.txt"]);
        assert_eq!(p.selected_size(), 30);
        p.click(None, false, false);
        assert_eq!(p.selected_count(), 0);
        p.cursor = None;
        p.move_cursor(1, false, false, false);
        assert_eq!(p.cursor, Some(0));
        p.move_cursor(1, true, true, false);
        assert_eq!(p.selected_count(), 4);
        p.select_all();
        assert_eq!(p.targets().len(), 4);
    }

    #[test]
    fn press_keeps_multi_selection() {
        let mut p = sample();
        p.select_all();
        p.press(2, false, false);
        assert_eq!(p.selected_count(), 4);
    }

    #[test]
    fn refresh_keeps_selection_by_name() {
        let mut p = sample();
        p.click(Some(3), false, false);
        p.set_entries(vec![e("b.txt", false, 10, 3), e("new.txt", false, 1, 1), e("a.txt", false, 1, 1)], None);
        assert_eq!(p.selection()[0].name, "b.txt");
        assert_eq!(p.cursor_entry().unwrap().name, "b.txt");
        p.set_entries(vec![e("b.txt", false, 10, 3)], Some("b.txt"));
        assert_eq!(p.cursor, Some(0));
    }

    #[test]
    fn history() {
        let mut p = PaneState::new("/a".into());
        p.navigate("/a/b".into());
        p.navigate("/a/b/c".into());
        assert!(p.back());
        assert_eq!(p.path, PathBuf::from("/a/b"));
        assert!(p.forward());
        assert_eq!(p.path, PathBuf::from("/a/b/c"));
        assert_eq!(p.up().as_deref(), Some("c"));
        assert_eq!(p.path, PathBuf::from("/a/b"));
        assert!(!p.can_forward());
        assert!(!p.navigate("/a/b".into()));
    }

    #[test]
    fn type_to_select() {
        let p = sample();
        assert_eq!(p.find_prefix("b"), Some(3));
        assert_eq!(p.find_prefix("a1"), Some(2));
        assert_eq!(p.find_prefix("z"), None);
    }
}
