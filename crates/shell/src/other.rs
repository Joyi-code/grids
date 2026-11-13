//! Fallbacks for non-Windows platforms: std file operations, the freedesktop
//! trash, `xdg-open` / `open`, an in-process file clipboard. No icons and no
//! native context menu.

use crate::{
    Drive, Handler, KnownFolder, KnownFolderKind, LinkTarget, MenuAction, NewItem, RgbaImage,
    classify_target, copy_tree, create_numbered, free_copy_name, is_within, new_base_name,
};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

fn spawn(cmd: &str, args: &[&std::ffi::OsStr]) -> io::Result<()> {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

pub fn open_with_system(path: &Path) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        spawn("open", &[path.as_os_str()])
    } else {
        spawn("xdg-open", &[path.as_os_str()])
    }
}

pub fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        return spawn("open", &["-R".as_ref(), path.as_os_str()]);
    }
    // FileManager1 selects the item (Nautilus, Dolphin, Nemo ...); else open the parent folder.
    let uri = format!("array:string:file://{}", path.display());
    let selected = Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
            &uri,
            "string:",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if selected {
        return Ok(());
    }
    open_with_system(path.parent().unwrap_or(path))
}

pub fn file_icon(_path: &Path, _is_dir: bool, _size: u32) -> Option<Arc<RgbaImage>> {
    None
}

pub fn context_menu(_hwnd: isize, _paths: &[PathBuf], _pos: (i32, i32)) -> MenuAction {
    MenuAction::Dismissed
}

pub fn background_menu(_hwnd: isize, _folder: &Path, _pos: (i32, i32)) -> MenuAction {
    MenuAction::Dismissed
}

// ---------- links ----------

pub fn resolve_link(lnk: &Path) -> io::Result<LinkTarget> {
    let meta = std::fs::symlink_metadata(lnk)?;
    if !meta.file_type().is_symlink() {
        return Ok(LinkTarget::Other);
    }
    let raw = std::fs::read_link(lnk)?;
    let target = match lnk.parent() {
        Some(dir) if raw.is_relative() => dir.join(&raw),
        _ => raw,
    };
    Ok(classify_target(&target.to_string_lossy()))
}

pub fn create_link(lnk: &Path, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, lnk)
    }
    #[cfg(not(unix))]
    {
        let _ = (lnk, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "links are not supported here",
        ))
    }
}

// ---------- trash ----------

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn trash_dir() -> io::Result<PathBuf> {
    if cfg!(target_os = "macos") {
        return home()
            .map(|h| h.join(".Trash"))
            .ok_or_else(|| io::Error::other("no HOME"));
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local/share")));
    data.map(|d| d.join("Trash"))
        .ok_or_else(|| io::Error::other("no HOME"))
}

/// Percent-encodes a path for a .trashinfo `Path=` line.
fn trash_encode(p: &Path) -> String {
    let mut out = String::new();
    for b in p.to_string_lossy().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn now_iso() -> String {
    // UTC is fine for DeletionDate; avoids a time crate.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

fn move_across(src: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(src, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(18) => {
            copy_tree(src, to)?;
            remove(src)
        }
        Err(e) => Err(e),
    }
}

fn remove(p: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(p)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

pub fn recycle(paths: &[PathBuf]) -> io::Result<()> {
    let trash = trash_dir()?;
    let files = if cfg!(target_os = "macos") {
        trash.clone()
    } else {
        trash.join("files")
    };
    let info = trash.join("info");
    std::fs::create_dir_all(&files)?;
    if !cfg!(target_os = "macos") {
        std::fs::create_dir_all(&info)?;
    }
    for p in paths {
        let abs = std::path::absolute(p)?;
        let name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| io::Error::other("cannot trash a root"))?;
        let free = free_copy_name(&name, true, |n| {
            files.join(n).exists() || info.join(format!("{n}.trashinfo")).exists()
        });
        if !cfg!(target_os = "macos") {
            let body = format!(
                "[Trash Info]\nPath={}\nDeletionDate={}\n",
                trash_encode(&abs),
                now_iso()
            );
            std::fs::write(info.join(format!("{free}.trashinfo")), body)?;
        }
        if let Err(e) = move_across(&abs, &files.join(&free)) {
            let _ = std::fs::remove_file(info.join(format!("{free}.trashinfo")));
            return Err(e);
        }
    }
    Ok(())
}

pub fn delete(paths: &[PathBuf]) -> io::Result<()> {
    paths.iter().try_for_each(|p| remove(p))
}

fn name_of(p: &Path) -> io::Result<String> {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no file name: {}", p.display()),
            )
        })
}

