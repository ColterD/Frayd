// Frayd web client — auth, dashboard, playback
'use strict';
console.log('frayd app.js v4');

const $ = (s) => document.querySelector(s);
const el = (tag, cls, html) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html != null) e.innerHTML = html;
  return e;
};
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, c =>
  ({ '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;' }[c]));

// ---------- toast ----------
function toast(msg, err = false) {
  let host = $('#toast-host');
  if (!host) { host = el('div'); host.id = 'toast-host'; document.body.appendChild(host); }
  const t = el('div', 'toast' + (err ? ' toast-err' : ''), esc(msg));
  host.appendChild(t);
  requestAnimationFrame(() => t.classList.add('on'));
  setTimeout(() => { t.classList.remove('on'); setTimeout(() => t.remove(), 300); }, err ? 5000 : 2600);
}

// ---------- identity / auth ----------
let IDENTITY = localStorage.frayd_identity;
function authHeaders(extra = {}) {
  const h = { ...extra };
  if (localStorage.frayd_token) h['Authorization'] = 'Bearer ' + localStorage.frayd_token;
  return h;
}
let account = null;

async function tryResume() {
  if (IDENTITY) {
    const r = await fetch('/api/library');
    if (r.ok) {
      // restore admin flag from token (button visibility)
      if (localStorage.frayd_token) {
        try {
          const me = await (await fetch('/api/auth/me', { headers: authHeaders() })).json();
          account = { name: me.name, isAdmin: !!me.is_admin };
        } catch {}
      }
      enterApp();
      return true;
    }
  }
  return false;
}

function enterApp() {
  document.body.classList.remove('auth');
  $('#login').style.display = 'none';
  $('#app').classList.add('on');
  const initial = (account?.name || 'G').trim()[0].toUpperCase();
  $('.avatar').textContent = initial;
  $('#user-name').textContent = account?.name || 'Local user';
  $('#user-status').textContent = account ? 'signed in' : 'LAN · hostname key';
  $('#settings-btn').style.display = account?.isAdmin ? '' : 'none';
  loadLibrary();
}

