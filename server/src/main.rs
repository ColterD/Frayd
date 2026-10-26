mod auth;
mod comic;
mod scanner;
mod store;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path as StdPath, PathBuf};
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
            let thumbs = thumbs_dir();
            let n = scanner::scan(&db, PathBuf::from(&lib), Some(&thumbs));
            tracing::info!("scan complete: {n} items");
        });
    }

    let app = Router::new()
        .route("/api/library", get(library))
        .route("/api/stream/:id", get(stream))
        .route("/api/progress", post(set_progress))
        .route("/api/progress/:media_id", get(get_progress))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/thumb/:id", get(thumb))
        .route("/api/comic/:id/pages", get(comic_pages))
        .route("/api/comic/:id/page/:n", get(comic_page))
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

#[derive(serde::Deserialize)]
struct LoginReq {
    email: String,
    password: String,
}

/// v0 auth: single-user household. Accepts any email; stores/verifies password
/// hash against identities.secret_hash on first login per username.
/// TODO(M5): argon2id + sessions/tokens.
async fn auth_login(
    State(app): State<std::sync::Arc<App>>,
    Json(req): Json<LoginReq>,
) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    let exists: Option<i64> = db
        .query_row(
            "SELECT id FROM identities WHERE kind='account' AND key = ?1",
            [&req.email],
            |r| r.get(0),
        )
        .ok();

    let identity_id = match exists {
        Some(id) => {
            // existing account: verify password
            match auth::verify_login(&db, &req.email, &req.password) {
                Ok(true) => id,
                _ => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({ "error": "invalid credentials" })),
                    )
                }
            }
        }
        None => {
            // first login mints the account (household bootstrap)
            if let Err(e) = auth::create_account(&db, &req.email, &req.password) {
                tracing::warn!("account create failed: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "account creation failed" })),
                );
            }
            db.query_row(
                "SELECT id FROM identities WHERE kind='account' AND key = ?1",
                [&req.email],
                |r| r.get(0),
            )
            .unwrap()
        }
    };

    let token = match auth::create_session(&db, identity_id) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("session create failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "session creation failed" })),
            );
        }
    };
    let name = req.email.split('@').next().unwrap_or("user").to_string();
    (StatusCode::OK, Json(serde_json::json!({ "name": name, "token": token })))
}

async fn auth_logout(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
) -> StatusCode {
    if let Some(token) = bearer_token(&headers) {
        let db = app.db.lock().unwrap();
        auth::delete_session(&db, &token);
    }
    StatusCode::NO_CONTENT
}

/// Extract bearer token from Authorization header.
fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

