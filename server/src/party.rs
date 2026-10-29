//! Listening party: host-controlled playback over WebSocket.
//! One active party per household; every replacement starts a fresh generation.

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::Value;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc;

#[derive(Default)]
pub struct Party {
    // Serializes generation transitions with authorization, registration, frames and cleanup.
    // All operations under this lock are synchronous; no guard crosses an await.
    session: StdMutex<Session>,
}

#[derive(Default)]
struct Session {
    generation: Arc<()>,
    code: Option<String>,
    host: Option<String>,
    host_capability: Option<uuid::Uuid>,
    state: PlaybackState,
    members: Vec<Member>,
    guest_ordinal: u64,
}

struct Member {
    key: String,
    public_label: String,
    socket: Arc<()>,
    tx: Tx,
}

/// Authorization captured before upgrade and checked again when the socket registers.
pub struct Admission {
    generation: Arc<()>,
    host_proof: Option<uuid::Uuid>,
}

struct Connection {
    admission: Admission,
    key: String,
    is_host: bool,
    public_label: String,
    socket: Arc<()>,
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
    /// Starts with no active generation or connected members.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the old party atomically and returns its public code and private creator proof.
    /// The proof belongs only in the create response and is discarded with this generation.
    pub fn create(&self, host: &str) -> (String, uuid::Uuid) {
        let mut session = self.session.lock().unwrap();
        let code = loop {
            let code = rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .filter(|c| c.is_ascii_alphabetic())
                .take(4)
                .map(char::from)
                .collect::<String>()
                .to_uppercase();
            if session.code.as_ref() != Some(&code) {
                break code;
            }
        };
        let host_capability = uuid::Uuid::new_v4();
        session.broadcast(&serde_json::json!({"type": "ended"}));
        *session = Session {
            code: Some(code.clone()),
            host: Some(host.to_string()),
            host_capability: Some(host_capability),
            state: PlaybackState { updated_at: now(), ..Default::default() },
            ..Default::default()
        };
        (code, host_capability)
    }

    /// Captures the matching generation before identity resolution and HTTP upgrade.
    /// Keeping this token alive prevents allocation reuse from reviving stale admissions.
    pub fn authorize(&self, code: &str, host_proof: Option<uuid::Uuid>) -> Option<Admission> {
        let session = self.session.lock().unwrap();
        if !session.code.as_ref()?.eq_ignore_ascii_case(code) {
            return None;
        }
        Some(Admission {
            generation: session.generation.clone(),
            host_proof,
        })
    }

    /// Rejects unproven host keys and occupied guest keys before changing membership.
    /// Registration repeats this check under the transition lock after upgrade.
    pub fn can_register(&self, admission: &Admission, key: &str) -> bool {
        self.session.lock().unwrap().role(admission, key).is_some()
    }

    /// Rejects upgrades whose generation retired while identity resolution was in flight.
    /// Role and guest occupancy are checked under the same lock before any replacement.
    fn register(&self, admission: Admission, key: String, tx: Tx) -> Option<Connection> {
        let mut session = self.session.lock().unwrap();
        let is_host = session.role(&admission, &key)?;
        // Labels are display-only and never participate in role or ownership checks.
        let public_label = if is_host {
            "Host".to_string()
        } else {
            session.guest_ordinal = session.guest_ordinal.checked_add(1)?;
            format!("Guest {}", session.guest_ordinal)
        };
        let connection = Connection {
            admission,
            is_host,
            public_label,
            key,
            socket: Arc::new(()),
        };
        // Only proven hosts or retired guests can be replaced; older callbacks lose authority.
        session.members.retain(|m| m.key != connection.key);
        session.members.push(Member {
            key: connection.key.clone(), public_label: connection.public_label.clone(), socket: connection.socket.clone(), tx: tx.clone(),
        });
        let _ = tx.send(Message::Text(serde_json::json!({
            "type": "state", "state": session.state, "is_host": connection.is_host,
            "self_label": connection.public_label,
        }).to_string()));
        session.broadcast_members();
        Some(connection)
    }

