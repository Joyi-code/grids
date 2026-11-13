//! File operations shared by the panes and the shelf: run on a background
//! thread through the shell's tracked calls, then recorded in the operation
//! journal (grids-journal.jsonl next to the settings file).
//!
//! Everything here that touches the disk must be called off the UI thread;
//! [`after_op`] is the UI-thread part (notes follow moved folders).

use grids_core::journal::{item_with_stat, Journal, OpKind, RealFs};
use grids_shell::OpItem;
use gpui::App;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Copy,
    Move,
    Recycle,
    Delete,
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static JOURNAL: OnceLock<Mutex<Journal>> = OnceLock::new();

/// Where the journal lives (the settings file's folder). Call once at start.
pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// The journal, read from disk on first use (off the UI thread).
pub fn journal() -> &'static Mutex<Journal> {
    JOURNAL.get_or_init(|| Mutex::new(Journal::open(DIR.get().map(PathBuf::as_path).unwrap_or(Path::new(".")))))
}

/// Appends a record for `items` (nothing for an empty list). Background only.
pub fn log(kind: OpKind, pane: Option<usize>, items: &[OpItem]) {
    if items.is_empty() { return }
    let recs = items.iter().map(|i| item_with_stat(i.src.clone(), i.dst.clone(), i.is_dir, &RealFs)).collect();
    if let Ok(mut j) = journal().lock() { let _ = j.append(kind, pane, recs); }
}

/// Runs `op` and journals what actually happened. Background only.
pub fn run(op: Op, paths: &[PathBuf], dest: Option<&Path>, pane: Option<usize>) -> io::Result<Vec<OpItem>> {
    let (kind, res) = match (op, dest) {
        (Op::Copy, Some(d)) => (OpKind::Copy, grids_shell::copy_tracked(paths, d)),
        (Op::Move, Some(d)) => (OpKind::Move, grids_shell::move_tracked(paths, d)),
        // Confirmed in our own dialog already: no system prompt.
        (Op::Recycle, _) => (OpKind::Recycle, grids_shell::recycle_tracked(paths, true)),
        (Op::Delete, _) => (OpKind::Delete, grids_shell::delete_tracked(paths, true)),
        _ => return Ok(Vec::new()),
    };
    if let Ok(items) = &res { log(kind, pane, items) }
    res
}

/// UI thread, after an operation: folder notes and shelf entries follow
/// moved items; deleted items leave the shelf. True when the settings
/// changed (they need saving).
pub fn after_op(op: Op, items: &[OpItem], cx: &mut App) -> bool {
    match op {
        Op::Move => follow_moves(items, cx),
        Op::Recycle | Op::Delete => {
            let shelf = &mut cx.global_mut::<crate::state::Shared>().config.shelf;
            let n = shelf.len();
            shelf.retain(|p| !items.iter().any(|i| &i.src == p));
            shelf.len() != n
        }
        Op::Copy => false,
    }
}

/// After a move or rename (`src` -> `dst`).
pub fn follow_moves(items: &[OpItem], cx: &mut App) -> bool {
    let config = &mut cx.global_mut::<crate::state::Shared>().config;
    let mut changed = false;
    for i in items {
        let Some(d) = &i.dst else { continue };
        if i.is_dir && grids_core::notes::rewrite_moved(&mut config.notes, &i.src, d) > 0 { changed = true }
        for p in config.shelf.iter_mut() {
            if p == &i.src { *p = d.clone(); changed = true }
            else if let Ok(rest) = p.strip_prefix(&i.src) { *p = d.join(rest); changed = true }
        }
    }
    changed
}

/// "截图 2026-11-13 153012.png" style names for pasted clipboard content.
pub fn stamp_name(prefix: &str, ext: &str) -> String {
    format!("{prefix} {}.{ext}", chrono::Local::now().format("%Y-%m-%d %H%M%S"))
}
