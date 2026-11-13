//! Operation journal and undo planning.
//!
//! Every file operation started in Grids is one [`Record`], appended as one
//! JSON line to `grids-journal.jsonl` next to the exe. The file is only
//! appended to; undoing a record appends an [`OpKind::Undo`] record naming
//! it, and [`Journal::open`] folds those back into [`Record::undone`]. Past
//! 1 MB or 2000 lines the file rotates to `grids-journal.1.jsonl` (one old
//! file kept).
//!
//! [`plan_undo`] turns a record plus the current file state into the steps
//! that reverse it, or the reason it can't be reversed. It only looks at the
//! file system through [`FsProbe`], so it is tested without touching disk.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub const JOURNAL_FILE: &str = "grids-journal.jsonl";
pub const JOURNAL_OLD_FILE: &str = "grids-journal.1.jsonl";
/// Rotate past this many bytes...
pub const ROTATE_BYTES: u64 = 1024 * 1024;
/// ...or this many records.
pub const ROTATE_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Copy,
    Move,
    Rename,
    Recycle,
    /// Permanent delete: recorded, never undoable.
    Delete,
    /// New file / folder from the New menu.
    New,
    /// Clipboard image / text pasted as a file.
    PasteFile,
    /// Undo of the record [`Record::undo_of`].
    Undo,
}

impl OpKind {
    pub fn undoable(self) -> bool {
        !matches!(self, OpKind::Delete | OpKind::Undo)
    }
}

/// One item of a record. `size` / `mtime` describe the result (`dst`) right
/// after the operation, to notice later edits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordItem {
    pub src: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst: Option<PathBuf>,
    #[serde(default)]
    pub is_dir: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    /// Milliseconds since the Unix epoch.
    pub time_ms: i64,
    pub kind: OpKind,
    /// The pane the operation started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<usize>,
    pub items: Vec<RecordItem>,
    /// Folded in on load from later Undo records; written as `false`.
    #[serde(default)]
    pub undone: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<u64>,
}

/// File state as [`plan_undo`] needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub is_dir: bool,
    pub size: u64,
    /// Seconds since the Unix epoch.
    pub mtime: Option<i64>,
}

/// Read-only view of the file system for [`plan_undo`].
pub trait FsProbe {
    /// `None` when nothing exists at `path`.
    fn stat(&self, path: &Path) -> Option<FileStat>;
}

/// The real file system.
pub struct RealFs;

impl FsProbe for RealFs {
    fn stat(&self, path: &Path) -> Option<FileStat> {
        let m = std::fs::symlink_metadata(path).ok()?;
        Some(FileStat {
            is_dir: m.is_dir(),
            size: if m.is_dir() { 0 } else { m.len() },
            mtime: m.modified().ok().map(unix_secs),
        })
    }
}

fn unix_secs(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// One step of an undo. Flags mark steps the UI should confirm first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoStep {
    /// Undo a copy / new / paste: send `path` to the Recycle Bin.
    /// `changed`: edited since (ask first).
    Recycle { path: PathBuf, changed: bool },
    /// Undo a move: move `from` back to `to`. `occupied`: something now sits
    /// at `to` (offer "put back under a new name" or cancel).
    MoveBack { from: PathBuf, to: PathBuf, occupied: bool },
    /// Undo a rename.
    Rename { from: PathBuf, to: PathBuf },
    /// Undo a recycle: restore `original` from the Recycle Bin, matching the
    /// entry deleted nearest `deleted_ms`.
    Restore { original: PathBuf, deleted_ms: i64 },
    /// This item can't be undone (others in the record can).
    Skip { path: PathBuf, why: UndoBlock },
}

/// Why a record (or one item) can't be undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoBlock {
    AlreadyUndone,
    /// Permanent deletes and undo records.
    NotUndoable(OpKind),
    /// The record has no items.
    Empty,
    /// The result of the operation is gone (deleted or moved elsewhere).
    Missing(PathBuf),
    /// The original location is taken again.
    Occupied(PathBuf),
    /// A renamed item has been renamed again since.
    Renamed(PathBuf),
}

