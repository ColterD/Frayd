//! Browser audio conversion with exact artifact ownership and a writable cache.
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::fs::{File, OpenOptions};
use std::sync::atomic::{AtomicBool, Ordering};

pub static BUSY: AtomicBool = AtomicBool::new(false);

/// Release admission even when a worker unwinds.
struct JobReset;
impl Drop for JobReset {
    /// Make the next queued job admissible after every worker exit.
    fn drop(&mut self) { BUSY.store(false, Ordering::SeqCst); }
}

#[cfg(test)]
mod retained_job_tests {
    use super::*;

    /// A retained uncertain pair bounds repeated reservations even when the source identity changes.
    #[test]
    fn unqualified_rescans_hold_one_job_and_preserve_other_media() {
        let root=std::env::temp_dir().join(format!("frayd-retained-job-{}",uuid::Uuid::new_v4()));
        let library=root.join("library");let cache=root.join("cache");
        std::fs::create_dir_all(&library).unwrap();
        let src=library.join("film.mkv");let replacement=library.join("replaced.mkv");
        let other=library.join("other.mkv");
        std::fs::write(&src,b"original").unwrap();
        std::fs::write(&replacement,b"replacement").unwrap();
        std::fs::write(&other,b"other original").unwrap();
        let mut db=crate::store::open(":memory:");
        for (id,path) in [(1,&src),(2,&other)] {
            db.execute("INSERT INTO media(id,kind,path,title) VALUES(?1,'movie',?2,'fixture')",
                rusqlite::params![id,path.to_str().unwrap()]).unwrap();
        }
        let roots=[library.clone()];
        let (tmp,_,lease)=reserve_job_with_roots(&mut db,&cache,&roots,1,&src,false).unwrap();
        std::fs::write(&tmp,b"retained partial").unwrap();
        for _ in 0..2 {
            assert!(reserve_job_with_roots(&mut db,&cache,&roots,1,&src,false).is_none());
        }
        drop(lease);
        db.execute("UPDATE media SET path=?1 WHERE id=1",[replacement.to_str().unwrap()]).unwrap();
        assert!(reserve_job_with_roots(&mut db,&cache,&roots,1,&replacement,false).is_none());
        assert_eq!(std::fs::read_dir(cache.join("1")).unwrap().count(),1);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE owner_media_id=1",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert_eq!(std::fs::read(&tmp).unwrap(),b"retained partial");
        assert_eq!(std::fs::read(&replacement).unwrap(),b"replacement");
        let (_,_,other_lease)=reserve_job_with_roots(&mut db,&cache,&roots,2,&other,false).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE owner_media_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        drop(other_lease);
        db.execute("UPDATE transcode_artifacts SET lifecycle='completed' WHERE owner_media_id=1",[]).unwrap();
        assert!(reserve_job_with_roots(&mut db,&cache,&roots,1,&replacement,false).is_some());
        assert_eq!(std::fs::read(&src).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }
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
/// Indexed legacy targets remain scan eligible; unindexed playback retains compatibility exclusion.
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
        AND NOT EXISTS (SELECT 1 FROM media other WHERE other.transcode_path=a.path AND other.id != m.id)
        AND NOT (a.provenance='legacy_reference' AND EXISTS (SELECT 1 FROM media indexed_media WHERE indexed_media.path=a.path))") else { return owned };
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

/// Resolve existing ancestors and canonical root aliases without creating a cache or mutating registry/files.
/// Refuse empty/unresolvable snapshots and both directions of overlap with every configured root.
/// This validates the private-cache boundary; it does not qualify hostile concurrent filesystem writers.
pub(crate) fn validate_cache(cache: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    if roots.is_empty() { return None; }
    let absolute=if cache.is_absolute() { cache.to_path_buf() } else { std::env::current_dir().ok()?.join(cache) };
    let mut ancestor=absolute.as_path(); let mut suffix=Vec::new();
    while !ancestor.exists() {
        suffix.push(ancestor.file_name()?.to_os_string()); ancestor=ancestor.parent()?;
    }
    let mut resolved=ancestor.canonicalize().ok()?;
    for part in suffix.into_iter().rev() {
        if part==".." || part=="." { return None; }
        resolved.push(part);
    }
    for root in roots {
        let root=root.canonicalize().ok()?;
        if !root.is_dir() || resolved.starts_with(&root) || root.starts_with(&resolved) { return None; }
    }
    Some(resolved)
}

/// Create and canonicalize the cache only after validating it against every admitted root.
/// Return None when creation or the post-creation boundary check fails.
fn prepare_cache(cache: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let resolved=validate_cache(cache,roots)?;
    std::fs::create_dir_all(&resolved).ok()?;
    let canonical=resolved.canonicalize().ok()?;
    if !canonical.is_dir() { return None; }
    validate_cache(&canonical,roots)
}

/// A fresh reservation capability binds publication to its exact output and recovery qualification.
struct JobLease {
    file: File,
    completed: PathBuf,
    reclaimable: bool,
}

/// Reserve a unique job namespace and persist both exact paths before launching ffmpeg.
/// The cache must remain private: path checks reject existing redirects, not hostile writer races.
#[cfg(test)]
fn reserve_job(db: &mut Connection, cache: &Path, library: &Path, id: i64, src: &Path) -> Option<(PathBuf,PathBuf,JobLease)> {
    reserve_job_with_roots(db,cache,&[library.to_path_buf()],id,src,cfg!(target_os = "linux"))
}

/// Preserve conversion without granting recovery authority when the platform/filesystem lease is unqualified.
#[cfg(test)]
fn reserve_job_with_recovery(db: &mut Connection, cache: &Path, library: &Path, id: i64, src: &Path,
    qualify_recovery: bool) -> Option<(PathBuf,PathBuf,JobLease)> {
    reserve_job_with_roots(db,cache,&[library.to_path_buf()],id,src,qualify_recovery)
}

/// Register unique temporary and completed paths for one source before conversion starts.
/// Return a lease only after cache and registry admission; unqualified recovery retains
/// existing unfinished work rather than taking its ownership.
fn reserve_job_with_roots(db: &mut Connection, cache: &Path, roots: &[PathBuf], id: i64, src: &Path, qualify_recovery: bool) -> Option<(PathBuf,PathBuf,JobLease)> {
    let cache = prepare_cache(cache, roots)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).ok()?;
    if !qualify_recovery && unfinished_generated_job(&tx,id) { return None; }
    let media = cache.join(id.to_string());
    match std::fs::create_dir(&media) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return None,
    }
    let metadata = std::fs::symlink_metadata(&media).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() { return None; }
    let media = media.canonicalize().ok()?;
    if media.parent() != Some(cache.as_path()) { return None; }
    let job = media.join(uuid::Uuid::new_v4().to_string());
    // Atomic creation refuses every preexisting object, including symlinks and directories.
    std::fs::create_dir(&job).ok()?;
    let reserved = (|| {
        let metadata = std::fs::symlink_metadata(&job).ok()?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() { return None; }
        let canonical = job.canonicalize().ok()?;
        if canonical.parent() != Some(media.as_path()) || !canonical.starts_with(&cache) { return None; }
        let lease = OpenOptions::new().create_new(true).read(true).write(true)
            .open(canonical.join("lease")).ok()?;
        let reclaimable=qualify_recovery && lease.try_lock().is_ok();
        let lifecycle=if reclaimable {"pending"} else {"unknown"};
        let tmp = canonical.join("audio.tmp.mp4");
        let out = canonical.join("audio.mp4");
        for (path,role) in [(&tmp,"temp"),(&out,"final")] {
            let inserted = tx.execute("INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance,lifecycle)
                SELECT ?1,id,path,?2,'generated',?5 FROM media WHERE id=?3 AND path=?4",
                rusqlite::params![path.to_str()?,role,id,src.to_str()?,lifecycle]).ok()?;
            if inserted != 1 { return None; }
        }
        tx.commit().ok()?;
        Some((tmp,out.clone(),JobLease {file:lease,completed:out,reclaimable}))
    })();
    if reserved.is_none() { let _ = std::fs::remove_file(job.join("lease")); let _ = std::fs::remove_dir(&job); }
    reserved
}

