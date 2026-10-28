//! Browser audio conversion with exact artifact ownership and a writable cache.
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

pub static BUSY: AtomicBool = AtomicBool::new(false);

/// Release admission even when a worker unwinds.
struct JobReset;
impl Drop for JobReset {
    /// Make the next queued job admissible after every worker exit.
    fn drop(&mut self) { BUSY.store(false, Ordering::SeqCst); }
}

/// Does any audio track use a codec browsers cannot decode?
pub fn needs_transcode(src: &str) -> bool {
    let Ok(out) = Command::new("ffprobe").args([
        "-v", "quiet", "-select_streams", "a", "-show_entries", "stream=codec_name",
        "-of", "csv=p=0", src,
    ]).output() else { return false };
    String::from_utf8_lossy(&out.stdout).lines()
        .any(|c| matches!(c.trim(), "ac3" | "eac3" | "dts" | "truehd" | "mlp"))
}

/// Backfill only distinct, unambiguous legacy final references. Never infer temp ownership.
/// Remove only generated artifact media rows without user data or competing ownership.
/// Legacy playback references do not prove generation and never authorize row deletion.
/// This is walk-independent, so missing registered artifacts are reconciled too.
pub fn reconcile_artifacts(db: &Connection) -> HashSet<String> {
    let result = db.execute_batch("INSERT OR IGNORE INTO transcode_artifacts
        (path,owner_media_id,owner_source_path,role,provenance)
        SELECT transcode_path,id,path,'final','legacy_reference' FROM media m
        WHERE transcode_path IS NOT NULL AND transcode_path != path
        AND (SELECT COUNT(*) FROM media x WHERE x.transcode_path=m.transcode_path)=1;");
    if let Err(error) = result { tracing::warn!(%error, "artifact backfill failed"); }
    let conflicts = db.query_row("SELECT COUNT(*) FROM transcode_artifacts a LEFT JOIN media m
        ON m.id=a.owner_media_id WHERE m.path IS NULL OR m.path != a.owner_source_path
        OR EXISTS (SELECT 1 FROM media other WHERE other.transcode_path=a.path AND other.id != a.owner_media_id)",
        [], |row| row.get::<_,i64>(0));
    match conflicts {
        Ok(count) if count > 0 => tracing::warn!(count,"artifact ownership conflicts retained"),
        Err(error) => tracing::warn!(%error,"artifact ownership check unavailable"),
        _ => (),
    }
    let mut owned = HashSet::new();
    let Ok(mut statement) = db.prepare("SELECT a.path FROM transcode_artifacts a JOIN media m
        ON m.id=a.owner_media_id AND m.path=a.owner_source_path WHERE a.path != m.path
        AND NOT EXISTS (SELECT 1 FROM media other WHERE other.transcode_path=a.path AND other.id != m.id)") else { return owned };
    let Ok(rows) = statement.query_map([], |r| r.get::<_, String>(0)) else { return owned };
    for path in rows.flatten() { owned.insert(path); }
    drop(statement);
    for path in &owned {
        match db.execute("DELETE FROM media WHERE path=?1
            AND EXISTS (SELECT 1 FROM transcode_artifacts a WHERE a.path=media.path AND a.provenance='generated')
            AND NOT EXISTS (SELECT 1 FROM progress WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM annotations WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM tracks WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM playlist_items WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM transcode_artifacts WHERE owner_media_id=media.id)", [path]) {
            Ok(0) => {
                let retained = db.query_row("SELECT id FROM media WHERE path=?1", [path], |r| r.get::<_,i64>(0));
                if let Ok(id) = retained { tracing::warn!(id, "artifact row retained: legacy reference, user data or ownership reference"); }
            }
            Ok(_) => (),
            Err(error) => tracing::warn!(%error, "artifact row cleanup held"),
        }
    }
    owned
}

/// Resolve the dedicated cache and refuse either direction of overlap with a library.
fn prepare_cache(cache: &Path, library: &Path) -> Option<PathBuf> {
    // Canonicalize the existing ancestor before creating anything inside a library.
    let absolute = if cache.is_absolute() { cache.to_path_buf() } else { std::env::current_dir().ok()?.join(cache) };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => { normalized.pop(); }
            std::path::Component::CurDir => (),
            other => normalized.push(other.as_os_str()),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(ancestor.file_name()?.to_os_string());
        ancestor = ancestor.parent()?;
    }
    let mut resolved = ancestor.canonicalize().ok()?;
    for part in suffix.into_iter().rev() { resolved.push(part); }
    let library = library.canonicalize().ok()?;
    if resolved.starts_with(&library) || library.starts_with(&resolved) { return None; }
    std::fs::create_dir_all(&resolved).ok()?;
    let canonical = resolved.canonicalize().ok()?;
    if !canonical.is_dir() || canonical.starts_with(&library) || library.starts_with(&canonical) { return None; }
    Some(canonical)
}

/// Reserve a unique job namespace and persist both exact paths before launching ffmpeg.
fn reserve_job(db: &mut Connection, cache: &Path, library: &Path, id: i64, src: &Path) -> Option<(PathBuf,PathBuf)> {
    let cache = prepare_cache(cache, library)?;
    let job = cache.join(id.to_string()).join(uuid::Uuid::new_v4().to_string());
    let tmp = job.join("audio.tmp.mp4");
    let out = job.join("audio.mp4");
    let tx = db.transaction().ok()?;
    for (path,role) in [(&tmp,"temp"),(&out,"final")] {
        let inserted = tx.execute("INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance)
            SELECT ?1,id,path,?2,'generated' FROM media WHERE id=?3 AND path=?4",
            rusqlite::params![path.to_str()?,role,id,src.to_str()?]).ok()?;
        if inserted != 1 { return None; }
    }
    tx.commit().ok()?;
    std::fs::create_dir_all(job).ok()?;
    Some((tmp,out))
}

/// Convert only to paths already registered for this job. Never write beside the source.
fn transcode_to_browser_mp4(src: &Path, tmp: &Path, out: &Path) -> Option<PathBuf> {
    let status = Command::new("ffmpeg").args([
        "-n", "-i", src.to_str()?, "-map", "0:v:0", "-map", "0:a:0?", "-c:v", "copy",
        "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-f", "mp4",
    ]).arg(tmp).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().ok()?;
    if !status.success() || !tmp.is_file() { let _ = std::fs::remove_file(tmp); return None; }
    std::fs::rename(tmp, out).ok()?;
    Some(out.to_path_buf())
}

/// Queue one source; codec detection and all conversion work use a worker connection.
pub fn maybe_enqueue(db_path: String, media_id: i64, src: PathBuf, library: PathBuf) {
    if BUSY.swap(true, Ordering::SeqCst) { return; }
    std::thread::spawn(move || {
        let _reset = JobReset;
        run_job(&db_path, media_id, &src, &library);
    });
}

/// Resolve an existing media ID before queueing the same guarded worker path.
pub fn maybe_enqueue_for_existing(db_path: String, src: PathBuf, library: PathBuf) {
    if BUSY.load(Ordering::SeqCst) { return; }
    let Ok(db) = crate::store::connect(&db_path) else { return };
    let Ok(id) = db.query_row("SELECT id FROM media WHERE path=?1 AND transcode_path IS NULL",
        [src.to_string_lossy().as_ref()], |r| r.get(0)) else { return };
    maybe_enqueue(db_path, id, src, library);
}

/// Fail closed on database/cache/registry errors and persist the actual completed path.
fn run_job(db_path: &str, media_id: i64, src: &Path, library: &Path) {
    let Ok(mut db) = crate::store::connect(db_path) else { return };
    let eligible = db.query_row("SELECT 1 FROM media WHERE id=?1 AND path=?2 AND transcode_path IS NULL",
        rusqlite::params![media_id,src.to_string_lossy()], |_| Ok(())).is_ok();
    if !eligible || !needs_transcode(&src.to_string_lossy()) { return; }
    let cache = std::env::var_os("FRAYD_TRANSCODES").map(PathBuf::from)
        .unwrap_or_else(|| Path::new(db_path).parent().unwrap_or(Path::new(".")).join("transcodes"));
    let Some((tmp,out)) = reserve_job(&mut db,&cache,library,media_id,src) else {
        tracing::warn!(media_id,"transcode held: cache or registry unavailable"); return;
    };
    let Some(completed) = transcode_to_browser_mp4(src,&tmp,&out) else { return };
    if let Err(error) = db.execute("UPDATE media SET transcode_path=?1 WHERE id=?2 AND path=?3",
        rusqlite::params![completed.to_string_lossy(),media_id,src.to_string_lossy()]) {
        tracing::warn!(%error,media_id,"transcode completed but publication failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    struct Fixture(PathBuf);
    impl Fixture {
        /// Create a disposable library and sibling cache parent for ownership tests.
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("frayd-transcode-{}",uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("library")).unwrap(); Self(root)
        }
        /// Create a recognizable original whose bytes must survive every worker path.
        fn source(&self, name: &str) -> PathBuf {
            let path = self.0.join("library").join(name); std::fs::write(&path,b"original").unwrap(); path
        }
    }
    impl Drop for Fixture {
        /// Remove only the UUID-scoped fixture tree owned by this test.
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    /// Seed an explicit media identity independently of scanner metadata probing.
    fn insert(db: &Connection,id:i64,path:&Path) {
        db.execute("INSERT INTO media(id,kind,path,title) VALUES(?1,'movie',?2,'fixture')",rusqlite::params![id,path.to_str().unwrap()]).unwrap();
    }
    /// Protect same-stem sources from collisions and source-directory output writes.
    #[test]
    fn reservations_are_unique_registered_and_outside_readonly_sources() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new(); let mut db = crate::store::open(":memory:");
        let first = fixture.source("film.mkv"); let second = fixture.source("film.mp4");
        insert(&db,1,&first); insert(&db,2,&second);
        let library=fixture.0.join("library"); let cache=fixture.0.join("cache");
        std::fs::set_permissions(&library,std::fs::Permissions::from_mode(0o555)).unwrap();
        let a=reserve_job(&mut db,&cache,&library,1,&first).unwrap();
        let b=reserve_job(&mut db,&cache,&library,2,&second).unwrap();
        assert_ne!(a.1,b.1); assert!(a.1.starts_with(&cache)); assert!(!a.1.starts_with(&library));
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),4);
        assert_eq!(std::fs::read(&first).unwrap(),b"original");
        assert_eq!(std::fs::read_dir(&library).unwrap().count(),2);
        std::fs::set_permissions(&library,std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    /// Refuse library overlap and registry/cache errors before creating job outputs.
    #[test]
    fn overlapping_cache_and_registry_failure_publish_no_job_files() {
        let fixture=Fixture::new(); let mut db=crate::store::open(":memory:");
        let src=fixture.source("film.mkv"); insert(&db,1,&src); let library=fixture.0.join("library");
        assert!(reserve_job(&mut db,&library.join("cache"),&library,1,&src).is_none());
        assert!(!library.join("cache").exists());
        assert!(reserve_job(&mut db,&fixture.0,&library,1,&src).is_none());
        db.execute_batch("CREATE TRIGGER reject_artifacts BEFORE INSERT ON transcode_artifacts BEGIN SELECT RAISE(FAIL,'fixture registry failure'); END;").unwrap();
        let cache=fixture.0.join("cache");
        assert!(reserve_job(&mut db,&cache,&library,1,&src).is_none());
        assert_eq!(std::fs::read_dir(cache).unwrap().count(),0);
        let bad_cache=fixture.0.join("file"); std::fs::write(&bad_cache,b"file").unwrap();
        assert!(reserve_job(&mut db,&bad_cache,&library,1,&src).is_none());
        assert!(!library.join("film.browser.mp4").exists());
    }
    /// Ensure a failed external conversion leaves original bytes and playback selection intact.
    #[test]
    fn failed_conversion_does_not_publish_or_touch_originals() {
        let fixture=Fixture::new(); let mut db=crate::store::open(":memory:");
        let src=fixture.source("invalid.mkv"); insert(&db,1,&src);
        let (tmp,out)=reserve_job(&mut db,&fixture.0.join("cache"),&fixture.0.join("library"),1,&src).unwrap();
        assert!(transcode_to_browser_mp4(&src,&tmp,&out).is_none());
        assert!(!out.exists()); assert!(!tmp.exists()); assert_eq!(std::fs::read(&src).unwrap(),b"original");
        assert_eq!(db.query_row("SELECT transcode_path FROM media",[],|r|r.get::<_,Option<String>>(0)).unwrap(),None);
    }
    /// Exercise walk-independent cleanup while protecting every supported user reference.
    #[test]
    fn registered_present_and_missing_rows_prune_but_all_user_references_and_conflicts_survive() {
        let fixture=Fixture::new(); let db=crate::store::open(":memory:");
        let src=fixture.source("source.mkv"); insert(&db,1,&src);
        let names=["present.mp4","missing.mp4","progress.mp4","annotation.mp4","track.mp4","playlist.mp4","conflict.mp4"];
        for (index,name) in names.iter().enumerate() {
            let path=fixture.0.join(name); if index==0 { std::fs::write(&path,b"owned").unwrap(); }
            insert(&db,index as i64+2,&path);
            db.execute("INSERT INTO transcode_artifacts VALUES(?1,1,?2,'temp','generated')",rusqlite::params![path.to_str().unwrap(),src.to_str().unwrap()]).unwrap();
        }
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture');
            INSERT INTO progress(identity_id,media_id,position) VALUES(1,4,42);
            INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi) VALUES(1,5,'bookmark',0,'fixture');
            INSERT INTO tracks(media_id) VALUES(6);
            INSERT INTO playlists(id,identity_id,title) VALUES(1,1,'fixture');
            INSERT INTO playlist_items VALUES(1,7,0);").unwrap();
        let conflict_src=fixture.source("other.mkv"); insert(&db,9,&conflict_src);
        db.execute("UPDATE media SET transcode_path=?1 WHERE id=9",[fixture.0.join("conflict.mp4").to_str().unwrap()]).unwrap();
        let owned=reconcile_artifacts(&db);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media WHERE id IN(2,3)",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media WHERE id BETWEEN 4 AND 8",[],|r|r.get::<_,i64>(0)).unwrap(),5);
        assert!(!owned.contains(fixture.0.join("conflict.mp4").to_str().unwrap()));
        assert_eq!(std::fs::read(fixture.0.join("present.mp4")).unwrap(),b"owned");
        reconcile_artifacts(&db);
    }
    /// A historical playback pointer can refer to a genuine indexed source after a sibling collision.
    #[test]
    fn legacy_playback_collision_preserves_genuine_media_identity_and_bytes() {
        let fixture=Fixture::new(); let db=crate::store::open(":memory:");
        let src=fixture.source("Film.mkv"); let genuine=fixture.source("Film.browser.mp4");
        insert(&db,1,&src); insert(&db,2,&genuine);
        db.execute("UPDATE media SET transcode_path=?1 WHERE id=1",[genuine.to_str().unwrap()]).unwrap();
        reconcile_artifacts(&db);
        assert_eq!(db.query_row("SELECT id FROM media WHERE path=?1",[genuine.to_str().unwrap()],|r|r.get::<_,i64>(0)).optional().unwrap(),Some(2));
        assert_eq!(std::fs::read(&genuine).unwrap(),b"original");
        assert_eq!(db.query_row("SELECT COALESCE(transcode_path,path) FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),genuine.to_string_lossy());
    }
    /// Preserve existing playback references and leave ambiguous historical temp names untouched.
    #[test]
    fn legacy_final_reference_backfill_is_idempotent_without_guessing_temp_ownership() {
        let fixture=Fixture::new(); let db=crate::store::open(":memory:");
        db.execute_batch(include_str!("../schema.sql")).unwrap();
        let src=fixture.source("film.mkv"); insert(&db,1,&src);
        let legacy=fixture.source("film.browser.mp4"); let unknown=fixture.source("film.browser.tmp.mp4"); insert(&db,2,&unknown);
        db.execute("UPDATE media SET transcode_path=?1 WHERE id=1",[legacy.to_str().unwrap()]).unwrap();
        for _ in 0..2 { reconcile_artifacts(&db); }
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        let playback:String=db.query_row("SELECT COALESCE(transcode_path,path) FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        assert_eq!(std::fs::read(playback).unwrap(),b"original");
        assert_eq!(db.query_row("SELECT id FROM media WHERE path=?1",[unknown.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),2);
    }
}
