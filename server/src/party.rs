//! Listening party: host-controlled playback over WebSocket.
//! One party at a time (household scale). Guests join by short code,
//! request tracks, host approves. Host sends play/pause/seek/queue state.

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc;

#[derive(Default)]
pub struct Party {
    /// 4-char join code; None = no party active.
    pub code: StdMutex<Option<String>>,
    /// Host identity key.
    pub host: StdMutex<Option<String>>,
    /// Names of connected guests (for host UI).
    pub guests: StdMutex<HashSet<String>>,
    /// Playback state broadcast to everyone.
    pub state: StdMutex<PlaybackState>,
    pub active: AtomicBool,
    /// Live member sockets: (key, tx) — host + guests, for fan-out.
    pub members: StdMutex<Vec<(String, Tx)>>,
}

impl Party {
    /// Names: host first, then guests. For members-list UI.
    pub fn member_names(&self, host: &str) -> Vec<String> {
        let mut v = vec![format!("{host} (host)")];
        v.extend(self.guests.lock().unwrap().iter().cloned());
        v
    }
}

#[derive(Default, Clone, serde::Serialize)]
pub struct PlaybackState {
    pub media_id: Option<i64>,
    /// External video (e.g. YouTube id) — takes precedence over media_id when set.
    pub external_url: Option<String>,
    pub title: Option<String>,
    pub position: f64,
    pub playing: bool,
    /// Queue of approved media ids (after current).
    pub queue: Vec<QueueItem>,
    /// Server unix-sec at last state change — guests use it for live drift.
    pub updated_at: i64,
}

#[derive(Clone, serde::Serialize)]
pub struct QueueItem {
    pub media_id: Option<i64>,
    pub external_url: Option<String>,
    pub title: String,
    pub requested_by: String,
}

/// Outbound message envelope to a member socket.
pub type Tx = mpsc::UnboundedSender<Message>;

impl Party {
    pub fn new() -> Self {
        Self {
            code: StdMutex::new(None),
            host: StdMutex::new(None),
            guests: StdMutex::new(HashSet::new()),
            state: StdMutex::new(PlaybackState {
                updated_at: now(),
                ..Default::default()
            }),
            active: AtomicBool::new(false),
            members: StdMutex::new(Vec::new()),
        }
    }

    /// Broadcast JSON to every live member.
    pub fn broadcast(&self, v: &Value) {
        let text = Message::Text(v.to_string());
        self.members.lock().unwrap().retain(|(_, tx)| {
            tx.send(text.clone()).is_ok()
        });
    }

    /// Host creates a party, returns join code.
    pub fn create(&self, host: &str) -> String {
        let code: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .filter(|c| c.is_ascii_alphabetic())
            .take(4)
            .map(char::from)
            .collect::<String>()
            .to_uppercase();
        *self.code.lock().unwrap() = Some(code.clone());
        *self.host.lock().unwrap() = Some(host.to_string());
        self.active.store(true, Ordering::SeqCst);
        code
    }

    pub fn is_host(&self, key: &str) -> bool {
        self.host.lock().unwrap().as_deref() == Some(key)
    }

    /// Host-only state mutation, then broadcast.
    pub fn host_update(&self, msg: &Value) -> bool {
        self.host_update_at(msg, now())
    }

    fn host_update_at(&self, msg: &Value, timestamp: i64) -> bool {
        if !self.active.load(Ordering::SeqCst) {
            return false;
        }
        let action = msg["type"].as_str();
        if !matches!(action, Some("play" | "pause" | "seek" | "load" | "approve" | "skip")) {
            return false;
        }
        let position = msg["position"].as_f64().filter(|p| p.is_finite() && *p >= 0.0);
        let mut st = self.state.lock().unwrap();
        // Preserve elapsed playback before replacing the timeline's reference time.
        if st.playing {
            st.position += timestamp.saturating_sub(st.updated_at).max(0) as f64;
        }
        match action {
            Some("play") => {
                if let Some(p) = position {
                    st.position = p;
                }
                st.playing = true;
            }
            Some("pause") => {
                if let Some(p) = position {
                    st.position = p;
                }
                st.playing = false;
            }
            Some("seek") => {
                if let Some(p) = position {
                    st.position = p;
                }
            }
            Some("load") => {
                st.media_id = msg["media_id"].as_i64();
                st.external_url = msg["external_url"].as_str().map(String::from);
                st.title = msg["title"].as_str().map(String::from);
                st.position = position.unwrap_or(0.0);
                st.playing = true;
            }
            Some("approve") => {
                if let Some(title) = msg["title"].as_str() {
                    st.queue.push(QueueItem {
                        media_id: msg["media_id"].as_i64(),
                        external_url: msg["external_url"].as_str().map(String::from),
                        title: title.to_string(),
                        requested_by: msg["by"].as_str().unwrap_or("guest").to_string(),
                    });
                }
            }
            Some("skip") => {
                // host advances: caller loads next via /api/library
                if !st.queue.is_empty() {
                    let next = st.queue.remove(0);
                    st.media_id = next.media_id;
                    st.external_url = next.external_url;
                    st.title = Some(next.title);
                    st.position = 0.0;
                    st.playing = true;
                } else {
                    st.media_id = None;
                    st.external_url = None;
                }
            }
            _ => return false,
        }
        st.updated_at = timestamp;
        true
    }

