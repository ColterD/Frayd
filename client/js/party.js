// Frayd party panel — host-controlled listening party over WS
'use strict';

const PartyUI = (() => {
  let ws = null;
  let isHost = false;
  let state = null;

  const $ = (s) => document.querySelector(s);

  function open() { $('#party-modal').classList.add('on'); render(); }
  function close() {
    $('#party-modal').classList.remove('on');
    if (ws) { ws.close(); ws = null; }
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
    if (ws) ws.close();
    const url = `/api/party/${code}/ws?identity=${encodeURIComponent(identity())}`;
    ws = new WebSocket(url);
    ws.onerror = () => renderError('Party not found or rejected');
    ws.onclose = () => { if ($('#party-modal').classList.contains('on')) renderEnded(); };
    ws.onmessage = (e) => handle(JSON.parse(e.data), code);
    renderConnecting(code);
  }

  function handle(msg, code) {
    if (msg.type === 'state') {
      if (typeof msg.is_host === 'boolean') isHost = msg.is_host;
      state = msg.state;
      if (isHost) { hookHost(); followMediaChangeOnly(); } else followState();
      render(code);
    } else if (msg.type === 'request') {
      pending.push(msg);
      render(code);
    } else if (msg.type === 'members') {
      members = msg.members || [];
      render(code);
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
    if (typeof window.partyFollow !== 'function') return;
    if (state && state.media_id != null) {
      window.partyFollow(state.media_id, state.title, state.position, state.playing, state.updated_at);
    }
  }

  // host: only load tracks the party state changed to (skip/queue-advance) —
  // never follow play/pause/seek (those originate from host's own player events)
  function followMediaChangeOnly() {
    const id = state?.media_id;
    if (id != null && id !== hostLoadedMedia && typeof window.partyFollow === 'function') {
      hostLoadedMedia = id;
      window.partyFollow(id, state.title, 0, state.playing, state.updated_at);
    }
  }

  // ---- host hooks: local player actions become party state ----
  function hookHost() {
    if (hookHost.done) return;
    hookHost.done = true;
    if (typeof window.partyHostHook === 'function') window.partyHostHook(send);
    window.partySend = send;
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

  function libraryPickerHtml() {
    if (isHost || !window.LIBRARY) return '';
    return '<div class="party-sec">Request a track</div><ul class="party-queue">' +
      window.LIBRARY.slice(0, 20).map(it =>
        `<li>${esc(it.title)} <button data-media="${it.id}" data-title="${esc(it.title)}" class="pt-request">Request</button></li>`
      ).join('') + '</ul>';
  }

  function render(code) {
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
    stage.innerHTML = `
      <div class="party-code">Code: <b>${esc(code)}</b> <span class="s">${isHost ? '(you are host)' : '(guest)'}</span></div>
      ${nowPlaying()}
      ${membersHtml()}
      ${queueHtml()}
      ${requestsHtml()}
      ${libraryPickerHtml()}`;
    const bind = (id, fn) => { const b = $(id); if (b) b.onclick = fn; };
    bind('#pt-play', () => send({ type: 'play' }));
    bind('#pt-pause', () => send({ type: 'pause' }));
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
  }

  function renderConnecting(code) {
    $('#party-stage').innerHTML = `<div class="party-code">Connecting to <b>${esc(code)}</b>…</div>`;
  }
  function renderError(msg) {
    $('#party-stage').insertAdjacentHTML('afterbegin', `<div class="party-error">${esc(msg)}</div>`);
  }
  function renderEnded() {
    ws = null; isHost = false; state = null; pending = []; members = [];
    render();
    renderError('Party ended');
  }

  return { open, close };
})();

document.addEventListener('DOMContentLoaded', () => {
  const btn = $('#party-btn');
  if (btn) btn.onclick = PartyUI.open;
  const x = $('#pt-close');
  if (x) x.onclick = PartyUI.close;
});