// ---------- settings (admin) ----------
const KINDS = ['movie', 'music', 'audiobook', 'ebook', 'comic', 'manga'];
function reopenSettings() { $('#settings-modal').classList.remove('on'); $('#settings-btn').click(); }
$('#settings-btn')?.addEventListener('click', async () => {
  $('#settings-modal').classList.add('on');
  const stage = $('#settings-stage');
  stage.innerHTML = '<div class="party-sec">Accounts & permissions</div>';
  try {
    const [perms, settings] = await Promise.all([
      fetch('/api/admin/perms', { headers: authHeaders() }).then(r => r.ok ? r.json() : Promise.reject(r.status)),
      fetch('/api/admin/settings', { headers: authHeaders() }).then(r => r.ok ? r.json() : {}),
    ]);
    const libs = settings.library_paths || [];
    if (settings.library_path && !libs.includes(settings.library_path)) libs.unshift(settings.library_path);
    const kindBoxes = (p) => KINDS.map(k => `
      <label class="p-kind"><input type="checkbox" value="${k}" ${!p.allowed_kinds || p.allowed_kinds.includes(k) ? 'checked' : ''}>${k}</label>`).join('');
    stage.innerHTML = '<div class="party-sec">Users — untick kinds to hide that section from them</div>'
      + (perms.length ? perms.map(p => `
      <div class="perm-row" data-id="${p.identity_id}">
        <span class="k">${esc(p.key)}</span>
        <label class="p-kind"><input type="checkbox" class="p-admin" ${p.is_admin ? 'checked' : ''}> admin</label>
        <span class="p-kinds-box">${kindBoxes(p)}</span>
        <button class="p-save">Save</button>
        <button class="p-remove" data-name="${esc(p.key)}" title="Remove user, their progress and highlights">Remove</button>
      </div>`).join('') : '<div class="party-sec">No accounts yet</div>')
      + '<div class="party-sec">Media folders</div>'
      + (libs.length ? libs.map((l, i) => `
      <div class="perm-row"><span class="k">${esc(l)}</span><button class="lib-remove" data-path="${esc(l)}">Remove</button></div>`).join('') : '<div class="party-sec">No folders configured</div>')
      + `<div class="perm-row"><span class="k">Add folder</span><input class="s-lib-add" placeholder="/path/to/more/media" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"><button class="lib-add">Add + scan</button></div>`
      + '<div class="party-sec">Server settings</div>'
      + `<div class="perm-row"><span class="k">Site name</span><input class="s-site" value="${esc(settings.site_name ?? 'Frayd')}" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px"><button id="s-site-save">Save</button></div>`
      + `<div class="party-sec">Network <span class="s">(restart server to apply)</span></div>`
      + `<div class="perm-row"><span class="k">Port</span><input class="s-port" type="number" min="1024" max="65535" value="${esc(settings.port ?? 8484)}" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;width:90px"></div>`
      + `<div class="perm-row"><span class="k">UPnP port-forward</span><label><input type="checkbox" class="s-upnp" ${settings.upnp ? 'checked' : ''}> router must have UPnP on + 'upnpc' installed</label></div>`
      + `<div class="perm-row"><span class="k">HTTPS cert path (PEM)</span><input class="s-cert" value="${esc(settings.tls_cert ?? '')}" placeholder="/path/cert.pem" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"></div>`
      + `<div class="perm-row"><span class="k">HTTPS key path (PEM)</span><input class="s-key" value="${esc(settings.tls_key ?? '')}" placeholder="/path/key.pem" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"></div>`
      + `<div class="perm-row"><span class="k">Domain</span><input class="s-domain" value="${esc(settings.domain ?? '')}" placeholder="media.example.com" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"><span class="s">A record → your IP; use a real cert (e.g. Let's Encrypt) for this</span></div>`
      + `<div class="party-sec">Library</div>`
      + `<div class="perm-row"><button class="p-scan">Scan all folders</button></div>`
      + `<div class="perm-row"><span class="k">Add from URL</span><input class="s-ingest" placeholder="video URL (yt-dlp supported)" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"><button class="p-ingest">Download</button><span class="s">needs yt-dlp installed</span></div>`
      + `<div class="perm-row" style="border:none"><span class="s">Suggested layout (any subset):</span></div>`
      + `<div class="perm-row" style="border:none"><pre class="s" style="color:#8A8F9E;font-size:12px;margin:0;line-height:1.5">Media/\n  Movies/    *.mp4 *.mkv\n  Shows/     S01E01 files\n  Music/     *.mp3 *.flac\n  Audiobooks/ *.m4b *.mp3\n  Ebooks/    *.epub\n  Comics/    *.cbz *.cbr\n  Manga/     folder of images under a *manga* dir</pre></div>`
      + `<button id="s-net-save" style="margin-top:10px;width:100%;background:#5B46F6;color:#fff;border:none;border-radius:8px;padding:10px;cursor:pointer">Save network settings</button>`
      + (account ? `<div class="party-sec">Your password</div>
      <div class="perm-row"><span class="k">Current</span><input type="password" class="pw-old" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px"></div>
      <div class="perm-row"><span class="k">New (min 4)</span><input type="password" class="pw-new" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px"></div>
      <button id="s-pw-save" style="margin-top:10px;width:100%;background:#16181E;color:#fff;border:1px solid #2a2d36;border-radius:8px;padding:10px;cursor:pointer">Change password</button>` : '');
    stage.querySelectorAll('.p-save').forEach(b => b.onclick = async () => {
      const row = b.closest('.perm-row');
      const checked = [...row.querySelectorAll('.p-kind input:checked')].map(o => o.value);
      const all = checked.length === KINDS.length;
      const r = await fetch('/api/admin/perms', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({
          identity_id: +row.dataset.id,
          is_admin: row.querySelector('.p-admin').checked,
          allowed_kinds: all ? null : checked, // null = no restriction
        }),
      });
      b.textContent = r.ok ? '✓' : '✗';
      setTimeout(() => { b.textContent = 'Save'; }, 1200);
    });
    $('#s-site-save').onclick = async () => {
      const r = await fetch('/api/admin/settings', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ site_name: $('.s-site').value }),
      });
      $('#s-site-save').textContent = r.ok ? '✓' : '✗';
      setTimeout(() => { $('#s-site-save').textContent = 'Save'; }, 1200);
    };
    $('#s-net-save').onclick = async (e) => {
      const stage = $('#settings-stage');
      const port = +stage.querySelector('.s-port').value;
      if (!Number.isInteger(port) || port < 1024 || port > 65535) { e.target.textContent = 'Bad port'; return; }
      const body = {
        port,
        upnp: stage.querySelector('.s-upnp').checked,
        tls_cert: stage.querySelector('.s-cert').value.trim() || null,
        tls_key: stage.querySelector('.s-key').value.trim() || null,
        domain: stage.querySelector('.s-domain').value.trim() || null,
      };
      const r = await fetch('/api/admin/settings', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(body),
      });
      e.target.textContent = r.ok ? '✓ Saved — restart to apply port/TLS' : '✗ Failed';
      setTimeout(() => { e.target.textContent = 'Save network settings'; }, 2000);
    };
    $('#s-pw-save')?.addEventListener('click', async (e) => {
      const old_ = stage.querySelector('.pw-old').value, nw = stage.querySelector('.pw-new').value;
      if (nw.length < 4) { e.target.textContent = 'Too short (min 4)'; return; }
      const r = await fetch('/api/auth/password', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ old: old_, new: nw }),
      });
      if (r.ok) { e.target.textContent = '✓ Changed'; stage.querySelector('.pw-old').value = stage.querySelector('.pw-new').value = ''; }
      else { const j = await r.json().catch(() => ({})); e.target.textContent = j.error || '✗ Failed'; }
      setTimeout(() => { e.target.textContent = 'Change password'; }, 2500);
    });
    // library: scan all configured roots
    const scanBtn = stage.querySelector('.p-scan');
    scanBtn.onclick = async () => {
      scanBtn.textContent = 'Scanning…';
      const r2 = await fetch('/api/scan', { method: 'POST', headers: authHeaders() });
      if (r2.status === 202) {
        const poll = setInterval(async () => {
          const s = await (await fetch('/api/scan/status')).json();
          if (!s.scanning) { clearInterval(poll); scanBtn.textContent = '✓ done'; loadLibrary(); setTimeout(() => { scanBtn.textContent = 'Scan all folders'; }, 2000); }
        }, 1000);
      } else {
        scanBtn.textContent = r2.status === 409 ? 'busy' : r2.status === 501 ? 'no folders' : '✗ ' + r2.status;
        setTimeout(() => { scanBtn.textContent = 'Scan all folders'; }, 2000);
      }
    };
    // add a media folder: validate server-side (400 if not a dir), persist to library_paths, scan
    stage.querySelector('.lib-add').onclick = async (e) => {
      const input = stage.querySelector('.s-lib-add');
      const path = input.value.trim();
      if (!path) return;
      e.target.textContent = '…';
      const r = await fetch('/api/admin/settings', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ library_paths: [...libs, path] }),
      });
      if (r.ok) {
        toast('Folder added — scanning');
        reopenSettings(); // re-render list with new folder
        fetch('/api/scan', { method: 'POST', headers: authHeaders() });
      } else {
        toast(r.status === 400 ? 'Path is not a directory' : `Add failed (${r.status})`, true);
        e.target.textContent = 'Add + scan';
      }
    };
    // remove a media folder (rows on disk untouched; rescan to drop from view? no — scan never deletes rows)
    stage.querySelectorAll('.lib-remove').forEach(b => b.onclick = async () => {
      const path = b.dataset.path;
      if (!confirm(`Stop scanning "${path}"? Files on disk are not touched, but already-scanned items stay in the library until you delete them from the library view.`)) return;
      const remaining = libs.filter(l => l !== path);
      const r = await fetch('/api/admin/settings', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ library_paths: remaining, ...(remaining.length ? {} : { library_path: null }) }),
      });
      if (r.ok) { toast('Folder removed'); reopenSettings(); } else toast('Remove failed', true);
    });
    // remove a user: sessions, progress, highlights gone. Cannot remove self/last admin.
    stage.querySelectorAll('.p-remove').forEach(b => b.onclick = async () => {
      const row = b.closest('.perm-row');
      if (!confirm(`Remove user ${b.dataset.name}? Their progress and highlights will be deleted. This cannot be undone.`)) return;
      const r = await fetch(`/api/admin/users/${row.dataset.id}?identity=${encodeURIComponent(IDENTITY)}`, { method: 'DELETE', headers: authHeaders() });
      if (r.ok) { toast('User removed'); reopenSettings(); }
      else if (r.status === 409) toast('Cannot remove: last admin (or yourself)', true);
      else toast(`Remove failed (${r.status})`, true);
    });
    // ingest: yt-dlp download into library, then rescan picks it up
    const ingestBtn = stage.querySelector('.p-ingest');
    ingestBtn.onclick = async () => {
      const url = stage.querySelector('.s-ingest').value.trim();
      if (!/^https?:\/\//.test(url)) { ingestBtn.textContent = '✗ bad url'; return; }
      ingestBtn.textContent = 'Starting…';
      const r = await fetch('/api/ingest', { method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }), body: JSON.stringify({ url }) });
      if (r.status === 202) {
        ingestBtn.textContent = 'Downloading…';
        ingestBtn.disabled = true;
        // poll until ingest finishes, then refresh library
        const poll = setInterval(async () => {
          let st;
          try { st = (await (await fetch('/api/scan/status')).json()).ingesting; }
          catch { return; }
          if (st === 'downloading') return;
          clearInterval(poll);
          ingestBtn.disabled = false;
          ingestBtn.textContent = 'Download';
          if (st === 'done') {
            toast('Download complete — added to library');
            LIBRARY = await (await fetch('/api/library')).json();
            renderRows(LIBRARY);
          } else if (st === 'failed') toast('yt-dlp download failed', true);
          else setTimeout(() => { ingestBtn.textContent = 'Download'; }, 2000);
        }, 2000);
      } else if (r.status === 409) { ingestBtn.textContent = '✗ already downloading'; setTimeout(() => { ingestBtn.textContent = 'Download'; }, 3000); }
      else if (r.status === 501) { ingestBtn.textContent = '✗ no yt-dlp'; setTimeout(() => { ingestBtn.textContent = 'Download'; }, 3000); }
      else { ingestBtn.textContent = '✗ ' + r.status; setTimeout(() => { ingestBtn.textContent = 'Download'; }, 3000); }
    };
  } catch (e) {
    stage.innerHTML = `<div class="party-sec">Admin access required (${e}). Log out and back in to refresh your session.</div>`;
    return;
  }
  // (bindings below only run on successful render)
});
$('#settings-close')?.addEventListener('click', () => $('#settings-modal').classList.remove('on'));

async function doLogin(ev) {
  ev.preventDefault();
  const email = $('#email').value.trim();
  const pw = $('#password').value;
  const err = $('#auth-error');
  err.style.display = 'none';
  try {
    const r = await fetch('/api/auth/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email, password: pw })
    });
    if (!r.ok) throw new Error(r.status === 401 ? 'Invalid email or password' : `Server error (${r.status})`);
    const data = await r.json();
    account = { name: data.name || email.split('@')[0], token: data.token, isAdmin: !!data.is_admin };
    localStorage.frayd_token = data.token;
    IDENTITY = localStorage.frayd_identity ||= location.hostname + ':' + crypto.randomUUID();
    enterApp();
  } catch (e) {
    err.textContent = e.message;
    err.style.display = 'block';
  }
}

$('#login-form').addEventListener('submit', doLogin);
$('#pw-toggle').onclick = () => {
  const p = $('#password');
  p.type = p.type === 'password' ? 'text' : 'password';
  $('#pw-toggle').setAttribute('aria-label', p.type === 'password' ? 'Show password' : 'Hide password');
};
$('#logout')?.addEventListener('click', () => {
  fetch('/api/auth/logout', { method: 'POST', headers: authHeaders() }).catch(() => {});
  account = null;
  delete localStorage.frayd_token;
  // force login screen: tryResume() would otherwise auto-resume as LAN guest
  localStorage.frayd_logout = '1';
  location.reload();
});
$('#guest')?.addEventListener('click', (e) => {
  e.preventDefault();
  IDENTITY = localStorage.frayd_identity ||= location.hostname + ':' + crypto.randomUUID();
  enterApp();
});