    /// Checks both generation and socket ownership before relaying or mutating any frame.
    fn receive(&self, connection: &Connection, value: &Value) {
        let mut session = self.session.lock().unwrap();
        if !session.contains(connection) {
            return;
        }
        if connection.is_host {
            if session.host_update_at(value, now()) {
                session.broadcast(&serde_json::json!({"type": "state", "state": session.state}));
            }
        } else if value["type"] == "request" {
            session.broadcast(&serde_json::json!({
                "type": "request", "media_id": value["media_id"],
                "by": connection.public_label, "title": value["title"],
            }));
        }
    }

    /// Only the currently registered socket can remove membership or end its generation.
    fn disconnect(&self, connection: &Connection) {
        let mut session = self.session.lock().unwrap();
        if !session.contains(connection) {
            return;
        }
        if connection.is_host {
            session.broadcast(&serde_json::json!({"type": "ended"}));
            *session = Session::default();
        } else {
            session.members.retain(|m| !Arc::ptr_eq(&m.socket, &connection.socket));
            session.broadcast_members();
        }
    }
}

impl Session {
    /// A public identity key alone never grants host authority, even before its first socket.
    /// Proof is accepted only for the creator key in this exact live generation.
    /// Guest labels cannot evict a live outbound receiver. Rejoin may fail until that
    /// receiver retires; this occupancy guard does not authenticate first-slot guests.
    fn role(&self, admission: &Admission, key: &str) -> Option<bool> {
        if self.code.is_none() || !Arc::ptr_eq(&self.generation, &admission.generation) {
            return None;
        }
        let is_host = self.host.as_deref() == Some(key);
        if is_host {
            if admission.host_proof.is_none() || admission.host_proof != self.host_capability {
                return None;
            }
        } else if admission.host_proof.is_some()
            || self.members.iter().any(|member| member.key == key && !member.tx.is_closed())
        {
            return None;
        }
        Some(is_host)
    }

    /// A reused join code or identity key never gives an old socket authority.
    fn contains(&self, connection: &Connection) -> bool {
        Arc::ptr_eq(&self.generation, &connection.admission.generation)
            && self.members.iter().any(|m| Arc::ptr_eq(&m.socket, &connection.socket))
    }

    /// Enqueues synchronously while the caller holds the session transition lock.
    fn broadcast(&self, value: &Value) {
        let text = Message::Text(value.to_string());
        for member in &self.members {
            let _ = member.tx.send(text.clone());
        }
    }

    /// Public labels are independent of the canonical keys used for authorization.
    fn broadcast_members(&self) {
        let mut names = vec!["Host (host)".to_string()];
        names.extend(self.members.iter().filter(|m| Some(&m.key) != self.host.as_ref()).map(|m| m.public_label.clone()));
        self.broadcast(&serde_json::json!({"type": "members", "members": names}));
    }

    /// Keep departed request attribution without retaining retired identity keys.
    /// Only labels issued in this generation can be echoed into state snapshots.
    fn approval_label<'a>(&self, value: &'a Value) -> &'a str {
        let label = value.as_str().unwrap_or("");
        if label == "Host" {
            return label;
        }
        if let Some(ordinal) = label.strip_prefix("Guest ").and_then(|n| n.parse::<u64>().ok()) {
            if ordinal > 0 && ordinal <= self.guest_ordinal && label == format!("Guest {ordinal}") {
                return label;
            }
        }
        "Guest"
    }

    /// Applies a host command while preserving the existing monotonic playback timeline.
    fn host_update_at(&mut self, msg: &Value, timestamp: i64) -> bool {
        if self.code.is_none() {
            return false;
        }
        let action = msg["type"].as_str();
        if !matches!(action, Some("play" | "pause" | "seek" | "load" | "approve" | "skip")) {
            return false;
        }
        let position = msg["position"].as_f64().filter(|p| p.is_finite() && *p >= 0.0);
        let requested_by = self.approval_label(&msg["by"]);
        let st = &mut self.state;
        // Clamp out-of-order timestamps to the current timeline reference.
        let timestamp = timestamp.max(st.updated_at);
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
                        requested_by: requested_by.to_string(),
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

}

/// Unix seconds used as the playback timeline reference.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Invalid or ambiguous capability offers carry no sensitive error payload.
#[derive(Debug)]
pub struct InvalidHostProof;

