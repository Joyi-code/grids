//! The update check: which release is the newest, and whether it is newer than
//! this build. The one HTTP request lives in the app; this part is pure.

/// The version out of a GitHub "latest release" reply, without the leading
/// `v`: `{"tag_name":"v0.9.1", ...}` gives `0.9.1`.
pub fn parse_latest(json: &str) -> Option<String> {
    let tag = json.split("\"tag_name\":\"").nth(1)?.split('"').next()?;
    let v = tag.trim().trim_start_matches(['v', 'V']);
    let ok = !v.is_empty() && v.bytes().next().is_some_and(|b| b.is_ascii_digit());
    ok.then(|| v.to_string())
}

/// True when `latest` is a later version than `current`. Numbers compare in
/// order and a missing part counts as 0, so 0.10 beats 0.9. A pre-release
/// suffix is ignored: 1.0.0-beta is not newer than 1.0.0.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (a, b) = (parts(latest), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y { return x > y }
    }
    false
}


/// The newest release's Windows executable: where it lives, and what it must
/// hash to. `digest` is the hex SHA-256 (GitHub's `sha256:` prefix stripped).
#[derive(Clone, Debug, PartialEq)]
pub struct Release { pub version: String, pub url: String, pub digest: String, pub size: u64 }

/// Reads the first `.exe` asset out of a GitHub "latest release" reply, with
/// its digest, so an update can be downloaded and checked. None when the
/// release has no exe asset or no digest.
pub fn parse_release(json: &str) -> Option<Release> {
    let version = parse_latest(json)?;
    for chunk in json.split("{\"url\":\"").skip(1) {
        let (Some(url), Some(digest)) = (field(chunk, "browser_download_url"), field(chunk, "digest")) else { continue };
        if !url.ends_with(".exe") || !digest.starts_with("sha256:") { continue }
        let size = field(chunk, "size").and_then(|s| s.parse().ok()).unwrap_or(0);
        return Some(Release { version, url: url.to_string(), digest: digest.trim_start_matches("sha256:").to_lowercase(), size })
    }
    None
}

fn field<'a>(chunk: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":\"");
    chunk.split(&pat).nth(1)?.split('"').next()
}
fn parts(v: &str) -> Vec<u64> {
    v.split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tag() {
        let json = "{\"url\":\"x\",\"tag_name\":\"v0.9.1\",\"name\":\"v0.9.1\"}";
        assert_eq!(parse_latest(json).as_deref(), Some("0.9.1"));
        assert_eq!(parse_latest("{\"tag_name\":\"0.10.0\"}").as_deref(), Some("0.10.0"));
        assert_eq!(parse_latest("{\"tag_name\":\"nightly\"}"), None);
        assert_eq!(parse_latest("not json"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.9.1", "0.9.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.9.0", "0.9.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("", "0.9.0"));
    }
}

