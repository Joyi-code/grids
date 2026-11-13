//! The grid of panes: rows × columns (each 1–4) with adjustable splits.
//! Panes are numbered row by row, left to right.

use serde::{Deserialize, Serialize};

pub const MAX: usize = 4;
/// The layout picker's presets: (panes, rows, cols).
pub const PRESETS: [(usize, usize, usize); 7] = [(1, 1, 1), (2, 1, 2), (4, 2, 2), (6, 2, 3), (9, 3, 3), (12, 3, 4), (16, 4, 4)];
/// The smallest share a row or column can be dragged to.
pub const MIN_SHARE: f32 = 0.06;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub rows: usize,
    pub cols: usize,
    /// Shares of the height per row, summing to 1.
    pub row_weights: Vec<f32>,
    /// Shares of the width per column, summing to 1.
    pub col_weights: Vec<f32>,
}

impl Default for Layout {
    fn default() -> Self { Self::new(2, 2) }
}

impl Layout {
    pub fn new(rows: usize, cols: usize) -> Self {
        let (rows, cols) = (rows.clamp(1, MAX), cols.clamp(1, MAX));
        Self { rows, cols, row_weights: vec![1. / rows as f32; rows], col_weights: vec![1. / cols as f32; cols] }
    }

    /// The preset with `n` panes (the nearest larger one for other counts).
    pub fn preset(n: usize) -> Self {
        let &(_, r, c) = PRESETS.iter().find(|p| p.0 >= n).unwrap_or(&PRESETS[PRESETS.len() - 1]);
        Self::new(r, c)
    }

    pub fn count(&self) -> usize { self.rows * self.cols }

    /// (row, col) of pane `i`.
    pub fn cell(&self, i: usize) -> (usize, usize) { (i / self.cols, i % self.cols) }

    pub fn index(&self, row: usize, col: usize) -> usize { row * self.cols + col }

    /// Repairs weights read from a file: right lengths, positive, summing to 1.
    pub fn normalized(mut self) -> Self {
        self.rows = self.rows.clamp(1, MAX);
        self.cols = self.cols.clamp(1, MAX);
        fix(&mut self.row_weights, self.rows);
        fix(&mut self.col_weights, self.cols);
        self
    }

    /// Moves the split after column `i` by `delta` (a share of the width).
    pub fn drag_col(&mut self, i: usize, delta: f32) { drag(&mut self.col_weights, i, delta) }
    /// Moves the split after row `i` by `delta` (a share of the height).
    pub fn drag_row(&mut self, i: usize, delta: f32) { drag(&mut self.row_weights, i, delta) }

    pub fn equalize_cols(&mut self) { self.col_weights = vec![1. / self.cols as f32; self.cols] }
    pub fn equalize_rows(&mut self) { self.row_weights = vec![1. / self.rows as f32; self.rows] }

    /// Where the split after column `i` sits, as a share of the width.
    pub fn col_split(&self, i: usize) -> f32 { self.col_weights[..=i].iter().sum() }
    pub fn row_split(&self, i: usize) -> f32 { self.row_weights[..=i].iter().sum() }

    /// For each pane of `self`, the pane of `old` it continues: same row and
    /// column when it exists; panes no longer shown are appended after the
    /// kept ones in order, so changing layout keeps as many paths as fit.
    pub fn carry_from(&self, old: &Layout) -> Vec<Option<usize>> {
        let mut out = vec![None; self.count()];
        let mut used = vec![false; old.count()];
        for (i, slot) in out.iter_mut().enumerate() {
            let (r, c) = self.cell(i);
            if r < old.rows && c < old.cols {
                let j = old.index(r, c);
                *slot = Some(j);
                used[j] = true;
            }
        }
        let mut spare = (0..old.count()).filter(|&j| !used[j]);
        for slot in out.iter_mut().filter(|s| s.is_none()) {
            *slot = spare.next();
        }
        out
    }
}

fn fix(w: &mut Vec<f32>, n: usize) {
    if w.len() != n || w.iter().any(|x| !x.is_finite() || *x <= 0.) {
        *w = vec![1. / n as f32; n];
        return;
    }
    let s: f32 = w.iter().sum();
    w.iter_mut().for_each(|x| *x /= s);
}

fn drag(w: &mut [f32], i: usize, delta: f32) {
    if i + 1 >= w.len() { return }
    let pair = w[i] + w[i + 1];
    let a = (w[i] + delta).clamp(MIN_SHARE, pair - MIN_SHARE);
    w[i] = a;
    w[i + 1] = pair - a;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets() {
        for (n, r, c) in PRESETS {
            let l = Layout::preset(n);
            assert_eq!((l.rows, l.cols, l.count()), (r, c, n));
        }
        assert_eq!(Layout::preset(5).count(), 6);
        assert_eq!(Layout::new(9, 0).count(), 4);
    }
    #[test]
    fn drag_keeps_sum() {
        let mut l = Layout::new(1, 3);
        l.drag_col(0, 0.5);
        let s: f32 = l.col_weights.iter().sum();
        assert!((s - 1.).abs() < 1e-5);
        assert!(l.col_weights[1] >= MIN_SHARE - 1e-6);
        l.equalize_cols();
        assert!((l.col_split(0) - 1. / 3.).abs() < 1e-5);
    }
    #[test]
    fn carry_keeps_paths() {
        let old = Layout::new(3, 3);
        let new = Layout::new(2, 2);
        assert_eq!(new.carry_from(&old), vec![Some(0), Some(1), Some(3), Some(4)]);
        let grow = Layout::new(2, 3).carry_from(&Layout::new(1, 2));
        assert_eq!(grow, vec![Some(0), Some(1), None, None, None, None]);
        // 1x4 → 2x2: the two panes off the first row move down.
        assert_eq!(Layout::new(2, 2).carry_from(&Layout::new(1, 4)), vec![Some(0), Some(1), Some(2), Some(3)]);
    }
    #[test]
    fn normalized_repairs() {
        let l = Layout { rows: 2, cols: 2, row_weights: vec![1.], col_weights: vec![2., 2.] }.normalized();
        assert_eq!(l.row_weights, vec![0.5, 0.5]);
        assert_eq!(l.col_weights, vec![0.5, 0.5]);
    }
}
