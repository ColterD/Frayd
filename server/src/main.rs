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
use rusqlite::Connection;
use std::process::Command;
use std::path::{Path as StdPath, PathBuf};
use std::sync::{Arc, Mutex};
use tower_http::services::fs::ServeDir;

pub(crate) static ROOT_ADMISSION: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// The committed effective roots and override lock state, published together under App.library.
pub struct LibraryConfig { roots: Vec<PathBuf>, locked: bool }

/// One scan generation with its original complete roots and optional ingest-completion obligation.
pub struct ScanRequest { generation:u64, roots:Vec<PathBuf>, for_ingest:bool }

#[derive(Default)]
/// Serialize pending generations and the transition between an admitted worker and an empty queue.
pub struct ScanQueue { next_generation: u64, requests: std::collections::VecDeque<ScanRequest> }

pub struct App {
    pub db: Mutex<Connection>,
    /// Path to the SQLite file (transcode thread reopens its own handle).
    pub db_path: String,
    /// Library root from FRAYD_LIBRARY (scan target). RwLock so admin UI can change it live.
    pub library: std::sync::RwLock<LibraryConfig>,
    pub scan_queue: Mutex<ScanQueue>,
    /// True while a scan is running (single scan at a time).
    pub scanning: std::sync::atomic::AtomicBool,
    /// Path-free failure for the most recently admitted scan generation.
    pub scan_error: Mutex<Option<&'static str>>,
    /// Login/setup/password attempts: 10 per 5 min per IP.
    pub auth_rl: auth::RateLimiter,
    /// Active listening party (single party — household scale).
    pub party: Arc<party::Party>,
    /// yt-dlp ingest state: "downloading" | "done" | "failed" | None. Single ingest at a time.
    pub ingest_status: std::sync::Mutex<Option<String>>,
}

/// Expose the exact path-free liveness response independently of library availability.
async fn health() -> Json<serde_json::Value> { Json(serde_json::json!({"status":"ok"})) }

/// Parse only a literal IPv4/IPv6 bind address; default to all IPv4 interfaces and reject invalid overrides.
fn bind_address(value: Option<&str>,port: u16) -> Result<std::net::SocketAddr,std::net::AddrParseError> {
    value.unwrap_or("0.0.0.0").parse::<std::net::IpAddr>().map(|ip|std::net::SocketAddr::new(ip,port))
}

/// Decide router mapping eligibility from the effective bind and explicit requests.
/// Loopback listeners must never acquire an external mapping; external binds keep
/// the existing environment-or-stored-setting opt-in behavior.
fn upnp_mapping_requested(bind: std::net::IpAddr, environment: bool, stored: bool) -> bool {
    let loopback=match bind {
        std::net::IpAddr::V4(address)=>address.is_loopback(),
        std::net::IpAddr::V6(address)=>address.is_loopback() || address.to_ipv4_mapped().is_some_and(|mapped|mapped.is_loopback()),
    };
    !loopback && (environment || stored)
}

