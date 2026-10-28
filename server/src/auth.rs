use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

/// Create a new account with argon2id password hash. Fails if key exists.
pub fn create_account(db: &Connection, key: &str, password: &str) -> Result<(), String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| e.to_string())?
        .to_string();
    let exists: bool = db
        .query_row("SELECT 1 FROM identities WHERE key = ?1", [key], |r| r.get::<_, i64>(0))
        .is_ok();
    if exists {
        return Err("account exists".into());
    }
    db.execute(
        "INSERT INTO identities (kind, key, secret_hash) VALUES ('account', ?1, ?2)",
        rusqlite::params![key, hash],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Verify password for an existing account key. Ok(false) if no such account.
pub fn verify_login(db: &Connection, key: &str, password: &str) -> Result<bool, String> {
    let hash: Option<Option<String>> = db
        .query_row(
            "SELECT secret_hash FROM identities WHERE kind='account' AND key = ?1",
            [key],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .ok()
        .flatten();
    let hash = match hash {
        Some(Some(h)) => h,
        _ => return Ok(false), // no account, or no password set
    };
    let parsed = PasswordHash::new(&hash).map_err(|e| e.to_string())?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Session TTL: FRAYD_SESSION_DAYS env (default 30). Public exposure: set 1-7.
pub fn session_ttl_secs() -> i64 {
    std::env::var("FRAYD_SESSION_DAYS").ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(30)
        .clamp(1, 365) * 24 * 3600
}

/// Mint a session token for identity. 128-bit random, hex-encoded.
pub fn create_session(db: &Connection, identity_id: i64) -> Result<String, String> {
    let token = Uuid::new_v4().to_string().replace('-', "")
        + &Uuid::new_v4().to_string().replace('-', "");
    // hex of 2 uuids = 64 chars; trim to 32 bytes (64 hex chars) — fine as opaque token
    db.execute(
        "INSERT INTO sessions (token, identity_id, expires_at)
         VALUES (?1, ?2, unixepoch() + ?3)",
        rusqlite::params![token, identity_id, session_ttl_secs()],
    )
    .map_err(|e| e.to_string())?;
    Ok(token)
}

/// Sliding-window rate limiter (in-process). Key = bucket name + client ip.
pub struct RateLimiter {
    hits: std::sync::Mutex<std::collections::HashMap<String, Vec<i64>>>,
    window: i64,
    max: usize,
}

impl RateLimiter {
    pub fn new(window_secs: i64, max_hits: usize) -> Self {
        Self { hits: std::sync::Mutex::new(std::collections::HashMap::new()), window: window_secs, max: max_hits }
    }
    /// true = allowed, false = rate-limited
    pub fn check(&self, key: &str) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let mut map = self.hits.lock().unwrap();
        let v = map.entry(key.to_string()).or_default();
        v.retain(|t| *t > now - self.window);
        if v.len() >= self.max { return false; }
        v.push(now);
        true
    }
}

/// Best-effort client IP from proxy headers, else socket addr text.
pub fn client_ip(headers: &axum::http::HeaderMap, fallback: &str) -> String {
    headers.get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| fallback.to_string())
}

/// Resolve an Authorization: Bearer token to identity_id, sliding-expiry refresh.
pub fn identity_from_token(db: &Connection, token: &str) -> Option<i64> {
    let id: i64 = db
        .query_row(
            "SELECT identity_id FROM sessions WHERE token = ?1 AND expires_at > unixepoch()",
            [token],
            |r| r.get(0),
        )
        .ok()?;
    let _ = db.execute(
        "UPDATE sessions SET expires_at = unixepoch() + ?2 WHERE token = ?1",
        rusqlite::params![token, session_ttl_secs()],
    );
    Some(id)
}

/// Change password: verify old, write new argon2id hash. Returns Err on bad old pw.
pub fn change_password(db: &Connection, key: &str, old: &str, new: &str) -> Result<(), String> {
    match verify_login(db, key, old) {
        Ok(true) => {}
        _ => return Err("current password is wrong".into()),
    }
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(new.as_bytes(), &salt)
        .map_err(|e| e.to_string())?
        .to_string();
    db.execute(
        "UPDATE identities SET secret_hash = ?2 WHERE kind='account' AND key = ?1",
        rusqlite::params![key, hash],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_session(db: &Connection, token: &str) {
    let _ = db.execute("DELETE FROM sessions WHERE token = ?1", [token]);
}


