use rusqlite::Connection;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

/// Extract a poster/cover for one media file into thumbs_dir. Best-effort:
/// failure is non-fatal (item just has no thumbnail). Returns thumb path.
fn extract_thumb(src: &Path, kind: &str, media_id: i64, thumbs_dir: &Path) -> Option<String> {
    let out = thumbs_dir.join(format!("{media_id}.jpg"));
    if out.exists() {
        return Some(out.to_string_lossy().into_owned());
    }
    let src = src.to_str()?;
    let status = if matches!(kind, "movie" | "episode") {
        // poster frame at 10% in (seeking to 0 often hits black/logos)
        let dur = ffprobe_duration(src)?;
        let at = (dur * 0.1).max(1.0);
        Command::new("ffmpeg")
            .args(["-y", "-ss", &format!("{at:.1}"), "-i", src, "-frames:v", "1",
                   "-vf", "scale=480:-2", "-q:v", "4"])
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    } else {
        // audio: embedded cover art
        Command::new("ffmpeg")
            .args(["-y", "-i", src, "-an", "-map", "0:v:0", "-frames:v", "1",
                   "-vf", "scale=480:-2", "-q:v", "4"])
            .arg(&out)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    };
    match status {
        Ok(s) if s.success() && out.exists() => Some(out.to_string_lossy().into_owned()),
        _ => {
            let _ = std::fs::remove_file(&out); // no partial files
            None
        }
    }
}

/// Read one format-level metadata tag (title, synopsis, description, ...).
fn ffprobe_tag(src: &str, tag: &str) -> Option<String> {
    let out = Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries", &format!("format_tags={tag}"),
               "-of", "csv=p=0", src])
        .output()
        .ok()?;
    let s = std::str::from_utf8(&out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn ffprobe_duration(src: &str) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries", "format=duration",
               "-of", "csv=p=0", src])
        .output()
        .ok()?;
    std::str::from_utf8(&out.stdout).ok()?.trim().parse().ok()
}

#[allow(dead_code)] // used by ad-hoc scans/tests
pub fn open(path: &str) -> Connection {
    let db = Connection::open(path).expect("open db");
    db.execute_batch(include_str!("../schema.sql")).expect("schema");
    db
}

/// Incremental scan of a library root. Returns number of new/updated items.
pub fn scan(db: &Connection, root: std::path::PathBuf, thumbs_dir: Option<&Path>) -> usize {
    if let Some(t) = thumbs_dir {
        let _ = std::fs::create_dir_all(t);
    }
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

        let is_new: bool = db
            .query_row(
                "SELECT 1 FROM media WHERE path = ?1",
                [&path_str],
                |r| r.get::<_, i64>(0),
            )
            .is_err();
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

        // metadata + thumbnails: for new items
        if updated > 0 && is_new {
            let id = db.last_insert_rowid();
            let path_pb = path.to_path_buf();
            let kind_owned = kind.to_string();
            // probing is slow — off the insert path is fine here (scan is already blocking)
            let dur = ffprobe_duration(&path_str);
            let title_tag = ffprobe_tag(&path_str, "title");
            let synopsis = ffprobe_tag(&path_str, "synopsis")
                .or_else(|| ffprobe_tag(&path_str, "description"));
            let _ = db.execute(
                "UPDATE media SET duration_secs = ?1,
                   title = CASE WHEN ?2 != '' THEN ?2 ELSE title END,
                   meta_json = json_set(COALESCE(meta_json,'{}'), '$.synopsis', ?3)
                 WHERE id = ?4",
                rusqlite::params![
                    dur,
                    title_tag.unwrap_or_default(),
                    synopsis.unwrap_or_default(),
                    id
                ],
            );
            if let Some(thumbs) = &thumbs_dir {
                if let Some(t) = extract_thumb(&path_pb, &kind_owned, id, thumbs) {
                    let _ = db.execute(
                        "UPDATE media SET thumb_path = ?1 WHERE id = ?2",
                        rusqlite::params![t, id],
                    );
                }
            }
        }
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