impl std::fmt::Display for UndoBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UndoBlock::AlreadyUndone => write!(f, "已经撤销过"),
            UndoBlock::NotUndoable(OpKind::Delete) => write!(f, "永久删除无法撤销"),
            UndoBlock::NotUndoable(_) => write!(f, "这一步无法撤销"),
            UndoBlock::Empty => write!(f, "记录里没有文件"),
            UndoBlock::Missing(p) => write!(f, "{} 已不存在", p.display()),
            UndoBlock::Occupied(p) => write!(f, "{} 已被占用", p.display()),
            UndoBlock::Renamed(p) => write!(f, "{} 之后又被改过名", p.display()),
        }
    }
}

fn is_case_only_rename(a: &Path, b: &Path) -> bool {
    crate::link::CASE_INSENSITIVE && crate::link::same_path(a, b)
}

/// The steps that reverse `record` given the current file state. `Err` when
/// nothing in it can be undone; items that can't are `Skip` steps
/// otherwise.
pub fn plan_undo(record: &Record, fs: &dyn FsProbe) -> Result<Vec<UndoStep>, UndoBlock> {
    if record.undone {
        return Err(UndoBlock::AlreadyUndone);
    }
    if !record.kind.undoable() {
        return Err(UndoBlock::NotUndoable(record.kind));
    }
    if record.items.is_empty() {
        return Err(UndoBlock::Empty);
    }
    let steps: Vec<UndoStep> = record.items.iter().map(|it| plan_item(record, it, fs)).collect();
    if steps.iter().all(|s| matches!(s, UndoStep::Skip { .. }))
        && let Some(UndoStep::Skip { why, .. }) = steps.first()
    {
        return Err(why.clone());
    }
    Ok(steps)
}

fn plan_item(record: &Record, it: &RecordItem, fs: &dyn FsProbe) -> UndoStep {
    let skip = |path: &Path, why: UndoBlock| UndoStep::Skip { path: path.to_path_buf(), why };
    let dst = it.dst.as_deref().unwrap_or(&it.src);
    match record.kind {
        OpKind::Copy | OpKind::New | OpKind::PasteFile => match fs.stat(dst) {
            None => skip(dst, UndoBlock::Missing(dst.to_path_buf())),
            Some(now) => {
                let same = it.size.is_none_or(|s| s == now.size) && (it.mtime.is_none() || it.mtime == now.mtime);
                // a new file still empty counts as untouched even if its time moved
                let untouched = same || (record.kind != OpKind::Copy && !now.is_dir && now.size == 0);
                UndoStep::Recycle {
                    path: dst.to_path_buf(),
                    changed: !untouched,
                }
            }
        },
        OpKind::Move => match fs.stat(dst) {
            None => skip(dst, UndoBlock::Missing(dst.to_path_buf())),
            Some(_) => UndoStep::MoveBack {
                from: dst.to_path_buf(),
                to: it.src.clone(),
                occupied: fs.stat(&it.src).is_some(),
            },
        },
        OpKind::Rename => {
            if fs.stat(dst).is_none() {
                return skip(dst, UndoBlock::Renamed(dst.to_path_buf()));
            }
            if fs.stat(&it.src).is_some() && !is_case_only_rename(&it.src, dst) {
                return skip(&it.src, UndoBlock::Occupied(it.src.clone()));
            }
            UndoStep::Rename {
                from: dst.to_path_buf(),
                to: it.src.clone(),
            }
        }
        OpKind::Recycle => {
            if fs.stat(&it.src).is_some() {
                return skip(&it.src, UndoBlock::Occupied(it.src.clone()));
            }
            UndoStep::Restore {
                original: it.src.clone(),
                deleted_ms: record.time_ms,
            }
        }
        OpKind::Delete | OpKind::Undo => skip(&it.src, UndoBlock::NotUndoable(record.kind)),
    }
}

