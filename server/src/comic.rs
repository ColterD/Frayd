use std::io::Read;
use std::path::Path;
use std::process::Command;

/// Loose image directory (manga): list image files directly.
fn dir_pages(path: &Path) -> Option<Vec<(String, u64)>> {
    let mut pages: Vec<(String, u64)> = std::fs::read_dir(path)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().ok().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if image_entry(&name) {
                Some((name, e.metadata().ok().map_or(0, |m| m.len())))
            } else {
                None
            }
        })
        .collect();
    pages.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    Some(pages)
}

/// Detect RAR archive by magic bytes (zip handled by zip crate).
fn is_rar(path: &Path) -> bool {
    std::fs::File::open(path)
        .ok()
        .and_then(|mut f| {
            let mut m = [0u8; 6];
            f.read_exact(&mut m).ok()?;
            Some(&m == b"Rar!\x1a\x07")
        })
        .unwrap_or(false)
}

fn image_entry(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "bmp")
}

/// List image entries in a cbz (zip), cbr (rar), or loose image dir, sorted by name.
pub fn comic_pages(path: &Path) -> Option<Vec<(String, u64)>> {
    if path.is_dir() {
        return dir_pages(path);
    }
    if is_rar(path) {
        let out = Command::new("unrar")
            .args(["lb", "--"])
            .arg(path)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let mut pages: Vec<(String, u64)> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty() && image_entry(l))
            .map(|l| (l.to_string(), 0))
            .collect();
        pages.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        Some(pages)
    } else {
        let f = std::fs::File::open(path).ok()?;
        let mut zip = zip::ZipArchive::new(f).ok()?;
        let mut pages: Vec<(String, u64)> = Vec::new();
        for i in 0..zip.len() {
            let entry = zip.by_index(i).ok()?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().ok()?;
            if image_entry(&name) {
                pages.push((name.into_owned(), entry.size()));
            }
        }
        pages.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        Some(pages)
    }
}

/// Read one file from a zip/rar archive or loose dir by exact name.
pub fn read_zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    if path.is_dir() {
        return std::fs::read(path.join(name)).ok();
    }
    if is_rar(path) {
        let out = Command::new("unrar")
            .args(["p", "-inul", "--"])
            .arg(path)
            .arg(name)
            .output()
            .ok()?;
        if !out.status.success() || out.stdout.is_empty() {
            return None;
        }
        return Some(out.stdout);
    }
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Natural sort so page2 < page10.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek(), bi.peek()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                if x.is_ascii_digit() && y.is_ascii_digit() {
                    let mut an = String::new();
                    let mut bn = String::new();
                    while ai.peek().is_some_and(|c| c.is_ascii_digit()) {
                        an.push(ai.next().unwrap());
                    }
                    while bi.peek().is_some_and(|c| c.is_ascii_digit()) {
                        bn.push(bi.next().unwrap());
                    }
                    let (av, bv): (u64, u64) = (
                        an.parse().unwrap_or(u64::MAX),
                        bn.parse().unwrap_or(u64::MAX),
                    );
                    if av != bv {
                        return av.cmp(&bv);
                    }
                } else {
                    let (x, y) = (ai.next().unwrap(), bi.next().unwrap());
                    let lx = x.to_ascii_lowercase();
                    let ly = y.to_ascii_lowercase();
                    if lx != ly {
                        return lx.cmp(&ly);
                    }
                }
            }
        }
    }
}