pub fn copy(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    for p in paths {
        let is_dir = p.is_dir();
        if is_dir && is_within(dest, p) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot copy {} into itself", p.display()),
            ));
        }
        let name = free_copy_name(&name_of(p)?, is_dir, |n| {
            dest.join(n).symlink_metadata().is_ok()
        });
        copy_tree(p, &dest.join(name))?;
    }
    Ok(())
}

pub fn move_(paths: &[PathBuf], dest: &Path) -> io::Result<()> {
    for p in paths {
        let is_dir = p.is_dir();
        if is_dir && is_within(dest, p) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot move {} into itself", p.display()),
            ));
        }
        // never overwrite: a clash gets a free name, as a copy would
        let name = free_copy_name(&name_of(p)?, is_dir, |n| {
            dest.join(n).symlink_metadata().is_ok()
        });
        move_across(p, &dest.join(name))?;
    }
    Ok(())
}

pub fn drives() -> Vec<Drive> {
    let mut out = vec![Drive {
        path: PathBuf::from("/"),
        label: String::new(),
    }];
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let Some(mnt) = line.split_whitespace().nth(1) else {
                continue;
            };
            let mnt = mnt.replace("\\040", " ");
            if ["/media/", "/run/media/", "/mnt/"]
                .iter()
                .any(|p| mnt.starts_with(p))
            {
                let label = Path::new(&mnt)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push(Drive {
                    path: PathBuf::from(mnt),
                    label,
                });
            }
        }
    }
    out
}

/// `XDG_DESKTOP_DIR="$HOME/Desktop"` lines from ~/.config/user-dirs.dirs.
fn xdg_user_dir(key: &str, home: &Path) -> Option<PathBuf> {
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let text = std::fs::read_to_string(cfg.join("user-dirs.dirs")).ok()?;
    parse_user_dir(&text, key, home)
}

fn parse_user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with('=')))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    (path != home).then_some(path)
}

pub fn known_folders() -> Vec<KnownFolder> {
    let Some(home) = home() else {
        return Vec::new();
    };
    let mut out = vec![KnownFolder {
        kind: KnownFolderKind::Home,
        path: home.clone(),
    }];
    let wanted = [
        (KnownFolderKind::Desktop, "XDG_DESKTOP_DIR", "Desktop"),
        (KnownFolderKind::Downloads, "XDG_DOWNLOAD_DIR", "Downloads"),
        (KnownFolderKind::Documents, "XDG_DOCUMENTS_DIR", "Documents"),
        (KnownFolderKind::Pictures, "XDG_PICTURES_DIR", "Pictures"),
    ];
    for (kind, key, plain) in wanted {
        let path = xdg_user_dir(key, &home).unwrap_or_else(|| home.join(plain));
        if path.is_dir() {
            out.push(KnownFolder { kind, path });
        }
    }
    out
}

static CLIP: Mutex<Option<(Vec<PathBuf>, bool)>> = Mutex::new(None);

pub fn clipboard_set_files(paths: &[PathBuf], cut: bool) -> io::Result<()> {
    *CLIP.lock().unwrap_or_else(|e| e.into_inner()) =
        (!paths.is_empty()).then(|| (paths.to_vec(), cut));
    Ok(())
}

pub fn clipboard_get_files() -> Option<(Vec<PathBuf>, bool)> {
    CLIP.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // the nearest existing ancestor: the target may not exist yet
        let dev = |p: &Path| {
            p.ancestors()
                .find_map(|q| std::fs::metadata(q).ok())
                .map(|m| m.dev())
        };
        match (dev(a), dev(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        a.components().next() == b.components().next()
    }
}

// ---------- open with / new / terminal / zip ----------

fn unsupported(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, format!("{what}: Windows only"))
}

pub fn open_with_handlers(_path: &Path) -> Vec<Handler> {
    Vec::new()
}

pub fn open_with(_path: &Path, _handler_id: &str) -> io::Result<()> {
    Err(unsupported("open with"))
}

pub fn open_with_dialog(_hwnd: isize, _path: &Path) -> io::Result<()> {
    Err(unsupported("choose another app"))
}

pub fn properties(_hwnd: isize, _paths: &[PathBuf]) -> io::Result<()> {
    Err(unsupported("properties"))
}

const TXT_NAME: &str = "文本文档";

pub fn new_items() -> Vec<NewItem> {
    vec![NewItem {
        ext: ".txt".into(),
        name: TXT_NAME.into(),
        icon: None,
    }]
}

