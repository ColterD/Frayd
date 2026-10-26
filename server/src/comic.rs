use std::io::Read;
use std::path::Path;

/// List image entries in a cbz (zip), sorted by name. Returns (name, size).
pub fn comic_pages(path: &Path) -> Option<Vec<(String, u64)>> {
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;
    let mut pages: Vec<(String, u64)> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).ok()?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "bmp") {
            pages.push((name, entry.size()));
        }
    }
    pages.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    Some(pages)
}

/// Read one file from a zip archive by exact name.
pub fn read_zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
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
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(&x), Some(&y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let an = take_number(&mut ai);
                let bn = take_number(&mut bi);
                if an != bn {
                    return an.cmp(&bn);
                }
            }
            (Some(&x), Some(&y)) => {
                ai.next();
                bi.next();
                let lx = x.to_ascii_lowercase();
                let ly = y.to_ascii_lowercase();
                if lx != ly {
                    return lx.cmp(&ly);
                }
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars>) -> u64 {
    let mut n: u64 = 0;
    while let Some(&c) = it.peek() {
        if let Some(d) = c.to_digit(10) {
            n = n.saturating_mul(10).saturating_add(d as u64);
            it.next();
        } else {
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["p10.jpg", "p2.jpg", "p1.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["p1.jpg", "p2.jpg", "p10.jpg"]);
    }
}
