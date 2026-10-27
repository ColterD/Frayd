//! On-demand transcoding for browser-incompatible codecs (AC3/DTS/etc).
//! Video stream is copied; audio re-encoded to AAC. Runs in background after
//! scan; clients fall back to direct stream until ready. One job at a time.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

pub static BUSY: AtomicBool = AtomicBool::new(false);

/// Does any audio track use a codec browsers can't decode?
#[allow(dead_code)] // kept for future client-capability checks
pub fn needs_transcode(src: &str) -> bool {
    let out = Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-select_streams", "a",
            "-show_entries", "stream=codec_name",
            "-of", "csv=p=0",
            src,
        ])
        .output();
    let Ok(out) = out else { return false };
    let s = std::str::from_utf8(&out.stdout).unwrap_or("");
    s.lines().any(|c| matches!(c.trim(), "ac3" | "eac3" | "dts" | "truehd" | "mlp"))
}

/// Transcode to <same dir>/<stem>.browser.mp4 (video copy, audio → AAC).
/// Returns output path on success. Skips if already done.
pub fn transcode_to_browser_mp4(src: &Path) -> Option<PathBuf> {
    let src_str = src.to_str()?;
    let stem = src.file_stem()?.to_str()?;
    let out = src.with_file_name(format!("{stem}.browser.mp4"));
    if out.exists() {
        return Some(out);
    }
    let tmp = src.with_file_name(format!("{stem}.browser.tmp.mp4"));
    let status = Command::new("ffmpeg")
        .args([
            "-y", "-i", src_str,
            "-map", "0:v:0", "-map", "0:a:0?",
            "-c:v", "copy",
            "-c:a", "aac", "-b:a", "192k",
            "-movflags", "+faststart",
            "-f", "mp4",
        ])
        .arg(&tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    if status.success() && tmp.exists() {
        std::fs::rename(&tmp, &out).ok()?;
        Some(out)
    } else {
        let _ = std::fs::remove_file(&tmp);
        None
    }
}

/// Spawn background transcode for one media id (if needed). Non-blocking.
pub fn maybe_enqueue(db_path: String, media_id: i64, src: PathBuf) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return; // one job at a time; scan re-enqueues later
    }
    std::thread::spawn(move || {
        run_job(&db_path, media_id, &src);
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// For items already in db: look up id by path, skip if already transcoded or
/// audio is browser-playable (no point burning CPU on a 1.5h re-mux).
pub fn maybe_enqueue_for_existing(db_path: String, src: PathBuf) {
    if BUSY.load(Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let Some(src_str) = src.to_str() else { return };
        let Ok(db) = Connection::open(&db_path) else { return };
        let row: Option<(i64, Option<String>)> = db
            .query_row(
                "SELECT id, transcode_path FROM media WHERE path = ?1",
                [src_str],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((id, done)) = row else { return };
        if done.is_some() {
            return;
        }
        if !needs_transcode(src_str) {
            return; // browser-native audio, no transcode needed
        }
        if BUSY.swap(true, Ordering::SeqCst) {
            return;
        }
        run_job(&db_path, id, &src);
        BUSY.store(false, Ordering::SeqCst);
    });
}

fn run_job(db_path: &str, media_id: i64, src: &Path) {
    let done = transcode_to_browser_mp4(src).is_some();
    if done {
        if let Ok(db) = Connection::open(db_path) {
            if let Some(out) = src
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| src.with_file_name(format!("{s}.browser.mp4")))
            {
                let _ = db.execute(
                    "UPDATE media SET transcode_path = ?1 WHERE id = ?2",
                    rusqlite::params![out.to_string_lossy().as_ref(), media_id],
                );
            }
        }
    }
}