// ---------- scan trigger ----------
const ADD_BTN_HTML = $('#add-media').innerHTML;
$('#add-media').onclick = async () => {
  const btn = $('#add-media');
  if (btn.disabled) return;
  btn.disabled = true;
  btn.textContent = 'Scanning…';
  try {
    const r = await fetch('/api/scan', { method: 'POST', headers: authHeaders() });
    if (r.status === 202) {
      toast('Scanning library…');
      // poll status until idle, then refresh library
      const poll = setInterval(async () => {
        const { scanning } = await (await fetch('/api/scan/status')).json();
        if (scanning) return;
        clearInterval(poll);
        const before = LIBRARY.length;
        LIBRARY = await (await fetch('/api/library')).json();
        renderRows(LIBRARY);
        toast(LIBRARY.length > before ? `Scan complete — ${LIBRARY.length - before} new item(s)` : 'Scan complete');
        btn.disabled = false;
        btn.innerHTML = ADD_BTN_HTML;
      }, 1500);
    } else if (r.status === 409) {
      toast('Scan already running', true);
      btn.disabled = false;
      btn.innerHTML = ADD_BTN_HTML;
    } else if (r.status === 501) {
      toast('No library folder configured — set it in settings', true);
      btn.disabled = false;
      btn.innerHTML = ADD_BTN_HTML;
    } else {
      toast(`Scan failed (${r.status})`, true);
      btn.disabled = false;
      btn.innerHTML = ADD_BTN_HTML;
    }
  } catch {
    toast('Scan failed — server unreachable', true);
    btn.disabled = false;
    btn.innerHTML = ADD_BTN_HTML;
  }
};

// ---------- nav ----------
document.querySelectorAll('.nav-item').forEach(btn => {
  btn.onclick = () => {
    document.querySelectorAll('.nav-item').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    filtering = btn.dataset.kind !== 'all';
    if (btn.dataset.kind !== 'show') showsDrill = null;
    if (!authorDrill[btn.dataset.kind]) { /* keep */ }
    else authorDrill[btn.dataset.kind] = null;
    filterKind(btn.dataset.kind);
  };
});
let searchTimer;
$('#search input').addEventListener('input', (e) => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => serverSearch(e.target.value), 250);
});

async function serverSearch(q) {
  filtering = !!q;
  if (!q) { filterKind(currentKind()); return; }
  try {
    const results = await (await fetch(`/api/library?q=${encodeURIComponent(q)}`)).json();
    renderRows(results);
  } catch { filterKind(currentKind(), q); }
}

function currentKind() {
  const active = document.querySelector('.nav-item.active');
  return active ? active.dataset.kind : 'all';
}

// ---------- library ----------
let LIBRARY = [];
let filtering = false; // suppress continue-watching row while a nav filter/search is active
window.getLibrary = () => LIBRARY;

async function loadLibrary() {
  LIBRARY = await (await fetch('/api/library')).json();
  renderRows(LIBRARY);
}

const KIND_ALIAS = { show: 'episode' };
function filterKind(kind, query) {
  let items = LIBRARY;
  const k = KIND_ALIAS[kind] || kind;
  if (k && k !== 'all') items = items.filter(i => i.kind === k);
  if (query) {
    const q = query.toLowerCase();
    items = items.filter(i =>
      (i.title || '').toLowerCase().includes(q) || (i.author || '').toLowerCase().includes(q));
  }
  renderRows(items);
}

function resumeFor(id) {
  // identity query needed for guest/LAN mode (no token); server auto-creates it
  return fetch(`/api/progress/${id}?identity=${encodeURIComponent(IDENTITY)}`, { headers: authHeaders() })
    .then(r => r.ok ? r.json() : null).catch(() => null);
}

function card(it, portrait) {
  const c = el('div', 'media-card ' + (portrait ? 'portrait' : 'landscape'));
  const isVideo = it.kind === 'movie' || it.kind === 'show';
  const isPaged = it.kind === 'comic' || it.kind === 'manga';
  c.title = `${it.title}${it.author ? ' — ' + it.author : ''}`;
  if (account?.isAdmin) {
    const del = el('button', 'card-del');
    del.setAttribute('aria-label', 'Remove from library');
    del.innerHTML = '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/></svg>';
    del.onclick = async (e) => {
      e.stopPropagation();
      if (!confirm(`Remove "${it.title}" from the library?\n(The file on disk is not deleted.)`)) return;
      const r = await fetch(`/api/media/${it.id}?identity=${encodeURIComponent(IDENTITY)}`, { method: 'DELETE', headers: authHeaders() });
      if (r.ok) { toast('Removed from library'); LIBRARY = LIBRARY.filter(x => x.id !== it.id); renderRows(LIBRARY); }
      else toast(r.status === 403 ? 'Admin access required' : 'Remove failed', true);
    };
    c.appendChild(del);
  }
  c.innerHTML = `
    ${isVideo
      ? `<img class="thumb" loading="lazy" src="/api/thumb/${it.id}" onerror="this.outerHTML='<div class=&quot;thumb-fallback${portrait?' portrait':''}&quot;>▶</div>'">`
      : `<div class="thumb-fallback${portrait ? ' portrait' : ''}">${it.kind === 'music' ? '♪' : it.kind === 'audiobook' ? '🎙' : isPaged ? '📄' : '📖'}</div>`}
    <div class="card-body">
      <div class="title">${esc(it.title)}</div>
      <div class="sub">${esc(it.author || it.kind)}</div>
    </div>
    <div class="progress-strip" style="display:none"><div></div></div>`;
  resumeFor(it.id).then(p => {
    if (p && p.position > 5 && it.duration_secs) {
      const frac = Math.min(1, p.position / it.duration_secs);
      const strip = c.querySelector('.progress-strip');
      strip.style.display = 'block';
      strip.firstElementChild.style.width = (frac * 100).toFixed(1) + '%';
    }
  });
  c.onclick = () => openItem(it);
  return c;
}

function section(title, items, portrait) {
  const wrap = el('section');
  const cards = el('div', 'cards');
  let shown = [...items];
  const head = el('div', 'row-head', `<h3>${esc(title)}</h3>`);
  const sortBtn = el('button', 'link', 'oldest first');
  sortBtn.onclick = () => {
    const oldest = sortBtn.textContent === 'oldest first';
    shown.sort((a, b) => oldest ? (a.created_at || 0) - (b.created_at || 0) : (b.created_at || 0) - (a.created_at || 0));
    sortBtn.textContent = oldest ? 'newest first' : 'oldest first';
    cards.innerHTML = '';
    shown.forEach(it => cards.appendChild(card(it, portrait)));
  };
  head.appendChild(sortBtn);
  shown.forEach(it => cards.appendChild(card(it, portrait)));
  wrap.append(head, cards);
  return wrap;
}

async function continueRow() {
  // in-progress items, most recently touched first
  const items = LIBRARY.filter(i => i.kind !== 'ebook' && i.kind !== 'comic' && i.kind !== 'manga');
  const withP = await Promise.all(items.map(async it => {
    const p = await resumeFor(it.id);
    return p && p.position > 5 && it.duration_secs && p.position < it.duration_secs * 0.95
      ? { it, frac: Math.min(1, p.position / it.duration_secs) } : null;
  }));
  const inProgress = withP.filter(Boolean);
  if (!inProgress.length) return null;
  inProgress.sort((a, b) => b.frac - a.frac);
  const wrap = el('section');
  wrap.appendChild(el('div', 'row-head', '<h3>Continue watching</h3>'));
  const cards = el('div', 'cards');
  inProgress.forEach(({ it, frac }) => {
    const c = card(it, false);
    const strip = c.querySelector('.progress-strip');
    if (strip) { strip.style.display = 'block'; strip.firstElementChild.style.width = (frac * 100).toFixed(1) + '%'; }
    cards.appendChild(c);
  });
  wrap.appendChild(cards);
  return wrap;
}

// TV Shows: series grid → click → season/episode view for that series
let showsDrill = null; // series name while drilled in

