use rusqlite::{Connection, OptionalExtension};
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

#[derive(Default)]
struct ProbeMetadata {
    duration: Option<f64>,
    title: Option<String>,
    synopsis: Option<String>,
    author: Option<String>,
    series: Option<String>,
}

/// Read new-item duration and decoded, trimmed tags in one process. Unusable output is optional metadata.
/// Exit status is not a parsing gate; valid JSON may still contain useful format metadata.
fn ffprobe_metadata(src: &str) -> Option<ProbeMetadata> {
    let out = Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries",
               "format=duration:format_tags=title,synopsis,description,artist,album_artist,author,album,series",
               "-of", "json", src])
        .output()
        .ok()?;
    let metadata: serde_json::Value = serde_json::from_str(std::str::from_utf8(&out.stdout).ok()?).ok()?;
    let format = metadata.get("format")?;
    let tags = format.get("tags").and_then(serde_json::Value::as_object);
    let tag = |name: &str| -> Option<String> {
        let value = tags?.iter().find(|(key, _)| key.eq_ignore_ascii_case(name))?.1.as_str()?.trim();
        if value.is_empty() { None } else { Some(value.to_string()) }
    };
    Some(ProbeMetadata {
        duration: format.get("duration").and_then(serde_json::Value::as_str).and_then(|value| value.trim().parse().ok()),
        title: tag("title"),
        synopsis: tag("synopsis").or_else(|| tag("description")),
        author: tag("artist").or_else(|| tag("album_artist")).or_else(|| tag("author")),
        series: tag("album").or_else(|| tag("series")),
    })
}

/// Return the positive media duration reported by ffprobe, or None when probing fails.
fn ffprobe_duration(src: &str) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries", "format=duration",
               "-of", "csv=p=0", src])
        .output()
        .ok()?;
    std::str::from_utf8(&out.stdout).ok()?.trim().parse().ok()
}

/// Open an initialized database for scan fixtures.
#[cfg(test)]
pub fn open(path: &str) -> Connection {
    crate::store::open(path)
}

/// Probe loose manga images without hiding directory, entry, or metadata failures.
/// Failed entries do not prevent a later accessible file from qualifying the directory.
fn manga_directory_has_files(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(operation="read_dir", path=%dir.display(), %error, "library scan directory unavailable");
            return false;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                // read_dir entry errors carry no entry path; retain the containing directory.
                tracing::warn!(operation="read_dir_entry", path=%dir.display(), %error, "library scan entry unavailable");
                continue;
            }
        };
        match entry.file_type() {
            Ok(file_type) if file_type.is_file() => return true,
            Ok(_) => (),
            Err(error) => tracing::warn!(operation="file_type", path=%entry.path().display(), %error, "library scan entry type unavailable"),
        }
    }
    false
}

/// Incremental scan of a library root. Returns number of new/updated items.
#[cfg(test)]
pub fn scan(db: &Connection, db_path: String, root: std::path::PathBuf, thumbs_dir: Option<&Path>) -> usize {
    let roots=vec![root.clone()];
    scan_with_roots(db,db_path,root,thumbs_dir,&roots)
}

/// Scan one root against the complete admitted root snapshot, preserving existing source-version refresh guards.
/// Refuse unavailable roots or either-direction canonical cache overlap before thumbnail creation,
/// artifact recovery/reconciliation or item updates. Source reads never grant source deletion authority.
pub fn scan_with_roots(db: &Connection, db_path: String, root: std::path::PathBuf, thumbs_dir: Option<&Path>, roots: &[std::path::PathBuf]) -> usize {
    if let Some(root)=roots.iter().find(|r| !r.is_dir()) {
        tracing::warn!(operation="walk",path=%root.display(),"library scan traversal failed: configured root unavailable");
        return 0;
    }
    let cache=std::env::var_os("FRAYD_TRANSCODES").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(&db_path).parent().unwrap_or(Path::new(".")).join("transcodes"));
    if crate::transcode::validate_cache(&cache,roots).is_none() {
        tracing::warn!(operation="validate_cache",cache_kind="transcode",path=%cache.display(),source_root=%root.display(),reason="unresolvable cache/root or overlap with configured roots","library scan held");
        return 0;
    }
    if let Some(t)=thumbs_dir {
        if crate::transcode::validate_cache(t,roots).is_none() {
            tracing::warn!(operation="validate_cache",cache_kind="thumbnail",path=%t.display(),source_root=%root.display(),reason="unresolvable cache/root or overlap with configured roots","library scan held");
            return 0;
        }
        if let Err(error)=std::fs::create_dir_all(t) {
            tracing::warn!(operation="create_dir_all",cache_kind="thumbnail",path=%t.display(),source_root=%root.display(),reason="thumbnail directory creation failed",%error,"library scan held");
            return 0;
        }
    }
    crate::transcode::recover_pending_jobs_with_roots(&db_path, roots);
    let owned = crate::transcode::reconcile_artifacts(db);
    let mut count = 0;
    let mut seen_manga_dirs: Vec<std::path::PathBuf> = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(operation="walk", path=%error.path().unwrap_or(&root).display(), %error, "library scan traversal failed");
                continue;
            }
        };
        // manga: directory of loose images under a "manga"-ish parent → one item per dir
        if entry.file_type().is_dir() && !seen_manga_dirs.iter().any(|d| d == entry.path()) {
            let dir = entry.path();
            let name_manga = |n: Option<&std::ffi::OsStr>| n
                .and_then(|n| n.to_str())
                .map(|n| n.to_ascii_lowercase().contains("manga"))
                .unwrap_or(false);
            let parent_manga = name_manga(dir.file_name())
                || name_manga(dir.parent().and_then(|p| p.file_name()));
            if parent_manga && manga_directory_has_files(dir) {
                seen_manga_dirs.push(dir.to_path_buf());
                count += upsert_item_with_roots(db, db_path.clone(), dir, "manga", thumbs_dir, roots);
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if owned.contains(path.to_string_lossy().as_ref()) {
            continue;
        }
        let Some(kind) = classify_with_root(path,Some(&root)) else { continue };
        count += upsert_item_with_roots(db, db_path.clone(), path, kind, thumbs_dir, roots);
    }
    count
}

/// Confirm downloaded supported files reached their current source-version rows.
/// A successful downloader exit alone does not prove indexing or database publication.
pub(crate) fn ingest_indexed(db: &Connection, directory: &Path) -> bool {
    let mut found=false;
    for entry in WalkDir::new(directory).follow_links(false) {
        let Ok(entry)=entry else { return false; };
        if !entry.file_type().is_file() || classify_with_root(entry.path(),Some(directory)).is_none() { continue; }
        let Ok(metadata)=entry.metadata() else { return false; };
        let Some(mtime)=metadata.modified().ok().and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t|t.as_secs() as i64) else { return false; };
        let indexed=db.query_row("SELECT COUNT(*) FROM media WHERE path=?1 AND size_bytes=?2 AND mtime_secs=?3",rusqlite::params![entry.path().to_string_lossy(),metadata.len() as i64,mtime],|r|r.get::<_,i64>(0));
        if !matches!(indexed,Ok(1)) { return false; }
        found=true;
    }
    found
}

/// Update scan fingerprints and reader caches; clear converted playback only for known replacements.
/// Preserve identity, user metadata, unchanged order and version-unqualified legacy playback.
#[cfg(test)]
fn upsert_item(
    db: &Connection,
    db_path: String,
    path: &Path,
    kind: &str,
    thumbs_dir: Option<&Path>,
    library_root: &Path,
) -> usize {
    upsert_item_with_roots(db,db_path,path,kind,thumbs_dir,&[library_root.to_path_buf()])
}