/// Any extension gives an empty file; `.txt` is named "新建 文本文档.txt".
pub fn create_new(dir: &Path, ext: &str) -> io::Result<PathBuf> {
    let ext = crate::norm_ext(ext)?;
    let type_name = if ext == ".txt" {
        TXT_NAME.to_string()
    } else {
        format!("{} 文件", ext.trim_start_matches('.').to_uppercase())
    };
    create_numbered(dir, &new_base_name(&type_name), &ext, &[])
}

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

pub fn open_terminal(dir: &Path) -> io::Result<()> {
    let start = |cmd: &str| {
        Command::new(cmd)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
    };
    if cfg!(target_os = "macos") {
        return spawn(
            "open",
            &["-a".as_ref(), "Terminal".as_ref(), dir.as_os_str()],
        );
    }
    let from_env = std::env::var("TERMINAL")
        .ok()
        .filter(|t| !t.trim().is_empty());
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no terminal emulator found");
    for cmd in from_env.iter().map(String::as_str).chain([
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "kitty",
        "alacritty",
        "xterm",
    ]) {
        if !cmd.contains('/') && !on_path(cmd) {
            continue;
        }
        match start(cmd) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

// ---------- http ----------

/// `curl` when installed (enough for the version check; updating the exe is
/// Windows only). Progress is reported once, at the end.
pub fn http_get(
    url: &str,
    max_bytes: usize,
    progress: &mut dyn FnMut(u64, Option<u64>),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::sync::atomic::Ordering::Relaxed;
    let https = crate::check_url(url)?;
    let mut cmd = Command::new("curl");
    cmd.args(["-sSL", "--connect-timeout", "15", "--max-time", "600"])
        .args([
            "--proto-redir",
            if https { "=https" } else { "=http,https" },
        ])
        .args(["-A", &crate::user_agent()])
        .args(["-H", crate::ACCEPT_HEADER])
        .args(["--max-filesize", &max_bytes.to_string()])
        .args(["-w", "\n%{http_code}"])
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|_| "未找到 curl，无法联网".to_string())?;
    let mut stdout = child.stdout.take().ok_or("无法读取 curl 输出")?;
    let limit = max_bytes.saturating_add(16);
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let r = (&mut stdout).take(limit as u64 + 1).read_to_end(&mut out);
        r.map(|_| out)
    });
    let status = loop {
        if cancel.load(Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => return Err("网络错误".into()),
        }
    };
    let mut out = reader
        .join()
        .map_err(|_| "网络错误".to_string())?
        .map_err(|_| "网络错误".to_string())?;
    // the last line is the status code (-w)
    let nl = out
        .iter()
        .rposition(|&b| b == b'\n')
        .ok_or("服务器响应无效")?;
    let code: u32 = String::from_utf8_lossy(out.get(nl + 1..).unwrap_or_default())
        .trim()
        .parse()
        .unwrap_or(0);
    out.truncate(nl);
    if !status.success() {
        return Err(match status.code() {
            Some(6) => "无法解析服务器地址，请检查网络".into(),
            Some(7) => "无法连接服务器".into(),
            Some(28) => "连接超时".into(),
            Some(35 | 60) => "安全连接失败（证书或 TLS 错误）".into(),
            Some(63) => crate::too_large(max_bytes),
            Some(n) => format!("网络错误（curl {n}）"),
            None => "网络错误".into(),
        });
    }
    crate::check_status(code)?;
    if out.len() > max_bytes {
        return Err(crate::too_large(max_bytes));
    }
    progress(out.len() as u64, Some(out.len() as u64));
    Ok(out)
}

// ---------- thumbnails ----------

/// Files larger than this are not decoded for a thumbnail.
const THUMB_MAX_FILE: u64 = 64 * 1024 * 1024;

