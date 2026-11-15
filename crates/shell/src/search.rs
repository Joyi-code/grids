//! Walking a folder tree for the deep search. Depth- and count-bounded, with a
//! cancel flag the UI can flip when the query changes or the panel closes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// One file found by [`walk`].
pub struct Found {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
    pub hidden: bool,
    pub ext: String,
}

const SKIP: [&str; 4] = ["$RECYCLE.BIN", "System Volume Information", "$WinREAgent", "node_modules"];
const MAX_DEPTH: usize = 24;

/// Walks `root` breadth-first, handing each entry to `on` until `cancel` is set,
/// `limit` entries have been passed on, or the tree ends. Returns the number
/// of entries passed on. Unreadable folders are skipped silently.
pub fn walk(root: &Path, cancel: &AtomicBool, limit: usize, on: &mut dyn FnMut(Found)) -> usize {
    let mut queue: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = queue.pop() {
        if cancel.load(Ordering::Relaxed) || seen >= limit { break }
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            if cancel.load(Ordering::Relaxed) || seen >= limit { break }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.is_empty() { continue }
            let is_dir = meta.is_dir();
            let attr = hidden_of(&meta, &name);
            let ext = if is_dir { String::new() } else { path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default() };
            seen += 1;
            on(Found {
                name,
                path: path.clone(),
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                modified: meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64),
                hidden: attr,
                ext,
            });
            if is_dir && depth + 1 < MAX_DEPTH && !SKIP.contains(&entry.file_name().to_string_lossy().as_ref()) {
                // Do not follow junctions or symlinked folders.
                if !entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
                    queue.push((path, depth + 1));
                }
            }
        }
    }
    seen
}

#[cfg(windows)]
fn hidden_of(meta: &std::fs::Metadata, name: &str) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    (meta.file_attributes() & (HIDDEN | SYSTEM)) != 0 || name.starts_with('.')
}

#[cfg(not(windows))]
fn hidden_of(_meta: &std::fs::Metadata, name: &str) -> bool { name.starts_with('.') }