    pub fn end(&self) {
        self.active.store(false, Ordering::SeqCst);
        *self.code.lock().unwrap() = None;
        *self.host.lock().unwrap() = None;
        self.guests.lock().unwrap().clear();
        self.members.lock().unwrap().clear();
        *self.state.lock().unwrap() = PlaybackState {
            updated_at: now(),
            ..Default::default()
        };
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// WS handler: member loop. Host controls state; guests request tracks.
/// All state changes broadcast to every member via the registry.
pub async fn member_socket(party: Arc<Party>, socket: WebSocket, key: String, is_host: bool) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    party.members.lock().unwrap().push((key.clone(), tx.clone()));
    if is_host {
        // rejoining host replaces any stale entry
        party.members.lock().unwrap().retain(|(k, _)| k != &key || true);
    } else {
        party.guests.lock().unwrap().insert(key.clone());
    }

    // initial state snapshot
    let snapshot = {
        let st = party.state.lock().unwrap();
        serde_json::json!({ "type": "state", "state": *st, "is_host": is_host })
    };
    if sink.send(Message::Text(snapshot.to_string())).await.is_err() {
        return;
    }
    if !is_host {
        let names = party.member_names(party.host.lock().unwrap().as_deref().unwrap_or("host"));
        party.broadcast(&serde_json::json!({ "type": "members", "members": names }));
    }

    // outbound pump: server→client
    let out_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // inbound: client→server
    while let Some(Ok(msg)) = stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        match v["type"].as_str() {
            // guest asks for a track → relay to host + show as pending on host UI
            Some("request") if !is_host => {
                let req = serde_json::json!({
                    "type": "request", "media_id": v["media_id"], "by": key,
                    "title": v["title"],
                });
                party.broadcast(&req);
            }
            // host control → mutate party state, broadcast new state to all
            Some(t) if is_host && matches!(t, "play" | "pause" | "seek" | "load" | "approve" | "skip") => {
                if party.host_update(&v) {
                    let st = party.state.lock().unwrap();
                    party.broadcast(&serde_json::json!({ "type": "state", "state": *st }));
                }
            }
            _ => {}
        }
    }

    // cleanup
    party.members.lock().unwrap().retain(|(k, _)| k != &key);
    if is_host {
        party.broadcast(&serde_json::json!({ "type": "ended" }));
        party.end();
    } else {
        party.guests.lock().unwrap().remove(&key);
        let names = party.member_names(party.host.lock().unwrap().as_deref().unwrap_or("host"));
        party.broadcast(&serde_json::json!({ "type": "members", "members": names }));
    }
    out_task.abort();
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn playing_party() -> Party {
        let party = Party::new();
        party.active.store(true, Ordering::SeqCst);
        *party.state.lock().unwrap() = PlaybackState {
            media_id: Some(1),
            position: 10.0,
            playing: true,
            updated_at: 100,
            ..Default::default()
        };
        party
    }

    #[test]
    fn play_pause_use_supplied_position_and_legacy_commands_preserve_elapsed_time() {
        let party = playing_party();
        assert!(party.host_update_at(&json!({"type": "pause", "position": 47.5}), 120));
        let paused = party.state.lock().unwrap().clone();
        assert_eq!(paused.position, 47.5);
        assert!(!paused.playing);
        assert!(party.host_update_at(&json!({"type": "play"}), 130));
        assert_eq!(party.state.lock().unwrap().position, 47.5);
        assert!(party.host_update_at(&json!({"type": "pause"}), 140));
        assert_eq!(party.state.lock().unwrap().position, 57.5);
        assert!(party.host_update_at(&json!({"type": "play", "position": 72.0}), 150));
        assert_eq!(party.state.lock().unwrap().position, 72.0);
    }

    #[test]
    fn approve_and_empty_skip_materialize_the_running_timeline() {
        let party = playing_party();
        assert!(party.host_update_at(&json!({"type": "approve", "media_id": 2, "title": "Next"}), 120));
        assert_eq!(party.state.lock().unwrap().position, 30.0);
        assert_eq!(party.state.lock().unwrap().updated_at, 120);
        assert!(party.host_update_at(&json!({"type": "skip"}), 125));
        let next = party.state.lock().unwrap().clone();
        assert_eq!(next.media_id, Some(2));
        assert_eq!(next.position, 0.0);
        assert!(party.host_update_at(&json!({"type": "skip"}), 135));
        let empty = party.state.lock().unwrap().clone();
        assert_eq!(empty.position, 10.0);
        assert_eq!(empty.updated_at, 135);
        assert_eq!(empty.media_id, None);
    }

    #[test]
    fn invalid_positions_fall_back_without_rewinding_and_load_resets() {
        for position in [json!(-1), json!("NaN"), Value::Null] {
            let party = playing_party();
            assert!(party.host_update_at(&json!({"type": "pause", "position": position}), 120));
            assert_eq!(party.state.lock().unwrap().position, 30.0);
        }
        let party = playing_party();
        let before = serde_json::to_value(&*party.state.lock().unwrap()).unwrap();
        assert!(!party.host_update_at(&json!({"type": "unknown"}), 120));
        assert_eq!(serde_json::to_value(&*party.state.lock().unwrap()).unwrap(), before);
        assert!(party.host_update_at(&json!({"type": "pause"}), 125));
        assert_eq!(party.state.lock().unwrap().position, 35.0);
        assert!(party.host_update_at(&json!({"type": "load", "media_id": 3}), 130));
        assert_eq!(party.state.lock().unwrap().position, 0.0);
    }
}
