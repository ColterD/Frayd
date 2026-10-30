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
    let exists = db
        .query_row("SELECT 1 FROM identities WHERE key = ?1", [key], |r| r.get::<_, i64>(0))
        .optional()
        .map_err(|e| e.to_string())?
        .is_some();
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

/// Verify password for an existing account key. Ok(false) if no such account; Err on database failure.
pub fn verify_login(db: &Connection, key: &str, password: &str) -> Result<bool, String> {
    let hash: Option<Option<String>> = db
        .query_row(
            "SELECT secret_hash FROM identities WHERE kind='account' AND key = ?1",
            [key],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
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

/// Mint a session token for identity: two random v4 UUIDs as 64 hex characters (244 random bits).
pub fn create_session(db: &Connection, identity_id: i64) -> Result<String, String> {
    let token = Uuid::new_v4().to_string().replace('-', "")
        + &Uuid::new_v4().to_string().replace('-', "");
    db.execute(
        "INSERT INTO sessions (token, identity_id, expires_at)
         VALUES (?1, ?2, unixepoch() + ?3)",
        rusqlite::params![token, identity_id, session_ttl_secs()],
    )
    .map_err(|e| e.to_string())?;
    Ok(token)
}

/// Sliding-window rate limiter (in-process). Key = bucket name + [`client_ip`].
pub struct RateLimiter {
    /// Hit times per key, and the time keys without a hit inside the window were last dropped.
    hits: std::sync::Mutex<(std::collections::HashMap<String, Vec<i64>>, i64)>,
    window: i64,
    max: usize,
}

impl RateLimiter {
    pub fn new(window_secs: i64, max_hits: usize) -> Self {
        Self { hits: std::sync::Mutex::new((std::collections::HashMap::new(), 0)), window: window_secs, max: max_hits }
    }
    /// true = allowed, false = rate-limited
    pub fn check(&self, key: &str) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.check_at(key, now)
    }
    /// `check` at `now` (Unix seconds). At most once per window, drop every key with no hit inside the
    /// window, so the map holds only keys seen in the last two windows instead of every client ever seen.
    fn check_at(&self, key: &str, now: i64) -> bool {
        let mut guard = self.hits.lock().unwrap();
        let (map, pruned_at) = &mut *guard;
        let cutoff = now - self.window;
        if now - *pruned_at >= self.window {
            map.retain(|_, hits| hits.iter().any(|t| *t > cutoff));
            *pruned_at = now;
        }
        let v = map.entry(key.to_string()).or_default();
        v.retain(|t| *t > cutoff);
        if v.len() >= self.max { return false; }
        v.push(now);
        true
    }
}

/// Rate-limit key for a request. By default the key is the TCP `peer` address and X-Forwarded-For is
/// ignored, because any client can send that header. With `trust_proxy` (FRAYD_TRUSTED_PROXY=1) the last
/// X-Forwarded-For entry replaces the peer when it parses as an IP address: the trusted proxy writes that
/// entry itself, both when it overwrites the header and when it appends to the client's value.
/// An IPv6 address keys by its /64 prefix, so one host cannot rotate source addresses inside its subnet;
/// IPv4 and IPv4-mapped IPv6 addresses key by the single IPv4 address.
pub fn client_ip(headers: &axum::http::HeaderMap, peer: std::net::IpAddr, trust_proxy: bool) -> String {
    let forwarded = headers.get_all("x-forwarded-for").iter().next_back()
        .filter(|_| trust_proxy)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok());
    match forwarded.unwrap_or(peer) {
        std::net::IpAddr::V6(address) => match address.to_ipv4_mapped() {
            Some(mapped) => mapped.to_string(),
            None => format!("{}/64", std::net::Ipv6Addr::from(u128::from(address) & (u128::MAX << 64))),
        },
        address => address.to_string(),
    }
}

