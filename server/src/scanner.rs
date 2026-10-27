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
    // comic: first page is the cover
    if matches!(kind, "comic" | "manga") {
        if let Some(pages) = crate::comic::comic_pages(src) {
            if let Some(first) = pages.first() {
                if let Some(bytes) = crate::comic::read_zip_entry(src, &first.0) {
                    let _ = std::fs::write(&out, &bytes);
                    return Some(out.to_string_lossy().into_owned());
                }
            }
        }
        return None;
    }
    // ebook: cover.jpg/jpeg/png at archive root, else first image
    if matches!(kind, "ebook") {
        if let Some(bytes) = crate::comic::read_zip_entry(src, "cover.jpeg")
            .or_else(|| crate::comic::read_zip_entry(src, "cover.jpg"))
            .or_else(|| crate::comic::read_zip_entry(src, "cover.png"))
            .or_else(|| {
                // fallback: first image entry in the epub
                crate::comic::comic_pages(src)
                    .and_then(|p| p.first().and_then(|f| crate::comic::read_zip_entry(src, &f.0)))
            })
        {
            let _ = std::fs::write(&out, &bytes);
            return Some(out.to_string_lossy().into_owned());
        }
        return None;
    }
    let Some(src) = src.to_str() else { return None };
    let src_str = src;
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
            // online fallback: iTunes Search API (no key) for audio kinds
            if matches!(kind, "music" | "audiobook") {
                fetch_online_thumb(title_of(Path::new(src)), kind, &out)
            } else {
                None
            }
        }
    }
}

/// Derive search terms from filename: strip leading track numbers/extension,
/// split on hyphens into artist/title segments, plus the whole cleaned name.
fn title_of(src: &Path) -> Vec<String> {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = stem
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ')
        .to_string();
    let mut terms = vec![stem.replace(['-', '_'], " ")];
    if let Some((a, b)) = stem.split_once('-') {
        terms.push(format!("{b} {a}")); // title + artist
        terms.push(b.to_string());
    }
    terms
}

/// Fetch cover art from iTunes Search API. Best-effort, short timeout, no key.
/// Tries each candidate search term until one yields artwork.
fn fetch_online_thumb(terms: Vec<String>, kind: &str, out: &Path) -> Option<String> {
    let entity = if kind == "audiobook" { "audiobook" } else { "album" };
    for term in terms.iter().filter(|t| !t.trim().is_empty()) {
        if let Some(art) = itunes_artwork(term, entity) {
            let mut img = ureq::get(&art)
                .timeout(std::time::Duration::from_secs(8))
                .call()
                .ok()?
                .into_reader();
            let mut bytes = Vec::new();
            img.read_to_end(&mut bytes).ok()?;
            if bytes.len() < 1024 {
                continue;
            }
            std::fs::write(out, &bytes).ok()?;
            tracing::info!("online thumb ok for {term:?}");
            return Some(out.to_string_lossy().into_owned());
        }
    }
    None
}

/// One iTunes Search lookup → artwork URL at 600x600, if any result.
fn itunes_artwork(term: &str, entity: &str) -> Option<String> {
    let url = format!(
        "https://itunes.apple.com/search?term={}&entity={entity}&limit=1",
        urlencoding::encode(term),
    );
    let body: serde_json::Value = serde_json::from_str(
        &ureq::get(&url)
            .timeout(std::time::Duration::from_secs(8))
            .call()
            .ok()?
            .into_string()
            .ok()?,
    )
    .ok()?;
    Some(body["results"][0]["artworkUrl100"].as_str()?.replace("100x100", "600x600"))
}

/// Filename heuristics: "Author - Series 03 - Title" / "Author - Title" → (author, series).
/// Series number stripped from series name.
fn filename_author_series(stem: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = stem.splitn(3, " - ").map(str::trim).collect();
    match parts.as_slice() {
        [author, rest] if !author.is_empty() => {
            (Some(author.to_string()), series_from(rest))
        }
        [author, series, _title] if !author.is_empty() => {
            (Some(author.to_string()), series_from(series))
        }
        _ => (None, None),
    }
}

