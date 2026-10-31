mod auth;
mod build_info;
mod transcode;
mod comic;
mod epub;
mod party;
mod scanner;
mod store;

use axum::{
    extract::ws::WebSocketUpgrade,
    extract::{ConnectInfo, Path, Query, State},
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

pub(crate) static ROOT_ADMISSION: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// The committed effective roots and override lock state, published together under App.library.
pub struct LibraryConfig { roots: Vec<PathBuf>, locked: bool }

/// One scan with its complete root snapshot and whether it completes a pending ingest.
pub struct ScanRequest { roots:Vec<PathBuf>, for_ingest:bool }

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum DownloadBackend {Direct,YtDlp}
impl DownloadBackend {
    fn name(self) -> &'static str {match self {Self::Direct=>"direct",Self::YtDlp=>"yt-dlp"}}
}

fn download_backend_from_override(value:Option<&std::ffi::OsStr>) -> Option<DownloadBackend> {
    match value {
        None=>Some(DownloadBackend::YtDlp),
        Some(value) if value=="direct"=>Some(DownloadBackend::Direct),
        Some(value) if value=="yt-dlp"=>Some(DownloadBackend::YtDlp),
        Some(_)=>None,
    }
}

pub struct App {
    pub db: Mutex<Connection>,
    /// Path to the SQLite file (scan and transcode workers open their own connections).
    pub db_path: String,
    /// Effective library roots: FRAYD_LIBRARY when set, else the stored setting. Settings writes replace it live.
    pub library: std::sync::RwLock<LibraryConfig>,
    /// Scans waiting for the admitted worker, which drains this queue before clearing `scanning`.
    pub scan_queue: Mutex<std::collections::VecDeque<ScanRequest>>,
    /// True while a scan is running (single scan at a time).
    pub scanning: std::sync::atomic::AtomicBool,
    /// Path-free failure for the most recently admitted scan.
    pub scan_error: Mutex<Option<&'static str>>,
    /// Login/setup/password attempts: 10 per 5 min per IP.
    pub auth_rl: auth::RateLimiter,
    /// FRAYD_TRUSTED_PROXY=1, read once at startup: key `auth_rl` by the proxy's X-Forwarded-For instead of the TCP peer.
    pub trust_proxy: bool,
    /// Immutable runtime download capability; absent override preserves native behavior.
    pub downloads_enabled: bool,
    download_backend: Option<DownloadBackend>,
    /// Active listening party (single party — household scale).
    pub party: Arc<party::Party>,
    /// yt-dlp ingest state: "downloading" | "indexing" | "done" | "failed" | None. Single ingest at a time.
    pub ingest_status: std::sync::Mutex<Option<String>>,
}

impl App {
    /// Idle application state over an opened database, with no scan, party or ingest in progress.
    fn new(db: Connection, db_path: String, library: LibraryConfig) -> Self {
        let download_backend=download_backend_from_override(std::env::var_os("FRAYD_DOWNLOAD_BACKEND").as_deref());
        let downloads_enabled=downloads_enabled_from_override(std::env::var_os("FRAYD_DOWNLOADS_ENABLED").as_deref()) && download_backend.is_some();
        App {
            db: Mutex::new(db),
            db_path,
            library: std::sync::RwLock::new(library),
            scan_queue: Mutex::new(std::collections::VecDeque::new()),
            scanning: std::sync::atomic::AtomicBool::new(false),
            scan_error: Mutex::new(None),
            auth_rl: auth::RateLimiter::new(300, 10),
            trust_proxy: false,
            downloads_enabled,
            download_backend,
            party: Arc::new(party::Party::new()),
            ingest_status: Mutex::new(None),
        }
    }
}

/// Only exact `1` enables an explicitly configured capability; absent keeps native compatibility.
fn downloads_enabled_from_override(value:Option<&std::ffi::OsStr>) -> bool {
    match value {None=>true,Some(value)=>value=="1"}
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
    if std::env::args().nth(1).as_deref() == Some("--build-info") {
        println!("{}", build_info::metadata());
        return;
    }
    tracing_subscriber::fmt::init();

    // default paths relative to this source dir so cwd doesn't matter (env overrides)
    let default_db = concat!(env!("CARGO_MANIFEST_DIR"), "/frayd.db");
    let db_path = std::env::var("FRAYD_DB").unwrap_or_else(|_| default_db.into());
    let db = store::open(&db_path);
    let library=environment_library_config(&db,std::env::var_os("FRAYD_LIBRARY").as_deref());

    let mut app_state = App::new(db, db_path, library.clone());
    app_state.trust_proxy = std::env::var("FRAYD_TRUSTED_PROXY").map(|v| v == "1").unwrap_or(false);
    let state = std::sync::Arc::new(app_state);

    // optional immediate scan
    if !library.roots.is_empty() {
        state
            .scanning
            .store(true, std::sync::atomic::Ordering::SeqCst);
        run_scan(state.clone(), ScanRequest { roots: library.roots, for_ingest: false });
    }

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/build", get(build_info::handler))
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
        .route("/api/scan", post(trigger_scan))
        .route("/api/ingest", post(ingest))
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
    tracing::info!("frayd listening on {bind}");

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
        axum_server::bind_rustls(bind, tls)
            .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
            .unwrap();
    } else {
        let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
        axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
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
    artwork_version: Option<String>,
}

#[derive(serde::Deserialize)]
struct LoginReq {
    email: String,
    password: String,
}

/// Household account login: create an Argon2id password hash on first login,
/// verify the stored hash on subsequent logins, and issue a session token.
async fn auth_login(
    State(app): State<std::sync::Arc<App>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(req): Json<LoginReq>,
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || auth_login_blocking(app,peer,headers,req).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn auth_login_blocking(app:Arc<App>,peer:std::net::SocketAddr,headers:axum::http::HeaderMap,req:LoginReq) -> impl IntoResponse {
    if !app.auth_rl.check(&auth::client_ip(&headers, peer.ip(), app.trust_proxy)) {
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
    if let Err(e) = db.execute(
        "INSERT OR IGNORE INTO user_perms (identity_id, is_admin)
         SELECT ?1, 1 WHERE NOT EXISTS (SELECT 1 FROM user_perms WHERE is_admin = 1)",
        [identity_id],
    ) {
        tracing::warn!("bootstrap admin grant failed: {e}");
    }

    // Read the admin flag before minting the session so a failed lookup leaves no unreturned token.
    let is_admin = match db
        .query_row(
            "SELECT is_admin FROM user_perms WHERE identity_id = ?1",
            [identity_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
    {
        Ok(is_admin) => is_admin.unwrap_or(0),
        Err(e) => {
            tracing::warn!("admin flag lookup failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "session creation failed" })),
            );
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
    tokio::task::spawn_blocking(move || auth_logout_blocking(app,headers)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn auth_logout_blocking(app:Arc<App>,headers:axum::http::HeaderMap) -> StatusCode {
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
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChangePwReq>,
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || auth_change_password_blocking(app,peer,headers,req).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn auth_change_password_blocking(app:Arc<App>,peer:std::net::SocketAddr,headers:axum::http::HeaderMap,req:ChangePwReq) -> impl IntoResponse {
    if !app.auth_rl.check(&format!("pw:{}", auth::client_ip(&headers, peer.ip(), app.trust_proxy))) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error": "too many attempts — wait 5 minutes"}))).into_response();
    }
    if req.new.len() < 4 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "new password too short (min 4)"}))).into_response();
    }
    let db = app.db.lock().unwrap();
    let Some(tok) = bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let iid = match auth::identity_from_token(&db, &tok) {
        Ok(Some(id)) => id,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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
async fn setup_status(State(app): State<std::sync::Arc<App>>) -> axum::response::Response {
    tokio::task::spawn_blocking(move || setup_status_blocking(app).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn setup_status_blocking(app:Arc<App>) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    let Ok(have) = db.query_row("SELECT COUNT(*) FROM identities WHERE kind='account'", [], |r| r.get::<_, i64>(0)) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    Json(serde_json::json!({ "needed": have == 0 })).into_response()
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
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SetupReq>,
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || auth_setup_blocking(app,peer,headers,req).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn auth_setup_blocking(app:Arc<App>,peer:std::net::SocketAddr,headers:axum::http::HeaderMap,req:SetupReq) -> impl IntoResponse {
    if !app.auth_rl.check(&auth::client_ip(&headers, peer.ip(), app.trust_proxy)) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({"error": "too many attempts — wait 5 minutes"}))).into_response();
    }
    if req.password.len() < 4 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "password too short (min 4)"}))).into_response();
    }
    let mut db = app.db.lock().unwrap();
    // Count, create, grant and mint the session in one write transaction. Any failure rolls the account
    // back, so no owner without the admin grant is left for a later login to claim bootstrap admin.
    let Ok(tx) = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(have) = tx.query_row("SELECT COUNT(*) FROM identities WHERE kind='account'", [], |r| r.get::<_, i64>(0)) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if have > 0 {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error": "setup already done — log in instead"}))).into_response();
    }
    if let Err(e) = auth::create_account(&tx, &req.email, &req.password) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e}))).into_response();
    }
    let Ok(iid) = tx.query_row("SELECT id FROM identities WHERE kind='account' AND key = ?1", [&req.email], |r| r.get::<_, i64>(0)) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if let Err(e) = tx.execute(
        "INSERT INTO user_perms (identity_id, is_admin) VALUES (?1, 1)
         ON CONFLICT(identity_id) DO UPDATE SET is_admin = 1",
        rusqlite::params![iid],
    ) {
        tracing::warn!("owner admin grant failed: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "owner admin grant failed"}))).into_response();
    }
    let token = match auth::create_session(&tx, iid) {
        Ok(token) => token,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e}))).into_response(),
    };
    if let Err(e) = tx.commit() {
        tracing::warn!("owner setup commit failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let name = req.email.split('@').next().unwrap_or("owner").to_string();
    (StatusCode::OK, Json(serde_json::json!({"token": token, "name": name, "is_admin": true}))).into_response()
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
        if let Some(id) = auth::identity_from_token(db, &token).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? {
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

/// Resolve a cache directory: the environment override when set, else `name` beside this application's DB.
fn cache_dir_for(db_path: &str, override_var: &str, name: &str) -> PathBuf {
    if let Some(path)=std::env::var_os(override_var) { return PathBuf::from(path); }
    StdPath::new(db_path)
        .parent()
        .unwrap_or(StdPath::new("."))
        .join(name)
}

fn thumbs_dir_for(db_path: &str) -> PathBuf { cache_dir_for(db_path, "FRAYD_THUMBS", "thumbs") }

pub(crate) fn transcodes_dir_for(db_path: &str) -> PathBuf { cache_dir_for(db_path, "FRAYD_TRANSCODES", "transcodes") }

fn thumbnail_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { "image/png" }
    else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") { "image/gif" }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { "image/webp" }
    else if bytes.starts_with(&[0xff,0xd8,0xff]) { "image/jpeg" }
    else { "application/octet-stream" }
}

/// Serve extracted thumbnail. 204 when none (client falls back gracefully).
async fn thumb(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let path=match tokio::task::spawn_blocking(move ||thumbnail_path_blocking(app,id)).await {
        Ok(path)=>path,
        Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Some(path) = path else { return StatusCode::NO_CONTENT.into_response() };
    use tokio::io::{AsyncReadExt,AsyncSeekExt};
    let Ok(mut file)=tokio::fs::File::open(&path).await else {return StatusCode::NO_CONTENT.into_response();};
    let Ok(metadata)=file.metadata().await else {return StatusCode::NO_CONTENT.into_response();};
    if !metadata.is_file() {return StatusCode::NO_CONTENT.into_response();}
    let mut signature=Vec::new();
    if (&mut file).take(12).read_to_end(&mut signature).await.is_err()
        || file.seek(std::io::SeekFrom::Start(0)).await.is_err() {return StatusCode::NO_CONTENT.into_response();}
    let stream=tokio_util::io::ReaderStream::with_capacity(file,64*1024);
    ([(header::CONTENT_TYPE,thumbnail_mime(&signature)),(header::CACHE_CONTROL,"no-cache"),
      (header::CONTENT_LENGTH,&metadata.len().to_string())],axum::body::Body::from_stream(stream)).into_response()
}

fn thumbnail_path_blocking(app:Arc<App>,id:i64) -> Option<String> {
    let db=app.db.lock().unwrap();
    db.query_row("SELECT thumb_path FROM media WHERE id=?1",[id],|r|r.get(0)).ok()
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
    let Some(names) = comic_page_names(&app, id, &path).await else { return StatusCode::NOT_FOUND.into_response() };
    let Some(index) = usize::try_from(n).ok().filter(|&index| index < names.len()) else { return StatusCode::NOT_FOUND.into_response() };
    let mime = mime_guess(&names[index]);
    match tokio::task::spawn_blocking(move || {
        comic::read_cached_page(std::path::Path::new(&path), &names, index)
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

/// Comic page names in the order shared by the listing and page endpoints.
async fn comic_page_names(app: &Arc<App>, id: i64, path: &str) -> Option<Vec<String>> {
    cached_names(app, id, path, "pages", |archive| {
        comic::comic_pages(archive).map(|pages| pages.into_iter().map(|(name,_)| name).collect())
    }).await
}

/// EPUB chapter names in the order shared by the spine and chunk endpoints.
async fn epub_spine_names(app: &Arc<App>, id: i64, path: &str) -> Option<Vec<String>> {
    cached_names(app, id, path, "spine", epub::epub_spine).await
}

/// Return the archive listing cached in `meta_json.<field>`, or list the archive outside the DB lock
/// and cache the result only while the row still matches the fingerprint read before listing.
async fn cached_names(app: &Arc<App>, id: i64, path: &str, field: &'static str,
    list: fn(&StdPath) -> Option<Vec<String>>) -> Option<Vec<String>> {
    let app=app.clone();let path=path.to_string();
    tokio::task::spawn_blocking(move || cached_names_blocking(app,id,path,field,list)).await.ok().flatten()
}

/// Query and publish under separate DB locks, with archive listing between them on the blocking pool.
fn cached_names_blocking(app:Arc<App>,id:i64,path:String,field:&'static str,
    list:fn(&StdPath)->Option<Vec<String>>) -> Option<Vec<String>> {
    let version = {
        let db = app.db.lock().unwrap();
        let (meta,mtime,size): (Option<String>,Option<i64>,Option<i64>) = db.query_row(
            "SELECT meta_json,mtime_secs,size_bytes FROM media WHERE id=?1",[id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).ok()?;
        let cached = meta.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| serde_json::from_value::<Vec<String>>(v[field].clone()).ok());
        if let Some(names) = cached { return Some(names); }
        (mtime,size)
    };
    let archive = PathBuf::from(&path);
    let names = list(&archive)?;
    let db = app.db.lock().unwrap();
    publish_names(&db, id, &path, field, version, names)
}

/// Merge only `meta_json.<field>`, and only for the unchanged source version. A changed row returns
/// None so no caller serves a stale listing; a failed cache write still returns the computed names.
fn publish_names(db: &Connection, id: i64, path: &str, field: &str,
    version: (Option<i64>,Option<i64>), names: Vec<String>) -> Option<Vec<String>> {
    match db.execute("UPDATE media SET meta_json=json_set(COALESCE(meta_json,'{}'),'$.'||?6,json(?1))
        WHERE id=?2 AND path=?3 AND mtime_secs IS ?4 AND size_bytes IS ?5",
        rusqlite::params![serde_json::json!(&names).to_string(),id,path,version.0,version.1,field]) {
        Ok(1)=>Some(names),
        Ok(_)=>None,
        Err(error)=>{tracing::warn!(%error,id,field,"archive listing cache write held");Some(names)}
    }
}

/// Clear scan admission and abandon queued ingest on errors or unwinding.
struct ScanReset { app:Arc<App>, armed:bool }
impl Drop for ScanReset {
    /// Release admission atomically with queue abandonment unless normal drainage disarmed this guard.
    fn drop(&mut self) {
        if !self.armed { return; }
        let mut queue=self.app.scan_queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.clear();
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
            Err(error)
                if retain_unavailable
                    && normal_absolute
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                    ) =>
            {
                tracing::warn!(operation="configure",path=%path.display(),%error,"configured library \
                    unavailable; retaining path without filesystem effects");
                path
            }
            Err(error)=>return Err(error.to_string()),
        };
        match std::fs::metadata(&root) {
            Ok(metadata) if metadata.is_dir()=>(),
            Ok(_)=>return Err("library path must be a directory".into()),
            Err(error)
                if retain_unavailable
                    && normal_absolute
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                    ) => {}
            Err(error) => return Err(error.to_string()),
        }
        if !roots.contains(&root) {roots.push(root);}
    }
    Ok(roots)
}

/// Read authoritative library_paths, including [], falling back to legacy only when the array key is absent.
/// Typed unavailable absolute roots retain configuration without acquiring filesystem-write authority.
fn stored_roots(db: &Connection) -> Result<Vec<PathBuf>,String> {
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

/// Return only an editable string array; malformed legacy values stay in the DB until a valid save.
fn stored_root_strings(db:&Connection) -> rusqlite::Result<Vec<String>> {
    let raw:Option<String>=db.query_row("SELECT value FROM settings WHERE key='library_paths'",[],|r|r.get(0)).optional()?;
    let value=if let Some(raw)=raw { serde_json::from_str::<serde_json::Value>(&raw).ok() } else {
        let legacy:Option<String>=db.query_row("SELECT value FROM settings WHERE key='library_path'",[],|r|r.get(0)).optional()?;
        legacy.and_then(|raw|serde_json::from_str::<serde_json::Value>(&raw).ok()).map(|v| {
            if v.is_null() || v=="" {serde_json::json!([])} else {serde_json::json!([v])}
        })
    };
    let value=value.unwrap_or_else(||serde_json::json!([]));
    let Some(array)=value.as_array() else {return Ok(Vec::new());};
    Ok(array.iter().map(|entry|entry.as_str().map(str::to_string)).collect::<Option<Vec<_>>>().unwrap_or_default())
}

/// Read an explicit environment value without allowing malformed UTF-8 to select saved roots.
fn environment_library_config(db:&Connection,value:Option<&std::ffi::OsStr>) -> LibraryConfig {
    let value=match value {
        Some(value)=>match value.to_str() {Some(value)=>Some(value),None=>return explicit_library_config(Err("Non-UTF-8 override".into()))},
        None=>None,
    };
    effective_library_config(db,value).unwrap_or_else(|error|explicit_library_config(Err(error)))
}

/// An invalid explicit override remains exclusive and grants no source effects or settings unlock.
fn explicit_library_config(roots:Result<Vec<PathBuf>,String>) -> LibraryConfig {
    if let Ok(roots)=roots {return LibraryConfig {roots,locked:true};}
    tracing::warn!("FRAYD_LIBRARY configuration unavailable; library held, saved roots are not used; correct the environment and restart");
    LibraryConfig {roots:Vec::new(),locked:true}
}

/// Select one exclusive locked nonempty environment root or the unlocked persisted root array.
/// Empty environment values enable persisted configuration and never add an implicit fallback root.
fn effective_library_config(db: &Connection, env_override: Option<&str>) -> Result<LibraryConfig,String> {
    if let Some(path)=env_override.filter(|s| !s.is_empty()) {
        return Ok(explicit_library_config(parse_roots(&serde_json::json!([path]),true)));
    }
    let roots=stored_roots(db).unwrap_or_else(|error| {
        tracing::warn!(%error,"stored library configuration held; repair through settings");
        Vec::new()
    });
    Ok(LibraryConfig { roots,locked:false })
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

/// Snapshot of the committed effective library roots.
fn library_roots(app: &App) -> Vec<PathBuf> { app.library.read().unwrap().roots.clone() }

/// Scan `first` on a dedicated connection, then drain queued requests before releasing admission.
/// A root unavailable when a pass starts, or any refused cache, stops the worker; ScanReset then
/// abandons queued and indexing work.
fn run_scan(state: Arc<App>, first: ScanRequest) {
    tokio::task::spawn_blocking(move || {
        let mut reset=ScanReset {app:state.clone(),armed:true};
        let db=match store::connect(&state.db_path) {
            Ok(db)=>db,
            Err(error) => {
                tracing::error!(%error,"scan database unavailable");
                *state.scan_error.lock().unwrap() = Some("Scan database unavailable");
                return;
            }
        };
        let thumbs=thumbs_dir_for(&state.db_path);
        let mut request=first;
        loop {
            let ScanRequest{roots,for_ingest}=request;
            *state.scan_error.lock().unwrap()=None;
            let n=match scanner::scan_roots(&db,state.db_path.clone(),&roots,Some(&thumbs)) {
                Ok(count)=>count,
                Err(error)=>{*state.scan_error.lock().unwrap()=Some(error.reason());return;}
            };
            if for_ingest {
                let indexed=scanner::ingest_indexed(&db,&roots[0].join("Internet"));
                *state.ingest_status.lock().unwrap()=Some(if indexed {"done"} else {"failed"}.into());
            }
            tracing::info!("scan complete: {n} items");
            let mut queue=state.scan_queue.lock().unwrap();
            if let Some(next)=queue.pop_front() { request=next;continue; }
            state.scanning.store(false,std::sync::atomic::Ordering::SeqCst);
            reset.armed=false;
            return;
        }
    });
}

/// Scan the download's root snapshot now, or queue it for the running worker, coalescing identical
/// pending roots. Admission and queueing share the queue lock so no indexing request loses its worker.
fn queue_ingest_scan(app: Arc<App>, roots: Vec<PathBuf>) {
    let _admission=ROOT_ADMISSION.lock().unwrap();
    let mut queue=app.scan_queue.lock().unwrap();
    if app.scanning.compare_exchange(false,true,std::sync::atomic::Ordering::SeqCst,std::sync::atomic::Ordering::SeqCst).is_ok() {
        *app.scan_error.lock().unwrap()=None;
        drop(queue);
        run_scan(app,ScanRequest{roots,for_ingest:true});
    } else if let Some(request)=queue.iter_mut().find(|request|request.roots==roots) {
        request.for_ingest=true;
    } else {
        queue.push_back(ScanRequest{roots,for_ingest:true});
    }
}

/// POST /api/scan → trigger library rescan. 202 if started, 409 if busy.
async fn trigger_scan(State(app): State<std::sync::Arc<App>>) -> StatusCode {
    tokio::task::spawn_blocking(move || trigger_scan_blocking(app)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn trigger_scan_blocking(app:Arc<App>) -> StatusCode {
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
            run_scan(app, ScanRequest { roots: libs, for_ingest: false });
            StatusCode::ACCEPTED
        }
        Err(_) => StatusCode::CONFLICT,
    }
}

/// Fail an abandoned downloader without overwriting a successful handoff to queued indexing.
struct IngestReset { app: Arc<App>, staging: Option<PathBuf>, downloading: bool }
impl IngestReset {
    /// Transfer status ownership before the scan can finish and admit a later downloader.
    fn handoff(&mut self) {
        let mut status=self.app.ingest_status.lock().unwrap();
        self.downloading=false;
        *status=Some("indexing".into());
    }
}
impl Drop for IngestReset {
    /// Remove only this job's create_dir-owned staging after its child completes, on every exit.
    /// Only downloading belongs to this guard; indexing has transferred to the scan coordinator.
    fn drop(&mut self) {
        if let Some(staging)=&self.staging {
            if let Err(error)=std::fs::remove_dir_all(staging) {tracing::warn!(%error,path=%staging.display(),"ingest staging cleanup failed");}
        }
        if self.downloading {
            let mut status=self.app.ingest_status.lock().unwrap_or_else(|e|e.into_inner());
            if status.as_deref()==Some("downloading") { *status=Some("failed".into()); }
        }
    }
}

#[derive(serde::Deserialize)]
struct IngestReq {
    url: String,
}

/// A configured canonical root grants only a single child-directory creation, never root recreation.
fn ingest_destination(root: &StdPath) -> std::io::Result<PathBuf> {
    if !std::fs::symlink_metadata(root)?.file_type().is_dir() || root.canonicalize()?!=root {
        return Err(std::io::Error::other("library root is not its configured canonical directory"));
    }
    let destination=root.join("Internet");
    match std::fs::create_dir(&destination) {
        Ok(())=>(),
        Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>(),
        Err(error)=>return Err(error),
    }
    if !std::fs::symlink_metadata(&destination)?.file_type().is_dir() || destination.canonicalize()?!=destination {
        return Err(std::io::Error::other("ingest destination is not a direct library directory"));
    }
    Ok(destination)
}

/// Publish only one validated video from this job. Partial copies keep an unsupported extension.
/// No preexisting library file is replaced; the caller owns staging cleanup on every exit.
fn publish_ingest(staging: &StdPath, destination: &StdPath, backend:DownloadBackend) -> std::io::Result<()> {
    let mut videos=Vec::new();
    for entry in std::fs::read_dir(staging)? {
        let entry=entry?;
        if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|ext| ext!="mp4") {
            return Err(std::io::Error::other("unexpected downloader output"));
        }
        videos.push(entry.path());
    }
    if videos.len()!=1 || !scanner::ingest_video_valid(&videos[0]) {
        return Err(std::io::Error::other("downloaded media validation failed"));
    }
    let source=&videos[0];
    if backend==DownloadBackend::Direct && !direct_mp4_container(source) {
        return Err(std::io::Error::other("direct download is not MP4 video"));
    }
    let target=destination.join(source.file_name().ok_or_else(||std::io::Error::other("missing media name"))?);
    let partial=destination.join(format!("{}.part",staging.file_name().ok_or_else(||std::io::Error::other("missing job name"))?.to_string_lossy()));
    let mut output=std::fs::OpenOptions::new().write(true).create_new(true).open(&partial)?;
    let result=(|| {
        std::io::copy(&mut std::fs::File::open(source)?,&mut output)?;
        output.sync_all()?;
        // Both names are in the destination filesystem. Linking creates atomically without clobbering.
        match std::fs::hard_link(&partial,&target) {
            Ok(())=>Ok(()),
            Err(error) if error.kind()==std::io::ErrorKind::AlreadyExists=>{
                if backend==DownloadBackend::Direct {return Err(std::io::Error::other("direct download destination already exists"));}
                if std::fs::symlink_metadata(&target)?.file_type().is_file() && scanner::ingest_video_valid(&target) {
                    Ok(())
                } else {Err(std::io::Error::other("existing destination is not valid video"))}
            }
            Err(error)=>Err(error),
        }
    })();
    drop(output);
    // Only the create_new temporary file belongs to this job.
    if let Err(error)=std::fs::remove_file(&partial) {tracing::warn!(%error,"ingest partial cleanup failed");}
    result?;
    Ok(())
}

#[derive(Clone,Copy)]
struct DirectDownloadLimits {
    max_bytes:u64,
    transfer_timeout:std::time::Duration,
    connect_timeout:std::time::Duration,
}
const DIRECT_DOWNLOAD_LIMITS:DirectDownloadLimits=DirectDownloadLimits {
    max_bytes:8*1024*1024*1024,
    transfer_timeout:std::time::Duration::from_secs(3600),
    connect_timeout:std::time::Duration::from_secs(10),
};

/// Inspect through the existing URL parser without retaining/logging the input or transport errors.
fn direct_url(value:&str) -> Result<ureq::RequestUrl,&'static str> {
    if value.chars().any(char::is_control) {return Err("Direct URL contains a control character");}
    let parsed=ureq::get(value).request_url().map_err(|_|"Direct URL is invalid")?;
    if !matches!(parsed.as_url().scheme(),"http"|"https") || parsed.as_url().host_str().is_none() {
        return Err("Direct URL must use HTTP or HTTPS");
    }
    Ok(parsed)
}

/// Preserve a portable single MP4 component only; neither query nor fragment contributes a name.
fn direct_filename(url:&ureq::RequestUrl) -> String {
    let candidate=url.as_url().path_segments().and_then(|mut parts|parts.next_back())
        .and_then(|name|urlencoding::decode(name).ok()).map(|name|name.into_owned());
    if let Some(name)=candidate {
        let stem=StdPath::new(&name).file_stem().and_then(|value|value.to_str()).unwrap_or("");
        let upper=stem.to_ascii_uppercase();let base=upper.split('.').next().unwrap_or("");let lower=name.to_ascii_lowercase();
        let reserved=matches!(base,"CON"|"PRN"|"AUX"|"NUL")
            || ["COM","LPT"].iter().any(|prefix|base.strip_prefix(prefix).is_some_and(|tail|
                tail.len()==1 && matches!(tail.as_bytes()[0],b'1'..=b'9')));
        let safe=name.len()<=180 && !stem.is_empty() && !name.starts_with('.') && !name.ends_with(['.',' '])
            && lower.ends_with(".mp4")
            && !name.chars().any(|value|value.is_control() || "/\\:%?*<>|\"".contains(value))
            && !reserved && !matches!(lower.as_str(),"audio.tmp.mp4"|"audio.mp4");
        if safe {return format!("{}.mp4",&name[..name.len()-4]);}
    }
    format!("{}.mp4",uuid::Uuid::new_v4())
}

/// The shared publisher already proves a real video stream; direct mode additionally requires MP4.
fn direct_mp4_container(path:&StdPath) -> bool {
    let Ok(output)=Command::new("ffprobe").args(["-v","error","-show_entries","format=format_name:format_tags=major_brand","-of","json"])
        .arg(path).output() else {return false;};
    if !output.status.success() {return false;}
    let Ok(value)=serde_json::from_slice::<serde_json::Value>(&output.stdout) else {return false;};
    value["format"]["format_name"].as_str().is_some_and(|name|name.split(',').any(|part|part=="mp4"))
        && value["format"]["tags"]["major_brand"].as_str().is_some_and(|brand|!brand.is_empty() && brand!="qt  ")
}

/// Stream one HTTP entity to owned staging, with a shared transfer deadline across bounded redirects.
/// ureq bounds transfer I/O; its platform DNS resolution is not interruptible by this deadline.
fn download_direct(url:&str,staging:&StdPath,limits:DirectDownloadLimits) -> Result<(),&'static str> {
    use std::io::{Read,Write};
    let deadline=std::time::Instant::now()+limits.transfer_timeout;
    let mut current=url.to_string();
    for redirects in 0..=5 {
        let parsed=direct_url(&current)?;
        let remaining=deadline.checked_duration_since(std::time::Instant::now()).ok_or("Direct transfer deadline exceeded")?;
        let agent=ureq::AgentBuilder::new().redirects(0).timeout_connect(limits.connect_timeout.min(remaining)).build();
        let response=agent.get(&current).timeout(remaining).set("Accept-Encoding","identity").set("Connection","close")
            .call().map_err(|_|"Direct HTTP request failed")?;
        if matches!(response.status(),301|302|303|307|308) {
            if redirects==5 {return Err("Direct redirect limit exceeded");}
            let location=response.header("Location").ok_or("Direct redirect missing location")?;
            if location.chars().any(char::is_control) {return Err("Direct redirect is invalid");}
            let next=parsed.as_url().join(location).map_err(|_|"Direct redirect is invalid")?;
            if !matches!(next.scheme(),"http"|"https") {return Err("Direct redirect must use HTTP or HTTPS");}
            current=next.to_string();continue;
        }
        if response.status()!=200 {return Err("Direct response must be a complete successful file");}
        if response.all("Content-Encoding").len()>1 || response.header("Content-Encoding").is_some_and(|value|!value.eq_ignore_ascii_case("identity")) {
            return Err("Direct response encoding is unsupported");
        }
        let lengths=response.all("Content-Length");let transfers=response.all("Transfer-Encoding");
        if lengths.len()>1 || transfers.len()>1 || (!lengths.is_empty() && !transfers.is_empty())
            || transfers.first().is_some_and(|value|!value.eq_ignore_ascii_case("chunked")) {
            return Err("Direct response framing is ambiguous");
        }
        let expected=if let Some(length)=lengths.first() {
            if length.is_empty() || !length.bytes().all(|value|value.is_ascii_digit()) {return Err("Direct content length is invalid");}
            Some(length.parse::<u64>().map_err(|_|"Direct content length is invalid")?)
        } else {None};
        if expected.is_some_and(|length|length==0 || length>limits.max_bytes) {return Err("Direct response size is unsupported");}
        let partial=staging.join(format!("{}.part",uuid::Uuid::new_v4()));
        let mut output=std::fs::OpenOptions::new().write(true).create_new(true).open(&partial).map_err(|_|"Direct staging creation failed")?;
        let mut reader=response.into_reader();let mut buffer=[0_u8;64*1024];let mut count=0_u64;
        loop {
            if std::time::Instant::now()>=deadline {return Err("Direct transfer deadline exceeded");}
            let read=reader.read(&mut buffer).map_err(|_|"Direct response body failed")?;
            if std::time::Instant::now()>=deadline {return Err("Direct transfer deadline exceeded");}
            if read==0 {break;}
            count=count.checked_add(read as u64).filter(|count|*count<=limits.max_bytes).ok_or("Direct response size limit exceeded")?;
            output.write_all(&buffer[..read]).map_err(|_|"Direct staging write failed")?;
        }
        if count==0 || expected.is_some_and(|length|count!=length) {return Err("Direct response body was incomplete");}
        output.sync_all().map_err(|_|"Direct staging sync failed")?;drop(output);
        let target=staging.join(direct_filename(&parsed));
        std::fs::hard_link(&partial,&target).map_err(|_|"Direct staged filename unavailable")?;
        std::fs::remove_file(&partial).map_err(|_|"Direct staged partial cleanup failed")?;
        return Ok(());
    }
    Err("Direct redirect limit exceeded")
}

/// POST /api/ingest { url } admits the configured native or Direct MP4 video URL backend.
/// Both reuse owned staging, validated no-overwrite publication and queued indexing.
/// Native yt-dlp receives its one HTTP(S) URL after the option terminator.
async fn ingest(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    Json(body): Json<IngestReq>,
) -> StatusCode {
    tokio::task::spawn_blocking(move || ingest_blocking(app,headers,q,body)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn ingest_blocking(app:Arc<App>,headers:axum::http::HeaderMap,q:std::collections::HashMap<String,String>,body:IngestReq) -> StatusCode {
    // admin gate: downloading remote files into the library is an owner action
    {
        let db = app.db.lock().unwrap();
        if let Err(status) = require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())) {
            return status;
        }
    }
    // trust boundary: only http(s) URLs, no shell (args passed as array)
    if !body.url.starts_with("http://") && !body.url.starts_with("https://") {
        return StatusCode::BAD_REQUEST;
    }
    if !app.downloads_enabled {return StatusCode::NOT_IMPLEMENTED;}
    let Some(backend)=app.download_backend else {return StatusCode::NOT_IMPLEMENTED;};
    if backend==DownloadBackend::Direct && direct_url(&body.url).is_err() {return StatusCode::BAD_REQUEST;}
    if backend==DownloadBackend::YtDlp && std::process::Command::new("yt-dlp").arg("--version").output().is_err() {
        return StatusCode::NOT_IMPLEMENTED; // yt-dlp not installed
    }
    let libs;
    {
        let _admission=ROOT_ADMISSION.lock().unwrap();
        libs=library_roots(&app);
        if libs.is_empty() { return StatusCode::NOT_IMPLEMENTED; }
        // An unavailable configured root cannot grant authority to recreate media directories.
        if libs.iter().any(|root| !root.is_dir()) || transcode::validate_cache(&transcodes_dir_for(&app.db_path),&libs).is_none()
            || transcode::validate_cache(&thumbs_dir_for(&app.db_path),&libs).is_none() {return StatusCode::SERVICE_UNAVAILABLE;}
        let mut st=app.ingest_status.lock().unwrap();
        if matches!(st.as_deref(),Some("downloading"|"indexing")) { return StatusCode::CONFLICT; }
        *st=Some("downloading".into());
        if let Err(error)=ingest_destination(&libs[0]) {
            tracing::warn!(%error,"ingest destination unavailable");
            *st=Some("failed".into());return StatusCode::SERVICE_UNAVAILABLE;
        }
    }
    let state = app.clone();
    let url = body.url;
    // spawn detached: download can take minutes; rescan after success
    tokio::task::spawn_blocking(move || {
        let mut reset=IngestReset {app:state.clone(),staging:None,downloading:true};
        let cache=transcodes_dir_for(&state.db_path);
        let staging=cache.join(format!("ingest-{}",uuid::Uuid::new_v4()));
        if std::fs::create_dir_all(&cache).is_err() || std::fs::create_dir(&staging).is_err() { return; }
        reset.staging=Some(staging.clone());
        let ok=match backend {
            DownloadBackend::YtDlp=>{
                let out=staging.join("%(title)s.%(ext)s");
                std::process::Command::new("yt-dlp").args(["-f","mp4","--no-playlist","-o"])
                    .arg(out).arg("--").arg(&url).status().map(|status|status.success()).unwrap_or(false)
            }
            DownloadBackend::Direct=>match download_direct(&url,&staging,DIRECT_DOWNLOAD_LIMITS) {
                Ok(())=>true,
                Err(reason)=>{tracing::warn!(reason,"direct download held");false}
            },
        };
        // A failed download stays "downloading" here, and IngestReset marks it failed.
        let ok=ok && match ingest_destination(&libs[0]).and_then(|destination|publish_ingest(&staging,&destination,backend)) {
            Ok(())=>true,
            Err(error)=>{tracing::warn!(%error,path=%staging.display(),"ingest publication held");false}
        };
        if ok {
            reset.handoff();
            queue_ingest_scan(state.clone(),libs);
        }
        tracing::info!(outcome=if ok {"ok"} else {"failed"},"ingest download complete");
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
    tokio::task::spawn_blocking(move || party_create_blocking(app,headers,req).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn party_create_blocking(app:Arc<App>,headers:axum::http::HeaderMap,req:PartyCreateReq) -> axum::response::Response {
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

    let identity=q.get("identity").cloned();
    let key=match tokio::task::spawn_blocking({
        let app=app.clone();move ||party_request_identity_blocking(app,headers,identity)
    }).await {
        Ok(Ok(key))=>key,
        Ok(Err(status))=>return status.into_response(),
        Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if !app.party.can_register(&admission, &key) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.protocols(["frayd-party"])
        .on_upgrade(move |socket| party::member_socket(app.party.clone(), socket, admission, key))
        .into_response()
}

fn party_request_identity_blocking(app:Arc<App>,headers:axum::http::HeaderMap,identity:Option<String>) -> Result<String,StatusCode> {
    let db=app.db.lock().unwrap();
    party_identity_key(&db,&headers,identity.as_deref())
}

/// Report scanning, configured/count metadata and ingest lifecycle from one effective configuration read.
/// Paths remain confined to authorized settings responses.
async fn scan_status(State(app): State<std::sync::Arc<App>>) -> axum::response::Response {
    tokio::task::spawn_blocking(move || scan_status_blocking(app).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn scan_status_blocking(app:Arc<App>) -> Json<serde_json::Value> {
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
    match epub_spine_names(&app, id, &path).await {
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
    let name=epub_spine_names(&app,id,&path).await
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

/// Look up the indexed source path for a media ID; return None for a missing row.
async fn media_path(app:&Arc<App>,id:i64) -> Option<String> {
    let app=app.clone();
    tokio::task::spawn_blocking(move ||media_path_blocking(app,id)).await.ok().flatten()
}

fn media_path_blocking(app:Arc<App>,id:i64) -> Option<String> {
    let db = app.db.lock().unwrap();
    db.query_row("SELECT path FROM media WHERE id = ?1", [id], |r| r.get(0))
        .ok()
}

/// Remove only another account's owned child rows, atomically with the identity.
async fn remove_user(
    State(app): State<Arc<App>>, headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String,String>>, Path(id): Path<i64>,
) -> StatusCode {
    tokio::task::spawn_blocking(move ||remove_user_blocking(app,headers,q,id)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Keep database locking, authorization and the complete transaction on the blocking pool.
fn remove_user_blocking(app:Arc<App>,headers:axum::http::HeaderMap,
    q:std::collections::HashMap<String,String>,id:i64) -> StatusCode {
    let mut db=app.db.lock().unwrap();
    let result=(|| -> Result<(),StatusCode> {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        let caller=require_admin(&tx,&headers,q.get("identity").map(String::as_str))?;
        if caller==id {return Err(StatusCode::CONFLICT);}
        let kind: Option<String> = tx
            .query_row("SELECT kind FROM identities WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if kind.as_deref()!=Some("account") {return Err(StatusCode::NOT_FOUND);}
        let target_admin: i64 = tx
            .query_row(
                "SELECT COALESCE((SELECT is_admin FROM user_perms WHERE identity_id=?1),0)",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let other_admins: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM user_perms WHERE is_admin=1 AND identity_id!=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
    // The deleted row is no longer in the reference query, so retain its original source explicitly.
    if StdPath::new(source).canonicalize().ok().is_some_and(|source| StdPath::new(thumbnail).canonicalize().ok().as_ref()==Some(&source)) {
        return Err("thumbnail is the original source".into());
    }
    let prepared=scanner::prepare_thumbnail_retirement(db,id,thumbnail,thumbs,roots)?;
    let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|error|error.to_string())?;
    scanner::finish_thumbnail_retirement(&tx,prepared)?;
    tx.commit().map_err(|error|error.to_string())
}

/// Delete only an unreferenced media row after rechecking admin and converter admission in a write transaction.
/// Return 403/404/409/500 for denial/missing/references-or-busy/database failure; committed deletion returns 204.
/// Source files never grant unlink authority; optional thumbnail cleanup follows the committed row deletion.
async fn delete_media(
    State(app): State<Arc<App>>, headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String,String>>, Path(id): Path<i64>,
) -> axum::response::Response {
    tokio::task::spawn_blocking(move ||delete_media_blocking(app,headers,q,id)).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Keep SQLite waits and filesystem cleanup on the blocking pool under the existing admission order.
fn delete_media_blocking(app:Arc<App>,headers:axum::http::HeaderMap,
    q:std::collections::HashMap<String,String>,id:i64) -> axum::response::Response {
    // Serialize conversion admission and retain one complete root snapshot through cleanup.
    let _admission=ROOT_ADMISSION.lock().unwrap();
    let config=app.library.read().unwrap();
    let mut db=app.db.lock().unwrap();
    let mut retryable_busy=false;
    let result=(|| -> Result<(String,Option<String>),StatusCode> {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        require_admin(&tx,&headers,q.get("identity").map(String::as_str))?;
        let row: Option<(String, Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT path,thumb_path,transcode_path FROM media WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let Some((source,thumbnail,conversion))=row else {return Err(StatusCode::NOT_FOUND);};
        if conversion.is_some() {
            return Err(StatusCode::CONFLICT);
        }
        let referenced: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM progress WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM annotations WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM tracks WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM playlist_items WHERE media_id=?1)
            OR EXISTS(SELECT 1 FROM media WHERE transcode_path=?2)
            OR EXISTS(SELECT 1 FROM transcode_artifacts WHERE owner_media_id=?1 OR path=?2 OR owner_source_path=?2)",
                rusqlite::params![id, source],
                |r| r.get(0),
            )
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if referenced {
            return Err(StatusCode::CONFLICT);
        }
        if transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst) {
            retryable_busy=true;
            return Err(StatusCode::CONFLICT);
        }
        let deleted=tx.execute("DELETE FROM media WHERE id=?1",[id]).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        if deleted!=1 {return Err(StatusCode::INTERNAL_SERVER_ERROR);}
        tx.commit().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok((source,thumbnail))
    })();
    match result {
        Err(status) if retryable_busy=>(status,[(axum::http::header::RETRY_AFTER,"1")]).into_response(),
        Err(status)=>status.into_response(),
        Ok((source,thumbnail))=>{
            // An admitted scan may regenerate the same ID's cache; leave optional bytes alone.
            if !app.scanning.load(std::sync::atomic::Ordering::SeqCst) {
                if let Err(error)=cleanup_owned_thumbnail(&mut db,&thumbs_dir_for(&app.db_path),&config.roots,id,&source,thumbnail.as_deref()) {
                    tracing::warn!(id,%error,"media deletion committed; thumbnail cleanup held");
                }
            }
            StatusCode::NO_CONTENT.into_response()
        }
    }
}

/// List indexed media with optional text search and identity-scoped kind permissions.
async fn library_handler(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || library_handler_blocking(app,headers,q).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn library_handler_blocking(app:Arc<App>,headers:axum::http::HeaderMap,q:std::collections::HashMap<String,String>) -> axum::response::Response {
    let db = app.db.lock().unwrap();
    // per-identity kind visibility: NULL row/no row = see all; else only listed kinds
    let identity=match resolve_identity(&db,&headers,q.get("identity").map(String::as_str)) {
        Ok(id)=>Some(id),
        Err(StatusCode::UNAUTHORIZED)=>None,
        Err(status)=>return status.into_response(),
    };
    let allowed:Option<Vec<String>>=if let Some(id)=identity {
        match db.query_row("SELECT allowed_kinds FROM user_perms WHERE identity_id=?1",[id],|r|r.get::<_,Option<String>>(0)).optional() {
            Ok(value)=>value.flatten().and_then(|value|serde_json::from_str(&value).ok()),
            Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    } else {None};
    let kinds_clause = allowed.as_ref().map(|ks| {
        format!(
            " AND kind IN ({})",
            ks.iter().map(|k| format!("'{}'", k.replace('\'', ""))).collect::<Vec<_>>().join(",")
        )
    });
    let filter = q.get("q").map(|s| format!("%{}%", s.replace(['%', '_'], "")));
    let sql = format!(
        "SELECT id, kind, title, author, duration_secs,
                json_extract(meta_json, '$.synopsis'), series, created_at, thumb_path
         FROM media
         WHERE (?1 IS NULL OR title LIKE ?1 OR author LIKE ?1
                OR json_extract(meta_json, '$.synopsis') LIKE ?1){}
         ORDER BY created_at DESC",
        kinds_clause.as_deref().unwrap_or("")
    );
    let mut stmt = db.prepare(&sql).unwrap();
    let items:Vec<LibraryItem> = stmt
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
                artwork_version: artwork_version(r.get(0)?, r.get::<_,Option<String>>(8)?.as_deref()),
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    Json(items).into_response()
}

/// Expose persisted artwork availability as an opaque version, never an operator filesystem path.
fn artwork_version(id: i64, saved: Option<&str>) -> Option<String> {
    let path=StdPath::new(saved?);
    let name=path.file_name()?.to_str()?;
    if name == format!("{id}.jpg") || scanner::generated_thumbnail_name(id,path) {Some(name.to_string())}else{None}
}

/// Stream the indexed conversion or source using the existing single-range file service.
async fn stream(
    State(app): State<std::sync::Arc<App>>,
    Path(id): Path<i64>,
    method: axum::http::Method,
    mut headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let path=match tokio::task::spawn_blocking(move ||playback_path_blocking(app,id)).await {
        Ok(Ok(Some(path)))=>path,
        Ok(Ok(None))=>return StatusCode::NOT_FOUND.into_response(),
        _=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let metadata = match tokio::fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    // RFC 9110 requires ignoring Range outside GET. Avoid ServeFile 0.5's invalid
    // empty-file partial response and ignore unsupported multipart ranges.
    if method != axum::http::Method::GET || metadata.len() == 0 || headers.get(header::RANGE).is_some_and(|value| value.as_bytes().contains(&b',')) {
        headers.remove(header::RANGE);
    }
    let mut request = axum::http::Request::new(axum::body::Body::empty());
    *request.method_mut() = method;
    *request.headers_mut() = headers;
    match tower_http::services::ServeFile::new(&path).try_call(request).await {
        Ok(response) => {
            let mut response = response.into_response();
            response.headers_mut().insert(header::CONTENT_TYPE, mime_guess(&path).parse().unwrap());
            response
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}


fn playback_path_blocking(app:Arc<App>,id:i64) -> rusqlite::Result<Option<String>> {
    let db=app.db.lock().unwrap();
    db.query_row("SELECT COALESCE(transcode_path,path) FROM media WHERE id=?1",[id],|r|r.get(0)).optional()
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
    tokio::task::spawn_blocking(move || set_progress_blocking(app,headers,p)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn set_progress_blocking(app:Arc<App>,headers:axum::http::HeaderMap,p:ProgressIn) -> StatusCode {
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
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || get_progress_blocking(app,media_id,headers,q).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn get_progress_blocking(app:Arc<App>,media_id:i64,headers:axum::http::HeaderMap,q:std::collections::HashMap<String,String>) -> impl IntoResponse {
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
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || list_annotations_blocking(app,id,headers,q).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn list_annotations_blocking(app:Arc<App>,id:i64,headers:axum::http::HeaderMap,q:std::collections::HashMap<String,String>) -> impl IntoResponse {
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
    tokio::task::spawn_blocking(move || create_annotation_blocking(app,headers,a)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn create_annotation_blocking(app:Arc<App>,headers:axum::http::HeaderMap,a:AnnotationIn) -> StatusCode {
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
    tokio::task::spawn_blocking(move || delete_annotation_blocking(app,headers,q)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn delete_annotation_blocking(app:Arc<App>,headers:axum::http::HeaderMap,q:std::collections::HashMap<String,String>) -> StatusCode {
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

/// Resolve the caller and require an admin grant: 500 for database failures, else 403 when denied.
/// Writers call this inside their IMMEDIATE transaction so the grant is rechecked under the write lock.
fn require_admin(db: &Connection, headers: &axum::http::HeaderMap, qid: Option<&str>) -> Result<i64,StatusCode> {
    let iid=resolve_identity(db,headers,qid).map_err(|status| if status==StatusCode::INTERNAL_SERVER_ERROR {status} else {StatusCode::FORBIDDEN})?;
    let admin: Option<i64> = db
        .query_row(
            "SELECT is_admin FROM user_perms WHERE identity_id=?1",
            [iid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if admin==Some(1) { Ok(iid) } else { Err(StatusCode::FORBIDDEN) }
}

/// GET /api/admin/perms → [{identity_id, key, is_admin, allowed_kinds}]
async fn list_perms(
    State(app): State<std::sync::Arc<App>>,
    headers: axum::http::HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || list_perms_blocking(app,headers,q).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn list_perms_blocking(app:Arc<App>,headers:axum::http::HeaderMap,q:std::collections::HashMap<String,String>) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    if let Err(status) = require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())) {
        return status.into_response();
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
    tokio::task::spawn_blocking(move ||set_perm_blocking(app,headers,q,body)).await
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Keep database locking, authorization and the complete transaction on the blocking pool.
fn set_perm_blocking(app:Arc<App>,headers:axum::http::HeaderMap,
    q:std::collections::HashMap<String,String>,body:serde_json::Value) -> StatusCode {
    let mut db=app.db.lock().unwrap();
    let result=(|| -> Result<(),StatusCode> {
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;
        require_admin(&tx,&headers,q.get("identity").map(String::as_str))?;
        let map=body.as_object().ok_or(StatusCode::BAD_REQUEST)?;
        let iid=body["identity_id"].as_i64().ok_or(StatusCode::BAD_REQUEST)?;
        let admin=match map.get("is_admin") {None=>None,Some(value)=>Some(value.as_bool().ok_or(StatusCode::BAD_REQUEST)?)};
        let kinds_present=map.contains_key("allowed_kinds");
        let kinds=match map.get("allowed_kinds") {
            None|Some(serde_json::Value::Null)=>None,
            Some(serde_json::Value::Array(values))=>{
                if !values.iter().all(|v| {
                    v.as_str().is_some_and(|kind| {
                        matches!(
                            kind,
                            "audiobook"
                                | "ebook"
                                | "comic"
                                | "manga"
                                | "movie"
                                | "music"
                                | "episode"
                        )
                    })
                }) {
                    return Err(StatusCode::BAD_REQUEST);
                }
                Some(serde_json::json!(values).to_string())
            }
            Some(_)=>return Err(StatusCode::BAD_REQUEST),
        };
        let target: Option<i64> = tx
            .query_row("SELECT id FROM identities WHERE id=?1", [iid], |r| r.get(0))
            .optional()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if target.is_none() {return Err(StatusCode::NOT_FOUND);}
        if admin==Some(false) {
            let other_admins: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM user_perms WHERE is_admin=1 AND identity_id!=?1",
                    [iid],
                    |r| r.get(0),
                )
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || get_settings_blocking(app,headers,q).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn get_settings_blocking(app:Arc<App>,headers:axum::http::HeaderMap,q:std::collections::HashMap<String,String>) -> impl IntoResponse {
    let config=app.library.read().unwrap();
    let db = app.db.lock().unwrap();
    if let Err(status) = require_admin(&db, &headers, q.get("identity").map(|s| s.as_str())) {
        return status.into_response();
    }
    let rows=match db.prepare("SELECT key, value FROM settings").and_then(|mut stmt| {
        stmt.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()
    }) {
        Ok(rows)=>rows,
        Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut map = serde_json::Map::new();
    for (k, v) in rows {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&v) {
            map.insert(k, parsed);
        }
    }
    // Persisted paths and effective override are distinct authorized metadata.
    let error=if config.locked {
        config.roots.is_empty().then_some("Library configuration unavailable")
    } else { stored_roots(&db).err().map(|_| "Library configuration unavailable") };
    let paths=match stored_root_strings(&db) {
        Ok(paths)=>paths,
        Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    map.remove("library_path");
    map.insert("library_paths".into(),serde_json::json!(paths));
    map.insert("library_config_error".into(),serde_json::json!(error));
    map.insert("library_locked".into(),serde_json::json!(config.locked));
    map.insert("downloads_enabled".into(),serde_json::json!(app.downloads_enabled));
    map.insert("download_backend".into(),serde_json::json!(app.download_backend.map(DownloadBackend::name)));
    map.insert("download_config_error".into(),serde_json::json!(app.download_backend.is_none().then_some("Download backend configuration invalid")));
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
    tokio::task::spawn_blocking(move ||set_setting_blocking(app,headers,q,body)).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Validate and commit settings on the blocking pool; publish roots only after the transaction succeeds.
fn set_setting_blocking(app:Arc<App>,headers:axum::http::HeaderMap,
    q:std::collections::HashMap<String,String>,body:serde_json::Value) -> axum::response::Response {
    // Global order: admission -> config -> database -> ingest. Workers never hold
    // application database/config locks during filesystem traversal.
    let _admission=ROOT_ADMISSION.lock().unwrap();
    let mut config=app.library.write().unwrap();
    let mut db=app.db.lock().unwrap();
    if let Err(status)=require_admin(&db,&headers,q.get("identity").map(String::as_str)) { return status.into_response(); }
    let Some(mut map)=body.as_object().cloned() else { return StatusCode::BAD_REQUEST.into_response(); };
    let paths_changed=map.contains_key("library_path") || map.contains_key("library_paths");
    if map.contains_key("library_path") && map.contains_key("library_paths") { return StatusCode::BAD_REQUEST.into_response(); }
    if map.keys().any(|k| {
        matches!(
            k.as_str(),
            "library_locked" | "effective_library_paths" | "library_config_error" | "downloads_enabled" | "download_backend" | "download_config_error" | "server_version"
        )
    }) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let roots=if paths_changed {
        let value=if let Some(value)=map.remove("library_path") {
            if value.is_null() || value=="" { serde_json::json!([]) } else { serde_json::json!([value]) }
        } else { map.remove("library_paths").unwrap() };
        let Some(list)=value.as_array() else { return StatusCode::BAD_REQUEST.into_response(); };
        let retained=match stored_root_strings(&db) { Ok(paths)=>paths.into_iter().collect::<std::collections::HashSet<_>>(), Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response() };
        let mut saved=Vec::new();let mut seen=std::collections::HashSet::new();
        for entry in list {
            let Some(text)=entry.as_str() else { return StatusCode::BAD_REQUEST.into_response(); };
            let path=if retained.contains(text) { text.to_string() } else {
                let Ok(roots)=normalize_roots(&serde_json::json!([text])) else { return StatusCode::BAD_REQUEST.into_response(); };
                roots[0].to_string_lossy().into_owned()
            };
            if seen.insert(path.clone()) { saved.push(path); }
        }
        let roots=parse_roots(&serde_json::json!(saved),true).unwrap_or_default();
        // Unavailable retained roots grant no effects, but cannot prevent editing another root.
        let available:Vec<_>=saved.iter().filter_map(|path|normalize_roots(&serde_json::json!([path])).ok()).flatten().collect();
        if !available.is_empty() && (transcode::validate_cache(&transcodes_dir_for(&app.db_path),&available).is_none()
            || transcode::validate_cache(&thumbs_dir_for(&app.db_path),&available).is_none()) { return StatusCode::BAD_REQUEST.into_response(); }
        map.insert("library_paths".into(),serde_json::json!(saved));
        Some(roots)
    } else { None };
    for (key,value) in &map {
        let valid=match key.as_str() {
            "port"=>value.as_u64().is_some_and(|p| p>0 && p<=65535),
            "upnp"=>value.is_boolean(),
            "tls_cert"|"tls_key"=>value.is_string() || value.is_null(),
            _=>true,
        };
        if !valid { return StatusCode::BAD_REQUEST.into_response(); }
    }
    if paths_changed && (config.locked || app.scanning.load(std::sync::atomic::Ordering::SeqCst)
        || transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst)
        || matches!(app.ingest_status.lock().unwrap().as_deref(),Some("downloading"|"indexing"))) {
        return StatusCode::CONFLICT.into_response();
    }
    let tx=match db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
        Ok(tx)=>tx,
        Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if let Err(status)=require_admin(&tx,&headers,q.get("identity").map(String::as_str)) {return status.into_response();}
    let result=(|| -> rusqlite::Result<()> {
        for (key,value) in map {
            tx.execute(
                "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
                rusqlite::params![key, value.to_string()],
            )?;
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
) -> axum::response::Response {
    tokio::task::spawn_blocking(move || auth_me_blocking(app,headers).into_response()).await
        .unwrap_or_else(|_|StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn auth_me_blocking(app:Arc<App>,headers:axum::http::HeaderMap) -> impl IntoResponse {
    let db = app.db.lock().unwrap();
    let Some(token) = bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let iid = match resolve_identity(&db, &headers, None) {
        Ok(id)=>id,
        Err(status)=>return status.into_response(),
    };
    // token validity is implied by resolve_identity; get key + admin flag
    let key: String = match db.query_row(
        "SELECT i.key FROM identities i JOIN sessions s ON s.identity_id = i.id WHERE s.token = ?1",
        [token], |r|r.get(0)) {
        Ok(key)=>key,
        Err(_)=>return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Ok(admin) = db
        .query_row("SELECT is_admin FROM user_perms WHERE identity_id = ?1", [iid], |r| r.get::<_, i64>(0))
        .optional()
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let name = key.split('@').next().unwrap_or("user").to_string();
    Json(serde_json::json!({ "name": name, "is_admin": admin.unwrap_or(0) != 0 })).into_response()
}




#[cfg(test)]
mod test_support {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A per-run password, so no credential literal lives in the test source.
    pub fn fixture_password() -> String { uuid::Uuid::new_v4().to_string() }

    /// Command that re-runs exactly one test of this binary in a fresh process.
    pub(crate) fn child(test: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", test, "--nocapture"]);
        command
    }

    /// Run a child test and fail with its output unless exactly one test ran and passed;
    /// an `--exact` name that matches nothing runs zero tests and must not pass.
    pub(crate) fn assert_child_passes(command: &mut Command) {
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "child test did not pass exactly one test ({}):\n{}{}", output.status, stdout, String::from_utf8_lossy(&output.stderr));
    }

    /// A child selection that matches no test fails instead of passing with zero tests run.
    #[test]
    fn child_that_runs_no_test_fails() {
        let outcome = std::panic::catch_unwind(|| assert_child_passes(&mut child("test_support::no_such_child_test")));
        assert!(outcome.is_err(), "a misspelled --exact child name passed");
    }

    /// The inherited PATH with `bin` searched first.
    pub(crate) fn path_with(bin: &Path) -> std::ffi::OsString {
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&inherited))).unwrap()
    }

    /// Write a fake tool that the test can put first on PATH.
    #[cfg(unix)]
    pub(crate) fn write_executable(path: &Path, script: impl AsRef<[u8]>) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Modification time of `path` in the whole seconds stored by the scanner.
    pub(crate) fn mtime_secs(path: &Path) -> i64 {
        crate::scanner::mtime_secs(&std::fs::metadata(path).unwrap()).unwrap()
    }

    /// Scan one root that is also the complete root snapshot; a refused scan counts as zero items.
    pub(crate) fn scan(db: &rusqlite::Connection, db_path: String, root: PathBuf, thumbs: Option<&Path>) -> usize {
        let roots = [root.clone()];
        crate::scanner::scan_with_roots(db, db_path, root, thumbs, &roots).unwrap_or(0)
    }

    /// Idle application state over `db`, as `main` builds it, without starting any worker.
    pub(crate) fn app(db: rusqlite::Connection, db_path: String, roots: Vec<PathBuf>) -> std::sync::Arc<crate::App> {
        std::sync::Arc::new(crate::App::new(db, db_path, crate::LibraryConfig { roots, locked: false }))
    }

    pub(crate) async fn scan_status_body(app:std::sync::Arc<crate::App>) -> serde_json::Value {
        use axum::response::IntoResponse;
        let response=crate::scan_status(axum::extract::State(app)).await.into_response();
        assert_eq!(response.status(),axum::http::StatusCode::OK);
        serde_json::from_slice(&axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap()).unwrap()
    }

    /// Wait up to four seconds for the scan worker to release admission.
    pub(crate) async fn scan_idle(app: &crate::App) {
        for _ in 0..800 {
            if !app.scanning.load(std::sync::atomic::Ordering::SeqCst) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst),"scan worker did not release admission before the deadline");
    }
}

#[cfg(test)]
mod backend_regressions {
    use super::*;
    #[tokio::test]
    async fn artwork_http_serves_legacy_png_and_revalidates_replacement_url() {
        let directory=std::env::temp_dir().join(format!("frayd-artwork-http-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&directory).unwrap();
        let app=app_for("fixture.epub");
        let image=directory.join("legacy.jpg");std::fs::write(&image,b"\x89PNG\r\n\x1a\nlegacy").unwrap();
        app.db.lock().unwrap().execute("UPDATE media SET kind='ebook',thumb_path=?1 WHERE id=1",[image.to_string_lossy()]).unwrap();
        let response=thumb(State(app.clone()),Path(1)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(axum::body::to_bytes(response.into_body(),1024).await.unwrap().as_ref(),b"\x89PNG\r\n\x1a\nlegacy");
        let replacement=directory.join("new.jpg");std::fs::write(&replacement,[0xff,0xd8,0xff,0xd9]).unwrap();
        app.db.lock().unwrap().execute("UPDATE media SET thumb_path=?1 WHERE id=1",[replacement.to_string_lossy()]).unwrap();
        let response=thumb(State(app.clone()),Path(1)).await.into_response();assert_eq!(response.headers()[header::CONTENT_TYPE],"image/jpeg");
        assert_eq!(axum::body::to_bytes(response.into_body(),1024).await.unwrap().as_ref(),&[0xff,0xd8,0xff,0xd9]);
        app.db.lock().unwrap().execute("UPDATE media SET thumb_path=NULL WHERE id=1",[]).unwrap();
        assert_eq!(thumb(State(app),Path(1)).await.into_response().status(),StatusCode::NO_CONTENT);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn epub_boundary_cached_legacy_thumbnail_streams_bounded_frames() {
        use futures::StreamExt;
        let directory=std::env::temp_dir().join(format!("frayd-thumb-stream-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&directory).unwrap();
        let image=directory.join("1.jpg");let mut file=std::fs::File::create(&image).unwrap();
        std::io::Write::write_all(&mut file,b"\x89PNG\r\n\x1a\nlegacy").unwrap();file.set_len(8*1024*1024).unwrap();
        let app = app_for("fixture.epub");
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE media SET thumb_path=?1 WHERE id=1",
                [image.to_string_lossy()],
            )
            .unwrap();
        let response = thumb(State(app.clone()), Path(1)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        let mut chunks = response.into_body().into_data_stream();
        let first = chunks.next().await.unwrap().unwrap();
        assert!(
            first.len() <= 64 * 1024,
            "legacy artwork first body frame buffered {} bytes",
            first.len()
        );
        drop(chunks);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/api/thumb/:id", get(thumb))
            .with_state(app);
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let head = tokio::task::spawn_blocking(move || {
            let response = ureq::head(&format!("http://{address}/api/thumb/1"))
                .call()
                .unwrap();
            (
                response.status(),
                response.header("Content-Length").unwrap().to_string(),
                response.header("Content-Type").unwrap().to_string(),
            )
        })
        .await
        .unwrap();
        assert_eq!(head,(200,(8*1024*1024).to_string(),"image/png".into()));server.abort();std::fs::remove_dir_all(directory).unwrap();
    }

    /// Isolate legacy metadata and saved progress without using an operator database.
    fn app_for(path: &str) -> Arc<App> {
        let db = store::open(":memory:");
        db.execute("INSERT INTO media(id,kind,path,title,meta_json) VALUES(1,'comic',?1,'Original',?2)",
            rusqlite::params![path,r#"{"pages":["À.png","Á.png","Â.png"],"synopsis":"keep"}"#]).unwrap();
        db.execute("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture')", []).unwrap();
        db.execute("INSERT INTO progress(identity_id,media_id,position) VALUES(1,1,7)", []).unwrap();
        test_support::app(db, String::new(), Vec::new())
    }
    /// Isolate an account owner, device owner, and private rows for identity-boundary tests.
    fn identity_boundary_app() -> Arc<App> {
        let app = app_for("synthetic.cbz");
        {
            let db = app.db.lock().unwrap();
            db.execute("INSERT INTO identities(id,kind,key) VALUES(2,'account','owner@example.invalid')", []).unwrap();
            db.execute("INSERT INTO user_perms(identity_id,is_admin) VALUES(2,1)", []).unwrap();
            db.execute(
                "INSERT INTO annotations(id,identity_id,media_id,kind,spine_idx,cfi,text) VALUES(1,2,1,'highlight',0,'char:0:5','owner')",
                [],
            )
            .unwrap();
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
            set_progress(
                State(app.clone()),
                headers.clone(),
                Json(ProgressIn {
                    identity: Some("owner@example.invalid".into()),
                    media_id: 1,
                    position: 99.0,
                }),
            )
            .await,
            delete_annotation(State(app.clone()), headers.clone(), Query(query.clone())).await,
            list_perms(State(app.clone()), headers.clone(), Query(query.clone())).await.into_response().status(),
        ];
        assert_eq!(
            statuses,
            [
                StatusCode::UNAUTHORIZED,
                StatusCode::UNAUTHORIZED,
                StatusCode::UNAUTHORIZED,
                StatusCode::UNAUTHORIZED,
                StatusCode::FORBIDDEN
            ]
        );
        let mut invalid = headers;
        invalid.insert(header::AUTHORIZATION, "Bearer synthetic-invalid-session".parse().unwrap());
        let db = app.db.lock().unwrap();
        assert_eq!(resolve_identity(&db, &invalid, Some("owner@example.invalid")), Err(StatusCode::UNAUTHORIZED));
        assert_eq!(require_admin(&db, &invalid, Some("owner@example.invalid")), Err(StatusCode::FORBIDDEN));
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
        assert_eq!(
            list_annotations(
                State(app.clone()),
                Path(1),
                headers.clone(),
                Query(query.clone())
            )
            .await
            .into_response()
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            set_progress(
                State(app.clone()),
                axum::http::HeaderMap::new(),
                Json(ProgressIn {
                    identity: Some("fixture".into()),
                    media_id: 1,
                    position: 8.0
                })
            )
            .await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            get_progress(
                State(app.clone()),
                Path(1),
                axum::http::HeaderMap::new(),
                Query(query)
            )
            .await
            .into_response()
            .status(),
            StatusCode::OK
        );
        let annotation = || AnnotationIn {
            media_id: 1,
            kind: "highlight".into(),
            spine_idx: 0,
            cfi: "char:8:3".into(),
            text: Some("new".into()),
            color: None,
            note: None,
        };
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
        for (name, bytes) in [
            (
                "META-INF/container.xml",
                b"<container><rootfiles><rootfile full-path=\"book.opf\"/></rootfiles></container>"
                    .as_slice(),
            ),
            ("book.opf", opf.as_bytes()),
        ]
        .into_iter()
        .chain(chapters.iter().copied())
        {
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
                rusqlite::params![test_support::mtime_secs(&path),original.len() as i64]).unwrap();
            db.execute("UPDATE progress SET position=1 WHERE media_id=1",[]).unwrap();
        }
        let response=epub_chunk(State(app.clone()),Path((1,0))).await.into_response();
        assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"<p>Old one</p>"[..]);
        {
            let db=app.db.lock().unwrap();
            let before:String=db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
            assert_eq!(test_support::scan(&db,String::new(),root.clone(),None),0);
            assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
        }
        write_epub_fixture(&path,&[("new-one.xhtml",b"<p>Replacement first chapter</p>"),("new-two.xhtml",b"<p>Replacement second chapter</p>")]);
        assert_ne!(std::fs::metadata(&path).unwrap().len(),original.len());
        std::fs::File::open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap())).unwrap();
        let cache_after_scan={
            let db=app.db.lock().unwrap();assert_eq!(test_support::scan(&db,String::new(),root.clone(),None),1);
            db.query_row("SELECT json_type(meta_json,'$.spine') FROM media WHERE id=1",[],|r|r.get::<_,Option<String>>(0)).unwrap()
        };
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
        let db = app.db.lock().unwrap();
        let meta: String = db
            .query_row("SELECT meta_json FROM media WHERE id=1", [], |r| r.get(0))
            .unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(meta["spine"], serde_json::json!(names));
        assert_eq!(meta["synopsis"], "keep");
        assert_eq!(meta["custom"]["nested"], "keep");
        assert_eq!(
            meta["pages"],
            serde_json::json!(["À.png", "Á.png", "Â.png"])
        );
        assert_eq!(db.query_row("SELECT id FROM media WHERE path=?1",[path.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT title FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),"Original");
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),1.0);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// A cached legacy spine order is served by both the spine and chunk endpoints, so a client's
    /// chapter list, asset base paths and chunk bytes always agree.
    #[tokio::test]
    async fn cached_epub_spine_order_is_shared_by_spine_and_chunk_endpoints() {
        let path=concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/stored.epub");let app=app_for(path);
        app.db.lock().unwrap().execute(r#"UPDATE media SET
            kind='ebook',meta_json=json_set(meta_json,'$.spine',json('["two.xhtml","one.xhtml"]')) WHERE id=1"#,[]).unwrap();
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        let response=epub_chunk(State(app.clone()),Path((1,0))).await.into_response();assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"<p>Chapter two</p>"[..]);
        let response=epub_spine(State(app.clone()),Path(1)).await.into_response();
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(
                &axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap()
            )
            .unwrap(),
            vec!["two.xhtml", "one.xhtml"]
        );
        assert_eq!(app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
    }
    /// A failed archive read leaves metadata intact and preserves the existing 404 response.
    #[tokio::test]
    async fn unreadable_epub_does_not_publish_a_spine() {
        let app = app_for("/does-not-exist.epub");
        let before: String = app
            .db
            .lock()
            .unwrap()
            .query_row("SELECT meta_json FROM media WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(epub_chunk(State(app.clone()),Path((1,0))).await.into_response().status(),StatusCode::NOT_FOUND);
        assert_eq!(epub_spine(State(app.clone()),Path(1)).await.into_response().status(),StatusCode::NOT_FOUND);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
    }
    /// Optional cache-write refusal must not turn a successfully read chapter into a read failure.
    #[tokio::test]
    async fn epub_cache_write_failure_keeps_successful_chunk_response() {
        let app=app_for(concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/stored.epub"));
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        app.db
            .lock()
            .unwrap()
            .execute_batch("CREATE TRIGGER reject_spine BEFORE UPDATE OF meta_json ON media BEGIN SELECT \
                RAISE(FAIL,'fixture cache write refused'); END;")
            .unwrap();
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
            assert_eq!(publish_names(&db,1,"fixture.epub","spine",(Some(10),Some(100)),vec!["stale.xhtml".into()]),None);
            assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
        }
        db.execute(r#"UPDATE media SET mtime_secs=10,size_bytes=100,
            meta_json=json_set(meta_json,'$.custom',json('{"nested":"newer concurrent metadata"}')) WHERE id=1"#,[]).unwrap();
        assert_eq!(publish_names(&db,1,"other.epub","spine",(Some(10),Some(100)),vec!["wrong-path.xhtml".into()]),None);
        assert_eq!(publish_names(&db,1,"fixture.epub","spine",(Some(10),Some(100)),vec!["fresh.xhtml".into()]),Some(vec!["fresh.xhtml".into()]));
        let meta: String = db
            .query_row("SELECT meta_json FROM media WHERE id=1", [], |r| r.get(0))
            .unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
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
            let original=std::fs::metadata(&src).unwrap(); let actual_mtime=test_support::mtime_secs(&src);
            let app=app_for(src.to_str().unwrap());
            {
                let db=app.db.lock().unwrap();
                db.execute("UPDATE media SET kind='movie',mtime_secs=?1,size_bytes=8,transcode_path=?2 WHERE id=1",
                    rusqlite::params![if known_prior {Some(actual_mtime)} else {None},genuine.to_str().unwrap()]).unwrap();
                db.execute("INSERT INTO media(id,kind,path,title,mtime_secs,size_bytes) VALUES(2,'movie',?1,'Genuine',?2,?3)",
                    rusqlite::params![
                        genuine.to_str().unwrap(),
                        test_support::mtime_secs(&genuine),
                        std::fs::metadata(&genuine).unwrap().len() as i64
                    ],
                )
                .unwrap();
                if known_prior { assert_eq!(test_support::scan(&db,String::new(),library.clone(),None),0); }
            }
            let response=stream(State(app.clone()),Path(1),axum::http::Method::GET,axum::http::HeaderMap::new()).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"genuine legacy playback"[..]);
            std::fs::write(&src,b"new replacement source bytes").unwrap();
            std::fs::File::open(&src).unwrap().set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap())).unwrap();
            { let db=app.db.lock().unwrap();test_support::scan(&db,String::new(),library.clone(),None); }
            let response=stream(State(app.clone()),Path(1),axum::http::Method::GET,axum::http::HeaderMap::new()).await.into_response();
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
            db.execute(r#"UPDATE media SET
                mtime_secs=?1,size_bytes=?2,meta_json=json_set(meta_json,'$.pages',json('["page10.png","page2.png"]'))
                WHERE id=1"#,
                rusqlite::params![test_support::mtime_secs(&path),original.len() as i64]).unwrap();
            assert_eq!(test_support::scan(&db,String::new(),root.clone(),None),0);
        }
        let response=comic_pages(State(app.clone()),Path(1)).await.into_response();
        let unchanged:Vec<String>=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),8192).await.unwrap()).unwrap();
        assert_eq!(unchanged,vec!["page10.png","page2.png"]);
        write_comic_fixture(&path,&[("page2.png",b"second original image"),("page5.png",b"new fifth image"),("page10.png",b"tenth original image")]);
        std::fs::File::open(&path).unwrap().set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap())).unwrap();
        {
            let db=app.db.lock().unwrap();
            test_support::scan(&db,String::new(),root.clone(),None);
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
    /// zip 0.6 cached unflagged UTF-8 names as CP437; serve their physical order without rewriting state.
    #[tokio::test]
    async fn legacy_cp437_cache_http_preserves_page_order_metadata_and_progress() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/legacy-utf8-order.cbz");
        for (names, expected) in [
            (vec!["├Ç.png", "├é.png", "├ü.png"], vec!["first physical page", "third physical page", "second physical page"]),
            (vec!["├é.png"], vec!["third physical page"]),
            (vec!["Á.png"], vec!["second physical page"]),
        ] {
            let app = app_for(path);
            let metadata = serde_json::json!({"pages": names, "synopsis": "keep"}).to_string();
            app.db.lock().unwrap().execute("UPDATE media SET meta_json=?1 WHERE id=1", [&metadata]).unwrap();
            let router = Router::new()
                .route("/api/comic/:id/pages", get(comic_pages))
                .route("/api/comic/:id/page/:n", get(comic_page)).with_state(app.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
            let page_count = names.len();
            let responses = tokio::task::spawn_blocking(move || {
                let routes = std::iter::once("pages".to_string())
                    .chain((0..=page_count).map(|index| format!("page/{index}")));
                routes.map(|route| {
                    let response = match ureq::get(&format!("http://{address}/api/comic/1/{route}")).call() {
                        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
                        Err(error) => panic!("owned HTTP transport: {error}"),
                    };
                    let status = response.status();
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes).unwrap();
                    (status, bytes)
                }).collect::<Vec<_>>()
            }).await;
            server.abort();
            let _ = server.await;
            let responses = responses.unwrap();
            assert_eq!(responses[0].0, 200);
            assert_eq!(serde_json::from_slice::<Vec<String>>(&responses[0].1).unwrap(), names);
            for (response, expected) in responses[1..=page_count].iter().zip(expected) {
                assert_eq!(response.0, 200, "cached names: {names:?}");
                assert_eq!(response.1, expected.as_bytes());
            }
            assert_eq!(responses.last().unwrap().0, 404);
            let db = app.db.lock().unwrap();
            assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1", [], |r| r.get::<_, String>(0)).unwrap(), metadata);
            assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1", [], |r| r.get::<_, f64>(0)).unwrap(), 7.0);
            assert_eq!(db.query_row("SELECT title FROM media WHERE id=1", [], |r| r.get::<_, String>(0)).unwrap(), "Original");
        }
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
        app.db.lock().unwrap().execute(r#"UPDATE media SET
            meta_json=json_set(json_remove(meta_json,'$.pages'),'$.custom',json('{"nested":"keep"}')) WHERE
            id=1"#,[]).unwrap();
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        app.db
            .lock()
            .unwrap()
            .execute_batch("CREATE TRIGGER reject_comic_pages BEFORE UPDATE OF meta_json ON media BEGIN \
                SELECT RAISE(FAIL,'fixture comic cache write refused'); END;")
            .unwrap();
        let response=comic_page(State(app.clone()),Path((1,0))).await.into_response();assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(axum::body::to_bytes(response.into_body(),8192).await.unwrap(),&b"second image"[..]);
        let response=comic_pages(State(app.clone()),Path(1)).await.into_response();assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Vec<String>>(
                &axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap()
            )
            .unwrap(),
            vec!["page2.png", "page10.png"]
        );
        let db=app.db.lock().unwrap();assert_eq!(db.query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get::<_,String>(0)).unwrap(),before);
        assert_eq!(db.query_row("SELECT position FROM progress WHERE media_id=1",[],|r|r.get::<_,f64>(0)).unwrap(),7.0);
    }
    /// A zero-row cache update remains a rejected snapshot, never a readable computed fallback.
    #[tokio::test]
    async fn comic_zero_row_cache_update_does_not_return_computed_names() {
        let path=concat!(env!("CARGO_MANIFEST_DIR"),"/tests/fixtures/stored.cbz");let app=app_for(path);
        app.db.lock().unwrap().execute("UPDATE media SET meta_json=json_remove(meta_json,'$.pages') WHERE id=1",[]).unwrap();
        let before:String=app.db.lock().unwrap().query_row("SELECT meta_json FROM media WHERE id=1",[],|r|r.get(0)).unwrap();
        app.db
            .lock()
            .unwrap()
            .execute_batch("CREATE TRIGGER ignore_comic_pages BEFORE UPDATE OF meta_json ON media BEGIN SELECT RAISE(IGNORE); END;")
            .unwrap();
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
        run_scan(app.clone(),ScanRequest{roots:vec![root.join("library")],for_ingest:false});
        test_support::scan_idle(&app).await;
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
        run_scan(app.clone(),ScanRequest{roots:vec![PathBuf::from("/nonexistent")],for_ingest:false});
        test_support::scan_idle(&app).await;
        assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(test_support::scan_status_body(app.clone()).await["scan_error"],"Scan database unavailable");
        app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        *app.scan_error.lock().unwrap()=None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _reset=ScanReset {app:app.clone(),armed:true}; panic!("fixture unwind");
        }));
        assert!(result.is_err());
        assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(test_support::scan_status_body(app.clone()).await["scan_error"],"Scan failed");
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
        test_support::scan_idle(&app).await;
        let status=test_support::scan_status_body(app.clone()).await;
        assert_eq!(status["scanning"],false);
        assert_eq!(status["scan_error"],"Library root unavailable");
        std::fs::create_dir_all(&library).unwrap();
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
        test_support::scan_idle(&app).await;
        assert!(test_support::scan_status_body(app.clone()).await["scan_error"].is_null());
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
        test_support::scan_idle(&app).await;
        assert_eq!(test_support::scan_status_body(app.clone()).await["scan_error"],"Cache boundary unavailable");
        std::fs::remove_dir_all(root).unwrap();
    }
    /// A thumbnail cache that cannot be created is a reported cache-boundary failure, not a silent empty scan.
    #[tokio::test]
    async fn thumbnail_cache_creation_failure_reports_scan_error() {
        let root=std::env::temp_dir().join(format!("frayd-scan-thumbs-status-{}",uuid::Uuid::new_v4()));
        let library=root.join("library");std::fs::create_dir_all(&library).unwrap();
        let path=root.join("db.sqlite");drop(store::open(path.to_str().unwrap()));
        std::fs::write(root.join("thumbs"),b"not a directory").unwrap();
        let mut app=app_for("unused.cbz");
        Arc::get_mut(&mut app).unwrap().db_path=path.to_string_lossy().into_owned();
        app.library.write().unwrap().roots=vec![library];
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
        test_support::scan_idle(&app).await;
        assert_eq!(test_support::scan_status_body(app.clone()).await["scan_error"],"Cache boundary unavailable");
        assert_eq!(std::fs::read(root.join("thumbs")).unwrap(),b"not a directory");
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

    /// Send one over-short owner setup per `forwarded` entry from this one TCP peer and return the status codes.
    /// Setup answers 400 while the shared auth limiter allows the attempt and 429 once it does not.
    async fn setup_statuses(app: Arc<App>, forwarded: Vec<Option<&'static str>>) -> Vec<u16> {
        let router = Router::new().route("/api/setup", post(auth_setup)).with_state(app);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
        });
        let statuses = tokio::task::spawn_blocking(move || {
            forwarded.into_iter().map(|value| {
                let mut request = ureq::post(&format!("http://{address}/api/setup")).set("Content-Type", "application/json");
                if let Some(value) = value { request = request.set("X-Forwarded-For", value); }
                match request.send_string(r#"{"email":"owner@example.test","password":"x"}"#) {
                    Ok(response) | Err(ureq::Error::Status(_, response)) => response.status(),
                    Err(error) => panic!("owned HTTP transport: {error}"),
                }
            }).collect::<Vec<_>>()
        }).await.unwrap();
        server.abort();
        statuses
    }

    /// Without FRAYD_TRUSTED_PROXY a client cannot rotate X-Forwarded-For to dodge the auth limiter.
    #[tokio::test]
    async fn rotating_forwarded_for_does_not_bypass_the_auth_limiter_by_default() {
        let app = app_for("unused.cbz");
        let rotated = vec![Some("203.0.113.1"),Some("203.0.113.2"),Some("203.0.113.3"),Some("203.0.113.4"),Some("203.0.113.5"),
            Some("203.0.113.6"),Some("203.0.113.7"),Some("203.0.113.8"),Some("203.0.113.9"),Some("203.0.113.10"),Some("203.0.113.11"),None];
        let mut expected = vec![400; 10];
        expected.extend([429, 429]);
        assert_eq!(setup_statuses(app, rotated).await, expected);
    }

    /// Behind a trusted proxy the last X-Forwarded-For entry keys the limiter, so clients behind it are limited
    /// separately and a client-supplied entry that an appending proxy keeps in front is ignored.
    #[tokio::test]
    async fn trusted_proxy_keys_the_auth_limiter_by_the_last_forwarded_for_entry() {
        let mut app = app_for("unused.cbz");
        Arc::get_mut(&mut app).unwrap().trust_proxy = true;
        let mut forwarded = vec![Some("203.0.113.1"); 10];
        forwarded.extend([Some("198.51.100.9 , 203.0.113.1"), Some("203.0.113.1, 198.51.100.9"), Some("203.0.113.2"), Some("2001:db8::1")]);
        let mut expected = vec![400; 10];
        expected.extend([429, 400, 400, 400]);
        assert_eq!(setup_statuses(app, forwarded).await, expected);
    }

    /// A missing or non-IP X-Forwarded-For under a trusted proxy falls back to the shared peer key.
    #[tokio::test]
    async fn malformed_forwarded_for_falls_back_to_the_peer_behind_a_trusted_proxy() {
        let mut app = app_for("unused.cbz");
        Arc::get_mut(&mut app).unwrap().trust_proxy = true;
        let forwarded = vec![Some("not-an-ip"),Some("300.1.1.1"),Some("203.0.113.9:4000"),Some(""),None,
            Some("203.0.113.9, unknown"),Some("203.0.113.9/24"),Some("not-an-ip"),Some("300.1.1.2"),None,Some("bogus"),Some("203.0.113.10")];
        let mut expected = vec![400; 10];
        expected.extend([429, 400]);
        assert_eq!(setup_statuses(app, forwarded).await, expected);
    }

    /// The loopback peer that direct handler calls present to the auth limiter.
    fn loopback() -> ConnectInfo<std::net::SocketAddr> { ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 9))) }

    /// Move the rows to `identities_rows` behind an `identities` view whose account count fails
    /// while key lookups and inserts still reach the rows.
    fn break_account_count(app: &App) {
        app.db.lock().unwrap().execute_batch(
            "INSERT INTO identities(kind,key) VALUES('device','count-poison');
             ALTER TABLE identities RENAME TO identities_rows;
             CREATE VIEW identities AS SELECT id, CASE WHEN key='count-poison' THEN json(key) ELSE kind END AS kind,
                 key, secret_hash FROM identities_rows;
             CREATE TRIGGER identities_insert INSTEAD OF INSERT ON identities BEGIN
                 INSERT INTO identities_rows(kind,key,secret_hash) VALUES(NEW.kind,NEW.key,NEW.secret_hash); END;",
        ).unwrap();
    }

    /// A failed account count answers 500 and creates no owner account, admin grant or session.
    #[tokio::test]
    async fn setup_count_failure_creates_no_owner() {
        let app = app_for("unused.cbz");
        break_account_count(&app);
        let request = SetupReq { email: "owner@example.invalid".into(), password: crate::test_support::fixture_password() };
        let response = auth_setup(State(app.clone()), loopback(), axum::http::HeaderMap::new(), Json(request)).await.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let db = app.db.lock().unwrap();
        let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        let accounts = count("SELECT COUNT(*) FROM identities_rows WHERE kind='account'");
        assert_eq!([accounts, count("SELECT COUNT(*) FROM user_perms"), count("SELECT COUNT(*) FROM sessions")], [0, 0, 0]);
    }

    /// A refused owner admin grant or session insert answers 500 and rolls back the account, grant and session,
    /// so the next setup, not a later plain login, creates the owner and it is admin.
    #[tokio::test]
    async fn setup_grant_or_session_failure_creates_no_owner() {
        for table in ["user_perms", "sessions"] {
            let app = app_for("unused.cbz");
            app.db.lock().unwrap().execute_batch(&format!("CREATE TRIGGER reject_write BEFORE INSERT ON {table} BEGIN
                SELECT RAISE(ABORT,'fixture write refused'); END;")).unwrap();
            let request = || Json(SetupReq { email: "owner@example.invalid".into(), password: crate::test_support::fixture_password() });
            let response = auth_setup(State(app.clone()), loopback(), axum::http::HeaderMap::new(), request()).await.into_response();
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR, "{table}");
            {
                let db = app.db.lock().unwrap();
                let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
                let accounts = count("SELECT COUNT(*) FROM identities WHERE kind='account'");
                let rows = [accounts, count("SELECT COUNT(*) FROM user_perms"), count("SELECT COUNT(*) FROM sessions")];
                assert_eq!(rows, [0, 0, 0], "{table}");
                db.execute_batch("DROP TRIGGER reject_write").unwrap();
            }
            let response = auth_setup(State(app.clone()), loopback(), axum::http::HeaderMap::new(), request()).await.into_response();
            assert_eq!(response.status(), StatusCode::OK, "{table}");
            let admin = app.db.lock().unwrap().query_row("SELECT p.is_admin FROM user_perms p JOIN identities i ON i.id=p.identity_id
                WHERE i.key='owner@example.invalid'", [], |r| r.get::<_, i64>(0)).unwrap();
            assert_eq!(admin, 1, "{table}");
        }
    }

    /// Setup status reports an unreadable account count as 500, never as "setup needed".
    #[tokio::test]
    async fn setup_status_count_failure_is_500() {
        let app = app_for("unused.cbz");
        assert_eq!(setup_status(State(app.clone())).await.into_response().status(), StatusCode::OK);
        break_account_count(&app);
        assert_eq!(setup_status(State(app.clone())).await.into_response().status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// A login whose admin-flag lookup fails answers 500 and stores no session the caller never receives.
    #[tokio::test]
    async fn login_admin_lookup_failure_stores_no_session() {
        let app = app_for("unused.cbz");
        let password = crate::test_support::fixture_password();
        {
            let db = app.db.lock().unwrap();
            auth::create_account(&db, "member@example.invalid", &password).unwrap();
            db.execute_batch("DROP TABLE user_perms").unwrap();
        }
        let request = LoginReq { email: "member@example.invalid".into(), password };
        let response = auth_login(State(app.clone()), loopback(), axum::http::HeaderMap::new(), Json(request)).await.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    /// Session resume reports an unreadable admin flag as 500 instead of a non-admin session.
    #[tokio::test]
    async fn auth_me_admin_lookup_failure_is_500() {
        let app = app_for("unused.cbz");
        let mut headers = axum::http::HeaderMap::new();
        {
            let db = app.db.lock().unwrap();
            db.execute("INSERT INTO identities(id,kind,key) VALUES(2,'account','member@example.invalid')", []).unwrap();
            let token = auth::create_session(&db, 2).unwrap();
            headers.insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
        }
        assert_eq!(auth_me(State(app.clone()), headers.clone()).await.into_response().status(), StatusCode::OK);
        app.db.lock().unwrap().execute_batch("DROP TABLE user_perms").unwrap();
        assert_eq!(auth_me(State(app.clone()), headers).await.into_response().status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Streaming answers 404 for an unknown item and 500 when the media table cannot be read.
    #[tokio::test]
    async fn stream_lookup_failure_is_500() {
        let app = app_for("unused.cbz");
        let status = |id: i64| {
            let app = app.clone();
            async move { stream(State(app), Path(id), axum::http::Method::GET, axum::http::HeaderMap::new()).await.into_response().status() }
        };
        assert_eq!(status(999).await, StatusCode::NOT_FOUND);
        app.db.lock().unwrap().execute_batch("ALTER TABLE media RENAME TO media_unavailable").unwrap();
        assert_eq!(status(1).await, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

#[cfg(test)]
mod ingest_tests {
    use super::*;

    /// One library root whose name contains spaces, with a synthetic admin and a non-admin device.
    fn fixture_app(root: &StdPath) -> Arc<App> {
        let db=store::open(":memory:");
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'),(2,'device','fixture-reader');
            INSERT INTO user_perms(identity_id,is_admin) VALUES(1,1),(2,0);").unwrap();
        let library=root.join("library with spaces"); std::fs::create_dir_all(&library).unwrap();
        test_support::app(db,root.join("db.sqlite").to_string_lossy().into_owned(),vec![library])
    }

    /// Exercise the real handler's authorization and argument construction without a network downloader.
    async fn submit(app: &Arc<App>, identity: &str, url: &str) -> StatusCode {
        ingest(State(app.clone()),axum::http::HeaderMap::new(),
            Query(std::collections::HashMap::from([("identity".to_string(),identity.to_string())])),
            Json(IngestReq { url:url.to_string() })).await
    }

    #[test]
    fn downloads_override_is_exact_and_native_default_is_enabled() {
        assert!(downloads_enabled_from_override(None));
        assert!(downloads_enabled_from_override(Some(std::ffi::OsStr::new("1"))));
        for value in ["","0","true","false","01"," 1","1 "] {
            assert!(!downloads_enabled_from_override(Some(std::ffi::OsStr::new(value))));
        }
        #[cfg(unix)] {
            use std::os::unix::ffi::OsStringExt;
            let invalid=std::ffi::OsString::from_vec(vec![0xff]);
            assert!(!downloads_enabled_from_override(Some(&invalid)));
        }
    }

    /// Disabled downloads reject before the version probe, source creation or job-state publication.
    #[tokio::test]
    async fn downloads_disabled_preserves_auth_and_url_guards_without_tool_or_job_effects() {
        if let Some(root)=std::env::var_os("FRAYD_TEST_DOWNLOAD_GATE_ROOT") {
            let root=PathBuf::from(root);let app=fixture_app(&root);
            assert_eq!(submit(&app,"fixture-reader","https://example.invalid/video").await,StatusCode::FORBIDDEN);
            assert_eq!(submit(&app,"fixture-admin","file:///invalid").await,StatusCode::BAD_REQUEST);
            assert_eq!(submit(&app,"fixture-admin","https://example.invalid/video").await,StatusCode::NOT_IMPLEMENTED);
            assert!(!app.downloads_enabled);
            let query=||Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())]));
            let response=get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            let settings:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap()).unwrap();
            assert_eq!(settings["downloads_enabled"],false);
            assert_eq!(set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"downloads_enabled":true}))).await.status(),StatusCode::BAD_REQUEST);
            assert!(!root.join("tool-called").exists(),"disabled mode probed or invoked the downloader");
            assert!(!root.join("library with spaces/Internet").exists());
            assert!(app.ingest_status.lock().unwrap().is_none());assert!(app.scan_queue.lock().unwrap().is_empty());
            assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
            return;
        }
        let root=std::env::temp_dir().join(format!("frayd-download-gate-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(root.join("bin")).unwrap();
        test_support::write_executable(&root.join("bin/yt-dlp"),"#!/bin/sh\ntouch \"$FRAYD_TEST_DOWNLOAD_MARKER\"\nexit 1\n");
        // Qualify the tool marker before using its absence as the disabled-mode oracle.
        std::process::Command::new(root.join("bin/yt-dlp")).arg("--version")
            .env("FRAYD_TEST_DOWNLOAD_MARKER",root.join("tool-called")).output().unwrap();
        assert!(root.join("tool-called").exists());std::fs::remove_file(root.join("tool-called")).unwrap();
        test_support::assert_child_passes(test_support::child("ingest_tests::downloads_disabled_preserves_auth_and_url_guards_without_tool_or_job_effects")
            .env("FRAYD_TEST_DOWNLOAD_GATE_ROOT",&root).env("FRAYD_TEST_DOWNLOAD_MARKER",root.join("tool-called"))
            .env("FRAYD_DOWNLOADS_ENABLED","0").env("PATH",test_support::path_with(&root.join("bin"))));
        assert!(!root.join("tool-called").exists());std::fs::remove_dir_all(root).unwrap();
    }

    /// An old downloader guard must not fail a newly admitted download after indexing completes.
    #[test]
    fn handed_off_ingest_guard_cannot_fail_next_download() {
        let root=std::env::temp_dir().join(format!("frayd-ingest-guard-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();let app=fixture_app(&root);
        *app.ingest_status.lock().unwrap()=Some("downloading".into());
        let stage=root.join("owned-stage");std::fs::create_dir(&stage).unwrap();
        let mut reset=IngestReset {app:app.clone(),staging:Some(stage.clone()),downloading:true};
        reset.handoff();
        // Deterministically schedule scan completion and B admission before A's destructor.
        *app.ingest_status.lock().unwrap()=Some("done".into());
        *app.ingest_status.lock().unwrap()=Some("downloading".into());
        drop(reset);
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("downloading"));
        assert!(!stage.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Ingest must refuse a preexisting redirect before any downloader or publication writes.
    #[cfg(unix)]
    #[tokio::test]
    async fn ingest_rejects_redirected_destination_without_touching_outside() {
        if let Some(root)=std::env::var_os("FRAYD_TEST_DEST_ROOT") {
            let root=PathBuf::from(root);let app=fixture_app(&root);
            let library=root.join("library with spaces");let outside=root.join("outside");
            std::fs::create_dir(&outside).unwrap();std::fs::write(outside.join("sentinel"),b"preserved").unwrap();
            std::os::unix::fs::symlink(&outside,library.join("Internet")).unwrap();
            assert_eq!(submit(&app,"fixture-admin","https://example.invalid/video").await,StatusCode::SERVICE_UNAVAILABLE);
            assert!(!root.join("download-started").exists());
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(),1);
            assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(),b"preserved");
            std::fs::remove_file(library.join("Internet")).unwrap();
            std::fs::write(library.join("Internet"),b"existing non-directory").unwrap();
            assert_eq!(submit(&app,"fixture-admin","https://example.invalid/video").await,StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(std::fs::read(library.join("Internet")).unwrap(),b"existing non-directory");
            std::fs::remove_file(library.join("Internet")).unwrap();std::fs::remove_dir(&library).unwrap();
            assert!(ingest_destination(&library).is_err());
            assert_eq!(submit(&app,"fixture-admin","https://example.invalid/video").await,StatusCode::SERVICE_UNAVAILABLE);
            assert!(!library.exists());assert!(!root.join("download-started").exists());
            return;
        }
        let root=std::env::temp_dir().join(format!("frayd-ingest-dest-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        test_support::write_executable(&root.join("bin/yt-dlp"),"#!/bin/sh\n[ \"$1\" = --version ] && exit 0\ntouch \"$FRAYD_TEST_DEST_ROOT/download-started\"\nexit 1\n");
        test_support::assert_child_passes(test_support::child("ingest_tests::ingest_rejects_redirected_destination_without_touching_outside")
            .env("FRAYD_TEST_DEST_ROOT",&root).env("PATH",test_support::path_with(&root.join("bin"))));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A zero-exit downloader must not publish invalid bytes; a later valid job remains admissible.
    #[cfg(unix)]
    #[tokio::test]
    async fn invalid_download_fails_before_indexing_and_next_valid_job_succeeds() {
        if let Some(root)=std::env::var_os("FRAYD_TEST_MEDIA_ROOT") {
            let root=PathBuf::from(root);
            let library=root.join("library");std::fs::create_dir(&library).unwrap();
            let db_path=root.join("db.sqlite");let db=store::open(db_path.to_str().unwrap());
            db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'); INSERT INTO user_perms(identity_id,is_admin) VALUES(1,1);").unwrap();
            let app=test_support::app(db,db_path.to_string_lossy().into_owned(),vec![library.clone()]);
            let preserved=library.join("unrelated.cbz");std::fs::write(&preserved,b"original").unwrap();
            let cache=root.join("transcodes");std::fs::create_dir(&cache).unwrap();
            let unowned=cache.join("unowned.txt");std::fs::write(&unowned,b"preserve cache neighbor").unwrap();
            let mut original=None;
            for (url,expected) in [("https://example.invalid/bad","failed"),("https://example.invalid/partial","failed"),("https://example.invalid/good","done"),("https://example.invalid/good","done"),("https://example.invalid/different","done")] {
                assert_eq!(submit(&app,"fixture-admin",url).await,StatusCode::ACCEPTED);
                for _ in 0..1200 {
                    if matches!(app.ingest_status.lock().unwrap().as_deref(),Some("done"|"failed")) {break;}
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some(expected));
                test_support::scan_idle(&app).await;
                for _ in 0..200 {
                    if std::fs::read_dir(&cache).unwrap().all(|entry|!entry.unwrap().file_name().to_string_lossy().starts_with("ingest-")) {break;}
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                assert!(std::fs::read_dir(&cache).unwrap().all(|entry|!entry.unwrap().file_name().to_string_lossy().starts_with("ingest-")));
                assert_eq!(std::fs::read(&unowned).unwrap(),b"preserve cache neighbor");
                let db=app.db.lock().unwrap();
                assert_eq!(db.query_row("SELECT COUNT(*) FROM media WHERE path LIKE '%wrong.mp4'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
                assert_eq!(std::fs::read(&preserved).unwrap(),b"original");
                if expected=="failed" {
                    assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|row|row.get::<_,i64>(0)).unwrap(),0);
                    assert_eq!(std::fs::read_dir(library.join("Internet")).unwrap().count(),0);
                }
                if expected=="done" {
                    let published=library.join("Internet/good.mp4");
                    let id=db.query_row("SELECT id FROM media WHERE path=?1",[published.to_str().unwrap()],|row|row.get::<_,i64>(0)).unwrap();
                    let bytes=std::fs::read(&published).unwrap();
                    let mtime=std::fs::metadata(&published).unwrap().modified().unwrap();
                    if let Some((prior_id,prior_bytes,prior_mtime))=&original {
                        assert_eq!(id,*prior_id);assert_eq!(&bytes,prior_bytes);assert_eq!(&mtime,prior_mtime);
                        assert_eq!(db.query_row("SELECT position FROM progress WHERE identity_id=1 AND media_id=?1",[id],|row|row.get::<_,f64>(0)).unwrap(),0.5);
                    } else {
                        db.execute("INSERT INTO progress(identity_id,media_id,position) VALUES(1,?1,0.5)",[id]).unwrap();
                        original=Some((id,bytes,mtime));
                    }
                }
            }
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE kind='movie'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            return;
        }
        let root=std::env::temp_dir().join(format!("frayd-invalid-ingest-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        assert!(Command::new("ffmpeg").args(["-v","error","-f","lavfi","-i","color=size=16x16:rate=1","-t","1","-c:v","libx264"]).arg(root.join("good.mp4")).status().unwrap().success());
        let mut different=std::fs::read(root.join("good.mp4")).unwrap();different.extend_from_slice(b"different output");
        std::fs::write(root.join("different.mp4"),different).unwrap();
        test_support::write_executable(&root.join("bin/yt-dlp"),"#!/bin/sh\n[ \"$1\" = --version ] && exit 0\nout=$5\ndest=${out%/*}\nfor arg do url=$arg; done\ncase \"$url\" in */bad) printf 'not an mp4' > \"$dest/wrong.mp4\";; */partial) cp \"$FRAYD_TEST_MEDIA_ROOT/good.mp4\" \"$dest/good.mp4\"; printf 'partial' > \"$dest/unfinished.mp4.part\";; */different) cp \"$FRAYD_TEST_MEDIA_ROOT/different.mp4\" \"$dest/good.mp4\";; *) cp \"$FRAYD_TEST_MEDIA_ROOT/good.mp4\" \"$dest/good.mp4\";; esac\n");
        test_support::assert_child_passes(test_support::child("ingest_tests::invalid_download_fails_before_indexing_and_next_valid_job_succeeds")
            .env("FRAYD_TEST_MEDIA_ROOT",&root).env("PATH",test_support::path_with(&root.join("bin"))));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Keep HTTP authorization, destination selection and explicit URL framing correct at the process boundary.
    #[tokio::test]
    async fn ingest_preserves_admin_guards_and_literal_downloader_arguments() {
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
                assert_eq!(actual.len(),7);
                let output=PathBuf::from(&actual[4]);
                assert_eq!(output.file_name().unwrap(),"%(title)s.%(ext)s");
                assert_eq!(output.parent().unwrap().parent().unwrap(),root.join("transcodes"));
                assert!(output.parent().unwrap().file_name().unwrap().to_str().unwrap().starts_with("ingest-"));
                assert_eq!(actual,vec!["-f","mp4","--no-playlist","-o",output.to_str().unwrap(),"--",url]);
                assert!(!marker.exists(),"URL metacharacters executed as a shell command");
            }
            assert_eq!(std::fs::read(calls).unwrap(),b"....");
            return;
        }
        let root=std::env::temp_dir().join(format!("frayd-ingest-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        test_support::write_executable(
            &root.join("bin/yt-dlp"),
            "#!/bin/sh\nprintf '.' >> \"$FRAYD_TEST_INGEST_CALLS\"\n\
                if [ \"$1\" = \"--version\" ]; then exit 0; fi\n\
                printf '%s\\0' \"$@\" > \"$FRAYD_TEST_INGEST_ARGV\"\nexit 1\n",
        );
        test_support::assert_child_passes(test_support::child("ingest_tests::ingest_preserves_admin_guards_and_literal_downloader_arguments")
            .env("FRAYD_TEST_INGEST_ROOT",&root).env("FRAYD_TEST_INGEST_CALLS",root.join("calls"))
            .env("FRAYD_TEST_INGEST_ARGV",root.join("argv")).env("PATH",test_support::path_with(&root.join("bin"))));
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod roots_scan_ingest_tests {
    use super::*;
    /// Create independent two-root data and a disposable database with a synthetic administrator.
    fn fixture() -> (PathBuf, Arc<App>) {
        let root=std::env::temp_dir().join(format!("frayd-roots-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let db=store::open(root.join("db.sqlite").to_str().unwrap());

        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'); INSERT INTO \
            user_perms(identity_id,is_admin) VALUES(1,1);")
            .unwrap();
        let app=test_support::app(db,root.join("db.sqlite").to_string_lossy().into_owned(),Vec::new());
        (root,app)
    }
    /// Payload errors stay 400 during conversion; a valid root change remains temporarily held.
    #[tokio::test]
    async fn settings_validate_payload_before_conversion_busy() {
        if std::env::var_os("FRAYD_TEST_SETTING_BUSY").is_none() {
            test_support::assert_child_passes(test_support::child("roots_scan_ingest_tests::settings_validate_payload_before_conversion_busy")
                .env("FRAYD_TEST_SETTING_BUSY","1"));return;
        }
        let (root,app)=fixture();
        transcode::BUSY.store(true,std::sync::atomic::Ordering::SeqCst);
        for body in [serde_json::json!({"library_paths":[],"port":0}),serde_json::json!({"library_paths":"invalid"}),
            serde_json::json!({"library_paths":[],"server_version":"invalid"})] {
            assert_eq!(setting(&app,body).await,StatusCode::BAD_REQUEST);
        }
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[]})).await,StatusCode::CONFLICT);
        transcode::BUSY.store(false,std::sync::atomic::Ordering::SeqCst);
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[]})).await,StatusCode::NO_CONTENT);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn roots_array_replaces_legacy() {
        let (root,app)=fixture();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO settings(key,value) VALUES('library_path',?1)",
                [serde_json::json!(root.join("a")).to_string()],
            )
            .unwrap();
        *app.library.write().unwrap()=effective_library_config(&app.db.lock().unwrap(),None).unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO settings(key,value) VALUES('library_paths',?1)",
                [serde_json::json!([root.join("b")]).to_string()],
            )
            .unwrap();
        *app.library.write().unwrap()=effective_library_config(&app.db.lock().unwrap(),None).unwrap();
        assert_eq!(library_roots(&app),vec![root.join("b")]);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Exercise the production settings transaction through the fixture administrator identity.
    async fn setting(app: &Arc<App>, value:serde_json::Value) -> StatusCode {
        set_setting(
            State(app.clone()),
            axum::http::HeaderMap::new(),
            Query(std::collections::HashMap::from([(
                "identity".into(),
                "fixture-admin".into(),
            )])),
            Json(value),
        )
        .await
        .status()
    }
    #[test]
    fn roots_override_empty_array_and_aliases() {
        let (root,app)=fixture(); let db=app.db.lock().unwrap();
        db.execute("INSERT INTO settings(key,value) VALUES('library_path',?1)",[serde_json::json!(root.join("a")).to_string()]).unwrap();
        assert_eq!(effective_library_config(&db,Some("")).unwrap().roots,vec![root.join("a")]);
        db.execute("INSERT INTO settings(key,value) VALUES('library_paths','[]')",[]).unwrap();
        assert!(effective_library_config(&db,None).unwrap().roots.is_empty());
        let overridden=effective_library_config(&db,root.join("b").to_str()).unwrap();
        assert_eq!(overridden.roots,vec![root.join("b")]);assert!(overridden.locked);
        #[cfg(unix)] {
            std::os::unix::fs::symlink(root.join("a"),root.join("alias")).unwrap();
            assert_eq!(
                normalize_roots(&serde_json::json!([
                    root.join("a"),
                    root.join("alias"),
                    root.join("b")
                ]))
                .unwrap(),
                vec![root.join("a"), root.join("b")]
            );
        }
        for value in [
            serde_json::json!("bad"),
            serde_json::json!([""]),
            serde_json::json!([root.join("a"), root.join("missing")]),
            serde_json::json!([root.join("db.sqlite")]),
        ] {
            assert!(normalize_roots(&value).is_err());
        }
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn roots_settings_atomic_failure_and_metadata() {
        let (root,app)=fixture();
        assert_eq!(setting(&app,serde_json::json!({"library_path":root.join("a"),"port":9999})).await,StatusCode::NO_CONTENT);
        assert_eq!(library_roots(&app),vec![root.join("a")]);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM settings WHERE key='library_path'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let before=app.library.read().unwrap().clone();
        for value in [
            serde_json::json!({"library_paths":[root.join("b"),root.join("missing")],"port":9998}),
            serde_json::json!({"library_paths":[root.join("b")],"port":0}),
            serde_json::json!({"library_path":root.join("a"),"library_paths":[]}),
        ] {
            assert_eq!(setting(&app,value).await,StatusCode::BAD_REQUEST);assert_eq!(*app.library.read().unwrap(),before);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT value FROM settings WHERE key='port'",[],|r|r.get::<_,String>(0)).unwrap(),"9999");
        }
        app.db
            .lock()
            .unwrap()
            .execute_batch("CREATE TRIGGER reject_port BEFORE UPDATE ON settings WHEN NEW.key='port' BEGIN SELECT RAISE(ABORT,'fixture'); END;")
            .unwrap();
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[root.join("b")],"port":9998})).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(*app.library.read().unwrap(),before);
        assert_eq!(stored_roots(&app.db.lock().unwrap()).unwrap(),before.roots);
        let response = get_settings(
            State(app.clone()),
            axum::http::HeaderMap::new(),
            Query(std::collections::HashMap::from([(
                "identity".into(),
                "fixture-admin".into(),
            )])),
        )
        .await
        .into_response();
        let value:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),65536).await.unwrap()).unwrap();
        assert_eq!(value["library_paths"],serde_json::json!([root.join("a")])); assert_eq!(value["effective_library_paths"],value["library_paths"]);
        assert_eq!(value["library_locked"],false);assert_eq!(value["server_version"],VERSION);
        assert!(value.get("library_path").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn roots_override_and_active_jobs_refuse_whole_request() {
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
    async fn scan_ingest_queue_indexes_before_done_and_retains_removed_roots() {
        let (root,app)=fixture();
        let roots=vec![root.join("a"),root.join("b")];
        *app.library.write().unwrap()=LibraryConfig{roots:roots.clone(),locked:false};
        std::fs::write(root.join("b/retained.cbz"),b"source").unwrap();
        std::fs::create_dir(root.join("a/Internet")).unwrap();std::fs::write(root.join("a/Internet/download.cbz"),b"download").unwrap();
        // Simulate an admitted busy worker: ingestion must queue its complete snapshot.
        app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        *app.ingest_status.lock().unwrap()=Some("indexing".into());
        queue_ingest_scan(app.clone(),roots.clone());
        assert_eq!(app.scan_queue.lock().unwrap().len(),1);
        let status=test_support::scan_status_body(app.clone()).await;
        assert_eq!(status,serde_json::json!({"scanning":true,"scan_error":null,"library_configured":true,"library_count":2,"ingesting":"indexing"}));
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        run_scan(app.clone(),ScanRequest{roots:roots.clone(),for_ingest:false});
        test_support::scan_idle(&app).await;
        assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("done"));
        let id: i64 = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM media WHERE path=?1",
                [root.join("b/retained.cbz").to_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        app.db.lock().unwrap().execute("INSERT INTO progress(identity_id,media_id,position) VALUES(1,?1,42)",[id]).unwrap();
        let thumbnail=root.join("thumbs/retained.jpg");std::fs::write(&thumbnail,b"thumbnail").unwrap();
        app.db.lock().unwrap().execute(r#"UPDATE media SET
            thumb_path=?1,meta_json='{"synopsis":"keep","pages":["one"]}' WHERE id=?2"#,rusqlite::params![thumbnail.to_str().unwrap(),id]).unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi,note) VALUES(1,?1,'bookmark',0,'fixture','keep')",
                [id],
            )
            .unwrap();
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[root.join("a")]})).await,StatusCode::NO_CONTENT);
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
        test_support::scan_idle(&app).await;
        assert_eq!(app.db.lock().unwrap().query_row("SELECT position FROM progress WHERE media_id=?1",[id],|r|r.get::<_,f64>(0)).unwrap(),42.0);
        assert_eq!(std::fs::read(root.join("b/retained.cbz")).unwrap(),b"source");
        assert_eq!(std::fs::read(&thumbnail).unwrap(),b"thumbnail");
        assert_eq!(app.db.lock().unwrap().query_row("SELECT note FROM annotations WHERE media_id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),"keep");
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT json_extract(meta_json,'$.pages[0]') FROM media WHERE id=?1",
                    [id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "one"
        );
        assert_eq!(setting(&app,serde_json::json!({"library_paths":[]})).await,StatusCode::NO_CONTENT);
        assert!(effective_library_config(&app.db.lock().unwrap(),Some("")).unwrap().roots.is_empty());
        assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::NOT_IMPLEMENTED);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT position FROM progress WHERE media_id=?1",[id],|r|r.get::<_,f64>(0)).unwrap(),42.0);
        std::fs::remove_dir_all(root).unwrap();
    }
    /// A required root lost after admission fails both ordinary scans and ingest, retaining its sources and rows.
    #[cfg(unix)]
    #[tokio::test]
    async fn root_vanishing_mid_pass_reports_failure_and_keeps_rows() {
        if std::env::var_os("FRAYD_TEST_VANISH_CHILD").is_none() {
            test_support::assert_child_passes(test_support::child("roots_scan_ingest_tests::root_vanishing_mid_pass_reports_failure_and_keeps_rows")
                .env("FRAYD_TEST_VANISH_CHILD","1").env_remove("FRAYD_TRANSCODES").env_remove("FRAYD_THUMBS"));
            return;
        }
        let (root,app)=fixture();
        let roots=vec![root.join("a"),root.join("b")];
        *app.library.write().unwrap()=LibraryConfig {roots:roots.clone(),locked:false};
        let bin=root.join("bin");std::fs::create_dir(&bin).unwrap();
        let started=root.join("started");let release=root.join("release");
        test_support::write_executable(&bin.join("ffprobe"),format!(
            "#!/bin/sh\ntouch '{}'\nfor i in $(seq 1 500); do\n  if test -f '{}'; then exit 1; fi\n  sleep 0.01\ndone\nexit 1\n",started.display(),release.display()));
        test_support::write_executable(&bin.join("ffmpeg"),"#!/bin/sh\nexit 1\n");
        transcode::BUSY.store(true,std::sync::atomic::Ordering::SeqCst);
        std::fs::write(root.join("b/retained.cbz"),b"source").unwrap();
        app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
        run_scan(app.clone(),ScanRequest{roots:roots.clone(),for_ingest:false});
        test_support::scan_idle(&app).await;
        std::env::set_var("PATH",test_support::path_with(&bin));
        std::fs::create_dir(root.join("a/Internet")).unwrap();
        std::fs::write(root.join("a/Internet/download.cbz"),b"download").unwrap();
        for for_ingest in [true,false] {
            std::fs::write(root.join(format!("a/clip-{for_ingest}.mp4")),b"video").unwrap();
            if for_ingest { *app.ingest_status.lock().unwrap()=Some("indexing".into()); }
            app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
            run_scan(app.clone(),ScanRequest{roots:roots.clone(),for_ingest});
            for _ in 0..800 {
                if started.exists() {break;}
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert!(started.exists(),"first root scan did not reach blocked probe");
            std::fs::rename(root.join("b"),root.join("b-away")).unwrap();
            std::fs::write(&release,b"release").unwrap();
            test_support::scan_idle(&app).await;
            let status=test_support::scan_status_body(app.clone()).await;
            assert_eq!(library_roots(&app),roots);
            assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(status["scan_error"],"Library root unavailable","{status}");
            if for_ingest {assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"));}
            assert_eq!(std::fs::read(root.join("b-away/retained.cbz")).unwrap(),b"source");
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE path=?1",
                [root.join("b/retained.cbz").to_string_lossy()],|row|row.get::<_,i64>(0)).unwrap(),1);
            std::fs::rename(root.join("b-away"),root.join("b")).unwrap();
            app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
            run_scan(app.clone(),ScanRequest{roots:roots.clone(),for_ingest:false});
            test_support::scan_idle(&app).await;
            assert!(test_support::scan_status_body(app.clone()).await["scan_error"].is_null());
            std::fs::remove_file(&started).unwrap();std::fs::remove_file(&release).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cache_all_roots_refuse_before_registry_or_directory_mutation() {
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
        assert_eq!(
            scanner::scan_with_roots(
                &db,
                app.db_path.clone(),
                root.join("a"),
                Some(&root.join("b/thumbs")),
                &roots
            ),
            Err(scanner::ScanError::CacheBoundaryUnavailable)
        );
        assert_eq!(db.query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(!root.join("b/thumbs").exists());drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn scan_ingest_blocked_downloader_queues_original_snapshot() {
        if let Some(root)=std::env::var_os("FRAYD_TEST_QUEUE_ROOT") {
            let root=PathBuf::from(root);let (fixture_root,app)=fixture();
            let roots=vec![fixture_root.join("a"),fixture_root.join("b")];
            *app.library.write().unwrap()=LibraryConfig{roots:roots.clone(),locked:false};
            std::fs::write(roots[1].join("blocking.mkv"),b"synthetic").unwrap();
            assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
            for _ in 0..400 {if root.join("probe-ready").exists(){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(root.join("probe-ready").exists());
            let submit = || {
                ingest(
                    State(app.clone()),
                    axum::http::HeaderMap::new(),
                    Query(std::collections::HashMap::from([(
                        "identity".into(),
                        "fixture-admin".into(),
                    )])),
                    Json(IngestReq {
                        url: "https://example.invalid/video".into(),
                    }),
                )
            };
            assert_eq!(submit().await,StatusCode::ACCEPTED);
            for _ in 0..400 {if root.join("download-ready").exists(){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(root.join("download-ready").exists());assert_eq!(submit().await,StatusCode::CONFLICT);
            assert_eq!(setting(&app,serde_json::json!({"library_paths":[]})).await,StatusCode::CONFLICT);
            std::fs::write(root.join("download-release"),[]).unwrap();
            for _ in 0..400 {
                if app.ingest_status.lock().unwrap().as_deref() == Some("indexing") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("indexing"));
            assert_eq!(app.scan_queue.lock().unwrap().front().unwrap().roots,roots);
            assert_eq!(
                app.db
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT COUNT(*) FROM media WHERE path LIKE '%download.mp4'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            std::fs::write(root.join("probe-release"),[]).unwrap();
            test_support::scan_idle(&app).await;
            assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("done"));
            assert_eq!(
                app.db
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT COUNT(*) FROM media WHERE path LIKE '%download.mp4'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
            // Conversion admission holds reconfiguration through the real blocked codec probe.
            for _ in 0..400 {
                if !transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert!(!transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst),"prior conversion worker did not become idle");
            std::fs::remove_file(root.join("probe-ready")).unwrap();std::fs::remove_file(root.join("probe-release")).unwrap();
            let movie=roots[1].join("blocking.mkv");
            let id=app.db.lock().unwrap().query_row("SELECT id FROM media WHERE path=?1",[movie.to_str().unwrap()],|r|r.get::<_,i64>(0)).unwrap();
            transcode::maybe_enqueue_with_roots(app.db_path.clone(),id,movie,roots.clone());
            for _ in 0..400 {if root.join("probe-ready").exists(){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
            assert!(root.join("probe-ready").exists(),"new worker did not reach its codec probe");
            assert!(transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst));
            assert_eq!(setting(&app,serde_json::json!({"library_paths":[],"port":9999})).await,StatusCode::CONFLICT);
            std::fs::write(root.join("probe-release"),[]).unwrap();
            for _ in 0..400 {
                if !transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert!(!transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst));
            // Failed/unwound scan abandons indexing, clears its queue and admits later scans.
            *app.ingest_status.lock().unwrap()=Some("indexing".into());app.scanning.store(true,std::sync::atomic::Ordering::SeqCst);
            queue_ingest_scan(app.clone(),roots.clone());
            drop(ScanReset {app:app.clone(),armed:true});
            assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"));assert!(app.scan_queue.lock().unwrap().is_empty());
            assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::ACCEPTED);
            test_support::scan_idle(&app).await;
            // Scan completion can leave its admitted converter running; finish owned work before teardown.
            for _ in 0..400 {
                if !transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst) {break;}
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert!(!transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst),"final converter did not become idle before fixture teardown");
            std::fs::remove_dir_all(fixture_root).unwrap();return;
        }
        let root=std::env::temp_dir().join(format!("frayd-queue-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(root.join("bin")).unwrap();
        test_support::write_executable(
            &root.join("bin/yt-dlp"),
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\n\
                touch \"$FRAYD_TEST_QUEUE_ROOT/download-ready\"\n\
                while [ ! -e \"$FRAYD_TEST_QUEUE_ROOT/download-release\" ]; do sleep 0.01; done\nout=$5\n\
                dest=${out%/*}\ncp \"$FRAYD_TEST_QUEUE_ROOT/valid.mp4\" \"$dest/download.mp4\"\n",
        );
        test_support::write_executable(
            &root.join("bin/ffprobe"),
            "#!/bin/sh\nfor arg do case \"$arg\" in */ingest-*/*) exec \"$FRAYD_TEST_REAL_FFPROBE\" \"$@\";; esac; done\ntouch \"$FRAYD_TEST_QUEUE_ROOT/probe-ready\"\n\
                while [ ! -e \"$FRAYD_TEST_QUEUE_ROOT/probe-release\" ]; do sleep 0.01; done\nexit 1\n",
        );
        assert!(Command::new("ffmpeg").args(["-v","error","-f","lavfi","-i","color=size=16x16:rate=1","-t","1","-c:v","libx264"]).arg(root.join("valid.mp4")).status().unwrap().success());
        let ffprobe=std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory|directory.join("ffprobe")).find(|path|path.is_file()).unwrap();
        test_support::assert_child_passes(test_support::child("roots_scan_ingest_tests::scan_ingest_blocked_downloader_queues_original_snapshot")
            .env("FRAYD_TEST_QUEUE_ROOT",&root).env("FRAYD_TEST_REAL_FFPROBE",ffprobe).env("PATH",test_support::path_with(&root.join("bin"))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn scan_ingest_database_rejection_and_unavailable_root_fail() {
        let (root,app)=fixture();let roots=vec![root.join("a")];
        std::fs::create_dir(root.join("a/Internet")).unwrap();std::fs::write(root.join("a/Internet/download.cbz"),b"download").unwrap();
        app.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_media BEFORE INSERT ON media BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        *app.ingest_status.lock().unwrap()=Some("indexing".into());queue_ingest_scan(app.clone(),roots.clone());
        test_support::scan_idle(&app).await;
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"));
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        *app.ingest_status.lock().unwrap()=Some("indexing".into());queue_ingest_scan(app.clone(),vec![root.join("missing")]);
        test_support::scan_idle(&app).await;
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"));assert!(app.scan_queue.lock().unwrap().is_empty());
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
                assert!(
                    !upnp_mapping_requested(bind.ip(), environment, stored),
                    "loopback mapping requested: {address}, env={environment}, stored={stored}"
                );
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
    fn roots_scan_admission_reconfiguration_race() {
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
            if result == StatusCode::NO_CONTENT {
                assert_eq!(config.roots, vec![root.join("b")]);
                assert_eq!(stored_roots(&app.db.lock().unwrap()).unwrap(), config.roots);
            } else {
                assert_eq!(config.roots, vec![root.join("a")]);
            }
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
        let app=test_support::app(db,root.join("db.sqlite").to_string_lossy().into_owned(),vec![root.join("library")]);
        (root,app)
    }
    #[tokio::test]
    async fn artwork_media_delete_retires_current_uuid_and_retains_original_and_legacy() {
        let (root,app)=fixture();seed_media(&root,&app);
        let legacy=root.join("thumbs/1.jpg");std::fs::write(&legacy,b"legacy thumbnail").unwrap();
        let generated=root.join("thumbs").join(format!("1-{}.jpg",uuid::Uuid::new_v4()));std::fs::write(&generated,b"generated artwork").unwrap();
        app.db.lock().unwrap().execute("UPDATE media SET thumb_path=?1 WHERE id=1",[generated.to_string_lossy()]).unwrap();
        assert_eq!(media(&app,1).await,StatusCode::NO_CONTENT);
        assert!(!generated.exists());
        assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(), b"thumbnail");
        assert_eq!(std::fs::read(legacy).unwrap(), b"legacy thumbnail");
        assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn artwork_api_library_and_search_expose_only_opaque_versions() {
        let (root,app)=fixture();
        let versions = [
            Some(format!("1-{}.jpg", uuid::Uuid::new_v4())),
            Some("2.jpg".into()),
            Some(format!("3-{}.jpg", uuid::Uuid::new_v4())),
            Some(format!("4-{}.jpg", uuid::Uuid::new_v4())),
            None,
            None,
            None,
        ];
        {
            let db=app.db.lock().unwrap();
            for (index,kind) in ["ebook","comic","episode","music","audiobook","manga","movie"].iter().enumerate() {
                let saved = if index == 5 {
                    Some("/operator/private/cache/unknown.jpg".into())
                } else if index == 6 {
                    Some(format!(
                        "/operator/private/cache/9-{}.jpg",
                        uuid::Uuid::new_v4()
                    ))
                } else {
                    versions[index]
                        .as_ref()
                        .map(|version| format!("/operator/private/cache/{version}"))
                };
                db.execute(
                    "INSERT INTO media(id,kind,path,title,thumb_path) VALUES(?1,?2,?3,?4,?5)",
                    rusqlite::params![
                        index as i64 + 1,
                        kind,
                        format!("/operator/private/source/{index}"),
                        if index < 4 {
                            format!("Cover {kind}")
                        } else {
                            format!("Absent {kind}")
                        },
                        saved
                    ],
                )
                .unwrap();
            }
            db.execute("UPDATE user_perms SET allowed_kinds='[\"ebook\",\"music\"]' WHERE identity_id=2",[]).unwrap();
        }
        let session=auth::create_session(&app.db.lock().unwrap(),2).unwrap();
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let router=Router::new().route("/api/library",get(library_handler)).with_state(app);
        let server=tokio::spawn(async move {axum::serve(listener,router).await.unwrap();});
        for (query,restricted,expected) in [("",false,7),("?q=Cover",false,4),("?q=Cover",true,2)] {
            let url=format!("http://{address}/api/library{query}");let token=session.clone();
            let body = tokio::task::spawn_blocking(move || {
                let request = ureq::get(&url);
                let request = if restricted {
                    request.set("Authorization", &format!("Bearer {token}"))
                } else {
                    request
                };
                request.call().unwrap().into_string().unwrap()
            })
            .await
            .unwrap();
            assert!(!body.contains("/operator/") && !body.contains("thumb_path"),"operator paths must remain internal");
            let items:Vec<serde_json::Value>=serde_json::from_str(&body).unwrap();assert_eq!(items.len(),expected);
            for item in items {
                let id = item["id"].as_u64().unwrap() as usize;
                assert!(
                    item.get("artwork_version").is_some(),
                    "availability field absent for id={id}"
                );
                assert_eq!(item["artwork_version"], serde_json::json!(versions[id - 1]));
                if restricted {
                    assert!(matches!(item["kind"].as_str(), Some("ebook" | "music")));
                }
            }
        }
        server.abort();std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn artwork_api_delete_retains_selected_legacy_thumbnail() {
        let (root,app)=fixture();seed_media(&root,&app);
        let legacy=root.join("thumbs/1.jpg");std::fs::rename(owned_thumb(&root),&legacy).unwrap();
        app.db.lock().unwrap().execute("UPDATE media SET thumb_path=?1 WHERE id=1",[legacy.to_string_lossy()]).unwrap();
        assert_eq!(media(&app,1).await,StatusCode::NO_CONTENT);
        assert_eq!(std::fs::read(legacy).unwrap(),b"thumbnail");
        assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Supply the existing fixture device administrator without creating an account-name fallback.
    fn query() -> Query<std::collections::HashMap<String, String>> {
        Query(std::collections::HashMap::from([(
            "identity".into(),
            "fixture-admin".into(),
        )]))
    }
    /// Retry hints identify only temporary conversion admission, with permanent references still unhinted.
    #[tokio::test]
    async fn admin_busy_delete_exposes_retry_after_only_for_transient_conflict() {
        if std::env::var_os("FRAYD_TEST_DELETE_RETRY").is_none() {
            test_support::assert_child_passes(test_support::child("integration_regressions::admin_busy_delete_exposes_retry_after_only_for_transient_conflict")
                .env("FRAYD_TEST_DELETE_RETRY","1"));return;
        }
        let (root,app)=fixture();seed_media(&root,&app);
        transcode::BUSY.store(true,std::sync::atomic::Ordering::SeqCst);
        let response=delete_media(State(app.clone()),axum::http::HeaderMap::new(),query(),Path(1)).await.into_response();
        assert_eq!(response.status(),StatusCode::CONFLICT);
        assert_eq!(response.headers().get("retry-after").and_then(|value|value.to_str().ok()),Some("1"));
        app.db.lock().unwrap().execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42)").unwrap();
        let response=delete_media(State(app.clone()),axum::http::HeaderMap::new(),query(),Path(1)).await.into_response();
        assert_eq!(response.status(),StatusCode::CONFLICT);assert!(!response.headers().contains_key("retry-after"));
        transcode::BUSY.store(false,std::sync::atomic::Ordering::SeqCst);
        assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A held SQLite writer must not stop a current-thread Tokio timer while an admin operation waits.
    #[tokio::test(flavor="current_thread")]
    async fn admin_database_wait_keeps_async_runtime_responsive() {
        for operation in ["settings","media","permission","user"] {
            let (root,app)=fixture();seed_media(&root,&app);
            let writer=store::connect(&app.db_path).unwrap();writer.execute_batch("BEGIN IMMEDIATE").unwrap();
            let (progress_tx,progress_rx)=std::sync::mpsc::channel();
            let release=std::thread::spawn(move || {
                let responsive=progress_rx.recv_timeout(std::time::Duration::from_millis(300)).is_ok();
                writer.execute_batch("COMMIT").unwrap();responsive
            });
            let request=async {
                if operation=="media" {media(&app,1).await} else if operation=="user" {
                    user(&app,2).await
                } else if operation=="permission" {
                    permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":["comic"]})).await
                } else {
                    set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"port":8989}))).await.status()
                }
            };
            let timer=async {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                let _=progress_tx.send(());
            };
            let (status,())=tokio::join!(request,timer);
            assert_eq!(status,StatusCode::NO_CONTENT);
            assert!(release.join().unwrap(),"{operation} blocked the async runtime while waiting for SQLite");
            assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// Positively observe a blocked database writer before ordinary reads/config readers join it.
    #[tokio::test(flavor="current_thread")]
    async fn mixed_database_and_configuration_waits_keep_async_runtime_responsive() {
        static WRITE_WAIT_ENTERED:std::sync::atomic::AtomicBool=std::sync::atomic::AtomicBool::new(false);
        fn observe_write_wait(attempt:i32) -> bool {
            WRITE_WAIT_ENTERED.store(true,std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1));attempt<5000
        }
        for operation in ["library","scan-status"] {
            let (root,app)=fixture();seed_media(&root,&app);
            WRITE_WAIT_ENTERED.store(false,std::sync::atomic::Ordering::SeqCst);
            app.db.lock().unwrap().busy_handler(Some(observe_write_wait)).unwrap();
            let writer=store::connect(&app.db_path).unwrap();writer.execute_batch("BEGIN IMMEDIATE").unwrap();
            let (armed_tx,armed_rx)=std::sync::mpsc::channel();let (progress_tx,progress_rx)=std::sync::mpsc::channel();
            let release=std::thread::spawn(move || {
                armed_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
                let responsive=progress_rx.recv_timeout(std::time::Duration::from_millis(300)).is_ok();
                writer.execute_batch("COMMIT").unwrap();responsive
            });
            let target=app.clone();
            let admin=tokio::spawn(async move {
                if operation=="library" {permission(&target,serde_json::json!({"identity_id":2,"allowed_kinds":["comic"]})).await}
                else {set_setting(State(target),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"port":8989}))).await.status()}
            });
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
            while !WRITE_WAIT_ENTERED.load(std::sync::atomic::Ordering::SeqCst) {
                assert!(std::time::Instant::now()<deadline,"admin never reached SQLite write admission");
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            armed_tx.send(()).unwrap();
            let read=||async {
                if operation=="library" {library_handler(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::new())).await.into_response().status()}
                else {scan_status(State(app.clone())).await.into_response().status()}
            };
            let readers=async {tokio::join!(read(),read())};
            let timer=async {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                assert_eq!(health().await.0,serde_json::json!({"status":"ok"}));
                let _=progress_tx.send(());
            };
            let ((first,second),())=tokio::join!(readers,timer);
            assert_eq!((first,second),(StatusCode::OK,StatusCode::OK));
            assert_eq!(admin.await.unwrap(),StatusCode::NO_CONTENT);
            assert!(release.join().unwrap(),"{operation} readers stalled the async runtime before writer release");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// Exercise the real media deletion handler with the fixture administrator and target ID.
    async fn media(app:&Arc<App>,id:i64)->StatusCode {delete_media(State(app.clone()),axum::http::HeaderMap::new(),query(),Path(id)).await.status()}
    /// Exercise the real account deletion handler through its administrative identity boundary.
    async fn user(app:&Arc<App>,id:i64)->StatusCode {remove_user(State(app.clone()),axum::http::HeaderMap::new(),query(),Path(id)).await}
    /// Exercise validated permission writes through the production transaction path.
    async fn permission(app: &Arc<App>, value: serde_json::Value) -> StatusCode {
        set_perm(
            State(app.clone()),
            axum::http::HeaderMap::new(),
            query(),
            Json(value),
        )
        .await
    }
    fn owned_thumb(root:&StdPath) -> PathBuf {root.join("thumbs/1-11111111-1111-4111-8111-111111111111.jpg")}
    /// Bind one indexed source and its generated UUID-named thumbnail.
    fn seed_media(root:&StdPath,app:&Arc<App>) {
        let source=root.join("library/source.cbz");let thumb=owned_thumb(root);
        std::fs::write(&source,b"original").unwrap();std::fs::write(&thumb,b"thumbnail").unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO media(id,kind,path,title,thumb_path) VALUES(1,'comic',?1,'fixture',?2)",
                rusqlite::params![source.to_str().unwrap(), thumb.to_str().unwrap()],
            )
            .unwrap();
    }
    #[tokio::test]
    /// Saved progress makes deletion conflict, and the refusal keeps source and thumbnail bytes.
    async fn admin_referenced_media_refuses_without_losing_thumbnail() {
        let (root,app)=fixture();seed_media(&root,&app);
        app.db.lock().unwrap().execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42);").unwrap();
        assert_eq!(media(&app,1).await,StatusCode::CONFLICT);
        assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(),b"thumbnail");
        assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// ABORT and IGNORE at the final identity delete roll back the already deleted child rows.
    async fn admin_user_failure_rolls_back_children() {
        let (root,app)=fixture();seed_media(&root,&app);
        app.db.lock().unwrap().execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42);
            CREATE TRIGGER reject_target BEFORE DELETE ON identities WHEN OLD.id=2 BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert_eq!(user(&app,2).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM progress WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM user_perms WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        app.db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_target;CREATE TRIGGER reject_target BEFORE DELETE ON \
                identities WHEN OLD.id=2 BEGIN SELECT RAISE(IGNORE); END")
            .unwrap();
        assert_eq!(user(&app,2).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM progress WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM user_perms WHERE identity_id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// Explicit null must persist SQL NULL rather than silently retaining the previous movie-only restriction.
    async fn admin_permission_null_clears_restriction() {
        let (root,app)=fixture();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":null})).await,StatusCode::NO_CONTENT);
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT allowed_kinds FROM user_perms WHERE identity_id=2",
                    [],
                    |r| r.get::<_, Option<String>>(0)
                )
                .unwrap(),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Compare complete table cardinalities for rollback without serializing session values.
    fn table_counts(app:&Arc<App>) -> Vec<(String,i64)> {
        let db=app.db.lock().unwrap();
        [
            "identities",
            "sessions",
            "user_perms",
            "progress",
            "annotations",
            "playlists",
            "playlist_items",
            "media",
            "albums",
            "tracks",
            "transcode_artifacts",
        ]
        .into_iter()
        .map(|table| {
            (
                table.to_string(),
                db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap(),
            )
        })
        .collect()
    }
    /// Create two owners with separate sessions/playlists/user data over shared media, tracks and artifacts.
    fn seed_user_children(app:&Arc<App>) {
        let db=app.db.lock().unwrap();
        for id in [2,3] {
            db.execute(
                "INSERT INTO sessions(token,identity_id,expires_at) VALUES(?1,?2,unixepoch()+3600)",
                rusqlite::params![uuid::Uuid::new_v4().to_string(), id],
            )
            .unwrap();
        }
        db.execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42),(3,1,73);
            INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi,note) VALUES(2,1,'bookmark',0,\
                'target','target-note'),(3,1,'bookmark',0,'other','other-note');
            INSERT INTO playlists(id,identity_id,title) VALUES(2,2,'target'),(3,3,'other');
            INSERT INTO playlist_items(playlist_id,media_id,pos) VALUES(2,1,0),(3,1,0);
            INSERT INTO albums(id,title) VALUES(1,'shared'); INSERT INTO tracks(media_id,album_id) VALUES(1,1);
            INSERT INTO transcode_artifacts(path,owner_media_id,owner_source_path,role,provenance) \
                VALUES('/synthetic-artifact',1,'/synthetic-source','final','generated');")
            .unwrap();
    }
    #[tokio::test]
    /// Seed each independent media reference and require conflict with unchanged table counts and file bytes.
    async fn admin_all_media_references_refuse_unchanged() {
        for reference in [
            "progress",
            "annotations",
            "tracks",
            "playlist_items",
            "own_conversion",
            "other_conversion",
            "artifact_owner",
            "artifact_path",
            "artifact_source",
        ] {
            let (root,app)=fixture();seed_media(&root,&app);
            {
                let db=app.db.lock().unwrap();
                match reference {
                    "progress"=>{db.execute_batch("INSERT INTO progress(identity_id,media_id,position) VALUES(2,1,42)").unwrap();}
                    "annotations" => {
                        db.execute_batch(
                            "INSERT INTO annotations(identity_id,media_id,kind,spine_idx,cfi) VALUES(2,1,'bookmark',0,'fixture')",
                        )
                        .unwrap();
                    }
                    "tracks"=>{db.execute_batch("INSERT INTO tracks(media_id) VALUES(1)").unwrap();}
                    "playlist_items" => {
                        db.execute_batch(
                            "INSERT INTO playlists(id,identity_id,title) VALUES(1,2,'fixture');INSERT INTO \
                                playlist_items(playlist_id,media_id,pos) VALUES(1,1,0)",
                        )
                        .unwrap();
                    }
                    "own_conversion"=>{db.execute("UPDATE media SET transcode_path='/synthetic-cache' WHERE id=1",[]).unwrap();}
                    "other_conversion" => {
                        db.execute(
                            "INSERT INTO media(id,kind,path,title,transcode_path) VALUES(2,'movie','/synthetic-other','other',?1)",
                            [root.join("library/source.cbz").to_str().unwrap()],
                        )
                        .unwrap();
                    }
                    "artifact_owner" => {
                        db.execute(
                            "INSERT INTO transcode_artifacts VALUES('/synthetic-output',1,?1,'final','generated','completed')",
                            [root.join("library/source.cbz").to_str().unwrap()],
                        )
                        .unwrap();
                    }
                    "artifact_path"|"artifact_source"=>{
                        db.execute_batch("INSERT INTO media(id,kind,path,title) VALUES(2,'movie','/synthetic-other','other')").unwrap();
                        let source=root.join("library/source.cbz").to_string_lossy().into_owned();
                        let (artifact_path, artifact_source) = if reference == "artifact_path" {
                            (source, "/synthetic-other".to_string())
                        } else {
                            ("/synthetic-output".into(), source)
                        };
                        db.execute(
                            "INSERT INTO transcode_artifacts VALUES(?1,2,?2,'final','generated','completed')",
                            rusqlite::params![artifact_path, artifact_source],
                        )
                        .unwrap();
                    }
                    _=>unreachable!(),
                }
            }
            let before = table_counts(&app);
            assert_eq!(
                media(&app, 1).await,
                StatusCode::CONFLICT,
                "reference {reference}"
            );
            assert_eq!(table_counts(&app), before);
            assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(), b"thumbnail");
            assert_eq!(
                std::fs::read(root.join("library/source.cbz")).unwrap(),
                b"original"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    /// Only a committed unreferenced deletion removes the owned thumbnail; ABORT/IGNORE retain source and cache.
    async fn admin_media_commit_and_database_failure_preserve_sources() {
        for failure in [false,true] {
            let (root,app)=fixture();seed_media(&root,&app);
            if failure {
                app.db
                    .lock()
                    .unwrap()
                    .execute_batch("CREATE TRIGGER reject_media BEFORE DELETE ON media BEGIN SELECT RAISE(ABORT,'fixture'); END")
                    .unwrap();
            }
            assert_eq!(media(&app,1).await,if failure {StatusCode::INTERNAL_SERVER_ERROR}else{StatusCode::NO_CONTENT});
            assert_eq!(owned_thumb(&root).exists(),failure);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),if failure {1}else{0});
            assert_eq!(std::fs::read(root.join("library/source.cbz")).unwrap(),b"original");
            if !failure {assert_eq!(media(&app,1).await,StatusCode::NOT_FOUND);}
            else {
                app.db
                    .lock()
                    .unwrap()
                    .execute_batch("DROP TRIGGER reject_media;CREATE TRIGGER reject_media BEFORE DELETE ON media BEGIN SELECT RAISE(IGNORE); END")
                    .unwrap();
                assert_eq!(media(&app,1).await,StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(),b"thumbnail");
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    /// Unproven/shared/redirected/root-overlapping or scan-active thumbnails survive a successful metadata deletion.
    async fn admin_thumbnail_refusals_are_postcommit_and_byte_preserving() {
        for refusal in [
            "shared",
            "alias-shared",
            "artifact-shared",
            "symlink",
            "unowned",
            "missing-saved",
            "library-b",
            "unavailable-root",
            "source",
            "scan-active",
        ] {
            let (root,app)=fixture();seed_media(&root,&app);
            match refusal {
                "shared"|"alias-shared"=>{
                    let mut path=owned_thumb(&root);
                    #[cfg(unix)]
                    if refusal == "alias-shared" {
                        std::os::unix::fs::symlink(&path, root.join("alias.jpg")).unwrap();
                        path = root.join("alias.jpg");
                    }
                    app.db
                        .lock()
                        .unwrap()
                        .execute(
                            "INSERT INTO media(id,kind,path,title,thumb_path) VALUES(2,'comic','/synthetic-other','other',?1)",
                            [path.to_str().unwrap()],
                        )
                        .unwrap();
                }
                "artifact-shared"=>{
                    let db = app.db.lock().unwrap();
                    db.execute_batch("INSERT INTO media(id,kind,path,title) VALUES(2,'movie','/synthetic-other','other')").unwrap();
                    db.execute(
                        "INSERT INTO transcode_artifacts VALUES(?1,2,'/synthetic-other','final','generated','completed')",
                        [owned_thumb(&root).to_str().unwrap()],
                    )
                    .unwrap();
                }
                "symlink"=>{
                    #[cfg(unix)]
                    {
                        std::fs::remove_file(owned_thumb(&root)).unwrap();
                        std::fs::write(root.join("outside.jpg"), b"thumbnail").unwrap();
                        std::os::unix::fs::symlink(root.join("outside.jpg"), owned_thumb(&root))
                            .unwrap();
                    }
                }
                "unowned"=>{app.db.lock().unwrap().execute("UPDATE media SET thumb_path=NULL WHERE id=1",[]).unwrap();}
                "missing-saved" => {
                    app.db
                        .lock()
                        .unwrap()
                        .execute(
                            "UPDATE media SET thumb_path=?1 WHERE id=1",
                            [root
                                .join("thumbs/1-22222222-2222-4222-8222-222222222222.jpg")
                                .to_str()
                                .unwrap()],
                        )
                        .unwrap();
                }
                "library-b"=>app.library.write().unwrap().roots.push(root.join("thumbs")),
                "unavailable-root"=>app.library.write().unwrap().roots.push(root.join("missing-root")),
                "source"=>{app.db.lock().unwrap().execute("UPDATE media SET path=?1 WHERE id=1",[owned_thumb(&root).to_str().unwrap()]).unwrap();}
                "scan-active"=>app.scanning.store(true,std::sync::atomic::Ordering::SeqCst),
                _=>unreachable!(),
            }
            assert_eq!(media(&app,1).await,StatusCode::NO_CONTENT,"refusal {refusal}");
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE id=1",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(), b"thumbnail");
            assert_eq!(
                std::fs::read(root.join("library/source.cbz")).unwrap(),
                b"original"
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[tokio::test]
    /// Remove one account including its playlist graph and session, preserving another owner and shared media graph.
    async fn admin_user_cleanup_targets_only_one_account() {
        let (root,app)=fixture();seed_media(&root,&app);seed_user_children(&app);
        let session=auth::create_session(&app.db.lock().unwrap(),2).unwrap();
        assert_eq!(auth::identity_from_token(&app.db.lock().unwrap(),&session).unwrap(),Some(2));
        assert_eq!(user(&app,2).await,StatusCode::NO_CONTENT);
        assert_eq!(auth::identity_from_token(&app.db.lock().unwrap(),&session).unwrap(),None);
        {
        let db=app.db.lock().unwrap();
        for table in ["sessions","user_perms","progress","annotations","playlists"] {
            assert_eq!(db.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE identity_id=2"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
            assert_eq!(db.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE identity_id=3"),[],|r|r.get::<_,i64>(0)).unwrap(),1);
        }
        assert_eq!(db.query_row("SELECT COUNT(*) FROM playlist_items WHERE playlist_id=3",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        for table in ["media", "albums", "tracks", "transcode_artifacts"] {
            assert_eq!(
                db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        assert_eq!(db.query_row("SELECT position FROM progress WHERE identity_id=3",[],|r|r.get::<_,f64>(0)).unwrap(),73.0);
        assert_eq!(db.query_row("SELECT note FROM annotations WHERE identity_id=3",[],|r|r.get::<_,String>(0)).unwrap(),"other-note");        }
        assert_eq!(
            std::fs::read(root.join("library/source.cbz")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(), b"thumbnail");
        assert_eq!(user(&app,2).await,StatusCode::NOT_FOUND);assert_eq!(user(&app,1).await,StatusCode::CONFLICT);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// Check whole-graph rollback, omitted/null/empty semantics, all schema kinds, type refusals and failed writes.
    async fn admin_user_full_rollback_and_permission_validation() {
        let (root,app)=fixture();seed_media(&root,&app);seed_user_children(&app);
        let before=table_counts(&app);
        app.db
            .lock()
            .unwrap()
            .execute_batch("CREATE TRIGGER reject_user BEFORE DELETE ON identities WHEN OLD.id=2 BEGIN SELECT RAISE(ABORT,'fixture'); END")
            .unwrap();
        assert_eq!(user(&app,2).await,StatusCode::INTERNAL_SERVER_ERROR);assert_eq!(table_counts(&app),before);
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"is_admin":true})).await,StatusCode::NO_CONTENT);
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT allowed_kinds FROM user_perms WHERE identity_id=2",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "[\"movie\"]"
        );
        for kinds in [serde_json::json!([]),serde_json::json!(["audiobook","ebook","comic","manga","movie","music","episode"])] {
            assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":kinds})).await,StatusCode::NO_CONTENT);
            assert_eq!(
                app.db
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT allowed_kinds FROM user_perms WHERE identity_id=2",
                        [],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                kinds.to_string()
            );
            let response=library_handler(State(app.clone()),bearer(&app,2),Query(std::collections::HashMap::new())).await;
            assert_eq!(response.status(),StatusCode::OK);
            let listed:Vec<serde_json::Value>=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap()).unwrap();
            assert_eq!(listed.len(),if kinds.as_array().unwrap().is_empty() {0}else{1});
        }
        let before = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT is_admin,allowed_kinds FROM user_perms WHERE identity_id=2",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        for value in [
            serde_json::json!({"identity_id":2,"is_admin":"true"}),
            serde_json::json!({"identity_id":2,"is_admin":null}),
            serde_json::json!({"identity_id":2,"allowed_kinds":["unknown"],"is_admin":false}),
            serde_json::json!({"identity_id":2,"allowed_kinds":[1]}),
            serde_json::json!({"identity_id":2,"allowed_kinds":"movie"}),
        ] {
            assert_eq!(permission(&app,value).await,StatusCode::BAD_REQUEST);
            assert_eq!(
                app.db
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT is_admin,allowed_kinds FROM user_perms WHERE identity_id=2",
                        [],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
                    )
                    .unwrap(),
                before
            );
        }
        app.db.lock().unwrap().execute_batch("UPDATE user_perms SET is_admin=0 WHERE identity_id=2").unwrap();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":1,"is_admin":false,"allowed_kinds":[]})).await,StatusCode::CONFLICT);
        app.db
            .lock()
            .unwrap()
            .execute_batch("CREATE TRIGGER reject_perm BEFORE UPDATE ON user_perms WHEN NEW.identity_id=2 BEGIN SELECT RAISE(ABORT,'fixture'); END")
            .unwrap();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":null})).await,StatusCode::INTERNAL_SERVER_ERROR);
        app.db
            .lock()
            .unwrap()
            .execute_batch("DROP TRIGGER reject_perm;CREATE TRIGGER reject_perm BEFORE UPDATE ON user_perms \
                WHEN NEW.identity_id=2 BEGIN SELECT RAISE(IGNORE); END")
            .unwrap();
        assert_eq!(permission(&app,serde_json::json!({"identity_id":2,"allowed_kinds":null})).await,StatusCode::INTERNAL_SERVER_ERROR);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    /// Revoke admin while each real handler waits behind a write lock; no cached pre-transaction authority may survive.
    fn admin_rechecks_authority_after_database_write_admission() {
        static WRITE_WAIT_ENTERED:std::sync::atomic::AtomicBool=std::sync::atomic::AtomicBool::new(false);
        fn observe_write_wait(attempt:i32) -> bool {
            WRITE_WAIT_ENTERED.store(true,std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1));
            attempt<3000
        }
        for operation in ["media","user","permission","settings","settings-db-error"] {
            let (root,app)=fixture();seed_media(&root,&app);
            let original_roots=library_roots(&app);
            WRITE_WAIT_ENTERED.store(false,std::sync::atomic::Ordering::SeqCst);
            app.db.lock().unwrap().busy_handler(Some(observe_write_wait)).unwrap();
            let other=store::connect(&app.db_path).unwrap();other.execute_batch("BEGIN IMMEDIATE").unwrap();
            let target=app.clone();let (ready_tx,ready_rx)=std::sync::mpsc::channel();let (done_tx,done_rx)=std::sync::mpsc::channel();
            let worker=std::thread::spawn(move || {
                let runtime=tokio::runtime::Runtime::new().unwrap();ready_tx.send(()).unwrap();
                let result = runtime.block_on(async {
                    match operation {
                        "media" => media(&target, 1).await,
                        "user" => user(&target, 2).await,
                        "settings" | "settings-db-error" => set_setting(State(target.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"library_paths":[],"port":8989}))).await.status(),
                        _ => {
                            permission(
                                &target,
                                serde_json::json!({"identity_id":2,"allowed_kinds":null}),
                            )
                            .await
                        }
                    }
                });
                done_tx.send(result).unwrap();
            });
            ready_rx.recv().unwrap();
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
            while !WRITE_WAIT_ENTERED.load(std::sync::atomic::Ordering::SeqCst) {
                assert!(done_rx.try_recv().is_err(),"{operation} completed before write admission");
                assert!(std::time::Instant::now()<deadline,"{operation} never reached SQLite write admission");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if operation=="settings-db-error" {other.execute_batch("DROP TABLE user_perms;COMMIT").unwrap();}
            else {other.execute_batch("UPDATE user_perms SET is_admin=0 WHERE identity_id=1;COMMIT").unwrap();}
            let expected=if operation=="settings-db-error" {StatusCode::INTERNAL_SERVER_ERROR} else {StatusCode::FORBIDDEN};
            assert_eq!(done_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),expected,"{operation}");worker.join().unwrap();
            assert_eq!(library_roots(&app),original_roots,"{operation} published configuration without authority");
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM settings WHERE key='port'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM identities WHERE id=2",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media WHERE id=1",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(),b"thumbnail");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    /// Hold conversion admission while starting deletion, then publish BUSY and require conflict with unchanged bytes.
    fn admin_converter_admission_refuses_media_delete() {
        if std::env::var_os("FRAYD_TEST_ADMIN_BUSY").is_none() {
            test_support::assert_child_passes(
                test_support::child(
                    "integration_regressions::admin_converter_admission_refuses_media_delete",
                )
                .env("FRAYD_TEST_ADMIN_BUSY", "1"),
            );
            return;
        }
        let (root,app)=fixture();seed_media(&root,&app);let admission=ROOT_ADMISSION.lock().unwrap();
        let target=app.clone();let (ready_tx,ready_rx)=std::sync::mpsc::channel();let (done_tx,done_rx)=std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            ready_tx.send(()).unwrap();
            done_tx.send(runtime.block_on(media(&target, 1))).unwrap();
        });
        ready_rx.recv().unwrap();std::thread::sleep(std::time::Duration::from_millis(30));assert!(done_rx.try_recv().is_err());
        transcode::BUSY.store(true,std::sync::atomic::Ordering::SeqCst);drop(admission);
        assert_eq!(done_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(),StatusCode::CONFLICT);worker.join().unwrap();
        assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(), b"thumbnail");
        assert_eq!(
            std::fs::read(root.join("library/source.cbz")).unwrap(),
            b"original"
        );
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
    async fn party_bearer_overrides_body_and_requires_current_proof() {
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
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM identities WHERE key='different-device-body'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// Public device creation remains compatible; copied account keys and failed device inserts cannot replace a room.
    async fn party_device_compatibility_and_creation_denial_preserve_room() {
        let (root, app) = fixture();
        let (status, created) =
            created_party(&app, axum::http::HeaderMap::new(), "new-device").await;
        assert_eq!(status, StatusCode::OK);
        let code=created["code"].as_str().unwrap();let proof=uuid::Uuid::parse_str(created["host_capability"].as_str().unwrap()).unwrap();
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),"new-device"));
        assert!(!app.party.can_register(&app.party.authorize(code,None).unwrap(),"new-device"));
        assert_eq!(created_party(&app,axum::http::HeaderMap::new(),"target").await.0,StatusCode::UNAUTHORIZED);
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),"new-device"));
        app.db
            .lock()
            .unwrap()
            .execute_batch("CREATE TRIGGER reject_identity BEFORE INSERT ON identities BEGIN SELECT RAISE(ABORT,'fixture'); END")
            .unwrap();
        assert_eq!(created_party(&app,axum::http::HeaderMap::new(),"blocked-device").await.0,StatusCode::INTERNAL_SERVER_ERROR);
        assert!(app.party.can_register(&app.party.authorize(code,Some(proof)).unwrap(),"new-device"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    /// Distinguish denied/missing/self-or-last-admin/database errors without committing metadata or filesystem changes.
    async fn admin_denials_last_admin_missing_and_query_failures() {
        let (root,app)=fixture();seed_media(&root,&app);
        let empty=||Query(std::collections::HashMap::new());
        assert_eq!(delete_media(State(app.clone()),axum::http::HeaderMap::new(),empty(),Path(1)).await.status(),StatusCode::FORBIDDEN);
        assert_eq!(remove_user(State(app.clone()),axum::http::HeaderMap::new(),empty(),Path(2)).await,StatusCode::FORBIDDEN);
        assert_eq!(
            set_perm(
                State(app.clone()),
                axum::http::HeaderMap::new(),
                empty(),
                Json(serde_json::json!({"identity_id":2}))
            )
            .await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(media(&app,999).await,StatusCode::NOT_FOUND);assert_eq!(user(&app,999).await,StatusCode::NOT_FOUND);
        assert_eq!(permission(&app,serde_json::json!({"identity_id":999,"allowed_kinds":[]})).await,StatusCode::NOT_FOUND);
        assert_eq!(permission(&app,serde_json::json!({"identity_id":1,"is_admin":false})).await,StatusCode::CONFLICT);
        app.db.lock().unwrap().execute_batch("UPDATE identities SET kind='account' WHERE id=1").unwrap();
        let headers=bearer(&app,1);
        assert_eq!(remove_user(State(app.clone()),headers.clone(),empty(),Path(1)).await,StatusCode::CONFLICT);
        assert_eq!(
            set_perm(
                State(app.clone()),
                headers.clone(),
                empty(),
                Json(serde_json::json!({"identity_id":1,"is_admin":false}))
            )
            .await,
            StatusCode::CONFLICT
        );
        app.db.lock().unwrap().execute_batch("DROP TABLE user_perms").unwrap();
        assert_eq!(delete_media(State(app.clone()),headers.clone(),empty(),Path(1)).await.status(),StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(remove_user(State(app.clone()),headers.clone(),empty(),Path(2)).await,StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            set_perm(
                State(app.clone()),
                headers,
                empty(),
                Json(serde_json::json!({"identity_id":2,"allowed_kinds":null}))
            )
            .await,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(std::fs::read(owned_thumb(&root)).unwrap(), b"thumbnail");
        assert_eq!(
            std::fs::read(root.join("library/source.cbz")).unwrap(),
            b"original"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    /// Status of the permission-list, settings and ingest admin gates for one caller.
    async fn admin_gate_statuses(app: &Arc<App>, headers: axum::http::HeaderMap) -> [StatusCode; 4] {
        let empty = || Query(std::collections::HashMap::new());
        [
            list_perms(State(app.clone()), headers.clone(), empty()).await.into_response().status(),
            get_settings(State(app.clone()), headers.clone(), empty()).await.into_response().status(),
            set_setting(State(app.clone()), headers.clone(), empty(), Json(serde_json::json!({}))).await.status(),
            ingest(State(app.clone()), headers, empty(), Json(IngestReq { url: "https://example.invalid/".into() })).await,
        ]
    }
    #[tokio::test]
    /// These gates deny a real non-admin with 403 and report an unreadable grant table as 500.
    async fn admin_gates_propagate_permission_lookup_failure() {
        let (root,app)=fixture();
        let member=bearer(&app,2);
        assert_eq!(admin_gate_statuses(&app,member.clone()).await,[StatusCode::FORBIDDEN;4]);
        app.db.lock().unwrap().execute_batch("DROP TABLE user_perms").unwrap();
        assert_eq!(admin_gate_statuses(&app,member).await,[StatusCode::INTERNAL_SERVER_ERROR;4]);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    /// A token pointing at a missing canonical key fails 500 rather than creating an anonymous host or replacing a room.
    async fn party_orphan_token_key_failure_preserves_room() {
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
    fn roots_persisted_unavailable_paths_are_retained_but_writes_remain_strict() {
        let (root,app)=fixture();let missing=root.join("temporarily-offline");
        let db = app.db.lock().unwrap();
        db.execute(
            "INSERT INTO settings(key,value) VALUES('library_paths',?1)",
            [serde_json::json!([missing]).to_string()],
        )
        .unwrap();
        let stored=effective_library_config(&db,Some("")).unwrap();assert_eq!(stored.roots,vec![missing.clone()]);assert!(!stored.locked);
        let locked=effective_library_config(&db,missing.to_str()).unwrap();assert_eq!(locked.roots,vec![missing.clone()]);assert!(locked.locked);
        assert!(normalize_roots(&serde_json::json!([missing])).is_err());assert!(!missing.exists());
        for value in [
            serde_json::json!("bad"),
            serde_json::json!([""]),
            serde_json::json!([root.join("db.sqlite")]),
            serde_json::json!(["unresolvable-relative-root"]),
        ] {
            assert!(parse_roots(&value, true).is_err());
        }
        std::fs::create_dir(&missing).unwrap();assert_eq!(effective_library_config(&db,None).unwrap(),stored);
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os="linux")]
    #[test]
    /// In an unprivileged child behind a mode-000 parent, PermissionDenied keeps configured roots on reads only.
    fn roots_persisted_permission_denied_path_stays_configured() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;
        if std::env::var_os("FRAYD_TEST_ROOT_DAC").is_none() {
            let mut child=test_support::child("integration_regressions::roots_persisted_permission_denied_path_stays_configured");
            child.env("FRAYD_TEST_ROOT_DAC","1");
            // SAFETY: geteuid has no preconditions and cannot fail.
            if unsafe { libc::geteuid() }==0 {child.gid(65534).uid(65534);}
            test_support::assert_child_passes(&mut child);
            return;
        }
        struct RestoreDirectory(PathBuf);
        impl Drop for RestoreDirectory {fn drop(&mut self){let _=std::fs::set_permissions(&self.0,std::fs::Permissions::from_mode(0o700));}}
        let (root,app)=fixture();let parent=root.join("protected");let library=parent.join("library");std::fs::create_dir_all(&library).unwrap();
        let db = app.db.lock().unwrap();
        db.execute(
            "INSERT INTO settings(key,value) VALUES('library_paths',?1)",
            [serde_json::json!([library]).to_string()],
        )
        .unwrap();
        let restore=RestoreDirectory(parent.clone());std::fs::set_permissions(&parent,std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(std::fs::read_dir(&library).unwrap_err().kind(),std::io::ErrorKind::PermissionDenied);
        assert_eq!(effective_library_config(&db,None).unwrap().roots,vec![library.clone()]);
        assert_eq!(effective_library_config(&db,library.to_str()).unwrap().roots,vec![library.clone()]);
        assert!(normalize_roots(&serde_json::json!([library])).is_err());
        drop(restore);drop(db);std::fs::remove_dir_all(root).unwrap();
    }

}

#[cfg(test)]
mod stream_range_tests {
    use super::*;

    #[tokio::test]
    async fn selected_files_obey_byte_range_contracts() {
        let root=std::env::temp_dir().join(format!("frayd-range-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let source=root.join("source.mp4");
        let conversion=root.join("conversion.m4b");
        let empty=root.join("empty.mp4");
        let bytes:Vec<u8>=(0..=255).cycle().take(1024).collect();
        std::fs::write(&source,&bytes).unwrap();
        std::fs::write(&conversion,b"conversion").unwrap();
        std::fs::write(&empty,[]).unwrap();
        let db=store::open(":memory:");
        for (id, path, converted) in [
            (1, source.clone(), None),
            (2, root.join("original.mp4"), Some(conversion)),
            (3, empty, None),
            (4, root.join("missing.mp4"), None),
        ] {
            db.execute(
                "INSERT INTO media(id,kind,path,title,size_bytes,transcode_path) VALUES(?1,'movie',?2,'fixture',1,?3)",
                rusqlite::params![
                    id,
                    path.to_str(),
                    converted.as_ref().and_then(|p| p.to_str())
                ],
            )
            .unwrap();
        }
        let app=test_support::app(db,String::new(),Vec::new());
        for (range,status,content_range,expected) in [
            (None,200,None,bytes.as_slice()),
            (Some("bytes=5-9"),206,Some("bytes 5-9/1024"),&bytes[5..10]),
            (Some("bytes=5-"),206,Some("bytes 5-1023/1024"),&bytes[5..]),
            (Some("bytes=-5"),206,Some("bytes 1019-1023/1024"),&bytes[1019..]),
            (Some("bytes=1020-9999"),206,Some("bytes 1020-1023/1024"),&bytes[1020..]),
            (Some("bytes=1024-"),416,Some("bytes */1024"),&[]),
            (Some("bytes=99999-"),416,Some("bytes */1024"),&[]),
            (Some("bytes=bad"),416,Some("bytes */1024"),&[]),
            (Some("bytes=9-5"),416,Some("bytes */1024"),&[]),
            (Some("bytes=0-1,4-5"),200,None,bytes.as_slice()),
        ] {
            let mut headers=axum::http::HeaderMap::new();
            if let Some(range)=range {headers.insert(header::RANGE,range.parse().unwrap());}
            let response=stream(State(app.clone()),Path(1),axum::http::Method::GET,headers).await.into_response();
            assert_eq!(response.status().as_u16(),status,"{range:?}");
            assert_eq!(response.headers().get(header::CONTENT_RANGE).map(|v|v.to_str().unwrap()),content_range,"{range:?}");
            assert_eq!(response.headers()[header::CONTENT_TYPE],"video/mp4");
            assert_eq!(response.headers()[header::ACCEPT_RANGES],"bytes");
            if status!=416 {assert_eq!(response.headers()[header::CONTENT_LENGTH],expected.len().to_string());}
            assert_eq!(axum::body::to_bytes(response.into_body(),4096).await.unwrap().as_ref(),expected,"{range:?}");
        }
        for (id, range, status, mime, expected) in [
            (
                2,
                Some("bytes=0-3"),
                206,
                Some("audio/mp4"),
                b"conv".as_slice(),
            ),
            (3, Some("bytes=0-0"), 200, Some("video/mp4"), b"".as_slice()),
            (3, None, 200, Some("video/mp4"), b"".as_slice()),
            (4, None, 404, None, b"".as_slice()),
            (999, None, 404, None, b"".as_slice()),
        ] {
            let mut headers=axum::http::HeaderMap::new();
            if let Some(range)=range {headers.insert(header::RANGE,range.parse().unwrap());}
            let response=stream(State(app.clone()),Path(id),axum::http::Method::GET,headers).await.into_response();
            assert_eq!(response.status().as_u16(),status,"id{id}");
            assert_eq!(response.headers().get(header::CONTENT_TYPE).map(|v|v.to_str().unwrap()),mime);
            if id==3 {assert_eq!(response.headers()[header::CONTENT_LENGTH],"0");assert!(!response.headers().contains_key(header::CONTENT_RANGE));}
            assert_eq!(axum::body::to_bytes(response.into_body(),4096).await.unwrap().as_ref(),expected);
        }
        for range in [None,Some("bytes=5-9")] {
            let mut headers=axum::http::HeaderMap::new();
            if let Some(range)=range {headers.insert(header::RANGE,range.parse().unwrap());}
            let response=stream(State(app.clone()),Path(1),axum::http::Method::HEAD,headers).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_LENGTH],"1024");
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            assert!(axum::body::to_bytes(response.into_body(),4096).await.unwrap().is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod final_server_regressions {
    use super::*;
    fn fixture() -> (PathBuf,Arc<App>) {
        let root=std::env::temp_dir().join(format!("frayd-final-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("library")).unwrap();
        let db=store::open(root.join("db.sqlite").to_str().unwrap());
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','admin'); INSERT INTO user_perms(identity_id,is_admin) VALUES(1,1);").unwrap();
        let app=test_support::app(db,root.join("db.sqlite").to_string_lossy().into_owned(),vec![]);
        (root,app)
    }
    fn query() -> Query<std::collections::HashMap<String,String>> { Query(std::collections::HashMap::from([("identity".into(),"admin".into())])) }
    #[tokio::test]
    async fn settings_error_describes_effective_roots_and_preserves_failures() {
        let (root,app)=fixture();let stored=serde_json::json!(["~/Media"]).to_string();
        for (override_path,locked,has_error) in [(Some(root.join("library")),true,false),(Some(root.join("unresolved/../alias")),true,true),(None,false,true)] {
            {
                let db=app.db.lock().unwrap();
                db.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('library_paths',?1)",[&stored]).unwrap();
                *app.library.write().unwrap()=environment_library_config(&db,override_path.as_deref().map(|path|path.as_os_str()));
            }
            let response=get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            let body:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap()).unwrap();
            assert_eq!(body["library_locked"],locked);assert_eq!(body["library_paths"],serde_json::json!(["~/Media"]));
            assert_eq!(!body["library_config_error"].is_null(),has_error,"only the effective configuration may report a failure: {body}");
            if !has_error {assert_eq!(body["effective_library_paths"],serde_json::json!([root.join("library")]));}
            else {assert_eq!(body["effective_library_paths"],serde_json::json!([]));}
            assert_eq!(app.db.lock().unwrap().query_row("SELECT value FROM settings WHERE key='library_paths'",[],|row|row.get::<_,String>(0)).unwrap(),stored);
            let edit=set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"library_paths":[root.join("library")]}))).await.status();
            assert_eq!(edit,if locked {StatusCode::CONFLICT}else{StatusCode::NO_CONTENT});
        }
        // A valid override suppresses unused parse diagnostics, never a real database or permission error.
        *app.library.write().unwrap()=environment_library_config(&app.db.lock().unwrap(),Some(root.join("library").as_os_str()));
        app.db.lock().unwrap().execute("UPDATE settings SET value=zeroblob(1) WHERE key='library_paths'",[]).unwrap();
        assert_eq!(get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        app.db.lock().unwrap().execute("UPDATE settings SET value='[]' WHERE key='library_paths'",[]).unwrap();
        app.db.lock().unwrap().execute("UPDATE user_perms SET is_admin=0 WHERE identity_id=1",[]).unwrap();
        assert_eq!(get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await.into_response().status(),StatusCode::FORBIDDEN);
        app.db.lock().unwrap().execute_batch("DROP TABLE user_perms").unwrap();
        assert_eq!(get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn explicit_environment_failures_hold_locked_without_saved_root_fallback() {
        let (root,mut app)=fixture();
        let state=Arc::get_mut(&mut app).unwrap();state.download_backend=Some(DownloadBackend::Direct);state.downloads_enabled=true;
        let saved=serde_json::json!([root.join("library")]).to_string();
        app.db.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES('library_paths',?1)",[&saved]).unwrap();
        let mut overrides=vec!["unresolved-relative-root".to_string(),root.join("db.sqlite").to_string_lossy().into_owned()];
        #[cfg(unix)] {
            let loop_path=root.join("loop");std::os::unix::fs::symlink(&loop_path,&loop_path).unwrap();
            assert!(loop_path.canonicalize().is_err());
            overrides.push(loop_path.to_string_lossy().into_owned());
        }
        let mut configurations:Vec<_>=overrides.iter().map(|value|environment_library_config(&app.db.lock().unwrap(),Some(std::ffi::OsStr::new(value)))).collect();
        // Inject mount failures at the same parse-result boundary, without mounting or altering live filesystems.
        #[cfg(target_os="linux")]
        for errno in [5,107,116] { // EIO, ENOTCONN, ESTALE.
            configurations.push(explicit_library_config(Err(std::io::Error::from_raw_os_error(errno).to_string())));
        }
        #[cfg(unix)] {
            use std::os::unix::ffi::OsStrExt;
            configurations.push(environment_library_config(&app.db.lock().unwrap(),Some(std::ffi::OsStr::from_bytes(b"\xff"))));
        }
        for config in configurations {
            assert!(config.locked);assert!(config.roots.is_empty());*app.library.write().unwrap()=config;
            let response=get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            let body:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap()).unwrap();
            assert_eq!(body["library_paths"],serde_json::json!([root.join("library")]));
            assert_eq!(body["effective_library_paths"],serde_json::json!([]));assert_eq!(body["library_locked"],true);
            assert_eq!(body["library_config_error"],"Library configuration unavailable");
            assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::NOT_IMPLEMENTED);
            assert_eq!(ingest(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(IngestReq{url:"https://example.invalid/video.mp4".into()})).await,StatusCode::NOT_IMPLEMENTED);
            assert_eq!(set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"library_paths":[root.join("library")]}))).await.status(),StatusCode::CONFLICT);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT value FROM settings WHERE key='library_paths'",[],|row|row.get::<_,String>(0)).unwrap(),saved);
            assert!(!app.scanning.load(std::sync::atomic::Ordering::SeqCst));assert!(app.ingest_status.lock().unwrap().is_none());
            assert!(!root.join("library/Internet").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn final_legacy_roots_remain_administrable_and_repairable() {
        let (root,app)=fixture();
        for value in [serde_json::json!("bad"),serde_json::json!([1]),serde_json::json!(["missing-relative"]),serde_json::json!(["~/Media"]),serde_json::json!([root.join("db.sqlite")]),serde_json::json!([root.join("library"),"~/Media"]),serde_json::json!([root.join("library"),1])] {
            let raw=value.to_string();
            app.db.lock().unwrap().execute("INSERT OR REPLACE INTO settings(key,value) VALUES('library_paths',?1)",[&raw]).unwrap();
            let config=effective_library_config(&app.db.lock().unwrap(),None).expect("legacy settings must not prevent startup");
            assert!(config.roots.is_empty());
            *app.library.write().unwrap()=config;
            let response=get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await.into_response();
            assert_eq!(response.status(),StatusCode::OK);
            let bytes=axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap();
            let body:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
            assert!(body["library_paths"].as_array().unwrap().iter().all(serde_json::Value::is_string));
            assert!(body["library_config_error"].is_string());
            if !value.is_array() || value.as_array().unwrap().iter().any(|v|!v.is_string()) {assert_eq!(body["library_paths"],serde_json::json!([]));}
            else {assert_eq!(body["library_paths"],value);}
            assert_eq!(trigger_scan(State(app.clone())).await,StatusCode::NOT_IMPLEMENTED);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT value FROM settings WHERE key='library_paths'",[],|r|r.get::<_,String>(0)).unwrap(),raw);
        }
        let response=set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"library_paths":[root.join("library")]}))).await;
        assert_eq!(response.status(),StatusCode::NO_CONTENT);
        assert_eq!(library_roots(&app),vec![root.join("library")]);
        assert_eq!(set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"library_config_error":null}))).await.status(),StatusCode::BAD_REQUEST);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn final_retained_offline_root_does_not_block_other_edits() {
        let (root,app)=fixture();let offline=root.join("offline");
        app.db.lock().unwrap().execute("INSERT INTO settings(key,value) VALUES('library_paths',?1)",[serde_json::json!([offline]).to_string()]).unwrap();
        *app.library.write().unwrap()=effective_library_config(&app.db.lock().unwrap(),None).unwrap();
        let edit=serde_json::json!({"library_paths":[offline,root.join("library")]});
        assert_eq!(set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(edit)).await.status(),StatusCode::NO_CONTENT);
        assert!(!offline.exists());
        assert_eq!(set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(serde_json::json!({"library_paths":[offline,root.join("new-missing")]}))).await.status(),StatusCode::BAD_REQUEST);
        assert!(!root.join("new-missing").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn final_thumbnail_resolution_precedes_write_transaction() {
        let (root,app)=fixture();let thumbs=root.join("thumbs");std::fs::create_dir(&thumbs).unwrap();
        let thumbnail=thumbs.join(format!("41-{}.jpg",uuid::Uuid::new_v4()));std::fs::write(&thumbnail,b"owned").unwrap();
        let source=root.join("library/source.mp3");std::fs::write(&source,b"source").unwrap();
        let mut db=app.db.lock().unwrap();
        db.execute("INSERT INTO media(id,kind,path,title) VALUES(42,'music',?1,'reference')",[source.to_string_lossy()]).unwrap();
        scanner::RETIREMENT_WRITE_PROBES.with(|count|count.set(0));
        cleanup_owned_thumbnail(&mut db,&thumbs,&[root.join("library")],41,source.to_str().unwrap(),thumbnail.to_str()).unwrap();
        assert!(!thumbnail.exists());assert!(source.exists());
        scanner::RETIREMENT_WRITE_PROBES.with(|count|assert_eq!(count.get(),0,"filesystem resolution held the SQLite write transaction"));
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn final_session_database_error_never_falls_back_to_device_or_guest() {
        let (root,app)=fixture();let token=uuid::Uuid::new_v4().to_string();
        let mut headers=axum::http::HeaderMap::new();headers.insert(header::AUTHORIZATION,format!("Bearer {token}").parse().unwrap());
        {
            let db=app.db.lock().unwrap();
            assert_eq!(resolve_identity(&db,&headers,None),Err(StatusCode::UNAUTHORIZED));
            db.execute("INSERT INTO sessions(token,identity_id,expires_at) VALUES(?1,1,unixepoch()-1)",[&token]).unwrap();
            assert_eq!(resolve_identity(&db,&headers,None),Err(StatusCode::UNAUTHORIZED));
            db.execute("UPDATE sessions SET expires_at=unixepoch()+3600",[]).unwrap();
            assert_eq!(resolve_identity(&db,&headers,Some("unused-device")),Ok(1));
            db.execute_batch("CREATE TRIGGER deny_refresh BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
            assert_eq!(resolve_identity(&db,&headers,Some("new-device")),Err(StatusCode::INTERNAL_SERVER_ERROR));
            db.execute_batch("DROP TRIGGER deny_refresh; ALTER TABLE sessions RENAME TO unavailable_sessions").unwrap();
        }
        assert_eq!(resolve_identity(&app.db.lock().unwrap(),&headers,Some("new-device")),Err(StatusCode::INTERNAL_SERVER_ERROR));
        assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM identities WHERE key='new-device'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(auth_me(State(app.clone()),headers.clone()).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(library_handler(State(app.clone()),headers.clone(),Query(std::collections::HashMap::new())).await.status(),StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(list_perms(State(app.clone()),headers.clone(),query()).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(get_progress(State(app.clone()),Path(41),headers.clone(),query()).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(list_annotations(State(app.clone()),Path(41),headers.clone(),query()).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        let password=ChangePwReq {old:uuid::Uuid::new_v4().to_string(),new:uuid::Uuid::new_v4().to_string()};
        assert_eq!(auth_change_password(State(app.clone()),ConnectInfo("127.0.0.1:1".parse().unwrap()),headers.clone(),Json(password)).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        let (code,proof)=app.party.create("admin");
        assert_eq!(party_create(State(app.clone()),headers.clone(),Json(PartyCreateReq {identity:"new-device".into()})).await.into_response().status(),StatusCode::INTERNAL_SERVER_ERROR);
        assert!(app.party.can_register(&app.party.authorize(&code,Some(proof)).unwrap(),"admin"));
        app.db.lock().unwrap().execute_batch("ALTER TABLE unavailable_sessions RENAME TO sessions").unwrap();
        assert_eq!(auth_me(State(app.clone()),headers).await.into_response().status(),StatusCode::OK);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod direct_download_tests {
    use super::*;
    use std::io::{Read,Write};

    /// Bounded raw responses expose real HTTP framing, redirects and body deadlines.
    fn scripted_server(responses:Vec<(Vec<u8>,Option<std::time::Duration>)>) -> (String,std::thread::JoinHandle<()>) {
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url=format!("http://{}/owned.mp4",listener.local_addr().unwrap());
        let worker=std::thread::spawn(move || {
            for (response,body_delay) in responses {
                let (mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                let mut request=Vec::new();let mut byte=[0];
                while !request.ends_with(b"\r\n\r\n") {if stream.read(&mut byte).unwrap_or(0)==0{return;}request.push(byte[0]);assert!(request.len()<16384);}
                if let Some(delay)=body_delay {
                    let split=response.windows(4).position(|bytes|bytes==b"\r\n\r\n").unwrap()+4;
                    let _=stream.write_all(&response[..split]);std::thread::sleep(delay);let _=stream.write_all(&response[split..]);
                } else {let _=stream.write_all(&response);}
            }
        });
        (url,worker)
    }
    fn response_server(response:Vec<u8>,body_delay:Option<std::time::Duration>) -> (String,std::thread::JoinHandle<()>) {
        scripted_server(vec![(response,body_delay)])
    }
    fn response(body:&[u8],headers:&str) -> Vec<u8> {
        let mut output=format!("HTTP/1.1 200 OK\r\n{headers}Connection: close\r\n\r\n").into_bytes();output.extend_from_slice(body);output
    }

    /// Build and independently decode a small public MP4 gzip fixture using the system test tool.
    fn gzip_fixture(input:&[u8],decode:bool) -> Vec<u8> {
        let mut child=Command::new("gzip").args(if decode {vec!["-d","-c"]} else {vec!["-c"]})
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
            .spawn().expect("system gzip is required to qualify the compressed response fixture");
        let mut stdin=child.stdin.take().unwrap();let bytes=input.to_vec();
        let writer=std::thread::spawn(move ||stdin.write_all(&bytes));
        let output=child.wait_with_output().unwrap();writer.join().unwrap().unwrap();
        assert!(output.status.success(),"independent gzip fixture transformation failed");output.stdout
    }

    #[tokio::test]
    async fn direct_backend_downloads_controlled_mp4_without_a_native_tool() {
        if std::env::var_os("FRAYD_TEST_DIRECT_ROUTE").is_none() {
            let tools=std::env::temp_dir().join(format!("frayd-direct-tools-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&tools).unwrap();
            let marker=tools.join("tool-called");
            test_support::write_executable(&tools.join("yt-dlp"),"#!/bin/sh\ntouch \"$FRAYD_TEST_DIRECT_TOOL_MARKER\"\nexit 1\n");
            Command::new(tools.join("yt-dlp")).arg("--version").env("FRAYD_TEST_DIRECT_TOOL_MARKER",&marker).output().unwrap();
            assert!(marker.exists());std::fs::remove_file(&marker).unwrap();
            test_support::assert_child_passes(test_support::child("direct_download_tests::direct_backend_downloads_controlled_mp4_without_a_native_tool")
                .env("FRAYD_TEST_DIRECT_ROUTE","1").env("FRAYD_DOWNLOAD_BACKEND","direct").env("FRAYD_DOWNLOADS_ENABLED","1")
                .env("FRAYD_TEST_DIRECT_TOOL_MARKER",&marker).env("PATH",test_support::path_with(&tools)));
            assert!(!marker.exists());std::fs::remove_dir_all(tools).unwrap();return;
        }
        let root=std::env::temp_dir().join(format!("frayd-direct-route-{}",uuid::Uuid::new_v4()));
        let library=root.join("library");std::fs::create_dir_all(&library).unwrap();
        let db_path=root.join("db.sqlite");let db=store::open(db_path.to_str().unwrap());
        db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'); INSERT INTO user_perms(identity_id,is_admin) VALUES(1,1);").unwrap();
        let app=test_support::app(db,db_path.to_string_lossy().into_owned(),vec![library.clone()]);
        let settings=get_settings(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())]))).await;
        assert_eq!(settings.status(),StatusCode::OK);
        let metadata:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(settings.into_body(),usize::MAX).await.unwrap()).unwrap();
        assert_eq!(metadata["download_backend"],"direct");assert_eq!(metadata["downloads_enabled"],true);assert!(metadata["download_config_error"].is_null());
        let bytes=include_bytes!("../../tests/fixtures/native-resume.mp4");
        let (url,server)=response_server(response(bytes,&format!("Content-Encoding: identity\r\nContent-Length: {}\r\n",bytes.len())),None);
        let status=ingest(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())])),Json(IngestReq{url})).await;
        assert_eq!(status,StatusCode::ACCEPTED);
        for _ in 0..1200 {
            if matches!(app.ingest_status.lock().unwrap().as_deref(),Some("done"|"failed")) {break;}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("done"));
        test_support::scan_idle(&app).await;server.join().unwrap();
        assert_eq!(std::fs::read(library.join("Internet/owned.mp4")).unwrap(),bytes);
        let published=library.join("Internet/owned.mp4");
        let id=app.db.lock().unwrap().query_row("SELECT id FROM media WHERE path=?1",[published.to_string_lossy()],|r|r.get::<_,i64>(0)).unwrap();
        app.db.lock().unwrap().execute("INSERT INTO progress(identity_id,media_id,position) VALUES(1,?1,0.5)",[id]).unwrap();
        let original_mtime=std::fs::metadata(&published).unwrap().modified().unwrap();
        let source=library.join("preserved.txt");std::fs::write(&source,b"source preserved").unwrap();
        let cache=transcodes_dir_for(&app.db_path);std::fs::create_dir_all(&cache).unwrap();
        let neighbor=cache.join("unowned.fixture");std::fs::write(&neighbor,b"cache preserved").unwrap();
        let compressed=gzip_fixture(bytes,false);assert_eq!(gzip_fixture(&compressed,true),bytes,"compressed fixture must independently decode to the actual valid MP4");
        let mut compressed_chunks=format!("{:x}\r\n",compressed.len()).into_bytes();compressed_chunks.extend_from_slice(&compressed);compressed_chunks.extend_from_slice(b"\r\n0\r\n\r\n");
        let cases=vec![
            ("valid gzip",response(&compressed,&format!("Content-Encoding: gzip\r\nContent-Length: {}\r\n",compressed.len()))),
            ("valid gzip duplicate length",response(&compressed,&format!("Content-Encoding: gzip\r\nContent-Length: {}\r\nContent-Length: {}\r\n",compressed.len(),compressed.len()))),
            ("valid gzip chunked and length",response(&compressed_chunks,&format!("Content-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nContent-Length: {}\r\n",compressed.len()))),
            ("404",b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()),
            ("HTML",response(b"<html>not video</html>","Content-Type: text/html\r\n")),
            ("truncated",response(b"short","Content-Length: 100\r\n")),
            ("malformed chunk",response(b"Z\r\nx\r\n0\r\n\r\n","Transfer-Encoding: chunked\r\n")),
            ("declared oversize",response(b"","Content-Length: 8589934593\r\n")),
            ("no overwrite",response(bytes,&format!("Content-Length: {}\r\n",bytes.len()))),
        ];
        for (index,(label,wire)) in cases.into_iter().enumerate() {
            let (mut url,server)=response_server(wire,None);
            // A new filename prevents an old valid source collision from masking a decoded-gzip bypass.
            if label!="no overwrite" {url=url.replace("owned.mp4",&format!("negative-{index}.mp4"));}
            assert_eq!(ingest(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())])),Json(IngestReq{url})).await,StatusCode::ACCEPTED);
            for _ in 0..1200 {
                if matches!(app.ingest_status.lock().unwrap().as_deref(),Some("done"|"failed")) {break;}
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            assert_eq!(app.ingest_status.lock().unwrap().as_deref(),Some("failed"),"{label}");server.join().unwrap();
            assert_eq!(std::fs::read(&published).unwrap(),bytes);assert_eq!(std::fs::metadata(&published).unwrap().modified().unwrap(),original_mtime);
            assert_eq!(std::fs::read(&source).unwrap(),b"source preserved");assert_eq!(std::fs::read(&neighbor).unwrap(),b"cache preserved");
            assert_eq!(app.db.lock().unwrap().query_row("SELECT id FROM media WHERE path=?1",[published.to_string_lossy()],|r|r.get::<_,i64>(0)).unwrap(),id);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT position FROM progress WHERE media_id=?1",[id],|r|r.get::<_,f64>(0)).unwrap(),0.5);
            assert!(std::fs::read_dir(&cache).unwrap().all(|entry|!entry.unwrap().file_name().to_string_lossy().starts_with("ingest-")));
            assert_eq!(std::fs::read_dir(library.join("Internet")).unwrap().count(),1);
            assert_eq!(app.db.lock().unwrap().query_row("SELECT COUNT(*) FROM media",[],|r|r.get::<_,i64>(0)).unwrap(),1);
            assert!(app.scan_queue.lock().unwrap().is_empty());
            eprintln!("direct pipeline control passed: {label}");
        }
        let invalid="http://127.0.0.1/invalid\0.mp4".to_string();
        assert_eq!(ingest(State(app.clone()),axum::http::HeaderMap::new(),Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())])),Json(IngestReq{url:invalid})).await,StatusCode::BAD_REQUEST);
        for _ in 0..400 {if !transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst){break;}tokio::time::sleep(std::time::Duration::from_millis(5)).await;}
        assert!(!transcode::BUSY.load(std::sync::atomic::Ordering::SeqCst));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn direct_transport_bounds_framing_deadline_and_redirects() {
        let root=std::env::temp_dir().join(format!("frayd-direct-bounds-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let limits=DirectDownloadLimits{max_bytes:64,transfer_timeout:std::time::Duration::from_millis(150),connect_timeout:std::time::Duration::from_millis(100)};
        let wires=vec![
            response(b"short","Content-Length: 100\r\n"),
            response(b"Z\r\nx\r\n0\r\n\r\n","Transfer-Encoding: chunked\r\n"),
            response(&[1;65],""),
            response(b"","Content-Length: 65\r\n"),
            response(b"x","Content-Length: 1\r\nTransfer-Encoding: chunked\r\n"),
            response(b"x","Content-Length: 1\r\nContent-Length: 1\r\n"),
            response(b"x","Content-Length: bad\r\n"),
            response(b"x","Content-Encoding: gzip\r\n"),
            b"HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx".to_vec(),
        ];
        for (index,wire) in wires.into_iter().enumerate() {
            let staging=root.join(index.to_string());std::fs::create_dir(&staging).unwrap();
            let (url,server)=response_server(wire,None);assert!(download_direct(&url,&staging,limits).is_err(),"framing/size case {index}");server.join().unwrap();
        }
        let staging=root.join("deadline");std::fs::create_dir(&staging).unwrap();
        let (url,server)=response_server(response(b"x","Content-Length: 1\r\n"),Some(std::time::Duration::from_secs(1)));
        let deadline_limits=DirectDownloadLimits{transfer_timeout:std::time::Duration::from_millis(300),..limits};
        let start=std::time::Instant::now();assert!(download_direct(&url,&staging,deadline_limits).is_err());
        assert!(start.elapsed()<std::time::Duration::from_millis(800),"deadline must interrupt the read before the delayed body arrives");server.join().unwrap();
        let redirect=b"HTTP/1.1 302 Found\r\nLocation: /redirected.mp4\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let bytes=include_bytes!("../../tests/fixtures/native-resume.mp4");
        let staging=root.join("redirect");std::fs::create_dir(&staging).unwrap();
        let (url,server)=scripted_server(vec![(redirect,None),(response(bytes,&format!("Content-Length: {}\r\n",bytes.len())),None)]);
        download_direct(&url,&staging,DIRECT_DOWNLOAD_LIMITS).unwrap();server.join().unwrap();assert_eq!(std::fs::read(staging.join("redirected.mp4")).unwrap(),bytes);
        let staging=root.join("chunked");std::fs::create_dir(&staging).unwrap();
        let mut chunks=format!("{:x}\r\n",bytes.len()).into_bytes();chunks.extend_from_slice(bytes);chunks.extend_from_slice(b"\r\n0\r\n\r\n");
        let (url,server)=response_server(response(&chunks,"Transfer-Encoding: chunked\r\nContent-Disposition: attachment; filename=ignored.mp4\r\n"),None);
        download_direct(&url,&staging,DIRECT_DOWNLOAD_LIMITS).unwrap();server.join().unwrap();assert_eq!(std::fs::read(staging.join("owned.mp4")).unwrap(),bytes);
        let wire=b"HTTP/1.1 302 Found\r\nLocation: file:///unsupported\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let (url,server)=response_server(wire,None);assert!(download_direct(&url,&root,DIRECT_DOWNLOAD_LIMITS).is_err());server.join().unwrap();
        let loop_wire=b"HTTP/1.1 302 Found\r\nLocation: /owned.mp4\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let (url,server)=scripted_server((0..6).map(|_|(loop_wire.clone(),None)).collect());
        assert_eq!(download_direct(&url,&root,DIRECT_DOWNLOAD_LIMITS),Err("Direct redirect limit exceeded"));server.join().unwrap();
        assert!(direct_url("http://127.0.0.1/invalid\0.mp4").is_err());assert!(direct_url("file:///unsupported").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_url_names_publish_only_safe_probed_mp4_components() {
        let root=std::env::temp_dir().join(format!("frayd-direct-names-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let bytes=include_bytes!("../../tests/fixtures/native-resume.mp4");
        for (index,path) in ["owned.mp4?ignored=1#fragment","%2Fescape.mp4","%5Cescape.mp4","CON.mp4","COM1.other.mp4","audio.tmp.mp4","AUDIO.MP4","%00bad.mp4","untyped","Upper.MP4"].iter().enumerate() {
            let staging=root.join(format!("stage-{index}"));let destination=root.join(format!("destination-{index}"));std::fs::create_dir(&staging).unwrap();std::fs::create_dir(&destination).unwrap();
            let (base,server)=response_server(response(bytes,&format!("Content-Length: {}\r\n",bytes.len())),None);
            let url=base.replace("owned.mp4",path);download_direct(&url,&staging,DIRECT_DOWNLOAD_LIMITS).unwrap();server.join().unwrap();
            publish_ingest(&staging,&destination,DownloadBackend::Direct).unwrap();
            let files:Vec<_>=std::fs::read_dir(&destination).unwrap().map(|entry|entry.unwrap().path()).collect();assert_eq!(files.len(),1);
            if index==0 {assert_eq!(files[0].file_name().unwrap(),"owned.mp4");}
            else if *path=="Upper.MP4" {assert_eq!(files[0].file_name().unwrap(),"Upper.mp4");}
            else {assert!(uuid::Uuid::parse_str(files[0].file_stem().unwrap().to_str().unwrap()).is_ok());}
            assert_eq!(std::fs::read(&files[0]).unwrap(),bytes);
        }
        let private=uuid::Uuid::new_v4().to_string();
        let parsed=direct_url(&format!("http://{private}@127.0.0.1/owned.mp4?{private}#{private}")).unwrap();
        assert_eq!(direct_filename(&parsed),"owned.mp4");
        let video=root.join("not-mp4.mkv");
        assert!(Command::new("ffmpeg").args(["-v","error","-i"]).arg(root.join("destination-0/owned.mp4")).args(["-c","copy"]).arg(&video).status().unwrap().success());
        let staging=root.join("wrong-container");let destination=root.join("wrong-destination");std::fs::create_dir(&staging).unwrap();std::fs::create_dir(&destination).unwrap();
        std::fs::copy(video,staging.join("wrong.mp4")).unwrap();assert!(scanner::ingest_video_valid(&staging.join("wrong.mp4")));assert!(publish_ingest(&staging,&destination,DownloadBackend::Direct).is_err());assert!(std::fs::read_dir(&destination).unwrap().next().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn backend_configuration_is_fail_closed_and_metadata_is_read_only() {
        assert_eq!(download_backend_from_override(None),Some(DownloadBackend::YtDlp));
        assert_eq!(download_backend_from_override(Some(std::ffi::OsStr::new("direct"))),Some(DownloadBackend::Direct));
        assert_eq!(download_backend_from_override(Some(std::ffi::OsStr::new("yt-dlp"))),Some(DownloadBackend::YtDlp));
        for value in ["","DIRECT","unknown"," direct"] {assert_eq!(download_backend_from_override(Some(std::ffi::OsStr::new(value))),None);}
        #[cfg(unix)] {
            use std::os::unix::ffi::OsStringExt;
            let invalid=std::ffi::OsString::from_vec(vec![0xff]);assert_eq!(download_backend_from_override(Some(&invalid)),None);
        }
        if std::env::var_os("FRAYD_TEST_INVALID_BACKEND").is_none() {
            test_support::assert_child_passes(test_support::child("direct_download_tests::backend_configuration_is_fail_closed_and_metadata_is_read_only")
                .env("FRAYD_TEST_INVALID_BACKEND","1").env("FRAYD_DOWNLOAD_BACKEND","unknown").env("FRAYD_DOWNLOADS_ENABLED","1"));return;
        }
        let db=store::open(":memory:");db.execute_batch("INSERT INTO identities(id,kind,key) VALUES(1,'device','fixture-admin'); INSERT INTO user_perms(identity_id,is_admin) VALUES(1,1);").unwrap();
        let app=test_support::app(db,String::new(),Vec::new());assert_eq!(app.download_backend,None);assert!(!app.downloads_enabled);
        let query=||Query(std::collections::HashMap::from([("identity".into(),"fixture-admin".into())]));
        let response=get_settings(State(app.clone()),axum::http::HeaderMap::new(),query()).await;
        let value:serde_json::Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),usize::MAX).await.unwrap()).unwrap();
        assert!(value["download_backend"].is_null());assert!(value["download_config_error"].is_string());assert_eq!(value["downloads_enabled"],false);
        for body in [serde_json::json!({"download_backend":"direct"}),serde_json::json!({"download_config_error":null})] {
            assert_eq!(set_setting(State(app.clone()),axum::http::HeaderMap::new(),query(),Json(body)).await.status(),StatusCode::BAD_REQUEST);
        }
        assert_eq!(ingest(State(app),axum::http::HeaderMap::new(),query(),Json(IngestReq{url:"https://example.invalid/video".into()})).await,StatusCode::NOT_IMPLEMENTED);
    }

}
