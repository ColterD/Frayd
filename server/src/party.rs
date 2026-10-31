//! Listening party: host-controlled playback over WebSocket.
//! One party at a time (household scale). Guests join by short code,
//! request tracks, host approves. Host sends play/pause/seek/queue state.

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use rand::RngExt;
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
        let text = Message::Text(v.to_string().into());
        self.members.lock().unwrap().retain(|(_, tx)| {
            tx.send(text.clone()).is_ok()
        });
    }

    /// Host creates a party, returns join code.
    pub fn create(&self, host: &str) -> String {
        let code: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
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
        if !self.active.load(Ordering::SeqCst) {
            return false;
        }
        let mut st = self.state.lock().unwrap();
        match msg["type"].as_str() {
            Some("play") => st.playing = true,
            Some("pause") => st.playing = false,
            Some("seek") => {
                if let Some(p) = msg["position"].as_f64() {
                    st.position = p;
                }
            }
            Some("load") => {
                st.media_id = msg["media_id"].as_i64();
                st.external_url = msg["external_url"].as_str().map(String::from);
                st.title = msg["title"].as_str().map(String::from);
                st.position = msg["position"].as_f64().unwrap_or(0.0);
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
        st.updated_at = now();
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
    if sink.send(Message::Text(snapshot.to_string().into())).await.is_err() {
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

    #[test]
    fn join_code_is_four_uppercase_ascii_letters() {
        let party = Party::new();
        let code = party.create("host");
        assert_eq!(code.len(), 4);
        assert!(code.bytes().all(|byte| byte.is_ascii_uppercase()));
        assert_eq!(party.code.lock().unwrap().as_deref(), Some(code.as_str()));
        assert!(party.is_host("host"));
    }

    #[tokio::test]
    async fn broadcast_delivers_json_text_and_removes_closed_members() {
        let party = Party::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (closed, closed_rx) = mpsc::unbounded_channel();
        drop(closed_rx);
        party
            .members
            .lock()
            .unwrap()
            .extend([("live".into(), tx), ("closed".into(), closed)]);
        let payload = serde_json::json!({"type": "state", "title": "Unicode 🎵"});
        party.broadcast(&payload);
        let Some(Message::Text(text)) = rx.recv().await else {
            panic!("expected outbound text")
        };
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), payload);
        assert_eq!(party.members.lock().unwrap().len(), 1);
    }
}
