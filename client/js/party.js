// Frayd party panel — host-controlled listening party over WS
'use strict';

const PartyUI = (() => {
  // youtube video id from watch?v=, youtu.be/, shorts/, embed/ URLs
  const YT_RE = /(?:youtube\.com\/(?:watch\?v=|shorts\/|embed\/)|youtu\.be\/)([\w-]{11})/;
  let ws = null;
  let isHost = false;
  let state = null;
  let partyCode = null;

  const $ = (s) => document.querySelector(s);

  function open() { $('#party-modal').classList.add('on'); render(); }
  function hide() { $('#party-modal').classList.remove('on'); }
  function resetSession() {
    ws = null; partyCode = null; isHost = false; state = null; pending = []; members = [];
    hostLoadedMedia = null; hostLoadedExt = null;
    window.partySend = null;
  }
  function close() {
    const socket = ws;
    resetSession();
    hide();
    if (socket) socket.close();
  }

  function identity() {
    return localStorage.frayd_identity || 'guest-' + Math.random().toString(36).slice(2, 8);
  }

  async function host() {
    const r = await fetch('/api/party', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ identity: identity() })
    });
    if (!r.ok) return renderError('Could not start party (' + r.status + ')');
    const { code } = await r.json();
    connect(code);
  }

  function join() {
    const code = $('#party-code-input').value.trim().toUpperCase();
    if (!/^[A-Z0-9]{4}$/.test(code)) return renderError('Enter the 4-character code');
    connect(code);
  }

  function connect(code) {
    const previous = ws;
    resetSession();
    if (previous) previous.close();
    partyCode = code;
    const url = `/api/party/${code}/ws?identity=${encodeURIComponent(identity())}`;
    const socket = new WebSocket(url);
    ws = socket;
    socket.onerror = () => { if (ws === socket) renderError('Party not found or rejected'); };
    socket.onclose = () => { if (ws === socket) renderEnded(); };
    socket.onmessage = (e) => { if (ws === socket) handle(JSON.parse(e.data)); };
    renderConnecting(code);
  }

  function handle(msg) {
    if (msg.type === 'state') {
      if (typeof msg.is_host === 'boolean') isHost = msg.is_host;
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

  function send(obj) { if (ws && ws.readyState === 1) ws.send(JSON.stringify(obj)); }

  // ---- guest follow: drive local audio player from host state ----
  let hostLoadedMedia = null;
  const DRIFT = 1.5; // seconds — resync beyond this

  function followState() {
    if (state && state.external_url) {
      if (typeof window.partyFollowExternal === 'function')
        window.partyFollowExternal(state.external_url, state.title, state.position, state.playing, state.updated_at, false);
      return;
    }
    if (typeof window.partyFollow !== 'function') return;
    if (state && state.media_id != null) {
      window.partyFollow(state.media_id, state.title, state.position, state.playing, state.updated_at);
    }
  }

  // host: only load tracks the party state changed to (skip/queue-advance) —
  // never follow play/pause/seek (those originate from host's own player events)
  function followMediaChangeOnly() {
    // external (YouTube) load → mirror into YT player, host events handled separately
    const ext = state?.external_url;
    if (ext && ext !== hostLoadedExt && typeof window.partyFollowExternal === 'function') {
      hostLoadedExt = ext;
      window.partyFollowExternal(ext, state.title, 0, state.playing, state.updated_at, true);
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
  function sendHost(obj) { if (isHost) send(obj); }

  function hookHost() {
    window.partySend = sendHost;
    if (hookHost.done) return;
    hookHost.done = true;
    if (typeof window.partyHostHook === 'function') window.partyHostHook(sendHost);
  }

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

  function membersHtml() {
    const me = identity();
    const list = isHost ? [me + ' (host)', ...members.filter(m => !m.endsWith('(host)'))] : members;
    if (!list.length) return '';
    return `<div class="party-sec">Members · ${list.length}</div><ul class="party-queue">` +
      list.map(m => {
        const isMe = m === me || (isHost && m === me + ' (host)');
        return `<li class="${isMe ? 'me' : ''}">${esc(m)}${isMe ? ' <span class="s">(you)</span>' : ''}</li>`;
      }).join('') + '</ul>';
  }

  function queueHtml() {
    const q = state?.queue || [];
    if (!q.length) return '';
    return '<div class="party-sec">Queue</div><ul class="party-queue">' +
      q.map(i => `<li>${esc(i.title)} <span class="s">by ${esc(i.requested_by)}</span></li>`).join('') +
      '</ul>';
  }

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
      library.filter(it => ['movie', 'show', 'music', 'audiobook'].includes(it.kind)).slice(0, 20).map(it =>
        `<li>${esc(it.title)} <button data-media="${it.id}" data-title="${esc(it.title)}" class="pt-request">Request</button></li>`
      ).join('') + '</ul>';
  }

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
      window.partyFollowExternal(id, 'YouTube · ' + id, 0, true, Math.floor(Date.now() / 1000), true);
    });
    $('#pt-yt')?.addEventListener('keydown', (e) => { if (e.key === 'Enter') $('#pt-yt-go').click(); });
  }

  function renderConnecting(code) {
    $('#party-stage').innerHTML = `<div class="party-code">Connecting to <b>${esc(code)}</b>…</div>`;
  }
  function renderError(msg) {
    $('#party-stage').insertAdjacentHTML('afterbegin', `<div class="party-error">${esc(msg)}</div>`);
  }
  function renderEnded() {
    const socket = ws;
    resetSession();
    if (socket && socket.readyState < 2) socket.close();
    render();
    renderError('Party ended');
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