/// The journal file pair in one folder (normally the exe's).
#[derive(Debug)]
pub struct Journal {
    dir: PathBuf,
    records: Vec<Record>,
    next_id: u64,
    /// Lines / bytes in the current file, for rotation.
    lines: usize,
    bytes: u64,
}

fn read_lines(path: &Path) -> (Vec<Record>, usize, u64) {
    let Ok(f) = std::fs::File::open(path) else { return (Vec::new(), 0, 0) };
    let bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut out = Vec::new();
    let mut lines = 0;
    for line in io::BufReader::new(f).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        lines += 1;
        // a torn last line (crash mid-write) or a newer format: skip it
        if let Ok(r) = serde_json::from_str::<Record>(&line) {
            out.push(r);
        }
    }
    (out, lines, bytes)
}

impl Journal {
    /// Reads `dir/grids-journal.1.jsonl` and `dir/grids-journal.jsonl`.
    /// Unreadable lines are skipped; a missing file is an empty journal.
    pub fn open(dir: &Path) -> Journal {
        let (mut records, _, _) = read_lines(&dir.join(JOURNAL_OLD_FILE));
        let (current, lines, bytes) = read_lines(&dir.join(JOURNAL_FILE));
        records.extend(current);
        let undone: std::collections::HashSet<u64> = records.iter().filter_map(|r| r.undo_of).collect();
        for r in &mut records {
            r.undone = undone.contains(&r.id);
        }
        let next_id = records.iter().map(|r| r.id).max().map_or(1, |m| m + 1);
        Journal {
            dir: dir.to_path_buf(),
            records,
            next_id,
            lines,
            bytes,
        }
    }

    /// All records, oldest first (Undo records included).
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    pub fn get(&self, id: u64) -> Option<&Record> {
        self.records.iter().find(|r| r.id == id)
    }

    /// The newest record Ctrl+Z should undo: undoable and not undone yet.
    pub fn last_undoable(&self) -> Option<&Record> {
        self.records.iter().rev().find(|r| r.kind.undoable() && !r.undone)
    }

    /// For the history panel: newest first, Undo records left out.
    pub fn history(&self) -> impl Iterator<Item = &Record> {
        self.records.iter().rev().filter(|r| r.kind != OpKind::Undo)
    }

    /// Appends a record stamped now. Empty `items` are not recorded (`None`).
    pub fn append(&mut self, kind: OpKind, pane: Option<usize>, items: Vec<RecordItem>) -> io::Result<Option<&Record>> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        self.append_at(now, kind, pane, items, None)
    }

    /// Records that `id` was undone by running `items` (what the undo did).
    pub fn mark_undone(&mut self, id: u64, pane: Option<usize>, items: Vec<RecordItem>) -> io::Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        self.append_at(now, OpKind::Undo, pane, items, Some(id))?;
        Ok(())
    }

    pub(crate) fn append_at(&mut self, time_ms: i64, kind: OpKind, pane: Option<usize>, items: Vec<RecordItem>, undo_of: Option<u64>) -> io::Result<Option<&Record>> {
        if items.is_empty() && undo_of.is_none() {
            return Ok(None);
        }
        let rec = Record {
            id: self.next_id,
            time_ms,
            kind,
            pane,
            items,
            undone: false,
            undo_of,
        };
        let mut line = serde_json::to_string(&rec).map_err(io::Error::other)?;
        line.push('\n');
        if self.lines >= ROTATE_LINES || self.bytes + line.len() as u64 > ROTATE_BYTES && self.lines > 0 {
            self.rotate()?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(JOURNAL_FILE))?;
        f.write_all(line.as_bytes())?;
        self.lines += 1;
        self.bytes += line.len() as u64;
        self.next_id += 1;
        if let Some(r) = undo_of.and_then(|t| self.records.iter_mut().find(|r| r.id == t)) {
            r.undone = true;
        }
        self.records.push(rec);
        Ok(self.records.last())
    }

    /// Current file -> `.1` (replacing the previous old file). Records from
    /// files older than that drop out of memory too.
    fn rotate(&mut self) -> io::Result<()> {
        let old = self.dir.join(JOURNAL_OLD_FILE);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(self.dir.join(JOURNAL_FILE), &old)?;
        let (kept, _, _) = read_lines(&old);
        let keep: std::collections::HashSet<u64> = kept.iter().map(|r| r.id).collect();
        self.records.retain(|r| keep.contains(&r.id));
        self.lines = 0;
        self.bytes = 0;
        Ok(())
    }
}

