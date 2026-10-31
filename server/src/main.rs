mod auth;
mod transcode;
mod comic;
mod epub;
mod party;
mod scanner;
mod store;

use axum::{
    extract::ws::WebSocketUpgrade,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rusqlite::{Connection, OptionalExtension};
use std::process::Command;
use std::path::{Path as StdPath, PathBuf};
use std::sync::{Arc, Mutex};
use tower_http::services::fs::ServeDir;

pub struct App {
    pub db: Mutex<Connection>,
    /// Path to the SQLite file (transcode thread reopens its own handle).
    pub db_path: String,
    /// Library root from FRAYD_LIBRARY (scan target). RwLock so admin UI can change it live.
    pub library: std::sync::RwLock<Option<PathBuf>>,
    /// True while a scan is running (single scan at a time).
    pub scanning: std::sync::atomic::AtomicBool,
    /// Login/setup/password attempts: 10 per 5 min per IP.
    pub auth_rl: auth::RateLimiter,
    /// Active listening party (single party — household scale).
    pub party: Arc<party::Party>,
    /// yt-dlp ingest state: "downloading" | "done" | "failed" | None. Single ingest at a time.
    pub ingest_status: std::sync::Mutex<Option<String>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // default paths relative to this source dir so cwd doesn't matter (env overrides)
    let default_db = concat!(env!("CARGO_MANIFEST_DIR"), "/frayd.db");
    let db_path = std::env::var("FRAYD_DB").unwrap_or_else(|_| default_db.into());
    let db = store::open(&db_path);
    // Library root: env FRAYD_LIBRARY > settings.library_path (persistent, set via admin UI)
    let library: Option<PathBuf> = std::env::var("FRAYD_LIBRARY").ok().map(PathBuf::from).or_else(|| {
        let raw: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key='library_path'", [], |r| r.get(0))
            .ok();
        raw.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    });

    let state = std::sync::Arc::new(App {
        db: Mutex::new(db),
        db_path: db_path.clone(),
        library: std::sync::RwLock::new(library.clone()),
        scanning: std::sync::atomic::AtomicBool::new(false),
        auth_rl: auth::RateLimiter::new(300, 10),
        party: Arc::new(party::Party::new()),
        ingest_status: std::sync::Mutex::new(None),
    });

    // optional immediate scan
    if let Some(lib) = library {
        state
            .scanning
            .store(true, std::sync::atomic::Ordering::SeqCst);
        run_scan(state.clone(), vec![lib]);
    }

    let app = app_router(state.clone(), client_dir());

    // Port: FRAYD_PORT env > settings.port > 8484
    let port: u16 = std::env::var("FRAYD_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| {
            let db = state.db.lock().unwrap();
            let raw: Option<String> = db
                .query_row("SELECT value FROM settings WHERE key='port'", [], |r| r.get(0))
                .ok();
            raw.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
                .and_then(|v| v.as_u64().map(|p| p as u16))
        })
        .unwrap_or(8484);
    let addr = format!("0.0.0.0:{port}");
    tracing::info!("frayd listening on {addr}");

    // setting helper: read a bool from settings table
    let setting_bool = |key: &str| -> bool {
        let db = state.db.lock().unwrap();
        let raw: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .ok();
        matches!(raw.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok()), Some(serde_json::Value::Bool(true)))
    };

    // UPnP port forward (best-effort, needs `upnpc` binary; router must have UPnP on)
    if std::env::var("FRAYD_UPNP").map(|v| v == "1").unwrap_or(false) || setting_bool("upnp") {
        let ext = port; // same external port
        std::thread::spawn(move || {
            match Command::new("upnpc")
                .args(["-a", "", &ext.to_string(), &ext.to_string(), "tcp", "900"])
                .status()
            {
                Ok(s) if s.success() => tracing::info!("upnp: forwarded external {ext} → tcp {ext}"),
                _ => tracing::warn!("upnp: forward failed (router UPnP off or upnpc missing?)"),
            }
        });
    }

    // HTTPS: FRAYD_CERT + FRAYD_KEY env (PEM files) → rustls; else plain HTTP
    let env_cert = std::env::var("FRAYD_CERT").ok();
    let env_key = std::env::var("FRAYD_KEY").ok();
    let setting_str = |key: &str| -> Option<String> {
        let db = state.db.lock().unwrap();
        let raw: Option<String> = db
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .ok();
        raw.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty())
    };
    if let (Some(cert), Some(key)) = (
        env_cert.or_else(|| setting_str("tls_cert")),
        env_key.or_else(|| setting_str("tls_key")),
    ) {
        tracing::info!("tls: loading cert {cert}");
        rustls::crypto::ring::default_provider().install_default().expect("crypto provider");
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
            .await
            .expect("tls config");
        let sock: std::net::SocketAddr = addr.parse().unwrap();
        axum_server::bind_rustls(sock, tls)
            .serve(app.into_make_service())
            .await
            .unwrap();
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app).await.unwrap();
    }
}

