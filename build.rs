//! Embeds the app icon in grids.exe (Windows targets only) and records the
//! build for the version popover: GRIDS_GIT_HASH (short commit, "unknown"
//! outside a checkout) and GRIDS_BUILD_DATE (UTC, YYYY-MM-DD; honours
//! SOURCE_DATE_EPOCH for reproducible builds).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon/grids.rc");
    println!("cargo:rerun-if-changed=assets/icon/grids.ico");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    // A new commit moves HEAD (or the branch it points at).
    for f in [".git/HEAD", ".git/refs/heads", ".git/packed-refs"] {
        if std::path::Path::new(f).exists() { println!("cargo:rerun-if-changed={f}") }
    }
    let hash = std::process::Command::new("git").args(["rev-parse", "--short=7", "HEAD"]).output().ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=GRIDS_GIT_HASH={hash}");
    let secs = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    println!("cargo:rustc-env=GRIDS_BUILD_DATE={y:04}-{m:02}-{d:02}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/icon/grids.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("grids.rc: resource compile failed");
    }

    bundle_theme();
}

/// Emits `bundled_theme.rs`: every file of `themes/cream-cat`, embedded so the
/// app can lay the theme down on first run and ship as one exe.
fn bundle_theme() {
    use std::path::Path;
    let root = Path::new("themes/cream-cat");
    println!("cargo:rerun-if-changed=themes/cream-cat");
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out)
            } else if let Ok(rel) = p.strip_prefix(base) {
                out.push((rel.to_string_lossy().replace('\\', "/"), p))
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    let mut src = String::from("/// The bundled theme's files, written out on first run.\npub static THEME_FILES: &[(&str, &[u8])] = &[\n");
    for (rel, p) in &files {
        let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        src.push_str(&format!("    ({:?}, include_bytes!({:?})),\n", rel, abs.to_string_lossy()));
    }
    src.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("bundled_theme.rs");
    std::fs::write(out, src).expect("writing bundled_theme.rs");
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian
/// (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
