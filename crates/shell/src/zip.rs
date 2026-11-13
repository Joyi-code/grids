//! A small streaming ZIP writer for "压缩为 ZIP 文件": deflate (flate2,
//! already in the dependency tree through `image`), UTF-8 names (flag bit
//! 11, which Explorer and 7-Zip honour), data descriptors so each file is
//! read once. No ZIP64: an archive or entry past 4 GiB is refused instead
//! of written broken. Symlinks / junctions are skipped, so a loop cannot
//! make it run forever.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

const LIMIT: u64 = u32::MAX as u64;
const FLAGS: u16 = 0x0808; // bit 3: sizes in the data descriptor; bit 11: UTF-8 names

struct Entry {
    name: Vec<u8>,
    dir: bool,
    crc: u32,
    packed: u32,
    size: u32,
    offset: u32,
    time: u16,
    date: u16,
}

/// Counts what goes through, so offsets need no seeking.
struct Counter<W: Write> {
    inner: W,
    n: u64,
}

impl<W: Write> Write for Counter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let k = self.inner.write(buf)?;
        self.n += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn too_big() -> io::Error {
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        "archive larger than 4 GB is not supported",
    )
}

fn u32_of(n: u64) -> io::Result<u32> {
    u32::try_from(n).map_err(|_| too_big())
}

/// MS-DOS date and time (UTC; the format has no zone).
fn dos_time(meta: &std::fs::Metadata) -> (u16, u16) {
    let secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    if year < 1980 {
        return (0, (1 << 5) | 1); // 1980-01-01
    }
    let year = year.min(2107);
    let time = (((rem / 3600) << 11) | ((rem % 3600 / 60) << 5) | ((rem % 60) / 2)) as u16;
    let date = (((year - 1980) << 9) | (month << 5) | day) as u16;
    (time, date)
}

struct Zip<'a, W: Write> {
    out: Counter<W>,
    entries: Vec<Entry>,
    /// The archive itself, never added to itself.
    dest: &'a Path,
}