/// Resolve identity for a request: Bearer token first, fall back to ?identity= query.
fn resolve_identity(
    db: &Connection,
    headers: &axum::http::HeaderMap,
    query_identity: Option<&str>,
) -> Result<i64, StatusCode> {
    if let Some(token) = bearer_token(headers) {
        if let Some(id) = auth::identity_from_token(db, &token) {
            return Ok(id);
        }
    }
    if let Some(key) = query_identity {
        // LAN fallback: hostname-keyed device identity
        return db
            .query_row("SELECT id FROM identities WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND);
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// Thumbs cache dir: $FRAYD_THUMBS or "thumbs" next to the db file.
fn thumbs_dir() -> PathBuf {
    if let Ok(t) = std::env::var("FRAYD_THUMBS") {
        return PathBuf::from(t);
    }
    let db_path = std::env::var("FRAYD_DB").unwrap_or_else(|_| "frayd.db".into());
    StdPath::new(&db_path)
        .parent()
        .unwrap_or(StdPath::new("."))
        .join("thumbs")
}

/// Serve extracted thumbnail. 204 when none (client falls back gracefully).
async fn thumb(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let path: Option<String> = {
        let db = app.db.lock().unwrap();
        db.query_row("SELECT thumb_path FROM media WHERE id = ?1", [id], |r| r.get(0))
            .ok()
    };
    let Some(path) = path else { return StatusCode::NO_CONTENT.into_response() };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, "image/jpeg"),
             (header::CACHE_CONTROL, "public, max-age=86400, immutable")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NO_CONTENT.into_response(),
    }
}

/// GET /api/comic/:id/pages → ["p001.jpg", ...] natural-sorted.
async fn comic_pages(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(path) = media_path(&app, id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking(move || comic::comic_pages(std::path::Path::new(&path)))
        .await
        .ok()
        .flatten()
    {
        Some(pages) => {
            let names: Vec<&str> = pages.iter().map(|(n, _)| n.as_str()).collect();
            Json(names).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /api/comic/:id/page/:n → image bytes for nth (0-based) page.
async fn comic_page(
    State(app): State<std::sync::Arc<App>>,
    Path((id, n)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let Some(path) = media_path(&app, id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let name = {
        let db = app.db.lock().unwrap();
        let meta: Option<String> = db
            .query_row("SELECT meta_json FROM media WHERE id = ?1", [id], |r| r.get(0))
            .ok()
            .flatten();
        // page list cached in meta_json.pages; recompute if absent
        let pages: Option<Vec<String>> = meta
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v.get("pages").cloned())
            .and_then(|p| serde_json::from_value(p).ok());
        match pages {
            Some(p) => p.get(n as usize).cloned(),
            None => {
                drop(db);
                let db = app.db.lock().unwrap();
                cache_page_list(&db, id, std::path::Path::new(&path))
                    .and_then(|list| list.get(n as usize).cloned())
            }
        }
    };
    let Some(name) = name else { return StatusCode::NOT_FOUND.into_response() };
    let mime = mime_guess(&name);
    match tokio::task::spawn_blocking(move || {
        comic::read_zip_entry(std::path::Path::new(&path), &name)
    })
    .await
    .ok()
    .flatten()
    {
        Some(bytes) => (
            [(header::CONTENT_TYPE, mime),
             (header::CACHE_CONTROL, "private, max-age=86400".to_string())],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Compute + persist page list into meta_json.pages. Returns names.
fn cache_page_list(db: &Connection, id: i64, path: &std::path::Path) -> Option<Vec<String>> {
    let pages = comic::comic_pages(path)?;
    let names: Vec<String> = pages.into_iter().map(|(n, _)| n).collect();
    let meta: Option<String> = db
        .query_row("SELECT meta_json FROM media WHERE id = ?1", [id], |r| r.get(0))
        .ok()
        .flatten();
    let mut v: serde_json::Value = meta
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    v["pages"] = serde_json::json!(names);
    let _ = db.execute(
        "UPDATE media SET meta_json = ?1 WHERE id = ?2",
        rusqlite::params![v.to_string(), id],
    );
    Some(names)
}

async fn media_path(app: &std::sync::Arc<App>, id: i64) -> Option<String> {
    let db = app.db.lock().unwrap();
    db.query_row("SELECT path FROM media WHERE id = ?1", [id], |r| r.get(0))
        .ok()
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
    identity: Option<String>,
    media_id: i64,
    position: f64,
}

async fn set_progress(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Json(p): Json<ProgressIn>,
) -> StatusCode {
    let db = app.db.lock().unwrap();
    let ident_id = match resolve_identity(&db, &headers, p.identity.as_deref()) {
        Ok(id) => id,
        Err(s) => return s,
    };
    if let Err(e) = db.execute(
        "INSERT INTO progress (identity_id, media_id, position, updated_at)
         VALUES (?1, ?2, ?3, unixepoch())
         ON CONFLICT(identity_id, media_id)
         DO UPDATE SET position = ?3, updated_at = unixepoch()",
        rusqlite::params![ident_id, p.media_id, p.position],
    ) {
        tracing::warn!("set_progress failed (media_id={}): {e}", p.media_id);
        return StatusCode::NOT_FOUND;
    }
    StatusCode::NO_CONTENT
}

async fn get_progress(
    State(app): State<std::sync::Arc<App>>,
    Path(media_id): Path<i64>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    let ident_id = match resolve_identity(&db, &headers, q.get("identity").map(|s| s.as_str())) {
        Ok(id) => id,
        Err(s) => return (s, "unauthorized".to_string()),
    };
    let pos: Option<(f64, i64)> = db
        .query_row(
            "SELECT p.position, p.updated_at FROM progress p
             WHERE identity_id = ?1 AND p.media_id = ?2",
            rusqlite::params![ident_id, media_id],
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