function groupedByShow(items) {
  if (showsDrill) return showDetail(showsDrill, items.filter(i => (i.series || 'Unknown series') === showsDrill));
  const wrap = el('section');
  const bySeries = {};
  items.forEach(i => (bySeries[i.series || 'Unknown series'] ||= []).push(i));
  const grid = el('div', 'cards show-grid');
  for (const [show, eps] of Object.entries(bySeries)) {
    const c = el('div', 'media-card landscape');
    c.innerHTML = `
      <img class="thumb" loading="lazy" src="/api/thumb/${eps[0].id}" alt=""
           onerror="this.outerHTML='<div class=&quot;thumb-fallback&quot;>${esc(show[0] || '?')}</div>'">
      <div class="card-body">
        <div class="title">${esc(show)}</div>
        <div class="sub">${eps.length} episode${eps.length === 1 ? '' : 's'}</div>
      </div>`;
    c.onclick = () => { showsDrill = show; filterKind('show'); };
    grid.appendChild(c);
  }
  wrap.appendChild(grid);
  return wrap;
}

function showDetail(show, eps) {
  const wrap = el('section');
  const back = el('button', 'link back-row', '← All shows');
  back.onclick = () => { showsDrill = null; filterKind('show'); };
  wrap.appendChild(back);
  wrap.appendChild(el('div', 'row-head', `<h3>${esc(show)}</h3>`));
  // season sub-grouping: parse SxxEyy from title
  const bySeason = {};
  eps.forEach(e => {
    const m = (e.title || '').toLowerCase().match(/s(\d{1,2})e/);
    (bySeason[m ? +m[1] : 0] ||= []).push(e);
  });
  for (const [season, seasonEps] of Object.entries(bySeason).sort((a, b) => a[0] - b[0])) {
    if (+season > 0) wrap.appendChild(el('div', 'row-head series-head', `<h4>Season ${+season}</h4>`));
    const cards = el('div', 'cards');
    seasonEps.sort((a, b) => (a.title || '').localeCompare(b.title || '', undefined, { numeric: true }));
    seasonEps.forEach(it => cards.appendChild(card(it, false)));
    wrap.appendChild(cards);
  }
  return wrap;
}

async function renderRows(items) {
  const main = $('#main');
  main.innerHTML = '<div id="hero"></div>';
  // hero: first movie with backdrop, else first item
  const heroItem = items.find(i => i.kind === 'movie') || items[0];
  if (heroItem) renderHero(heroItem);
  if (!filtering) {
    const cont = await continueRow();
    if (cont) main.appendChild(cont);
  }
  const byKind = {};
  // DB kind 'episode' renders as the 'show' module
  items.forEach(i => (byKind[i.kind === 'episode' ? 'show' : i.kind] ||= []).push(i));
  const order = ['movie', 'show', 'music', 'audiobook', 'ebook', 'comic', 'manga'];
  const soloKind = filtering && currentKind() !== 'all' && byKind[order.find(k => byKind[k]?.length)] && order.filter(k => byKind[k]?.length).length === 1;
  const solo = soloKind ? order.find(k => byKind[k]?.length) : null;
  for (const kind of order) {
    const kindItems = byKind[kind];
    if (!kindItems?.length) continue;
    if (kind === 'show') {
      main.appendChild(groupedByShow(kindItems));
    } else if (kind === 'audiobook' || kind === 'ebook' || kind === 'music') {
      main.appendChild(groupedByAuthor(kind, kindItems, solo === kind));
    } else {
      main.appendChild(section(kind[0].toUpperCase() + kind.slice(1) + 's', kindItems, false));
    }
  }
  if (filtering && !items.length) { $('#hero').style.display = 'none'; }
  if (!items.length) {
    if (currentKind() !== 'all' || $('#search input').value) {
      main.appendChild(el('p', 'empty-state', 'Nothing matches that filter. Try “All”.'));
    } else {
      main.appendChild(el('p', 'empty-state',
        'No media found. Place files in the server’s media folder, then choose Scan library.<br>' +
        'Supported: video (mp4/mkv/avi…), music, audiobooks (m4b/mp3), epubs, comics (cbz/cbr), manga folders. ' +
        'Ask the server administrator if no media folder is configured.'));
    }
  }
}

// author → series → books: one section per author, sub-rows per series
// Audiobooks/Ebooks/Music: folder drill-down — author grid → (series grid) → items
// drill state per kind: { author, series }
const authorDrill = { audiobook: null, ebook: null, music: null };

function groupedByAuthor(kind, items, solo = false) {
  const label = kind[0].toUpperCase() + kind.slice(1) + 's';
  const wrap = el('section');
  const drill = authorDrill[kind];

  if (!drill) {
    // level 1: author folders
    const byAuthor = {};
    items.forEach(i => (byAuthor[i.author || 'Unknown'] ||= []).push(i));
    const grid = el('div', 'cards');
    for (const [author, authorItems] of Object.entries(byAuthor)) {
      const c = el('div', 'media-card landscape');
      c.innerHTML = `
        <div class="thumb-fallback folder-fallback">${esc(author[0] || '?')}</div>
        <div class="card-body">
          <div class="title">${esc(author)}</div>
          <div class="sub">${authorItems.length} item${authorItems.length === 1 ? '' : 's'}</div>
        </div>`;
      c.onclick = () => { authorDrill[kind] = { author }; filterKind(kind); };
      grid.appendChild(c);
    }
    wrap.appendChild(grid);
    return wrap;
  }

  const authorItems = items.filter(i => (i.author || 'Unknown') === drill.author);
  const back = el('button', 'link back-row', `← All ${label.toLowerCase()}`);
  back.onclick = () => { authorDrill[kind] = null; filterKind(kind); };
  wrap.appendChild(back);

  if (!drill.series) {
    // level 2: series folders within author (skip if author has no series)
    const bySeries = {};
    authorItems.forEach(i => (bySeries[i.series || ''] ||= []).push(i));
    const seriesKeys = Object.keys(bySeries).filter(Boolean);
    if (!seriesKeys.length) {
      // flat author: straight to items
      const cards = el('div', 'cards');
      authorItems.forEach(it => cards.appendChild(card(it, false)));
      wrap.appendChild(el('div', 'row-head', `<h3>${esc(drill.author)}</h3>`), cards);
      return wrap;
    }
    wrap.appendChild(el('div', 'row-head', `<h3>${esc(drill.author)}</h3>`));
    const grid = el('div', 'cards');
    for (const series of seriesKeys) {
      const seriesItems = bySeries[series];
      const c = el('div', 'media-card landscape');
      c.innerHTML = `
        <div class="thumb-fallback folder-fallback">${esc(series[0])}</div>
        <div class="card-body">
          <div class="title">${esc(series)}</div>
          <div class="sub">${seriesItems.length} item${seriesItems.length === 1 ? '' : 's'}</div>
        </div>`;
      c.onclick = () => { authorDrill[kind] = { ...drill, series }; filterKind(kind); };
      grid.appendChild(c);
    }
    wrap.appendChild(grid);
    return wrap;
  }

  // level 3: items in series
  const seriesItems = authorItems.filter(i => i.series === drill.series);
  const up = el('button', 'link back-row', `← ${drill.author}`);
  up.onclick = () => { authorDrill[kind] = { author: drill.author }; filterKind(kind); };
  wrap.appendChild(up);
  wrap.appendChild(el('div', 'row-head', `<h3>${esc(drill.series)}</h3>`));
  const cards = el('div', 'cards');
  seriesItems.forEach(it => cards.appendChild(card(it, false)));
  wrap.appendChild(cards);
  return wrap;
}