/// Initialize schemas, background scanning, and HTTP routes before accepting requests.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // default paths relative to this source dir so cwd doesn't matter (env overrides)
    let default_db = concat!(env!("CARGO_MANIFEST_DIR"), "/frayd.db");
    let db_path = std::env::var("FRAYD_DB").unwrap_or_else(|_| default_db.into());
    let db = store::open(&db_path);
    let library=effective_library_config(&db,std::env::var("FRAYD_LIBRARY").ok().as_deref()).expect("valid library configuration");

    let state = std::sync::Arc::new(App {
        db: Mutex::new(db),
        db_path: db_path.clone(),
        library: std::sync::RwLock::new(library.clone()),
        scan_queue: Mutex::new(ScanQueue::default()),
        scanning: std::sync::atomic::AtomicBool::new(false),
        scan_error: Mutex::new(None),
        auth_rl: auth::RateLimiter::new(300, 10),
        party: Arc::new(party::Party::new()),
        ingest_status: std::sync::Mutex::new(None),
    });

    // optional immediate scan
    if !library.roots.is_empty() {
        state
            .scanning
            .store(true, std::sync::atomic::Ordering::SeqCst);
        run_scan(state.clone(), library.roots);
    }

    // The scan trigger consumes server-owned state and no HTTP-provided arguments.
    let scan_state = state.clone();
    let ingest_state = state.clone();
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/media/:id", axum::routing::delete(delete_media))
        .route("/api/admin/users/:id", axum::routing::delete(remove_user))
        .route("/api/library", get(library_handler))
        .route("/api/stream/:id", get(stream))
        .route("/api/progress", post(set_progress))
        .route("/api/progress/:media_id", get(get_progress))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/auth/me", get(auth_me))
        .route("/api/auth/password", post(auth_change_password))
        .route("/api/setup", get(setup_status).post(auth_setup))
        .route("/api/admin/perms", get(list_perms).post(set_perm))
        .route("/api/admin/settings", get(get_settings).post(set_setting))
        .route("/api/thumb/:id", get(thumb))
        .route("/api/scan", post(move || trigger_scan(State(scan_state.clone()))))
        .route("/api/ingest", post(move |headers: axum::http::HeaderMap,
            query: Query<std::collections::HashMap<String,String>>, body: Json<IngestReq>| {
            ingest(State(ingest_state.clone()),headers,query,body)
        }))
        .route("/api/scan/status", get(scan_status))
        .route("/api/party", post(party_create))
        .route("/api/party/:code/ws", get(party_ws))
        .route("/api/comic/:id/pages", get(comic_pages))
        .route("/api/comic/:id/page/:n", get(comic_page))
        .route("/api/epub/:id/spine", get(epub_spine))
        .route("/api/epub/:id/toc", get(epub_toc))
        .route("/api/epub/:id/chunk/:n", get(epub_chunk))
        .route("/api/epub/:id/asset/*path", get(epub_asset))
        .route("/api/annotations/:id", get(list_annotations))
        .route("/api/annotations", post(create_annotation))
        .route("/api/annotations", axum::routing::delete(delete_annotation))
        .nest_service("/", ServeDir::new(client_dir()))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::DELETE])
                .allow_headers([axum::http::header::AUTHORIZATION, axum::http::header::CONTENT_TYPE]),
        )
        .with_state(state.clone());

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
    let bind=bind_address(std::env::var("FRAYD_BIND").ok().as_deref(),port).expect("FRAYD_BIND must be an IP address");
    let addr=bind.to_string();
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
    if upnp_mapping_requested(bind.ip(),std::env::var("FRAYD_UPNP").map(|v| v == "1").unwrap_or(false),setting_bool("upnp")) {
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

/// Retire the supplied bearer session and return success even when no session is present.
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

/// Resolve a valid Bearer session first; unauthenticated keys may identify only devices.
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
        let existing: rusqlite::Result<(i64, String)> = db.query_row(
            "SELECT id, kind FROM identities WHERE key = ?1", [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );
        return match existing {
            Ok((id, kind)) if kind == "device" => Ok(id),
            Ok(_) => Err(StatusCode::UNAUTHORIZED),
            Err(rusqlite::Error::QueryReturnedNoRows) => db
                .execute(
                    "INSERT INTO identities (kind, key) VALUES ('device', ?1)",
                    [key],
                )
                .map(|_| db.last_insert_rowid())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        };
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// Resolve the explicit thumbnail override or cache beside this application's DB.
fn thumbs_dir_for(db_path: &str) -> PathBuf {
    if let Some(path)=std::env::var_os("FRAYD_THUMBS") { return PathBuf::from(path); }
    StdPath::new(db_path)
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
    match comic_page_names(&app, id, &path).await {
        Some(names) => Json(names).into_response(),
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
    let name = comic_page_names(&app, id, &path).await
        .and_then(|names| usize::try_from(n).ok().and_then(|index| names.get(index).cloned()));
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

/// Share the existing page order between listing and content endpoints.
/// Populate missing caches outside the DB lock, publishing only if the source fingerprint is unchanged.
/// Optional SQLite cache failures retain computed names; rejected snapshots return no result.
async fn comic_page_names(app: &Arc<App>, id: i64, path: &str) -> Option<Vec<String>> {
    let (mtime,size) = {
        let db = app.db.lock().unwrap();
        let (meta,mtime,size): (Option<String>,Option<i64>,Option<i64>) = db.query_row(
            "SELECT meta_json,mtime_secs,size_bytes FROM media WHERE id=?1",[id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).ok()?;
        let cached = meta.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| serde_json::from_value::<Vec<String>>(v["pages"].clone()).ok());
        if let Some(names) = cached { return Some(names); }
        (mtime,size)
    };
    let archive = PathBuf::from(path);
    let pages = tokio::task::spawn_blocking(move || comic::comic_pages(&archive)).await.ok()??;
    let names: Vec<String> = pages.into_iter().map(|(name,_)| name).collect();
    let db = app.db.lock().unwrap();
    // Patch only the cache field; a concurrent rescan must not receive a stale archive listing.
    match db.execute("UPDATE media SET meta_json=json_set(COALESCE(meta_json,'{}'),
        '$.pages',json(?1)) WHERE id=?2 AND path=?3 AND mtime_secs IS ?4 AND size_bytes IS ?5",
        rusqlite::params![serde_json::to_string(&names).ok()?,id,path,mtime,size]) {
        Ok(1)=>Some(names),
        Ok(_)=>None,
        Err(error)=>{tracing::warn!(%error,id,"comic page cache write held");Some(names)}
    }
}

/// Clear scan admission and abandon queued ingest on errors or unwinding.
struct ScanReset { app:Arc<App>, armed:bool }
impl Drop for ScanReset {
    /// Release admission atomically with queue abandonment unless normal drainage disarmed this guard.
    fn drop(&mut self) {
        if !self.armed { return; }
        let mut queue=self.app.scan_queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.requests.clear();
        let mut ingest=self.app.ingest_status.lock().unwrap_or_else(|e| e.into_inner());
        if ingest.as_deref()==Some("indexing") { *ingest=Some("failed".into()); }
        let mut error=self.app.scan_error.lock().unwrap_or_else(|e| e.into_inner());
        if error.is_none() { *error=Some("Scan failed"); }
        self.app.scanning.store(false,std::sync::atomic::Ordering::SeqCst);
    }
}

/// Validate new settings paths strictly, canonicalizing existing directories and preserving ordered deduplication.
fn normalize_roots(value: &serde_json::Value) -> Result<Vec<PathBuf>,String> { parse_roots(value,false) }

/// Persisted canonical/absolute roots remain configured during a temporary outage.
/// Only configuration reads tolerate unavailability; new path writes stay strict.
fn parse_roots(value:&serde_json::Value,retain_unavailable:bool) -> Result<Vec<PathBuf>,String> {
    let list=value.as_array().ok_or("library_paths must be an array")?;
    let mut roots=Vec::new();
    for value in list {
        let text=value.as_str().filter(|s| !s.is_empty()).ok_or("library path must be nonempty")?;
        let path=PathBuf::from(text);
        let normal_absolute=path.is_absolute() && !path.components().any(|c|matches!(c,std::path::Component::ParentDir|std::path::Component::CurDir));
        let root=match path.canonicalize() {
            Ok(root)=>root,
            Err(error) if retain_unavailable && normal_absolute && matches!(error.kind(),std::io::ErrorKind::NotFound|std::io::ErrorKind::PermissionDenied)=>{
                tracing::warn!(operation="configure",path=%path.display(),%error,"configured library unavailable; retaining path without filesystem effects");
                path
            }
            Err(error)=>return Err(error.to_string()),
        };
        match std::fs::metadata(&root) {
            Ok(metadata) if metadata.is_dir()=>(),
            Ok(_)=>return Err("library path must be a directory".into()),
            Err(error) if retain_unavailable && normal_absolute && matches!(error.kind(),std::io::ErrorKind::NotFound|std::io::ErrorKind::PermissionDenied)=>(),
            Err(error)=>return Err(error.to_string()),
        }
        if !roots.contains(&root) {roots.push(root);}
    }
    Ok(roots)
}

/// Read authoritative library_paths, including [], falling back to legacy only when the array key is absent.
/// Typed unavailable absolute roots retain configuration without acquiring filesystem-write authority.
fn stored_roots(db: &Connection) -> Result<Vec<PathBuf>,String> {
    use rusqlite::OptionalExtension;
    let read=|key| -> Result<Option<serde_json::Value>,String> {
        let raw:Option<String>=db.query_row("SELECT value FROM settings WHERE key=?1",[key],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        raw.map(|v|serde_json::from_str(&v).map_err(|e|e.to_string())).transpose()
    };
    if let Some(value)=read("library_paths")? { return parse_roots(&value,true); }
    match read("library_path")? {
        None|Some(serde_json::Value::Null)=>Ok(Vec::new()),
        Some(serde_json::Value::String(s)) if s.is_empty()=>Ok(Vec::new()),
        Some(value)=>parse_roots(&serde_json::json!([value]),true),
    }
}

/// Select one exclusive locked nonempty environment root or the unlocked persisted root array.
/// Empty environment values enable persisted configuration and never add an implicit fallback root.
fn effective_library_config(db: &Connection, env_override: Option<&str>) -> Result<LibraryConfig,String> {
    if let Some(path)=env_override.filter(|s| !s.is_empty()) {
        return Ok(LibraryConfig { roots:parse_roots(&serde_json::json!([path]),true)?,locked:true });
    }
    Ok(LibraryConfig { roots:stored_roots(db)?,locked:false })
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

/// Copy one committed configuration snapshot without rereading settings or merging legacy paths.
fn library_roots(app: &App) -> Vec<PathBuf> { app.library.read().unwrap().roots.clone() }

/// Run admitted scans on a dedicated connection and drain queued snapshots before releasing admission.
/// Unavailable roots/cache boundaries hold filesystem work; errors/unwinds fail abandoned indexing through ScanReset.
fn run_scan(state: Arc<App>, libs: Vec<PathBuf>) {
    tokio::task::spawn_blocking(move || {
        let mut reset=ScanReset {app:state.clone(),armed:true};
        let db=match store::connect(&state.db_path) {
            Ok(db)=>db,
            Err(error)=>{ tracing::error!(%error,"scan database unavailable");*state.scan_error.lock().unwrap()=Some("Scan database unavailable");return; }
        };
        let thumbs=thumbs_dir_for(&state.db_path);
        let mut request={
            let mut queue=state.scan_queue.lock().unwrap();
            if let Some(request)=queue.requests.pop_front() { request }
            else {queue.next_generation+=1;ScanRequest{generation:queue.next_generation,roots:libs,for_ingest:false}}
        };
        loop {
            let ScanRequest{generation,roots,for_ingest}=request;
            *state.scan_error.lock().unwrap()=None;
            let cache=std::env::var_os("FRAYD_TRANSCODES").map(PathBuf::from).unwrap_or_else(|| StdPath::new(&state.db_path).parent().unwrap_or(StdPath::new(".")).join("transcodes"));
            if let Some(root)=roots.iter().find(|root| !root.is_dir()) {
                tracing::warn!(operation="walk",path=%root.display(),generation,"library scan held: configured root unavailable");*state.scan_error.lock().unwrap()=Some("Library root unavailable");return;
            }
            if transcode::validate_cache(&cache,&roots).is_none() || transcode::validate_cache(&thumbs,&roots).is_none() {
                tracing::warn!(generation,"library scan held: cache boundary unavailable or overlapping");*state.scan_error.lock().unwrap()=Some("Cache boundary unavailable");return;
            }
            let mut n=0;
            for root in &roots { n+=scanner::scan_with_roots(&db,state.db_path.clone(),root.clone(),Some(&thumbs),&roots); }
            if for_ingest {
                let indexed=scanner::ingest_indexed(&db,&roots[0].join("Internet"));
                *state.ingest_status.lock().unwrap()=Some(if indexed {"done"} else {"failed"}.into());
            }
            tracing::info!(generation,"scan complete: {n} items");
            let mut queue=state.scan_queue.lock().unwrap();
            if let Some(next)=queue.requests.pop_front() { request=next;continue; }
            state.scanning.store(false,std::sync::atomic::Ordering::SeqCst);
            reset.armed=false;
            drop(queue);
            drop(reset);
            return;
        }
    });
}

/// Queue the download admission snapshot, coalescing only identical pending roots.
/// Queue mutation and CAS worker admission share a mutex so no indexing request loses its worker.
fn queue_ingest_scan(app: Arc<App>, roots: Vec<PathBuf>) {
    let _admission=ROOT_ADMISSION.lock().unwrap();
    let mut queue=app.scan_queue.lock().unwrap();
    if let Some(request)=queue.requests.iter_mut().find(|request|request.roots==roots) { request.for_ingest=true; }
    else {
        queue.next_generation+=1;
        let generation=queue.next_generation;
        queue.requests.push_back(ScanRequest{generation,roots:roots.clone(),for_ingest:true});
    }
    if app.scanning.compare_exchange(false,true,std::sync::atomic::Ordering::SeqCst,std::sync::atomic::Ordering::SeqCst).is_ok() {
        *app.scan_error.lock().unwrap()=None;
        drop(queue);
        run_scan(app,roots);
    }
}

/// POST /api/scan → trigger library rescan. 202 if started, 409 if busy.
async fn trigger_scan(State(app): State<std::sync::Arc<App>>) -> StatusCode {
    use std::sync::atomic::Ordering;
    let _admission=ROOT_ADMISSION.lock().unwrap();
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
            *app.scan_error.lock().unwrap()=None;
            run_scan(app, libs);
            StatusCode::ACCEPTED
        }
        Err(_) => StatusCode::CONFLICT,
    }
}

/// Fail an abandoned downloader without overwriting a successful handoff to queued indexing.
struct IngestReset(Arc<App>);
impl Drop for IngestReset {
    /// Only downloading belongs to this guard; indexing has transferred to the scan coordinator.
    fn drop(&mut self) {
        let mut status=self.0.ingest_status.lock().unwrap_or_else(|e|e.into_inner());
        if status.as_deref()==Some("downloading") { *status=Some("failed".into()); }
    }
}

/// POST /api/ingest { url } → download via yt-dlp into <library>/Internet/, then rescan.
/// Admin-only. yt-dlp is best-effort external binary (like upnpc/ffmpeg).
#[derive(serde::Deserialize)]
struct IngestReq {
    url: String,
}

/// Authorize ingestion and pass one HTTP(S) URL after the downloader option terminator.
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
    let libs;
    let dest;
    {
        let _admission=ROOT_ADMISSION.lock().unwrap();
        libs=library_roots(&app);
        if libs.is_empty() { return StatusCode::NOT_IMPLEMENTED; }
        // An unavailable configured root cannot grant authority to recreate media directories.
        let cache=std::env::var_os("FRAYD_TRANSCODES").map(PathBuf::from).unwrap_or_else(|| StdPath::new(&app.db_path).parent().unwrap_or(StdPath::new(".")).join("transcodes"));
        if libs.iter().any(|root| !root.is_dir()) || transcode::validate_cache(&cache,&libs).is_none()
            || transcode::validate_cache(&thumbs_dir_for(&app.db_path),&libs).is_none() {return StatusCode::SERVICE_UNAVAILABLE;}
        let mut st=app.ingest_status.lock().unwrap();
        if matches!(st.as_deref(),Some("downloading"|"indexing")) { return StatusCode::CONFLICT; }
        *st=Some("downloading".into());
        dest=libs[0].join("Internet");
        if std::fs::create_dir_all(&dest).is_err() { *st=Some("failed".into());return StatusCode::INTERNAL_SERVER_ERROR; }
    }
    let state = app.clone();
    let url = body.url;
    // spawn detached: download can take minutes; rescan after success
    tokio::task::spawn_blocking(move || {
        let _reset=IngestReset(state.clone());
        let out = dest.join("%(title)s.%(ext)s");
        let ok = std::process::Command::new("yt-dlp")
            .args(["-f", "mp4", "--no-playlist", "-o"])
            .arg(out)
            .arg("--")
            .arg(&url)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            *state.ingest_status.lock().unwrap()=Some("indexing".into());
            queue_ingest_scan(state.clone(),libs);
        } else { *state.ingest_status.lock().unwrap()=Some("failed".into()); }
        tracing::info!("ingest yt-dlp {url}: {}", if ok { "ok" } else { "failed" });
    });
    StatusCode::ACCEPTED
}

#[derive(serde::Deserialize)]
struct PartyCreateReq {
    /// Identity key (token-derived or LAN fallback).
    identity: String,
}

/// POST /api/party → { code, host_capability }; the proof is returned only to this creator.
async fn party_create(
    State(app): State<Arc<App>>, headers:axum::http::HeaderMap, Json(req): Json<PartyCreateReq>,
) -> axum::response::Response {
    let key={
        let db=app.db.lock().unwrap();
        match party_identity_key(&db,&headers,Some(&req.identity)) {
            Ok(key)=>key,
            Err(status)=>return status.into_response(),
        }
    };
    let (code,host_capability)=app.party.create(&key);
    (StatusCode::OK,Json(serde_json::json!({"code":code,"host_capability":host_capability.to_string()}))).into_response()
}

/// Both HTTP creation and socket registration use the canonical token-first identity.
fn party_identity_key(db:&Connection,headers:&axum::http::HeaderMap,identity:Option<&str>) -> Result<String,StatusCode> {
    let id=resolve_identity(db,headers,identity)?;
    db.query_row("SELECT key FROM identities WHERE id=?1",[id],|r|r.get(0)).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/party/:code/ws — join party socket. Identity via ?identity= or Bearer.
async fn party_ws(
    State(app): State<std::sync::Arc<App>>,
    Path(code): Path<String>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let Ok(host_proof) = party::host_proof(&headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(admission) = app.party.authorize(&code, host_proof) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let key={
        let db=app.db.lock().unwrap();
        match party_identity_key(&db,&headers,q.get("identity").map(String::as_str)) {
            Ok(key)=>key,
            Err(status)=>return status.into_response(),
        }
    };
    if !app.party.can_register(&admission, &key) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.protocols(["frayd-party"])
        .on_upgrade(move |socket| party::member_socket(app.party.clone(), socket, admission, key))
        .into_response()
}

/// Report scanning, configured/count metadata and ingest lifecycle from one effective configuration read.
/// Paths remain confined to authorized settings responses.
async fn scan_status(State(app): State<std::sync::Arc<App>>) -> Json<serde_json::Value> {
    let config=app.library.read().unwrap();
    Json(serde_json::json!({
        "scanning": app.scanning.load(std::sync::atomic::Ordering::SeqCst),
        "scan_error": *app.scan_error.lock().unwrap(),
        "library_configured": !config.roots.is_empty(),
        "library_count":config.roots.len(),
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
    let name=cache_spine(&app,id,&path).await
        .and_then(|spine|usize::try_from(n).ok().and_then(|index|spine.get(index).cloned()));
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

/// Reuse unchanged cached reading order or compute a missing spine outside the application DB mutex.
async fn cache_spine(app: &Arc<App>, id: i64, path: &str) -> Option<Vec<String>> {
    let version={
        let db=app.db.lock().unwrap();
        let (meta,mtime,size):(Option<String>,Option<i64>,Option<i64>)=db.query_row(
            "SELECT meta_json,mtime_secs,size_bytes FROM media WHERE id=?1",[id],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).ok()?;
        let cached=meta.and_then(|value|serde_json::from_str::<serde_json::Value>(&value).ok())
            .and_then(|value|serde_json::from_value::<Vec<String>>(value["spine"].clone()).ok());
        if let Some(spine)=cached {return Some(spine);}
        (mtime,size)
    };
    let archive=PathBuf::from(path);
    let spine=tokio::task::spawn_blocking(move ||epub::epub_spine(&archive)).await.ok()??;
    let db=app.db.lock().unwrap();
    publish_spine_cache(&db,id,path,version,spine)
}

/// Merge only a source-current spine, rejecting changed snapshots without replacing other metadata.
/// Optional SQLite cache failures retain the computed reading result, preserving existing HTTP behavior.
fn publish_spine_cache(db: &Connection, id: i64, path: &str,
    version: (Option<i64>,Option<i64>), spine: Vec<String>) -> Option<Vec<String>> {
    match db.execute("UPDATE media SET meta_json=json_set(COALESCE(meta_json,'{}'),'$.spine',json(?1))
        WHERE id=?2 AND path=?3 AND mtime_secs IS ?4 AND size_bytes IS ?5",
        rusqlite::params![serde_json::json!(&spine).to_string(),id,path,version.0,version.1]) {
        Ok(1)=>Some(spine),
        Ok(_)=>None,
        Err(error)=>{tracing::warn!(%error,id,"EPUB spine cache write held");Some(spine)}
    }
}

/// Look up the indexed source path for a media ID; return None for a missing row.
async fn media_path(app: &std::sync::Arc<App>, id: i64) -> Option<String> {
    let db = app.db.lock().unwrap();
    db.query_row("SELECT path FROM media WHERE id = ?1", [id], |r| r.get(0))
        .ok()
}

/// Resolve and recheck administrative authority under the caller's write transaction.
fn transaction_admin(db: &Connection, headers: &axum::http::HeaderMap, qid: Option<&str>) -> Result<i64,StatusCode> {
    use rusqlite::OptionalExtension;
    let iid=resolve_identity(db,headers,qid).map_err(|status| if status==StatusCode::INTERNAL_SERVER_ERROR {status} else {StatusCode::FORBIDDEN})?;
    let admin:Option<i64>=db.query_row("SELECT is_admin FROM user_perms WHERE identity_id=?1",[iid],|r|r.get(0)).optional().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
    if admin==Some(1) { Ok(iid) } else { Err(StatusCode::FORBIDDEN) }
}

/// Remove only another account's owned child rows, atomically with the identity.
async fn remove_user(
    State(app): State<Arc<App>>, headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String,String>>, Path(id): Path<i64>,
) -> StatusCode {
    use rusqlite::OptionalExtension;
    let mut db=app.db.lock().unwrap();
    let result=(|| -> Result<(),StatusCode> {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        let caller=transaction_admin(&tx,&headers,q.get("identity").map(String::as_str))?;
        if caller==id {return Err(StatusCode::CONFLICT);}
        let kind:Option<String>=tx.query_row("SELECT kind FROM identities WHERE id=?1",[id],|r|r.get(0)).optional().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if kind.as_deref()!=Some("account") {return Err(StatusCode::NOT_FOUND);}
        let target_admin:i64=tx.query_row("SELECT COALESCE((SELECT is_admin FROM user_perms WHERE identity_id=?1),0)",[id],|r|r.get(0)).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        let other_admins:i64=tx.query_row("SELECT COUNT(*) FROM user_perms WHERE is_admin=1 AND identity_id!=?1",[id],|r|r.get(0)).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if target_admin==1 && other_admins==0 {return Err(StatusCode::CONFLICT);}
        for sql in [
            "DELETE FROM playlist_items WHERE playlist_id IN (SELECT id FROM playlists WHERE identity_id=?1)",
            "DELETE FROM playlists WHERE identity_id=?1",
            "DELETE FROM sessions WHERE identity_id=?1",
            "DELETE FROM user_perms WHERE identity_id=?1",
            "DELETE FROM progress WHERE identity_id=?1",
            "DELETE FROM annotations WHERE identity_id=?1",
        ] {tx.execute(sql,[id]).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;}
        let deleted=tx.execute("DELETE FROM identities WHERE id=?1",[id]).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if deleted!=1 {return Err(StatusCode::INTERNAL_SERVER_ERROR);}
        tx.commit().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)
    })();
    match result {Ok(())=>StatusCode::NO_CONTENT,Err(status)=>status}
}

/// Remove a proven, unshared cache thumbnail only after its media row committed deletion.
/// A second write transaction prevents new database references during optional cleanup.
fn cleanup_owned_thumbnail(db: &mut Connection, thumbs: &StdPath, roots: &[PathBuf], id:i64,
    source:&str, thumbnail:Option<&str>) -> Result<(),String> {
    let Some(thumbnail)=thumbnail else {return Ok(());};
    let expected=thumbs.join(format!("{id}.jpg"));
    let metadata=match std::fs::symlink_metadata(&expected) {
        Ok(metadata)=>metadata,
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(()),
        Err(error)=>return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {return Err("thumbnail is not an owned regular file".into());}
    let directory=thumbs.canonicalize().map_err(|e|e.to_string())?;
    let owned=expected.canonicalize().map_err(|e|e.to_string())?;
    let saved=StdPath::new(thumbnail);
    let saved_metadata=std::fs::symlink_metadata(saved).map_err(|e|e.to_string())?;
    if saved_metadata.file_type().is_symlink() || !saved_metadata.is_file()
        || owned!=directory.join(format!("{id}.jpg")) || saved.canonicalize().map_err(|e|e.to_string())?!=owned {
        return Err("thumbnail path does not prove configured cache ownership".into());
    }
    if StdPath::new(source).canonicalize().ok().as_ref()==Some(&owned) {return Err("thumbnail is the original source".into());}
    for root in roots {
        let canonical=root.canonicalize().map_err(|e|e.to_string())?;
        if owned.starts_with(canonical) {return Err("thumbnail overlaps a configured library".into());}
    }
    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    {
        let mut statement=tx.prepare("SELECT path FROM media UNION ALL SELECT thumb_path FROM media WHERE thumb_path IS NOT NULL
            UNION ALL SELECT transcode_path FROM media WHERE transcode_path IS NOT NULL
            UNION ALL SELECT path FROM transcode_artifacts UNION ALL SELECT owner_source_path FROM transcode_artifacts").map_err(|e|e.to_string())?;
        let paths=statement.query_map([],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?;
        for path in paths {
            let path=path.map_err(|e|e.to_string())?;
            if StdPath::new(&path).canonicalize().ok().as_ref()==Some(&owned) {return Err("thumbnail still has a database reference".into());}
        }
    }
    std::fs::remove_file(&owned).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e|e.to_string())
}

/// Delete only an unreferenced media row after rechecking admin and converter admission in a write transaction.
/// Return 403/404/409/500 for denial/missing/references-or-busy/database failure; committed deletion returns 204.
/// Source files never grant unlink authority; optional thumbnail cleanup follows the committed row deletion.
async fn delete_media(
    State(app): State<Arc<App>>, headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String,String>>, Path(id): Path<i64>,
) -> StatusCode {
    use rusqlite::OptionalExtension;
    // Serialize conversion admission and retain one complete root snapshot through cleanup.
    let _admission=ROOT_ADMISSION.lock().unwrap();
    let config=app.library.read().unwrap();
    let mut db=app.db.lock().unwrap();
    let result=(|| -> Result<(String,Option<String>),StatusCode> {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        transaction_admin(&tx,&headers,q.get("identity").map(String::as_str))?;
        let row:Option<(String,Option<String>,Option<String>)>=tx.query_row("SELECT path,thumb_path,transcode_path FROM media WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        let Some((source,thumbnail,conversion))=row else {return Err(StatusCode::NOT_FOUND);};
        if conversion.is_some() || transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst) {return Err(StatusCode::CONFLICT);}
        let referenced:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM progress WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM annotations WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM tracks WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM playlist_items WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM media WHERE transcode_path=?2)
            OR EXISTS(SELECT 1 FROM transcode_artifacts WHERE owner_media_id=?1 OR path=?2 OR owner_source_path=?2)",rusqlite::params![id,source],|r|r.get(0)).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if referenced {return Err(StatusCode::CONFLICT);}
        let deleted=tx.execute("DELETE FROM media WHERE id=?1",[id]).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if deleted!=1 {return Err(StatusCode::INTERNAL_SERVER_ERROR);}
        tx.commit().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok((source,thumbnail))
    })();
    match result {
        Err(status)=>status,
        Ok((source,thumbnail))=>{
            // An admitted scan may regenerate the same ID's cache; leave optional bytes alone.
            if !app.scanning.load(std::sync::atomic::Ordering::SeqCst) {
                if let Err(error)=cleanup_owned_thumbnail(&mut db,&thumbs_dir_for(&app.db_path),&config.roots,id,&source,thumbnail.as_deref()) {
                    tracing::warn!(id,%error,"media deletion committed; thumbnail cleanup held");
                }
            }
            StatusCode::NO_CONTENT
        }
    }
}

/// List indexed media with optional text search and identity-scoped kind permissions.
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
    let filter = q.get("q").map(|s| format!("%{}%", s.replace(['%', '_'], "")));
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

/// Stream the indexed conversion or source with optional byte-range start; missing files return 404.
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


/// Map a path suffix to the response MIME type, defaulting to application/octet-stream.
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

/// Resolve the request identity and persist a media position, returning an HTTP status.
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

/// Return identity-scoped saved progress for a media item, or an HTTP error.
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

/// List annotations for one media item under the resolved identity.
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

/// Upsert an annotation for the resolved bearer identity, returning 201 or the auth/database failure status.
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

/// Delete an annotation only under its resolved owning identity.
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

/// Atomically update validated permissions while preserving at least one administrator.
/// Omitted kinds preserve the prior restriction, null clears it, and [] denies every kind.
async fn set_perm(
    State(app): State<Arc<App>>, headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String,String>>, Json(body): Json<serde_json::Value>,
) -> StatusCode {
    use rusqlite::OptionalExtension;
    let mut db=app.db.lock().unwrap();
    let result=(|| -> Result<(),StatusCode> {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        transaction_admin(&tx,&headers,q.get("identity").map(String::as_str))?;
        let map=body.as_object().ok_or(StatusCode::BAD_REQUEST)?;
        let iid=body["identity_id"].as_i64().ok_or(StatusCode::BAD_REQUEST)?;
        let admin=match map.get("is_admin") {None=>None,Some(value)=>Some(value.as_bool().ok_or(StatusCode::BAD_REQUEST)?)};
        let kinds_present=map.contains_key("allowed_kinds");
        let kinds=match map.get("allowed_kinds") {
            None|Some(serde_json::Value::Null)=>None,
            Some(serde_json::Value::Array(values))=>{
                if !values.iter().all(|v|v.as_str().is_some_and(|kind|matches!(kind,"audiobook"|"ebook"|"comic"|"manga"|"movie"|"music"|"episode"))) {return Err(StatusCode::BAD_REQUEST);}
                Some(serde_json::json!(values).to_string())
            }
            Some(_)=>return Err(StatusCode::BAD_REQUEST),
        };
        let target:Option<i64>=tx.query_row("SELECT id FROM identities WHERE id=?1",[iid],|r|r.get(0)).optional().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if target.is_none() {return Err(StatusCode::NOT_FOUND);}
        if admin==Some(false) {
            let other_admins:i64=tx.query_row("SELECT COUNT(*) FROM user_perms WHERE is_admin=1 AND identity_id!=?1",[iid],|r|r.get(0)).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
            if other_admins==0 {return Err(StatusCode::CONFLICT);}
        }
        let changed=tx.execute("INSERT INTO user_perms(identity_id,is_admin,allowed_kinds) VALUES(?1,?2,?3)
            ON CONFLICT(identity_id) DO UPDATE SET
            is_admin=CASE WHEN ?4 THEN excluded.is_admin ELSE user_perms.is_admin END,
            allowed_kinds=CASE WHEN ?5 THEN excluded.allowed_kinds ELSE user_perms.allowed_kinds END",
            rusqlite::params![iid,admin.unwrap_or(false) as i64,kinds,admin.is_some(),kinds_present]).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if changed!=1 {return Err(StatusCode::INTERNAL_SERVER_ERROR);}
        tx.commit().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)
    })();
    match result {Ok(())=>StatusCode::NO_CONTENT,Err(status)=>status}
}

/// GET /api/admin/settings → {key: value,...}
async fn get_settings(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let config=app.library.read().unwrap();
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
    // Persisted paths and effective override are distinct authorized metadata.
    let normalized=stored_roots(&db).unwrap_or_else(|_| config.roots.clone());
    map.remove("library_path");
    map.insert("library_paths".into(),serde_json::json!(normalized));
    map.insert("library_locked".into(),serde_json::json!(config.locked));
    map.insert("effective_library_paths".into(),serde_json::json!(config.roots));
    map.insert("server_version".into(),serde_json::json!(VERSION));
    Json(serde_json::Value::Object(map)).into_response()
}

/// Validate the whole settings payload before one immediate transaction and publish roots only after commit.
/// Admission/config/database lock ordering prevents jobs from retaining an obsolete newly expanded root set.
async fn set_setting(
    State(app): State<Arc<App>>, headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String,String>>, Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    // Global order: admission -> config -> database -> ingest. Workers never hold
    // application database/config locks during filesystem traversal.
    let _admission=ROOT_ADMISSION.lock().unwrap();
    let mut config=app.library.write().unwrap();
    let mut db=app.db.lock().unwrap();
    if require_admin(&db,&headers,q.get("identity").map(String::as_str)).is_err() { return StatusCode::FORBIDDEN.into_response(); }
    let Some(mut map)=body.as_object().cloned() else { return StatusCode::BAD_REQUEST.into_response(); };
    let paths_changed=map.contains_key("library_path") || map.contains_key("library_paths");
    if map.contains_key("library_path") && map.contains_key("library_paths") { return StatusCode::BAD_REQUEST.into_response(); }
    if paths_changed && (config.locked || app.scanning.load(std::sync::atomic::Ordering::SeqCst)
        || transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst)
        || matches!(app.ingest_status.lock().unwrap().as_deref(),Some("downloading"|"indexing"))) {
        return StatusCode::CONFLICT.into_response();
    }
    if map.keys().any(|k| matches!(k.as_str(),"library_locked"|"effective_library_paths"|"server_version")) { return StatusCode::BAD_REQUEST.into_response(); }
    let roots=if paths_changed {
        let value=if let Some(value)=map.remove("library_path") {
            if value.is_null() || value=="" { serde_json::json!([]) } else { serde_json::json!([value]) }
        } else { map.remove("library_paths").unwrap() };
        let Ok(roots)=normalize_roots(&value) else { return StatusCode::BAD_REQUEST.into_response(); };
        let cache=std::env::var_os("FRAYD_TRANSCODES").map(PathBuf::from).unwrap_or_else(|| StdPath::new(&app.db_path).parent().unwrap_or(StdPath::new(".")).join("transcodes"));
        if !roots.is_empty() && (transcode::validate_cache(&cache,&roots).is_none() || transcode::validate_cache(&thumbs_dir_for(&app.db_path),&roots).is_none()) { return StatusCode::BAD_REQUEST.into_response(); }
        map.insert("library_paths".into(),serde_json::json!(roots));
        Some(roots)
    } else { None };
    for (key,value) in &map {
        let valid=match key.as_str() {
            "library_paths"=>true,
            "port"=>value.as_u64().is_some_and(|p| p>0 && p<=65535),
            "upnp"=>value.is_boolean(),
            "tls_cert"|"tls_key"=>value.is_string() || value.is_null(),
            _=>true,
        };
        if !valid { return StatusCode::BAD_REQUEST.into_response(); }
    }
    let result=(|| -> rusqlite::Result<()> {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (key,value) in map {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",rusqlite::params![key,value.to_string()])?;
        }
        if paths_changed { tx.execute("DELETE FROM settings WHERE key='library_path'",[])?; }
        tx.commit()
    })();
    if result.is_err() { return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    if let Some(roots)=roots { config.roots=roots; }
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
mod backend_regressions {
    use super::*;
    /// Isolate legacy metadata and saved progress without using an operator database.
    fn app_for(path: &str) -> Arc<App> {
        let db = store::open(":memory:");
        db.execute("INSERT INTO media(id,kind,path,title,meta_json) VALUES(1,'comic',?1,'Original',?2)",
            rusqlite::params![path,r#"{"pages":["À.png","Á.png","Â.png"],"synopsis":"keep"}"#]).unwrap();
        db.execute("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture')", []).unwrap();
        db.execute("INSERT INTO progress(identity_id,media_id,position) VALUES(1,1,7)", []).unwrap();
        Arc::new(App { db: Mutex::new(db), db_path: String::new(), library: std::sync::RwLock::new(LibraryConfig::default()), scan_queue:Mutex::new(ScanQueue::default()), ingest_status: Mutex::new(None),
            scanning: std::sync::atomic::AtomicBool::new(false), scan_error:Mutex::new(None), auth_rl: auth::RateLimiter::new(300,10),
            party: Arc::new(party::Party::new()) })
    }
    /// Isolate an account owner, device owner, and private rows for identity-boundary tests.
    fn identity_boundary_app() -> Arc<App> {
        let app = app_for("synthetic.cbz");
        {
            let db = app.db.lock().unwrap();
            db.execute("INSERT INTO identities(id,kind,key) VALUES(2,'account','owner@example.invalid')", []).unwrap();
            db.execute("INSERT INTO user_perms(identity_id,is_admin) VALUES(2,1)", []).unwrap();
            db.execute("INSERT INTO annotations(id,identity_id,media_id,kind,spine_idx,cfi,text) VALUES(1,2,1,'highlight',0,'char:0:5','owner')", []).unwrap();
            db.execute("INSERT INTO progress(identity_id,media_id,position,updated_at) VALUES(2,1,42,unixepoch())", []).unwrap();
        }
        app
    }

    /// A public account key must never authorize private reads, writes, deletion, or administration.
    #[tokio::test]
    async fn account_key_fallback_cannot_cross_identity_boundary() {
        let app = identity_boundary_app();
        let query = std::collections::HashMap::from([("identity".into(), "owner@example.invalid".into()), ("ann_id".into(), "1".into())]);
        let headers = axum::http::HeaderMap::new();
        let statuses = [
            list_annotations(State(app.clone()), Path(1), headers.clone(), Query(query.clone())).await.into_response().status(),
            get_progress(State(app.clone()), Path(1), headers.clone(), Query(query.clone())).await.into_response().status(),
            set_progress(State(app.clone()), headers.clone(), Json(ProgressIn { identity: Some("owner@example.invalid".into()), media_id: 1, position: 99.0 })).await,
            delete_annotation(State(app.clone()), headers.clone(), Query(query.clone())).await,
            list_perms(State(app.clone()), headers.clone(), Query(query.clone())).await.into_response().status(),
        ];
        assert_eq!(statuses, [StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN]);
        let mut invalid = headers;
        invalid.insert(header::AUTHORIZATION, "Bearer synthetic-invalid-session".parse().unwrap());
        let db = app.db.lock().unwrap();
        assert_eq!(resolve_identity(&db, &invalid, Some("owner@example.invalid")), Err(StatusCode::UNAUTHORIZED));
        assert_eq!(require_admin(&db, &invalid, Some("owner@example.invalid")), Err(StatusCode::UNAUTHORIZED));
        assert_eq!(db.query_row("SELECT position FROM progress WHERE identity_id=2", [], |r| r.get::<_,f64>(0)).unwrap(), 42.0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM annotations WHERE id=1 AND identity_id=2", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM identities", [], |r| r.get::<_,i64>(0)).unwrap(), 2);
    }

    /// Valid sessions retain precedence; ordinary device fallback and account-only annotation creation remain intact.
    #[tokio::test]
    async fn device_fallback_and_authenticated_account_remain_compatible() {
        let app = identity_boundary_app();
        let mut headers = axum::http::HeaderMap::new();
        {
            let db = app.db.lock().unwrap();
            let token = auth::create_session(&db, 2).unwrap();
            headers.insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
            assert_eq!(resolve_identity(&db, &headers, Some("fixture")), Ok(2));
            assert_eq!(require_admin(&db, &headers, Some("fixture")), Ok(2));
            assert_eq!(resolve_identity(&db, &axum::http::HeaderMap::new(), Some("fixture")), Ok(1));
            let mut invalid = axum::http::HeaderMap::new();
            invalid.insert(header::AUTHORIZATION, "Bearer synthetic-invalid-session".parse().unwrap());
            assert_eq!(resolve_identity(&db, &invalid, Some("fixture")), Ok(1));
            let created = resolve_identity(&db, &axum::http::HeaderMap::new(), Some("new-device")).unwrap();
            assert_eq!(resolve_identity(&db, &axum::http::HeaderMap::new(), Some("new-device")), Ok(created));
            assert_eq!(db.query_row("SELECT kind FROM identities WHERE id=?1", [created], |r| r.get::<_,String>(0)).unwrap(), "device");
            assert_eq!(resolve_identity(&db, &axum::http::HeaderMap::new(), None), Err(StatusCode::UNAUTHORIZED));
        }
        let query = std::collections::HashMap::from([("identity".into(), "fixture".into())]);
        assert_eq!(list_annotations(State(app.clone()), Path(1), headers.clone(), Query(query.clone())).await.into_response().status(), StatusCode::OK);
        assert_eq!(set_progress(State(app.clone()), axum::http::HeaderMap::new(), Json(ProgressIn { identity: Some("fixture".into()), media_id: 1, position: 8.0 })).await, StatusCode::NO_CONTENT);
        assert_eq!(get_progress(State(app.clone()), Path(1), axum::http::HeaderMap::new(), Query(query)).await.into_response().status(), StatusCode::OK);
        let annotation = || AnnotationIn { media_id: 1, kind: "highlight".into(), spine_idx: 0, cfi: "char:8:3".into(), text: Some("new".into()), color: None, note: None };
        assert_eq!(create_annotation(State(app.clone()), axum::http::HeaderMap::new(), Json(annotation())).await, StatusCode::UNAUTHORIZED);
        assert_eq!(create_annotation(State(app.clone()), headers.clone(), Json(annotation())).await, StatusCode::CREATED);
        let query = std::collections::HashMap::from([("identity".into(), "fixture".into()), ("ann_id".into(), "1".into())]);
        assert_eq!(delete_annotation(State(app.clone()), axum::http::HeaderMap::new(), Query(query.clone())).await, StatusCode::NOT_FOUND);
        assert_eq!(delete_annotation(State(app.clone()), headers, Query(query)).await, StatusCode::NO_CONTENT);
        let db = app.db.lock().unwrap();
        assert_eq!(db.query_row("SELECT position FROM progress WHERE identity_id=1", [], |r| r.get::<_,f64>(0)).unwrap(), 8.0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM annotations WHERE identity_id=2 AND cfi='char:8:3'", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
    }

    /// Build an actual minimal EPUB with explicit OPF reading order and literal chapter payloads.
    fn write_epub_fixture(path: &StdPath, chapters: &[(&str,&[u8])]) {
        use std::io::Write;
        let mut manifest=String::new();let mut spine=String::new();
        for (index,(href,_)) in chapters.iter().enumerate() {
            manifest.push_str(&format!("<item id=\"c{index}\" href=\"{href}\" media-type=\"application/xhtml+xml\"/>"));
            spine.push_str(&format!("<itemref idref=\"c{index}\"/>"));
        }
        let opf=format!("<package><manifest>{manifest}</manifest><spine>{spine}</spine></package>");
        let mut archive=zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name,bytes) in [("META-INF/container.xml",b"<container><rootfiles><rootfile full-path=\"book.opf\"/></rootfiles></container>".as_slice()),("book.opf",opf.as_bytes())].into_iter().chain(chapters.iter().copied()) {
            archive.start_file(name,options).unwrap();archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }
    /// Replace a warmed EPUB and observe current chapter bytes after rescan without losing user state.
    #[tokio::test]
    async fn rescan_invalidates_replaced_epub_spine_and_lazy_reads_current_chapters() {
        let root=std::env::temp_dir().join(format!("frayd-epub-refresh-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();let path=root.join("book.epub");
        write_epub_fixture(&path,&[("old-one.xhtml",b"<p>Old one</p>"),("old-two.xhtml",b"<p>Old two</p>"),("old-three.xhtml",b"<p>Old three</p>")]);
        let original=std::fs::metadata(&path).unwrap();let app=app_for(path.to_str().unwrap());
        {
            let db=app.db.lock().unwrap();
            db.execute(r#"UPDATE media SET kind='ebook',mtime_secs=?1,size_bytes=?2,
                meta_json=json_set(meta_json,'$.custom',json('{"nested":"keep"}')) WHERE id=1"#,
                rusqlite::params![original.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,original.len() as i64]).unwrap();
            db.execute("UPDATE progress SET position=1 WHERE media_id=1",[]).unwrap();
        }
        let response=epub_chunk(State(app.clone()),Path((1,0))).await.into_response();
        assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"<p>Old one</p>"[..]);
        {
            let db=app.db.lock().unwrap();
            let before:String=db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
            assert_eq!(scanner::scan(&db,String::new(),root.clone(),None),0);
            assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
        }
        write_epub_fixture(&path,&[("new-one.xhtml",b"<p>Replacement first chapter</p>"),("new-two.xhtml",b"<p>Replacement second chapter</p>")]);
        assert_ne!(std::fs::metadata(&path).unwrap().len(),original.len());
        std::fs::File::open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap())).unwrap();
        let cache_after_scan={
            let db=app.db.lock().unwrap();assert_eq!(scanner::scan(&db,String::new(),root.clone(),None),1);
            db.query_row("SELECT json_type(meta_json,'$.spine') FROM media WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap()
        };
        println!("EPUB_CACHE_AFTER_REPLACEMENT_SCAN={cache_after_scan:?}");
        let response=epub_chunk(State(app.clone()),Path((1,0))).await.into_response();
        assert_eq!(response.status(),StatusCode::OK,"replacement chunk still uses an old cached href");
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"<p>Replacement first chapter</p>"[..]);
        assert_eq!(cache_after_scan,None,"rescan did not clear the obsolete spine");
        let response=epub_spine(State(app.clone()),Path(1)).await.into_response();
        let names:Vec<String>=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap();
        assert_eq!(names,vec!["new-one.xhtml","new-two.xhtml"]);
        let response=epub_chunk(State(app.clone()),Path((1,1))).await.into_response();
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"<p>Replacement second chapter</p>"[..]);
        assert_eq!(epub_chunk(State(app.clone()),Path((1,2))).await.into_response().status(),StatusCode::NOT_FOUND);
        let db=app.db.lock().unwrap();let meta:String=db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();let meta:serde_json::Value=serde_json::from_str(&meta).unwrap();
        assert_eq!(meta["spine"],serde_json::json!(names));assert_eq!(meta["synopsis"],"keep");assert_eq!(meta["custom"]["nested"],"keep");assert_eq!(meta["pages"],serde_json::json!(["À.png","Á.png","Â.png"]));
        assert_eq!(db.query_row("SELECT id FROM media WHERE path=?1",[path.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT title FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"Original");
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),1.0);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Preserve existing cached chunk order while the spine endpoint retains fresh-parse ordering.
    #[tokio::test]
    async fn unchanged_epub_chunk_cache_keeps_legacy_order() {
        let path=concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/stored.epub");let app=app_for(path);
        app.db.lock().unwrap().execute(r#"UPDATE media SET kind='ebook',meta_json=json_set(meta_json,'$.spine',json('["two.xhtml","one.xhtml"]')) WHERE id=1"#,[]).unwrap();
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        let response=epub_chunk(State(app.clone()),Path((1,0))).await.into_response();assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"<p>Chapter two</p>"[..]);
        let response=epub_spine(State(app.clone()),Path(1)).await.into_response();
        assert_eq!(serde_json::from_slice::<Vec<String>>(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap(),vec!["one.xhtml","two.xhtml"]);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
    }
    /// A failed archive read leaves metadata intact and preserves the existing 404 response.
    #[tokio::test]
    async fn unreadable_epub_does_not_publish_a_spine() {
        let app=app_for("/does-not-exist.epub");let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        assert_eq!(epub_chunk(State(app.clone()),Path((1,0))).await.into_response().status(),StatusCode::NOT_FOUND);
        assert_eq!(epub_spine(State(app.clone()),Path(1)).await.into_response().status(),StatusCode::NOT_FOUND);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
    }
    /// Optional cache-write refusal must not turn a successfully read chapter into a read failure.
    #[tokio::test]
    async fn epub_cache_write_failure_keeps_successful_chunk_response() {
        let app=app_for(concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/stored.epub"));
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_spine BEFORE UPDATE OF meta_json ON media BEGIN SELECT RAISE(FAIL,'fixture cache write refused'); END;").unwrap();
        let response=epub_chunk(State(app.clone()),Path((1,0))).await.into_response();assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"<p>Chapter one</p>"[..]);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
    }
    /// Reject stale source snapshots while retaining newer metadata and the computed-read fallback contract.
    #[test]
    fn epub_spine_publication_checks_version_and_merges_only_spine() {
        let app=app_for("fixture.epub");let db=app.db.lock().unwrap();
        for (mtime,size) in [(11,100),(10,101)] {
            db.execute("UPDATE media SET mtime_secs=?1,size_bytes=?2 WHERE id=1",rusqlite::params![mtime,size]).unwrap();
            let before:String=db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
            assert_eq!(publish_spine_cache(&db,1,"fixture.epub",(Some(10),Some(100)),vec!["stale.xhtml".into()]),None);
            assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
        }
        db.execute(r#"UPDATE media SET mtime_secs=10,size_bytes=100,
            meta_json=json_set(meta_json,'$.custom',json('{"nested":"newer concurrent metadata"}')) WHERE id=1"#,[]).unwrap();
        assert_eq!(publish_spine_cache(&db,1,"other.epub",(Some(10),Some(100)),vec!["wrong-path.xhtml".into()]),None);
        assert_eq!(publish_spine_cache(&db,1,"fixture.epub",(Some(10),Some(100)),vec!["fresh.xhtml".into()]),Some(vec!["fresh.xhtml".into()]));
        let meta:String=db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();let meta:serde_json::Value=serde_json::from_str(&meta).unwrap();
        assert_eq!(meta["spine"],serde_json::json!(["fresh.xhtml"]));assert_eq!(meta["synopsis"],"keep");
        assert_eq!(meta["custom"]["nested"],"newer concurrent metadata");assert_eq!(meta["pages"],serde_json::json!(["À.png","Á.png","Â.png"]));
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
    }
    /// Known replacements must stream current source bytes while valid unchanged/unknown legacy playback survives.
    #[tokio::test]
    async fn source_replacement_does_not_stream_a_stale_conversion() {
        for known_prior in [true,false] {
            let root=std::env::temp_dir().join(format!("frayd-video-version-{}",uuid::Uuid::new_v4()));
            let library=root.join("library");std::fs::create_dir_all(&library).unwrap();
            let src=library.join("Film.mkv"); let genuine=library.join("Film.browser.mp4");
            std::fs::write(&src,b"original").unwrap(); std::fs::write(&genuine,b"genuine legacy playback").unwrap();
            let original=std::fs::metadata(&src).unwrap(); let actual_mtime=original.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            let app=app_for(src.to_str().unwrap());
            {
                let db=app.db.lock().unwrap();
                db.execute("UPDATE media SET kind='movie',mtime_secs=?1,size_bytes=8,transcode_path=?2 WHERE id=1",
                    rusqlite::params![if known_prior {Some(actual_mtime)} else {None},genuine.to_str().unwrap()]).unwrap();
                let meta=std::fs::metadata(&genuine).unwrap();
                db.execute("INSERT INTO media(id,kind,path,title,mtime_secs,size_bytes) VALUES(2,'movie',?1,'Genuine',?2,?3)",
                    rusqlite::params![genuine.to_str().unwrap(),meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,meta.len() as i64]).unwrap();
                if known_prior { assert_eq!(scanner::scan(&db,String::new(),library.clone(),None),0); }
            }
            let response=stream(State(app.clone()),Path(1),axum::http::HeaderMap::new()).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"genuine legacy playback"[..]);
            std::fs::write(&src,b"new replacement source bytes").unwrap();
            std::fs::File::open(&src).unwrap().set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap())).unwrap();
            { let db=app.db.lock().unwrap();scanner::scan(&db,String::new(),library.clone(),None); }
            let response=stream(State(app.clone()),Path(1),axum::http::HeaderMap::new()).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            let want:&[u8]=if known_prior {b"new replacement source bytes"} else {b"genuine legacy playback"};
            assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),want,"stale playback for known_prior={known_prior}");
            let db=app.db.lock().unwrap();
            assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
            assert_eq!(db.query_row("SELECT title FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"Original");
            assert_eq!(db.query_row("SELECT json_extract(meta_json,'$.synopsis') FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"keep");
            assert_eq!(db.query_row("SELECT id FROM media WHERE path=?1",[genuine.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),2);
            assert_eq!(std::fs::read(genuine).unwrap(),b"genuine legacy playback");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    /// Write a small real CBZ whose image bytes and natural page order are explicit.
    fn write_comic_fixture(path: &std::path::Path, pages: &[(&str,&[u8])]) {
        use std::io::Write;
        let mut archive=zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name,bytes) in pages {
            archive.start_file(*name,zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }
    /// A changed archive must refresh both endpoints while unchanged cached order and user state survive.
    #[tokio::test]
    async fn rescan_refreshes_changed_comic_cache_without_replacing_identity_or_progress() {
        let root=std::env::temp_dir().join(format!("frayd-comic-refresh-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap(); let path=root.join("book.cbz");
        write_comic_fixture(&path,&[("page2.png",b"second original image"),("page10.png",b"tenth original image")]);
        let original=std::fs::metadata(&path).unwrap(); let app=app_for(path.to_str().unwrap());
        {
            let db=app.db.lock().unwrap();
            db.execute(r#"UPDATE media SET mtime_secs=?1,size_bytes=?2,meta_json=json_set(meta_json,'$.pages',json('["page10.png","page2.png"]')) WHERE id=1"#,
                rusqlite::params![original.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,original.len() as i64]).unwrap();
            assert_eq!(scanner::scan(&db,String::new(),root.clone(),None),0);
        }
        let response=comic_pages(State(app.clone()),Path(1)).await.into_response();
        let unchanged:Vec<String>=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap();
        assert_eq!(unchanged,vec!["page10.png","page2.png"]);
        write_comic_fixture(&path,&[("page2.png",b"second original image"),("page5.png",b"new fifth image"),("page10.png",b"tenth original image")]);
        std::fs::File::open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap())).unwrap();
        {
            let db=app.db.lock().unwrap();
            scanner::scan(&db,String::new(),root.clone(),None);
        }
        let response=comic_pages(State(app.clone()),Path(1)).await.into_response();
        assert_eq!(response.status(),StatusCode::OK);
        let changed:Vec<String>=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap();
        assert_eq!(changed,vec!["page2.png","page5.png","page10.png"]);
        let response=comic_page(State(app.clone()),Path((1,1))).await.into_response();
        assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"new fifth image"[..]);
        let db=app.db.lock().unwrap();
        assert_eq!(db.query_row("SELECT id FROM media WHERE path=?1",[path.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT title FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"Original");
        assert_eq!(db.query_row("SELECT json_extract(meta_json,'$.synopsis') FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"keep");
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Protect cached UTF-8 names, physical page bytes, metadata, and saved progress together.
    #[tokio::test]
    async fn legacy_comic_cache_keeps_listing_content_and_user_state_consistent() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/legacy-utf8-order.cbz");
        let app = app_for(path);
        let response = comic_pages(State(app.clone()),Path(1)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let names: Vec<String> = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(names, vec!["À.png", "Á.png", "Â.png"]);
        let response = comic_page(State(app.clone()),Path((1,1))).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(), &b"second physical page"[..]);
        let db = app.db.lock().unwrap();
        let meta: String = db.query_row("SELECT meta_json FROM media WHERE id=1", [], |r| r.get(0)).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(meta["pages"], serde_json::json!(names));
        assert_eq!(meta["synopsis"], "keep");
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
    }
    /// Catch listing responses that disagree with the page order already used for content.
    #[tokio::test]
    async fn listing_honors_persisted_page_order_used_by_content() {
        let path=concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/legacy-utf8-order.cbz");
        let app=app_for(path);
        app.db.lock().unwrap().execute(r#"UPDATE media SET meta_json=json_set(meta_json,'$.pages',json('["Á.png","À.png","Â.png"]'))"#,[]).unwrap();
        let response=comic_pages(State(app.clone()),Path(1)).await.into_response();
        let names:Vec<String>=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap();
        assert_eq!(names,vec!["Á.png","À.png","Â.png"]);
        let response=comic_page(State(app.clone()),Path((1,1))).await.into_response();
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"first physical page"[..]);
    }
    /// Optional SQLite cache refusal must not hide page bytes already readable from a valid comic.
    #[tokio::test]
    async fn comic_cache_write_failure_keeps_successful_page_response() {
        let path=concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/stored.cbz");let app=app_for(path);
        app.db.lock().unwrap().execute(r#"UPDATE media SET meta_json=json_set(json_remove(meta_json,'$.pages'),'$.custom',json('{"nested":"keep"}')) WHERE id=1"#,[]).unwrap();
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_comic_pages BEFORE UPDATE OF meta_json ON media BEGIN SELECT RAISE(FAIL,'fixture comic cache write refused'); END;").unwrap();
        let response=comic_page(State(app.clone()),Path((1,0))).await.into_response();assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"second image"[..]);
        let response=comic_pages(State(app.clone()),Path(1)).await.into_response();assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Vec<String>>(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap(),vec!["page2.png","page10.png"]);
        let db=app.db.lock().unwrap();assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
    }
    /// A zero-row cache update remains a rejected snapshot, never a readable computed fallback.
    #[tokio::test]
    async fn comic_zero_row_cache_update_does_not_return_computed_names() {
        let path=concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/stored.cbz");let app=app_for(path);
        app.db.lock().unwrap().execute("UPDATE media SET meta_json=json_remove(meta_json,'$.pages') WHERE id=1",[]).unwrap();
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER ignore_comic_pages BEFORE UPDATE OF meta_json ON media BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert_eq!(comic_page_names(&app,1,path).await,None);
        assert_eq!(comic_page(State(app.clone()),Path((1,0))).await.into_response().status(),StatusCode::NOT_FOUND);
        let db=app.db.lock().unwrap();assert_eq!(db.changes(),0);
        assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
    }
    /// Ensure lazy listing publication cannot replace unrelated metadata or user progress.
    #[tokio::test]
    async fn missing_comic_cache_populates_only_cache_fields() {
        let path=concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/legacy-utf8-order.cbz");
        let app=app_for(path);
        app.db.lock().unwrap().execute("UPDATE media SET meta_json=json_remove(meta_json,'$.pages')",[]).unwrap();
        assert_eq!(comic_pages(State(app.clone()),Path(1)).await.into_response().status(),StatusCode::OK);
        let db=app.db.lock().unwrap();
        let meta:String=db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        let meta:serde_json::Value=serde_json::from_str(&meta).unwrap();
        assert_eq!(meta["pages"],serde_json::json!(["À.png","Á.png","Â.png"]));
        assert_eq!(meta["synopsis"],"keep");
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
    }
    /// Exercise the real scan while a separate thread deliberately holds App.db.
    #[tokio::test]
    async fn scan_uses_worker_connection_while_application_database_is_locked() {
        let root = std::env::temp_dir().join(format!("frayd-scan-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("library")).unwrap();
        let path = root.join("db.sqlite");
        drop(store::open(path.to_str().unwrap()));
        let mut app = app_for("unused.cbz");
        Arc::get_mut(&mut app).unwrap().db_path = path.to_string_lossy().into_owned();
        app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        let (locked_tx,locked_rx)=std::sync::mpsc::channel();
        let (release_tx,release_rx)=std::sync::mpsc::channel();
        let held_app=app.clone();
        let holder=std::thread::spawn(move || {
            let _lock=held_app.db.lock().unwrap();
            locked_tx.send(()).unwrap(); release_rx.recv().unwrap();
        });
        locked_rx.recv().unwrap();
        run_scan(app.clone(),vec![root.join("library")]);
        for _ in 0..100 {
            if !app.scanning.load(std::sync::atomic::Ordering::SeqCst) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let still_scanning = app.scanning.load(std::sync::atomic::Ordering::SeqCst);
        release_tx.send(()).unwrap(); holder.join().unwrap();
        assert!(!still_scanning,"scan waited for App.db mutex");
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Ensure failed opening and unwinding both allow a later scan to be admitted.
    #[tokio::test]
    async fn scan_connection_failure_and_unwinding_clear_admission() {
        let mut app = app_for("unused.cbz");
        Arc::get_mut(&mut app).unwrap().db_path = "/nonexistent/frayd/fixture.db".into();
        app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        run_scan(app.clone(),vec![PathBuf::from("/nonexistent")]);
        for _ in 0..100 {
            if !app.scanning.load(std::sync::atomic::Ordering::SeqCst) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(scan_status(State(app.clone())).await.0["scan_error"],"Scan database unavailable");
        app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        *app.scan_error.lock().unwrap()=None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _reset=ScanReset {app:app.clone(),armed:true}; panic!("fixture unwind");
        }));
        assert!(result.is_err());
        assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(scan_status(State(app.clone())).await.0["scan_error"],"Scan failed");
    }
    /// Refused scans publish a path-free error and a later admitted scan clears it.
    #[tokio::test]
    async fn refused_root_scan_reports_failure_then_success_clears_it() {
        let root=std::env::temp_dir().join(format!("frayd-scan-status-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path=root.join("db.sqlite");drop(store::open(path.to_str().unwrap()));
        let library=root.join("library");
        let mut app=app_for("unused.cbz");
        Arc::get_mut(&mut app).unwrap().db_path=path.to_string_lossy().into_owned();
        app.library.write().unwrap().roots=vec![library.clone()];
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
        for _ in 0..100 {if !app.scanning.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        let status=scan_status(State(app.clone())).await.0;
        assert_eq!(status["scanning"],false);
        assert_eq!(status["scan_error"],"Library root unavailable");
        std::fs::create_dir_all(&library).unwrap();
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
        for _ in 0..100 {if !app.scanning.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        assert!(scan_status(State(app.clone())).await.0["scan_error"].is_null());
        std::fs::remove_dir_all(root).unwrap();
    }
    /// A cache inside the scanned root is refused and visible to status consumers.
    #[tokio::test]
    async fn refused_cache_scan_reports_failure() {
        let root=std::env::temp_dir().join(format!("frayd-scan-cache-status-{}",uuid::Uuid::new_v4()));
        let library=root.join("library");std::fs::create_dir_all(&library).unwrap();
        let path=library.join("db.sqlite");drop(store::open(path.to_str().unwrap()));
        let mut app=app_for("unused.cbz");
        Arc::get_mut(&mut app).unwrap().db_path=path.to_string_lossy().into_owned();
        app.library.write().unwrap().roots=vec![library];
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
        for _ in 0..100 {if !app.scanning.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        assert_eq!(scan_status(State(app.clone())).await.0["scan_error"],"Cache boundary unavailable");
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Verify unreadable archives leave metadata intact rather than publishing an empty cache.
    #[tokio::test]
    async fn failed_comic_refresh_preserves_old_metadata() {
        let app = app_for("/does-not-exist.cbz");
        app.db.lock().unwrap().execute(r#"UPDATE media SET meta_json='{"synopsis":"keep"}'"#,[]).unwrap();
        let before: String = app.db.lock().unwrap().query_row("SELECT meta_json FROM media",[],|r|r.get(0)).unwrap();
        assert_eq!(comic_pages(State(app.clone()),Path(1)).await.into_response().status(),StatusCode::NOT_FOUND);
        assert_eq!(comic_page(State(app.clone()),Path((1,0))).await.into_response().status(),StatusCode::NOT_FOUND);
        let after: String = app.db.lock().unwrap().query_row("SELECT meta_json FROM media",[],|r|r.get(0)).unwrap();
        assert_eq!(before,after);
    }
}

#[cfg(test)]
mod ingest_tests {
    use super::*;

    /// Isolate the same server state on both fork-main and PR branches with synthetic admin identities.
    fn fixture_app(root: &StdPath) -> Arc<App> {
        let db=store::open(":memory:");
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'),(2,'device','fixture-reader');
            INSERT INTO user_perms(identity_id,is_admin) VALUES(1,1),(2,0);").unwrap();
        let library=root.join("library with spaces"); std::fs::create_dir_all(&library).unwrap();
        Arc::new(App { db:Mutex::new(db),db_path:String::new(),library:std::sync::RwLock::new(LibraryConfig{roots:vec![library],locked:false}),scan_queue:Mutex::new(ScanQueue::default()),
            scanning:std::sync::atomic::AtomicBool::new(false),scan_error:Mutex::new(None),auth_rl:auth::RateLimiter::new(300,10),
            party:Arc::new(party::Party::new()),ingest_status:Mutex::new(None) })
    }

    /// Exercise the real handler's authorization and argument construction without a network downloader.
    async fn submit(app: &Arc<App>, identity: &str, url: &str) -> StatusCode {
        ingest(State(app.clone()),axum::http::HeaderMap::new(),
            Query(std::collections::HashMap::from([("identity".to_string(),identity.to_string())])),
            Json(IngestReq { url:url.to_string() })).await
    }

    /// Keep HTTP authorization, destination selection and explicit URL framing correct at the process boundary.
    #[tokio::test]
    async fn ingest_preserves_admin_guards_and_literal_downloader_arguments() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(root)=std::env::var_os("FRAYD_TEST_INGEST_ROOT") {
            let root=PathBuf::from(root); let app=fixture_app(&root);
            let calls=root.join("calls"); let argv=root.join("argv");
            assert_eq!(submit(&app,"fixture-reader","https://example.invalid/video").await,StatusCode::FORBIDDEN);
            for url in ["--exec=touch-marker","file:///tmp/source","ftp://example.invalid/video"] {
                assert_eq!(submit(&app,"fixture-admin",url).await,StatusCode::BAD_REQUEST);
            }
            assert!(!calls.exists(),"rejected request launched downloader");
            assert!(!root.join("library with spaces/Internet").exists());
            let marker=root.join("must-not-exist");
            let metacharacters=format!("https://example.invalid/video?q=$(touch {});--exec=ignored&n=1",marker.display());
            for url in ["http://example.invalid/video",metacharacters.as_str()] {
                let _=std::fs::remove_file(&argv);
                assert_eq!(submit(&app,"fixture-admin",url).await,StatusCode::ACCEPTED);
                let mut actual=Vec::new();
                for _ in 0..200 {
                    if let Ok(bytes)=std::fs::read(&argv) {
                        if bytes.ends_with(&[0]) {
                            actual=bytes[..bytes.len()-1].split(|byte|*byte==0)
                                .map(|part|String::from_utf8(part.to_vec()).unwrap()).collect::<Vec<_>>();
                            if actual.last().is_some_and(|last|last==url) { break; }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                let expected_output=format!("{}/library with spaces/Internet/%(title)s.%(ext)s",root.display());
                assert_eq!(actual,vec!["-f","mp4","--no-playlist","-o",expected_output.as_str(),"--",url]);
                assert!(!marker.exists(),"URL metacharacters executed as a shell command");
            }
            assert_eq!(std::fs::read(calls).unwrap(),b"....");
            return;
        }
        let root=std::env::temp_dir().join(format!("frayd-ingest-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let executable=root.join("bin/yt-dlp");
        std::fs::write(&executable,b"#!/bin/sh\nprintf '.' >> \"$FRAYD_TEST_INGEST_CALLS\"\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nprintf '%s\\0' \"$@\" > \"$FRAYD_TEST_INGEST_ARGV\"\nexit 1\n").unwrap();
        std::fs::set_permissions(&executable,std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut paths=vec![root.join("bin")]; paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        let output=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","ingest_tests::ingest_preserves_admin_guards_and_literal_downloader_arguments","--nocapture"])
            .env("FRAYD_TEST_INGEST_ROOT",&root).env("FRAYD_TEST_INGEST_CALLS",root.join("calls"))
            .env("FRAYD_TEST_INGEST_ARGV",root.join("argv")).env("PATH",std::env::join_paths(paths).unwrap())
            .output().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(output.status.success(),"isolated ingestion worker: {}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
    }
}

#[cfg(test)]
mod upstream_integration_tests {
    use super::*;
    /// Create independent two-root data and a disposable database with a synthetic administrator.
    fn fixture() -> (PathBuf, Arc<App>) {
        let root=std::env::temp_dir().join(format!("frayd-roots-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let db=store::open(root.join("db.sqlite").to_str().unwrap());

        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'); INSERT INTO user_perms(identity_id,is_admin) VALUES(1,1);").unwrap();
        let app=Arc::new(App {db:Mutex::new(db),db_path:root.join("db.sqlite").to_string_lossy().into_owned(),library:std::sync::RwLock::new(LibraryConfig::default()),scan_queue:Mutex::new(ScanQueue::default()),scanning:std::sync::atomic::AtomicBool::new(false),scan_error:Mutex::new(None),auth_rl:auth::RateLimiter::new(300,10),party:Arc::new(party::Party::new()),ingest_status:Mutex::new(None)});
        (root,app)
    }
    #[test]
    fn upstream_integration_roots_array_replaces_legacy() {
        let (root,app)=fixture();
        app.db.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES('library_path',?1)",[serde_json::json!(root.join("a")).to_string()]).unwrap();
        *app.library.write().unwrap()=effective_library_config(&app.db.lock().unwrap(),None).unwrap();
        app.db.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES('library_paths',?1)",[serde_json::json!([root.join("b")]).to_string()]).unwrap();
        *app.library.write().unwrap()=effective_library_config(&app.db.lock().unwrap(),None).unwrap();
        assert_eq!(library_roots(&app),vec![root.join("b")]);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Exercise the production settings transaction through the fixture administrator identity.
    async fn setting(app: &Arc<App>, value:serde_json::Value) -> StatusCode {
        set_setting(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())])),Json(value)).await.status()
    }
    #[test]
    fn upstream_integration_roots_override_empty_array_and_aliases() {
        let (root,app)=fixture(); let db=app.db.lock().unwrap();
        db.execute("INSERT INTO settings(key,value) VALUES('library_path',?1)",[serde_json::json!(root.join("a")).to_string()]).unwrap();
        assert_eq!(effective_library_config(&db,Some("")).unwrap().roots,vec![root.join("a")]);
        db.execute("INSERT INTO settings(key,value) VALUES('library_paths','[]')",[]).unwrap();
        assert!(effective_library_config(&db,None).unwrap().roots.is_empty());
        let overridden=effective_library_config(&db,root.join("b").to_str()).unwrap();
        assert_eq!(overridden.roots,vec![root.join("b")]);assert!(overridden.locked);
        #[cfg(unix)] {
            std::os::unix::fs::symlink(root.join("a"),root.join("alias")).unwrap();
            assert_eq!(normalize_roots(&serde_json::json!([root.join("a"),root.join("alias"),root.join("b")])).unwrap(),vec![root.join("a"),root.join("b")]);
        }
        for value in [serde_json::json!("bad"),serde_json::json!([""]),serde_json::json!([root.join("a"),root.join("missing")]),serde_json::json!([root.join("db.sqlite")])] { assert!(normalize_roots(&value).is_err()); }
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn upstream_integration_roots_settings_atomic_failure_and_metadata() {
        let (root,app)=fixture();
        assert_eq!(setting(&app,serde_json::json!({"library_path":root.join("a"),"port":9999})).await,StatusCode::NO_CONTENT);
        assert_eq!(library_roots(&app),vec![root.join("a")]);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM settings WHERE key='library_path'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let before=app.library.read().unwrap().clone();
        for value in [serde_json::json!({"library_paths":[root.join("b"),root.join("missing")],"port":9998}),serde_json::json!({"library_paths":[root.join("b")],"port":0}),serde_json::json!({"library_path":root.join("a"),"library_paths":[]})] {
            assert_eq!(setting(&app,value).await,StatusCode::BAD_REQUEST);assert_eq!(*app.library.read().unwrap(),before);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT value FROM settings WHERE key='port'",[],|r|r.get::<_,String>(0)).unwrap(),"9999");
        }
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_port BEFORE UPDATE ON settings WHEN NEW.key='port' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[root.join("b")],"port":9998})).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(*app.library.read().unwrap(),before);
        assert_eq!(stored_roots(&app.db.lock().unwrap()).unwrap(),before.roots);
        let response=get_settings(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())]))).await.into_response();
        let value:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),65536).await.unwrap()).unwrap();
        assert_eq!(value["library_paths"],serde_json::json!([root.join("a")])); assert_eq!(value["effective_library_paths"],value["library_paths"]);
        assert_eq!(value["library_locked"],false);assert_eq!(value["server_version"],VERSION);
        assert!(value.get("library_path").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn upstream_integration_roots_override_and_active_jobs_refuse_whole_request() {
        let (root,app)=fixture();
        *app.library.write().unwrap()=LibraryConfig{roots:vec![root.join("a")],locked:true};
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[],"port":9998})).await,StatusCode::CONFLICT);
        app.library.write().unwrap().locked=false;
        for status in ["downloading","indexing"] {
            *app.ingest_status.lock().unwrap()=Some(status.into());
            assert_eq!(setting(&app,serde_json::json!({"library_paths":[],"port":9998})).await,StatusCode::CONFLICT);
        }
        *app.ingest_status.lock().unwrap()=None;app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[],"port":9998})).await,StatusCode::CONFLICT);
        app.scanning.store(false,std::sync::atomic::Ordering::SeqCst);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM settings",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn upstream_integration_scan_ingest_queue_indexes_before_done_and_retains_removed_roots() {
        let (root,app)=fixture();
        let roots=vec![root.join("a"),root.join("b")];
        *app.library.write().unwrap()=LibraryConfig{roots:roots.clone(),locked:false};
        std::fs::write(root.join("b/retained.cbz"),b"source").unwrap();
        std::fs::create_dir(root.join("a/Internet")).unwrap();std::fs::write(root.join("a/Internet/download.cbz"),b"download").unwrap();
        // Simulate an admitted busy worker: ingestion must queue its complete snapshot.
        app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        *app.ingest_status.lock().unwrap()=Some("indexing".into());
        queue_ingest_scan(app.clone(),roots.clone());
        assert_eq!(app.scan_queue.lock().unwrap().requests.len(),1);
        let status=scan_status(State(app.clone())).await.0;
        assert_eq!(status,serde_json::json!({"scanning":true,"scan_error":null,"library_configured":true,"library_count":2,"ingesting":"indexing"}));
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        run_scan(app.clone(),roots.clone());
        for _ in 0..400 { if !app.scanning.load(std::sync::atomic::Ordering::SeqCst) {break;} tokio::time::sleep(std::time::Duration::from_millis(5)).await; }
        assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("done"));
        let id:i64=app.db.lock().unwrap().query_row("SELECT id FROM media WHERE path=?1",[root.join("b/retained.cbz").to_str().unwrap()],|r|r.get(0)).unwrap();
        app.db.lock().unwrap().execute("INSERT INTO progress(identity_id,media_id,position) VALUES(1,?1,42)",[id]).unwrap();
        let thumbnail=root.join("thumbs/retained.jpg");std::fs::write(&thumbnail,b"thumbnail").unwrap();
        app.db.lock().unwrap().execute(r#"UPDATE media SET thumb_path=?1,meta_json='{"synopsis":"keep","pages":["one"]}' WHERE id=?2"#,rusqlite::params![thumbnail.to_str().unwrap(),id]).unwrap();
        app.db.lock().unwrap().execute("INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi,note) VALUES(1,?1,'bookmark',0,'fixture','keep')",[id]).unwrap();
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[root.join("a")]})).await,StatusCode::NO_CONTENT);
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
        for _ in 0..400 { if !app.scanning.load(std::sync::atomic::Ordering::SeqCst) {break;} tokio::time::sleep(std::time::Duration::from_millis(5)).await; }
        assert_eq!(app.db.lock().unwrap().query_row("SELECT position FROM progress WHERE media_id=?1",[id],|r|r.get::<_,f64>(0)).unwrap(),42.0);
        assert_eq!(std::fs::read(root.join("b/retained.cbz")).unwrap(),b"source");
        assert_eq!(std::fs::read(&thumbnail).unwrap(),b"thumbnail");
        assert_eq!(app.db.lock().unwrap().query_row("SELECT note FROM annotations WHERE media_id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),"keep");
        assert_eq!(app.db.lock().unwrap().query_row("SELECT json_extract(meta_json,'$.pages[0]') FROM media WHERE id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),"one");
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[]})).await,StatusCode::NO_CONTENT);
        assert!(effective_library_config(&app.db.lock().unwrap(),Some("")).unwrap().roots.is_empty());
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::NOT_IMPLEMENTED);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT position FROM progress WHERE media_id=?1",[id],|r|r.get::<_,f64>(0)).unwrap(),42.0);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn upstream_integration_cache_all_roots_refuse_before_registry_or_directory_mutation() {
        let (root,app)=fixture(); let roots=vec![root.join("a"),root.join("b")];
        let alias=root.join("alias");
        #[cfg(unix)] std::os::unix::fs::symlink(root.join("b"),&alias).unwrap();
        for cache in [root.join("b/nested/missing"),root.clone(),alias.join("nested")] {
            assert!(transcode::validate_cache(&cache,&roots).is_none());
            assert!(!root.join("b/nested").exists());
        }
        assert!(transcode::validate_cache(&root.join("cache/nested"),&roots).is_some());
        assert!(!root.join("cache").exists());
        assert!(transcode::validate_cache(&root.join("cache"),&[]).is_none());
        // Thumbnail overlap with B refuses the entire A pass, before reconciliation.
        std::fs::write(root.join("a/untouched.cbz"),b"source").unwrap();
        let db=app.db.lock().unwrap();
        assert_eq!(scanner::scan_with_roots(&db,app.db_path.clone(),root.join("a"),Some(&root.join("b/thumbs")),&roots),0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(!root.join("b/thumbs").exists());drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn upstream_integration_scan_ingest_blocked_downloader_queues_original_snapshot() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(root)=std::env::var_os("FRAYD_TEST_QUEUE_ROOT") {
            let root=PathBuf::from(root);let (fixture_root,app)=fixture();
            let roots=vec![fixture_root.join("a"),fixture_root.join("b")];
            *app.library.write().unwrap()=LibraryConfig{roots:roots.clone(),locked:false};
            std::fs::write(roots[1].join("blocking.mkv"),b"synthetic").unwrap();
            assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
            for _ in 0..400 {if root.join("probe-ready").exists(){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(root.join("probe-ready").exists());
            let submit=|| ingest(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())])),Json(IngestReq{url:"https://example.invalid/video".into()}));
            assert_eq!(submit().await,StatusCode::ACCEPTED);
            for _ in 0..400 {if root.join("download-ready").exists(){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(root.join("download-ready").exists());assert_eq!(submit().await,StatusCode::CONFLICT);
            assert_eq!(setting(&app,serde_json::json!({"library_paths":[]})).await,StatusCode::CONFLICT);
            std::fs::write(root.join("download-release"),[]).unwrap();
            for _ in 0..400 {if app.ingest_status.lock().unwrap().as_deref()==Some("indexing"){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("indexing"));
            assert_eq!(app.scan_queue.lock().unwrap().requests.front().unwrap().roots,roots);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE path LIKE '%download.cbz'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            std::fs::write(root.join("probe-release"),[]).unwrap();
            for _ in 0..800 {if !app.scanning.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("done"));
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE path LIKE '%download.cbz'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            // Conversion admission holds reconfiguration through the real blocked codec probe.
            for _ in 0..400 {if !transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            std::fs::remove_file(root.join("probe-ready")).unwrap();std::fs::remove_file(root.join("probe-release")).unwrap();
            let movie=roots[1].join("blocking.mkv");
            let id=app.db.lock().unwrap().query_row("SELECT id FROM media WHERE path=?1",[movie.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap();
            transcode::maybe_enqueue_with_roots(app.db_path.clone(),id,movie,roots.clone());
            for _ in 0..400 {if root.join("probe-ready").exists(){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(setting(&app,serde_json::json!({"library_paths":[],"port":9999})).await,StatusCode::CONFLICT);
            std::fs::write(root.join("probe-release"),[]).unwrap();
            for _ in 0..400 {if !transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(!transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst));
            // Failed/unwound scan abandons indexing, clears its queue and admits later scans.
            *app.ingest_status.lock().unwrap()=Some("indexing".into());app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
            queue_ingest_scan(app.clone(),roots.clone());
            drop(ScanReset {app:app.clone(),armed:true});
            assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"));assert!(app.scan_queue.lock().unwrap().requests.is_empty());
            assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
            for _ in 0..800 {if !app.scanning.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            std::fs::remove_dir_all(fixture_root).unwrap();return;
        }
        let root=std::env::temp_dir().join(format!("frayd-queue-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(root.join("bin")).unwrap();
        let downloader=root.join("bin/yt-dlp");let probe=root.join("bin/ffprobe");
        std::fs::write(&downloader,b"#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\ntouch \"$FRAYD_TEST_QUEUE_ROOT/download-ready\"\nwhile [ ! -e \"$FRAYD_TEST_QUEUE_ROOT/download-release\" ]; do sleep 0.01; done\nout=$5\ndest=${out%/*}\nprintf 'fixture' > \"$dest/download.cbz\"\n").unwrap();
        std::fs::write(&probe,b"#!/bin/sh\ntouch \"$FRAYD_TEST_QUEUE_ROOT/probe-ready\"\nwhile [ ! -e \"$FRAYD_TEST_QUEUE_ROOT/probe-release\" ]; do sleep 0.01; done\nexit 1\n").unwrap();
        for path in [&downloader,&probe] {std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o755)).unwrap();}
        let mut paths=vec![root.join("bin")];paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        let result=Command::new(std::env::current_exe().unwrap()).args(["--exact","upstream_integration_tests::upstream_integration_scan_ingest_blocked_downloader_queues_original_snapshot","--nocapture"]).env("FRAYD_TEST_QUEUE_ROOT",&root).env("PATH",std::env::join_paths(paths).unwrap()).output().unwrap();
        std::fs::remove_dir_all(root).unwrap();assert!(result.status.success(),"{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));
    }

    #[tokio::test]
    async fn upstream_integration_scan_ingest_database_rejection_and_unavailable_root_fail() {
        let (root,app)=fixture();let roots=vec![root.join("a")];
        std::fs::create_dir(root.join("a/Internet")).unwrap();std::fs::write(root.join("a/Internet/download.cbz"),b"download").unwrap();
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_media BEFORE INSERT ON media BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        *app.ingest_status.lock().unwrap()=Some("indexing".into());queue_ingest_scan(app.clone(),roots.clone());
        for _ in 0..400 {if !app.scanning.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"));
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        *app.ingest_status.lock().unwrap()=Some("indexing".into());queue_ingest_scan(app.clone(),vec![root.join("missing")]);
        for _ in 0..400 {if !app.scanning.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"));assert!(app.scan_queue.lock().unwrap().requests.is_empty());
        assert_eq!(health().await.0,serde_json::json!({"status":"ok"}));
        assert_eq!(bind_address(None,8484).unwrap().to_string(),"0.0.0.0:8484");
        assert_eq!(bind_address(Some("127.0.0.1"),8484).unwrap().to_string(),"127.0.0.1:8484");
        assert_eq!(bind_address(Some("::1"),8484).unwrap().to_string(),"[::1]:8484");
        assert!(bind_address(Some("localhost"),8484).is_err());assert!(bind_address(Some(""),8484).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    /// Router intent never escapes loopback, while external binds retain either explicit opt-in.
    fn native_loopback_upnp_gate_preserves_external_opt_in() {
        for address in ["127.0.0.1","127.42.0.8","::1","::ffff:127.0.0.1"] {
            let bind=bind_address(Some(address),8484).unwrap();
            for (environment,stored) in [(false,false),(true,false),(false,true),(true,true)] {
                assert!(!upnp_mapping_requested(bind.ip(),environment,stored),"loopback mapping requested: {address}, env={environment}, stored={stored}");
            }
        }
        for address in ["0.0.0.0","192.0.2.1","::","2001:db8::1"] {
            let bind=bind_address(Some(address),8484).unwrap();
            for (environment,stored) in [(false,false),(true,false),(false,true),(true,true)] {
                assert_eq!(upnp_mapping_requested(bind.ip(),environment,stored),environment||stored,"external opt-in changed: {address}");
            }
        }
    }

    #[test]
    fn upstream_integration_roots_scan_admission_reconfiguration_race() {
        for _ in 0..12 {
            let (root,app)=fixture();
            *app.library.write().unwrap()=LibraryConfig{roots:vec![root.join("a")],locked:false};
            let barrier=Arc::new(std::sync::Barrier::new(2));
            let scanning=app.clone();let scan_barrier=barrier.clone();
            let worker=std::thread::spawn(move || {
                let runtime=tokio::runtime::Runtime::new().unwrap();
                scan_barrier.wait();
                runtime.block_on(trigger_scan(State(scanning)))
            });
            let runtime=tokio::runtime::Runtime::new().unwrap();
            barrier.wait();
            let result=runtime.block_on(setting(&app,serde_json::json!({"library_paths":[root.join("b")]})));
            assert!(matches!(result,StatusCode::NO_CONTENT|StatusCode::CONFLICT));
            assert_eq!(worker.join().unwrap(),StatusCode::ACCEPTED);
            let config=app.library.read().unwrap().clone();
            if result==StatusCode::NO_CONTENT { assert_eq!(config.roots,vec![root.join("b")]);assert_eq!(stored_roots(&app.db.lock().unwrap()).unwrap(),config.roots); }
            else {assert_eq!(config.roots,vec![root.join("a")]);}
            assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

}

#[cfg(test)]
mod integration_regressions {
    use super::*;
    /// Keep source bytes, thumbnail ownership and account data separate in a fresh disposable database.
    fn fixture() -> (PathBuf,Arc<App>) {
        let root=std::env::temp_dir().join(format!("frayd-admin-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("library")).unwrap();std::fs::create_dir(root.join("thumbs")).unwrap();
        let db=store::open(root.join("db.sqlite").to_str().unwrap());
        db.execute_batch(r#"INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'),(2,'account','target'),(3,'account','other');
            INSERT INTO user_perms(identity_id,is_admin,allowed_kinds) VALUES(1,1,NULL),(2,0,'["movie"]'),(3,0,NULL);"#).unwrap();
        let app=Arc::new(App{db:Mutex::new(db),db_path:root.join("db.sqlite").to_string_lossy().into_owned(),library:std::sync::RwLock::new(LibraryConfig{roots:vec![root.join("library")],locked:false}),scan_queue:Mutex::new(ScanQueue::default()),scanning:std::sync::atomic::AtomicBool::new(false),scan_error:Mutex::new(None),auth_rl:auth::RateLimiter::new(300,10),party:Arc::new(party::Party::new()),ingest_status:Mutex::new(None)});
        (root,app)
    }
    /// Supply the existing fixture device administrator without creating an account-name fallback.
    fn query() -> Query<std::collections::HashMap<String,String>> {Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())]))}
    /// Exercise the real media deletion handler with the fixture administrator and target ID.
    async fn media(app:&Arc<App>,id:i64)->StatusCode {delete_media(State(app.clone()),axum::http::HeaderMap::new(),query(),Path(id)).await}
    /// Exercise the real account deletion handler through its administrative identity boundary.
    async fn user(app:&Arc<App>,id:i64)->StatusCode {remove_user(State(app.clone()),axum::http::HeaderMap::new(),query(),Path(id)).await}
    /// Exercise validated permission writes through the production transaction path.
    async fn permission(app:&Arc<App>,value:serde_json::Value)->StatusCode {set_perm(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(value)).await}
    /// Bind one indexed source and its distinct configured id.jpg thumbnail for byte-preservation oracles.
    fn seed_media(root:&StdPath,app:&Arc<App>) {
        let source=root.join("library/source.cbz");let thumb=root.join("thumbs/1.jpg");
        std::fs::write(&source,b"original").unwrap();std::fs::write(&thumb,b"thumbnail").unwrap();
        app.db.lock().unwrap().execute("INSERT INTO media(id,kind,path,title,thumb_path) VALUES(1,'comic',?1,'fixture',?2)",rusqlite::params![source.to_str().unwrap(),thumb.to_str().unwrap()]).unwrap();
    }
    #[tokio::test]
    /// Preserve the causal progress-reference regression: conflict refusal must retain both source and thumbnail bytes.
    async fn upstream_integration_admin_referenced_media_refuses_without_losing_thumbnail() {
        let (root,app)=fixture();seed_media(&root,&app);
        app.db.lock().unwrap().execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42);").unwrap();
        assert_eq!(media(&app,1).await,StatusCode::CONFLICT);
        assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");
        assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// Prove ABORT and IGNORE at the final identity delete roll back previously removed children.
    async fn upstream_integration_admin_user_failure_rolls_back_children() {
        let (root,app)=fixture();seed_media(&root,&app);
        app.db.lock().unwrap().execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42);
            CREATE TRIGGER reject_target BEFORE DELETE ON identities WHEN OLD.id=2 BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert_eq!(user(&app,2).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM progress WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM user_perms WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        app.db.lock().unwrap().execute_batch("DROP TRIGGER reject_target;CREATE TRIGGER reject_target BEFORE DELETE ON identities WHEN OLD.id=2 BEGIN SELECT RAISE(IGNORE); END").unwrap();
        assert_eq!(user(&app,2).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM progress WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM user_perms WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// Explicit null must persist SQL NULL rather than silently retaining the previous movie-only restriction.
    async fn upstream_integration_admin_permission_null_clears_restriction() {
        let (root,app)=fixture();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":null})).await,StatusCode::NO_CONTENT);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT allowed_kinds FROM user_perms WHERE identity_id=2",[],|r|r.get::<_,Option<String>>(0)).unwrap(),None);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Compare complete table cardinalities for rollback without serializing session values.
    fn table_counts(app:&Arc<App>) -> Vec<(String,i64)> {
        let db=app.db.lock().unwrap();
        ["identities","sessions","user_perms","progress","annotations","playlists","playlist_items","media","albums","tracks","transcode_artifacts"].into_iter().map(|table|(table.to_string(),db.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap())).collect()
    }
    /// Create two owners with separate sessions/playlists/user data over shared media, tracks and artifacts.
    fn seed_user_children(app:&Arc<App>) {
        let db=app.db.lock().unwrap();
        for id in [2,3] {
            db.execute("INSERT INTO sessions(token,identity_id,expires_at) VALUES(?1,?2,unixepoch()+3600)",rusqlite::params![uuid::Uuid::new_v4().to_string(),id]).unwrap();
        }
        db.execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42),(3,1,73);
            INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi,note) VALUES(2,1,'bookmark',0,'target','target-note'),(3,1,'bookmark',0,'other','other-note');
            INSERT INTO playlists(id,identity_id,title) VALUES(2,2,'target'),(3,3,'other');
            INSERT INTO playlist_items(playlist_id,media_id,pos) VALUES(2,1,0),(3,1,0);
            INSERT INTO albums(id,title) VALUES(1,'shared'); INSERT INTO tracks(media_id,album_id) VALUES(1,1);
            INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance) VALUES('/synthetic-artifact',1,'/synthetic-source','final','generated');").unwrap();
    }
    #[tokio::test]
    /// Seed each independent media reference and require conflict with unchanged table counts and file bytes.
    async fn upstream_integration_admin_all_media_references_refuse_unchanged() {
        for reference in ["progress","annotations","tracks","playlist_items","own_conversion","other_conversion","artifact_owner","artifact_path","artifact_source"] {
            let (root,app)=fixture();seed_media(&root,&app);
            {
                let db=app.db.lock().unwrap();
                match reference {
                    "progress"=>{db.execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42)").unwrap();}
                    "annotations"=>{db.execute_batch("INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi) VALUES(2,1,'bookmark',0,'fixture')").unwrap();}
                    "tracks"=>{db.execute_batch("INSERT INTO tracks(media_id) VALUES(1)").unwrap();}
                    "playlist_items"=>{db.execute_batch("INSERT INTO playlists(id,identity_id,title) VALUES(1,2,'fixture');INSERT INTO playlist_items(playlist_id,media_id,pos) VALUES(1,1,0)").unwrap();}
                    "own_conversion"=>{db.execute("UPDATE media SET transcode_path='/synthetic-cache' WHERE id=1",[]).unwrap();}
                    "other_conversion"=>{db.execute("INSERT INTO media(id,kind,path,title,transcode_path) VALUES(2,'movie','/synthetic-other','other',?1)",[root.join("library/source.cbz").to_str().unwrap()]).unwrap();}
                    "artifact_owner"=>{db.execute("INSERT INTO transcode_artifacts VALUES('/synthetic-output',1,?1,'final','generated','completed')",[root.join("library/source.cbz").to_str().unwrap()]).unwrap();}
                    "artifact_path"|"artifact_source"=>{
                        db.execute_batch("INSERT INTO media(id,kind,path,title) VALUES(2,'movie','/synthetic-other','other')").unwrap();
                        let source=root.join("library/source.cbz").to_string_lossy().into_owned();
                        let (artifact_path,artifact_source)=if reference=="artifact_path" {(source,"/synthetic-other".to_string())}else{("/synthetic-output".into(),source)};
                        db.execute("INSERT INTO transcode_artifacts VALUES(?1,2,?2,'final','generated','completed')",rusqlite::params![artifact_path,artifact_source]).unwrap();
                    }
                    _=>unreachable!(),
                }
            }
            let before=table_counts(&app);assert_eq!(media(&app,1).await,StatusCode::CONFLICT,"reference {reference}");assert_eq!(table_counts(&app),before);
            assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    /// Only a committed unreferenced deletion removes the owned thumbnail; ABORT/IGNORE retain source and cache.
    async fn upstream_integration_admin_media_commit_and_database_failure_preserve_sources() {
        for failure in [false,true] {
            let (root,app)=fixture();seed_media(&root,&app);
            if failure {app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_media BEFORE DELETE ON media BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();}
            assert_eq!(media(&app,1).await,if failure {StatusCode::INTERNAL_SERVER_ERROR}else{StatusCode::NO_CONTENT});
            assert_eq!(root.join("thumbs/1.jpg").exists(),failure);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),if failure {1}else{0});
            assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
            if !failure {assert_eq!(media(&app,1).await,StatusCode::NOT_FOUND);}
            else {
                app.db.lock().unwrap().execute_batch("DROP TRIGGER reject_media;CREATE TRIGGER reject_media BEFORE DELETE ON media BEGIN SELECT RAISE(IGNORE); END").unwrap();
                assert_eq!(media(&app,1).await,StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    /// Unproven/shared/redirected/root-overlapping or scan-active thumbnails survive a successful metadata deletion.
    async fn upstream_integration_admin_thumbnail_refusals_are_postcommit_and_byte_preserving() {
        for refusal in ["shared","alias-shared","artifact-shared","symlink","unowned","missing-saved","library-b","unavailable-root","source","scan-active"] {
            let (root,app)=fixture();seed_media(&root,&app);
            match refusal {
                "shared"|"alias-shared"=>{
                    let mut path=root.join("thumbs/1.jpg");
                    #[cfg(unix)] if refusal=="alias-shared" {std::os::unix::fs::symlink(&path,root.join("alias.jpg")).unwrap();path=root.join("alias.jpg");}
                    app.db.lock().unwrap().execute("INSERT INTO media(id,kind,path,title,thumb_path) VALUES(2,'comic','/synthetic-other','other',?1)",[path.to_str().unwrap()]).unwrap();
                }
                "artifact-shared"=>{
                    let db=app.db.lock().unwrap();db.execute_batch("INSERT INTO media(id,kind,path,title) VALUES(2,'movie','/synthetic-other','other')").unwrap();
                    db.execute("INSERT INTO transcode_artifacts VALUES(?1,2,'/synthetic-other','final','generated','completed')",[root.join("thumbs/1.jpg").to_str().unwrap()]).unwrap();
                }
                "symlink"=>{
                    #[cfg(unix)] {std::fs::remove_file(root.join("thumbs/1.jpg")).unwrap();std::fs::write(root.join("outside.jpg"),b"thumbnail").unwrap();std::os::unix::fs::symlink(root.join("outside.jpg"),root.join("thumbs/1.jpg")).unwrap();}
                }
                "unowned"=>{app.db.lock().unwrap().execute("UPDATE media SET thumb_path=NULL WHERE id=1",[]).unwrap();}
                "missing-saved"=>{app.db.lock().unwrap().execute("UPDATE media SET thumb_path=?1 WHERE id=1",[root.join("missing.jpg").to_str().unwrap()]).unwrap();}
                "library-b"=>app.library.write().unwrap().roots.push(root.join("thumbs")),
                "unavailable-root"=>app.library.write().unwrap().roots.push(root.join("missing-root")),
                "source"=>{app.db.lock().unwrap().execute("UPDATE media SET path=?1 WHERE id=1",[root.join("thumbs/1.jpg").to_str().unwrap()]).unwrap();}
                "scan-active"=>app.scanning.store(true,std::sync::atomic::Ordering::SeqCst),
                _=>unreachable!(),
            }
            assert_eq!(media(&app,1).await,StatusCode::NO_CONTENT,"refusal {refusal}");
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE id=1",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    /// Remove one account including its playlist graph and session, preserving another owner and shared media graph.
    async fn upstream_integration_admin_user_cleanup_targets_only_one_account() {
        let (root,app)=fixture();seed_media(&root,&app);seed_user_children(&app);
        let session=auth::create_session(&app.db.lock().unwrap(),2).unwrap();
        assert_eq!(auth::identity_from_token(&app.db.lock().unwrap(),&session),Some(2));
        assert_eq!(user(&app,2).await,StatusCode::NO_CONTENT);
        assert_eq!(auth::identity_from_token(&app.db.lock().unwrap(),&session),None);
        {
        let db=app.db.lock().unwrap();
        for table in ["sessions","user_perms","progress","annotations","playlists"] {
            assert_eq!(db.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE identity_id=2"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
            assert_eq!(db.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE identity_id=3"),[],|r|r.get::<_,i64>(0)).unwrap(),1);
        }
        assert_eq!(db.query_row("SELECT COUNT(*) FROM playlist_items WHERE playlist_id=3",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        for table in ["media","albums","tracks","transcode_artifacts"] {assert_eq!(db.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),1);}
        assert_eq!(db.query_row("SELECT position FROM progress WHERE identity_id=3",[],|r|r.get::<_,f64>(0)).unwrap(),73.0);
        assert_eq!(db.query_row("SELECT note FROM annotations WHERE identity_id=3",[],|r|r.get::<_,String>(0)).unwrap(),"other-note");        }
        assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");
        assert_eq!(user(&app,2).await,StatusCode::NOT_FOUND);assert_eq!(user(&app,1).await,StatusCode::CONFLICT);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// Check whole-graph rollback, omitted/null/empty semantics, all schema kinds, type refusals and failed writes.
    async fn upstream_integration_admin_user_full_rollback_and_permission_validation() {
        let (root,app)=fixture();seed_media(&root,&app);seed_user_children(&app);
        let before=table_counts(&app);
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_user BEFORE DELETE ON identities WHEN OLD.id=2 BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
        assert_eq!(user(&app,2).await,StatusCode::INTERNAL_SERVER_ERROR);assert_eq!(table_counts(&app),before);
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"is_admin":true})).await,StatusCode::NO_CONTENT);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT allowed_kinds FROM user_perms WHERE identity_id=2",[],|r|r.get::<_,String>(0)).unwrap(),"[\"movie\"]");
        for kinds in [serde_json::json!([]),serde_json::json!(["audiobook","ebook","comic","manga","movie","music","episode"])] {
            assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":kinds})).await,StatusCode::NO_CONTENT);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT allowed_kinds FROM user_perms WHERE identity_id=2",[],|r|r.get::<_,String>(0)).unwrap(),kinds.to_string());
            let listed=library_handler(State(app.clone()),bearer(&app,2),Query(std::collections::HashMap::new())).await.0;
            assert_eq!(listed.len(),if kinds.as_array().unwrap().is_empty() {0}else{1});
        }
        let before=app.db.lock().unwrap().query_row("SELECT is_admin,allowed_kinds FROM user_perms WHERE identity_id=2",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,Option<String>>(1)?))).unwrap();
        for value in [serde_json::json!({"identity_id":2,"is_admin":"true"}),serde_json::json!({"identity_id":2,"is_admin":null}),serde_json::json!({"identity_id":2,"allowed_kinds":["unknown"],"is_admin":false}),serde_json::json!({"identity_id":2,"allowed_kinds":[1]}),serde_json::json!({"identity_id":2,"allowed_kinds":"movie"})] {
            assert_eq!(permission(&app,value).await,StatusCode::BAD_REQUEST);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT is_admin,allowed_kinds FROM user_perms WHERE identity_id=2",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,Option<String>>(1)?))).unwrap(),before);
        }
        app.db.lock().unwrap().execute_batch("UPDATE user_perms SET is_admin=0 WHERE identity_id=2").unwrap();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":1,"is_admin":false,"allowed_kinds":[]})).await,StatusCode::CONFLICT);
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_perm BEFORE UPDATE ON user_perms WHEN NEW.identity_id=2 BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":null})).await,StatusCode::INTERNAL_SERVER_ERROR);
        app.db.lock().unwrap().execute_batch("DROP TRIGGER reject_perm;CREATE TRIGGER reject_perm BEFORE UPDATE ON user_perms WHEN NEW.identity_id=2 BEGIN SELECT RAISE(IGNORE); END").unwrap();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":null})).await,StatusCode::INTERNAL_SERVER_ERROR);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    /// Revoke admin while each real handler waits behind a write lock; no cached pre-transaction authority may survive.
    fn upstream_integration_admin_rechecks_authority_after_database_write_admission() {
        for operation in ["media","user","permission"] {
            let (root,app)=fixture();seed_media(&root,&app);
            let other=store::connect(&app.db_path).unwrap();other.execute_batch("BEGIN IMMEDIATE").unwrap();
            let target=app.clone();let (ready_tx,ready_rx)=std::sync::mpsc::channel();let (done_tx,done_rx)=std::sync::mpsc::channel();
            let worker=std::thread::spawn(move || {
                let runtime=tokio::runtime::Runtime::new().unwrap();ready_tx.send(()).unwrap();
                let result=runtime.block_on(async {match operation {"media"=>media(&target,1).await,"user"=>user(&target,2).await,_=>permission(&target,serde_json::json!({"identity_id":2,"allowed_kinds":null})).await}});
                done_tx.send(result).unwrap();
            });
            ready_rx.recv().unwrap();std::thread::sleep(std::time::Duration::from_millis(30));assert!(done_rx.try_recv().is_err());
            other.execute_batch("UPDATE user_perms SET is_admin=0 WHERE identity_id=1;COMMIT").unwrap();
            assert_eq!(done_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),StatusCode::FORBIDDEN);worker.join().unwrap();
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM identities WHERE id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE id=1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    /// Hold conversion admission while starting deletion, then publish BUSY and require conflict with unchanged bytes.
    fn upstream_integration_admin_converter_admission_refuses_media_delete() {
        if std::env::var_os("FRAYD_TEST_ADMIN_BUSY").is_none() {
            let result=Command::new(std::env::current_exe().unwrap()).args(["--exact","integration_regressions::upstream_integration_admin_converter_admission_refuses_media_delete","--nocapture"]).env("FRAYD_TEST_ADMIN_BUSY","1").output().unwrap();
            assert!(result.status.success(),"{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));return;
        }
        let (root,app)=fixture();seed_media(&root,&app);let admission=ROOT_ADMISSION.lock().unwrap();
        let target=app.clone();let (ready_tx,ready_rx)=std::sync::mpsc::channel();let (done_tx,done_rx)=std::sync::mpsc::channel();
        let worker=std::thread::spawn(move || {let runtime=tokio::runtime::Runtime::new().unwrap();ready_tx.send(()).unwrap();done_tx.send(runtime.block_on(media(&target,1))).unwrap();});
        ready_rx.recv().unwrap();std::thread::sleep(std::time::Duration::from_millis(30));assert!(done_rx.try_recv().is_err());
        transcode::BUSY.store(true,std::sync::atomic::Ordering::SeqCst);drop(admission);
        assert_eq!(done_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),StatusCode::CONFLICT);worker.join().unwrap();
        assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Decode the creation status/code/proof in memory for admission tests; do not record private proof values.
    async fn created_party(app:&Arc<App>,headers:axum::http::HeaderMap,identity:&str) -> (StatusCode,serde_json::Value) {
        let response=party_create(State(app.clone()),headers,Json(PartyCreateReq{identity:identity.into()})).await;
        let status=response.status();let bytes=axum::body::to_bytes(response.into_body(),65536).await.unwrap();
        (status,serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }
    /// Create a disposable account session and return its header without logging or persisting credentials.
    fn bearer(app:&Arc<App>,id:i64) -> axum::http::HeaderMap {
        let token=auth::create_session(&app.db.lock().unwrap(),id).unwrap();
        let mut headers=axum::http::HeaderMap::new();headers.insert(header::AUTHORIZATION,format!("Bearer {token}").parse().unwrap());headers
    }
    #[tokio::test]
    /// The creator key matches socket token precedence; only the current generation private proof admits that host.
    async fn upstream_integration_identity_party_bearer_overrides_body_and_requires_current_proof() {
        let (root,app)=fixture();let headers=bearer(&app,2);
        let (status,created)=created_party(&app,headers.clone(),"different-device-body").await;assert_eq!(status,StatusCode::OK);
        let key=party_identity_key(&app.db.lock().unwrap(),&headers,Some("different-device-query")).unwrap();assert_eq!(key,"target");
        let code=created["code"].as_str().unwrap();let proof=uuid::Uuid::parse_str(created["host_capability"].as_str().unwrap()).unwrap();
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),&key));
        assert!(!app.party.can_register(&app.party.authorize(code,None).unwrap(),&key));
        assert!(!app.party.can_register(&app.party.authorize(code,Some(uuid::Uuid::new_v4())).unwrap(),&key));
        let old_admission=app.party.authorize(code,Some(proof)).unwrap();
        let (_,replacement)=created_party(&app,headers.clone(),"different-device-body").await;
        assert!(!app.party.can_register(&old_admission,&key));
        let replacement_code=replacement["code"].as_str().unwrap();
        assert!(!app.party.can_register(&app.party.authorize(replacement_code,Some(proof)).unwrap(),&key));
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM identities WHERE key='different-device-body'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// Public device creation remains compatible; copied account keys and failed device inserts cannot replace a room.
    async fn upstream_integration_identity_party_device_compatibility_and_creation_denial_preserve_room() {
        let (root,app)=fixture();let (status,created)=created_party(&app,axum::http::HeaderMap::new(),"new-device").await;assert_eq!(status,StatusCode::OK);
        let code=created["code"].as_str().unwrap();let proof=uuid::Uuid::parse_str(created["host_capability"].as_str().unwrap()).unwrap();
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),"new-device"));
        assert!(!app.party.can_register(&app.party.authorize(code,None).unwrap(),"new-device"));
        assert_eq!(created_party(&app,axum::http::HeaderMap::new(),"target").await.0,StatusCode::UNAUTHORIZED);
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),"new-device"));
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_identity BEFORE INSERT ON identities BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
        assert_eq!(created_party(&app,axum::http::HeaderMap::new(),"blocked-device").await.0,StatusCode::INTERNAL_SERVER_ERROR);
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),"new-device"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    /// Distinguish denied/missing/self-or-last-admin/database errors without committing metadata or filesystem changes.
    async fn upstream_integration_admin_denials_last_admin_missing_and_query_failures() {
        let (root,app)=fixture();seed_media(&root,&app);
        let empty=||Query(std::collections::HashMap::new());
        assert_eq!(delete_media(State(app.clone()),axum::http::HeaderMap::new(),empty(),Path(1)).await,StatusCode::FORBIDDEN);
        assert_eq!(remove_user(State(app.clone()),axum::http::HeaderMap::new(),empty(),Path(2)).await,StatusCode::FORBIDDEN);
        assert_eq!(set_perm(State(app.clone()),axum::http::HeaderMap::new(),empty(),Json(serde_json::json!({"identity_id":2}))).await,StatusCode::FORBIDDEN);
        assert_eq!(media(&app,999).await,StatusCode::NOT_FOUND);assert_eq!(user(&app,999).await,StatusCode::NOT_FOUND);
        assert_eq!(permission(&app,serde_json::json!({"identity_id":999,"allowed_kinds":[]})).await,StatusCode::NOT_FOUND);
        assert_eq!(permission(&app,serde_json::json!({"identity_id":1,"is_admin":false})).await,StatusCode::CONFLICT);
        app.db.lock().unwrap().execute_batch("UPDATE identities SET kind='account' WHERE id=1").unwrap();
        let headers=bearer(&app,1);
        assert_eq!(remove_user(State(app.clone()),headers.clone(),empty(),Path(1)).await,StatusCode::CONFLICT);
        assert_eq!(set_perm(State(app.clone()),headers.clone(),empty(),Json(serde_json::json!({"identity_id":1,"is_admin":false}))).await,StatusCode::CONFLICT);
        app.db.lock().unwrap().execute_batch("DROP TABLE user_perms").unwrap();
        assert_eq!(delete_media(State(app.clone()),headers.clone(),empty(),Path(1)).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(remove_user(State(app.clone()),headers.clone(),empty(),Path(2)).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(set_perm(State(app.clone()),headers,empty(),Json(serde_json::json!({"identity_id":2,"allowed_kinds":null}))).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(std::fs::read(root.join("thumbs/1.jpg")).unwrap(),b"thumbnail");assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// A token pointing at a missing canonical key fails 500 rather than creating an anonymous host or replacing a room.
    async fn upstream_integration_identity_party_orphan_token_key_failure_preserves_room() {
        let (root,app)=fixture();let (_,created)=created_party(&app,axum::http::HeaderMap::new(),"device").await;
        let code=created["code"].as_str().unwrap();let proof=uuid::Uuid::parse_str(created["host_capability"].as_str().unwrap()).unwrap();
        let token=uuid::Uuid::new_v4().to_string();
        {
            let db=app.db.lock().unwrap();db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
            db.execute("INSERT INTO sessions(token,identity_id,expires_at) VALUES(?1,999,unixepoch()+3600)",[&token]).unwrap();
            db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        }
        let mut headers=axum::http::HeaderMap::new();headers.insert(header::AUTHORIZATION,format!("Bearer {token}").parse().unwrap());
        assert_eq!(party_identity_key(&app.db.lock().unwrap(),&headers,Some("device")),Err(StatusCode::INTERNAL_SERVER_ERROR));
        assert_eq!(created_party(&app,headers,"device").await.0,StatusCode::INTERNAL_SERVER_ERROR);
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),"device"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    /// An unavailable absolute stored/env root retains configuration while new writes and malformed values stay rejected.
    fn upstream_integration_roots_persisted_unavailable_paths_are_retained_but_writes_remain_strict() {
        let (root,app)=fixture();let missing=root.join("temporarily-offline");
        let db=app.db.lock().unwrap();db.execute("INSERT INTO settings(key,value) VALUES('library_paths',?1)",[serde_json::json!([missing]).to_string()]).unwrap();
        let stored=effective_library_config(&db,Some("")).unwrap();assert_eq!(stored.roots,vec![missing.clone()]);assert!(!stored.locked);
        let locked=effective_library_config(&db,missing.to_str()).unwrap();assert_eq!(locked.roots,vec![missing.clone()]);assert!(locked.locked);
        assert!(normalize_roots(&serde_json::json!([missing])).is_err());assert!(!missing.exists());
        for value in [serde_json::json!("bad"),serde_json::json!([""]),serde_json::json!([root.join("db.sqlite")]),serde_json::json!(["unresolvable-relative-root"])] {assert!(parse_roots(&value,true).is_err());}
        std::fs::create_dir(&missing).unwrap();assert_eq!(effective_library_config(&db,None).unwrap(),stored);
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os="linux")]
    #[test]
    /// Use an actually unprivileged child and mode000 parent to prove PermissionDenied retains configured roots only on reads.
    fn upstream_integration_roots_persisted_permission_denied_path_stays_configured() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        if std::env::var_os("FRAYD_TEST_ROOT_DAC").is_none() {
            let status=std::fs::read_to_string("/proc/self/status").unwrap();
            let effective_uid:u32=status.lines().find(|line|line.starts_with("Uid:")).unwrap().split_whitespace().nth(2).unwrap().parse().unwrap();
            let mut child=Command::new(std::env::current_exe().unwrap());child.args(["--exact","integration_regressions::upstream_integration_roots_persisted_permission_denied_path_stays_configured","--nocapture"]).env("FRAYD_TEST_ROOT_DAC","1");
            if effective_uid==0 {child.gid(65534).uid(65534);}
            let result=child.output().unwrap();assert!(result.status.success(),"{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));return;
        }
        struct RestoreDirectory(PathBuf);
        impl Drop for RestoreDirectory {fn drop(&mut self){let _=std::fs::set_permissions(&self.0,std::fs::Permissions::from_mode(0o700));}}
        let (root,app)=fixture();let parent=root.join("protected");let library=parent.join("library");std::fs::create_dir_all(&library).unwrap();
        let db=app.db.lock().unwrap();db.execute("INSERT INTO settings(key,value) VALUES('library_paths',?1)",[serde_json::json!([library]).to_string()]).unwrap();
        let restore=RestoreDirectory(parent.clone());std::fs::set_permissions(&parent,std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(std::fs::read_dir(&library).unwrap_err().kind(),std::io::ErrorKind::PermissionDenied);
        assert_eq!(effective_library_config(&db,None).unwrap().roots,vec![library.clone()]);
        assert_eq!(effective_library_config(&db,library.to_str()).unwrap().roots,vec![library.clone()]);
        assert!(normalize_roots(&serde_json::json!([library])).is_err());
        drop(restore);drop(db);std::fs::remove_dir_all(root).unwrap();
    }

}
