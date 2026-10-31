use std::collections::{HashMap, HashSet};
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
        let mut zip = open_zip(path)?;
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

/// Open a ZIP archive file; None for unreadable files or invalid archives.
pub(crate) fn open_zip(path: &Path) -> Option<zip::ZipArchive<std::fs::File>> {
    zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()
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
    let mut zip = open_zip(path)?;
    let mut entry = zip.by_name(name).ok()?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// CP437 code points for bytes 0x80..=0xFF, as Python's bytes(range(128, 256)).decode('cp437').
/// zip 0.6 decoded names without the UTF-8 flag as CP437, while zip 9 prefers any valid UTF-8,
/// so page names cached by older builds can differ from the names zip 9 reports.
const CP437_HIGH: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}',
    '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{00ec}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}',
    '\u{00ff}', '\u{00d6}', '\u{00dc}', '\u{00a2}', '\u{00a3}', '\u{00a5}', '\u{20a7}', '\u{0192}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{00aa}', '\u{00ba}',
    '\u{00bf}', '\u{2310}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{03b1}', '\u{00df}', '\u{0393}', '\u{03c0}', '\u{03a3}', '\u{03c3}', '\u{00b5}', '\u{03c4}',
    '\u{03a6}', '\u{0398}', '\u{03a9}', '\u{03b4}', '\u{221e}', '\u{03c6}', '\u{03b5}', '\u{2229}',
    '\u{2261}', '\u{00b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00f7}', '\u{2248}',
    '\u{00b0}', '\u{2219}', '\u{00b7}', '\u{221a}', '\u{207f}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

fn legacy_zip_name(raw: &[u8], utf8_flag: bool) -> String {
    if utf8_flag { return String::from_utf8_lossy(raw).into_owned(); }
    raw.iter().map(|&byte| {
        if byte < 128 { char::from(byte) } else { CP437_HIGH[usize::from(byte - 128)] }
    }).collect()
}

enum CachedNameMapping { Missing, Ambiguous, Resolved(Vec<usize>) }

fn index_zip_name(names: &mut HashMap<String, Option<usize>>, name: String, index: usize) {
    names.entry(name).and_modify(|entry| *entry = None).or_insert(Some(index));
}

/// A cached subset is allowed, but every selected label must have one distinct physical entry.
fn map_cached_names(index: &HashMap<String, Option<usize>>, names: &[String]) -> CachedNameMapping {
    let mut selected = Vec::with_capacity(names.len());
    let mut used = HashSet::new();
    let mut missing = false;
    let mut ambiguous = false;
    for name in names {
        let entry = match index.get(name) {
            None => { missing = true; continue; }
            Some(None) => { ambiguous = true; continue; }
            Some(Some(entry)) => *entry,
        };
        if !used.insert(entry) { ambiguous = true; continue; }
        selected.push(entry);
    }
    if missing { CachedNameMapping::Missing }
    else if ambiguous { CachedNameMapping::Ambiguous }
    else { CachedNameMapping::Resolved(selected) }
}

/// Read a persisted comic page in its saved order without changing metadata or progress.
/// Both complete name interpretations must agree, or exactly one must resolve uniquely.
/// Legacy recovery holds on duplicate central-directory names because zip 9 exposes only
/// their global presence. Canonical reads retain the existing ZIP duplicate behavior.
pub fn read_cached_page(path: &Path, names: &[String], page: usize) -> Option<Vec<u8>> {
    let name = names.get(page)?;
    if path.is_dir() || is_rar(path) { return read_zip_entry(path, name); }
    let mut zip = open_zip(path)?;
    let mut current_names = HashMap::new();
    let mut legacy_names = HashMap::new();
    for index in 0..zip.len() {
        let entry = zip.by_index_data(index).ok()?;
        if entry.is_dir() { continue; }
        let current = entry.name().ok()?.into_owned();
        if !image_entry(&current) { continue; }
        let legacy = legacy_zip_name(entry.name_raw(), entry.flags().as_u16() & (1 << 11) != 0);
        index_zip_name(&mut current_names, current, index);
        index_zip_name(&mut legacy_names, legacy, index);
    }
    use CachedNameMapping::{Ambiguous, Missing, Resolved};
    let selected = match (map_cached_names(&current_names, names), map_cached_names(&legacy_names, names)) {
        (Ambiguous, _) | (_, Ambiguous) => return None,
        (Resolved(current), Resolved(legacy)) if current == legacy => current,
        (Resolved(current), Missing) => current,
        (Missing, Resolved(legacy)) if !zip.has_duplicate_names() => legacy,
        _ => return None,
    };
    // A resolved mapping holds one entry per cached name, and `page` indexed `names` above.
    let mut entry = zip.by_index(selected[page]).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
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
