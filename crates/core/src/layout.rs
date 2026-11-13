//! The grid of panes: 1–4 columns, each with its own number of panes (1–4),
//! with adjustable splits. Panes are numbered down each column, left to
//! right (2-1-2: panes 1–2 on the left, 3 in the middle, 4–5 on the right).

use serde::{Deserialize, Deserializer, Serialize};

pub const MAX: usize = 4;
/// The layout picker's presets, as panes per column: even grids first, then
/// the common uneven ones.
pub const PRESETS: [&[usize]; 13] = [
    &[1],
    &[1, 1],
    &[2, 2],
    &[2, 2, 2],
    &[3, 3, 3],
    &[3, 3, 3, 3],
    &[4, 4, 4, 4],
    &[1, 2],
    &[2, 1],
    &[1, 3],
    &[3, 1],
    &[2, 1, 2],
    &[1, 1, 2],
];
/// The smallest share a column or pane can be dragged to.
pub const MIN_SHARE: f32 = 0.06;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Layout {
    /// Panes per column (each 1–4), 1–4 columns.
    pub cols: Vec<usize>,
    /// Shares of the width per column, summing to 1.
    pub col_weights: Vec<f32>,
    /// Per column, the shares of its height per pane, summing to 1.
    pub row_weights: Vec<Vec<f32>>,
    /// Read from an old rows × cols file whose panes were numbered row by
    /// row; [`crate::session::Session::normalized`] reorders them once.
    #[serde(skip)]
    pub legacy_rows: Option<usize>,
}

impl Default for Layout {
    fn default() -> Self { Self::new(2, 2) }
}

/// What a file may hold: the current shape or the old rows × cols one.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Columns {
        cols: Vec<usize>,
        #[serde(default)]
        col_weights: Vec<f32>,
        #[serde(default)]
        row_weights: Vec<Vec<f32>>,
    },
    Grid {
        rows: usize,
        cols: usize,
        #[serde(default)]
        row_weights: Vec<f32>,
        #[serde(default)]
        col_weights: Vec<f32>,
    },
}

impl<'de> Deserialize<'de> for Layout {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Stored::deserialize(d)? {
            Stored::Columns { cols, col_weights, row_weights } => Layout { cols, col_weights, row_weights, legacy_rows: None },
            Stored::Grid { rows, cols, row_weights, col_weights } => {
                let (rows, cols) = (rows.clamp(1, MAX), cols.clamp(1, MAX));
                Layout { cols: vec![rows; cols], col_weights, row_weights: vec![row_weights; cols], legacy_rows: Some(rows) }
            }
        })
    }
}

impl Layout {
    /// An even grid: `cols` columns of `rows` panes.
    pub fn new(rows: usize, cols: usize) -> Self {
        let (rows, cols) = (rows.clamp(1, MAX), cols.clamp(1, MAX));
        Self::columns(&vec![rows; cols])
    }

    /// Panes per column; zeros are dropped (a column with no panes is no
    /// column), counts are clamped to 1–4, at most 4 columns.
    pub fn columns(cols: &[usize]) -> Self {
        let mut c: Vec<usize> = cols.iter().copied().filter(|&n| n > 0).map(|n| n.min(MAX)).take(MAX).collect();
        if c.is_empty() { c.push(1) }
        let col_weights = vec![1. / c.len() as f32; c.len()];
        let row_weights = c.iter().map(|&n| vec![1. / n as f32; n]).collect();
        Self { cols: c, col_weights, row_weights, legacy_rows: None }
    }

    /// The even preset with at least `n` panes.
    pub fn preset(n: usize) -> Self {
        let p = PRESETS[..7].iter().find(|p| p.iter().sum::<usize>() >= n).unwrap_or(&PRESETS[6]);
        Self::columns(p)
    }

    pub fn count(&self) -> usize { self.cols.iter().sum() }

    /// Same columns and pane counts (splits aside).
    pub fn same_shape(&self, cols: &[usize]) -> bool { self.cols == cols }

    /// "2-1-2".
    pub fn label(&self) -> String { label(&self.cols) }

    /// The rows × cols of an even grid, None for an uneven one.
    pub fn even(&self) -> Option<(usize, usize)> {
        let r = self.cols[0];
        self.cols.iter().all(|&n| n == r).then_some((r, self.cols.len()))
    }

