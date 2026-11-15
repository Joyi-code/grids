//! Query matching for the deep search: every space-separated term must appear
//! in the name or the folder path, in any order (Everything's default style).

/// The terms of `query`, lower-cased; empty when the query is only spaces.
///
/// A Chinese phrase typed without spaces is split into two-character terms, so
/// 「设备评价」 also finds 「设备状态评价…」: Windows tokenises the same way.
pub fn terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in query.split_whitespace() {
        let t = raw.to_lowercase();
        let chars: Vec<char> = t.chars().collect();
        if chars.len() >= 4 && chars.iter().all(|c| is_cjk(*c)) {
            let mut i = 0;
            while i + 1 < chars.len() {
                out.push(chars[i..i + 2].iter().collect());
                i += 2;
            }
            if i < chars.len() { out.push(chars[i].to_string()) }
        } else {
            out.push(t);
        }
    }
    out.dedup();
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// Whether every term appears somewhere in `name` or `path` (both case-folded
/// here). An empty query matches nothing, so a fresh search box stays quiet.
pub fn matches(name: &str, path: &str, terms: &[String]) -> bool {
    if terms.is_empty() { return false }
    let name = name.to_lowercase();
    let path = path.to_lowercase();
    terms.iter().all(|t| name.contains(t) || path.contains(t))
}

/// 0..=100 for sorting: how tightly the first term sits in the name. A hit at
/// the start beats a hit in the middle, a name hit beats a path-only hit.
pub fn score(name: &str, terms: &[String]) -> u32 {
    let name = name.to_lowercase();
    let Some(first) = terms.first() else { return 0 };
    if name == *first { return 100 }
    if name.starts_with(first.as_str()) { return 90 }
    if name.contains(first.as_str()) { return 70 }
    40
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_without_spaces_is_split() {
        let t = terms("设备评价");
        assert_eq!(t, vec!["设备", "评价"]);
        let t = terms("设备评价"); 
        assert!(matches("设备状态评价和异常分析v1.2.docx", "", &t));
        assert!(matches("配网设备运行状态综合评价页面.png", "", &t));
        assert!(!matches("设备台账.xlsx", "", &t));
    }

    #[test]
    fn every_term_must_hit() {
        let t = terms("设备 评价");
        assert!(matches("设备状态评价和异常分析v1.2.docx", "E:\\电网\\01", &t));
        assert!(matches("配网设备运行状态综合评价页面.png", "E:\\电网", &t));
        assert!(!matches("会议纪要.docx", "E:\\电网", &t));
        assert!(!matches("设备台账.xlsx", "E:\\电网", &t));
    }

    #[test]
    fn terms_also_look_at_the_folder_path() {
        let t = terms("张家口 记录");
        assert!(matches("记录.docx", "E:\\电网\\01-张家口", &t));
        assert!(!matches("记录.docx", "E:\\电网\\01-张家口", &terms("张家口 评价")));
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert!(terms("   ").is_empty());
        assert!(!matches("a.txt", "E:\\", &terms("")));
    }

    #[test]
    fn score_prefers_tight_name_hits() {
        let t = terms("评价");
        assert_eq!(score("评价", &t), 100);
        assert_eq!(score("评价.txt", &t), 90);
        assert_eq!(score("评价报告.txt", &t), 90);
        assert_eq!(score("年度评价表.xlsx", &t), 70);
        assert_eq!(score("其他.docx", &t), 40);
    }
}