function fmtDuration(s) {
  if (!s) return '';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

function renderHero(it) {
  const hero = $('#hero');
  hero.style.display = 'block';
  const vk = it.kind === 'episode' ? 'show' : it.kind;
  const verb = ['movie', 'show'].includes(vk) ? 'Play' : ['comic', 'manga', 'ebook'].includes(vk) ? 'Read' : 'Listen';
  const blurb = it.synopsis || `Resume where you left off, or start fresh — progress syncs across your devices.`;
  const tags = [it.kind, fmtDuration(it.duration_secs), it.author].filter(Boolean)
    .map(t => `<span class="tag">${esc(t)}</span>`).join('');
  hero.innerHTML = `
    <div class="hero-backdrop" id="hero-backdrop"></div>
    <div class="hero-tags">${tags}</div>
    <div class="hero-copy">
      <h2>${esc(it.title)}</h2>
      <p>${esc(blurb)}</p>
      <div class="cta-row">
        <button class="btn-play" id="hero-play">▶ ${verb}</button>
        <button class="btn-ghost" id="hero-info">More info</button>
      </div>
    </div>`;
  // backdrop: full thumb, blurred + darkened behind copy
  const bd = $('#hero-backdrop');
  const img = new Image();
  img.onload = () => { bd.style.backgroundImage = `url(${img.src})`; hero.classList.add('has-backdrop'); };
  img.src = `/api/thumb/${it.id}`;
  $('#hero-play').onclick = () => openItem(it);
}

// ---------- playback ----------
const pbAudio = $('#pb-audio');

function openItem(it) {
  const kind = it.kind === 'episode' ? 'show' : it.kind;
  if (kind === 'movie' || kind === 'show') openVideo(it);
  else if (kind === 'music' || kind === 'audiobook') playAudio(it);
  else if (kind === 'comic' || kind === 'manga') openComic(it);
  else if (kind === 'ebook') openEbook(it);
}

// ---------- comic reader ----------
let comic = null; // { it, pages, page }

async function openComic(it) {
  const pages = await (await fetch(`/api/comic/${it.id}/pages`)).json();
  if (!Array.isArray(pages) || !pages.length) return;
  comic = { it, pages, page: 0 };
  $('#cm-title').textContent = it.title;
  $('#comic-modal').classList.add('on');
  resumeFor(it.id).then(p => { if (p) showComicPage(Math.min(Math.floor(p.position), pages.length - 1)); else showComicPage(0); });
}

function showComicPage(n) {
  if (!comic) return;
  comic.page = Math.max(0, Math.min(n, comic.pages.length - 1));
  $('#cm-img').src = `/api/comic/${comic.it.id}/page/${comic.page}`;
  $('#cm-pageinfo').textContent = `${comic.page + 1} / ${comic.pages.length}`;
  $('#cm-prev').disabled = comic.page === 0;
  $('#cm-next').disabled = comic.page === comic.pages.length - 1;
  syncProgress(comic.it.id, comic.page); // progress = page index per schema
}

$('#cm-prev').onclick = () => comic && showComicPage(comic.page - 1);
$('#cm-next').onclick = () => comic && showComicPage(comic.page + 1);
$('#cm-close').onclick = closeComic;
document.addEventListener('keydown', e => {
  if (!$('#comic-modal').classList.contains('on')) return;
  if (e.key === 'ArrowRight' || e.key === ' ') { e.preventDefault(); comic && showComicPage(comic.page + 1); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); comic && showComicPage(comic.page - 1); }
  else if (e.key === 'Escape') closeComic();
});
// ebook shares arrow/escape handling via separate listener
document.addEventListener('keydown', e => {
  if (!$('#ebook-modal').classList.contains('on')) return;
  if (e.key === 'ArrowRight' || e.key === ' ') { e.preventDefault(); ebookFlip(1); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); ebookFlip(-1); }
  else if (e.key === 'Escape') closeEbook();
});
function closeComic() {
  $('#comic-modal').classList.remove('on');
  if (comic) { syncProgress(comic.it.id, comic.page); comic = null; }
  $('#cm-img').src = '';
}

// ---------- ebook reader ----------
let ebook = null; // { it, spine, chunk }

async function openEbook(it) {
  const spine = await (await fetch(`/api/epub/${it.id}/spine`)).json();
  if (!Array.isArray(spine) || !spine.length) return;
  const annots = await fetch(`/api/annotations/${it.id}?identity=${encodeURIComponent(IDENTITY)}`, { headers: authHeaders() })
    .then(r => r.ok ? r.json() : []).catch(() => []);
  ebook = { it, spine, chunk: 0, annots, toc: [] };
  $('#em-title').textContent = it.title;
  $('#ebook-modal').classList.add('on');
  // TOC (best-effort)
  ebook.toc = await fetch(`/api/epub/${it.id}/toc`).then(r => r.ok ? r.json() : []).catch(() => []);
  resumeFor(it.id).then(p => {
    const ch = p ? Math.min(Math.floor(p.position), spine.length - 1) : 0;
    const frac = p ? p.position - Math.floor(p.position) : 0;
    showChunk(ch, { frac });
  });
}