    /// (column, row) of pane `i`.
    pub fn cell(&self, mut i: usize) -> (usize, usize) {
        for (c, &n) in self.cols.iter().enumerate() {
            if i < n { return (c, i) }
            i -= n;
        }
        let c = self.cols.len() - 1;
        (c, self.cols[c] - 1)
    }

    pub fn index(&self, col: usize, row: usize) -> usize { self.cols[..col].iter().sum::<usize>() + row }

    /// Repairs a layout read from a file: counts in range, weights of the
    /// right lengths, positive, summing to 1.
    pub fn normalized(mut self) -> Self {
        let legacy = self.legacy_rows;
        let fresh = Self::columns(&self.cols);
        if fresh.cols != self.cols {
            self = Self { legacy_rows: legacy, ..fresh };
            return self;
        }
        fix(&mut self.col_weights, self.cols.len());
        self.row_weights.resize(self.cols.len(), Vec::new());
        for (w, &n) in self.row_weights.iter_mut().zip(&self.cols) { fix(w, n) }
        self
    }

    /// Moves the split after column `i` by `delta` (a share of the width).
    pub fn drag_col(&mut self, i: usize, delta: f32) { drag(&mut self.col_weights, i, delta) }
    /// Moves the split after pane `i` of column `col` (a share of its height).
    pub fn drag_row(&mut self, col: usize, i: usize, delta: f32) {
        if let Some(w) = self.row_weights.get_mut(col) { drag(w, i, delta) }
    }

    pub fn equalize_cols(&mut self) { self.col_weights = vec![1. / self.cols.len() as f32; self.cols.len()] }
    pub fn equalize_rows(&mut self, col: usize) {
        if let Some(&n) = self.cols.get(col) { self.row_weights[col] = vec![1. / n as f32; n] }
    }

    /// Where the split after column `i` sits, as a share of the width.
    pub fn col_split(&self, i: usize) -> f32 { self.col_weights[..=i].iter().sum() }
    /// Where the split after pane `i` of column `col` sits, as a share of the height.
    pub fn row_split(&self, col: usize, i: usize) -> f32 { self.row_weights[col][..=i].iter().sum() }