/// Resolve an Authorization: Bearer token to identity_id, sliding-expiry refresh.
pub fn identity_from_token(db: &Connection, token: &str) -> rusqlite::Result<Option<i64>> {
    let id:Option<i64>=db.query_row(
        "SELECT identity_id FROM sessions WHERE token=?1 AND expires_at>unixepoch()",[token],|r|r.get(0)).optional()?;
    if id.is_some() {
        db.execute("UPDATE sessions SET expires_at=unixepoch()+?2 WHERE token=?1",
            rusqlite::params![token,session_ttl_secs()])?;
    }
    Ok(id)
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

/// Delete a session token; logout succeeds even if the row is already gone.
pub fn delete_session(db: &Connection, token: &str) {
    if let Err(e) = db.execute("DELETE FROM sessions WHERE token = ?1", [token]) {
        tracing::warn!("session delete failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::fixture_password;

    /// IPv6 peers share one key per /64; IPv4 and IPv4-mapped IPv6 peers key by the single IPv4 address.
    #[test]
    fn client_ip_groups_ipv6_by_64_prefix_and_maps_ipv4() {
        let key = |peer: &str| client_ip(&axum::http::HeaderMap::new(), peer.parse().unwrap(), false);
        assert_eq!(key("2001:db8:1:2::1"), key("2001:db8:1:2:ffff:ffff:ffff:ffff"));
        assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
        assert_eq!(key("::ffff:203.0.113.7"), key("203.0.113.7"));
        assert_ne!(key("203.0.113.7"), key("203.0.113.8"));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-forwarded-for", "2001:db8:1:2::9".parse().unwrap());
        assert_eq!(client_ip(&headers, "198.51.100.1".parse().unwrap(), true), key("2001:db8:1:2::1"));
    }

    /// Keys whose hits have all left the window are dropped; keys with a live hit stay limited.
    #[test]
    fn rate_limiter_drops_keys_without_live_hits() {
        let limiter = RateLimiter::new(300, 10);
        assert!(limiter.check_at("old", 1_000));
        for _ in 0..10 { assert!(limiter.check_at("busy", 1_200)); }
        assert!(limiter.check_at("new", 1_301));
        assert!(!limiter.check_at("busy", 1_301));
        let mut keys: Vec<String> = limiter.hits.lock().unwrap().0.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["busy", "new"]);
    }

    /// Move the rows to `identities_rows` behind an `identities` view whose key lookups fail
    /// while inserts still reach the rows.
    fn break_key_lookup(db: &Connection) {
        db.execute_batch(
            "INSERT INTO identities(kind,key) VALUES('account','lookup-poison');
             ALTER TABLE identities RENAME TO identities_rows;
             CREATE VIEW identities AS SELECT id, kind, CASE WHEN key='lookup-poison' THEN json(key) ELSE key END AS key,
                 secret_hash FROM identities_rows;
             CREATE TRIGGER identities_insert INSTEAD OF INSERT ON identities BEGIN
                 INSERT INTO identities_rows(kind,key,secret_hash) VALUES(NEW.kind,NEW.key,NEW.secret_hash); END;",
        ).unwrap();
    }

    /// A failed existence check refuses account creation instead of inserting a possible duplicate.
    #[test]
    fn create_account_lookup_failure_inserts_nothing() {
        let db = crate::store::open(":memory:");
        break_key_lookup(&db);
        assert!(create_account(&db, "new@example.invalid", &fixture_password()).is_err());
        let created: i64 = db.query_row("SELECT COUNT(*) FROM identities_rows WHERE key='new@example.invalid'", [], |r| r.get(0)).unwrap();
        assert_eq!(created, 0);
    }

    /// A failed account lookup is a login error, not "no such account".
    #[test]
    fn verify_login_lookup_failure_is_an_error() {
        let db = crate::store::open(":memory:");
        break_key_lookup(&db);
        assert!(verify_login(&db, "new@example.invalid", &fixture_password()).is_err());
    }
}
