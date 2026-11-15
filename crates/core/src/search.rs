//! Query matching for the deep search: every space-separated term must appear
//! in the name or the folder path, in any order (Everything's default style).

/// The terms of `query`, lower-cased; empty when the query is only spaces.
pub fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(|t| t.to_lowercase()).collect()
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