/// Refresh source fingerprints through the existing guarded path, or atomically change only a stale kind.
/// Known unchanged fingerprints permit no cache, duration, metadata, lease or converter work on reclassification.
fn upsert_item_with_roots(db: &Connection, db_path: String, path: &Path, kind: &str, thumbs_dir: Option<&Path>, roots: &[std::path::PathBuf]) -> usize {
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

    let refreshed=(|| -> rusqlite::Result<(usize,bool,bool)> {
        let tx=rusqlite::Transaction::new_unchecked(db,rusqlite::TransactionBehavior::Immediate)?;
        let existing=tx.query_row("SELECT kind,mtime_secs,size_bytes FROM media WHERE path=?1",[&path_str],
            |row|Ok((row.get::<_,String>(0)?,row.get::<_,Option<i64>>(1)?,row.get::<_,Option<i64>>(2)?))).optional()?;
        let is_new=existing.is_none();
        let kind_only=existing.as_ref().is_some_and(|(old_kind,old_mtime,old_size)|
            old_kind!=kind && *old_mtime==Some(mtime) && *old_size==Some(meta.len() as i64));
        if kind_only {
            let changed=tx.execute("UPDATE media SET kind=?1 WHERE path=?2 AND kind IS NOT ?1
                AND mtime_secs IS ?3 AND size_bytes IS ?4",rusqlite::params![kind,path_str,mtime,meta.len() as i64])?;
            if changed!=1 {return Err(rusqlite::Error::InvalidQuery);}
            tx.commit()?;
            return Ok((changed,false,true));
        }
        // Capture the exact prior pointer only when both fingerprint components were known.
        let prior:Option<(i64,String)>=tx.query_row("SELECT id,transcode_path FROM media WHERE path=?1
            AND mtime_secs IS NOT NULL AND size_bytes IS NOT NULL AND transcode_path IS NOT NULL
            AND (mtime_secs IS NOT ?2 OR size_bytes IS NOT ?3)",
            rusqlite::params![path_str,mtime,meta.len() as i64],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let updated = tx.execute(
            "INSERT INTO media (kind, path, title, size_bytes, mtime_secs)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
               kind = ?1, mtime_secs = ?5, size_bytes = ?4,
               duration_secs = CASE WHEN ?1 IN ('movie','episode')
                   AND media.mtime_secs IS NOT NULL AND media.size_bytes IS NOT NULL
                   THEN NULL ELSE media.duration_secs END,
               transcode_path = CASE WHEN media.mtime_secs IS NOT NULL AND media.size_bytes IS NOT NULL
                   THEN NULL ELSE media.transcode_path END,
               meta_json = CASE WHEN ?1 IN ('comic','manga') AND json_valid(media.meta_json)
                   THEN json_remove(media.meta_json,'$.pages')
                   WHEN ?1='ebook' AND json_valid(media.meta_json)
                   THEN json_remove(media.meta_json,'$.spine') ELSE media.meta_json END
             WHERE media.mtime_secs IS NOT ?5 OR media.size_bytes IS NOT ?4",
            rusqlite::params![kind, path_str, title, meta.len() as i64, mtime],
        )?;

        let lease=if let Some((id,out))=prior.filter(|_|updated==1) {
            crate::transcode::retire_replaced_job_with_roots(&tx,&db_path,roots,id,path,Path::new(&out))?
        } else { None };
        tx.commit()?;
        drop(lease);
        Ok((updated,is_new,false))
    })();
    let (updated,is_new,kind_only)=match refreshed {
        Ok(result)=>result,
        Err(error)=>{tracing::warn!(%error,"scan refresh held: source and artifact state retained");return 0;}
    };

    // Classification policy changes do not grant source-version or cache mutation authority.
    if kind_only {return updated;}

    // metadata + thumbnails: for new items
    if updated > 0 && is_new {
        let id = db.last_insert_rowid();
        let path_pb = path.to_path_buf();
        let ProbeMetadata { duration: dur, title: title_tag, synopsis, author: tag_author, series: tag_series } =
            ffprobe_metadata(&path_str).unwrap_or_default();
        // Tags take precedence over filename heuristics independently for author and series.
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
            crate::transcode::maybe_enqueue_with_roots(db_path, id, path_pb, roots.to_vec());
        }
    } else if updated > 0 && matches!(kind, "movie" | "episode") {
        // Known replacements already invalidated their old duration in the fingerprint transaction.
        // Failed probes preserve only unqualified legacy values; publish successful probes for this version.
        if let Some(duration) = ffprobe_duration(&path_str) {
            let published = (|| -> rusqlite::Result<()> {
                let tx=rusqlite::Transaction::new_unchecked(db,rusqlite::TransactionBehavior::Immediate)?;
                // Check the file after acquiring the write lock, including any time spent waiting.
                let same_version = std::fs::metadata(path).ok()
                    .filter(|current| current.len() == meta.len())
                    .and_then(|current| current.modified().ok())
                    .is_some_and(|modified| Some(modified) == meta.modified().ok());
                if same_version {
                    tx.execute("UPDATE media SET duration_secs=?1 WHERE path=?2
                        AND mtime_secs IS ?3 AND size_bytes IS ?4",
                        rusqlite::params![duration,path_str,mtime,meta.len() as i64])?;
                }
                tx.commit()
            })();
            if let Err(error) = published { tracing::warn!(%error,"video duration refresh held"); }
        }
        // A replaced video can queue its current version without waiting for another scan.
        crate::transcode::maybe_enqueue_for_existing_with_roots(db_path,path.to_path_buf(),roots.to_vec());
    } else if updated == 0 {
        // Existing unchanged items still need thumbnails and possibly a conversion.
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
                crate::transcode::maybe_enqueue_for_existing_with_roots(db_path, path.to_path_buf(), roots.to_vec());
            }
        }
    }
    updated
}

/// Preserve upstream standalone-path test classification with the six-ancestor context limit.
/// Scans use classify_with_root so hints cannot escape their selected library boundary.
#[cfg(test)]
fn classify(p: &Path) -> Option<&'static str> {classify_with_root(p,None)}

/// Select the nearest explicit whole-word/plural folder context; substrings such as Bookshop do not qualify.
fn audio_folder_kind(name:&str) -> Option<&'static str> {
    let normalized=name.to_ascii_lowercase();
    let words:Vec<&str>=normalized.split(|c:char|!c.is_alphanumeric()).collect();
    if words.iter().any(|word|matches!(*word,"music"|"song"|"songs"|"album"|"albums"|"artist"|"artists")) {return Some("music");}
    if words.iter().any(|word|matches!(*word,"audiobook"|"audiobooks"|"book"|"books"|"librivox")) {return Some("audiobook");}
    None
}

/// Recognize complete chapter/part/disc/CD/book words and digit suffixes, excluding Chapterhouse and partition.
fn audio_filename_is_book(p:&Path) -> bool {
    let stem=p.file_stem().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();
    stem.split(|c:char|!c.is_alphanumeric()).any(|word|
        ["chapter","chapters","part","parts","disc","discs","cd","cds","book","books"].iter().any(|hint|
            word==*hint || word.strip_prefix(hint).is_some_and(|suffix|!suffix.is_empty() && suffix.chars().all(|c|c.is_ascii_digit()))))
}

