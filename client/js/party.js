// Frayd party panel — host-controlled listening party over WS
'use strict';

const PartyUI = (() => {
  // youtube video id from watch?v=, youtu.be/, shorts/, embed/ URLs
  const YT_RE = /(?:youtube\.com\/(?:watch\?v=|shorts\/|embed\/)|youtu\.be\/)([\w-]{11})/;
  let ws = null;
  let isHost = false;
  let selfLabel = null;
  let state = null;
  let partyCode = null;
  let pendingStart = null;

  /**
   * Return the first party DOM node matching a selector.
   * @param {string} s
   * @returns {Element|null}
   */
  const $ = (s) => document.querySelector(s);

  /** Show the party panel and render its current session without disconnecting. */
  function open() { $('#party-modal').classList.add('on'); render(); }
  /** Hide the party panel while retaining the active socket and state. */
  function hide() { $('#party-modal').classList.remove('on'); }
  /** Retire the socket identity, role, requests and loaded-media markers before stale callbacks run. */
  function resetSession() {
    pendingStart = null;
    ws = null; partyCode = null; isHost = false; selfLabel = null; state = null; pending = []; members = [];
    hostLoadedMedia = null; hostLoadedExt = null;
    window.partySend = null;
  }
  /** Leave the current party, close its socket and hide the panel. */
  function close() {
    const socket = ws;
    resetSession();
    hide();
    if (socket) socket.close();
  }

  /**
   * Use the persisted device identity or make a temporary guest identity for this call.
   * @returns {string}
   */
  function identity() {
    return localStorage.frayd_identity || 'guest-' + Math.random().toString(36).slice(2, 8);
  }

  /** Start with a private proof tied to the exact creation identity; cancel both awaits on reset.
   * Keep the proof scoped to this call and reject older create APIs without an insecure fallback.
   */
  async function host() {
    if (pendingStart) return;
    // This token spans both awaits; session reset cancels its authority to connect.
    const start = {};
    const hostIdentity = identity();
    pendingStart = start;
    try {
      const r = await fetch('/api/party', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ identity: hostIdentity })
      });
      if (pendingStart !== start) return;
      if (!r.ok) return renderError('Could not start party (' + r.status + ')');
      const { code, host_capability: capability } = await r.json();
      if (pendingStart !== start) return;
      if (typeof capability !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(capability))
        return renderError('Could not start party');
      connect(code, capability, hostIdentity);
    } catch {
      if (pendingStart === start) renderError('Could not start party');
    } finally {
      if (pendingStart === start) pendingStart = null;
    }
  }

  /** Validate the entered four-character code and connect as a guest. */
  function join() {
    const code = $('#party-code-input').value.trim().toUpperCase();
    if (!/^[A-Z0-9]{4}$/.test(code)) return renderError('Enter the 4-character code');
    connect(code);
  }

  /**
   * Replace the current party session and give its socket exclusive ownership of
   * incoming updates. Identity checks on every callback prevent a superseded
   * socket's messages, errors, or close event from changing the new session.
   * The creator's capability lives only in this connection call and the upgrade
   * protocol offer; it never enters a URL, storage, or a party message.
   * @param {string} code Party code used by the WebSocket endpoint and panel.
   * @param {string|null} capability Private creator proof; absent for guest joins.
   * @param {string} key Identity captured at creation, or the current guest identity.
   */
  function connect(code, capability = null, key = identity()) {
    const previous = ws;
    resetSession();
    if (previous) previous.close();
    partyCode = code;
    const url = `/api/party/${code}/ws?identity=${encodeURIComponent(key)}`;
    const protocols = capability ? ['frayd-party', 'frayd-host.' + capability] : ['frayd-party'];
    const socket = new WebSocket(url, protocols);
    ws = socket;
    let admitted = false;
    let rejectedBeforeAdmission = false;
    socket.onerror = () => {
      if (ws !== socket) return;
      rejectedBeforeAdmission = !admitted;
      renderError('Party not found or rejected');
    };
    socket.onclose = () => {
      if (ws === socket) renderEnded(rejectedBeforeAdmission ? 'Party not found or rejected' : 'Party ended');
    };
    socket.onmessage = (e) => {
      if (ws !== socket) return;
      const msg = JSON.parse(e.data);
      if (msg.type === 'state') admitted = true;
      handle(msg);
    };
    renderConnecting(code);
  }

  /**
   * Apply a message admitted by the active socket's callback. State snapshots
   * replace shared playback state; only an explicit host flag changes our role.
   * Guests follow authoritative playback, while hosts load changed media without
   * replaying their own player events. Requests and membership update the panel;
   * an ended message retires the entire session.
   * @param {Object} msg Parsed server message with a type-specific payload.
   */
  function handle(msg) {
    if (msg.type === 'state') {
      if (typeof msg.is_host === 'boolean') isHost = msg.is_host;
      if (typeof msg.self_label === 'string') selfLabel = msg.self_label;
      state = msg.state;
      if (isHost) { hookHost(); followMediaChangeOnly(); } else followState();
      render();
    } else if (msg.type === 'request') {
      pending.push(msg);
      render();
    } else if (msg.type === 'members') {
      members = msg.members || [];
      render();
    } else if (msg.type === 'ended') {
      renderEnded();
    }
  }
  let pending = [];
  let members = [];

  /**
   * Send a JSON party command only while the current socket is open.
   * @param {Object} obj
   */
  function send(obj) { if (ws && ws.readyState === 1) ws.send(JSON.stringify(obj)); }

  // ---- guest follow: drive local audio player from host state ----
  let hostLoadedMedia = null;
  const DRIFT = 1.5; // seconds — resync beyond this

  /** Apply authoritative host playback to a guest player when a supported callback exists. */
  function followState() {
    if (state && state.external_url) {
      if (typeof window.partyFollowExternal === 'function')
        window.partyFollowExternal(state.external_url, state.title, state.position, state.playing, state.updated_at);
      return;
    }
    if (typeof window.partyFollow !== 'function') return;
    if (state && state.media_id != null) {
      window.partyFollow(state.media_id, state.title, state.position, state.playing, state.updated_at);
    }
  }

  // host: only load tracks the party state changed to (skip/queue-advance) —
  // never follow play/pause/seek (those originate from host's own player events)
  /**
   * Load each changed external or library media identifier once for the host.
   * Local host events own play, pause, seek, and resume position, so these loads
   * start at zero instead of following the echoed server position. The library
   * callback may preserve an already active video and its local resume position.
   * Guests use followState for authoritative position and playback updates.
   */
  function followMediaChangeOnly() {
    // external (YouTube) load → mirror into YT player, host events handled separately
    const ext = state?.external_url;
    if (ext && ext !== hostLoadedExt && typeof window.partyFollowExternal === 'function') {
      hostLoadedExt = ext;
      window.partyFollowExternal(ext, state.title, 0, state.playing, state.updated_at);
      return;
    }
    const id = state?.media_id;
    if (id != null && id !== hostLoadedMedia && typeof window.partyFollow === 'function') {
      hostLoadedMedia = id;
      window.partyFollow(id, state.title, 0, state.playing, state.updated_at, { preserveActiveVideo: true });
    }
  }
  let hostLoadedExt = null;

  // ---- host hooks: local player actions become party state ----
  /**
   * Publish a local player command only while this session has host authority.
   * @param {Object} obj
   */
  function sendHost(obj) { if (isHost) send(obj); }

  /**
   * Restore the current session's public sender on every host snapshot, while
   * installing player listeners only once for this page. Their shared sendHost
   * callback reads the current role and socket, allowing rehosting without
   * duplicate listeners or broadcasts from a later guest session.
   */
  function hookHost() {
    window.partySend = sendHost;
    if (hookHost.done) return;
    hookHost.done = true;
    if (typeof window.partyHostHook === 'function') window.partyHostHook(sendHost);
  }

  /**
   * Render escaped playback details and host controls, or an empty state.
   * @returns {string}
   */
  function nowPlaying() {
    if (!state) return '<div class="party-empty">Nothing playing.</div>';
    let html = `<div class="party-now">
      <div class="t">${esc(state.title || '—')}</div>
      <div class="s">${state.playing ? '▶ playing' : '⏸ paused'} · ${Math.floor(state.position)}s</div>`;
    if (isHost) {
      html += `<div class="party-host-controls">
        <button id="pt-play">Play</button>
        <button id="pt-pause">Pause</button>
        <button id="pt-skip">Skip</button>
      </div>
      <div class="party-sec">Play a YouTube video</div>
      <div class="party-join">
        <input id="pt-yt" placeholder="YouTube URL" autocomplete="off">
        <button id="pt-yt-go">Play</button>
      </div>`;
    }
    return html + '</div>';
  }

  /**
   * Render escaped party membership with the local participant marked.
   * @returns {string}
   */
  function membersHtml() {
    const list = members;
    if (!list.length) return '';
    return `<div class="party-sec">Members · ${list.length}</div><ul class="party-queue">` +
      list.map(m => {
        const isMe = selfLabel !== null && (m === selfLabel || (isHost && m === selfLabel + ' (host)'));
        return `<li class="${isMe ? 'me' : ''}">${esc(m)}${isMe ? ' <span class="s">(you)</span>' : ''}</li>`;
      }).join('') + '</ul>';
  }

  /**
   * Render the current queued requests, or empty markup when there are none.
   * @returns {string}
   */
  function queueHtml() {
    const q = state?.queue || [];
    if (!q.length) return '';
    return '<div class="party-sec">Queue</div><ul class="party-queue">' +
      q.map(i => `<li>${esc(i.title)} <span class="s">by ${esc(i.requested_by)}</span></li>`).join('') +
      '</ul>';
  }

  /**
   * Render pending approval controls for hosts only.
   * @returns {string}
   */
  function requestsHtml() {
    if (!isHost || !pending.length) return '';
    return '<div class="party-sec">Requests</div><ul class="party-queue">' +
      pending.map((r, i) =>
        `<li>${esc(r.title)} <span class="s">by ${esc(r.by)}</span> <button data-req="${i}" class="pt-approve">Approve</button></li>`
      ).join('') + '</ul>';
  }

  /** Render up to twenty items supported by the party's video and audio players. */
  function libraryPickerHtml() {
    if (isHost) return '';
    const library = window.getLibrary?.() || [];
    return '<div class="party-sec">Request a track</div><ul class="party-queue">' +
      library.filter(it => ['movie', 'episode', 'music', 'audiobook'].includes(it.kind)).slice(0, 20).map(it =>
        `<li>${esc(it.title)} <button data-media="${it.id}" data-title="${esc(it.title)}" class="pt-request">Request</button></li>`
      ).join('') + '</ul>';
  }

  /**
   * Rebuild controls from the current session and bind actions to its live state.
   * Socket ownership selects entry, connecting, or active-party content; modal
   * visibility is independent, so Hide and reopen retain the connection and its
   * updates. Leave clears the session separately before entry controls return.
   * Host playback controls act through the local player, which publishes events.
   */
  function render() {
    const stage = $('#party-stage');
    if (!ws) {
      stage.innerHTML = `
        <button id="pt-host">Start a party</button>
        <div class="party-join">
          <input id="party-code-input" maxlength="4" placeholder="CODE" autocomplete="off">
          <button id="pt-join">Join</button>
        </div>`;
      $('#pt-host').onclick = host;
      $('#pt-join').onclick = join;
      return;
    }
    if (!state) return renderConnecting(partyCode);
    stage.innerHTML = `
      <div class="party-code">Code: <b>${esc(partyCode)}</b> <span class="s">${isHost ? '(you are host)' : '(guest)'}</span></div>
      ${nowPlaying()}
      ${membersHtml()}
      ${queueHtml()}
      ${requestsHtml()}
      ${libraryPickerHtml()}`;
    const bind = (id, fn) => { const b = $(id); if (b) b.onclick = fn; };
    const control = async type => {
      if (typeof window.partyControlPlayback !== 'function') return renderError('Player is not ready');
      try { await window.partyControlPlayback(type, state); }
      catch (error) { renderError(error.message); }
    };
    bind('#pt-play', () => control('play'));
    bind('#pt-pause', () => control('pause'));
    bind('#pt-skip', () => send({ type: 'skip' }));
    bind('#pt-host', host);
    bind('#pt-join', join);
    document.querySelectorAll('.pt-approve').forEach(b => {
      b.onclick = () => {
        const r = pending.splice(+b.dataset.req, 1)[0];
        send({ type: 'approve', media_id: r.media_id, title: r.title, by: r.by });
      };
    });
    document.querySelectorAll('.pt-request').forEach(b => {
      b.onclick = () => send({ type: 'request', media_id: +b.dataset.media, title: b.dataset.title });
    });

    bind('#pt-yt-go', () => {
      const m = ($('#pt-yt')?.value || '').match(YT_RE);
      if (!m) return renderError('Paste a YouTube URL (youtube.com/watch?v=… or youtu.be/…)');
      const id = m[1];
      send({ type: 'load', external_url: id, title: 'YouTube · ' + id, position: 0 });
      window.partyFollowExternal(id, 'YouTube · ' + id, 0, true, Math.floor(Date.now() / 1000));
    });
    $('#pt-yt')?.addEventListener('keydown', (e) => { if (e.key === 'Enter') $('#pt-yt-go').click(); });
  }

  /**
   * Show the code for a socket awaiting admission.
   * @param {string} code
   */
  function renderConnecting(code) {
    $('#party-stage').innerHTML = `<div class="party-code">Connecting to <b>${esc(code)}</b>…</div>`;
  }
  /**
   * Prepend an escaped party error to the current panel.
   * @param {string} msg
   */
  function renderError(msg) {
    $('#party-stage').insertAdjacentHTML('afterbegin', `<div class="party-error">${esc(msg)}</div>`);
  }
  /**
   * Retire an ended session before closing its socket so late callbacks cannot
   * restore stale state. Clear role, sender, members, requests, and loaded-media
   * markers, then show entry controls with an ended notice. Preserve visibility
   * so a remotely ended hidden panel stays hidden until the user reopens it.
   */
  function renderEnded(message = 'Party ended') {
    const socket = ws;
    resetSession();
    if (socket && socket.readyState < 2) socket.close();
    render();
    renderError(message);
  }

  return { open, hide, close };
})();

document.addEventListener('DOMContentLoaded', () => {
  const btn = $('#party-btn');
  if (btn) btn.onclick = PartyUI.open;
  const hide = $('#pt-hide');
  if (hide) hide.onclick = PartyUI.hide;
  const x = $('#pt-close');
  if (x) x.onclick = PartyUI.close;
});