/// Convert only to paths already registered for this job. Never write beside the source.
fn transcode_to_browser_mp4(src: &Path, tmp: &Path, out: &Path, lease: &JobLease) -> Option<PathBuf> {
    let status = Command::new("ffmpeg").args([
        "-nostdin", "-n", "-i", src.to_str()?, "-map", "0:v:0", "-map", "0:a:0?", "-c:v", "copy",
        "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-f", "mp4",
    ]).arg(tmp).stdin(Stdio::from(lease.file.try_clone().ok()?)).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().ok()?;
    if !status.success() || !tmp.is_file() { return None; }
    std::fs::rename(tmp, out).ok()?;
    Some(out.to_path_buf())
}

/// Validate fixed private job names and reject redirects or unregistered children before deletion.
fn job_namespace(tmp: &Path, out: &Path, id: i64) -> Option<PathBuf> {
    let job = tmp.parent()?;
    if tmp.file_name()? != "audio.tmp.mp4" || out != job.join("audio.mp4")
        || job.parent()?.file_name()?.to_str()? != id.to_string()
        || uuid::Uuid::parse_str(job.file_name()?.to_str()?).is_err() { return None; }
    for directory in [job.parent()?, job] {
        match std::fs::symlink_metadata(directory) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                if directory.canonicalize().ok()?.as_path() != directory { return None; }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && directory == job => (),
            _ => return None,
        }
    }
    if job.exists() {
        for entry in std::fs::read_dir(job).ok()? {
            let entry = entry.ok()?;
            if !["audio.tmp.mp4", "audio.mp4", "lease"].contains(&entry.file_name().to_str()?)
                || !entry.file_type().ok()?.is_file() { return None; }
        }
    }
    Some(job.to_path_buf())
}

/// Check the exact consistent generated pair and all media references while holding a write transaction.
fn job_state(db: &Connection, id: i64, src: &Path, tmp: &Path, out: &Path, allowed_states: &[&str]) -> Option<String> {
    let rows = db.prepare("SELECT path,role,lifecycle FROM transcode_artifacts
        WHERE owner_media_id=?1 AND owner_source_path=?2 AND provenance='generated'
        AND path IN (?3,?4)").ok()?.query_map(rusqlite::params![id,src.to_str()?,tmp.to_str()?,out.to_str()?],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).ok()?
        .collect::<rusqlite::Result<Vec<_>>>().ok()?;
    if rows.len()!=2 || !rows.iter().any(|r| r.0==tmp.to_string_lossy() && r.1=="temp")
        || !rows.iter().any(|r| r.0==out.to_string_lossy() && r.1=="final")
        || rows[0].2!=rows[1].2 || !allowed_states.contains(&rows[0].2.as_str()) { return None; }
    let owner: i64 = db.query_row("SELECT COUNT(*) FROM media WHERE id=?1 AND path=?2",
        rusqlite::params![id,src.to_str()?],|row|row.get(0)).ok()?;
    let references: i64 = db.query_row("SELECT COUNT(*) FROM media WHERE path IN (?1,?2)
        OR transcode_path IN (?1,?2)",rusqlite::params![tmp.to_str()?,out.to_str()?],|row|row.get(0)).ok()?;
    let prefix=format!("{}/",tmp.parent()?.to_str()?);
    let upper=format!("{}0",tmp.parent()?.to_str()?);
    let registered: i64 = db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE path>=?1 AND path<?2",
        rusqlite::params![prefix,upper],|row|row.get(0)).ok()?;
    if owner!=1 || references!=0 || registered!=2 { return None; }
    Some(rows[0].2.clone())
}

/// Grant recovery authority only to the exact completed pair displaced by a known source replacement.
/// The caller holds an immediate transaction after clearing the old pointer; retain the returned lease
/// until commit so a live converter and a partially updated registry cannot acquire retirement authority.
#[allow(dead_code)]
pub(crate) fn retire_replaced_job(db: &Connection, db_path: &str, library: &Path, id: i64, src: &Path, out: &Path) -> rusqlite::Result<Option<File>> {
    retire_replaced_job_with_roots(db,db_path,&[library.to_path_buf()],id,src,out)
}

/// Check the complete root snapshot before granting retirement of an exact inactive, unreferenced generated pair.
/// The caller holds its immediate source-replacement transaction and retains the returned lease through commit.
/// Canonical cache overlap/unavailable roots and unqualified non-Linux recovery retain bytes and registry state.
pub(crate) fn retire_replaced_job_with_roots(db: &Connection, db_path: &str, roots: &[PathBuf],
    id: i64, src: &Path, out: &Path) -> rusqlite::Result<Option<File>> {
    if !cfg!(target_os = "linux") { return Ok(None); }
    let Some(parent)=out.parent() else { return Ok(None) };
    let tmp=parent.join("audio.tmp.mp4");
    let Some(job)=job_namespace(&tmp,out,id) else { return Ok(None) };
    let cache=std::env::var_os("FRAYD_TRANSCODES").map(PathBuf::from)
        .unwrap_or_else(|| Path::new(db_path).parent().unwrap_or(Path::new(".")).join("transcodes"));
    let Some(cache)=prepare_cache(&cache,roots) else { return Ok(None) };
    if job.parent().and_then(Path::parent)!=Some(cache.as_path()) { return Ok(None); }
    let lease_path=job.join("lease");
    let Ok(metadata)=std::fs::symlink_metadata(&lease_path) else { return Ok(None) };
    if !metadata.is_file() || metadata.file_type().is_symlink() { return Ok(None); }
    let Ok(lease)=OpenOptions::new().read(true).write(true).open(lease_path) else { return Ok(None) };
    if lease.try_lock().is_err() { return Ok(None); }
    if job_state(db,id,src,&tmp,out,&["completed"]).is_none() { return Ok(None); }
    let changed=db.execute("UPDATE transcode_artifacts SET lifecycle='retiring' WHERE path IN (?1,?2) AND lifecycle='completed'",
        rusqlite::params![tmp.to_string_lossy(),out.to_string_lossy()])?;
    if changed!=2 || job_state(db,id,src,&tmp,out,&["retiring"]).is_none() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(Some(lease))
}

/// Retire a proven inactive/current finished worker under its lease; durable retiring survives filesystem/commit gaps.
/// Unknown and completed pairs never grant cleanup authority, even when currently unreferenced.
fn discard_failed_job(db: &mut Connection, id: i64, src: &Path, tmp: &Path, out: &Path, completed: Option<&Path>) -> bool {
    let retired = (|| {
        if completed.is_some_and(|path| path != out) { return None; }
        let job = job_namespace(tmp,out,id)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).ok()?;
        let state = job_state(&tx,id,src,tmp,out,&["pending","retiring"])?;
        if !job.exists() && state!="retiring" { return None; }
        if tx.execute("UPDATE transcode_artifacts SET lifecycle='retiring' WHERE path IN (?1,?2)",
            rusqlite::params![tmp.to_str()?,out.to_str()?]).ok()?!=2 { return None; }
        tx.commit().ok()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).ok()?;
        if job_state(&tx,id,src,tmp,out,&["pending","retiring"])?.as_str()!="retiring" { return None; }
        job_namespace(tmp,out,id)?;
        if job.exists() {
            for path in [tmp,out,&job.join("lease")] {
                match std::fs::remove_file(path) {
                    Ok(()) => (),
                    Err(error) if error.kind()==std::io::ErrorKind::NotFound => (),
                    Err(_) => return None,
                }
            }
            std::fs::remove_dir(&job).ok()?;
        }
        if tx.execute("DELETE FROM transcode_artifacts WHERE path IN (?1,?2) AND lifecycle='retiring'",
            rusqlite::params![tmp.to_str()?,out.to_str()?]).ok()?!=2 { return None; }
        tx.commit().ok()?;
        Some(())
    })();
    if retired.is_none() { tracing::warn!(id,"failed transcode cleanup held: ownership, references, filesystem or database unavailable"); }
    retired.is_some()
}

/// Recover only inactive known pending jobs on Linux; unsupported locks and historical unknown jobs retain provenance.
/// Startup and explicit scans invoke this path again after a database timeout; no age or PID grants ownership.
#[allow(dead_code)] // Single-root compatibility API.
pub fn recover_pending_jobs(db_path: &str, library: &Path) { recover_pending_jobs_with_roots(db_path,&[library.to_path_buf()]); }

