//! Recent folders ranked by use: score = visits × 0.5^(days since last / 14).

use crate::link::{CASE_INSENSITIVE, same_path_ci};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Most folders remembered.
pub const MAX_RECENT: usize = 300;
/// Days for a score to halve.
pub const HALF_LIFE_DAYS: f64 = 14.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    pub path: PathBuf,
    pub count: u32,
    /// Last visit, seconds since the Unix epoch.
    pub last: i64,
}

/// The frecency score of `r` at `now` (Unix seconds).
pub fn score(r: &Recent, now: i64) -> f64 {
    let days = (now - r.last).max(0) as f64 / 86_400.0;
    f64::from(r.count) * 0.5f64.powf(days / HALF_LIFE_DAYS)
}

/// Records a visit to `path` at `now`; past [`MAX_RECENT`] the lowest-scored
/// folder is forgotten.
pub fn visit(list: &mut Vec<Recent>, path: &Path, now: i64) {
    visit_ci(list, path, now, CASE_INSENSITIVE)
}

fn visit_ci(list: &mut Vec<Recent>, path: &Path, now: i64, ci: bool) {
    if let Some(r) = list.iter_mut().find(|r| same_path_ci(&r.path, path, ci)) {
        r.count = r.count.saturating_add(1);
        r.last = now;
        r.path = path.to_path_buf();
        return;
    }
    list.push(Recent {
        path: path.to_path_buf(),
        count: 1,
        last: now,
    });
    while list.len() > MAX_RECENT {
        let worst = list
            .iter()
            .enumerate()
            .filter(|(_, r)| !same_path_ci(&r.path, path, ci))
            .min_by(|a, b| score(a.1, now).total_cmp(&score(b.1, now)))
            .map(|(i, _)| i);
        match worst {
            Some(i) => {
                list.remove(i);
            }
            None => break,
        }
    }
}

/// `list` best first (ties: most recent first).
pub fn ranked(list: &[Recent], now: i64) -> Vec<&Recent> {
    let mut v: Vec<&Recent> = list.iter().collect();
    v.sort_by(|a, b| score(b, now).total_cmp(&score(a, now)).then(b.last.cmp(&a.last)));
    v
}

/// Drops a folder (e.g. it no longer exists).
pub fn forget(list: &mut Vec<Recent>, path: &Path) {
    list.retain(|r| !same_path_ci(&r.path, path, CASE_INSENSITIVE));
}

#[cfg(test)]
mod tests {
    use super::*;
    const DAY: i64 = 86_400;

    #[test]
    fn decay_and_rank() {
        let r = Recent { path: "/a".into(), count: 8, last: 0 };
        assert!((score(&r, 0) - 8.0).abs() < 1e-9);
        assert!((score(&r, 14 * DAY) - 4.0).abs() < 1e-9);
        assert!((score(&r, 28 * DAY) - 2.0).abs() < 1e-9);
        assert!((score(&r, -DAY) - 8.0).abs() < 1e-9);
        let mut l = Vec::new();
        for _ in 0..8 {
            visit_ci(&mut l, Path::new("/old"), 0, false);
        }
        visit_ci(&mut l, Path::new("/new"), 30 * DAY, false);
        visit_ci(&mut l, Path::new("/new"), 30 * DAY, false);
        // /old: 8 × 0.5^(30/14) ≈ 1.82 < /new: 2
        let order: Vec<&str> = ranked(&l, 30 * DAY).iter().map(|r| r.path.to_str().unwrap()).collect();
        assert_eq!(order, vec!["/new", "/old"]);
        assert_eq!(ranked(&l, 0)[0].path, PathBuf::from("/old"));
    }

    #[test]
    fn case_and_cap() {
        let mut l = Vec::new();
        visit_ci(&mut l, Path::new("/A/b"), 1, true);
        visit_ci(&mut l, Path::new("/a/B/"), 2, true);
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].count, l[0].last), (2, 2));
        for i in 0..(MAX_RECENT as i64 + 10) {
            visit_ci(&mut l, Path::new(&format!("/d{i}")), 100 + i, false);
        }
        assert_eq!(l.len(), MAX_RECENT);
        // the twice-visited folder survives; the earliest single visits went
        assert!(l.iter().any(|r| r.count == 2));
        assert!(!l.iter().any(|r| r.path == Path::new("/d0")));
        forget(&mut l, Path::new("/d300"));
        assert_eq!(l.len(), MAX_RECENT - 1);
    }
}
