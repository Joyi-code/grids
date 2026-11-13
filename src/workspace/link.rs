//! Linked panes: a pane entering a sub-folder takes the others of its group
//! to the same place relative to each one's base folder; two linked panes
//! can compare their folders (green: only on this side, amber: same name,
//! other size or date). Groups live in the session (`Session::links`).

use super::Workspace;
use crate::pane::{LinkCmd, LinkView, Mark};
use crate::state::{self, Shared, tr};
use gpui::*;
use grids_core::link::{is_within, linked_target, same_path, CASE_INSENSITIVE};
use grids_core::listing::Entry;
use grids_core::session::{LinkGroup, LinkMember, MAX_LINK_GROUPS};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn groups(cx: &mut App) -> &mut Vec<LinkGroup> { &mut cx.global_mut::<Shared>().config.current.links }

/// Drops one-pane groups and compare where it no longer applies.
fn tidy(gs: &mut Vec<LinkGroup>) {
    gs.retain(|g| g.members.len() >= 2);
    for g in gs.iter_mut() { g.compare &= g.members.len() == 2 }
}

fn key(name: &str) -> String { if CASE_INSENSITIVE { name.to_lowercase() } else { name.to_string() } }

/// Marks for `mine` against `theirs`: folders only by existence.
pub fn compare_marks(mine: &[Entry], theirs: &[Entry]) -> HashMap<String, Mark> {
    let other: HashMap<String, &Entry> = theirs.iter().map(|e| (key(&e.name), e)).collect();
    mine.iter().filter_map(|e| {
        let m = match other.get(&key(&e.name)) {
            None => Mark::Only,
            Some(o) if o.is_dir != e.is_dir => Mark::Differs,
            Some(o) if !e.is_dir && (o.size != e.size || o.modified != e.modified) => Mark::Differs,
            Some(_) => return None,
        };
        Some((e.name.clone(), m))
    }).collect()
}

impl Workspace {
    fn group_of(&self, i: usize, cx: &App) -> Option<usize> {
        state::shared(cx).config.current.links.iter().position(|g| g.members.iter().any(|m| m.pane == i))
    }

    fn pane_path(&self, i: usize, cx: &App) -> Option<PathBuf> { self.panes.get(i).map(|p| p.read(cx).state.path.clone()) }

    pub(super) fn link_cmd(&mut self, i: usize, cmd: &LinkCmd, cx: &mut Context<Self>) {
        match cmd {
            LinkCmd::With(j) => {
                let j = *j;
                let (Some(pi), Some(pj)) = (self.pane_path(i, cx), self.pane_path(j, cx)) else { return };
                let (gi, gj) = (self.group_of(i, cx), self.group_of(j, cx));
                if gi.is_some() && gi == gj { return }
                if gi.is_none() && gj.is_none() && state::shared(cx).config.current.links.len() >= MAX_LINK_GROUPS {
                    let msg = tr(cx, "最多 4 组联动", "At most 4 link groups").to_string();
                    return self.show_toast(msg, cx);
                }
                let gs = groups(cx);
                // i leaves its group and joins j's (or a new one with j).
                for g in gs.iter_mut() { g.members.retain(|m| m.pane != i) }
                match gj {
                    Some(g) => { gs[g].members.push(LinkMember { pane: i, base: pi }); gs[g].compare = false }
                    None => gs.push(LinkGroup { members: vec![LinkMember { pane: j, base: pj }, LinkMember { pane: i, base: pi }], compare: false }),
                }
                tidy(gs);
            }
            LinkCmd::Unlink => {
                let gs = groups(cx);
                for g in gs.iter_mut() { g.members.retain(|m| m.pane != i) }
                tidy(gs);
                self.panes[i].update(cx, |p, cx| { p.link = None; cx.notify() });
            }
            LinkCmd::Compare(on) => {
                let Some(g) = self.group_of(i, cx) else { return };
                let gs = groups(cx);
                gs[g].compare = *on && gs[g].members.len() == 2;
            }
            LinkCmd::CreateMissing => return self.create_missing(i, cx),
        }
        self.sync_links(cx);
        self.schedule_save(cx);
    }

    /// Pane `i` went to `path`: the rest of its group follows when `path`
    /// is inside its base (and theirs are inside their own).
    pub(super) fn link_moved(&mut self, i: usize, path: &Path, cx: &mut Context<Self>) {
        // Moving on its own clears the "missing" placeholder.
        self.panes[i].update(cx, |p, cx| if let Some(l) = &mut p.link && l.missing.take().is_some() { cx.notify() });
        let Some(g) = self.group_of(i, cx) else { return };
        let group = state::shared(cx).config.current.links[g].clone();
        let Some(me) = group.members.iter().find(|m| m.pane == i) else { return };
        if is_within(path, &me.base) {
            for m in group.members.iter().filter(|m| m.pane != i) {
                let Some(cur) = self.pane_path(m.pane, cx) else { continue };
                if !is_within(&cur, &m.base) { continue }
                let Some(target) = linked_target(&me.base, path, &m.base) else { continue };
                if same_path(&target, &cur) { continue }
                let pane = self.panes[m.pane].clone();
                // The folder may be on a slow drive: look on a background thread.
                cx.spawn(async move |_, cx| {
                    let t = target.clone();
                    let exists = cx.background_executor().spawn(async move { t.is_dir() }).await;
                    let _ = pane.update(cx, |p, cx| {
                        if exists { p.navigate(target, cx) }
                        else if let Some(l) = &mut p.link { l.missing = Some(target); cx.notify() }
                    });
                }).detach();
            }
        }
        self.sync_links(cx);
    }