fn app_router(state: Arc<App>, client: PathBuf) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/media/{id}", axum::routing::delete(delete_media))
        .route("/api/admin/users/{id}", axum::routing::delete(remove_user))
        .route("/api/library", get(library_handler))
        .route("/api/stream/{id}", get(stream))
        .route("/api/progress", post(set_progress))
        .route("/api/progress/{media_id}", get(get_progress))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/me", get(auth_me))
        .route("/api/auth/password", post(auth_change_password))
        .route("/api/setup", get(setup_status).post(auth_setup))
        .route("/api/admin/perms", get(list_perms).post(set_perm))
        .route("/api/admin/settings", get(get_settings).post(set_setting))
        .route("/api/thumb/{id}", get(thumb))
        .route("/api/scan", post(trigger_scan))
        .route("/api/ingest", post(ingest))
        .route("/api/scan/status", get(scan_status))
        .route("/api/party", post(party_create))
        .route("/api/party/{code}/ws", get(party_ws))
        .route("/api/comic/{id}/pages", get(comic_pages))
        .route("/api/comic/{id}/page/{n}", get(comic_page))
        .route("/api/epub/{id}/spine", get(epub_spine))
        .route("/api/epub/{id}/toc", get(epub_toc))
        .route("/api/epub/{id}/chunk/{n}", get(epub_chunk))
        .route("/api/epub/{id}/asset/{*path}", get(epub_asset))
        .route("/api/annotations/{id}", get(list_annotations))
        .route("/api/annotations", post(create_annotation))
        .route("/api/annotations", axum::routing::delete(delete_annotation))
        .fallback_service(ServeDir::new(client))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::DELETE])
                .allow_headers([axum::http::header::AUTHORIZATION, axum::http::header::CONTENT_TYPE]),
        )
        .with_state(state)
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(serde::Serialize)]
struct LibraryItem {
    id: i64,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<i64>,
    title: String,
    author: Option<String>,
    duration_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    synopsis: Option<String>,
    series: Option<String>,
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
    headers: axum::http::HeaderMap,
    Json(req): Json<LoginReq>,
) -> impl IntoResponse {
    if !app.auth_rl.check(&auth::client_ip(&headers, "unknown")) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error": "too many attempts — wait 5 minutes"})));
    }
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

    // first account ever = admin (bootstrap); grant if no admin exists yet
    let _ = db.execute(
        "INSERT OR IGNORE INTO user_perms (identity_id, is_admin) 
         SELECT ?1, 1 WHERE NOT EXISTS (SELECT 1 FROM user_perms WHERE is_admin = 1)",
        [identity_id],
    );

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
    let is_admin: i64 = db
        .query_row(
            "SELECT is_admin FROM user_perms WHERE identity_id = ?1",
            [identity_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "name": name, "token": token, "is_admin": is_admin != 0 })),
    )
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

#[derive(serde::Deserialize)]
struct ChangePwReq {
    old: String,
    new: String,
}

/// POST /api/auth/password {old, new} — change own password (requires session).
async fn auth_change_password(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChangePwReq>,
) -> impl IntoResponse {
    if !app.auth_rl.check(&format!("pw:{}", auth::client_ip(&headers, "unknown"))) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error": "too many attempts — wait 5 minutes"}))).into_response();
    }
    if req.new.len() < 4 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "new password too short (min 4)"}))).into_response();
    }
    let db = app.db.lock().unwrap();
    let Some(tok) = bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(iid) = auth::identity_from_token(&db, &tok) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(key) = db.query_row(
        "SELECT key FROM identities WHERE id = ?1 AND kind='account'",
        rusqlite::params![iid],
        |r| r.get::<_, String>(0),
    ) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match auth::change_password(&db, &key, &req.old, &req.new) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

/// GET /api/setup → {needed: bool} — true while zero accounts exist.
async fn setup_status(State(app): State<std::sync::Arc<App>>) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    let have: i64 = db
        .query_row("SELECT COUNT(*) FROM identities WHERE kind='account'", [], |r| r.get(0))
        .unwrap_or(0);
    Json(serde_json::json!({ "needed": have == 0 }))
}

#[derive(serde::Deserialize)]
struct SetupReq {
    email: String,
    password: String,
}

