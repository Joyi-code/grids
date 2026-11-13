//! Natural, case-insensitive name order, as Explorer sorts ("file2" before
//! "file10").

use std::cmp::Ordering;

/// Compares two names: digit runs by value, other characters case-insensitively.
pub fn compare(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let n = take_digits(&mut x);
                let m = take_digits(&mut y);
                let (nt, mt) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = nt.len().cmp(&mt.len()).then_with(|| nt.cmp(mt)).then_with(|| m.len().cmp(&n.len()));
                if o != Ordering::Equal { return o }
            }
            (Some(c), Some(d)) => {
                x.next();
                y.next();
                if c == d { continue }
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal { return o }
            }
        }
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied() {
        if !c.is_ascii_digit() { break }
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_by_value() {
        let mut v = vec!["file10", "File2", "file1", "a", "file02"];
        v.sort_by(|a, b| compare(a, b));
        assert_eq!(v, ["a", "file1", "file02", "File2", "file10"]);
    }
    #[test]
    fn case_insensitive() {
        assert_eq!(compare("abc", "ABD"), Ordering::Less);
        assert_eq!(compare("报告", "报告2"), Ordering::Less);
    }
}
