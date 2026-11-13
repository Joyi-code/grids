//! Display strings: sizes, dates, type names. Chinese first, English kept.

use crate::listing::{Entry, FileKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Zh,
    En,
}

/// Picks the string for `lang`.
pub fn tr(lang: Lang, zh: &'static str, en: &'static str) -> &'static str {
    match lang { Lang::Zh => zh, Lang::En => en }
}

/// "0 字节", "512 字节", "1.5 KB", "12.3 MB", "2.00 GB".
pub fn size(bytes: u64, lang: Lang) -> String {
    const U: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return match lang { Lang::Zh => format!("{bytes} 字节"), Lang::En => format!("{bytes} B") };
    }
    let mut v = bytes as f64 / 1024.;
    let mut i = 0;
    while v >= 1024. && i + 1 < U.len() {
        v /= 1024.;
        i += 1;
    }
    if v >= 100. { format!("{v:.0} {}", U[i]) } else if v >= 10. { format!("{v:.1} {}", U[i]) } else { format!("{v:.2} {}", U[i]) }
}

/// Civil date and time (y, m, d, h, min) of `secs` since the epoch, shifted
/// by `offset` seconds (the local UTC offset).
pub fn civil(secs: i64, offset: i64) -> (i64, u32, u32, u32, u32) {
    let t = secs + offset;
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    // Howard Hinnant's days-from-civil, inverted.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, (rem / 3600) as u32, ((rem % 3600) / 60) as u32)
}

/// "2026/11/13 14:03", as Explorer shows modification times.
pub fn date(secs: Option<i64>, offset: i64) -> String {
    match secs {
        Some(s) => {
            let (y, m, d, h, mi) = civil(s, offset);
            format!("{y}/{m}/{d} {h:02}:{mi:02}")
        }
        None => String::new(),
    }
}

/// The type column: "文件夹", "Word 文档", "PNG 图片", "ZIP 压缩文件", "LOG 文件".
pub fn type_name(e: &Entry, lang: Lang) -> String {
    if e.is_dir { return tr(lang, "文件夹", "Folder").into() }
    let up = e.ext.to_uppercase();
    let named = match e.ext.as_str() {
        "doc" | "docx" => Some(("Word 文档", "Word Document")),
        "xls" | "xlsx" | "xlsm" => Some(("Excel 工作表", "Excel Worksheet")),
        "ppt" | "pptx" => Some(("PowerPoint 演示文稿", "PowerPoint Presentation")),
        "wps" => Some(("WPS 文字文件", "WPS Writer Document")),
        "et" => Some(("WPS 表格文件", "WPS Spreadsheet")),
        "dps" => Some(("WPS 演示文件", "WPS Presentation")),
        "pdf" => Some(("PDF 文件", "PDF Document")),
        "txt" => Some(("文本文档", "Text Document")),
        "exe" => Some(("应用程序", "Application")),
        "lnk" => Some(("快捷方式", "Shortcut")),
        "dll" => Some(("应用程序扩展", "Application Extension")),
        _ => None,
    };
    if let Some((zh, en)) = named { return tr(lang, zh, en).into() }
    if up.is_empty() { return tr(lang, "文件", "File").into() }
    let suffix = match (e.kind(), lang) {
        (FileKind::Image, Lang::Zh) => "图片",
        (FileKind::Video, Lang::Zh) => "视频",
        (FileKind::Audio, Lang::Zh) => "音频",
        (FileKind::Archive, Lang::Zh) => "压缩文件",
        (FileKind::Image, Lang::En) => "Image",
        (FileKind::Video, Lang::En) => "Video",
        (FileKind::Audio, Lang::En) => "Audio",
        (FileKind::Archive, Lang::En) => "Archive",
        (_, Lang::Zh) => "文件",
        (_, Lang::En) => "File",
    };
    format!("{up} {suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizes() {
        assert_eq!(size(0, Lang::Zh), "0 字节");
        assert_eq!(size(1536, Lang::En), "1.50 KB");
        assert_eq!(size(15 * 1024 * 1024, Lang::En), "15.0 MB");
        assert_eq!(size(300 * 1024 * 1024 * 1024, Lang::En), "300 GB");
    }
    #[test]
    fn dates() {
        assert_eq!(civil(0, 0), (1970, 1, 1, 0, 0));
        // 2026-11-13 06:00 UTC, shown in UTC+8.
        assert_eq!(date(Some(1_794_549_600), 8 * 3600), "2026/11/13 14:00");
        assert_eq!(civil(951_782_400, 0), (2000, 2, 29, 0, 0));
    }
    #[test]
    fn types() {
        let mk = |n: &str| Entry { name: n.into(), path: n.into(), is_dir: false, size: 0, modified: None, hidden: false, ext: crate::listing::ext_of(n) };
        assert_eq!(type_name(&mk("a.docx"), Lang::Zh), "Word 文档");
        assert_eq!(type_name(&mk("a.png"), Lang::Zh), "PNG 图片");
        assert_eq!(type_name(&mk("a.log"), Lang::En), "LOG File");
        assert_eq!(type_name(&mk("Makefile"), Lang::Zh), "文件");
    }
}