    /// For each pane of `self`, the pane of `old` it continues: same column
    /// and row when that cell exists; panes no longer shown fill the new
    /// cells in order, so changing layout keeps as many paths as fit.
    pub fn carry_from(&self, old: &Layout) -> Vec<Option<usize>> {
        let mut out = vec![None; self.count()];
        let mut used = vec![false; old.count()];
        for (i, slot) in out.iter_mut().enumerate() {
            let (c, r) = self.cell(i);
            if c < old.cols.len() && r < old.cols[c] {
                let j = old.index(c, r);
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

    /// The pane next to `i` in a direction (dx, dy ∈ -1..=1, one of them 0):
    /// up/down within the column, left/right to the pane of the next column
    /// whose band overlaps the middle of `i` most.
    pub fn neighbour(&self, i: usize, dx: i32, dy: i32) -> Option<usize> {
        let (c, r) = self.cell(i);
        if dy != 0 {
            let r2 = r as i32 + dy;
            return (r2 >= 0 && (r2 as usize) < self.cols[c]).then(|| self.index(c, r2 as usize));
        }
        let c2 = c as i32 + dx;
        if dx == 0 || c2 < 0 || c2 as usize >= self.cols.len() { return None }
        let c2 = c2 as usize;
        let w = &self.row_weights[c];
        let mid = w[..r].iter().sum::<f32>() + w[r] / 2.;
        let mut top = 0.;
        for (r2, h) in self.row_weights[c2].iter().enumerate() {
            if mid < top + h || r2 + 1 == self.cols[c2] { return Some(self.index(c2, r2)) }
            top += h;
        }
        None
    }
}

/// "2-1-2" for panes per column.
pub fn label(cols: &[usize]) -> String {
    cols.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("-")
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
        for (p, n) in PRESETS[..7].iter().zip([1, 2, 4, 6, 9, 12, 16]) {
            assert_eq!(Layout::columns(p).count(), n);
            assert_eq!(Layout::preset(n).cols, p.to_vec());
        }
        assert_eq!(Layout::preset(5).count(), 6);
        assert_eq!(Layout::new(9, 0).count(), 4);
        assert_eq!(Layout::columns(&[2, 0, 1, 2]).cols, vec![2, 1, 2]);
        assert_eq!(Layout::columns(&[0, 0]).cols, vec![1]);
        assert_eq!(Layout::columns(&[2, 1, 2]).label(), "2-1-2");
        assert_eq!(Layout::new(3, 3).even(), Some((3, 3)));
        assert_eq!(Layout::columns(&[2, 1]).even(), None);
    }
    #[test]
    fn cells() {
        let l = Layout::columns(&[2, 1, 2]);
        assert_eq!((0..5).map(|i| l.cell(i)).collect::<Vec<_>>(), vec![(0, 0), (0, 1), (1, 0), (2, 0), (2, 1)]);
        assert_eq!(l.index(2, 1), 4);
        assert_eq!(l.neighbour(1, 1, 0), Some(2));
        assert_eq!(l.neighbour(2, 1, 0), Some(4)); // middle of the tall pane: the lower right one
        assert_eq!(l.neighbour(0, 0, 1), Some(1));
        assert_eq!(l.neighbour(0, 0, -1), None);
        assert_eq!(l.neighbour(3, -1, 0), Some(2));
    }
    #[test]
    fn drag_keeps_sum() {
        let mut l = Layout::new(2, 3);
        l.drag_col(0, 0.5);
        let s: f32 = l.col_weights.iter().sum();
        assert!((s - 1.).abs() < 1e-5);
        assert!(l.col_weights[1] >= MIN_SHARE - 1e-6);
        l.equalize_cols();
        assert!((l.col_split(0) - 1. / 3.).abs() < 1e-5);
        l.drag_row(1, 0, 0.2);
        assert!((l.row_split(1, 0) - 0.7).abs() < 1e-5);
        assert!((l.row_split(0, 0) - 0.5).abs() < 1e-5);
        l.equalize_rows(1);
        assert!((l.row_split(1, 0) - 0.5).abs() < 1e-5);
    }
    #[test]
    fn carry_keeps_paths() {
        let old = Layout::new(3, 3);
        let new = Layout::new(2, 2);
        // columns of 3 → columns of 2: the third pane of each column moves.
        assert_eq!(new.carry_from(&old), vec![Some(0), Some(1), Some(3), Some(4)]);
        let grow = Layout::new(2, 3).carry_from(&Layout::new(1, 2));
        assert_eq!(grow, vec![Some(0), None, Some(1), None, None, None]);
        // 2-2 → 2-1-2: same (column, row) stays; the spare pane fills the
        // first free cell.
        assert_eq!(Layout::columns(&[2, 1, 2]).carry_from(&Layout::new(2, 2)), vec![Some(0), Some(1), Some(2), Some(3), None]);
    }
    #[test]
    fn reads_old_grids() {
        let l: Layout = serde_json::from_str(r#"{"rows": 2, "cols": 3, "row_weights": [0.25, 0.75], "col_weights": []}"#).unwrap();
        let l = l.normalized();
        assert_eq!(l.cols, vec![2, 2, 2]);
        assert_eq!(l.legacy_rows, Some(2));
        assert_eq!(l.row_weights[2], vec![0.25, 0.75]);
        assert_eq!(l.col_weights.len(), 3);
        let back: Layout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back.legacy_rows, None);
        assert_eq!(back.cols, l.cols);
    }
    #[test]
    fn normalized_repairs() {
        let l = Layout { cols: vec![2, 9], col_weights: vec![1.], row_weights: vec![], legacy_rows: None }.normalized();
        assert_eq!(l.cols, vec![2, 4]);
        assert_eq!(l.row_weights[1], vec![0.25; 4]);
        let l = Layout { cols: vec![2, 2], col_weights: vec![2., 2.], row_weights: vec![vec![1.]], legacy_rows: None }.normalized();
        assert_eq!(l.col_weights, vec![0.5, 0.5]);
        assert_eq!(l.row_weights, vec![vec![0.5, 0.5], vec![0.5, 0.5]]);
    }
}
