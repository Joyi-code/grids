//! Subsequence fuzzy matching for the command palette: every query character
//! must appear in order; consecutive hits, word starts and an early first
//! hit score higher, gaps cost a little.

/// A match: higher `score` is better; `positions` are char indices of the
/// candidate that matched (for highlighting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: i32,
    pub positions: Vec<usize>,
}

const HIT: i32 = 16;
const CONSECUTIVE: i32 = 24;
const WORD_START: i32 = 30;
const FIRST_CHAR: i32 = 20;
const GAP: i32 = 2;
const GAP_START: i32 = 6;

/// Word starts: the first char, after a separator (space _ - . / \ :), a
/// lower→upper change ("openFile"), or a letter after a digit / CJK char.
fn word_starts(c: &[char]) -> Vec<bool> {
    (0..c.len())
        .map(|i| {
            let Some(&cur) = c.get(i) else { return false };
            let Some(&prev) = i.checked_sub(1).and_then(|p| c.get(p)) else { return true };
            matches!(prev, ' ' | '_' | '-' | '.' | '/' | '\\' | ':' | '(' | '[')
                || (prev.is_lowercase() && cur.is_uppercase())
                || (prev.is_ascii_digit() && cur.is_alphabetic())
                || (!prev.is_ascii() && prev.is_alphabetic() && cur.is_ascii_alphanumeric())
        })
        .collect()
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Scores `candidate` against `query` (case-insensitive; spaces in the query
/// are ignored). `None` when the query is not a subsequence. An empty query
/// matches everything with score 0.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).map(fold).collect();
    if q.is_empty() {
        return Some(FuzzyMatch { score: 0, positions: Vec::new() });
    }
    let raw: Vec<char> = candidate.chars().collect();
    let c: Vec<char> = raw.iter().copied().map(fold).collect();
    let (n, m) = (c.len(), q.len());
    if m > n {
        return None;
    }
    let starts = word_starts(&raw);
    const NONE: i32 = i32::MIN / 2;
    // best[i][j]: best score with q[..=i] matched and q[i] at c[j]; from[i][j]: where q[i-1] sat
    let mut best = vec![vec![NONE; n]; m];
    let mut from = vec![vec![usize::MAX; n]; m];
    for i in 0..m {
        // running max over k < j-1 of best[i-1][k] - gap(k, j)
        let mut run = NONE;
        let mut run_k = usize::MAX;
        for j in 0..n {
            if i > 0 && j >= 2 {
                // extend the gap by one for the running candidate, then consider k = j-2
                if run > NONE {
                    run -= GAP;
                }
                let k = j - 2;
                let cand = best[i - 1][k].saturating_sub(GAP_START);
                if best[i - 1][k] > NONE && cand > run {
                    run = cand;
                    run_k = k;
                }
            }
            if c[j] != q[i] {
                continue;
            }
            let bonus = HIT + if starts[j] { WORD_START } else { 0 };
            if i == 0 {
                best[0][j] = bonus + if j == 0 { FIRST_CHAR } else { 0 } - (j as i32).min(10);
                continue;
            }
            let mut s = NONE;
            let mut k_best = usize::MAX;
            if j >= 1 && best[i - 1][j - 1] > NONE {
                s = best[i - 1][j - 1] + CONSECUTIVE;
                k_best = j - 1;
            }
            if run > NONE && run > s {
                s = run;
                k_best = run_k;
            }
            if s > NONE {
                best[i][j] = s + bonus;
                from[i][j] = k_best;
            }
        }
    }
    let (mut j, score) = best[m - 1]
        .iter()
        .enumerate()
        .filter(|(_, s)| **s > NONE)
        .max_by_key(|(j, s)| (**s, std::cmp::Reverse(*j)))
        .map(|(j, s)| (j, *s))?;
    let mut positions = vec![0; m];
    for i in (0..m).rev() {
        positions[i] = j;
        if i > 0 {
            j = from[i][j];
        }
    }
    // shorter candidates win ties
    Some(FuzzyMatch {
        score: score - (n as i32 / 8),
        positions,
    })
}

/// Candidates matching `query`, best first (stable for equal scores).
pub fn rank<'a, T>(query: &str, items: &'a [T], text: impl Fn(&T) -> &str) -> Vec<(&'a T, FuzzyMatch)> {
    let mut v: Vec<(usize, &T, FuzzyMatch)> = items.iter().enumerate().filter_map(|(i, it)| fuzzy_match(query, text(it)).map(|m| (i, it, m))).collect();
    v.sort_by(|a, b| b.2.score.cmp(&a.2.score).then(a.0.cmp(&b.0)));
    v.into_iter().map(|(_, it, m)| (it, m)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(q: &str, c: &str) -> i32 {
        fuzzy_match(q, c).map(|m| m.score).unwrap_or(i32::MIN)
    }

    #[test]
    fn subsequence_required() {
        assert!(fuzzy_match("abc", "a_b_c").is_some());
        assert!(fuzzy_match("acb", "abc").is_none());
        assert!(fuzzy_match("abcd", "abc").is_none());
        assert_eq!(fuzzy_match("", "x").unwrap().score, 0);
        assert_eq!(fuzzy_match("DL", "downloads").unwrap().positions, vec![0, 4]);
        assert!(fuzzy_match("下载", "我的下载文件夹").is_some());
    }

    #[test]
    fn ranking_rules() {
        // consecutive beats scattered
        assert!(s("doc", "Documents") > s("doc", "dxoxcxxxx"));
        // word starts beat mid-word hits
        assert!(s("nt", "New Tab") > s("nt", "content"));
        assert!(s("of", "openFile") > s("of", "proof"));
        // an early hit beats a late one
        assert!(s("pro", "projects") > s("pro", "my projects"));
        // shorter wins a tie
        assert!(s("src", "src") > s("src", "src-old-backup"));
        let pos = fuzzy_match("gr", "grids/game/r").unwrap().positions;
        assert_eq!(pos, vec![0, 1]);
    }

    #[test]
    fn ranks_list() {
        let items = ["Settings", "Split pane", "New session", "Show hidden files"];
        let r: Vec<&str> = rank("sh", &items, |s| s).into_iter().map(|(s, _)| *s).collect();
        assert_eq!(r, vec!["Show hidden files"]);
        let r: Vec<&str> = rank("se", &items, |s| s).into_iter().map(|(s, _)| *s).collect();
        assert_eq!(&r[..2], &["Settings", "New session"]);
        assert_eq!(r.len(), 4);
    }
}