async function showChunk(n, opts = {}) {
  if (!ebook) return;
  ebook.chunk = Math.max(0, Math.min(n, ebook.spine.length - 1));
  ebook.pendingCol = opts.atEnd ? Infinity : opts.frac != null ? opts.frac : 0;
  ebook.lastFrac = opts.atEnd ? 0.999 : (opts.frac ?? 0);
  const chunkUrl = ebook.spine[ebook.chunk]; // zip-relative path
  const base = chunkUrl.includes('/') ? chunkUrl.slice(0, chunkUrl.lastIndexOf('/') + 1) : '';
  try {
    let html = await (await fetch(`/api/epub/${ebook.it.id}/chunk/${ebook.chunk}`)).text();
    const PW = ebookPageW(), PAD = 28; // page margin; column gap must be 2*PAD (see stride below)
    const FRAME_H = Math.round((document.getElementById('em-frame')?.clientHeight || window.innerHeight * 0.78) / window.innerHeight * 100);
    // rewrite relative src/href to asset endpoint (skip absolute, data:, #anchors, mailto)
    html = html.replace(/(src|href)="([^"]*)"/g, (m, attr, url) => {
      if (/^(https?:|data:|#|mailto:|javascript:)/i.test(url)) return m;
      const resolved = base + url.replace(/^\.\//, '');
      return `${attr}="/api/epub/${ebook.it.id}/asset/${resolved}"`;
    });
    // paginate: CSS multi-column; flip via injected script postMessage (sandbox-safe)
    html = html.replace(/<\/head>/i,
      `<style>
        /* multicol on body (html overflows). padding both sides: every in-flow page
           gets symmetric 28px margins, stride stays exact */
        html{overflow:hidden}
        body{column-width:${PW - 2 * PAD}px;column-gap:${2 * PAD}px;column-fill:auto;
          height:${FRAME_H}vh;width:${PW}px;padding:${PAD}px;
          margin:0;box-sizing:border-box;font-family:Georgia,serif;
          text-align:justify;overflow-wrap:break-word}
        img{max-width:100%}
      </style>
      <script>
        const PW = ${PW}, PAD = ${PAD}; // inner page padding (visual margin, kills edge-butt)
        const STEP = ${PW}; // column (PW-2*PAD) + gap (2*PAD) = exactly PW: viewport = one page, no bleed
        let col = 0;
        function report() { parent.postMessage({type:'page', page: col + 1, pages: pageCount()}, '*'); }
        function pageCount() {
          const pages = Math.round((document.documentElement.scrollWidth - ${PW}) / STEP) + 1;
          return Math.max(1, pages - 1); // minus the blank filler column
        }
        // scroll clamp/reflow guard: re-apply until scrollLeft actually matches col*STEP
        let snapT = null;
        function snap() {
          const x = Math.round(col * STEP);
          if (Math.abs(window.scrollX - x) <= 1) return;
          window.scrollTo({ left: x, behavior: 'instant' });
          setTimeout(snap, 60); // layout (fonts/images) can clamp mid-scroll; re-apply
        }
        window.addEventListener('message', (e) => {
          if (e.data === 'next' || e.data === 'prev') {
            const last = pageCount() - 1;
            if (e.data === 'next') {
              if (col >= last) parent.postMessage({type:'chapter', dir:1}, '*');
              else { col++; snap(); report(); }
            } else {
              if (col <= 0) parent.postMessage({type:'chapter', dir:-1}, '*');
              else { col--; snap(); report(); }
            }
          } else if (typeof e.data === 'object' && e.data?.type === 'goto') {
            col = Math.max(0, Math.min(e.data.col, pageCount() - 1));
            snap(); report();
          }
        });
        window.addEventListener('load', () => {
          // tail page: scroller excludes body's right padding from max scroll.
          // spacer forces ONE EXTRA overflow column (column-break + 100% height so
          // it can't fit inside the last text column) -> extends scrollWidth by a
          // full stride, making the last page's margin reachable
          const sp = document.createElement('div');
          sp.style.cssText = 'width:1px;height:100%;break-before:column';
          document.body.appendChild(sp);
          // resume position: parent messages {type:'goto', col} after load
          col = 0; report();
        });
        // parent-driven resume: apply once, after first page report (layout settled)
        window.addEventListener('message', (e) => {
          if (typeof e.data === 'object' && e.data?.type === 'resume') {
            const lastReal = pageCount(); // filler excluded from count
            const c = e.data.col === 'end' ? lastReal - 1
              : Math.round(e.data.col * (lastReal - 1)); // fraction → column
            col = Math.max(0, Math.min(c, lastReal - 1));
            snap(); report();
          } else if (e.data?.type === 'inject-hl') {
            // highlight restore runs HERE (sandbox: parent can't touch our DOM)
            const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
            const nodes = [];
            while (walker.nextNode()) nodes.push(walker.currentNode);
            let pos = 0;
            const offs = nodes.map(n => { const st = pos; pos += n.nodeValue.length; return { n, st }; });
            for (const h of [...e.data.items].sort((a, b) => b.off - a.off)) {
              for (const { n, st } of offs) {
                const en = st + n.nodeValue.length;
                if (h.off >= st && h.off + h.len <= en) {
                  const r = document.createRange();
                  r.setStart(n, h.off - st);
                  r.setEnd(n, h.off - st + h.len);
                  const m = document.createElement('mark');
                  m.className = 'user-highlight';
                  m.style.setProperty('--hl', h.color);
                  try { r.surroundContents(m); } catch {}
                  break;
                }
              }
            }
          }
        });
        // geometry readback: parent asks with {type:'geom'} → returns real numbers
        window.addEventListener('message', (e) => {
          if (typeof e.data === 'object' && e.data?.type === 'geom') {
            const b = document.documentElement;
            const lastText = [...document.body.children].filter(n => n.nodeType === 1).pop();
            parent.postMessage({ type: 'geom', innerWidth: window.innerWidth,
              scrollWidth: b.scrollWidth, clientWidth: b.clientWidth,
              scrollLeft: window.scrollX, col, STEP,
              bodyPad: getComputedStyle(b).paddingLeft + '/' + getComputedStyle(b).paddingRight }, '*');
          }
        });
        // in-chapter search: wrap matches in <mark>, report count + positions
        window.addEventListener('message', (e) => {
          if (typeof e.data === 'object' && e.data?.type === 'search') {
            document.querySelectorAll('mark.search-hit').forEach(m => {
              const p = m.parentNode;
              p.replaceChild(document.createTextNode(m.textContent), m);
              p.normalize();
            });
            const q = (e.data.query || '').toLowerCase();
            const hits = [];
            if (q) {
              const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
              const nodes = [];
              while (walker.nextNode()) nodes.push(walker.currentNode);
              for (const node of nodes) {
                const text = node.nodeValue;
                const lower = text.toLowerCase();
                let idx = lower.indexOf(q);
                while (idx !== -1) {
                  hits.push({node, idx, len: q.length});
                  idx = lower.indexOf(q, idx + q.length);
                }
              }
              for (const h of hits.reverse()) {
                const range = document.createRange();
                range.setStart(h.node, h.idx);
                range.setEnd(h.node, h.idx + h.len);
                const mark = document.createElement('mark');
                mark.className = 'search-hit';
                try { range.surroundContents(mark); } catch {}
              }
              if (hits.length) {
                const first = document.querySelector('mark.search-hit');
                const pw = PW;
                window.scrollTo({ left: Math.round(first.getBoundingClientRect().left + window.scrollX - PW / 2), behavior: 'instant' });
              }
            }
            parent.postMessage({type:'search-results', count: hits.length}, '*');
          }
        });
        window.addEventListener('message', (e) => {
          if (e.data === 'search-next') {
            const marks = [...document.querySelectorAll('mark.search-hit')];
            if (!marks.length) return;
            const cur = marks.findIndex(m => m.classList.contains('current'));
            marks.forEach(m => m.classList.remove('current'));
            const nx = marks[(cur + 1) % marks.length];
            nx.classList.add('current');
            const pw = PW;
            window.scrollTo({ left: Math.round(nx.getBoundingClientRect().left + window.scrollX - PW / 2), behavior: 'instant' });
            parent.postMessage({type:'search-hit', idx: (cur + 1) % marks.length + 1, total: marks.length}, '*');
          }
        });
        const searchStyle = document.createElement('style');
        searchStyle.textContent = 'mark.search-hit{background:#ffd54f;color:#000} mark.search-hit.current{background:#ff8f00;color:#000}'
          + 'mark.user-highlight{background:var(--hl,#ffd54f);color:inherit;border-radius:2px}'
          + '.hl-popup{position:absolute;background:#1b1d26;color:#fff;border:1px solid #5B46F6;border-radius:8px;'
          + 'padding:8px 12px;font:13px sans-serif;z-index:99;box-shadow:0 4px 16px rgba(0,0,0,.5)}'
          + '.hl-popup button{margin-left:8px;background:#5B46F6;border:none;color:#fff;border-radius:6px;padding:4px 10px;cursor:pointer}';
        document.head.appendChild(searchStyle);
        // text selection → highlight: report serialized range to parent
        document.addEventListener('mouseup', () => {
          const sel = window.getSelection();
          if (!sel || sel.isCollapsed || !sel.rangeCount) return;
          const range = sel.getRangeAt(0);
          const text = sel.toString().trim();
          if (!text) return;
          // wrap immediately
          const mark = document.createElement('mark');
          mark.className = 'user-highlight';
          try { range.surroundContents(mark); } catch { return; }
          sel.removeAllRanges();
          // serialize: count preceding chars in body text
          const pre = document.createRange();
          pre.selectNodeContents(document.body);
          pre.setEnd(range.startContainer, range.startOffset);
          const offset = pre.toString().length;
          parent.postMessage({type:'highlight', cfi: 'char:' + offset + ':' + text.length, text, page: Math.round(window.scrollX / PW)}, '*');
        });
      <\/script></head>`);
    // re-apply saved highlights for this chapter (char-offset based)
    const saved = (ebook.annots || []).filter(a => a.kind === 'highlight' && a.spine_idx === ebook.chunk);
    const inject = saved.map(a => {
      const m = a.cfi.match(/^char:(\d+):(\d+)$/);
      return m ? { off: +m[1], len: +m[2], color: a.color || 'yellow' } : null;
    }).filter(Boolean).sort((a, b) => b.off - a.off); // reverse: offsets stay valid
    if (inject.length) {
      html = html.replace(/<body[^>]*>/i, (m) => m + '<span id="__hlroot"></span>');
    }
    $('#em-frame').srcdoc = html;
    // resume position after iframe layout settles (want 'end' = last real page)
    const want = ebook.pendingCol;
    ebook.pendingCol = 0;
    $('#em-frame').addEventListener('load', () => {
      setTimeout(() => {
        $('#em-frame').contentWindow?.postMessage({ type: 'resume', col: want === Infinity ? 'end' : want }, '*'); // want: 'end' | fraction-of-chapter 0..1
      }, 80);
    }, { once: true });
    // highlights injected via postMessage (sandbox-safe; same job as before)
    if (inject.length) {
      const items = inject.map(h => ({ off: h.off, len: h.len, color: h.color }));
      const send = () => $('#em-frame').contentWindow?.postMessage({ type: 'inject-hl', items }, '*');
      setTimeout(send, 100); // after resume/first layout
    }
  } catch {
    $('#em-frame').srcdoc = '<p style="font-family:sans-serif;color:#333">Failed to load chapter.</p>';
  }
}

