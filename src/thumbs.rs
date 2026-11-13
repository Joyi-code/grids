//! Thumbnails for the icon views. Pictures, videos and PDFs get their real
//! thumbnail (the shell's, or a decoded picture off Windows); everything
//! else the shell's big icon, or the app's own glyph when there is none.
//!
//! Requests come from what each pane draws: a pane hands over the keys it
//! shows on every frame, and a worker skips a request whose item has since
//! scrolled out of every pane, so a fast scroll does not queue up hundreds of
//! decodes. Newest requests go first. Results sit in a memory cache keyed by
//! path, modification time and pixel size, dropping the least recently used
//! once it holds more than [`BUDGET`] bytes; dropped images are also released
//! from the GPU atlas (see [`take_evicted`]).

use crate::state::render_image;
use gpui::{App, EntityId, Global, RenderImage};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// (path, modified, edge in device px).
pub type Key = (PathBuf, Option<i64>, u32);

const BUDGET: usize = 96 << 20;
const WORKERS: usize = 3;

struct Slot {
    image: Option<Arc<RenderImage>>,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Thumbs {
    cache: HashMap<Key, Slot>,
    bytes: usize,
    tick: u64,
    queue: Vec<Key>,
    pending: HashSet<Key>,
    wanted: HashMap<EntityId, HashSet<Key>>,
    workers: usize,
    evicted: Vec<Arc<RenderImage>>,
}

impl Global for Thumbs {}

fn thumbs(cx: &mut App) -> &mut Thumbs {
    if !cx.has_global::<Thumbs>() { cx.set_global(Thumbs::default()) }
    cx.global_mut::<Thumbs>()
}

/// The keys `pane` shows now (replacing what it showed before); empty when
/// it shows none, e.g. in the details view.
pub fn set_visible(pane: EntityId, keys: HashSet<Key>, cx: &mut App) {
    let t = thumbs(cx);
    if keys.is_empty() { t.wanted.remove(&pane); } else { t.wanted.insert(pane, keys); }
}

/// The thumbnail for `key`: None while it loads (draw the fallback),
/// Some(None) when there is none (draw the fallback for good).
pub fn get(key: &Key, cx: &mut App) -> Option<Option<Arc<RenderImage>>> {
    let t = thumbs(cx);
    t.tick += 1;
    let tick = t.tick;
    if let Some(s) = t.cache.get_mut(key) {
        s.used = tick;
        return Some(s.image.clone());
    }
    if t.pending.insert(key.clone()) {
        t.queue.push(key.clone());
        if t.workers < WORKERS {
            t.workers += 1;
            spawn_worker(cx);
        }
    }
    None
}

/// Images dropped from the cache since the last call, to release from the
/// window's atlas (`Window::drop_image`).
pub fn take_evicted(cx: &mut App) -> Vec<Arc<RenderImage>> {
    if !cx.has_global::<Thumbs>() { return Vec::new() }
    std::mem::take(&mut cx.global_mut::<Thumbs>().evicted)
}

fn spawn_worker(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            // The newest request still on screen somewhere; stale ones go.
            let next = cx.update(|cx| {
                let t = cx.global_mut::<Thumbs>();
                while let Some(k) = t.queue.pop() {
                    if t.wanted.values().any(|w| w.contains(&k)) { return Some(k) }
                    t.pending.remove(&k);
                }
                t.workers -= 1;
                None
            });
            let Ok(Some(key)) = next else { break };
            let (path, size) = (key.0.clone(), key.2);
            let image = cx.background_executor().spawn(async move {
                grids_shell::thumbnail(&path, size).map(|rgba| {
                    let bytes = rgba.width() as usize * rgba.height() as usize * 4;
                    (render_image(&rgba), bytes)
                })
            }).await;
            let done = cx.update(|cx| {
                let t = cx.global_mut::<Thumbs>();
                t.pending.remove(&key);
                t.tick += 1;
                let (image, bytes) = match image { Some((i, b)) => (Some(i), b), None => (None, 0) };
                t.bytes += bytes;
                t.cache.insert(key, Slot { image, bytes, used: t.tick });
                evict(t);
                cx.refresh_windows();
            });
            if done.is_err() { break }
        }
    }).detach();
}

fn evict(t: &mut Thumbs) {
    while t.bytes > BUDGET && t.cache.len() > 1 {
        let Some(old) = t.cache.iter().min_by_key(|(_, s)| s.used).map(|(k, _)| k.clone()) else { break };
        if let Some(s) = t.cache.remove(&old) {
            t.bytes -= s.bytes;
            t.evicted.extend(s.image);
        }
    }
}