/// Recover only known pending/retiring generated pairs under qualified Linux leases and the complete root snapshot.
/// Empty/unresolvable roots or either-direction canonical cache overlap stop before registry or file mutations.
/// Unsupported platforms and unknown/completed history remain unchanged; age or path shape alone grants no authority.
pub fn recover_pending_jobs_with_roots(db_path: &str, roots: &[PathBuf]) {
    if !cfg!(target_os = "linux") { return; }
    let cache = std::env::var_os("FRAYD_TRANSCODES").map(PathBuf::from)
        .unwrap_or_else(|| Path::new(db_path).parent().unwrap_or(Path::new(".")).join("transcodes"));
    if !cache.exists() { return; }
    let Some(cache) = prepare_cache(&cache,roots) else { return };
    let Ok(mut db) = crate::store::connect(db_path) else { return };
    let candidates = (|| {
        db.prepare("SELECT owner_media_id,owner_source_path,path,lifecycle FROM transcode_artifacts
            WHERE role='temp' AND provenance='generated' AND lifecycle IN ('pending','retiring')").ok()?
            .query_map([],|row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?))).ok()?
            .collect::<rusqlite::Result<Vec<_>>>().ok()
    })().unwrap_or_default();
    for (id,src,tmp,state) in candidates {
        let tmp=PathBuf::from(tmp);
        let Some(parent)=tmp.parent() else { continue };
        let out=parent.join("audio.mp4");
        let Some(job)=job_namespace(&tmp,&out,id) else { continue };
        if job.parent().and_then(Path::parent)!=Some(cache.as_path()) { continue; }
        if !job.exists() && state=="retiring" {
            discard_failed_job(&mut db,id,Path::new(&src),&tmp,&out,None); continue;
        }
        let lease_path=job.join("lease");
        // Retiring was committed only after proving no converter can still write. Its lease
        // may have been unlinked just before a crash; a new lease serializes remaining cleanup.
        let lease=match std::fs::symlink_metadata(&lease_path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() =>
                OpenOptions::new().read(true).write(true).open(&lease_path),
            Err(error) if error.kind()==std::io::ErrorKind::NotFound && state=="retiring" =>
                OpenOptions::new().create_new(true).read(true).write(true).open(&lease_path),
            _ => continue,
        };
        let Ok(lease)=lease else { continue };
        if lease.try_lock().is_err() { continue; }
        discard_failed_job(&mut db,id,Path::new(&src),&tmp,&out,None);
    }
}

/// Queue one source; codec detection and all conversion work use a worker connection.
#[allow(dead_code)] // Compatibility entrypoint for single-root callers.
pub fn maybe_enqueue(db_path: String, media_id: i64, src: PathBuf, library: PathBuf) { maybe_enqueue_with_roots(db_path,media_id,src,vec![library]); }

/// Serialize converter admission with root reconfiguration, retaining the worker's complete immutable root snapshot.
/// The shared BUSY gate admits one worker; cache checks precede registry/filesystem mutation and publication
/// still requires current file/database source versions plus the exact reservation lease. Non-Linux jobs
/// retain conversion support without acquiring Linux-only reclamation qualification.
pub fn maybe_enqueue_with_roots(db_path: String, media_id: i64, src: PathBuf, roots: Vec<PathBuf>) {
    let _admission=crate::ROOT_ADMISSION.lock().unwrap();
    if BUSY.swap(true, Ordering::SeqCst) { return; }
    std::thread::spawn(move || {
        let _reset = JobReset;
        run_job_with_roots(&db_path, media_id, &src, &roots);
    });
}

/// Resolve an existing media ID before queueing the same guarded worker path.
#[allow(dead_code)] // Compatibility entrypoint for single-root callers.
pub fn maybe_enqueue_for_existing(db_path: String, src: PathBuf, library: PathBuf) { maybe_enqueue_for_existing_with_roots(db_path,src,vec![library]); }

/// Resolve an existing media ID on a dedicated non-migrating connection, then use the same whole-root worker admission.
/// Passing all roots preserves cache-containment, source-version publication and lease guards for unchanged sources.
pub fn maybe_enqueue_for_existing_with_roots(db_path: String, src: PathBuf, roots: Vec<PathBuf>) {
    if BUSY.load(Ordering::SeqCst) { return; }
    let Ok(db) = crate::store::connect(&db_path) else { return };
    let Ok(id) = db.query_row("SELECT id FROM media WHERE path=?1 AND transcode_path IS NULL",
        [src.to_string_lossy().as_ref()], |r| r.get(0)) else { return };
    maybe_enqueue_with_roots(db_path, id, src, roots);
}

/// Capture the actual file's modification time and length without treating it as a content hash.
fn source_version(src: &Path) -> Option<(std::time::SystemTime,u64)> {
    let metadata=std::fs::metadata(src).ok()?;
    Some((metadata.modified().ok()?,metadata.len()))
}

/// A retained generated job without a completed publication must not acquire a second namespace.
/// The media ID is stable across source replacement; an old source path cannot prove this job inactive.
fn unfinished_generated_job(db: &Connection, media_id: i64) -> bool {
    db.query_row("SELECT EXISTS(SELECT 1 FROM transcode_artifacts
        WHERE owner_media_id=?1 AND provenance='generated'
        AND lifecycle IN ('unknown','pending','retiring'))",
        [media_id], |row| row.get::<_,bool>(0)).unwrap_or(true)
}

/// Publish only the converted file/database snapshot, after acquiring the bounded SQLite write lock.
/// Checking the actual source inside this transaction also covers time spent waiting for that lock.
fn publish_completed_job(db: &mut Connection, id: i64, src: &Path, completed: &Path,
    database_version: (Option<i64>,Option<i64>), file_version: (std::time::SystemTime,u64), lease: &JobLease) -> rusqlite::Result<bool> {
    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if lease.completed!=completed || source_version(src)!=Some(file_version) { return Ok(false); }
    let tmp=completed.parent().map(|job|job.join("audio.tmp.mp4")).ok_or(rusqlite::Error::InvalidQuery)?;
    let expected=if lease.reclaimable {"pending"} else {"unknown"};
    if job_state(&tx,id,src,&tmp,completed,&[expected]).as_deref()!=Some(expected) { return Ok(false); }
    if tx.execute("UPDATE transcode_artifacts SET lifecycle='completed' WHERE path IN (?1,?2) AND lifecycle=?3",
        rusqlite::params![tmp.to_string_lossy(),completed.to_string_lossy(),expected])?!=2 { return Ok(false); }
    let changed=tx.execute("UPDATE media SET transcode_path=?1 WHERE id=?2 AND path=?3
        AND mtime_secs IS ?4 AND size_bytes IS ?5 AND transcode_path IS NULL",
        rusqlite::params![completed.to_string_lossy(),id,src.to_string_lossy(),database_version.0,database_version.1])?;
    if changed!=1 { return Ok(false); }
    tx.commit()?;
    Ok(true)
}

/// Fail closed on database/cache/registry errors and persist the actual completed path.
#[cfg(test)]
fn run_job(db_path: &str, media_id: i64, src: &Path, library: &Path) {
    run_job_with_roots(db_path,media_id,src,&[library.to_path_buf()]);
}

