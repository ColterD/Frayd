use rusqlite::Connection;
use std::path::Path;
use walkdir::WalkDir;

#[allow(dead_code)] // used by ad-hoc scans/tests
pub fn open(path: &str) -> Connection {
    let db = Connection::open(path).expect("open db");
    db.execute_batch(include_str!("../schema.sql")).expect("schema");
    db
}

/// Incremental scan of a library root. Returns number of new/updated items.
pub fn scan(db: &Connection, root: std::path::PathBuf) -> usize {
    let mut count = 0;
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(kind) = classify(path) else { continue };
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let path_str = path.to_string_lossy();
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let updated = db
            .execute(
                "INSERT INTO media (kind, path, title, size_bytes, mtime_secs)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(path) DO UPDATE SET
                   mtime_secs = ?5, size_bytes = ?4
                 WHERE media.mtime_secs < ?5",
                rusqlite::params![kind, path_str, title, meta.len() as i64, mtime],
            )
            .unwrap_or(0);
        count += updated;
    }
    count
}

fn classify(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "m4b" | "mp3" | "flac" | "m4a" => "audiobook", // refine: music vs audiobook by dir later
        "epub" => "ebook",
        "cbz" | "cbr" => "comic",
        "mkv" | "mp4" | "webm" => "movie",
        "opus" | "ogg" => "music",
        _ => return None,
    })
}
