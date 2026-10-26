mod scanner;
mod store;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;
use tower_http::services::fs::ServeDir;

pub struct App {
    pub db: Mutex<Connection>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let db_path = std::env::var("FRAYD_DB").unwrap_or_else(|_| "frayd.db".into());
    let db = store::open(&db_path);
    let state = std::sync::Arc::new(App { db: Mutex::new(db) });

    // optional immediate scan
    if let Ok(lib) = std::env::var("FRAYD_LIBRARY") {
        let st = state.clone();
        tokio::task::spawn_blocking(move || {
            let db = st.db.lock().unwrap();
            let n = scanner::scan(&db, PathBuf::from(&lib));
            tracing::info!("scan complete: {n} items");
        });
    }

    let app = Router::new()
        .route("/api/library", get(library))
        .route("/api/stream/:id", get(stream))
        .route("/api/progress", post(set_progress))
        .route("/api/progress/:media_id", get(get_progress))
        .nest_service("/", ServeDir::new("../client"))
        .with_state(state);

    let addr = "0.0.0.0:8484";
    tracing::info!("frayd listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[derive(serde::Serialize)]
struct LibraryItem {
    id: i64,
    kind: String,
    title: String,
    author: Option<String>,
    duration_secs: Option<f64>,
}

async fn library(State(app): State<std::sync::Arc<App>>) -> Json<Vec<LibraryItem>> {
    let db = app.db.lock().unwrap();
    let mut stmt = db
        .prepare("SELECT id, kind, title, author, duration_secs FROM media ORDER BY kind, title")
        .unwrap();
    let items = stmt
        .query_map([], |r| {
            Ok(LibraryItem {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                author: r.get(3)?,
                duration_secs: r.get(4)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    Json(items)
}

async fn stream(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let (path, _size): (String, i64) = {
        let db = app.db.lock().unwrap();
        db.query_row(
            "SELECT path, size_bytes FROM media WHERE id = ?1",
            [id],
            |r| -> rusqlite::Result<(String, i64)> {
                Ok((r.get(0)?, r.get(1)?))
            },
        )
        .unwrap_or_default()
    };
    let p = path;
    if p.is_empty() { return StatusCode::NOT_FOUND.into_response(); }
    // fall back to real file size if scan missed it
    let size = match tokio::fs::metadata(&p).await {
        Ok(m) => m.len() as i64,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let mime = mime_guess(&p);

    // parse "bytes=start-end" (end optional)
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("bytes="))
        .and_then(|s| s.split('-').next())
        .and_then(|s| s.parse::<u64>().ok());

    match range {
        Some(start) => {
            let file = match tokio::fs::File::open(&p).await {
                Ok(f) => f,
                Err(_) => return StatusCode::NOT_FOUND.into_response(),
            };
            let mut file = file;
            use tokio::io::AsyncSeekExt;
            if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
                return StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            }
            let len = size - start as i64;
            use tokio::io::AsyncReadExt;
            let stream = tokio_util::io::ReaderStream::new(file.take(len as u64));
            axum::response::Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_TYPE, mime)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_RANGE, format!("bytes {start}-{}/{len}", size - 1))
                .header(header::CONTENT_LENGTH, len)
                .body(axum::body::Body::from_stream(stream))
                .unwrap()
                .into_response()
        }
        None => {
            let file = match tokio::fs::File::open(&p).await {
                Ok(f) => f,
                Err(_) => return StatusCode::NOT_FOUND.into_response(),
            };
            let stream = tokio_util::io::ReaderStream::new(file);
            axum::response::Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_LENGTH, size)
                .body(axum::body::Body::from_stream(stream))
                .unwrap()
                .into_response()
        }
    }
}


fn mime_guess(p: &str) -> String {
    match p.rsplit('.').next().unwrap_or("") {
        "m4b" | "m4a" | "mp4" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "opus" | "ogg" => "audio/ogg",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "epub" => "application/epub+zip",
        "cbz" => "application/vnd.comicbook+zip",
        _ => "application/octet-stream",
    }
    .into()
}

#[derive(serde::Deserialize)]
struct ProgressIn {
    identity: String,
    media_id: i64,
    position: f64,
}

async fn set_progress(
    State(app): State<std::sync::Arc<App>>,
    Json(p): Json<ProgressIn>,
) -> StatusCode {
    let db = app.db.lock().unwrap();
    let ident_id: Option<i64> = db
        .query_row("SELECT id FROM identities WHERE key = ?1", [&p.identity], |r| r.get(0))
        .ok();
    let ident_id = match ident_id {
        Some(id) => id,
        None => {
            let key = &p.identity;
            if db
                .execute(
                    "INSERT INTO identities (kind, key) VALUES ('device', ?1)",
                    [key],
                )
                .is_err()
            {
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
            db.last_insert_rowid()
        }
    };
    let _ = db.execute(
        "INSERT INTO progress (identity_id, media_id, position, updated_at)
         VALUES (?1, ?2, ?3, unixepoch())
         ON CONFLICT(identity_id, media_id)
         DO UPDATE SET position = ?3, updated_at = unixepoch()",
        rusqlite::params![ident_id, p.media_id, p.position],
    );
    StatusCode::NO_CONTENT
}

async fn get_progress(
    State(app): State<std::sync::Arc<App>>,
    Path(media_id): Path<i64>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let identity = match q.get("identity") {
        Some(i) => i,
        None => return (StatusCode::BAD_REQUEST, "missing identity".to_string()),
    };
    let db = app.db.lock().unwrap();
    let pos: Option<(f64, i64)> = db
        .query_row(
            "SELECT p.position, p.updated_at FROM progress p
             JOIN identities i ON i.id = p.identity_id
             WHERE i.key = ?1 AND p.media_id = ?2",
            rusqlite::params![identity, media_id],
            |r| -> rusqlite::Result<(f64, i64)> {
                Ok((r.get(0)?, r.get(1)?))
            },
        )
        .ok();
    match pos {
        Some((position, updated_at)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "position": position, "updated_at": updated_at })).to_string(),
        ),
        None => (StatusCode::NOT_FOUND, "{}".to_string()),
    }
}