/// A [`RecordItem`] for `src` -> `dst`, with `dst`'s current size and time.
pub fn item_with_stat(src: PathBuf, dst: Option<PathBuf>, is_dir: bool, fs: &dyn FsProbe) -> RecordItem {
    let st = dst.as_deref().and_then(|d| fs.stat(d));
    RecordItem {
        src,
        dst,
        is_dir,
        size: st.map(|s| s.size),
        mtime: st.and_then(|s| s.mtime),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeFs(HashMap<PathBuf, FileStat>);

    impl FakeFs {
        fn new(files: &[(&str, u64, i64)]) -> FakeFs {
            FakeFs(
                files
                    .iter()
                    .map(|(p, s, t)| {
                        (
                            PathBuf::from(p),
                            FileStat {
                                is_dir: false,
                                size: *s,
                                mtime: Some(*t),
                            },
                        )
                    })
                    .collect(),
            )
        }
    }

    impl FsProbe for FakeFs {
        fn stat(&self, path: &Path) -> Option<FileStat> {
            self.0.get(path).copied()
        }
    }

    fn item(src: &str, dst: Option<&str>, size: u64, mtime: i64) -> RecordItem {
        RecordItem {
            src: src.into(),
            dst: dst.map(PathBuf::from),
            is_dir: false,
            size: Some(size),
            mtime: Some(mtime),
        }
    }

    fn rec(kind: OpKind, items: Vec<RecordItem>) -> Record {
        Record {
            id: 7,
            time_ms: 1_700_000_000_000,
            kind,
            pane: Some(1),
            items,
            undone: false,
            undo_of: None,
        }
    }

    #[test]
    fn serialises_compactly() {
        let r = rec(OpKind::Copy, vec![item("/a/x.txt", Some("/b/x - 副本.txt"), 3, 99)]);
        let line = serde_json::to_string(&r).unwrap();
        assert!(line.contains("\"kind\":\"copy\""), "{line}");
        assert!(!line.contains("undo_of"));
        assert_eq!(serde_json::from_str::<Record>(&line).unwrap(), r);
        // older / hand-edited lines without optional fields still load
        let min: Record = serde_json::from_str(r#"{"id":1,"time_ms":5,"kind":"paste_file","items":[{"src":"/n.png"}]}"#).unwrap();
        assert_eq!(min.kind, OpKind::PasteFile);
        assert_eq!(min.items[0].dst, None);
        assert!(!min.undone);
    }

    #[test]
    fn undo_copy_and_new() {
        let fs = FakeFs::new(&[("/b/x.txt", 3, 99), ("/b/y.txt", 4, 100), ("/n.txt", 0, 500), ("/m.txt", 9, 600)]);
        let r = rec(
            OpKind::Copy,
            vec![
                item("/a/x.txt", Some("/b/x.txt"), 3, 99),
                item("/a/y.txt", Some("/b/y.txt"), 4, 99),
                item("/a/z.txt", Some("/b/z.txt"), 1, 1),
            ],
        );
        let steps = plan_undo(&r, &fs).unwrap();
        assert_eq!(
            steps[0],
            UndoStep::Recycle {
                path: "/b/x.txt".into(),
                changed: false
            }
        );
        assert_eq!(
            steps[1],
            UndoStep::Recycle {
                path: "/b/y.txt".into(),
                changed: true
            }
        );
        assert_eq!(
            steps[2],
            UndoStep::Skip {
                path: "/b/z.txt".into(),
                why: UndoBlock::Missing("/b/z.txt".into())
            }
        );
        // a new file that is still empty is untouched even if its time moved
        let r = rec(OpKind::New, vec![item("/n.txt", Some("/n.txt"), 0, 1)]);
        assert_eq!(
            plan_undo(&r, &fs).unwrap(),
            vec![UndoStep::Recycle {
                path: "/n.txt".into(),
                changed: false
            }]
        );
        let r = rec(OpKind::PasteFile, vec![item("/m.txt", Some("/m.txt"), 5, 600)]);
        assert_eq!(plan_undo(&r, &fs).unwrap(), vec![UndoStep::Recycle { path: "/m.txt".into(), changed: true }]);
        // all gone: an error, not a list of skips
        let r = rec(OpKind::Copy, vec![item("/a", Some("/gone"), 1, 1)]);
        assert_eq!(plan_undo(&r, &fs), Err(UndoBlock::Missing("/gone".into())));
    }

    #[test]
    fn undo_move_rename_recycle() {
        let fs = FakeFs::new(&[
            ("/to/a.txt", 1, 1),
            ("/to/b.txt", 1, 1),
            ("/from/b.txt", 2, 2),
            ("/d/new.txt", 1, 1),
            ("/d/taken.txt", 1, 1),
            ("/d/taken2.txt", 1, 1),
        ]);
        let r = rec(OpKind::Move, vec![item("/from/a.txt", Some("/to/a.txt"), 1, 1), item("/from/b.txt", Some("/to/b.txt"), 1, 1)]);
        assert_eq!(
            plan_undo(&r, &fs).unwrap(),
            vec![
                UndoStep::MoveBack {
                    from: "/to/a.txt".into(),
                    to: "/from/a.txt".into(),
                    occupied: false
                },
                UndoStep::MoveBack {
                    from: "/to/b.txt".into(),
                    to: "/from/b.txt".into(),
                    occupied: true
                },
            ]
        );
        let r = rec(OpKind::Rename, vec![item("/d/old.txt", Some("/d/new.txt"), 1, 1)]);
        assert_eq!(
            plan_undo(&r, &fs).unwrap(),
            vec![UndoStep::Rename {
                from: "/d/new.txt".into(),
                to: "/d/old.txt".into()
            }]
        );
        let r = rec(OpKind::Rename, vec![item("/d/old.txt", Some("/d/renamed-again.txt"), 1, 1)]);
        assert_eq!(plan_undo(&r, &fs), Err(UndoBlock::Renamed("/d/renamed-again.txt".into())));
        let r = rec(OpKind::Rename, vec![item("/d/taken.txt", Some("/d/taken2.txt"), 1, 1)]);
        assert_eq!(plan_undo(&r, &fs), Err(UndoBlock::Occupied("/d/taken.txt".into())));
        let r = rec(OpKind::Recycle, vec![item("/d/gone.txt", Some("/bin/$R1.txt"), 1, 1), item("/d/taken.txt", None, 1, 1)]);
        let steps = plan_undo(&r, &fs).unwrap();
        assert_eq!(
            steps[0],
            UndoStep::Restore {
                original: "/d/gone.txt".into(),
                deleted_ms: 1_700_000_000_000
            }
        );
        assert!(matches!(&steps[1], UndoStep::Skip { why: UndoBlock::Occupied(_), .. }));
    }

    #[test]
    fn not_undoable() {
        let fs = FakeFs::new(&[]);
        assert_eq!(plan_undo(&rec(OpKind::Delete, vec![item("/a", None, 1, 1)]), &fs), Err(UndoBlock::NotUndoable(OpKind::Delete)));
        assert_eq!(plan_undo(&rec(OpKind::Copy, vec![]), &fs), Err(UndoBlock::Empty));
        let mut r = rec(OpKind::Copy, vec![item("/a", Some("/b"), 1, 1)]);
        r.undone = true;
        assert_eq!(plan_undo(&r, &fs), Err(UndoBlock::AlreadyUndone));
        assert_eq!(UndoBlock::NotUndoable(OpKind::Delete).to_string(), "永久删除无法撤销");
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grids-journal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn file_round_trip_and_undo_folding() {
        let d = scratch("rt");
        let mut j = Journal::open(&d);
        assert!(j.records().is_empty());
        assert!(j.append(OpKind::Copy, Some(0), vec![]).unwrap().is_none());
        let a = j.append_at(1000, OpKind::Copy, Some(0), vec![item("/a", Some("/b"), 1, 1)], None).unwrap().unwrap().id;
        let b = j.append_at(2000, OpKind::Delete, Some(1), vec![item("/c", None, 1, 1)], None).unwrap().unwrap().id;
        assert_eq!((a, b), (1, 2));
        // Delete is not undoable: Ctrl+Z goes past it
        assert_eq!(j.last_undoable().map(|r| r.id), Some(a));
        j.mark_undone(a, Some(0), vec![item("/b", None, 1, 1)]).unwrap();
        assert!(j.get(a).unwrap().undone);
        assert!(j.last_undoable().is_none());
        // a torn line is skipped on load
        std::fs::OpenOptions::new().append(true).open(d.join(JOURNAL_FILE)).unwrap().write_all(b"{\"id\":9,\"ti").unwrap();
        let j2 = Journal::open(&d);
        assert_eq!(j2.records().len(), 3);
        assert!(j2.get(a).unwrap().undone);
        assert!(!j2.get(b).unwrap().undone);
        assert_eq!(j2.history().map(|r| r.id).collect::<Vec<_>>(), vec![b, a]);
        assert_eq!(j2.next_id, 4);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rotates_by_lines_keeping_one_old_file() {
        let d = scratch("rot");
        let mut j = Journal::open(&d);
        for i in 0..(ROTATE_LINES * 2 + 5) {
            j.append_at(i as i64, OpKind::Copy, None, vec![item("/a", Some("/b"), 1, 1)], None).unwrap();
        }
        let (old, old_lines, _) = read_lines(&d.join(JOURNAL_OLD_FILE));
        let (cur, cur_lines, _) = read_lines(&d.join(JOURNAL_FILE));
        assert_eq!((old_lines, cur_lines), (ROTATE_LINES, 5));
        assert_eq!(old.first().unwrap().id, ROTATE_LINES as u64 + 1);
        assert_eq!(cur.last().unwrap().id, (ROTATE_LINES * 2 + 5) as u64);
        assert_eq!(j.records().len(), ROTATE_LINES + 5);
        let reopened = Journal::open(&d);
        assert_eq!(reopened.records().len(), ROTATE_LINES + 5);
        assert_eq!(reopened.next_id, (ROTATE_LINES * 2 + 6) as u64);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rotates_by_size() {
        let d = scratch("size");
        let mut j = Journal::open(&d);
        let long = "x".repeat(4000);
        let mut n = 0;
        while !d.join(JOURNAL_OLD_FILE).exists() {
            j.append_at(n, OpKind::Copy, None, vec![item(&format!("/{long}"), Some("/b"), 1, 1)], None).unwrap();
            n += 1;
            assert!(n < 1000);
        }
        assert!(std::fs::metadata(d.join(JOURNAL_OLD_FILE)).unwrap().len() <= ROTATE_BYTES);
        assert_eq!(read_lines(&d.join(JOURNAL_FILE)).1, 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn real_fs_probe() {
        let d = scratch("probe");
        let f = d.join("f.txt");
        std::fs::write(&f, "abc").unwrap();
        let it = item_with_stat(PathBuf::from("/src"), Some(f.clone()), false, &RealFs);
        assert_eq!(it.size, Some(3));
        assert!(it.mtime.is_some());
        assert!(RealFs.stat(&d).unwrap().is_dir);
        assert!(RealFs.stat(&d.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