impl<W: Write> Zip<'_, W> {
    fn local_header(&mut self, e: &Entry, method: u16) -> io::Result<()> {
        let o = &mut self.out;
        o.write_all(&0x0403_4b50u32.to_le_bytes())?;
        o.write_all(&20u16.to_le_bytes())?; // version needed
        o.write_all(&(if e.dir { 0x0800 } else { FLAGS }).to_le_bytes())?;
        o.write_all(&method.to_le_bytes())?;
        o.write_all(&e.time.to_le_bytes())?;
        o.write_all(&e.date.to_le_bytes())?;
        o.write_all(&[0; 12])?; // crc, sizes: zero here, in the descriptor
        o.write_all(&(e.name.len() as u16).to_le_bytes())?;
        o.write_all(&0u16.to_le_bytes())?;
        o.write_all(&e.name)
    }

    fn add_dir(&mut self, name: String, meta: &std::fs::Metadata) -> io::Result<()> {
        let (time, date) = dos_time(meta);
        let e = Entry {
            name: format!("{name}/").into_bytes(),
            dir: true,
            crc: 0,
            packed: 0,
            size: 0,
            offset: u32_of(self.out.n)?,
            time,
            date,
        };
        self.local_header(&e, 0)?;
        self.entries.push(e);
        Ok(())
    }

    fn add_file(&mut self, name: String, path: &Path, meta: &std::fs::Metadata) -> io::Result<()> {
        if meta.len() >= LIMIT {
            return Err(too_big());
        }
        let (time, date) = dos_time(meta);
        let mut e = Entry {
            name: name.into_bytes(),
            dir: false,
            crc: 0,
            packed: 0,
            size: 0,
            offset: u32_of(self.out.n)?,
            time,
            date,
        };
        if e.name.len() > u16::MAX as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "name too long"));
        }
        self.local_header(&e, 8)?;
        let start = self.out.n;
        let mut src = File::open(path)?;
        let mut hasher = crc32fast::Hasher::new();
        let mut size = 0u64;
        let mut enc =
            flate2::write::DeflateEncoder::new(&mut self.out, flate2::Compression::default());
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let chunk = buf.get(..n).unwrap_or_default();
            hasher.update(chunk);
            enc.write_all(chunk)?;
            size += n as u64;
        }
        enc.finish()?;
        e.crc = hasher.finalize();
        e.size = u32_of(size)?;
        e.packed = u32_of(self.out.n - start)?;
        let o = &mut self.out;
        o.write_all(&0x0807_4b50u32.to_le_bytes())?;
        o.write_all(&e.crc.to_le_bytes())?;
        o.write_all(&e.packed.to_le_bytes())?;
        o.write_all(&e.size.to_le_bytes())?;
        self.entries.push(e);
        Ok(())
    }

    fn add_tree(&mut self, name: String, path: &Path) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() || crate::same_path(path, self.dest) {
            return Ok(());
        }
        if meta.is_dir() {
            self.add_dir(name.clone(), &meta)?;
            let mut children: Vec<_> = std::fs::read_dir(path)?.collect::<io::Result<_>>()?;
            children.sort_by_key(|c| c.file_name());
            for c in children {
                let child = format!("{name}/{}", c.file_name().to_string_lossy());
                self.add_tree(child, &c.path())?;
            }
            Ok(())
        } else {
            self.add_file(name, path, &meta)
        }
    }

    fn finish(mut self) -> io::Result<W> {
        let start = u32_of(self.out.n)?;
        for e in &self.entries {
            let o = &mut self.out;
            o.write_all(&0x0201_4b50u32.to_le_bytes())?;
            o.write_all(&20u16.to_le_bytes())?; // made by: MS-DOS, 2.0
            o.write_all(&20u16.to_le_bytes())?;
            o.write_all(&(if e.dir { 0x0800 } else { FLAGS }).to_le_bytes())?;
            o.write_all(&(if e.dir { 0u16 } else { 8 }).to_le_bytes())?;
            o.write_all(&e.time.to_le_bytes())?;
            o.write_all(&e.date.to_le_bytes())?;
            o.write_all(&e.crc.to_le_bytes())?;
            o.write_all(&e.packed.to_le_bytes())?;
            o.write_all(&e.size.to_le_bytes())?;
            o.write_all(&(e.name.len() as u16).to_le_bytes())?;
            o.write_all(&[0; 8])?; // extra, comment, disk, internal attrs
            o.write_all(&(if e.dir { 0x10u32 } else { 0x20 }).to_le_bytes())?;
            o.write_all(&e.offset.to_le_bytes())?;
            o.write_all(&e.name)?;
        }
        let size = u32_of(self.out.n - u64::from(start))?;
        let count = u16::try_from(self.entries.len())
            .map_err(|_| io::Error::other("more than 65535 entries is not supported"))?;
        let o = &mut self.out;
        o.write_all(&0x0605_4b50u32.to_le_bytes())?;
        o.write_all(&[0; 4])?;
        o.write_all(&count.to_le_bytes())?;
        o.write_all(&count.to_le_bytes())?;
        o.write_all(&size.to_le_bytes())?;
        o.write_all(&start.to_le_bytes())?;
        o.write_all(&0u16.to_le_bytes())?;
        o.flush()?;
        Ok(self.out.inner)
    }
}

/// Writes `items` (name in the archive, path on disk) to the new file `dest`.
pub(crate) fn write(dest: &Path, items: &[(String, &Path)]) -> io::Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)?;
    let mut zip = Zip {
        out: Counter {
            inner: BufWriter::new(file),
            n: 0,
        },
        entries: Vec::new(),
        dest,
    };
    for (name, path) in items {
        zip.add_tree(name.clone(), path)?;
    }
    let file = zip.finish()?.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_dates() {
        let d = std::env::temp_dir();
        let m = std::fs::metadata(&d).unwrap();
        let (_, date) = dos_time(&m);
        assert!(date >> 9 >= 44, "year before 2024: {date:#x}"); // 1980 + 44
    }
}