    /// Creates the folder pane `i` is missing (journalled as New), then goes there.
    fn create_missing(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(target) = self.panes[i].read(cx).link.as_ref().and_then(|l| l.missing.clone()) else { return };
        if let Some(root) = state::locked_root(cx, &target) { return self.show_locked(root, cx) }
        let pane = self.panes[i].clone();
        let idx = i;
        cx.spawn(async move |this, cx| {
            let t = target.clone();
            let r = cx.background_executor().spawn(async move {
                // Only the levels that don't exist yet go in the journal.
                let mut new = vec![];
                let mut p = t.as_path();
                while !p.exists() { new.push(p.to_path_buf()); match p.parent() { Some(q) => p = q, None => break } }
                std::fs::create_dir_all(&t)?;
                if let Some(top) = new.last() { crate::ops::log(grids_core::journal::OpKind::New, Some(idx), &[grids_shell::OpItem { src: top.clone(), dst: None, is_dir: true }]) }
                Ok::<_, std::io::Error>(())
            }).await;
            let _ = this.update(cx, |this, cx| match r {
                Ok(()) => pane.update(cx, |p, cx| { if let Some(l) = &mut p.link { l.missing = None } p.navigate(target, cx) }),
                Err(e) => { let msg = format!("{}{e}", tr(cx, "创建失败：", "Couldn't create it: ")); this.show_toast(msg, cx) }
            });
        }).detach();
    }

    /// Hands each pane its link state and compare marks.
    pub(super) fn sync_links(&mut self, cx: &mut Context<Self>) {
        let gs = state::shared(cx).config.current.links.clone();
        let n = self.panes.len();
        let mut views: Vec<Option<LinkView>> = vec![None; n];
        for (gi, g) in gs.iter().enumerate() {
            let pair = (g.compare && g.members.len() == 2).then(|| (g.members[0].pane, g.members[1].pane));
            for m in &g.members {
                let Some(p) = self.panes.get(m.pane) else { continue };
                let p = p.read(cx);
                let old = p.link.as_ref();
                views[m.pane] = Some(LinkView {
                    group: gi,
                    live: is_within(&p.state.path, &m.base),
                    compare: g.compare,
                    can_compare: g.members.len() == 2,
                    missing: old.and_then(|l| l.missing.clone()),
                    marks: Rc::default(),
                });
            }
            if let Some((a, b)) = pair && let (Some(pa), Some(pb)) = (self.panes.get(a), self.panes.get(b)) {
                let (ra, rb) = (pa.read(cx), pb.read(cx));
                if let (Some((da, ea)), Some((db, eb))) = (ra.listing(), rb.listing())
                    && same_path(da, &ra.state.path) && same_path(db, &rb.state.path)
                {
                    let (ma, mb) = (compare_marks(ea, eb), compare_marks(eb, ea));
                    if let Some(v) = &mut views[a] { v.marks = Rc::new(ma) }
                    if let Some(v) = &mut views[b] { v.marks = Rc::new(mb) }
                }
            }
        }
        for (i, v) in views.into_iter().enumerate() {
            self.panes[i].update(cx, |p, cx| { p.link = v; p.pane_count = n; cx.notify() });
        }
    }

    /// After panes moved between cells: `map(old) -> new` (None: gone).
    pub(super) fn remap_links(&mut self, map: impl Fn(usize) -> Option<usize>, cx: &mut Context<Self>) {
        let gs = groups(cx);
        for g in gs.iter_mut() {
            g.members.retain_mut(|m| match map(m.pane) { Some(k) => { m.pane = k; true } None => false });
        }
        tidy(gs);
        self.sync_links(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{compare_marks, Entry, Mark, PathBuf};

    fn e(name: &str, dir: bool, size: u64, t: i64) -> Entry {
        Entry { name: name.into(), path: PathBuf::from(name), is_dir: dir, size, modified: Some(t), hidden: false, ext: String::new() }
    }

    #[test]
    fn marks() {
        let a = vec![e("same.txt", false, 1, 1), e("new.txt", false, 1, 1), e("edit.txt", false, 2, 1), e("d", true, 0, 5), e("only", true, 0, 1)];
        let b = vec![e("same.txt", false, 1, 1), e("edit.txt", false, 3, 1), e("d", true, 0, 9)];
        let m = compare_marks(&a, &b);
        assert_eq!(m.get("new.txt"), Some(&Mark::Only));
        assert_eq!(m.get("only"), Some(&Mark::Only));
        assert_eq!(m.get("edit.txt"), Some(&Mark::Differs));
        assert_eq!(m.get("same.txt"), None);
        assert_eq!(m.get("d"), None, "folders compare by existence only");
        let back = compare_marks(&b, &a);
        assert_eq!((back.len(), back.get("edit.txt")), (1, Some(&Mark::Differs)));
    }
}