// page-flip: injected iframe script reports page state via postMessage
function ebookPageW() { const f = document.getElementById("em-frame"); return Math.round(f ? f.clientWidth : Math.min(window.innerWidth * 0.85, 760)); }
function ebookFlip(dir) {
  if (!ebook) return;
  $('#em-frame').contentWindow?.postMessage(dir > 0 ? 'next' : 'prev', '*');
}
// resize: repaginate (reload chapter at same column); debounce 300ms
let ebookResizeT = null;
window.addEventListener('resize', () => {
  if (!ebook || !$('#ebook-modal').classList.contains('on')) return;
  clearTimeout(ebookResizeT);
  ebookResizeT = setTimeout(() => showChunk(ebook.chunk), 300);
});
window.addEventListener('message', (e) => {
  if (!ebook) return;
  if (e.data?.type === 'chapter') {
    const n = ebook.chunk + e.data.dir;
    if (n < 0 || n >= ebook.spine.length) return;
    // entering prev chapter from its top → land on its last real page
    showChunk(n, e.data.dir < 0 ? { atEnd: true } : {});
  } else if (e.data?.type === 'geom') {
    console.log('[ebook geom]', e.data);
  } else if (e.data?.type === 'highlight') {
    // persist selection as highlight
    fetch('/api/annotations', {
      method: 'POST',
      headers: authHeaders({ 'Content-Type': 'application/json' }),
      body: JSON.stringify({
        media_id: ebook.it.id, kind: 'highlight',
        spine_idx: ebook.chunk, cfi: e.data.cfi,
        text: e.data.text, color: 'yellow'
      })
    });
    $('#em-searchinfo').textContent = 'Highlighted';
    setTimeout(() => { if (ebook) $('#em-searchinfo').textContent = ''; }, 1500);
  } else if (e.data?.type === 'page') {
    $('#em-pageinfo').textContent = `${ebook.chunk + 1} · p${e.data.page}/${e.data.pages}`;
    // progress = chunk + fractional page (fraction-of-chapter, resolution-independent)
    const frac = e.data.pages > 1 ? (e.data.page - 1) / (e.data.pages - 1) : 0;
    ebook.lastFrac = Math.min(frac, 0.999);
    syncProgress(ebook.it.id, ebook.chunk + ebook.lastFrac);
  } else if (e.data?.type === 'search-results') {
    ebook.searchHits = e.data.count;
    $('#em-searchinfo').textContent = e.data.count
      ? `${e.data.count} hits — Enter for next` : 'No hits';
  } else if (e.data?.type === 'search-hit') {
    $('#em-searchinfo').textContent = `${e.data.idx}/${e.data.total} in ch.`;
  }
});

// ebook search UI
function ebookSearch(q) {
  if (!ebook) return;
  ebook.searchQuery = q;
  $('#em-frame').contentWindow?.postMessage({ type: 'search', query: q }, '*');
}
$('#em-search').addEventListener('keydown', (e) => {
  if (e.key !== 'Enter') return;
  e.preventDefault();
  const q = e.target.value.trim();
  if (!q) { $('#em-searchinfo').textContent = ''; return; }
  if (q !== ebook.searchQuery) { ebookSearch(q); return; }
  // same query: jump to next hit in current chapter, else next chapter
  ebookSearchNext();
});
function ebookSearchNext() {
  const w = $('#em-frame').contentWindow;
  w?.postMessage({ type: 'search-next' }, '*');
}

$('#em-prev').onclick = () => ebookFlip(-1);
$('#em-next').onclick = () => ebookFlip(1);
// reader shortcuts: ←/→/space flip, s search, t toc, f fullscreen, Esc closes overlays
document.addEventListener('keydown', (e) => {
  if (!$('#ebook-modal').classList.contains('on')) return;
  if (e.target.tagName === 'INPUT') return;
  if (e.key === 'ArrowRight' || e.key === ' ') { e.preventDefault(); ebookFlip(1); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); ebookFlip(-1); }
  else if (e.key === 's') { e.preventDefault(); $('#em-search').focus(); }
  else if (e.key === 't') { renderToc(); $('#em-toc').classList.toggle('on'); }
  else if (e.key === 'f') { $('#em-toc').classList.remove('on'); document.fullscreenElement ? document.exitFullscreen() : $('#ebook-modal').requestFullscreen?.(); }
  else if (e.key === 'Escape' && $('#em-toc').classList.contains('on')) $('#em-toc').classList.remove('on');
});
$('#em-close').onclick = closeEbook;
// TOC drawer
$('#em-toc-btn').onclick = () => { renderToc(); $('#em-toc').classList.toggle('on'); };
$('#em-prev').ondblclick = $('#em-next').ondblclick = () =>
  $('#em-frame').contentWindow?.postMessage({ type: 'geom' }, '*');
function renderToc() {
  const el = $('#em-toc');
  if (!ebook?.toc?.length) { el.innerHTML = '<h3>No table of contents</h3>'; return; }
  el.innerHTML = '<h3>Contents</h3>' + ebook.toc.map((t, i) =>
    `<button data-i="${t.spine_idx}" class="${t.spine_idx === ebook.chunk ? 'cur' : ''}">${esc(t.title)}</button>`).join('');
  el.querySelectorAll('button').forEach(b => b.onclick = () => {
    showChunk(+b.dataset.i);
    $('#em-toc').classList.remove('on');
  });
}
function closeEbook() {
  $('#ebook-modal').classList.remove('on');
  if (ebook) { syncProgress(ebook.it.id, ebook.chunk + (ebook.lastFrac ?? 0)); ebook = null; }
  $('#em-frame').src = 'about:blank';
  $('#em-search').value = '';
  $('#em-searchinfo').textContent = '';
}

function playAudio(it) {
  $('#video-modal').classList.remove('on', 'yt');
  $('#playerbar').classList.add('on');
  if (window.partySend) { partyLoadPending = true; partyLoadId = it.id; partyLoadTitle = it.title; }
  $('#pb-title').textContent = it.title;
  $('#pb-sub').textContent = it.author || it.kind;
  resumeFor(it.id).then(p => {
    pbAudio.src = `/api/stream/${it.id}`;
    const seek = (p && p.position > 5) ? p.position : 0;
    const onMeta = () => { if (seek) pbAudio.currentTime = seek; pbAudio.play(); };
    if (pbAudio.readyState >= 1) onMeta();
    else pbAudio.addEventListener('loadedmetadata', onMeta, { once: true });
  });
  trackProgress(pbAudio, it);
}

let progressTimer = null;
function trackProgress(media, it) {
  clearInterval(progressTimer);
  let last = 0;
  progressTimer = setInterval(() => {
    if (media.paused) return;
    if (media.currentTime - last >= 10) {
      last = media.currentTime;
      syncProgress(it.id, media.currentTime);
    }
  }, 1000);
  media.onpause = () => syncProgress(it.id, media.currentTime);
  media.onended = () => { clearInterval(progressTimer); syncProgress(it.id, media.duration); };
}

// ---------- party sync ----------
// guest: follow host playback state (drives bottom audio bar)
const vidEl = document.getElementById('plyr-video');
const isVideoId = (id) => LIBRARY.some(x => x.id === id && (x.kind === 'movie' || x.kind === 'show'));

// ---- YouTube party playback (guests + host-load mirror) ----
// loads YT iframe API once; ytPlayer mirrors host state. Looser drift (YT seek latency ~2s).
let ytPlayer = null, ytId = null, ytReady = false, ytPending = null;
const YT_DRIFT = 3;
window.partyFollowExternal = (vid, title, position, playing, updatedAt, isHostOwn) => {
  const expected = () => position + (playing ? Math.max(0, Date.now() / 1000 - updatedAt) : 0);
  $('#video-modal').classList.add('on');
  $('#video-modal').classList.add('yt');
  $('#pb-title').textContent = title || 'YouTube';
  $('#pb-sub').textContent = 'party · youtube';
  const ensure = (cb) => {
    if (ytReady) return cb();
    if (!window.YT) {
      const t = document.createElement('script');
      t.src = 'https://www.youtube.com/iframe_api';
      document.head.appendChild(t);
      window.onYouTubeIframeAPIReady = () => { ytReady = true; };
    }
    const iv = setInterval(() => { if (ytReady) { clearInterval(iv); cb(); } }, 200);
  };
  ensure(() => {
    if (ytId !== vid) {
      ytId = vid;
      if (ytPlayer) ytPlayer.destroy();
      ytPlayer = new YT.Player('yt-party', {
        videoId: vid,
        playerVars: { autoplay: 0, playsinline: 1 },
        events: {
          onReady: () => { ytPlayer.seekTo(expected(), true); if (playing) ytPlayer.playVideo(); },
          // host: mirror player state into party broadcasts
          onStateChange: (e) => {
            if (!isHostOwn || !window.partySend) return;
            if (e.data === 1) window.partySend({ type: 'play' });
            else if (e.data === 2) window.partySend({ type: 'pause' });
            else if (e.data === 0 && window.partySend) window.partySend({ type: 'pause' });
          }
        }
      });
      // host: periodic seek mirror (YT has no 'seeked' event)
      if (isHostOwn) {
        clearInterval(window._ytSeekTimer);
        window._ytSeekTimer = setInterval(() => {
          if (!ytPlayer || !ytPlayer.getCurrentTime || !window.partySend) return;
          if (ytPlayer.getPlayerState() === 1) window.partySend({ type: 'seek', position: ytPlayer.getCurrentTime() });
        }, 2000);
      }
      return;
    }
    if (!ytPlayer || !ytPlayer.seekTo) return;
    const cur = ytPlayer.getCurrentTime ? ytPlayer.getCurrentTime() : 0;
    const drift = Math.abs(cur - expected());
    if (playing && drift > YT_DRIFT) ytPlayer.seekTo(expected(), true);
    if (playing && ytPlayer.getPlayerState && ytPlayer.getPlayerState() !== 1) ytPlayer.playVideo();
    if (!playing && ytPlayer.getPlayerState && ytPlayer.getPlayerState() === 1) ytPlayer.pauseVideo();
  });
  void isHostOwn;
};