/// POST /api/setup {email, password} — one-shot owner creation.
/// Only works while ZERO accounts exist; the created account is always admin.
async fn auth_setup(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SetupReq>,
) -> impl IntoResponse {
    if !app.auth_rl.check(&auth::client_ip(&headers, "unknown")) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error": "too many attempts — wait 5 minutes"}))).into_response();
    }
    if req.password.len() < 4 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "password too short (min 4)"}))).into_response();
    }
    let db = app.db.lock().unwrap();
    let have: i64 = db
        .query_row("SELECT COUNT(*) FROM identities WHERE kind='account'", [], |r| r.get(0))
        .unwrap_or(0);
    if have > 0 {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error": "setup already done — log in instead"}))).into_response();
    }
    if let Err(e) = auth::create_account(&db, &req.email, &req.password) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e}))).into_response();
    }
    let iid: i64 = db
        .query_row("SELECT id FROM identities WHERE kind='account' AND key = ?1", [&req.email], |r| r.get(0))
        .unwrap_or(0);
    let _ = db.execute(
        "INSERT INTO user_perms (identity_id, is_admin) VALUES (?1, 1)
         ON CONFLICT(identity_id) DO UPDATE SET is_admin = 1",
        rusqlite::params![iid],
    );
    match auth::create_session(&db, iid) {
        Ok(token) => {
            let name = req.email.split('@').next().unwrap_or("owner").to_string();
            (StatusCode::OK, Json(serde_json::json!({"token": token, "name": name, "is_admin": true}))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e}))).into_response(),
    }
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
        // LAN fallback: hostname-keyed device identity, auto-created on first use
        let existing: Option<i64> = db
            .query_row("SELECT id FROM identities WHERE key = ?1", [key], |r| r.get(0))
            .ok();
        return match existing {
            Some(id) => Ok(id),
            None => db
                .execute(
                    "INSERT INTO identities (kind, key) VALUES ('device', ?1)",
                    [key],
                )
                .map(|_| db.last_insert_rowid())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR),
        };
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// Thumbs cache dir: $FRAYD_THUMBS or "thumbs" next to the db file.
fn thumbs_dir() -> PathBuf {
    if let Ok(t) = std::env::var("FRAYD_THUMBS") {
        return PathBuf::from(t);
    }
    let db_path = std::env::var("FRAYD_DB").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/frayd.db").into()
    });
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

/// Web client directory. Resolution order:
/// 1. FRAYD_CLIENT_DIR env (explicit)
/// 2. ./client next to the server binary (release zips: server + client/ side by side)
/// 3. ./client in cwd
/// 4. source checkout (CARGO_MANIFEST_DIR/../client) — dev builds
fn client_dir() -> PathBuf {
    if let Ok(d) = std::env::var("FRAYD_CLIENT_DIR") {
        return PathBuf::from(d);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent().map(|p| p.join("client")) {
            if d.is_dir() { return d; }
        }
    }
    if PathBuf::from("client").is_dir() {
        return PathBuf::from("client");
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../client"))
}

/// Spawn a background library scan. Caller must have set `scanning` = true
/// via compare_exchange; this just runs the scan and clears the flag after.
/// All library roots: App.library (env or last-set single path) + settings.library_paths JSON array.
/// Deduped, ordered. Postgres-free — plain SQLite read.
fn library_roots(app: &App) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(l) = app.library.read().unwrap().clone() {
        roots.push(l);
    }
    let db = app.db.lock().unwrap();
    let raw: Option<String> = db
        .query_row("SELECT value FROM settings WHERE key='library_paths'", [], |r| r.get(0))
        .ok();
    let parsed = raw.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    if let Some(serde_json::Value::Array(list)) = parsed {
        for v in list {
            if let Some(s) = v.as_str().filter(|s| !s.is_empty()) {
                let p = PathBuf::from(s);
                if !roots.contains(&p) {
                    roots.push(p);
                }
            }
        }
    }
    roots
}

fn run_scan(state: std::sync::Arc<App>, libs: Vec<PathBuf>) {
    tokio::task::spawn_blocking(move || {
        // own connection: a long scan must NOT hold the shared DB mutex
        // (it blocked server bind + every request until the scan finished)
        let db = store::open(&state.db_path);
        let thumbs = thumbs_dir();
        let mut n = 0;
        for lib in libs {
            n += scanner::scan(&db, state.db_path.clone(), lib, Some(&thumbs));
        }
        // sweep: drop rows whose path is no longer under any configured root
        // (folder removed from settings, or moved). Transcode temps already purged by scan.
        let roots = library_roots(&state);
        if !roots.is_empty() {
            let mut stmt = db
                .prepare("SELECT id, path FROM media")
                .unwrap();
            let orphans: Vec<(i64, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|(_, p)| {
                    let path = std::path::Path::new(p);
                    !roots.iter().any(|r| path.starts_with(r))
                })
                .collect();
            drop(stmt);
            for (id, p) in orphans {
                let _ = std::fs::remove_file(thumbs.join(format!("{id}.jpg")));
                let _ = db.execute("DELETE FROM media WHERE id = ?1", [id]);
                tracing::info!("sweep: removed orphaned media {id} ({p})");
            }
        }
        state
            .scanning
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("scan complete: {n} items");
    });
}

