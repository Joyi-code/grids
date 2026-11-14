//! Downloading and swapping in a new exe. The window downloads beside the
//! current exe, then a copy of this exe finishes the swap once it exits.

use std::path::{Path, PathBuf};

/// `--apply-update <downloaded> <target exe>`: wait for the old process to let
/// go of `target`, put the new file in its place, start it. Some exit code.
pub fn apply_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--apply-update")?;
    let new = PathBuf::from(args.get(i + 1)?);
    let target = PathBuf::from(args.get(i + 2)?);
    Some(swap(&new, &target))
}

fn swap(new: &Path, target: &Path) -> i32 {
    if !swap_files(new, target) { return 1 }
    let _ = std::process::Command::new(target).spawn();
    0
}

/// Replaces only `target`: the settings, the journal and the theme folders
/// beside it are never opened, renamed or deleted. The previous exe stays as
/// `<target>.old.exe` until the next start clears it. False leaves the disk as
/// it was.
fn swap_files(new: &Path, target: &Path) -> bool {
    let old = target.with_extension("old.exe");
    let _ = std::fs::remove_file(&old);
    let mut moved = false;
    for _ in 0..150 {
        if std::fs::rename(target, &old).is_ok() { moved = true; break }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    if !moved { return false }
    if std::fs::rename(new, target).is_err() {
        let _ = std::fs::rename(&old, target); // put the old one back
        return false;
    }
    true
}

/// Copies this exe to the temp folder and starts the copy with `--apply-update`,
/// so it can replace the original while this process exits.
pub fn start_helper(new: &Path, target: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let helper = std::env::temp_dir().join("grids-update-helper.exe");
    if std::fs::copy(&exe, &helper).is_err() { return false }
    std::process::Command::new(&helper).arg("--apply-update").arg(new).arg(target).spawn().is_ok()
}

/// Downloads `url` to `dest` with the system's curl.exe. False on any failure.
pub fn download(url: &str, dest: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("curl.exe")
            .args(["-sS", "-L", "--fail", "--max-time", "300", "-o"])
            .arg(dest).arg(url)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    { let _ = (url, dest); false }
}

/// True when `path` hashes to the expected hex digest (empty: not checked).
pub fn verify(path: &Path, digest: &str) -> bool {
    if digest.is_empty() { return true }
    let Ok(bytes) = std::fs::read(path) else { return false };
    grids_core::sha256::hex(&bytes).eq_ignore_ascii_case(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_touches_only_the_exe() {
        let dir = std::env::temp_dir().join(format!("grids-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("themes").join("cream-cat")).unwrap();
        let exe = dir.join("grids.exe");
        let new = dir.join("grids-update-0.9.5.exe");
        std::fs::write(&exe, b"old binary").unwrap();
        std::fs::write(&new, b"new binary").unwrap();
        std::fs::write(dir.join("grids-gpui.json"), b"{\"locked\":[\"D:\\keep\"]}").unwrap();
        std::fs::write(dir.join("grids-journal.jsonl"), b"{\"id\":1}\n").unwrap();
        std::fs::write(dir.join("themes").join("cream-cat").join("theme.json"), b"{\"name\":\"x\"}").unwrap();

        assert!(swap_files(&new, &exe));

        assert_eq!(std::fs::read(&exe).unwrap(), b"new binary");
        assert!(!new.exists());
        assert_eq!(std::fs::read(dir.join("grids-gpui.json")).unwrap(), b"{\"locked\":[\"D:\\keep\"]}");
        assert_eq!(std::fs::read(dir.join("grids-journal.jsonl")).unwrap(), b"{\"id\":1}\n");
        assert!(dir.join("themes").join("cream-cat").join("theme.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