/// Reopen the worker database and convert one indexed source under the admitted root snapshot.
/// Version mismatches, missing leases, or failed publication leave source bytes untouched.
fn run_job_with_roots(db_path: &str, media_id: i64, src: &Path, roots: &[PathBuf]) {
    recover_pending_jobs_with_roots(db_path,roots);
    let Ok(mut db) = crate::store::connect(db_path) else { return };
    let Ok(database_version)=db.query_row("SELECT mtime_secs,size_bytes FROM media WHERE id=?1 AND path=?2 AND transcode_path IS NULL",
        rusqlite::params![media_id,src.to_string_lossy()],|row|Ok((row.get::<_,Option<i64>>(0)?,row.get::<_,Option<i64>>(1)?))) else { return };
    let Some(file_version)=source_version(src) else { return };
    let seconds=file_version.0.duration_since(std::time::UNIX_EPOCH).map_or(0,|time|time.as_secs() as i64);
    if database_version.0.is_some_and(|mtime|mtime!=seconds)
        || database_version.1.is_some_and(|size|size!=file_version.1 as i64) {
        tracing::warn!(media_id,"transcode held: source awaits rescan"); return;
    }
    if !needs_transcode(&src.to_string_lossy()) { return; }
    let cache = std::env::var_os("FRAYD_TRANSCODES").map(PathBuf::from)
        .unwrap_or_else(|| Path::new(db_path).parent().unwrap_or(Path::new(".")).join("transcodes"));
    let Some((tmp,out,lease)) = reserve_job_with_roots(&mut db,&cache,roots,media_id,src,cfg!(target_os="linux")) else {
        tracing::warn!(media_id,"transcode held: cache or registry unavailable"); return;
    };
    let Some(completed) = transcode_to_browser_mp4(src,&tmp,&out,&lease) else {
        discard_failed_job(&mut db,media_id,src,&tmp,&out,None);
        return;
    };
    match publish_completed_job(&mut db,media_id,src,&completed,database_version,file_version,&lease) {
        Ok(true) => return,
        Ok(false) => tracing::warn!(media_id,"transcode publication held: source or database version changed"),
        Err(error) => tracing::warn!(%error,media_id,"transcode completed but publication failed"),
    }
    discard_failed_job(&mut db,media_id,src,&tmp,&out,Some(&completed));
}