/// POST /api/scan → trigger library rescan. 202 if started, 409 if busy.
async fn trigger_scan(State(app): State<std::sync::Arc<App>>) -> StatusCode {
    use std::sync::atomic::Ordering;
    // re-read roots: env > settings (library_path single + library_paths array)
    let libs = library_roots(&app);
    if libs.is_empty() {
        return StatusCode::NOT_IMPLEMENTED; // no library configured
    }
    match app.scanning.compare_exchange(
        false,
        true,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => {
            run_scan(app, libs);
            StatusCode::ACCEPTED
        }
        Err(_) => StatusCode::CONFLICT,
    }
}

/// POST /api/ingest { url } → download via yt-dlp into <library>/Internet/, then rescan.
/// Admin-only. yt-dlp is best-effort external binary (like upnpc/ffmpeg).
#[derive(serde::Deserialize)]
struct IngestReq {
    url: String,
}

async fn ingest(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    Json(body): Json<IngestReq>,
) -> StatusCode {
    // admin gate: downloading remote files into the library is an owner action
    {
        let db = app.db.lock().unwrap();
        if require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())).is_err() {
            return StatusCode::FORBIDDEN;
        }
    }
    // trust boundary: only http(s) URLs, no shell (args passed as array)
    if !body.url.starts_with("http://") && !body.url.starts_with("https://") {
        return StatusCode::BAD_REQUEST;
    }
    if std::process::Command::new("yt-dlp").arg("--version").output().is_err() {
        return StatusCode::NOT_IMPLEMENTED; // yt-dlp not installed
    }
    let libs = library_roots(&app);
    if libs.is_empty() { return StatusCode::NOT_IMPLEMENTED; }
    let dest = libs[0].join("Internet");
    std::fs::create_dir_all(&dest).ok();
    // one ingest at a time
    {
        let mut st = app.ingest_status.lock().unwrap();
        if st.as_deref() == Some("downloading") { return StatusCode::CONFLICT; }
        *st = Some("downloading".into());
    }
    let state = app.clone();
    let url = body.url;
    // spawn detached: download can take minutes; rescan after success
    tokio::task::spawn_blocking(move || {
        let out = dest.join("%(title)s.%(ext)s");
        let ok = std::process::Command::new("yt-dlp")
            .args(["-f", "mp4", "--no-playlist", "-o"])
            .arg(out)
            .arg(&url)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        *state.ingest_status.lock().unwrap() = Some(if ok { "done" } else { "failed" }.into());
        if ok {
            // rescan so the new file appears
            if let Some(lib) = state.library.read().unwrap().clone() {
                let st = state.clone();
                st.scanning.store(true, std::sync::atomic::Ordering::SeqCst);
                run_scan(st, vec![lib]);
            }
        }
        tracing::info!("ingest yt-dlp {url}: {}", if ok { "ok" } else { "failed" });
    });
    StatusCode::ACCEPTED
}

#[derive(serde::Deserialize)]
struct PartyCreateReq {
    /// Identity key (token-derived or LAN fallback).
    identity: String,
}

/// POST /api/party → { code } — host starts a party.
async fn party_create(
    State(app): State<std::sync::Arc<App>>,
    Json(req): Json<PartyCreateReq>,
) -> impl IntoResponse {
    let code = app.party.create(&req.identity);
    (StatusCode::OK, Json(serde_json::json!({ "code": code })))
}

/// GET /api/party/:code/ws — join party socket. Identity via ?identity= or Bearer.
async fn party_ws(
    State(app): State<std::sync::Arc<App>>,
    Path(code): Path<String>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // validate: party exists + code matches
    let code_ok = app
        .party
        .code
        .lock()
        .unwrap()
        .as_deref()
        .map(|c| c.eq_ignore_ascii_case(&code))
        .unwrap_or(false);
    if !code_ok {
        return StatusCode::NOT_FOUND.into_response();
    }

    // resolve identity: token first, LAN fallback
    let key = {
        let db = app.db.lock().unwrap();
        match resolve_identity(&db, &headers, q.get("identity").map(|s| s.as_str())) {
            Ok(id) => db
                .query_row("SELECT key FROM identities WHERE id = ?1", [id], |r| r.get(0))
                .unwrap_or_else(|_| format!("anon-{id}")),
            Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
        }
    };
    let is_host = app.party.is_host(&key);
    ws.on_upgrade(move |socket| party::member_socket(app.party.clone(), socket, key, is_host))
        .into_response()
}

/// GET /api/scan/status → { scanning: bool }
async fn scan_status(State(app): State<std::sync::Arc<App>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "scanning": app.scanning.load(std::sync::atomic::Ordering::SeqCst),
        "library_configured": app.library.read().unwrap().is_some(),
        "ingesting": app.ingest_status.lock().unwrap().as_deref(),
    }))
}