pub fn thumbnail(path: &Path, size: u32) -> Option<RgbaImage> {
    let size = size.clamp(16, 1024);
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "jpe" | "gif" | "webp" | "bmp"
    ) {
        return None;
    }
    if std::fs::metadata(path).ok()?.len() > THUMB_MAX_FILE {
        return None;
    }
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    Some(img.thumbnail(size, size).to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grids_other_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copy_into_same_folder_gets_copy_names() {
        let d = scratch("copy");
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::create_dir(d.join("dir")).unwrap();
        std::fs::write(d.join("dir/x"), "x").unwrap();
        copy(&[d.join("a.txt"), d.join("dir")], &d).unwrap();
        copy(&[d.join("a.txt")], &d).unwrap();
        assert!(d.join("a - 副本.txt").is_file());
        assert!(d.join("a - 副本 (2).txt").is_file());
        assert!(d.join("dir - 副本/x").is_file());
        assert!(
            copy(&[d.join("dir")], &d.join("dir")).is_err(),
            "into itself"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn move_and_clash() {
        let d = scratch("move");
        std::fs::create_dir_all(d.join("to")).unwrap();
        std::fs::write(d.join("a.txt"), "new").unwrap();
        std::fs::write(d.join("to/a.txt"), "old").unwrap();
        move_(&[d.join("a.txt")], &d.join("to")).unwrap();
        assert!(!d.join("a.txt").exists());
        assert_eq!(std::fs::read_to_string(d.join("to/a.txt")).unwrap(), "old");
        assert_eq!(
            std::fs::read_to_string(d.join("to/a - 副本.txt")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn clipboard_round_trip() {
        clipboard_set_files(&[PathBuf::from("/tmp/x")], true).unwrap();
        assert_eq!(
            clipboard_get_files(),
            Some((vec![PathBuf::from("/tmp/x")], true))
        );
        clipboard_set_files(&[], false).unwrap();
        assert_eq!(clipboard_get_files(), None);
    }

    #[test]
    fn user_dirs_parse() {
        let home = Path::new("/home/u");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/桌面\"\nXDG_DOWNLOAD_DIR=\"/data/dl\"\nXDG_PICTURES_DIR=\"$HOME/\"\n";
        assert_eq!(
            parse_user_dir(text, "XDG_DESKTOP_DIR", home),
            Some(PathBuf::from("/home/u/桌面"))
        );
        assert_eq!(
            parse_user_dir(text, "XDG_DOWNLOAD_DIR", home),
            Some(PathBuf::from("/data/dl"))
        );
        assert_eq!(parse_user_dir(text, "XDG_PICTURES_DIR", home), None);
        assert_eq!(parse_user_dir(text, "XDG_DOCUMENTS_DIR", home), None);
    }

    #[test]
    fn symlinks_resolve() {
        let d = scratch("links");
        std::fs::create_dir(d.join("dir")).unwrap();
        std::fs::write(d.join("f.txt"), "x").unwrap();
        create_link(&d.join("to_dir"), &d.join("dir")).unwrap();
        create_link(&d.join("to_file"), Path::new("f.txt")).unwrap(); // relative
        create_link(&d.join("to_none"), &d.join("gone")).unwrap();
        assert_eq!(
            resolve_link(&d.join("to_dir")).unwrap(),
            LinkTarget::Folder(d.join("dir"))
        );
        assert_eq!(
            resolve_link(&d.join("to_file")).unwrap(),
            LinkTarget::File(d.join("f.txt"))
        );
        assert_eq!(
            resolve_link(&d.join("to_none")).unwrap(),
            LinkTarget::Missing(d.join("gone"))
        );
        assert_eq!(resolve_link(&d.join("f.txt")).unwrap(), LinkTarget::Other);
        assert!(resolve_link(&d.join("nothing")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn trash_paths() {
        assert_eq!(trash_encode(Path::new("/a b/文")), "/a%20b/%E6%96%87");
        assert_eq!(now_iso().len(), 19);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn recycle_to_xdg_trash() {
        let d = scratch("trash");
        let data = d.join("data");
        // SAFETY: tests touching XDG_DATA_HOME run in this one test only
        unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
        std::fs::write(d.join("gone.txt"), "x").unwrap();
        recycle(&[d.join("gone.txt")]).unwrap();
        assert!(!d.join("gone.txt").exists());
        if !cfg!(target_os = "macos") {
            assert!(data.join("Trash/files/gone.txt").is_file());
            let info = std::fs::read_to_string(data.join("Trash/info/gone.txt.trashinfo")).unwrap();
            assert!(info.contains("Path=/") && info.contains("DeletionDate="));
        }
        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod thumb_tests {
    use super::*;

    #[test]
    fn decodes_common_images() {
        let d = std::env::temp_dir().join(format!("grids_thumb_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let img = RgbaImage::from_fn(400, 200, |x, _| image::Rgba([(x % 256) as u8, 0, 0, 255]));
        for ext in ["png", "jpg", "bmp", "gif", "webp"] {
            let p = d.join(format!("a.{ext}"));
            let saved = if ext == "jpg" {
                image::DynamicImage::ImageRgba8(img.clone())
                    .to_rgb8()
                    .save(&p)
            } else {
                img.save(&p)
            };
            if saved.is_err() {
                continue; // no encoder for this format: decoding is what matters
            }
            let t = thumbnail(&p, 100).unwrap_or_else(|| panic!("{ext}"));
            assert_eq!(t.dimensions(), (100, 50), "{ext}");
        }
        std::fs::write(d.join("a.txt"), "x").unwrap();
        assert!(thumbnail(&d.join("a.txt"), 100).is_none());
        std::fs::write(d.join("bad.png"), "not a png").unwrap();
        assert!(thumbnail(&d.join("bad.png"), 100).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