/// Parses a single canonical capability offer without accepting aliases or silent fallback.
/// Only the public protocol is negotiated; capability offers never enter URLs or envelopes.
pub fn host_proof(headers: &axum::http::HeaderMap) -> Result<Option<uuid::Uuid>, InvalidHostProof> {
    let mut proof = None;
    let mut public_protocol = false;
    for header in headers.get_all("sec-websocket-protocol") {
        let value = header.to_str().map_err(|_| InvalidHostProof)?;
        for protocol in value.split(',').map(str::trim) {
            if protocol == "frayd-party" && !public_protocol {
                public_protocol = true;
            } else if let Some(value) = protocol.strip_prefix("frayd-host.") {
                let parsed = uuid::Uuid::parse_str(value).map_err(|_| InvalidHostProof)?;
                if proof.is_some() || parsed.get_version_num() != 4
                    || parsed.get_variant() != uuid::Variant::RFC4122 || parsed.to_string() != value {
                    return Err(InvalidHostProof);
                }
                proof = Some(parsed);
            } else {
                return Err(InvalidHostProof);
            }
        }
    }
    if proof.is_some() && !public_protocol {
        return Err(InvalidHostProof);
    }
    Ok(proof)
}

/// Every frame and disconnect is scoped to the registered generation and socket.
pub async fn member_socket(party: Arc<Party>, socket: WebSocket, admission: Admission, key: String) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let Some(connection) = party.register(admission, key, tx) else { return };
    let mut out_task = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });
    loop {
        // Retiring membership drops its sender and terminates the old socket as well.
        tokio::select! {
            _ = &mut out_task => break,
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            party.receive(&connection, &value);
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }
        }
    }
    party.disconnect(&connection);
    out_task.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Seeds a deterministic timeline without depending on wall-clock progress.
    fn playing_party() -> Party {
        let party = Party::new();
        party.create("host");
        party.session.lock().unwrap().state = PlaybackState {
            media_id: Some(1),
            position: 10.0,
            playing: true,
            updated_at: 100,
            ..Default::default()
        };
        party
    }

    #[test]
    fn out_of_order_timestamps_do_not_double_count_elapsed_playback() {
        let party = playing_party();
        let mut timeline = Vec::new();
        for timestamp in [120, 110, 120, 130] {
            assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "approve", "title": "Next"}), timestamp));
            let session = party.session.lock().unwrap();
            let state = &session.state;
            timeline.push((state.position, state.updated_at));
        }
        assert_eq!(timeline, [(30.0, 120), (30.0, 120), (30.0, 120), (40.0, 130)]);
        assert_eq!(party.session.lock().unwrap().state.queue.len(), 4);
    }

    #[test]
    fn stale_pause_play_and_seek_preserve_positions_and_monotonic_time() {
        let party = playing_party();
        for (message, timestamp, expected_position, expected_playing, expected_time) in [
            (json!({"type": "pause"}), 120, 30.0, false, 120),
            (json!({"type": "approve", "title": "Next"}), 110, 30.0, false, 120),
            (json!({"type": "play"}), 115, 30.0, true, 120),
            (json!({"type": "seek", "position": 5.0}), 119, 5.0, true, 120),
            (json!({"type": "pause"}), 130, 15.0, false, 130),
            (json!({"type": "play", "position": 2.0}), 125, 2.0, true, 130),
            (json!({"type": "pause", "position": 1.0}), 129, 1.0, false, 130),
            (json!({"type": "play"}), 140, 1.0, true, 140),
            (json!({"type": "load", "media_id": 3}), 135, 0.0, true, 140),
            (json!({"type": "skip"}), 139, 0.0, true, 140),
            (json!({"type": "pause"}), 150, 10.0, false, 150),
        ] {
            assert!(party.session.lock().unwrap().host_update_at(&message, timestamp));
            let session = party.session.lock().unwrap();
            let state = &session.state;
            assert_eq!((state.position, state.playing, state.updated_at),
                (expected_position, expected_playing, expected_time), "{message} at {timestamp}");
        }
    }

    #[test]
    fn play_pause_use_supplied_position_and_legacy_commands_preserve_elapsed_time() {
        let party = playing_party();
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "pause", "position": 47.5}), 120));
        let paused = party.session.lock().unwrap().state.clone();
        assert_eq!(paused.position, 47.5);
        assert!(!paused.playing);
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "play"}), 130));
        assert_eq!(party.session.lock().unwrap().state.position, 47.5);
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "pause"}), 140));
        assert_eq!(party.session.lock().unwrap().state.position, 57.5);
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "play", "position": 72.0}), 150));
        assert_eq!(party.session.lock().unwrap().state.position, 72.0);
    }

    #[test]
    fn approve_and_empty_skip_materialize_the_running_timeline() {
        let party = playing_party();
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "approve", "media_id": 2, "title": "Next"}), 120));
        assert_eq!(party.session.lock().unwrap().state.position, 30.0);
        assert_eq!(party.session.lock().unwrap().state.updated_at, 120);
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "skip"}), 125));
        let next = party.session.lock().unwrap().state.clone();
        assert_eq!(next.media_id, Some(2));
        assert_eq!(next.position, 0.0);
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "skip"}), 135));
        let empty = party.session.lock().unwrap().state.clone();
        assert_eq!(empty.position, 10.0);
        assert_eq!(empty.updated_at, 135);
        assert_eq!(empty.media_id, None);
    }

    #[test]
    fn invalid_positions_fall_back_without_rewinding_and_load_resets() {
        for position in [json!(-1), json!("NaN"), Value::Null] {
            let party = playing_party();
            assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "pause", "position": position}), 120));
            assert_eq!(party.session.lock().unwrap().state.position, 30.0);
        }
        let party = playing_party();
        let before = serde_json::to_value(&party.session.lock().unwrap().state).unwrap();
        assert!(!party.session.lock().unwrap().host_update_at(&json!({"type": "unknown"}), 120));
        assert_eq!(serde_json::to_value(&party.session.lock().unwrap().state).unwrap(), before);
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "pause"}), 125));
        assert_eq!(party.session.lock().unwrap().state.position, 35.0);
        assert!(party.session.lock().unwrap().host_update_at(&json!({"type": "load", "media_id": 3}), 130));
        assert_eq!(party.session.lock().unwrap().state.position, 0.0);
    }

    /// Registers a socket channel so tests exercise the production ownership checks.
    fn connect(party: &Party, code: &str, key: &str) -> (Connection, mpsc::UnboundedReceiver<Message>) {
        let proof = {
            let session = party.session.lock().unwrap();
            if session.host.as_deref() == Some(key) { session.host_capability } else { None }
        };
        let admission = party.authorize(code, proof).unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        (party.register(admission, key.to_string(), tx).unwrap(), rx)
    }

    /// Collects queued protocol envelopes without starting an outbound socket task.
    fn drain(rx: &mut mpsc::UnboundedReceiver<Message>) -> Vec<Value> {
        let mut messages = Vec::new();
        while let Ok(Message::Text(text)) = rx.try_recv() {
            messages.push(serde_json::from_str(&text).unwrap());
        }
        messages
    }

    #[test]
    fn public_labels_cover_members_requests_queue_and_late_join_without_identity_keys() {
        let party = Party::new();
        let private_keys = ["fixture-host-key@example.invalid", "fixture-guest-key@example.invalid", "fixture-late-key@example.invalid"];
        let (code, _) = party.create(private_keys[0]);
        let (host, mut host_rx) = connect(&party, &code, private_keys[0]);
        let (guest, mut guest_rx) = connect(&party, &code, private_keys[1]);
        let mut outbound = drain(&mut host_rx);
        assert!(outbound.iter().any(|v| v["self_label"] == "Host"));
        let initial = drain(&mut guest_rx);
        assert!(initial.iter().any(|v| v["self_label"] == "Guest 1"));
        assert!(initial.iter().any(|v| v["members"] == json!(["Host (host)", "Guest 1"])));
        outbound.extend(initial);
        party.receive(&guest, &json!({"type":"request", "media_id":2, "title":"Next", "by": private_keys[0]}));
        for rx in [&mut host_rx, &mut guest_rx] {
            let messages = drain(rx);
            assert!(messages.iter().any(|v| v["type"] == "request" && v["by"] == "Guest 1"));
            outbound.extend(messages);
        }
        party.disconnect(&guest);
        party.receive(&host, &json!({"type":"approve", "media_id":2, "title":"Next", "by":"Guest 1"}));
        outbound.extend(drain(&mut host_rx));
        let (_late, mut late_rx) = connect(&party, &code, private_keys[2]);
        let initial = drain(&mut late_rx);
        assert!(initial.iter().any(|v| v["self_label"] == "Guest 2" && v["state"]["queue"][0]["requested_by"] == "Guest 1"));
        outbound.extend(initial);
        outbound.extend(drain(&mut host_rx));
        let serialized = serde_json::to_string(&outbound).unwrap();
        for key in private_keys {
            assert!(!serialized.contains(key), "outbound protocol exposed a canonical identity");
        }
    }

    #[test]
    fn approval_attribution_accepts_only_canonical_issued_display_labels() {
        let party = Party::new();
        let (code, _) = party.create("fixture-host-key@example.invalid");
        let (host, mut host_rx) = connect(&party, &code, "fixture-host-key@example.invalid");
        let (guest, _guest_rx) = connect(&party, &code, "fixture-guest-key@example.invalid");
        party.disconnect(&guest);
        for by in [json!("Host"), json!("Guest 1"), json!("Guest 0"), json!("Guest 2"),
            json!("Guest 01"), json!("Guest +1"), json!("Guest 1 "), json!("guest 1"),
            json!("fixture-guest-key@example.invalid"), json!("fixture-host-key@example.invalid"),
            json!("Guest 18446744073709551616"), json!(1), Value::Null] {
            party.receive(&host, &json!({"type":"approve", "title":"Next", "by":by}));
        }
        let messages = drain(&mut host_rx);
        let queue = messages.last().unwrap()["state"]["queue"].as_array().unwrap();
        assert_eq!(queue.len(), 13);
        assert!(queue[0]["requested_by"] == "Host");
        assert!(queue[1]["requested_by"] == "Guest 1");
        assert!(queue[2..].iter().all(|item| item["requested_by"] == "Guest"));
    }

    #[test]
    fn labels_follow_registration_lifecycle_without_changing_private_authority() {
        let party = Party::new();
        let (code, _) = party.create("private-host");
        let (old_host, mut old_host_rx) = connect(&party, &code, "private-host");
        let (_old_guest, old_guest_rx) = connect(&party, &code, "private-guest");
        drop(old_guest_rx);
        let (_guest, mut guest_rx) = connect(&party, &code, "private-guest");
        assert!(drain(&mut guest_rx).iter().any(|v| v["self_label"] == "Guest 2"));
        let (host, mut host_rx) = connect(&party, &code, "private-host");
        assert!(drain(&mut host_rx).iter().any(|v| v["self_label"] == "Host"));
        party.disconnect(&old_host);
        assert!(party.session.lock().unwrap().contains(&host));
        for label in ["Host", "Guest 1", "Guest 2"] {
            let admission = party.authorize(&code, None).unwrap();
            assert!(party.can_register(&admission, label));
        }
        party.disconnect(&host);
        assert!(drain(&mut guest_rx).iter().any(|v| v["type"] == "ended"));
        drain(&mut old_host_rx);
        let (next_code, _) = party.create("private-host");
        let (_next, mut next_rx) = connect(&party, &next_code, "private-guest");
        assert!(drain(&mut next_rx).iter().any(|v| v["self_label"] == "Guest 1"));
        let (replacement_code, _) = party.create("private-host");
        let (_replacement, mut replacement_rx) = connect(&party, &replacement_code, "private-guest");
        assert!(drain(&mut replacement_rx).iter().any(|v| v["self_label"] == "Guest 1"));
    }

    #[test]
    fn replacement_rejects_stale_registration_even_when_code_and_host_are_reused() {
        let party = Party::new();
        let (code, _) = party.create("host");
        let proof = party.session.lock().unwrap().host_capability;
        let delayed = party.authorize(&code.to_lowercase(), proof).unwrap();
        party.create("host");
        // Force a short-code collision: only the generation conveys authority.
        party.session.lock().unwrap().code = Some(code.clone());
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(party.register(delayed, "host".into(), tx).is_none());
        assert!(party.authorize("missing", None).is_none());
        let (host, _) = connect(&party, &code, "host");
        party.receive(&host, &json!({"type":"load", "title":"current"}));
        assert_eq!(party.session.lock().unwrap().state.title.as_deref(), Some("current"));
    }

    #[test]
    fn stale_host_frames_and_guest_cleanup_cannot_touch_replacement() {
        let party = Party::new();
        let (old_code, _) = party.create("host");
        let (old_host, mut old_host_rx) = connect(&party, &old_code, "host");
        let (old_guest, mut old_guest_rx) = connect(&party, &old_code, "guest");
        party.receive(&old_host, &json!({"type":"load", "external_url":"old", "position":27}));
        party.receive(&old_host, &json!({"type":"approve", "title":"old queue"}));
        let (code, _) = party.create("host");
        assert!(drain(&mut old_host_rx).iter().any(|v| v["type"] == "ended"));
        assert!(drain(&mut old_guest_rx).iter().any(|v| v["type"] == "ended"));
        let (host, mut host_rx) = connect(&party, &code, "host");
        let (_guest, mut guest_rx) = connect(&party, &code, "guest");
        {
            let session = party.session.lock().unwrap();
            assert_eq!(session.state.external_url, None);
            assert_eq!(session.state.position, 0.0);
            assert!(!session.state.playing);
            assert!(session.state.queue.is_empty());
            assert_eq!(session.members.len(), 2);
        }
        party.receive(&host, &json!({"type":"load", "external_url":"new"}));
        drain(&mut host_rx);
        drain(&mut guest_rx);
        for message in [json!({"type":"load", "external_url":"stale"}), json!({"type":"pause"}), json!({"type":"approve", "title":"stale"})] {
            party.receive(&old_host, &message);
        }
        party.receive(&old_guest, &json!({"type":"request", "title":"stale request"}));
        party.disconnect(&old_guest);
        party.disconnect(&old_host);
        let session = party.session.lock().unwrap();
        assert_eq!(session.code.as_deref(), Some(code.as_str()));
        assert_eq!(session.state.external_url.as_deref(), Some("new"));
        assert!(session.state.playing);
        assert!(session.state.queue.is_empty());
        assert_eq!(session.members.len(), 2);
        assert!(drain(&mut host_rx).is_empty());
        assert!(drain(&mut guest_rx).is_empty());
    }

    #[test]
    fn replaced_host_and_closed_guest_sockets_lose_frame_and_cleanup_authority() {
        let party = Party::new();
        let (code, _) = party.create("host");
        let (old_host, _) = connect(&party, &code, "host");
        let (host, mut host_rx) = connect(&party, &code, "host");
        // The old receiver is dropped here, so this is a retired guest replacement.
        let (old_guest, _) = connect(&party, &code, "guest");
        let (_guest, mut guest_rx) = connect(&party, &code, "guest");
        drain(&mut host_rx);
        drain(&mut guest_rx);
        party.receive(&old_host, &json!({"type":"load", "title":"stale"}));
        party.receive(&old_guest, &json!({"type":"request", "title":"stale"}));
        party.disconnect(&old_host);
        party.disconnect(&old_guest);
        assert!(drain(&mut host_rx).is_empty());
        assert_eq!(party.session.lock().unwrap().members.len(), 2);
        party.receive(&host, &json!({"type":"load", "title":"current"}));
        assert!(drain(&mut guest_rx).iter().any(|v| v["state"]["title"] == "current"));
        party.disconnect(&host);
        assert!(drain(&mut guest_rx).iter().any(|v| v["type"] == "ended"));
        assert!(party.authorize(&code, None).is_none());
        assert!(party.session.lock().unwrap().members.is_empty());
    }

    #[test]
    fn host_proof_is_required_before_any_host_registration_or_eviction() {
        let party = Party::new();
        let (code, proof) = party.create("host");
        for supplied in [None, Some(uuid::Uuid::new_v4())] {
            let admission = party.authorize(&code, supplied).unwrap();
            assert!(!party.can_register(&admission, "host"));
            let (tx, _rx) = mpsc::unbounded_channel();
            assert!(party.register(admission, "host".into(), tx).is_none());
        }
        assert!(party.session.lock().unwrap().members.is_empty());
        let (host, mut host_rx) = connect(&party, &code, "host");
        let (_guest, mut guest_rx) = connect(&party, &code, "guest");
        drain(&mut host_rx);
        drain(&mut guest_rx);
        for (key, supplied) in [("host", None), ("host", Some(uuid::Uuid::new_v4())), ("guest", Some(proof))] {
            let admission = party.authorize(&code, supplied).unwrap();
            assert!(!party.can_register(&admission, key));
            let (tx, _rx) = mpsc::unbounded_channel();
            assert!(party.register(admission, key.into(), tx).is_none());
        }
        assert_eq!(party.session.lock().unwrap().members.len(), 2);
        assert!(drain(&mut host_rx).is_empty());
        assert!(drain(&mut guest_rx).is_empty());
        party.receive(&host, &json!({"type":"load", "title":"legitimate"}));
        let messages = drain(&mut guest_rx);
        assert!(messages.iter().any(|v| v["state"]["title"] == "legitimate"));
        assert!(!serde_json::to_string(&messages).unwrap().contains(&proof.to_string()));
        party.disconnect(&host);
        assert!(drain(&mut guest_rx).iter().any(|v| v["type"] == "ended"));
    }

    #[test]
    fn capability_rechecks_generation_and_identity_after_preflight_before_registration() {
        let party = Party::new();
        let (old_code, old_proof) = party.create("host");
        let admission = party.authorize(&old_code, Some(old_proof)).unwrap();
        assert!(party.can_register(&admission, "host"));
        let (code, _) = party.create("host");
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(party.register(admission, "host".into(), tx).is_none());
        let stale = party.authorize(&code, Some(old_proof)).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(party.register(stale, "host".into(), tx).is_none());
        let (host, _) = connect(&party, &code, "host");
        let proof = party.session.lock().unwrap().host_capability;
        let mismatched = party.authorize(&code, proof).unwrap();
        assert!(party.can_register(&mismatched, "host"));
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(party.register(mismatched, "guest".into(), tx).is_none());
        assert!(party.session.lock().unwrap().contains(&host));
    }

    #[test]
    fn protocol_parser_rejects_aliases_empty_and_duplicate_proofs_without_fallback() {
        use axum::http::{HeaderMap, HeaderValue};
        let proof = uuid::Uuid::new_v4().to_string();
        let mut headers = HeaderMap::new();
        assert!(host_proof(&headers).unwrap().is_none());
        headers.insert("sec-websocket-protocol", HeaderValue::from_static("frayd-party"));
        assert!(host_proof(&headers).unwrap().is_none());
        headers.append("sec-websocket-protocol", HeaderValue::from_str(&format!("frayd-host.{proof}")).unwrap());
        assert!(host_proof(&headers).unwrap().is_some());
        headers.append("sec-websocket-protocol", HeaderValue::from_str(&format!("frayd-host.{proof}")).unwrap());
        assert!(host_proof(&headers).is_err());
        for value in [
            "".to_string(), "frayd-party, ".to_string(), "frayd-party, frayd-party".to_string(),
            "frayd-party, frayd-host.".to_string(), "frayd-party, frayd-host.invalid".to_string(),
            format!("frayd-host.{proof}"), format!("frayd-party, frayd-host.{}", proof.to_uppercase()),
            format!("frayd-party, frayd-host.{}", proof.replace('-', "")),
            format!("frayd-party, frayd-host%2E{proof}"),
            format!("frayd-party, frayd-host.{proof}, frayd-host.{}", uuid::Uuid::new_v4()),
        ] {
            headers.clear();
            headers.insert("sec-websocket-protocol", HeaderValue::from_str(&value).unwrap());
            assert!(host_proof(&headers).is_err());
        }
    }

    #[test]
    fn active_guest_rejects_duplicate_registration_without_losing_requests_or_updates() {
        let party = Party::new();
        let (code, _) = party.create("host");
        let (host, mut host_rx) = connect(&party, &code, "host");
        let (victim, mut victim_rx) = connect(&party, &code, "victim");
        let (_observer, mut observer_rx) = connect(&party, &code, "observer");
        drain(&mut host_rx);
        drain(&mut victim_rx);
        drain(&mut observer_rx);
        let duplicate = party.authorize(&code, None).unwrap();
        assert!(!party.can_register(&duplicate, "victim"));
        let (tx, mut rejected_rx) = mpsc::unbounded_channel();
        assert!(party.register(duplicate, "victim".into(), tx).is_none());
        assert!(drain(&mut rejected_rx).is_empty());
        assert!(drain(&mut host_rx).is_empty());
        assert!(drain(&mut victim_rx).is_empty());
        assert!(drain(&mut observer_rx).is_empty());
        assert_eq!(party.session.lock().unwrap().members.len(), 3);
        assert!(party.session.lock().unwrap().contains(&victim));
        party.receive(&host, &json!({"type":"load", "title":"current host"}));
        assert!(drain(&mut victim_rx).iter().any(|v| v["state"]["title"] == "current host"));
        party.receive(&victim, &json!({"type":"request", "title":"original victim"}));
        for rx in [&mut host_rx, &mut observer_rx] {
            assert!(drain(rx).iter().any(|v| v["type"] == "request" && v["by"] == "Guest 1" && v["title"] == "original victim"));
        }
    }

    #[test]
    fn guest_replacement_requires_closed_receiver_or_completed_cleanup_and_retires_old_tokens() {
        let party = Party::new();
        let (code, _) = party.create("host");
        let (host, mut host_rx) = connect(&party, &code, "host");
        let (old_guest, old_rx) = connect(&party, &code, "guest");
        let replacement = party.authorize(&code, None).unwrap();
        assert!(!party.can_register(&replacement, "guest"));
        drop(old_rx);
        assert!(party.can_register(&replacement, "guest"));
        let (tx, mut current_rx) = mpsc::unbounded_channel();
        let current = party.register(replacement, "guest".into(), tx).unwrap();
        drain(&mut host_rx);
        drain(&mut current_rx);
        party.receive(&old_guest, &json!({"type":"request", "title":"stale guest"}));
        party.disconnect(&old_guest);
        assert!(drain(&mut host_rx).is_empty());
        assert!(party.session.lock().unwrap().contains(&current));
        party.receive(&host, &json!({"type":"load", "title":"new guest state"}));
        assert!(drain(&mut current_rx).iter().any(|v| v["state"]["title"] == "new guest state"));
        party.receive(&current, &json!({"type":"request", "title":"current guest"}));
        assert!(drain(&mut host_rx).iter().any(|v| v["type"] == "request" && v["title"] == "current guest"));
        party.disconnect(&current);
        let after_cleanup = party.authorize(&code, None).unwrap();
        assert!(party.can_register(&after_cleanup, "guest"));
        let (tx, _rx) = mpsc::unbounded_channel();
        assert!(party.register(after_cleanup, "guest".into(), tx).is_some());
    }

    #[test]
    fn racing_guest_registrations_recheck_ownership_after_both_preflights_pass() {
        let party = Party::new();
        let (code, _) = party.create("host");
        let first = party.authorize(&code, None).unwrap();
        let second = party.authorize(&code, None).unwrap();
        assert!(party.can_register(&first, "guest"));
        assert!(party.can_register(&second, "guest"));
        let (first_tx, mut first_rx) = mpsc::unbounded_channel();
        let (second_tx, mut second_rx) = mpsc::unbounded_channel();
        let barrier = std::sync::Barrier::new(2);
        let (one, two) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                party.register(first, "guest".into(), first_tx)
            });
            let second = scope.spawn(|| {
                barrier.wait();
                party.register(second, "guest".into(), second_tx)
            });
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_eq!(usize::from(one.is_some()) + usize::from(two.is_some()), 1);
        let winner = one.or(two).unwrap();
        assert!(party.session.lock().unwrap().contains(&winner));
        assert_eq!(party.session.lock().unwrap().members.len(), 1);
        let initial_states = [&mut first_rx, &mut second_rx].into_iter()
            .flat_map(drain).filter(|v| v["type"] == "state").count();
        assert_eq!(initial_states, 1);
    }
}