/// GET /api/epub/:id/asset/OEBPS/images/x.png → raw zip entry bytes.
/// Path traversal guard: reject absolute paths, "..", backslashes.
async fn epub_asset(
    State(app): State<std::sync::Arc<App>>,
    Path((id, path)): Path<(i64, String)>,
) -> impl IntoResponse {
    if path.starts_with('/') || path.contains("..") || path.contains('\\') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(mpath) = media_path(&app, id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess(&path);
    match tokio::task::spawn_blocking(move || {
        epub::read_zip_entry(std::path::Path::new(&mpath), &path)
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

/// GET /api/epub/:id/spine → [chunk hrefs] in reading order.
async fn epub_spine(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(path) = media_path(&app, id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking(move || epub::epub_spine(std::path::Path::new(&path)))
        .await
        .ok()
        .flatten()
    {
        Some(spine) => Json(spine).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /api/epub/:id/toc → [{title, spine_idx}] in reading order.
async fn epub_toc(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let Some(path) = media_path(&app, id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking(move || epub::epub_toc(std::path::Path::new(&path)))
        .await
        .ok()
        .flatten()
    {
        Some(toc) => Json(
            toc.into_iter()
                .map(|(title, spine_idx)| serde_json::json!({ "title": title, "spine_idx": spine_idx }))
                .collect::<Vec<_>>(),
        )
        .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /api/epub/:id/chunk/:n → XHTML bytes of nth spine item, sandboxed MIME.
async fn epub_chunk(
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
        let spine: Option<Vec<String>> = meta
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v.get("spine").cloned())
            .and_then(|p| serde_json::from_value(p).ok());
        match spine {
            Some(s) => s.get(n as usize).cloned(),
            None => {
                drop(db);
                let db = app.db.lock().unwrap();
                cache_spine(&db, id, std::path::Path::new(&path))
                    .and_then(|s| s.get(n as usize).cloned())
            }
        }
    };
    let Some(name) = name else { return StatusCode::NOT_FOUND.into_response() };
    match tokio::task::spawn_blocking(move || {
        epub::read_zip_entry(std::path::Path::new(&path), &name)
    })
    .await
    .ok()
    .flatten()
    {
        Some(bytes) => (
            [(header::CONTENT_TYPE, "application/xhtml+xml"),
             (header::CACHE_CONTROL, "private, max-age=86400"),
             (header::CONTENT_SECURITY_POLICY,
              "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:")],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Compute + persist spine into meta_json.spine. Returns hrefs.
fn cache_spine(db: &Connection, id: i64, path: &std::path::Path) -> Option<Vec<String>> {
    let spine = epub::epub_spine(path)?;
    let meta: Option<String> = db
        .query_row("SELECT meta_json FROM media WHERE id = ?1", [id], |r| r.get(0))
        .ok()
        .flatten();
    let mut v: serde_json::Value = meta
        .as_deref()
        .and_then(|m| serde_json::from_str(m).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    v["spine"] = serde_json::json!(spine);
    let _ = db.execute(
        "UPDATE media SET meta_json = ?1 WHERE id = ?2",
        rusqlite::params![v.to_string(), id],
    );
    Some(spine)
}

async fn media_path(app: &std::sync::Arc<App>, id: i64) -> Option<String> {
    let db = app.db.lock().unwrap();
    db.query_row("SELECT path FROM media WHERE id = ?1", [id], |r| r.get(0))
        .ok()
}

/// DELETE /api/admin/users/:id?identity= — remove a user account: sessions, perms,
/// progress, annotations, and the identity row. Admin-only; cannot remove self or last admin.
async fn remove_user(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    Path(id): Path<i64>,
) -> StatusCode {
    let admin_iid = {
        let db = app.db.lock().unwrap();
        match require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())) {
            Ok(iid) => iid,
            Err(s) => return s,
        }
    };
    if admin_iid == id {
        return StatusCode::CONFLICT; // don't remove yourself
    }
    let db = app.db.lock().unwrap();
    let is_account: Option<String> = db
        .query_row("SELECT kind FROM identities WHERE id = ?1", [id], |r| r.get(0))
        .ok();
    if is_account.as_deref() != Some("account") {
        return StatusCode::NOT_FOUND; // only accounts removable — device identities are guests
    }
    let admins: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM user_perms WHERE is_admin = 1 AND identity_id != ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if admins == 0 {
        return StatusCode::CONFLICT; // last admin
    }
    let _ = db.execute("DELETE FROM sessions WHERE identity_id = ?1", [id]);
    let _ = db.execute("DELETE FROM user_perms WHERE identity_id = ?1", [id]);
    let _ = db.execute("DELETE FROM progress WHERE identity_id = ?1", [id]);
    let _ = db.execute("DELETE FROM annotations WHERE identity_id = ?1", [id]);
    match db.execute("DELETE FROM identities WHERE id = ?1", [id]) {
        Ok(0) => StatusCode::NOT_FOUND,
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// GET /api/health — liveness + build/library info (no auth; nothing sensitive).
async fn health(State(app): State<std::sync::Arc<App>>) -> Json<serde_json::Value> {
    let library = app.library.read().unwrap().as_ref().map(|p| p.display().to_string());
    Json(serde_json::json!({
        "ok": true,
        "version": VERSION,
        "library": library,
        "scanning": app.scanning.load(std::sync::atomic::Ordering::SeqCst),
    }))
}

/// DELETE /api/media/:id — remove item from library (DB row + thumbnails). Admin-only.
/// Never touches the source file on disk (owner can delete it themselves).
async fn delete_media(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    Path(id): Path<i64>,
) -> StatusCode {
    let iid = {
        let db = app.db.lock().unwrap();
        match require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())) {
            Ok(iid) => iid,
            Err(s) => return s,
        }
    };
    let db = app.db.lock().unwrap();
    // only allow deleting items visible to this admin's perms (paranoia: consistent with library view)
    let exists: Option<i64> = db
        .query_row("SELECT id FROM media WHERE id = ?1", [id], |r| r.get(0))
        .ok();
    if exists.is_none() {
        return StatusCode::NOT_FOUND;
    }
    let _ = iid; // require_admin already validated identity
    // thumbnail: per-media-id file <id>.jpg in thumbs dir
    let _ = std::fs::remove_file(thumbs_dir().join(format!("{id}.jpg")));
    match db.execute("DELETE FROM media WHERE id = ?1", [id]) {
        Ok(0) => StatusCode::NOT_FOUND,
        Ok(_) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn library_handler(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<Vec<LibraryItem>> {
    let db = app.db.lock().unwrap();
    // per-identity kind visibility: NULL row/no row = see all; else only listed kinds
    let allowed: Option<Vec<String>> = resolve_identity(&db, &headers, q.get("identity").map(|s| s.as_str()))
        .ok()
        .and_then(|iid| {
            db.query_row("SELECT allowed_kinds FROM user_perms WHERE identity_id = ?1", [iid], |r| r.get::<_, Option<String>>(0))
                .ok()
                .flatten()
                .and_then(|s| serde_json::from_str(&s).ok())
        });
    let kinds_clause = allowed.as_ref().map(|ks| {
        (
            format!(
                " AND kind IN ({})",
                ks.iter().map(|k| format!("'{}'", k.replace('\'', ""))).collect::<Vec<_>>().join(",")
            ),
            (),
        )
    });
    let filter = q.get("q").map(|s| format!("%{}%", s.replace('%', "").replace('_', "")));
    let sql = format!(
        "SELECT id, kind, title, author, duration_secs,
                json_extract(meta_json, '$.synopsis'), series, created_at
         FROM media
         WHERE (?1 IS NULL OR title LIKE ?1 OR author LIKE ?1
                OR json_extract(meta_json, '$.synopsis') LIKE ?1){}
         ORDER BY created_at DESC",
        kinds_clause.as_ref().map(|(c, _)| c.as_str()).unwrap_or("")
    );
    let mut stmt = db.prepare(&sql).unwrap();
    let items = stmt
        .query_map(rusqlite::params![filter], |r| {
            Ok(LibraryItem {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                author: r.get(3)?,
                duration_secs: r.get(4)?,
                synopsis: r.get(5)?,
                series: r.get(6)?,
                created_at: r.get(7)?,
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
            "SELECT COALESCE(transcode_path, path), size_bytes FROM media WHERE id = ?1",
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
                .header(header::CONTENT_RANGE, format!("bytes {start}-{}/{size}", size - 1))
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
        "mp4" => "video/mp4",
        "m4b" | "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "opus" | "ogg" => "audio/ogg",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "epub" => "application/epub+zip",
        "cbz" => "application/vnd.comicbook+zip",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
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



// ---------- annotations (highlights + bookmarks) ----------

#[derive(serde::Deserialize)]
struct AnnotationIn {
    media_id: i64,
    kind: String, // highlight | bookmark
    spine_idx: i64,
    cfi: String,
    text: Option<String>,
    color: Option<String>,
    note: Option<String>,
}

#[derive(serde::Serialize)]
struct AnnotationOut {
    id: i64,
    kind: String,
    spine_idx: i64,
    cfi: String,
    text: Option<String>,
    color: Option<String>,
    note: Option<String>,
}

async fn list_annotations(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    let ident_id = match resolve_identity(&db, &headers, q.get("identity").map(|s| s.as_str())) {
        Ok(v) => v,
        Err(s) => return (s, "[]".to_string()),
    };
    let mut stmt = db
        .prepare(
            "SELECT id, kind, spine_idx, cfi, text, color, note
             FROM annotations WHERE identity_id = ?1 AND media_id = ?2 ORDER BY spine_idx, cfi",
        )
        .unwrap();
    let items = stmt
        .query_map(rusqlite::params![ident_id, id], |r| {
            Ok(AnnotationOut {
                id: r.get(0)?,
                kind: r.get(1)?,
                spine_idx: r.get(2)?,
                cfi: r.get(3)?,
                text: r.get(4)?,
                color: r.get(5)?,
                note: r.get(6)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    (StatusCode::OK, serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()))
}

async fn create_annotation(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Json(a): Json<AnnotationIn>,
) -> StatusCode {
    let db = app.db.lock().unwrap();
    let ident_id = match resolve_identity(&db, &headers, None) {
        Ok(v) => v,
        Err(s) => return s,
    };
    match db.execute(
        "INSERT INTO annotations (identity_id, media_id, kind, spine_idx, cfi, text, color, note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(identity_id, media_id, cfi) DO UPDATE SET
           text = ?6, color = ?7, note = ?8",
        rusqlite::params![
            ident_id,
            a.media_id,
            a.kind,
            a.spine_idx,
            a.cfi,
            a.text,
            a.color,
            a.note
        ],
    ) {
        Ok(_) => StatusCode::CREATED,
        Err(e) => {
            tracing::warn!("create_annotation failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn delete_annotation(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> StatusCode {
    let db = app.db.lock().unwrap();
    let ident_id = match resolve_identity(&db, &headers, q.get("identity").map(|s| s.as_str())) {
        Ok(v) => v,
        Err(s) => return s,
    };
    let ann_id: i64 = match q.get("ann_id").and_then(|v| v.parse().ok()) {
        Some(v) => v,
        None => return StatusCode::BAD_REQUEST,
    };
    match db.execute(
        "DELETE FROM annotations WHERE id = ?1 AND identity_id = ?2",
        rusqlite::params![ann_id, ident_id],
    ) {
        Ok(n) if n > 0 => StatusCode::NO_CONTENT,
        _ => StatusCode::NOT_FOUND,
    }
}

/// Admin guard: resolve identity, check is_admin. Returns ident id or Err(status).
fn require_admin(db: &Connection, headers: &axum::http::HeaderMap, qid: Option<&str>) -> Result<i64, StatusCode> {
    let iid = resolve_identity(db, headers, qid).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let admin: i64 = db
        .query_row("SELECT is_admin FROM user_perms WHERE identity_id = ?1", [iid], |r| r.get(0))
        .unwrap_or(0);
    if admin == 0 { Err(StatusCode::FORBIDDEN) } else { Ok(iid) }
}

/// GET /api/admin/perms → [{identity_id, key, is_admin, allowed_kinds}]
async fn list_perms(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    if require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut stmt = db
        .prepare(
            "SELECT i.id, i.key, COALESCE(p.is_admin, 0), p.allowed_kinds
             FROM identities i LEFT JOIN user_perms p ON p.identity_id = i.id
             WHERE i.kind = 'account' ORDER BY i.key",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            let kinds: Option<String> = r.get(3)?;
            Ok(serde_json::json!({
                "identity_id": r.get::<_, i64>(0)?,
                "key": r.get::<_, String>(1)?,
                "is_admin": r.get::<_, i64>(2)? != 0,
                "allowed_kinds": kinds.and_then(|k| serde_json::from_str::<Vec<String>>(&k).ok()),
            }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    Json(rows).into_response()
}

/// POST /api/admin/perms {identity_id, is_admin?, allowed_kinds?: [..]|null}
async fn set_perm(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    Json(body): Json<serde_json::Value>,
) -> StatusCode {
    let db = app.db.lock().unwrap();
    if require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())).is_err() {
        return StatusCode::FORBIDDEN;
    }
    let Some(iid) = body["identity_id"].as_i64() else { return StatusCode::BAD_REQUEST };
    // demoting last admin blocked
    if body["is_admin"] == serde_json::json!(false) {
        let admins: i64 = db.query_row("SELECT COUNT(*) FROM user_perms WHERE is_admin = 1 AND identity_id != ?1", [iid], |r| r.get(0)).unwrap_or(0);
        if admins == 0 { return StatusCode::CONFLICT; }
    }
    let kinds_json: Option<String> = match body.get("allowed_kinds") {
        Some(v) if v.is_null() => None,
        Some(v) if v.is_array() => Some(v.to_string()),
        _ => None, // not provided: leave unchanged
    };
    let admin_chg = body.get("is_admin").and_then(|v| v.as_bool());
    let _ = db.execute(
        "INSERT INTO user_perms (identity_id, is_admin, allowed_kinds) VALUES (?1, ?2, ?3)
         ON CONFLICT(identity_id) DO UPDATE SET
           is_admin = COALESCE(?2, is_admin),
           allowed_kinds = COALESCE(?3, allowed_kinds)",
        rusqlite::params![iid, admin_chg.map(|b| b as i64), kinds_json],
    );
    StatusCode::NO_CONTENT
}

/// GET /api/admin/settings → {key: value,...}
async fn get_settings(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    if require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut stmt = db.prepare("SELECT key, value FROM settings").unwrap();
    let mut map = serde_json::Map::new();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .filter_map(Result::ok);
    for (k, v) in rows {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&v) {
            map.insert(k, parsed);
        }
    }
    Json(serde_json::Value::Object(map)).into_response()
}

/// POST /api/admin/settings {key: value}
async fn set_setting(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let db = app.db.lock().unwrap();
    if require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(serde_json::Value::Object(map)) = Some(body) else { return StatusCode::BAD_REQUEST.into_response() };
    for (k, v) in map {
        // library_path: validate the folder exists before persisting (typo = empty scan)
        if k == "library_path" && !v.is_null() {
            let Some(p) = v.as_str().map(PathBuf::from) else { return StatusCode::BAD_REQUEST.into_response() };
            if !p.is_dir() {
                return (StatusCode::BAD_REQUEST, "path is not a directory").into_response();
            }
        }
        let _ = db.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
            rusqlite::params![k, v.to_string()],
        );
        // live-apply library path change (scan handler also re-reads, but keep App in sync)
        if k == "library_path" {
            *app.library.write().unwrap() = v.as_str().filter(|s| !s.is_empty()).map(PathBuf::from);
        }
    }
    StatusCode::NO_CONTENT.into_response()
}


/// GET /api/auth/me → {name, is_admin} for session resume (Bearer token).
async fn auth_me(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    let Some(token) = bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(iid) = resolve_identity(&db, &headers, None) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // token validity is implied by resolve_identity; get key + admin flag
    let key: String = db
        .query_row(
            "SELECT i.key FROM identities i JOIN sessions s ON s.identity_id = i.id WHERE s.token = ?1",
            [token],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "user".into());
    let admin: i64 = db
        .query_row("SELECT is_admin FROM user_perms WHERE identity_id = ?1", [iid], |r| r.get(0))
        .unwrap_or(0);
    let name = key.split('@').next().unwrap_or("user").to_string();
    Json(serde_json::json!({ "name": name, "is_admin": admin != 0 })).into_response()
}




#[cfg(test)]
mod router_tests {
    use super::*;
    use std::io::{Read, Write};

    #[tokio::test]
    async fn production_router_serves_api_nested_epub_assets_and_static_files() {
        let root = std::env::temp_dir().join(format!("frayd-router-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("index.html"), b"static root fixture").unwrap();
        let book = root.join("fixture.epub");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&book).unwrap());
        archive
            .start_file(
                "OEBPS/images/pixel.png",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let asset = b"nested asset bytes\0\xff";
        archive.write_all(asset).unwrap();
        archive.finish().unwrap();
        let db = store::open(":memory:");
        db.execute(
            "INSERT INTO media (id, kind, path, title) VALUES (1, 'ebook', ?1, 'fixture')",
            [book.to_str().unwrap()],
        )
        .unwrap();
        let state = Arc::new(App {
            db: Mutex::new(db),
            db_path: String::new(),
            library: std::sync::RwLock::new(None),
            scanning: std::sync::atomic::AtomicBool::new(false),
            auth_rl: auth::RateLimiter::new(300, 10),
            party: Arc::new(party::Party::new()),
            ingest_status: Mutex::new(None),
        });
        let router = app_router(state, root.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        tokio::task::spawn_blocking(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(5))
                .build();
            let health: serde_json::Value = serde_json::from_reader(
                agent
                    .get(&format!("http://{address}/api/health"))
                    .call()
                    .unwrap()
                    .into_reader(),
            )
            .unwrap();
            assert_eq!(health["ok"], true);
            for (path, expected) in [
                ("/", b"static root fixture".as_slice()),
                ("/api/epub/1/asset/OEBPS/images/pixel.png", asset.as_slice()),
            ] {
                let mut body = Vec::new();
                agent
                    .get(&format!("http://{address}{path}"))
                    .call()
                    .unwrap()
                    .into_reader()
                    .read_to_end(&mut body)
                    .unwrap();
                assert_eq!(body, expected, "{path}");
            }
            for path in [
                "/missing",
                "/api/epub/1/asset/OEBPS/images/missing.png",
                "/api/epub/999/asset/OEBPS/images/pixel.png",
            ] {
                assert!(
                    matches!(
                        agent.get(&format!("http://{address}{path}")).call(),
                        Err(ureq::Error::Status(404, _))
                    ),
                    "{path}"
                );
            }
        })
        .await
        .unwrap();
        server.abort();
        let _ = server.await;
        std::fs::remove_dir_all(root).unwrap();
    }
}