#[cfg(all(test, target_os = "linux"))]
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
    const ARTIFACT_PLAN_QUERIES: [&str; 4] = [
        r#"INSERT OR IGNORE INTO transcode_artifacts
        (path,owner_media_id,owner_source_path,role,provenance)
        SELECT transcode_path,id,path,'final','legacy_reference' FROM media m
        WHERE transcode_path IS NOT NULL AND transcode_path != path
        AND (SELECT COUNT(*) FROM media x WHERE x.transcode_path=m.transcode_path)=1;"#,
        r#"SELECT COUNT(*) FROM transcode_artifacts a LEFT JOIN media m
        ON m.id=a.owner_media_id WHERE m.path IS NULL OR m.path != a.owner_source_path
        OR EXISTS (SELECT 1 FROM media other WHERE other.transcode_path=a.path AND other.id != a.owner_media_id)"#,
        r#"SELECT a.path FROM transcode_artifacts a JOIN media m
        ON m.id=a.owner_media_id AND m.path=a.owner_source_path WHERE a.path != m.path
        AND NOT EXISTS (SELECT 1 FROM media other WHERE other.transcode_path=a.path AND other.id != m.id)
        AND NOT (a.provenance='legacy_reference' AND EXISTS (SELECT 1 FROM media indexed_media WHERE indexed_media.path=a.path))"#,
        r#"DELETE FROM media WHERE path=?1
            AND EXISTS (SELECT 1 FROM transcode_artifacts a WHERE a.path=media.path AND a.provenance='generated')
            AND NOT EXISTS (SELECT 1 FROM progress WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM annotations WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM tracks WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM playlist_items WHERE media_id=media.id)
            AND NOT EXISTS (SELECT 1 FROM transcode_artifacts WHERE owner_media_id=media.id)"#,
    ];

    /// Observe bundled SQLite planning for the exact production reconciliation statements.
    fn artifact_query_plan(db: &Connection, sql: &str, path: Option<&str>) -> Vec<String> {
        let mut statement=db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        statement.query_map(rusqlite::params_from_iter(path),|row|row.get(3)).unwrap()
            .collect::<rusqlite::Result<Vec<String>>>().unwrap()
    }

    /// Keep correlated lookups indexed at realistic cardinality while retaining protected media identities.
    #[test]
    fn upstream_integration_cache_other_root_holds_reservation_recovery_and_retirement() {
        let fixture=std::env::temp_dir().join(format!("frayd-all-root-cache-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(fixture.join("a")).unwrap();std::fs::create_dir(fixture.join("b")).unwrap();
        let roots=vec![fixture.join("a"),fixture.join("b")];
        let path=fixture.join("db.sqlite");let mut db=crate::store::open(path.to_str().unwrap());
        let src=roots[0].join("film.mkv");std::fs::write(&src,b"source").unwrap();
        db.execute("INSERT INTO media(id,kind,path,title) VALUES(1,'movie',?1,'fixture')",[src.to_str().unwrap()]).unwrap();
        for cache in [roots[1].join("cache/nested"),fixture.clone()] {
            assert!(reserve_job_with_roots(&mut db,&cache,&roots,1,&src,true).is_none());
            assert!(!roots[1].join("cache").exists());
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
        let cache=fixture.join("transcodes");
        let (tmp,out,lease)=reserve_job_with_roots(&mut db,&cache,&roots,1,&src,true).unwrap();
        std::fs::write(&tmp,b"pending").unwrap();drop(lease);
        // Adding the former cache directory itself as a root holds every recovery byte/row.
        let blocked_roots=vec![roots[0].clone(),cache.clone()];
        recover_pending_jobs_with_roots(path.to_str().unwrap(),&blocked_roots);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE lifecycle='pending'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        db.execute("UPDATE transcode_artifacts SET lifecycle='completed'",[]).unwrap();
        assert!(retire_replaced_job_with_roots(&db,path.to_str().unwrap(),&blocked_roots,1,&src,&out).unwrap().is_none());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE lifecycle='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        assert!(prepare_cache(&cache,&blocked_roots).is_none());
        assert_eq!(std::fs::read(&tmp).unwrap(),b"pending");
        assert!(!out.exists());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn reconciliation_uses_indexed_lookups_and_preserves_referenced_rows_at_scale() {
        let db=crate::store::open(":memory:");
        // Model an existing populated database that predates both additive indexes.
        db.execute_batch("DROP INDEX IF EXISTS idx_media_transcode_path; DROP INDEX IF EXISTS idx_transcode_artifacts_owner;").unwrap();
        db.execute_batch("WITH RECURSIVE ids(id) AS (SELECT 1 UNION ALL SELECT id+1 FROM ids WHERE id<3000)
            INSERT INTO media(id,kind,path,title,transcode_path)
            SELECT id,'movie','/source/'||id||'.mkv','fixture','/cache/'||id||'/audio.mp4' FROM ids;
            INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance)
            SELECT '/cache/'||id||'/audio.tmp.mp4',id,path,'temp','generated' FROM media;
            INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance)
            SELECT transcode_path,id,path,'final','generated' FROM media;
            INSERT INTO media(id,kind,path,title) VALUES
                (3001,'movie','/cache/1/audio.tmp.mp4','unreferenced'),
                (3002,'movie','/cache/2/audio.tmp.mp4','saved progress'),
                (3003,'movie','/cache/3/audio.tmp.mp4','owns another artifact'),
                (3004,'movie','/cache/4/audio.tmp.mp4','conflicting playback reference'),
                (3005,'movie','/legacy/genuine.mp4','legacy genuine source');
            INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture');
            INSERT INTO progress(identity_id,media_id,position) VALUES(1,3002,42);
            INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance) VALUES('/cache/retained-child.mp4',3003,'/cache/3/audio.tmp.mp4','final','generated');
            UPDATE media SET transcode_path='/cache/4/audio.tmp.mp4' WHERE id=3000;
            UPDATE media SET transcode_path='/legacy/genuine.mp4' WHERE id=5;").unwrap();
        // Apply the migration over existing rows, then prove repeated initialization is idempotent.
        for _ in 0..2 { db.execute_batch(include_str!("../schema.sql")).unwrap(); }
        let counts=db.query_row("SELECT (SELECT COUNT(*) FROM media WHERE transcode_path IS NOT NULL),
            (SELECT COUNT(*) FROM media),(SELECT COUNT(*) FROM transcode_artifacts),(SELECT COUNT(*) FROM progress)",[],
            |row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?))).unwrap();
        assert_eq!(counts,(3000,3005,6001,1));
        println!("SQLite {} fixture converted/media/artifacts/progress={counts:?}",
            db.query_row("SELECT sqlite_version()",[],|row|row.get::<_,String>(0)).unwrap());
        let plans=ARTIFACT_PLAN_QUERIES.iter().enumerate().map(|(index,sql)| {
            let plan=artifact_query_plan(&db,sql,if index==3 {Some("/cache/1/audio.tmp.mp4")} else {None});
            println!("PLAN {} SQL={} DETAILS={plan:?}",["backfill","conflicts","ownership","cleanup"][index],sql);
            plan
        }).collect::<Vec<_>>();
        for (index,alias,column) in [(0,"x","transcode_path"),(1,"other","transcode_path"),
            (2,"other","transcode_path"),(2,"indexed_media","path"),(3,"transcode_artifacts","owner_media_id")] {
            assert!(plans[index].iter().any(|line|line.contains(&format!("SEARCH {alias} USING"))
                && line.contains(&format!("({column}=?)"))),"unindexed correlated lookup: {:?}",plans[index]);
        }
        assert_eq!(db.execute(ARTIFACT_PLAN_QUERIES[0],[]).unwrap(),1);
        assert_eq!(db.query_row(ARTIFACT_PLAN_QUERIES[1],[],|row|row.get::<_,i64>(0)).unwrap(),1);
        let ownership_count=db.prepare(ARTIFACT_PLAN_QUERIES[2]).unwrap().query_map([],|row|row.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap().len();
        assert_eq!(ownership_count,6000);
        assert_eq!(db.execute(ARTIFACT_PLAN_QUERIES[3],["/cache/1/audio.tmp.mp4"]).unwrap(),1);
        for _ in 0..2 { assert_eq!(reconcile_artifacts(&db).len(),6000); }
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|row|row.get::<_,i64>(0)).unwrap(),3004);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|row|row.get::<_,i64>(0)).unwrap(),6002);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media WHERE id BETWEEN 3002 AND 3005",[],|row|row.get::<_,i64>(0)).unwrap(),4);
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=3002",[],|row|row.get::<_,f64>(0)).unwrap(),42.0);
        println!("RESULTS backfill_inserted=1 conflicts=1 owned_paths=6000 cleanup_removed=1 final_media=3004 final_artifacts=6002 protected_media=4 progress=42 repeated_reconciliation=2");
    }

    /// Reject preexisting namespace objects before any outside write or ownership publication.
    #[test]
    fn media_namespace_symlinks_and_files_cannot_redirect_job_reservation() {
        use std::os::unix::fs::symlink;
        for as_symlink in [true,false] {
            let fixture=Fixture::new(); let mut db=crate::store::open(":memory:");
            let src=fixture.source("film.mkv"); insert(&db,1,&src);
            let cache=fixture.0.join("cache"); let outside=fixture.0.join("outside");
            std::fs::create_dir(&cache).unwrap(); std::fs::create_dir(&outside).unwrap();
            std::fs::write(outside.join("sentinel"),b"untouched").unwrap();
            if as_symlink { symlink(&outside,cache.join("1")).unwrap(); }
            else { std::fs::write(cache.join("1"),b"preexisting").unwrap(); }
            assert!(reserve_job(&mut db,&cache,&fixture.0.join("library"),1,&src).is_none(),"untrusted media namespace accepted");
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(),1);
            assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(),b"untouched");
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
    }
    /// Repeated failed workers must release their own records without touching another job.
    #[test]
    fn failed_conversion_jobs_do_not_accumulate_registry_or_empty_namespaces() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(root)=std::env::var_os("FRAYD_TEST_FAILED_JOBS_ROOT") {
            let root=PathBuf::from(root); let db_path=root.join("db.sqlite"); let src=root.join("library/invalid.mkv");
            for _ in 0..2 { run_job(db_path.to_str().unwrap(),1,&src,&root.join("library")); }
            let db=crate::store::connect(db_path.to_str().unwrap()).unwrap();
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),0,"failed jobs retained artifact records");
            assert_eq!(db.query_row("SELECT transcode_path FROM media WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap(),None);
            let jobs=root.join("cache/1");
            assert_eq!(std::fs::read_dir(jobs).map(|entries|entries.count()).unwrap_or(0),0);
            assert_eq!(std::fs::read(src).unwrap(),b"original");
            return;
        }
        let fixture=Fixture::new(); let src=fixture.source("invalid.mkv");
        let db=crate::store::open(fixture.0.join("db.sqlite").to_str().unwrap()); insert(&db,1,&src); drop(db);
        let bin=fixture.0.join("bin");std::fs::create_dir(&bin).unwrap();
        let probe=bin.join("ffprobe"); std::fs::write(&probe,b"#!/bin/sh\nprintf 'ac3\\n'\n").unwrap();
        std::fs::set_permissions(&probe,std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut paths=vec![bin];paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        let output=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","transcode::tests::failed_conversion_jobs_do_not_accumulate_registry_or_empty_namespaces","--nocapture"])
            .env("FRAYD_TEST_FAILED_JOBS_ROOT",&fixture.0).env("FRAYD_TRANSCODES",fixture.0.join("cache"))
            .env("PATH",std::env::join_paths(paths).unwrap()).output().unwrap();
        assert!(output.status.success(),"isolated failed-job worker: {}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    }
    /// A conversion must publish only its captured file/database version; unchanged jobs still complete.
    #[test]
    fn in_flight_conversion_rejects_changed_source_or_database_version() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(root)=std::env::var_os("FRAYD_TEST_VERSION_ROOT") {
            let root=PathBuf::from(root); let mode=std::env::var("FRAYD_TEST_VERSION_MODE").unwrap();
            let db_path=root.join("db.sqlite"); let src=root.join("library/film.mkv");
            let worker_db=db_path.clone();let worker_src=src.clone();let library=root.join("library");
            let worker=std::thread::spawn(move || run_job(worker_db.to_str().unwrap(),1,&worker_src,&library));
            let mut ready=false;
            for _ in 0..500 { if root.join("ready").exists() {ready=true;break;} std::thread::sleep(std::time::Duration::from_millis(5)); }
            if ready {
                if mode=="source" {std::fs::write(&src,b"replaced while ffmpeg was running").unwrap();}
                if mode=="database" {let db=crate::store::connect(db_path.to_str().unwrap()).unwrap();db.execute("UPDATE media SET mtime_secs=mtime_secs+1 WHERE id=1",[]).unwrap();}
            }
            let mut locker=crate::store::connect(db_path.to_str().unwrap()).unwrap();
            let lock=if ready && mode=="locked-source" {
                Some(locker.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap())
            } else {None};
            std::fs::write(root.join("release"),b"go").unwrap();
            if let Some(lock)=lock {
                let out:String=lock.query_row("SELECT path FROM transcode_artifacts WHERE role='final'",[],|r|r.get(0)).unwrap();
                let mut completed=false;
                for _ in 0..500 {if Path::new(&out).exists() {completed=true;break;}std::thread::sleep(std::time::Duration::from_millis(5));}
                if completed {std::fs::write(&src,b"replaced while ffmpeg was running").unwrap();}
                drop(lock);
                worker.join().unwrap();
                assert!(completed,"worker did not finish conversion before lock release");
            } else {worker.join().unwrap();}
            assert!(ready,"worker did not reach converter");
            let db=crate::store::connect(db_path.to_str().unwrap()).unwrap();
            let playback=db.query_row("SELECT transcode_path FROM media WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap();
            if mode=="unchanged" {
                assert_eq!(std::fs::read(playback.unwrap()).unwrap(),b"converted fixture");
                assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),2);
            } else {
                assert_eq!(playback,None,"stale conversion published after {mode} version changed");
                assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
                assert_eq!(std::fs::read_dir(root.join("cache/1")).unwrap().count(),0);
            }
            let want:&[u8]=if mode=="source" || mode=="locked-source" {b"replaced while ffmpeg was running"} else {b"original"};
            assert_eq!(std::fs::read(src).unwrap(),want); return;
        }
        let mut failures=Vec::new();
        for mode in ["source","database","locked-source","unchanged"] {
            let fixture=Fixture::new(); let src=fixture.source("film.mkv");
            let db=crate::store::open(fixture.0.join("db.sqlite").to_str().unwrap());insert(&db,1,&src);
            let meta=std::fs::metadata(&src).unwrap();
            db.execute("UPDATE media SET mtime_secs=?1,size_bytes=?2 WHERE id=1",rusqlite::params![meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,meta.len() as i64]).unwrap();drop(db);
            let bin=fixture.0.join("bin");std::fs::create_dir(&bin).unwrap();
            for (name,script) in [("ffprobe",b"#!/bin/sh\nprintf 'ac3\\n'\n".as_slice()),("ffmpeg",b"#!/bin/sh\nprintf ready > \"$FRAYD_TEST_VERSION_READY\"\nwhile [ ! -e \"$FRAYD_TEST_VERSION_RELEASE\" ]; do /bin/sleep 0.01; done\nfor destination do :; done\nprintf 'converted fixture' > \"$destination\"\n".as_slice())] {
                let executable=bin.join(name);std::fs::write(&executable,script).unwrap();std::fs::set_permissions(executable,std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let mut paths=vec![bin];paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
            let output=Command::new(std::env::current_exe().unwrap()).args(["--exact","transcode::tests::in_flight_conversion_rejects_changed_source_or_database_version","--nocapture"])
                .env("FRAYD_TEST_VERSION_ROOT",&fixture.0).env("FRAYD_TEST_VERSION_MODE",mode)
                .env("FRAYD_TEST_VERSION_READY",fixture.0.join("ready")).env("FRAYD_TEST_VERSION_RELEASE",fixture.0.join("release"))
                .env("FRAYD_TRANSCODES",fixture.0.join("cache")).env("PATH",std::env::join_paths(paths).unwrap()).output().unwrap();
            println!("VERSION_CASE {mode}: {}",output.status);
            if !output.status.success() {failures.push(format!("{mode}: {}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr)));}
        }
        assert!(failures.is_empty(),"{}",failures.join("\n"));
    }
    /// A completed file whose pointer cannot be published must not accumulate across retries.
    #[test]
    fn failed_publication_retires_unreferenced_current_job_outputs() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(root)=std::env::var_os("FRAYD_TEST_PUBLICATION_ROOT") {
            let root=PathBuf::from(root); let db_path=root.join("db.sqlite"); let src=root.join("library/film.mkv");
            for _ in 0..2 { run_job(db_path.to_str().unwrap(),1,&src,&root.join("library")); }
            let db=crate::store::connect(db_path.to_str().unwrap()).unwrap();
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),0,"failed publication retained unused cache files");
            assert_eq!(std::fs::read_dir(root.join("cache/1")).map(|entries|entries.count()).unwrap_or(0),0);
            assert_eq!(db.query_row("SELECT transcode_path FROM media WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap(),None);
            assert_eq!(std::fs::read(src).unwrap(),b"original");
            return;
        }
        let fixture=Fixture::new(); let src=fixture.source("film.mkv");
        let db=crate::store::open(fixture.0.join("db.sqlite").to_str().unwrap()); insert(&db,1,&src);
        db.execute_batch("CREATE TRIGGER fail_publication BEFORE UPDATE OF transcode_path ON media BEGIN SELECT RAISE(FAIL,'fixture publication refused'); END;").unwrap();drop(db);
        let bin=fixture.0.join("bin");std::fs::create_dir(&bin).unwrap();
        for (name,script) in [("ffprobe",b"#!/bin/sh\nprintf 'ac3\\n'\n".as_slice()),("ffmpeg",b"#!/bin/sh\nfor destination do :; done\nprintf 'converted fixture' > \"$destination\"\n".as_slice())] {
            let executable=bin.join(name);std::fs::write(&executable,script).unwrap();
            std::fs::set_permissions(executable,std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut paths=vec![bin];paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        let output=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","transcode::tests::failed_publication_retires_unreferenced_current_job_outputs","--nocapture"])
            .env("FRAYD_TEST_PUBLICATION_ROOT",&fixture.0).env("FRAYD_TRANSCODES",fixture.0.join("cache"))
            .env("PATH",std::env::join_paths(paths).unwrap()).output().unwrap();
        assert!(output.status.success(),"isolated publication worker: {}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    }
    /// Interrupted and write-timeout reservations need a reachable recovery pass after their workers exit.
    #[test]
    fn interrupted_and_timeout_reservations_recover_on_rescan() {
        if let Some(root) = std::env::var_os("FRAYD_TEST_INTERRUPTED_RESERVATION_ROOT") {
            let root = PathBuf::from(root);
            let mut db = crate::store::connect(root.join("db.sqlite").to_str().unwrap()).unwrap();
            let (tmp, _, _lease) = reserve_job(&mut db, &root.join("transcodes"), &root.join("library"), 1, &root.join("library/film.mkv")).unwrap();
            std::fs::write(tmp, b"interrupted partial").unwrap();
            std::fs::write(root.join("ready"), b"reserved").unwrap();
            loop { std::thread::park(); }
        }
        let fixture = Fixture::new();
        let db_path = fixture.0.join("db.sqlite");
        let src = fixture.source("film.mkv");
        let mut db = crate::store::open(db_path.to_str().unwrap());
        insert(&db, 1, &src);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "transcode::tests::interrupted_and_timeout_reservations_recover_on_rescan"])
            .env("FRAYD_TEST_INTERRUPTED_RESERVATION_ROOT", &fixture.0)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
        let ready = (0..500).any(|_| {
            if fixture.0.join("ready").exists() { return true; }
            std::thread::sleep(std::time::Duration::from_millis(5)); false
        });
        child.kill().unwrap(); let status = child.wait().unwrap();
        assert!(ready && !status.success(), "worker did not reach its durable reservation before interruption");
        let interrupted: String = db.query_row("SELECT path FROM transcode_artifacts WHERE role='temp'", [], |r| r.get(0)).unwrap();
        let (tmp, out, lease) = reserve_job(&mut db, &fixture.0.join("transcodes"), &fixture.0.join("library"), 1, &src).unwrap();
        std::fs::write(&tmp, b"timeout partial").unwrap();
        let mut writer = crate::store::connect(db_path.to_str().unwrap()).unwrap();
        let lock = writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        assert!(!discard_failed_job(&mut db, 1, &src, &tmp, &out, None));
        drop(lock); drop(writer); drop(db);
        assert_eq!(std::fs::read(&interrupted).unwrap(), b"interrupted partial");
        assert_eq!(std::fs::read(&tmp).unwrap(), b"timeout partial");
        drop(lease);
        // Reopen the database as a restarted application would, then use its real scan entry point.
        let db = crate::store::connect(db_path.to_str().unwrap()).unwrap();
        crate::scanner::scan(&db, db_path.to_string_lossy().into_owned(), fixture.0.join("library"), None);
        let remaining = db.query_row("SELECT COUNT(*) FROM transcode_artifacts", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), b"original");
        assert_eq!(db.query_row("SELECT transcode_path FROM media WHERE id=1", [], |r| r.get::<_, Option<String>>(0)).unwrap(), None);
        assert_eq!(remaining, 0, "rescan retained interrupted and DB-timeout reservations plus both partial files");
        assert!(!Path::new(&interrupted).exists() && !tmp.exists());
    }

    /// An orphan converter inheriting stdin retains the OS lease until its own exit, across real recovery calls.
    #[test]
    fn orphan_converter_lease_blocks_recovery_until_converter_exit() {
        if let Some(root)=std::env::var_os("FRAYD_TEST_ORPHAN_LEASE_ROOT") {
            let root=PathBuf::from(root);
            if std::env::var("FRAYD_TEST_ORPHAN_LEASE_MODE").unwrap()=="converter" {
                std::fs::write(root.join("converter-ready"),b"ready").unwrap();
                for _ in 0..500 {
                    if root.join("converter-exit").exists() {return;}
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                panic!("fixture converter exit timed out");
            }
            let path=std::fs::read_to_string(root.join("lease-path")).unwrap();
            let lease=OpenOptions::new().read(true).write(true).open(path).unwrap();lease.try_lock().unwrap();
            let mut converter=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","transcode::tests::orphan_converter_lease_blocks_recovery_until_converter_exit"])
                .env("FRAYD_TEST_ORPHAN_LEASE_MODE","converter")
                .stdin(Stdio::from(lease.try_clone().unwrap())).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            assert!(converter.wait().unwrap().success());
            return;
        }
        let fixture=Fixture::new();let path=fixture.0.join("db.sqlite");let src=fixture.source("film.mkv");
        let mut db=crate::store::open(path.to_str().unwrap());insert(&db,1,&src);
        let (tmp,out,lease)=reserve_job(&mut db,&fixture.0.join("transcodes"),&fixture.0.join("library"),1,&src).unwrap();
        std::fs::write(&tmp,b"active converter partial").unwrap();
        std::fs::write(fixture.0.join("lease-path"),tmp.parent().unwrap().join("lease").to_str().unwrap()).unwrap();drop(lease);
        let mut worker=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","transcode::tests::orphan_converter_lease_blocks_recovery_until_converter_exit"])
            .env("FRAYD_TEST_ORPHAN_LEASE_ROOT",&fixture.0).env("FRAYD_TEST_ORPHAN_LEASE_MODE","worker")
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let ready=(0..500).any(|_| {if fixture.0.join("converter-ready").exists(){return true;}std::thread::sleep(std::time::Duration::from_millis(5));false});
        worker.kill().unwrap();worker.wait().unwrap();assert!(ready,"converter did not inherit its lease");
        recover_pending_jobs(path.to_str().unwrap(),&fixture.0.join("library"));
        assert_eq!(std::fs::read(&tmp).unwrap(),b"active converter partial");
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        std::fs::write(fixture.0.join("converter-exit"),b"exit").unwrap();
        let probe=File::open(tmp.parent().unwrap().join("lease")).unwrap();
        let released=(0..500).any(|_| {if probe.try_lock().is_ok(){return true;}std::thread::sleep(std::time::Duration::from_millis(5));false});
        assert!(released,"converter did not release its inherited lease");drop(probe);
        recover_pending_jobs(path.to_str().unwrap(),&fixture.0.join("library"));
        assert!(!tmp.exists() && !out.exists() && !tmp.parent().unwrap().exists());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }

    /// Unqualified platforms keep conversion and fresh-token publication, but never gain automatic unknown-job cleanup.
    #[test]
    fn unqualified_recovery_preserves_conversion_and_retains_unknown_failures() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(root)=std::env::var_os("FRAYD_TEST_UNQUALIFIED_ROOT") {
            let root=PathBuf::from(root);let mode=std::env::var("FRAYD_TEST_UNQUALIFIED_MODE").unwrap();
            let path=root.join("db.sqlite");let src=root.join("library/film.mkv");
            let mut db=crate::store::open(path.to_str().unwrap());insert(&db,1,&src);
            let (tmp,out,lease)=reserve_job_with_recovery(&mut db,&root.join("transcodes"),&root.join("library"),1,&src,false).unwrap();
            assert!(!lease.reclaimable);
            let converted=transcode_to_browser_mp4(&src,&tmp,&out,&lease);
            assert_eq!(converted.is_some(),mode=="success");
            let other_src=root.join("library/other.mkv");std::fs::write(&other_src,b"other source").unwrap();insert(&db,2,&other_src);
            let (other_tmp,other_out,other)=reserve_job_with_recovery(&mut db,&root.join("transcodes"),&root.join("library"),2,&other_src,false).unwrap();
            assert!(!publish_completed_job(&mut db,1,&src,&out,(None,None),source_version(&src).unwrap(),&other).unwrap(),"reservation token authorized another job");
            if mode=="success" {
                assert!(publish_completed_job(&mut db,1,&src,&out,(None,None),source_version(&src).unwrap(),&lease).unwrap());
                assert_eq!(std::fs::read(&out).unwrap(),b"synthetic converted bytes");
            } else {
                assert!(!discard_failed_job(&mut db,1,&src,&tmp,&out,None));
                assert_eq!(std::fs::read(&tmp).unwrap(),b"synthetic converted bytes");
            }
            drop(lease);drop(other);recover_pending_jobs(path.to_str().unwrap(),&root.join("library"));
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),4);
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE lifecycle='unknown'",[],|r|r.get::<_,i64>(0)).unwrap(),if mode=="success"{2}else{4});
            assert!(other_tmp.parent().unwrap().exists() && !other_out.exists());
            assert_eq!(db.query_row("SELECT transcode_path IS NOT NULL FROM media WHERE id=1",[],|r|r.get::<_,bool>(0)).unwrap(),mode=="success");
            return;
        }
        for mode in ["success","failure"] {
            let fixture=Fixture::new();fixture.source("film.mkv");let bin=fixture.0.join("bin");std::fs::create_dir(&bin).unwrap();
            let executable=bin.join("ffmpeg");
            std::fs::write(&executable,b"#!/bin/sh\nfor destination do :; done\nprintf 'synthetic converted bytes' > \"$destination\"\n[ \"$FRAYD_TEST_UNQUALIFIED_MODE\" = success ]\n").unwrap();
            std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o755)).unwrap();
            let result=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","transcode::tests::unqualified_recovery_preserves_conversion_and_retains_unknown_failures","--nocapture"])
                .env("FRAYD_TEST_UNQUALIFIED_ROOT",&fixture.0).env("FRAYD_TEST_UNQUALIFIED_MODE",mode).env("PATH",&bin).output().unwrap();
            assert!(result.status.success(),"unqualified {mode}: {}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
        }
    }

    /// Recovery retains active, historical, completed, referenced, redirected, and uncertain namespaces.
    #[test]
    fn recovery_retains_every_unqualified_job_and_retires_only_known_pending_pairs() {
        use std::os::unix::fs::symlink;
        for mode in ["active","unknown","completed","legacy","reference","extra","symlink","conflict","pending-missing","retiring-missing","retiring-unlinked-lease","pending"] {
            let fixture=Fixture::new(); let path=fixture.0.join("db.sqlite");
            let mut db=crate::store::open(path.to_str().unwrap());let src=fixture.source("film.mkv");insert(&db,1,&src);
            let (tmp,out,lease)=reserve_job(&mut db,&fixture.0.join("transcodes"),&fixture.0.join("library"),1,&src).unwrap();
            std::fs::write(&tmp,b"partial").unwrap();
            if ["unknown","completed"].contains(&mode) {
                db.execute("UPDATE transcode_artifacts SET lifecycle=?1",[mode]).unwrap();std::fs::write(&out,b"retained final").unwrap();
            }
            if mode=="legacy" { db.execute("UPDATE transcode_artifacts SET provenance='legacy_reference'",[]).unwrap(); }
            if mode=="reference" {insert(&db,2,&tmp);db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture'); INSERT INTO progress(identity_id,media_id,position) VALUES(1,2,42);").unwrap();}
            if mode=="extra" {std::fs::write(tmp.parent().unwrap().join("unregistered"),b"genuine").unwrap();}
            if mode=="conflict" {let other=fixture.source("other.mkv");insert(&db,2,&other);db.execute("UPDATE transcode_artifacts SET owner_media_id=2 WHERE role='final'",[]).unwrap();}
            if mode=="symlink" {let moved=fixture.0.join("moved");std::fs::rename(tmp.parent().unwrap(),&moved).unwrap();symlink(moved,tmp.parent().unwrap()).unwrap();}
            if mode=="retiring-unlinked-lease" {
                db.execute("UPDATE transcode_artifacts SET lifecycle='retiring'",[]).unwrap();
                std::fs::remove_file(tmp.parent().unwrap().join("lease")).unwrap();
            }
            if mode.ends_with("-missing") {
                if mode=="retiring-missing" {db.execute("UPDATE transcode_artifacts SET lifecycle='retiring'",[]).unwrap();}
                std::fs::remove_dir_all(tmp.parent().unwrap()).unwrap();
            }
            let held=if mode=="active" {Some(lease)} else {drop(lease);None};
            for _ in 0..2 {recover_pending_jobs(path.to_str().unwrap(),&fixture.0.join("library"));}
            let retired=["pending","retiring-missing","retiring-unlinked-lease"].contains(&mode);
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),if retired {0}else{2},"{mode}");
            assert_eq!(std::fs::read(&src).unwrap(),b"original");
            if !retired && !mode.ends_with("-missing") {assert_eq!(std::fs::read(&tmp).unwrap(),b"partial","{mode}");}
            if ["unknown","completed"].contains(&mode) {assert_eq!(std::fs::read(&out).unwrap(),b"retained final");}
            if mode=="reference" {assert_eq!(db.query_row("SELECT position FROM progress",[],|r|r.get::<_,f64>(0)).unwrap(),42.0);}
            drop(held);
        }
    }

    /// Completion and publication commit together; failed retirement can resume after its directory is gone.
    #[test]
    fn lifecycle_publication_is_atomic_and_retiring_commit_failures_remain_recoverable() {
        for mode in ["published","publication-failure","publication-ignore","lifecycle-failure","lifecycle-ignore","retirement-failure","retirement-ignore"] {
            let fixture=Fixture::new();let path=fixture.0.join("db.sqlite");let src=fixture.source("film.mkv");
            let mut db=crate::store::open(path.to_str().unwrap());insert(&db,1,&src);
            let (tmp,out,lease)=reserve_job(&mut db,&fixture.0.join("transcodes"),&fixture.0.join("library"),1,&src).unwrap();
            std::fs::write(&out,b"completed bytes").unwrap();
            if mode=="publication-failure" {db.execute_batch("CREATE TRIGGER reject_publication BEFORE UPDATE OF transcode_path ON media BEGIN SELECT RAISE(FAIL,'fixture refusal'); END;").unwrap();}
            if mode=="lifecycle-failure" {db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF lifecycle ON transcode_artifacts WHEN NEW.lifecycle='completed' BEGIN SELECT RAISE(FAIL,'fixture refusal'); END;").unwrap();}
            if mode=="publication-ignore" {db.execute_batch("CREATE TRIGGER reject_publication BEFORE UPDATE OF transcode_path ON media BEGIN SELECT RAISE(IGNORE); END;").unwrap();}
            if mode=="lifecycle-ignore" {db.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF lifecycle ON transcode_artifacts WHEN NEW.lifecycle='completed' AND OLD.role='final' BEGIN SELECT RAISE(IGNORE); END;").unwrap();}
            if mode.starts_with("retirement-") {
                let trigger=if mode=="retirement-ignore" {"CREATE TRIGGER reject_retirement BEFORE DELETE ON transcode_artifacts WHEN OLD.role='final' BEGIN SELECT RAISE(IGNORE); END;"}
                    else {"CREATE TRIGGER reject_retirement BEFORE DELETE ON transcode_artifacts BEGIN SELECT RAISE(FAIL,'fixture refusal'); END;"};
                db.execute_batch(trigger).unwrap();
                assert!(!discard_failed_job(&mut db,1,&src,&tmp,&out,Some(&out)));
                assert!(!tmp.parent().unwrap().exists());
                assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE lifecycle='retiring'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
                db.execute_batch("DROP TRIGGER reject_retirement;").unwrap();
            } else {
                let result=publish_completed_job(&mut db,1,&src,&out,(None,None),source_version(&src).unwrap(),&lease);
                assert_eq!(result.unwrap_or(false),mode=="published");
                assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts WHERE lifecycle='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),if mode=="published"{2}else{0});
                let pointer=db.query_row("SELECT transcode_path FROM media WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap();
                assert_eq!(pointer.is_some(),mode=="published");
                if mode=="published" {db.execute("UPDATE media SET transcode_path=NULL WHERE id=1",[]).unwrap();}
            }
            drop(lease);recover_pending_jobs(path.to_str().unwrap(),&fixture.0.join("library"));
            assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),if mode=="published"{2}else{0});
            if mode=="published" {assert_eq!(std::fs::read(&out).unwrap(),b"completed bytes");}
        }
    }

    /// A busy database must retain provenance and bytes until the write lock becomes available.
    #[test]
    fn failed_job_cleanup_holds_all_state_during_database_lock() {
        let fixture=Fixture::new(); let path=fixture.0.join("db.sqlite");
        let mut db=crate::store::open(path.to_str().unwrap()); let src=fixture.source("film.mkv"); insert(&db,1,&src);
        let (tmp,out,_lease)=reserve_job(&mut db,&fixture.0.join("cache"),&fixture.0.join("library"),1,&src).unwrap();
        std::fs::write(&tmp,b"partial owned output").unwrap();
        let mut writer=crate::store::connect(path.to_str().unwrap()).unwrap();
        let lock=writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
        assert!(!discard_failed_job(&mut db,1,&src,&tmp,&out,None));
        assert_eq!(std::fs::read(&tmp).unwrap(),b"partial owned output");
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        drop(lock);
        assert!(discard_failed_job(&mut db,1,&src,&tmp,&out,None));
        assert!(!tmp.parent().unwrap().exists());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    /// Retire only the failed job, preserving unrelated, legacy, and user-referenced artifacts.
    #[test]
    fn failed_job_cleanup_preserves_neighbors_and_holds_unproven_or_referenced_jobs() {
        let fixture=Fixture::new(); let mut db=crate::store::open(":memory:");
        let src=fixture.source("film.mkv"); insert(&db,1,&src);
        let cache=fixture.0.join("cache"); let library=fixture.0.join("library");
        let (tmp,out,_lease)=reserve_job(&mut db,&cache,&library,1,&src).unwrap();
        let (neighbor_tmp,neighbor_out,_neighbor_lease)=reserve_job(&mut db,&cache,&library,1,&src).unwrap();
        std::fs::write(&tmp,b"partial").unwrap();std::fs::write(&neighbor_tmp,b"neighbor").unwrap();
        assert!(discard_failed_job(&mut db,1,&src,&tmp,&out,None));
        assert_eq!(std::fs::read(&neighbor_tmp).unwrap(),b"neighbor");
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        insert(&db,2,&neighbor_tmp);
        assert!(!discard_failed_job(&mut db,1,&src,&neighbor_tmp,&neighbor_out,None));
        assert_eq!(std::fs::read(&neighbor_tmp).unwrap(),b"neighbor");
        db.execute("DELETE FROM media WHERE id=2",[]).unwrap();
        db.execute("UPDATE transcode_artifacts SET provenance='legacy_reference'",[]).unwrap();
        assert!(!discard_failed_job(&mut db,1,&src,&neighbor_tmp,&neighbor_out,None));
        assert_eq!(std::fs::read(&neighbor_tmp).unwrap(),b"neighbor");
        assert_eq!(db.query_row("SELECT COUNT(*) FROM transcode_artifacts",[],|r|r.get::<_,i64>(0)).unwrap(),2);
        std::fs::rename(&neighbor_tmp,&neighbor_out).unwrap();
        assert!(!discard_failed_job(&mut db,1,&src,&neighbor_tmp,&neighbor_out,Some(&neighbor_out)));
        db.execute("UPDATE transcode_artifacts SET provenance='generated'",[]).unwrap();
        insert(&db,2,&neighbor_out);
        assert!(!discard_failed_job(&mut db,1,&src,&neighbor_tmp,&neighbor_out,Some(&neighbor_out)));
        assert_eq!(std::fs::read(&neighbor_out).unwrap(),b"neighbor");
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
        assert_eq!(std::fs::read_dir(cache.join("1")).map(|entries|entries.count()).unwrap_or(0),0);
        let bad_cache=fixture.0.join("file"); std::fs::write(&bad_cache,b"file").unwrap();
        assert!(reserve_job(&mut db,&bad_cache,&library,1,&src).is_none());
        assert!(!library.join("film.browser.mp4").exists());
    }
    /// Ensure a failed external conversion leaves original bytes and playback selection intact.
    #[test]
    fn failed_conversion_does_not_publish_or_touch_originals() {
        let fixture=Fixture::new(); let mut db=crate::store::open(":memory:");
        let src=fixture.source("invalid.mkv"); insert(&db,1,&src);
        let (tmp,out,_lease)=reserve_job(&mut db,&fixture.0.join("cache"),&fixture.0.join("library"),1,&src).unwrap();
        assert!(transcode_to_browser_mp4(&src,&tmp,&out,&_lease).is_none());
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
            db.execute("INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance) VALUES(?1,1,?2,'temp','generated')",rusqlite::params![path.to_str().unwrap(),src.to_str().unwrap()]).unwrap();
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