/// "Series Name 03" / "Series Name" → Some("Series Name")
fn series_from(s: &str) -> Option<String> {
    let trimmed = s.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
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
pub fn scan(db: &Connection, db_path: String, root: std::path::PathBuf, thumbs_dir: Option<&Path>) -> usize {
    if let Some(t) = thumbs_dir {
        let _ = std::fs::create_dir_all(t);
    }
    let mut count = 0;
    let mut seen_manga_dirs: Vec<std::path::PathBuf> = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        // manga: directory of loose images under a "manga"-ish parent → one item per dir
        if entry.file_type().is_dir() && !seen_manga_dirs.iter().any(|d| d == entry.path()) {
            let dir = entry.path();
            let name_manga = |n: Option<&std::ffi::OsStr>| n
                .and_then(|n| n.to_str())
                .map(|n| n.to_ascii_lowercase().contains("manga"))
                .unwrap_or(false);
            let parent_manga = name_manga(dir.file_name())
                || name_manga(dir.parent().and_then(|p| p.file_name()));
            if parent_manga && std::fs::read_dir(dir).map(|rd| {
                rd.filter_map(|e| e.ok())
                    .any(|e| e.file_type().map_or(false, |t| t.is_file()))
            }).unwrap_or(false) {
                seen_manga_dirs.push(dir.to_path_buf());
                count += upsert_item(db, db_path.clone(), dir, "manga", thumbs_dir);
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        // skip our own transcode outputs (also purge any mistakenly-scanned rows)
        if path.file_name().and_then(|f| f.to_str()).map(|f| f.ends_with(".browser.mp4")).unwrap_or(false) {
            let _ = db.execute("DELETE FROM media WHERE path = ?1", [path.to_string_lossy().as_ref()]);
            continue;
        }
        let Some(kind) = classify(path) else { continue };
        count += upsert_item(db, db_path.clone(), path, kind, thumbs_dir);
    }
    count
}

/// Insert/update one media row (file or manga dir) + thumbs/transcode/meta work.
fn upsert_item(
    db: &Connection,
    db_path: String,
    path: &Path,
    kind: &str,
    thumbs_dir: Option<&Path>,
) -> usize {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return 0,
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
               kind = ?1, mtime_secs = ?5, size_bytes = ?4
             WHERE media.mtime_secs < ?5",
            rusqlite::params![kind, path_str, title, meta.len() as i64, mtime],
        )
        .unwrap_or(0);

    // metadata + thumbnails: for new items
    if updated > 0 && is_new {
        let id = db.last_insert_rowid();
        let path_pb = path.to_path_buf();
        // probing is slow — off the insert path is fine here (scan is already blocking)
        let dur = ffprobe_duration(&path_str);
        let title_tag = ffprobe_tag(&path_str, "title");
        let synopsis = ffprobe_tag(&path_str, "synopsis")
            .or_else(|| ffprobe_tag(&path_str, "description"));
        // author/series: tags first, then parent-dir / filename heuristics
        let tag_author = ffprobe_tag(&path_str, "artist")
            .or_else(|| ffprobe_tag(&path_str, "album_artist"))
            .or_else(|| ffprobe_tag(&path_str, "author"));
        let tag_series = ffprobe_tag(&path_str, "album")
            .or_else(|| ffprobe_tag(&path_str, "series"));
        let (author, series) = match (tag_author, tag_series) {
            (Some(a), Some(s)) => (Some(a), Some(s)),
            (a, s) => {
                let (fa, fs) = filename_author_series(&title);
                (a.or(fa), s.or(fs))
            }
        };
        let _ = db.execute(
            "UPDATE media SET duration_secs = ?1,
               title = CASE WHEN ?2 != '' THEN ?2 ELSE title END,
               author = COALESCE(?3, author),
               series = COALESCE(?4, series),
               meta_json = json_set(COALESCE(meta_json,'{}'), '$.synopsis', ?5)
             WHERE id = ?6",
            rusqlite::params![
                dur,
                title_tag.unwrap_or_default(),
                author,
                series,
                synopsis.unwrap_or_default(),
                id
            ],
        );
        // movies/shows: background transcode if audio codec isn't browser-playable
        if matches!(kind, "movie" | "episode") {
            crate::transcode::maybe_enqueue(db_path, id, path_pb);
        }
    } else if updated == 0 {
        // existing item (no new metadata work): still needs thumbs + maybe transcode
        if let Some(thumbs) = thumbs_dir {
            let media_id: Option<i64> = db
                .query_row("SELECT id FROM media WHERE path = ?1", [&path_str], |r| r.get(0))
                .ok();
            if let Some(t) = media_id.and_then(|mid| extract_thumb(path, kind, mid, thumbs)) {
                let _ = db.execute(
                    "UPDATE media SET thumb_path = ?1 WHERE path = ?2",
                    rusqlite::params![t, path_str],
                );
            }
        }
        if matches!(kind, "movie" | "episode") {
            let has: Option<String> = db
                .query_row("SELECT transcode_path FROM media WHERE path = ?1", [&path_str], |r| r.get(0))
                .ok()
                .flatten();
            if has.is_none() {
                crate::transcode::maybe_enqueue_for_existing(db_path, path.to_path_buf());
            }
        }
    }
    updated
}

fn classify(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        // audio: parent dir "music"-ish → music, else audiobook
        "m4b" | "mp3" | "flac" | "m4a" => {
            let dir = p.parent().and_then(|d| d.file_name()).and_then(|d| d.to_str()).unwrap_or("").to_ascii_lowercase();
            if dir.contains("music") || dir.contains("song") || dir.contains("album") { "music" } else { "audiobook" }
        }
        "epub" => "ebook",
        "cbz" | "cbr" => "comic",
        "mkv" | "mp4" | "webm" => "movie",
        "opus" | "ogg" => "music",
        _ => return None,
    })
}
