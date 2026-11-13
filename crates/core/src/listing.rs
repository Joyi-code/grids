//! Reading a directory into entries (run on a background thread).

use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    /// Last modified, seconds since the Unix epoch.
    pub modified: Option<i64>,
    pub hidden: bool,
    /// Lower-case extension without the dot ("" for folders and bare names).
    pub ext: String,
}

impl Entry {
    pub fn kind(&self) -> FileKind { if self.is_dir { FileKind::Folder } else { FileKind::of_ext(&self.ext) } }
}

/// Broad file types: the fallback icon and its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Folder,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    Text,
    Word,
    Sheet,
    Slides,
    Pdf,
    App,
    Other,
}

impl FileKind {
    pub fn of_ext(ext: &str) -> FileKind {
        match ext {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tif" | "tiff" | "heic" | "avif" | "psd" | "raw" => FileKind::Image,
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg" | "ts" => FileKind::Video,
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "ape" => FileKind::Audio,
            "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "iso" | "cab" => FileKind::Archive,
            "rs" | "c" | "h" | "cpp" | "hpp" | "cs" | "java" | "js" | "mjs" | "jsx" | "tsx" | "py" | "go" | "rb" | "php" | "html" | "htm" | "css" | "json" | "xml" | "yml" | "yaml" | "toml" | "sh" | "ps1" | "bat" | "cmd" | "sql" | "kt" | "swift" | "lua" | "vue" | "ini" => FileKind::Code,
            "txt" | "md" | "log" | "rtf" | "csv" => FileKind::Text,
            "doc" | "docx" | "wps" | "wpt" | "odt" | "dot" | "dotx" => FileKind::Word,
            "xls" | "xlsx" | "et" | "ett" | "ods" | "xlsm" => FileKind::Sheet,
            "ppt" | "pptx" | "dps" | "dpt" | "odp" | "pps" | "ppsx" => FileKind::Slides,
            "pdf" | "ofd" => FileKind::Pdf,
            "exe" | "msi" | "lnk" | "appimage" | "app" | "dll" => FileKind::App,
            _ => FileKind::Other,
        }
    }
}

/// Whether the platform marks the file hidden (dot files elsewhere).
#[cfg(windows)]
fn is_hidden(_name: &str, md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x2 != 0
}
#[cfg(not(windows))]
fn is_hidden(name: &str, _md: &std::fs::Metadata) -> bool { name.starts_with('.') }

/// All entries of `dir`, hidden ones included, unsorted. Entries whose
/// metadata cannot be read are listed with what is known.
pub fn read_dir(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for de in std::fs::read_dir(dir)? {
        let Ok(de) = de else { continue };
        let name = de.file_name().to_string_lossy().into_owned();
        let path = de.path();
        // DirEntry::metadata does not follow links; a link to a folder lists as a folder.
        let md = de.metadata().ok();
        let is_link = md.as_ref().is_some_and(|m| m.file_type().is_symlink());
        let target = if is_link { std::fs::metadata(&path).ok() } else { None };
        let m = target.as_ref().or(md.as_ref());
        let is_dir = m.is_some_and(|m| m.is_dir());
        let hidden = md.as_ref().map_or(name.starts_with('.'), |m| is_hidden(&name, m));
        let size = if is_dir { 0 } else { m.map_or(0, |m| m.len()) };
        let modified = m.and_then(|m| m.modified().ok()).map(|t| match t.duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        });
        let ext = if is_dir { String::new() } else { ext_of(&name) };
        out.push(Entry { name, path, is_dir, size, modified, hidden, ext });
    }
    Ok(out)
}

/// Lower-case extension of a file name ("" when none; ".bashrc" has none).
pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_a_folder() {
        let dir = std::env::temp_dir().join(format!("grids-listing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.TXT"), b"hello").unwrap();
        let mut v = read_dir(&dir).unwrap();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].name.as_str(), v[0].size, v[0].ext.as_str(), v[0].is_dir), ("a.TXT", 5, "txt", false));
        assert!(v[1].is_dir && v[1].kind() == FileKind::Folder);
        assert!(v[0].modified.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn extensions() {
        assert_eq!(ext_of("a.tar.GZ"), "gz");
        assert_eq!(ext_of(".bashrc"), "");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(FileKind::of_ext("xlsx"), FileKind::Sheet);
    }
}