window.partyFollow = (mediaId, title, position, playing, updatedAt) => {
  const expected = () => position + (playing ? Math.max(0, Date.now() / 1000 - updatedAt) : 0);
  // video: load into Plyr modal, mirror play/pause/seek on the <video> element
  if (isVideoId(mediaId)) {
    if (partyFollowId !== mediaId) {
      partyFollowId = mediaId;
      pbAudio.pause();
      vidEl.src = `/api/stream/${mediaId}`;
      const onMeta = () => { vidEl.currentTime = expected(); if (playing) vidEl.play().catch(() => {}); };
      if (vidEl.readyState >= 1) onMeta();
      else vidEl.addEventListener('loadedmetadata', onMeta, { once: true });
      $('#video-modal').classList.add('on');
    } else {
      const drift = Math.abs(vidEl.currentTime - expected());
      if (drift > 2) vidEl.currentTime = expected();
      if (playing && vidEl.paused) vidEl.play().catch(() => {});
      if (!playing && !vidEl.paused) vidEl.pause();
    }
    return;
  }
  // already playing this media locally (host loaded it themselves) — nothing to do
  if (pbAudio.src.endsWith(`/api/stream/${mediaId}`)) { partyFollowId = mediaId; }
  if (partyFollowId !== mediaId) {
    partyFollowId = mediaId;
    $('#playerbar').classList.add('on');
    $('#pb-title').textContent = title || '—';
    $('#pb-sub').textContent = 'party';
    pbAudio.src = `/api/stream/${mediaId}`;
    const onMeta = () => { pbAudio.currentTime = expected(); if (playing) pbAudio.play().catch(() => {}); };
    if (pbAudio.readyState >= 1) onMeta();
    else pbAudio.addEventListener('loadedmetadata', onMeta, { once: true });
    return;
  }
  const drift = Math.abs(pbAudio.currentTime - expected());
  if (drift > 1.5) pbAudio.currentTime = expected(); // resync
  if (playing && pbAudio.paused) pbAudio.play().catch(() => {});
  if (!playing && !pbAudio.paused) pbAudio.pause();
};
let partyFollowId = null;

// host: local player events push party state
window.partyHostHook = (send) => {
  pbAudio.addEventListener('play', () => send({ type: 'play' }));
  pbAudio.addEventListener('pause', () => send({ type: 'pause' }));
  pbAudio.addEventListener('seeked', () => send({ type: 'seek', position: pbAudio.currentTime }));
  // host loads a track locally → broadcast it
  pbAudio.addEventListener('loadedmetadata', () => {
    if (window.partySend && partyLoadPending) {
      partyLoadPending = false;
      send({ type: 'load', media_id: partyLoadId, title: partyLoadTitle, position: pbAudio.currentTime });
    }
  });
  // host video: mirror Plyr play/pause/seek + load
  vidEl.addEventListener('play', () => send({ type: 'play' }));
  vidEl.addEventListener('pause', () => send({ type: 'pause' }));
  vidEl.addEventListener('seeked', () => send({ type: 'seek', position: vidEl.currentTime }));
  vidEl.addEventListener('loadedmetadata', () => {
    if (window.partySend && videoLoadPending && videoItem) {
      videoLoadPending = false;
      send({ type: 'load', media_id: videoItem.id, title: videoItem.title, position: vidEl.currentTime });
    }
  });
};
let partyLoadPending = false, partyLoadId = null, partyLoadTitle = null;
let videoLoadPending = false;

function syncProgress(mediaId, position) {
  fetch('/api/progress', {
    method: 'POST',
    headers: authHeaders({ 'Content-Type': 'application/json' }),
    body: JSON.stringify({ identity: IDENTITY, media_id: mediaId, position })
  }).then(r => { if (!r.ok) console.warn('progress save failed', r.status); }).catch(() => {});
}

// bottom bar wiring
$('#pb-play').onclick = () => pbAudio.paused ? pbAudio.play() : pbAudio.pause();
pbAudio.onplay = () => $('#pb-play').textContent = '❚❚';
pbAudio.onpause = () => $('#pb-play').textContent = '▶';
pbAudio.ontimeupdate = () => {
  const dur = pbAudio.duration || 1;
  $('#pb-fill').style.width = (pbAudio.currentTime / dur * 100).toFixed(2) + '%';
  $('#pb-cur').textContent = fmt(pbAudio.currentTime);
  $('#pb-dur').textContent = fmt(pbAudio.duration);
};
$('#pb-bar').onclick = (e) => {
  const r = e.currentTarget.getBoundingClientRect();
  pbAudio.currentTime = (e.clientX - r.left) / r.width * (pbAudio.duration || 0);
};
$('#pb-vol').onclick = (e) => {
  const r = e.currentTarget.getBoundingClientRect();
  const v = Math.min(1, (e.clientX - r.left) / r.width);
  pbAudio.volume = v;
  $('#pb-vol .fill').style.width = (v * 100) + '%';
};
$('#pb-close').onclick = () => {
  pbAudio.pause();
  clearInterval(progressTimer);
  $('#playerbar').classList.remove('on');
};

function fmt(s) {
  if (!isFinite(s)) return '0:00';
  s = Math.floor(s);
  return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0');
}

// ---------- video (Plyr) ----------
const plyr = new Plyr('#plyr-video', {
  controls: ['play-large', 'play', 'progress', 'current-time', 'duration', 'mute', 'volume', 'settings', 'pip', 'airplay', 'fullscreen'],
  settings: ['speed'],
  seekTime: 10,
});
let videoItem = null;

function openVideo(it) {
  $('#playerbar').classList.remove('on');
  pbAudio.pause();
  videoItem = it;
  if (window.partySend) videoLoadPending = true;
  $('#vm-title').textContent = it.title;
  $('#video-modal').classList.add('on');
  $('#video-modal').classList.remove('yt');
  resumeFor(it.id).then(p => {
    plyr.source = {
      type: 'video',
      sources: [{ src: `/api/stream/${it.id}`, type: 'video/mp4' }],
    };
    if (p && p.position > 5) plyr.on('ready', () => { plyr.currentTime = p.position; }, { once: true });
    plyr.play();
  });
  trackProgress(plyr, it);
  startChromeAutoHide();
}

// auto-hide title/back/party when mouse idle over player
let chromeHideTimer = null;
function wakeChrome() {
  const ov = document.querySelector('.vm-overlay');
  if (!ov) return;
  ov.classList.remove('hidden');
  clearTimeout(chromeHideTimer);
  chromeHideTimer = setTimeout(() => {
    if ($('#video-modal').classList.contains('on')) ov.classList.add('hidden');
  }, 2500);
}
function startChromeAutoHide() {
  const modal = $('#video-modal');
  modal.onmousemove = wakeChrome;
  wakeChrome();
}

$('#vm-back').onclick = () => {
  if (plyr.currentTime > 0 && videoItem) syncProgress(videoItem.id, plyr.currentTime);
  plyr.stop();
  $('#video-modal').classList.remove('on');
  videoItem = null;
};
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') $('#vm-back').click();
});
$('#vm-party').onclick = () => {
  // party panel opens over the player
  const pm = $('#party-modal');
  pm.classList.add('on');
  pm.style.zIndex = 200;
};

// ---------- boot ----------
tryResume().then(resumed => {
  if (!resumed || localStorage.frayd_logout) {
    delete localStorage.frayd_logout;
    document.body.classList.add('auth');
    $('#login').style.display = 'flex';
  }
});