/// Classify supported files using at most six nearest ancestors, stopping after the selected root itself.
/// Explicit folder context wins over filenames and containers; otherwise m4b or book-name hints are books,
/// and other supported audio defaults to music. No duration probing or ambient parent hints are used.
fn classify_with_root(p:&Path,library_root:Option<&Path>) -> Option<&'static str> {
    let ext=p.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "m4b"|"mp3"|"flac"|"m4a"|"opus"|"ogg"=>{
            if let Some(parent)=p.parent() {
                for directory in parent.ancestors().take(6) {
                    if library_root.is_some_and(|root|!directory.starts_with(root)) {break;}
                    let name=directory.file_name().and_then(|name|name.to_str()).unwrap_or("");
                    if let Some(kind)=audio_folder_kind(name) {return Some(kind);}
                    if library_root==Some(directory) {break;}
                }
            }
            if ext=="m4b" || audio_filename_is_book(p) {"audiobook"}else{"music"}
        }
        "epub"=>"ebook",
        "cbz"|"cbr"=>"comic",
        "mkv"|"mp4"|"webm"=>"movie",
        _=>return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Library(std::path::PathBuf);

    impl Library {
        fn new(names: &[&str]) -> Self {
            let root = std::env::temp_dir().join(format!("frayd-scanner-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            for name in names {
                std::fs::write(root.join(name), []).unwrap();
            }
            Self(root)
        }
    }

    impl Drop for Library {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Construct synthetic upstream paths without accessing the filesystem.
    fn p(s: impl Into<std::path::PathBuf>) -> std::path::PathBuf { s.into() }

    /// Retain every upstream ea29 nested-folder, six-extension, m4b, chapter-name and default-music case.
    #[test]
    fn classify_audio_music_vs_audiobook() {
        let audio_exts = ["mp3", "flac", "m4a", "m4b", "opus", "ogg"];
        // folder-name signals (walks multiple ancestors)
        for ext in audio_exts {
            assert_eq!(classify(&p(format!("/media/Music/Album/song.1.{ext}"))), Some("music"), "music folder .{ext}");
            assert_eq!(classify(&p(format!("/media/audiobooks/Dune/chapter01.{ext}"))), Some("audiobook"), "audiobook folder .{ext}");
            assert_eq!(classify(&p(format!("/home/me/music/fav/track.{ext}"))), Some("music"), "nested music .{ext}");
        }
        // m4b defaults to audiobook without folder signal
        assert_eq!(classify(&p("/media/stuff/x.m4b")), Some("audiobook"));
        // chapter/part naming defaults to audiobook
        assert_eq!(classify(&p("/media/stuff/Book 01 - Chapter 3.mp3")), Some("audiobook"));
        // plain title defaults to music
        assert_eq!(classify(&p("/media/stuff/delain-virtue-and-vice.mp3")), Some("music"));
    }

    /// One structured probe persists decoded tags and retains filename fallbacks on unusable output.
    #[cfg(target_os = "linux")]
    #[test]
    fn new_item_metadata_uses_one_probe_and_persists_decoded_tags() {
        if std::env::var_os("FRAYD_TEST_METADATA_CHILD").is_none() {
            let result=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","scanner::tests::new_item_metadata_uses_one_probe_and_persists_decoded_tags","--nocapture"])
                .env("FRAYD_TEST_METADATA_CHILD","1").output().unwrap();
            print!("{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
            assert!(result.status.success(),"new-item metadata oracle failed: {}",result.status);
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let fixture=Library::new(&[]);
        let original_path=std::env::var_os("PATH").unwrap();
        let real_probe=std::env::split_paths(&original_path).map(|directory|directory.join("ffprobe"))
            .find(|candidate|candidate.is_file()).unwrap().canonicalize().unwrap();
        let bin=fixture.0.join("bin");std::fs::create_dir(&bin).unwrap();
        let count=fixture.0.join("count");let arguments=fixture.0.join("arguments");
        let output=fixture.0.join("output");let wrapper=bin.join("ffprobe");
        std::fs::write(&wrapper,r#"#!/bin/sh
printf 'probe\n' >> "$FRAYD_TEST_METADATA_COUNT"
printf '%s\n' "$@" > "$FRAYD_TEST_METADATA_ARGUMENTS"
if [ -n "$FRAYD_TEST_METADATA_OUTPUT" ]; then
    /bin/cat "$FRAYD_TEST_METADATA_OUTPUT"
    printf 'captured diagnostic' >&2
    exit "$FRAYD_TEST_METADATA_STATUS"
fi
exec "$FRAYD_TEST_METADATA_REAL_PROBE" "$@"
"#).unwrap();
        std::fs::set_permissions(&wrapper,std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("FRAYD_TEST_METADATA_COUNT",&count);
        std::env::set_var("FRAYD_TEST_METADATA_ARGUMENTS",&arguments);
        std::env::set_var("FRAYD_TEST_METADATA_REAL_PROBE",&real_probe);
        std::env::remove_var("FRAYD_TEST_METADATA_OUTPUT");
        std::env::set_var("PATH",std::env::join_paths([bin.clone()].into_iter().chain(std::env::split_paths(&original_path))).unwrap());
        let db=open(":memory:");
        let persist=|src: &Path, expected: (Option<f64>, &str, &str, &str, &str), expected_count: usize| {
            std::fs::write(&count,[]).unwrap();
            let start=std::time::Instant::now();
            assert_eq!(upsert_item(&db,String::new(),src,"music",None,&fixture.0),1);
            let elapsed=start.elapsed();
            let actual=db.query_row("SELECT duration_secs,title,json_extract(meta_json,'$.synopsis'),author,series FROM media WHERE path=?1",
                [src.to_str().unwrap()],|row|Ok((row.get::<_,Option<f64>>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?))).unwrap();
            assert_eq!(actual,(expected.0,expected.1.into(),expected.2.into(),expected.3.into(),expected.4.into()),"{}",src.display());
            assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(),expected_count,"{}",src.display());
            if expected_count==1 {
                assert_eq!(std::fs::read_to_string(&arguments).unwrap(),format!("-v\nquiet\n-show_entries\nformat=duration:format_tags=title,synopsis,description,artist,album_artist,author,album,series\n-of\njson\n{}\n",src.display()));
            }
            assert_eq!(upsert_item(&db,String::new(),src,"music",None,&fixture.0),0);
            assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(),expected_count,"unchanged music must not probe");
            println!("metadata_case={} probes={expected_count} elapsed_ms={:.3} persisted={actual:?} unchanged_reprobe=false",src.parent().unwrap().file_name().unwrap().to_string_lossy(),elapsed.as_secs_f64()*1000.0);
        };
        for (name,tags,expected) in [
            ("preferred",vec![("title","Preferred"),("synopsis","Synopsis"),("description","Ignored"),("artist","Artist"),("album_artist","Ignored"),("author","Ignored"),("album","Album"),("series","Ignored")],("Preferred","Synopsis","Artist","Album")),
            ("fallback",vec![("description","Description"),("album_artist","Album artist"),("author","Ignored"),("series","Series")],("Filename Author - Filename Series 03 - Title","Description","Album artist","Series")),
            ("absent",vec![],("Filename Author - Filename Series 03 - Title","","Filename Author","Filename Series")),
            ("decoded",vec![("title","  Comma, \"title\"  "),("synopsis","  Line\none  "),("artist","  Artist  "),("album","Back\\slash")],("Comma, \"title\"","Line\none","Artist","Back\\slash")),
        ] {
            let directory=fixture.0.join(name);std::fs::create_dir(&directory).unwrap();
            let src=directory.join("Filename Author - Filename Series 03 - Title.mka");
            let mut generate=Command::new("ffmpeg");
            generate.args(["-v","error","-nostdin","-f","lavfi","-i","sine=frequency=440:sample_rate=8000","-t","0.2","-c:a","pcm_s16le","-threads","1"]);
            for (key,value) in tags { generate.args(["-metadata",&format!("{key}={value}")]); }
            assert!(generate.arg(&src).output().unwrap().status.success());
            persist(&src,(Some(0.2),expected.0,expected.1,expected.2,expected.3),1);
        }
        std::env::set_var("FRAYD_TEST_METADATA_OUTPUT",&output);
        for (name,bytes,status,expected) in [
            ("valid-nonzero",br#"{"format":{"duration":" -2 ","tags":{"TITLE":" Title ","ARTIST":" ","ALBUM_ARTIST":" ","AUTHOR":" Author ","SYNOPSIS":" ","DESCRIPTION":" Description ","ALBUM":" ","SERIES":" Series "}}}"#.as_slice(),"9",(Some(-2.0),"Title","Description","Author","Series")),
            ("malformed",b"garbage".as_slice(),"0",(None,"Filename Author - Filename Series 03 - Title","","Filename Author","Filename Series")),
            ("non-utf8",b"\xff".as_slice(),"0",(None,"Filename Author - Filename Series 03 - Title","","Filename Author","Filename Series")),
            ("empty",b"".as_slice(),"0",(None,"Filename Author - Filename Series 03 - Title","","Filename Author","Filename Series")),
            ("wrong-types",br#"{"format":{"duration":{},"tags":{"title":42,"artist":[],"synopsis":true,"album":null}}}"#.as_slice(),"0",(None,"Filename Author - Filename Series 03 - Title","","Filename Author","Filename Series")),
        ] {
            let directory=fixture.0.join(name);std::fs::create_dir(&directory).unwrap();
            let src=directory.join("Filename Author - Filename Series 03 - Title.mka");std::fs::write(&src,[]).unwrap();
            std::fs::write(&output,bytes).unwrap();std::env::set_var("FRAYD_TEST_METADATA_STATUS",status);
            persist(&src,expected,1);
        }
        std::env::set_var("PATH",fixture.0.join("missing-tool"));
        let directory=fixture.0.join("missing-tool");std::fs::create_dir(&directory).unwrap();
        let src=directory.join("Filename Author - Filename Series 03 - Title.mka");std::fs::write(&src,[]).unwrap();
        persist(&src,(None,"Filename Author - Filename Series 03 - Title","","Filename Author","Filename Series"),0);
    }

    /// Replacement duration comes from the current media while identity and user state remain stable.
    #[cfg(target_os = "linux")]
    #[test]
    fn replaced_video_refreshes_duration_and_invalidates_only_known_failed_versions() {
        if std::env::var_os("FRAYD_TEST_DURATION_CHILD").is_none() {
            let result=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","scanner::tests::replaced_video_refreshes_duration_and_invalidates_only_known_failed_versions","--nocapture"])
                .env("FRAYD_TEST_DURATION_CHILD","1").env_remove("FRAYD_TRANSCODES").output().unwrap();
            print!("{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
            assert!(result.status.success(),"replacement duration oracle failed: {}",result.status);
            return;
        }
        crate::transcode::BUSY.store(true,std::sync::atomic::Ordering::SeqCst);
        let fixture=Library::new(&[]);
        let library=fixture.0.join("library");std::fs::create_dir(&library).unwrap();
        let mut generated=Vec::new();
        for seconds in [1,3] {
            let file=fixture.0.join(format!("{seconds}-seconds.mp4"));
            let result=Command::new("ffmpeg").args(["-v","error","-nostdin","-f","lavfi","-i","color=c=black:s=16x16:r=10",
                "-t",&seconds.to_string(),"-c:v","mpeg4","-q:v","5","-threads","1","-an"]).arg(&file).output().unwrap();
            assert!(result.status.success(),"real media generation failed: {}",String::from_utf8_lossy(&result.stderr));
            let duration=ffprobe_duration(file.to_str().unwrap()).expect("real ffprobe must qualify fixture duration");
            assert!((duration-seconds as f64).abs()<0.01);
            generated.push((file,duration));
        }
        assert_ne!(std::fs::metadata(&generated[0].0).unwrap().len(),std::fs::metadata(&generated[1].0).unwrap().len());
        let src=library.join("movie.mp4");std::fs::copy(&generated[0].0,&src).unwrap();
        let database=fixture.0.join("duration.sqlite");let db=open(database.to_str().unwrap());
        assert_eq!(scan(&db,String::new(),library.clone(),None),1);
        let (id,initial):(i64,f64)=db.query_row("SELECT id,duration_secs FROM media WHERE path=?1",[src.to_str().unwrap()],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert!((initial-generated[0].1).abs()<0.01);
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture');").unwrap();
        db.execute("INSERT INTO progress(identity_id,media_id,position) VALUES(1,?1,0.5)",[id]).unwrap();
        db.execute("UPDATE media SET title='owner title',author='owner author',transcode_path='legacy-unqualified' WHERE id=?1",[id]).unwrap();
        std::fs::copy(&generated[1].0,&src).unwrap();
        assert_eq!(scan(&db,String::new(),library.clone(),None),1);
        let duration=||db.query_row("SELECT duration_secs FROM media WHERE id=?1",[id],|row|row.get::<_,Option<f64>>(0)).unwrap();
        let replaced=duration().unwrap();
        println!("real_ffprobe_initial={} real_ffprobe_replacement={} stored_initial={initial} stored_replacement={replaced}",generated[0].1,generated[1].1);
        assert!((replaced-generated[1].1).abs()<0.01,"replaced source retained stale duration");
        assert_eq!(db.query_row("SELECT transcode_path FROM media WHERE id=?1",[id],|row|row.get::<_,Option<String>>(0)).unwrap(),None);
        assert_eq!(scan(&db,String::new(),library.clone(),None),0);
        assert_eq!(duration(),Some(replaced),"unchanged source must retain its qualified duration");

        std::fs::write(&src,b"invalid replacement media").unwrap();
        assert_eq!(ffprobe_duration(src.to_str().unwrap()),None,"failure control unexpectedly probed");
        assert_eq!(scan(&db,String::new(),library.clone(),None),1);
        assert_eq!(duration(),None,"known changed source must not retain an obsolete duration after probe failure");
        std::fs::copy(&generated[0].0,&src).unwrap();
        assert_eq!(scan(&db,String::new(),library.clone(),None),1);
        assert!((duration().unwrap()-generated[0].1).abs()<0.01);

        for (mtime,size) in [(None,Some(17)),(Some(7),None),(None,None)] {
            db.execute("UPDATE media SET mtime_secs=?1,size_bytes=?2,duration_secs=83,transcode_path='legacy-unqualified' WHERE id=?3",
                rusqlite::params![mtime,size,id]).unwrap();
            std::fs::write(&src,b"unknown original version").unwrap();
            assert_eq!(scan(&db,String::new(),library.clone(),None),1);
            assert_eq!(duration(),Some(83.0),"unknown prior version cannot prove legacy duration obsolete");
            assert_eq!(db.query_row("SELECT transcode_path FROM media WHERE id=?1",[id],|row|row.get::<_,String>(0)).unwrap(),"legacy-unqualified");
        }
        db.execute("UPDATE media SET mtime_secs=NULL,size_bytes=NULL WHERE id=?1",[id]).unwrap();
        std::fs::copy(&generated[1].0,&src).unwrap();
        assert_eq!(scan(&db,String::new(),library.clone(),None),1);
        assert!((duration().unwrap()-generated[1].1).abs()<0.01,"successful probe must qualify an unknown prior version");
        assert_eq!(db.query_row("SELECT id,title,author FROM media WHERE path=?1",[src.to_str().unwrap()],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).unwrap(),(id,"owner title".into(),"owner author".into()));
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=?1",[id],|row|row.get::<_,f64>(0)).unwrap(),0.5);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|row|row.get::<_,i64>(0)).unwrap(),1);
        println!("unchanged_duration_retained=true known_failed_probe_duration_null=true unknown_failed_probe_legacy_retained=true unknown_success_refreshed=true identity_progress_preserved=true");

        // Pause the real ffprobe after it reads the file, then alter one publication precondition.
        use std::os::unix::fs::PermissionsExt;
        let original_path=std::env::var_os("PATH").unwrap();
        let real_probe=std::env::split_paths(&original_path).map(|directory|directory.join("ffprobe"))
            .find(|candidate|candidate.is_file()).unwrap().canonicalize().unwrap();
        let bin=fixture.0.join("bin");std::fs::create_dir(&bin).unwrap();
        let wrapper=bin.join("ffprobe");
        std::fs::write(&wrapper,r#"#!/bin/sh
"$FRAYD_TEST_DURATION_PROBE" "$@"
probe_status=$?
: > "$FRAYD_TEST_DURATION_READY"
attempt=0
while [ ! -f "$FRAYD_TEST_DURATION_RELEASE" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -gt 1000000 ]; then exit 70; fi
done
exit "$probe_status"
"#).unwrap();
        std::fs::set_permissions(&wrapper,std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("FRAYD_TEST_DURATION_PROBE",real_probe);
        for mode in ["source","database","locked-source"] {
            let ready=fixture.0.join(format!("{mode}-ready"));let release=fixture.0.join(format!("{mode}-release"));
            std::env::set_var("FRAYD_TEST_DURATION_READY",&ready);
            std::env::set_var("FRAYD_TEST_DURATION_RELEASE",&release);
            std::fs::copy(&generated[0].0,&src).unwrap();
            db.execute("UPDATE media SET mtime_secs=0,size_bytes=0,duration_secs=83 WHERE id=?1",[id]).unwrap();
            std::env::set_var("PATH",&bin);
            std::thread::scope(|scope| {
                let change=scope.spawn(|| {
                    let reached=(0..500).any(|_| {
                        if ready.exists() {return true;}
                        std::thread::sleep(std::time::Duration::from_millis(5));false
                    });
                    if reached {
                        if mode=="source" {std::fs::copy(&generated[1].0,&src).unwrap();}
                        else if mode=="locked-source" {
                            let lock=crate::store::connect(database.to_str().unwrap()).unwrap();
                            lock.execute_batch("BEGIN IMMEDIATE").unwrap();
                            std::fs::write(&release,[]).unwrap();
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            std::fs::copy(&generated[1].0,&src).unwrap();
                            lock.execute_batch("COMMIT").unwrap();
                        } else {
                            let other=crate::store::connect(database.to_str().unwrap()).unwrap();
                            assert_eq!(other.execute("UPDATE media SET mtime_secs=mtime_secs+1,duration_secs=23 WHERE id=?1",[id]).unwrap(),1);
                        }
                    }
                    std::fs::write(&release,[]).unwrap();
                    assert!(reached,"real ffprobe did not reach the controlled publication boundary");
                });
                assert_eq!(scan(&db,String::new(),library.clone(),None),1);
                change.join().unwrap();
            });
            std::env::set_var("PATH",&original_path);
            assert_eq!(duration(),if mode=="database" {Some(23.0)} else {None},"{mode} race published a stale probe");
            println!("duration_publication_race={mode} real_probe_finished_before_change=true stale_probe_rejected=true");
        }
    }

    /// Permission failures warn with their path while accessible files and existing rows survive.
    #[cfg(target_os = "linux")]
    #[test]
    fn scan_warns_for_unreadable_entries_without_losing_accessible_or_retained_media() {
        use std::os::unix::{fs::PermissionsExt, process::CommandExt};
        use std::sync::{Arc, Mutex};

        let status=std::fs::read_to_string("/proc/self/status").unwrap();
        let effective_uid=status.lines().find(|line|line.starts_with("Uid:")).unwrap()
            .split_whitespace().nth(2).unwrap().parse::<u32>().unwrap();
        if std::env::var_os("FRAYD_TEST_SCAN_PERMISSIONS_CHILD").is_none() {
            let mut child=Command::new(std::env::current_exe().unwrap());
            child.args(["--exact","scanner::tests::scan_warns_for_unreadable_entries_without_losing_accessible_or_retained_media","--nocapture"])
                .env("FRAYD_TEST_SCAN_PERMISSIONS_CHILD","1").env_remove("FRAYD_TRANSCODES");
            // Cargo builders may run as root. Drop privilege before the child creates any fixture.
            if effective_uid==0 {child.gid(65534).uid(65534);}
            let result=child.output().unwrap();
            print!("{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
            assert!(result.status.success(),"unprivileged scan warning oracle failed: {}",result.status);
            return;
        }
        assert_ne!(effective_uid,0,"root cannot qualify a DAC permission failure");

        struct RestoreDirectory(std::path::PathBuf);
        impl Drop for RestoreDirectory {
            fn drop(&mut self) {
                let _=std::fs::set_permissions(&self.0,std::fs::Permissions::from_mode(0o700));
            }
        }
        #[derive(Clone)]
        struct WarningWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for WarningWriter {
            fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);Ok(bytes.len())
            }
            fn flush(&mut self)->std::io::Result<()> {Ok(())}
        }

        let fixture=Library::new(&[]);
        let library=fixture.0.join("library");std::fs::create_dir(&library).unwrap();
        let blocked=library.join("blocked_manga");std::fs::create_dir(&blocked).unwrap();
        let retained=blocked.join("retained.cbz");std::fs::write(&retained,b"retained source bytes").unwrap();
        let accessible=library.join("accessible.cbz");std::fs::write(&accessible,[]).unwrap();
        let outside=fixture.0.join("outside.cbz");std::fs::write(&outside,b"symlink target bytes").unwrap();
        let link=library.join("linked.cbz");std::os::unix::fs::symlink(&outside,&link).unwrap();
        let db=open(":memory:");
        db.execute("INSERT INTO media(id,kind,path,title,mtime_secs,size_bytes) VALUES(1,'comic',?1,'retained title',7,21)",[retained.to_str().unwrap()]).unwrap();
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture'); INSERT INTO progress(identity_id,media_id,position) VALUES(1,1,42);").unwrap();
        let restore=RestoreDirectory(blocked.clone());
        std::fs::set_permissions(&blocked,std::fs::Permissions::from_mode(0o000)).unwrap();
        let denial=std::fs::read_dir(&blocked).unwrap_err();
        assert_eq!(denial.kind(),std::io::ErrorKind::PermissionDenied,"fixture was not actually restricted");
        let warnings=Arc::new(Mutex::new(Vec::new()));let writer=WarningWriter(warnings.clone());
        let subscriber=tracing_subscriber::fmt().without_time().with_ansi(false)
            .with_max_level(tracing::Level::WARN).with_writer(move ||writer.clone()).finish();
        let missing=fixture.0.join("missing-root");
        tracing::subscriber::with_default(subscriber,|| {
            assert_eq!(scan(&db,String::new(),library.clone(),None),1);
            assert_eq!(scan(&db,String::new(),missing.clone(),None),0);
        });
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media WHERE path=?1",[accessible.to_str().unwrap()],|row|row.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT title,mtime_secs,size_bytes FROM media WHERE id=1",[],|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?))).unwrap(),("retained title".into(),7,21));
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|row|row.get::<_,f64>(0)).unwrap(),42.0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|row|row.get::<_,i64>(0)).unwrap(),2,"symlink or inaccessible media was indexed");
        drop(restore);
        assert_eq!(std::fs::read(&retained).unwrap(),b"retained source bytes");
        assert_eq!(std::fs::read(&outside).unwrap(),b"symlink target bytes");
        let captured=String::from_utf8(warnings.lock().unwrap().clone()).unwrap();
        println!("unprivileged=true permission_error={:?} accessible_indexed=1 retained_rows=1 progress=42 symlink_skipped=true",denial.kind());
        println!("WARN_CAPTURE_BEGIN\n{captured}WARN_CAPTURE_END");
        let fixture_path=fixture.0.clone();drop(fixture);assert!(!fixture_path.exists());
        println!("owned_fixture_removed=true mode_restored_before_cleanup=true");
        assert!(captured.lines().any(|line|line.contains(blocked.to_str().unwrap()) && line.contains("operation=\"read_dir\"")),"missing manga read_dir warning for {blocked:?}");
        assert!(captured.lines().any(|line|line.contains(blocked.to_str().unwrap()) && line.contains("operation=\"walk\"")),"missing WalkDir warning for {blocked:?}");
        assert!(captured.lines().any(|line|line.contains(missing.to_str().unwrap()) && line.contains("operation=\"walk\"")),"missing root-open warning for {missing:?}");
    }

    /// Known replacement grants retirement only to its exact inactive, unreferenced generated pair.
    #[cfg(target_os = "linux")]
    #[test]
    fn known_replacement_retires_only_the_prior_completed_pair() {
        // Keep the real scanner entry point but suppress asynchronous codec workers in this isolated child.
        if std::env::var_os("FRAYD_TEST_REPLACEMENT_CHILD").is_none() {
            let result=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","scanner::tests::known_replacement_retires_only_the_prior_completed_pair","--nocapture"])
                .env("FRAYD_TEST_REPLACEMENT_CHILD","1").env_remove("FRAYD_TRANSCODES").output().unwrap();
            print!("{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
            assert!(result.status.success(),"isolated replacement oracle failed: {}",result.status);
            return;
        }
        crate::transcode::BUSY.store(true,std::sync::atomic::Ordering::SeqCst);
        for mode in ["eligible", "reference-temp", "reference-final", "genuine", "unknown", "legacy", "active", "pending", "historical", "extra", "conflict", "symlink", "missing-lease", "wrong-cache", "failure", "ignored", "locked"] {
            let fixture=Library::new(&[]);
            let library=fixture.0.join("library");std::fs::create_dir(&library).unwrap();
            let src=library.join("film.mkv");std::fs::write(&src,b"source").unwrap();
            let database=fixture.0.join("db.sqlite");let db=open(database.to_str().unwrap());
            db.busy_timeout(std::time::Duration::from_millis(20)).unwrap();
            db.execute("INSERT INTO media(id,kind,path,title,mtime_secs,size_bytes) VALUES(1,'movie',?1,'keep',0,6)",[src.to_str().unwrap()]).unwrap();
            // An unrelated completed pair for this same owner must retain its historical policy.
            let neighbor=fixture.0.join("transcodes/1").join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir_all(&neighbor).unwrap();
            for (name,role) in [("audio.tmp.mp4","temp"),("audio.mp4","final")] {
                std::fs::write(neighbor.join(name),b"historical neighbor bytes").unwrap();
                db.execute("INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance,lifecycle) VALUES(?1,1,?2,?3,'generated','completed')",
                    rusqlite::params![neighbor.join(name).to_str().unwrap(),src.to_str().unwrap(),role]).unwrap();
            }
            let mut eligible_jobs=Vec::new();
            for iteration in 0..if mode=="eligible" {3} else {1} {
                let namespace=if mode=="wrong-cache" {"unmanaged/1"} else {"transcodes/1"};
                let job=fixture.0.join(namespace).join(uuid::Uuid::new_v4().to_string());std::fs::create_dir_all(&job).unwrap();
                let tmp=job.join("audio.tmp.mp4");let out=job.join("audio.mp4");
                std::fs::write(&tmp,b"prior temporary bytes").unwrap();std::fs::write(&out,b"prior completed bytes").unwrap();
                let lease=std::fs::OpenOptions::new().create_new(true).read(true).write(true).open(job.join("lease")).unwrap();
                if mode=="active" || mode=="pending" {lease.try_lock().unwrap();}
                for (path,role) in [(&tmp,"temp"),(&out,"final")] {
                    db.execute("INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance,lifecycle) VALUES(?1,1,?2,?3,'generated','completed')",
                        rusqlite::params![path.to_str().unwrap(),src.to_str().unwrap(),role]).unwrap();
                }
                db.execute("UPDATE media SET transcode_path=?1 WHERE id=1",[out.to_str().unwrap()]).unwrap();
                if mode=="unknown" || mode=="pending" {db.execute("UPDATE transcode_artifacts SET lifecycle=?1 WHERE path IN (?2,?3)",rusqlite::params![mode,tmp.to_str().unwrap(),out.to_str().unwrap()]).unwrap();}
                if mode=="legacy" {db.execute("UPDATE transcode_artifacts SET provenance='legacy_reference' WHERE path IN (?1,?2)",rusqlite::params![tmp.to_str().unwrap(),out.to_str().unwrap()]).unwrap();}
                if mode=="historical" {db.execute("UPDATE media SET size_bytes=NULL WHERE id=1",[]).unwrap();}
                if mode=="extra" {std::fs::write(job.join("genuine-extra"),b"keep extra").unwrap();}
                if mode=="missing-lease" {std::fs::remove_file(job.join("lease")).unwrap();}
                if mode=="symlink" {
                    let redirected=fixture.0.join("redirected");std::fs::rename(&job,&redirected).unwrap();
                    std::os::unix::fs::symlink(redirected,&job).unwrap();
                }
                if mode=="reference-temp" || mode=="reference-final" || mode=="genuine" {
                    db.execute("INSERT INTO media(id,kind,path,title,transcode_path) VALUES(2,'movie',?1,'other',?2)",
                        rusqlite::params![if mode=="genuine" {out.to_str().unwrap()} else {"other-source"},if mode=="reference-temp" {tmp.to_str().unwrap()} else {out.to_str().unwrap()}]).unwrap();
                    db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture'); INSERT INTO progress(identity_id,media_id,position) VALUES(1,2,42);").unwrap();
                }
                if mode=="conflict" {db.execute("UPDATE transcode_artifacts SET owner_source_path='different-source' WHERE path=?1",[tmp.to_str().unwrap()]).unwrap();}
                if mode=="failure" {db.execute_batch("CREATE TRIGGER reject_retirement BEFORE UPDATE OF lifecycle ON transcode_artifacts BEGIN SELECT RAISE(FAIL,'fixture refusal'); END;").unwrap();}
                if mode=="ignored" {db.execute_batch("CREATE TRIGGER reject_retirement BEFORE UPDATE OF lifecycle ON transcode_artifacts WHEN OLD.role='final' BEGIN SELECT RAISE(IGNORE); END;").unwrap();}
                let locked=if mode=="locked" {let lock=open(database.to_str().unwrap());lock.execute_batch("BEGIN IMMEDIATE").unwrap();Some(lock)} else {None};
                let before:Vec<(String,i64,String,String,String,String)>=db.prepare("SELECT path,owner_media_id,owner_source_path,role,provenance,lifecycle FROM transcode_artifacts ORDER BY path").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
                let replacement=vec![b'x';20+iteration];std::fs::write(&src,&replacement).unwrap();
                let changed=upsert_item(&db,database.to_string_lossy().into_owned(),&src,"movie",None,&library);
                let rollback=["failure","ignored","locked"].contains(&mode);
                assert_eq!(changed,if rollback {0}else{1},"{mode}");
                let pointer:Option<String>=db.query_row("SELECT transcode_path FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
                assert_eq!(pointer.is_some(),rollback || mode=="historical","{mode}");
                let states:Vec<String>=db.prepare("SELECT lifecycle FROM transcode_artifacts WHERE path IN (?1,?2)").unwrap().query_map(rusqlite::params![tmp.to_str().unwrap(),out.to_str().unwrap()],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
                if mode!="eligible" {assert!(states.iter().all(|state|state==if mode=="unknown" || mode=="pending" {mode} else {"completed"}),"{mode}: {states:?}");}
                if rollback {
                    assert_eq!(db.query_row("SELECT mtime_secs,size_bytes,title FROM media WHERE id=1",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?))).unwrap(),(0,6,"keep".into()),"{mode}");
                }
                crate::transcode::recover_pending_jobs(database.to_str().unwrap(),&library);
                assert_eq!(std::fs::read(&src).unwrap(),replacement,"{mode}");
                if mode=="eligible" {
                    eligible_jobs.push(job);
                } else {
                    assert_eq!(std::fs::read(&tmp).unwrap(),b"prior temporary bytes","{mode}");
                    assert_eq!(std::fs::read(&out).unwrap(),b"prior completed bytes","{mode}");
                    let after:Vec<(String,i64,String,String,String,String)>=db.prepare("SELECT path,owner_media_id,owner_source_path,role,provenance,lifecycle FROM transcode_artifacts ORDER BY path").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
                    assert_eq!(after,before,"{mode}");
                    if mode=="extra" {assert_eq!(std::fs::read(job.join("genuine-extra")).unwrap(),b"keep extra");}
                    if mode=="missing-lease" {assert!(!job.join("lease").exists());}
                    if mode=="symlink" {assert!(std::fs::symlink_metadata(&job).unwrap().file_type().is_symlink());}
                    if mode.starts_with("reference-") || mode=="genuine" {assert_eq!(db.query_row("SELECT position FROM progress",[],|r|r.get::<_,f64>(0)).unwrap(),42.0);}
                }
                drop(locked);drop(lease);
                println!("replacement mode={mode} iteration={iteration} changed={changed} remaining_pair_rows={}",db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap());
            }
            for name in ["audio.tmp.mp4","audio.mp4"] {
                assert_eq!(std::fs::read(neighbor.join(name)).unwrap(),b"historical neighbor bytes","{mode}");
            }
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE path IN (?1,?2) AND lifecycle='completed' AND provenance='generated' AND owner_source_path=?3 AND owner_media_id=1",
                rusqlite::params![neighbor.join("audio.tmp.mp4").to_str().unwrap(),neighbor.join("audio.mp4").to_str().unwrap(),src.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),2,"{mode}");
            if mode=="eligible" {
                let remaining:i64=db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get(0)).unwrap();
                println!("repeated replacement remaining_rows={remaining} remaining_jobs={}",eligible_jobs.iter().filter(|job|job.exists()).count());
                assert_eq!(remaining,2,"repeated known replacements retained completed provenance beyond the historical neighbor");
                assert!(eligible_jobs.iter().all(|job|!job.exists()));
            }
        }
    }

    /// Indexed genuine legacy targets refresh normally; generated and unindexed compatibility paths stay excluded.
    #[test]
    fn indexed_legacy_playback_targets_refresh_without_losing_artifact_exclusions() {
        let names=["Film.mkv","Film.browser.mp4","generated.browser.tmp.mp4","Other.mkv","Other.browser.mp4"];
        let library=Library::new(&names); let db=open(":memory:");
        for name in names {std::fs::write(library.0.join(name),b"original").unwrap();}
        for (index,name) in names[..4].iter().enumerate() {
            let path=library.0.join(name);let meta=std::fs::metadata(&path).unwrap();
            db.execute(r#"INSERT INTO media(id,kind,path,title,mtime_secs,size_bytes,meta_json)
                VALUES(?1,'movie',?2,?3,?4,?5,'{"synopsis":"keep"}')"#,
                rusqlite::params![index as i64+1,path.to_str().unwrap(),name,meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,meta.len() as i64]).unwrap();
        }
        db.execute("UPDATE media SET transcode_path=?1 WHERE id=1",[library.0.join(names[1]).to_str().unwrap()]).unwrap();
        db.execute("UPDATE media SET transcode_path=?1 WHERE id=4",[library.0.join(names[4]).to_str().unwrap()]).unwrap();
        db.execute("INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance) VALUES(?1,1,?2,'temp','generated')",
            rusqlite::params![library.0.join(names[2]).to_str().unwrap(),library.0.join(names[0]).to_str().unwrap()]).unwrap();
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture');
            INSERT INTO progress(identity_id,media_id,position) VALUES(1,2,42),(1,3,7);").unwrap();
        let genuine=library.0.join(names[1]);let modified=std::fs::metadata(&genuine).unwrap().modified().unwrap();
        let new_bytes=b"genuine replacement with a different size";std::fs::write(&genuine,new_bytes).unwrap();
        std::fs::File::open(&genuine).unwrap().set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
        let generated=library.0.join(names[2]);std::fs::write(&generated,b"changed generated output remains excluded").unwrap();
        let thumbs=library.0.with_file_name(format!("frayd-thumbs-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&thumbs).unwrap();
        for id in 1..=4 {std::fs::write(thumbs.join(format!("{id}.jpg")),b"fixture thumbnail").unwrap();}
        let changed=scan(&db,String::new(),library.0.clone(),Some(&thumbs));
        assert_eq!(db.query_row("SELECT size_bytes FROM media WHERE id=2",[],|r|r.get::<_,i64>(0)).unwrap(),new_bytes.len() as i64);
        assert_eq!(changed,1);
        assert_eq!(scan(&db,String::new(),library.0.clone(),Some(&thumbs)),0);
        assert_eq!(db.query_row("SELECT thumb_path FROM media WHERE id=2",[],|r|r.get::<_,String>(0)).unwrap(),thumbs.join("2.jpg").to_string_lossy());
        assert_eq!(db.query_row("SELECT size_bytes FROM media WHERE id=3",[],|r|r.get::<_,i64>(0)).unwrap(),8);
        assert_eq!(db.query_row("SELECT thumb_path FROM media WHERE id=3",[],|r|r.get::<_,Option<String>>(0)).unwrap(),None);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),4);
        assert_eq!(db.query_row("SELECT id FROM media WHERE path=?1",[genuine.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media WHERE path=?1",[library.0.join(names[4]).to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=2",[],|r|r.get::<_,f64>(0)).unwrap(),42.0);
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=3",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
        assert_eq!(db.query_row("SELECT json_extract(meta_json,'$.synopsis') FROM media WHERE id=2",[],|r|r.get::<_,String>(0)).unwrap(),"keep");
        assert_eq!(std::fs::read(genuine).unwrap(),new_bytes);
        assert_eq!(std::fs::read(generated).unwrap(),b"changed generated output remains excluded");
        for name in names {assert!(library.0.join(name).exists());}
        std::fs::remove_dir_all(thumbs).unwrap();
    }
    /// Filling any unknown prior fingerprint component cannot qualify legacy playback as stale.
    #[test]
    fn initial_unknown_source_versions_preserve_legacy_playback_pointers() {
        let library=Library::new(&["film.mkv"]);let src=library.0.join("film.mkv");
        for (mtime,size) in [(None,Some(8)),(Some(i64::MAX),None),(None,None)] {
            let db=open(":memory:");
            db.execute("INSERT INTO media(id,kind,path,title,mtime_secs,size_bytes,transcode_path)
                VALUES(1,'movie',?1,'Original',?2,?3,'legacy-unqualified')",rusqlite::params![src.to_str().unwrap(),mtime,size]).unwrap();
            assert_eq!(scan(&db,String::new(),library.0.clone(),None),1);
            assert_eq!(db.query_row("SELECT transcode_path FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"legacy-unqualified");
            assert_eq!(db.query_row("SELECT title FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"Original");
        }
    }
    /// Backdated and legacy-null fingerprints still invalidate only the changed comic cache field.
    #[test]
    fn changed_or_missing_comic_fingerprints_clear_only_page_names() {
        let library=Library::new(&["book.cbz"]); let db=open(":memory:");
        let path=library.0.join("book.cbz");
        db.execute(r#"INSERT INTO media(id,kind,path,title,mtime_secs,size_bytes,meta_json)
            VALUES(1,'comic',?1,'Original',?2,0,'{"pages":["old.png"],"synopsis":"keep"}')"#,
            rusqlite::params![path.to_str().unwrap(),i64::MAX]).unwrap();
        for legacy_null in [false,true] {
            if legacy_null { db.execute(r#"UPDATE media SET mtime_secs=NULL,size_bytes=NULL,
                meta_json=json_set(meta_json,'$.pages',json('["old.png"]')) WHERE id=1"#,[]).unwrap(); }
            assert_eq!(scan(&db,String::new(),library.0.clone(),None),1);
            assert_eq!(db.query_row("SELECT json_type(meta_json,'$.pages') FROM media WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap(),None);
            assert_eq!(db.query_row("SELECT json_extract(meta_json,'$.synopsis') FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"keep");
            assert_eq!(scan(&db,String::new(),library.0.clone(),None),0);
        }
    }
    /// Distinguish genuine exact-suffix sources from artifacts using ownership, never names.
    #[test]
    fn scan_preserves_genuine_browser_suffix_sources_and_progress() {
        let names = ["film.browser.mp4", "film.browser.tmp.mp4", "film.browser.mkv", "unreferenced.browser.mp4"];
        let library = Library::new(&names);
        let db = open(":memory:");
        db.execute("INSERT INTO identities (id,kind,key) VALUES (1,'device','fixture')", []).unwrap();
        for (index, name) in names.iter().enumerate() {
            db.execute("INSERT INTO media (id,kind,path,title,mtime_secs,transcode_path) VALUES (?1,'movie',?2,?3,?4,'done')",
                rusqlite::params![index as i64 + 1, library.0.join(name).to_str().unwrap(), name, i64::MAX]).unwrap();
            if index < 3 { db.execute("INSERT INTO progress(identity_id,media_id,position) VALUES (1,?1,42)", [index as i64 + 1]).unwrap(); }
        }
        scan(&db, String::new(), library.0.clone(), None);
        let count: i64 = db.query_row("SELECT COUNT(*) FROM media JOIN progress ON media.id=progress.media_id WHERE progress.position=42", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 3);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),4);
        for name in names { assert!(library.0.join(name).exists()); }
    }
    /// Preserve all persisted row fields except the one deliberately reclassified kind.
    fn audio_row_snapshot(db:&Connection,id:i64)->String {
        db.query_row("SELECT json_object('id',id,'path',path,'title',title,'subtitle',subtitle,'author',author,
            'series',series,'duration',duration_secs,'size',size_bytes,'mtime',mtime_secs,'meta',meta_json,
            'thumb',thumb_path,'conversion',transcode_path,'created',created_at) FROM media WHERE id=?1",[id],|r|r.get(0)).unwrap()
    }
    /// Folder signals are complete tokens; filename hints may be words or a hint followed only by digits.
    #[test]
    fn upstream_ea29_audio_tokens_defaults_and_nearest_folder() {
        for ext in ["mp3","flac","m4a","m4b","opus","ogg"] {
            for folder in ["Music","Songs","Albums","Artists","my music","song-collection"] {
                assert_eq!(classify(&p(format!("/media/{folder}/neutral/track.{ext}"))),Some("music"),"{folder} .{ext}");
            }
            for folder in ["Audiobooks","Audio Books","audio-book","Books","LibriVox"] {
                assert_eq!(classify(&p(format!("/media/{folder}/neutral/track.{ext}"))),Some("audiobook"),"{folder} .{ext}");
            }
            for folder in ["Booker T","Bookshop","Chapterhouse","Bookkeeping","Musical","Albumen","Artistically"] {
                assert_eq!(classify(&p(format!("/media/{folder}/neutral/title.{ext}"))),Some(if ext=="m4b" {"audiobook"}else{"music"}),"false context {folder} .{ext}");
            }
            assert_eq!(classify_with_root(&p(format!("/media/Music/ordinary.{ext}")),Some(Path::new("/media/Music"))),Some("music"));
            assert_eq!(classify_with_root(&p(format!("/media/Books/ordinary.{ext}")),Some(Path::new("/media/Books"))),Some("audiobook"));
            assert_eq!(classify(&p(format!("/media/Music/Booker T/track.{ext}"))),Some("music"));
            assert_eq!(classify(&p(format!("/media/Books/Chapterhouse/track.{ext}"))),Some("audiobook"));
            assert_eq!(classify(&p(format!("/media/Music/Books/neutral/track.{ext}"))),Some("audiobook"));
            assert_eq!(classify(&p(format!("/media/Books/Music/neutral/chapter01.{ext}"))),Some("music"));
            for stem in ["chapter01","part2","disc03","cd4","Book 01 - Chapter 3","part two"] {
                assert_eq!(classify(&p(format!("/media/neutral/{stem}.{ext}"))),Some("audiobook"),"filename {stem} .{ext}");
            }
            for stem in ["Chapterhouse","Booker T","Bookshop","partition","discovery","ordinary-title"] {
                assert_eq!(classify(&p(format!("/media/neutral/{stem}.{ext}"))),Some(if ext=="m4b" {"audiobook"}else{"music"}),"false filename {stem} .{ext}");
            }
        }
        assert_eq!(classify(&p("/Music/a/b/c/d/e/track.mp3")),Some("music"));
        assert_eq!(classify(&p("/Books/a/b/c/d/e/chapterless.mp3")),Some("audiobook"));
        assert_eq!(classify(&p("/Books/a/b/c/d/e/f/ordinary.mp3")),Some("music"));
        assert_eq!(classify(&p("/Books/a/b/c/d/e/f/ordinary.M4B")),Some("audiobook"));
    }
    /// Real rescans must update stale kinds without touching bytes, user state, cached metadata or conversion leases.
    #[cfg(unix)]
    #[test]
    fn upstream_ea29_audio_unchanged_fingerprint_refreshes_kind_only() {
        use std::os::unix::fs::{MetadataExt,PermissionsExt};
        if std::env::var_os("FRAYD_TEST_AUDIO_NO_PROBES").is_none() {
            let fixture=Library::new(&[]);let bin=fixture.0.join("bin");std::fs::create_dir(&bin).unwrap();
            for name in ["ffprobe","ffmpeg"] {
                let path=bin.join(name);std::fs::write(&path,b"#!/bin/sh\nprintf 'called' >> \"$FRAYD_TEST_AUDIO_PROBES\"\nexit 1\n").unwrap();std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let mut paths=vec![bin];paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
            let result=Command::new(std::env::current_exe().unwrap()).args(["--exact","scanner::tests::upstream_ea29_audio_unchanged_fingerprint_refreshes_kind_only","--nocapture"])
                .env("FRAYD_TEST_AUDIO_NO_PROBES","1").env("FRAYD_TEST_AUDIO_PROBES",fixture.0.join("probes")).env("PATH",std::env::join_paths(paths).unwrap()).env_remove("FRAYD_TRANSCODES").env_remove("FRAYD_THUMBS").output().unwrap();
            assert!(result.status.success(),"{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));return;
        }
        let fixture=Library::new(&[]);let library=fixture.0.join("Music");std::fs::create_dir(&library).unwrap();
        let database=fixture.0.join("db.sqlite");let db=open(database.to_str().unwrap());
        let thumbs=fixture.0.join("thumbs");std::fs::create_dir(&thumbs).unwrap();
        let job=fixture.0.join("transcodes/41").join(uuid::Uuid::new_v4().to_string());std::fs::create_dir_all(&job).unwrap();
        let output=job.join("audio.mp4");let temporary=job.join("audio.tmp.mp4");let lease_path=job.join("lease");
        std::fs::write(&output,b"converted bytes").unwrap();std::fs::write(&temporary,b"temporary bytes").unwrap();std::fs::write(&lease_path,b"lease bytes").unwrap();
        let lease=std::fs::OpenOptions::new().read(true).write(true).open(&lease_path).unwrap();
        #[cfg(target_os="linux")] assert!(lease.try_lock().is_ok());
        let mut sources=Vec::new();
        for (id,name,old_kind) in [(41,"ordinary.mp3","audiobook"),(42,"movie.mp4","music")] {
            let path=library.join(name);std::fs::write(&path,b"original exact source bytes").unwrap();let metadata=std::fs::metadata(&path).unwrap();
            let retained=thumbs.join(format!("retained-{id}.jpg"));std::fs::write(&retained,b"retained thumbnail").unwrap();std::fs::write(thumbs.join(format!("{id}.jpg")),b"unrelated cached thumbnail").unwrap();
            db.execute("INSERT INTO media(id,kind,path,title,subtitle,author,series,duration_secs,size_bytes,mtime_secs,meta_json,thumb_path,transcode_path,created_at)
                VALUES(?1,?2,?3,'owner title','owner subtitle','owner author','owner series',111.25,?4,?5,?6,?7,?8,123)",
                rusqlite::params![id,old_kind,path.to_str().unwrap(),metadata.len() as i64,metadata.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,r#"{"synopsis":"keep","pages":["keep"],"spine":["keep"],"chapters":[42]}"#,retained.to_str().unwrap(),if id==41 {output.to_str()}else{None}]).unwrap();sources.push(path);
        }
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture');
            INSERT INTO progress(identity_id,media_id,position) VALUES(1,41,42),(1,42,73);
            INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi,note) VALUES(1,41,'bookmark',0,'audio','keep'),(1,42,'bookmark',0,'video','keep');").unwrap();
        for (path,role) in [(&temporary,"temp"),(&output,"final")] {db.execute("INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance,lifecycle) VALUES(?1,41,?2,?3,'generated','completed')",rusqlite::params![path.to_str().unwrap(),sources[0].to_str().unwrap(),role]).unwrap();}
        let rows_before=[audio_row_snapshot(&db,41),audio_row_snapshot(&db,42)];
        let artifacts=||db.query_row("SELECT json_group_array(json_object('path',path,'owner',owner_media_id,'source',owner_source_path,'role',role,'provenance',provenance,'lifecycle',lifecycle)) FROM transcode_artifacts",[],|r|r.get::<_,String>(0)).unwrap();let artifacts_before=artifacts();
        let mut checked_paths=sources.clone();
        checked_paths.extend([output.clone(),temporary.clone(),lease_path.clone()]);
        for id in [41,42] {checked_paths.push(thumbs.join(format!("retained-{id}.jpg")));checked_paths.push(thumbs.join(format!("{id}.jpg")));}
        let file_versions:Vec<_>=checked_paths.into_iter().map(|path|{let metadata=std::fs::metadata(&path).unwrap();(path,metadata.len(),metadata.modified().unwrap(),metadata.dev(),metadata.ino())}).collect();
        let updated=scan(&db,database.to_string_lossy().into_owned(),library.clone(),Some(&thumbs));
        assert_eq!(db.query_row("SELECT kind FROM media WHERE id=41",[],|r|r.get::<_,String>(0)).unwrap(),"music","unchanged audio fingerprint retained the stale audiobook kind");
        assert_eq!(db.query_row("SELECT kind FROM media WHERE id=42",[],|r|r.get::<_,String>(0)).unwrap(),"movie");assert_eq!(updated,2);
        assert_eq!([audio_row_snapshot(&db,41),audio_row_snapshot(&db,42)],rows_before);assert_eq!(artifacts(),artifacts_before);
        for (id,position,cfi) in [(41,42.0,"audio"),(42,73.0,"video")] {
            assert_eq!(db.query_row("SELECT position FROM progress WHERE identity_id=1 AND media_id=?1",[id],|r|r.get::<_,f64>(0)).unwrap(),position);
            assert_eq!(db.query_row("SELECT cfi,note FROM annotations WHERE identity_id=1 AND media_id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap(),(cfi.to_string(),"keep".to_string()));
        }
        for (path,length,modified,device,inode) in file_versions {
            let current=std::fs::metadata(path).unwrap();assert_eq!((current.len(),current.modified().unwrap(),current.dev(),current.ino()),(length,modified,device,inode));
        }
        for source in sources {assert_eq!(std::fs::read(source).unwrap(),b"original exact source bytes");}
        for id in [41,42] {assert_eq!(std::fs::read(thumbs.join(format!("retained-{id}.jpg"))).unwrap(),b"retained thumbnail");assert_eq!(std::fs::read(thumbs.join(format!("{id}.jpg"))).unwrap(),b"unrelated cached thumbnail");}
        assert_eq!(std::fs::read(output).unwrap(),b"converted bytes");assert_eq!(std::fs::read(temporary).unwrap(),b"temporary bytes");assert_eq!(std::fs::read(&lease_path).unwrap(),b"lease bytes");
        #[cfg(target_os="linux")] assert!(std::fs::OpenOptions::new().read(true).write(true).open(lease_path).unwrap().try_lock().is_err());
        assert!(!crate::transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst));
        std::thread::sleep(std::time::Duration::from_millis(50));assert!(!std::path::PathBuf::from(std::env::var_os("FRAYD_TEST_AUDIO_PROBES").unwrap()).exists(),"kind-only refresh invoked a media probe/converter");
        for policy in ["ABORT,'fixture'","IGNORE"] {
            db.execute("UPDATE media SET kind='audiobook' WHERE id=41",[]).unwrap();
            db.execute_batch(&format!("CREATE TRIGGER reject_kind BEFORE UPDATE OF kind ON media WHEN OLD.id=41 BEGIN SELECT RAISE({policy}); END")).unwrap();
            let before=audio_row_snapshot(&db,41);
            assert_eq!(upsert_item(&db,database.to_string_lossy().into_owned(),&library.join("ordinary.mp3"),"music",Some(&thumbs),&library),0);
            assert_eq!(db.query_row("SELECT kind FROM media WHERE id=41",[],|r|r.get::<_,String>(0)).unwrap(),"audiobook");
            assert_eq!(audio_row_snapshot(&db,41),before);assert_eq!(artifacts(),artifacts_before);
            assert!(!std::path::PathBuf::from(std::env::var_os("FRAYD_TEST_AUDIO_PROBES").unwrap()).exists());
            db.execute_batch("DROP TRIGGER reject_kind").unwrap();
        }
    }
    /// A configured boundary, including its own name, limits ancestor hints; ambient folders never classify its files.
    #[test]
    fn upstream_ea29_audio_scans_ignore_ambient_ancestors() {
        for ambient in ["Books","Music"] {
            let fixture=Library::new(&[]);let root=fixture.0.join(ambient).join("selected");std::fs::create_dir_all(&root).unwrap();
            let database=fixture.0.join("db.sqlite");let db=open(database.to_str().unwrap());
            for (index,ext) in ["mp3","flac","m4a","m4b","opus","ogg"].into_iter().enumerate() {
                let source=root.join(format!("ordinary.{ext}"));std::fs::write(&source,b"source").unwrap();let metadata=std::fs::metadata(&source).unwrap();
                let expected=if ext=="m4b" {"audiobook"}else{"music"};let wrong=if expected=="music" {"audiobook"}else{"music"};
                db.execute("INSERT INTO media(id,kind,path,title,size_bytes,mtime_secs) VALUES(?1,?2,?3,'retained',?4,?5)",rusqlite::params![index as i64+1,wrong,source.to_str().unwrap(),metadata.len() as i64,metadata.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64]).unwrap();
            }
            scan(&db,database.to_string_lossy().into_owned(),root.clone(),None);
            for ext in ["mp3","flac","m4a","m4b","opus","ogg"] {assert_eq!(db.query_row("SELECT kind FROM media WHERE path=?1",[root.join(format!("ordinary.{ext}")).to_str().unwrap()],|r|r.get::<_,String>(0)).unwrap(),if ext=="m4b" {"audiobook"}else{"music"},"ambient {ambient}, .{ext}");}
        }
    }

    /// Real overlap and directory-creation refusals must retain state/bytes while identifying the failed path and reason.
    #[test]
    fn scan_cache_refusals_warn_without_index_or_filesystem_mutation() {
        if std::env::var_os("FRAYD_TEST_CACHE_WARN_CHILD").is_none() {
            let result=Command::new(std::env::current_exe().unwrap()).args(["--exact","scanner::tests::scan_cache_refusals_warn_without_index_or_filesystem_mutation","--nocapture"])
                .env("FRAYD_TEST_CACHE_WARN_CHILD","1").env_remove("FRAYD_TRANSCODES").env_remove("FRAYD_THUMBS").output().unwrap();
            print!("{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
            assert!(result.status.success(),"isolated scan refusal oracle failed: {}",result.status);return;
        }
        use std::sync::{Arc,Mutex};
        #[derive(Clone)]
        struct WarningWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for WarningWriter {
            fn write(&mut self,bytes:&[u8])->std::io::Result<usize>{self.0.lock().unwrap().extend_from_slice(bytes);Ok(bytes.len())}
            fn flush(&mut self)->std::io::Result<()>{Ok(())}
        }
        for failure in ["transcode-overlap","thumbnail-overlap","thumbnail-create"] {
            let fixture=Library::new(&[]);let library=fixture.0.join("library");std::fs::create_dir(&library).unwrap();
            let source=library.join("retained.cbz");std::fs::write(&source,b"retained source bytes").unwrap();
            let thumbnail=fixture.0.join("retained.jpg");std::fs::write(&thumbnail,b"retained thumbnail bytes").unwrap();
            let generated=fixture.0.join("generated.mp4");std::fs::write(&generated,b"retained generated bytes").unwrap();
            let database=if failure=="transcode-overlap" {library.join("frayd.sqlite")}else{fixture.0.join("frayd.sqlite")};
            let db=open(database.to_str().unwrap());
            db.execute("INSERT INTO media(id,kind,path,title,size_bytes,mtime_secs,meta_json,thumb_path,transcode_path) VALUES(1,'comic',?1,'retained title',7,1,?2,?3,?4)",
                rusqlite::params![source.to_str().unwrap(),r#"{"pages":["cached"],"synopsis":"keep"}"#,thumbnail.to_str().unwrap(),generated.to_str().unwrap()]).unwrap();
            db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture');INSERT INTO progress(identity_id,media_id,position) VALUES(1,1,42);").unwrap();
            db.execute("INSERT INTO transcode_artifacts VALUES(?1,1,?2,'final','generated','completed')",rusqlite::params![generated.to_str().unwrap(),source.to_str().unwrap()]).unwrap();
            let blocker=fixture.0.join("blocker");std::fs::write(&blocker,b"retained blocking file").unwrap();
            let requested_thumbs=if failure=="thumbnail-overlap" {library.join("unsafe/thumbs")}else if failure=="thumbnail-create" {blocker.join("thumbs")}else{fixture.0.join("thumbs")};
            let failed_path=if failure=="transcode-overlap" {library.join("transcodes")}else{requested_thumbs.clone()};
            if failure=="transcode-overlap" {assert!(crate::transcode::validate_cache(&failed_path,std::slice::from_ref(&library)).is_none());}
            if failure=="thumbnail-overlap" {assert!(crate::transcode::validate_cache(&requested_thumbs,std::slice::from_ref(&library)).is_none());}
            if failure=="thumbnail-create" {assert!(crate::transcode::validate_cache(&requested_thumbs,std::slice::from_ref(&library)).is_some());assert!(blocker.is_file());}
            let before=audio_row_snapshot(&db,1);let warnings=Arc::new(Mutex::new(Vec::new()));let writer=WarningWriter(warnings.clone());
            let subscriber=tracing_subscriber::fmt().without_time().with_ansi(false).with_max_level(tracing::Level::WARN).with_writer(move||writer.clone()).finish();
            let refreshed=tracing::subscriber::with_default(subscriber,||scan(&db,database.to_string_lossy().into_owned(),library.clone(),Some(&requested_thumbs)));
            assert_eq!(refreshed,0);assert_eq!(audio_row_snapshot(&db,1),before);
            assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),42.0);
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE provenance='generated' AND lifecycle='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert_eq!(std::fs::read(&source).unwrap(),b"retained source bytes");assert_eq!(std::fs::read(&thumbnail).unwrap(),b"retained thumbnail bytes");assert_eq!(std::fs::read(&generated).unwrap(),b"retained generated bytes");assert_eq!(std::fs::read(&blocker).unwrap(),b"retained blocking file");
            assert!(!library.join("transcodes").exists());assert!(!library.join("unsafe").exists());assert!(!requested_thumbs.exists());
            let captured=String::from_utf8(warnings.lock().unwrap().clone()).unwrap();
            assert!(captured.lines().any(|line|line.contains(failed_path.to_str().unwrap()) && line.contains(library.to_str().unwrap()) && line.contains("source_root=") && line.contains("reason=")),"missing path/source/reason diagnostic for {failure}: {captured}");
            if failure=="thumbnail-create" {assert!(captured.contains("error="),"filesystem error missing: {captured}");}
            println!("guard={failure} refreshed=0 state_retained=true bytes_retained=true no_directories_created=true warning={captured}");
        }
    }

}
