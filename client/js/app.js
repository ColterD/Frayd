// Frayd web client — auth, dashboard, playback
'use strict';

const $ = (s) => document.querySelector(s);
const el = (tag, cls, html) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html != null) e.innerHTML = html;
  return e;
};
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, c =>
  ({ '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;' }[c]));

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
$('#settings-btn')?.addEventListener('click', async () => {
  $('#settings-modal').classList.add('on');
  const stage = $('#settings-stage');
  stage.innerHTML = '<div class="party-sec">Accounts & permissions</div>';
  try {
    const [perms, settings] = await Promise.all([
      fetch('/api/admin/perms', { headers: authHeaders() }).then(r => r.ok ? r.json() : Promise.reject(r.status)),
      fetch('/api/admin/settings', { headers: authHeaders() }).then(r => r.ok ? r.json() : {}),
    ]);
    stage.innerHTML = (perms.length ? perms.map(p => `
      <div class="perm-row" data-id="${p.identity_id}">
        <span class="k">${esc(p.key)}</span>
        <label><input type="checkbox" class="p-admin" ${p.is_admin ? 'checked' : ''}> admin</label>
        <select class="p-kinds" multiple size="1">
          ${KINDS.map(k => `<option value="${k}" ${p.allowed_kinds?.includes(k) ? 'selected' : ''}>${k}</option>`).join('')}
        </select>
        <button class="p-save">Save</button>
      </div>`).join('') : '<div class="party-sec">No accounts</div>')
      + '<div class="party-sec">Server settings</div>'
      + `<div class="perm-row"><span class="k">Site name</span><input class="s-site" value="${esc(settings.site_name ?? 'Frayd')}" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px"><button id="s-site-save">Save</button></div>`
      + `<div class="party-sec">Network <span class="s">(restart server to apply)</span></div>`
      + `<div class="perm-row"><span class="k">Port</span><input class="s-port" type="number" min="1024" max="65535" value="${esc(settings.port ?? 8484)}" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;width:90px"></div>`
      + `<div class="perm-row"><span class="k">UPnP port-forward</span><label><input type="checkbox" class="s-upnp" ${settings.upnp ? 'checked' : ''}> router must have UPnP on + 'upnpc' installed</label></div>`
      + `<div class="perm-row"><span class="k">HTTPS cert path (PEM)</span><input class="s-cert" value="${esc(settings.tls_cert ?? '')}" placeholder="/path/cert.pem" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"></div>`
      + `<div class="perm-row"><span class="k">HTTPS key path (PEM)</span><input class="s-key" value="${esc(settings.tls_key ?? '')}" placeholder="/path/key.pem" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"></div>`
      + `<div class="perm-row"><span class="k">Domain</span><input class="s-domain" value="${esc(settings.domain ?? '')}" placeholder="media.example.com" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"><span class="s">A record → your IP; use a real cert (e.g. Let's Encrypt) for this</span></div>`
      + `<div class="party-sec">Library</div>`
      + `<div class="perm-row"><span class="k">Media folder</span><input class="s-library" value="${esc(settings.library_path ?? '')}" placeholder="/home/you/Media" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"><button class="p-scan">Scan now</button></div>`
      + `<div class="perm-row" style="border:none"><span class="s">Suggested layout (any subset):</span></div>`
      + `<div class="perm-row" style="border:none"><pre class="s" style="color:#8A8F9E;font-size:12px;margin:0;line-height:1.5">Media/\n  Movies/    *.mp4 *.mkv\n  Shows/     S01E01 files\n  Music/     *.mp3 *.flac\n  Audiobooks/ *.m4b *.mp3\n  Ebooks/    *.epub\n  Comics/    *.cbz *.cbr\n  Manga/     folder of images under a *manga* dir</pre></div>`
      + `<button id="s-net-save" style="margin-top:10px;width:100%;background:#5B46F6;color:#fff;border:none;border-radius:8px;padding:10px;cursor:pointer">Save network settings</button>`;
    stage.querySelectorAll('.p-save').forEach(b => b.onclick = async () => {
      const row = b.closest('.perm-row');
      const kinds = [...row.querySelector('.p-kinds').selectedOptions].map(o => o.value);
      const r = await fetch('/api/admin/perms', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({
          identity_id: +row.dataset.id,
          is_admin: row.querySelector('.p-admin').checked,
          allowed_kinds: kinds.length ? kinds : null,
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
    // library: save path then scan (scan picks up new path without restart)
    const scanBtn = stage.querySelector('.p-scan');
    scanBtn.onclick = async () => {
      const path = stage.querySelector('.s-library').value.trim();
      const r1 = await fetch('/api/admin/settings', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ library_path: path || null }),
      });
      if (!r1.ok) { scanBtn.textContent = '✗ save'; return; }
      scanBtn.textContent = 'Scanning…';
      const r2 = await fetch('/api/scan', { method: 'POST', headers: authHeaders() });
      if (r2.status === 202) {
        const poll = setInterval(async () => {
          const s = await (await fetch('/api/scan/status')).json();
          if (!s.scanning) { clearInterval(poll); scanBtn.textContent = '✓ done'; loadLibrary(); setTimeout(() => { scanBtn.textContent = 'Scan now'; }, 2000); }
        }, 1000);
      } else {
        scanBtn.textContent = r2.status === 409 ? 'busy' : '✗ ' + r2.status;
        setTimeout(() => { scanBtn.textContent = 'Scan now'; }, 2000);
      }
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
      // poll status until idle, then refresh library
      const poll = setInterval(async () => {
        const { scanning } = await (await fetch('/api/scan/status')).json();
        if (scanning) return;
        clearInterval(poll);
        LIBRARY = await (await fetch('/api/library')).json();
        renderRows(LIBRARY);
        btn.disabled = false;
        btn.innerHTML = ADD_BTN_HTML;
      }, 1500);
    } else {
      btn.disabled = false;
      btn.innerHTML = ADD_BTN_HTML;
    }
  } catch {
    btn.disabled = false;
    btn.innerHTML = ADD_BTN_HTML;
  }
};

// ---------- nav ----------
document.querySelectorAll('.nav-item').forEach(btn => {
  btn.onclick = () => {
    document.querySelectorAll('.nav-item').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    filterKind(btn.dataset.kind);
  };
});
let searchTimer;
$('#search input').addEventListener('input', (e) => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => serverSearch(e.target.value), 250);
});

async function serverSearch(q) {
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

async function loadLibrary() {
  LIBRARY = await (await fetch('/api/library')).json();
  renderRows(LIBRARY);
}

function filterKind(kind, query) {
  let items = LIBRARY;
  if (kind && kind !== 'all') items = items.filter(i => i.kind === kind);
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
  const head = el('div', 'row-head',
    `<h3>${esc(title)}</h3><button class="link">view all</button>`);
  const cards = el('div', 'cards');
  items.forEach(it => cards.appendChild(card(it, portrait)));
  wrap.append(head, cards);
  return wrap;
}

function renderRows(items) {
  const main = $('#main');
  main.innerHTML = '<div id="hero"></div>';
  // hero: first movie with backdrop, else first item
  const heroItem = items.find(i => i.kind === 'movie') || items[0];
  if (heroItem) renderHero(heroItem);
  const byKind = {};
  items.forEach(i => (byKind[i.kind] ||= []).push(i));
  const order = ['movie', 'music', 'audiobook', 'ebook', 'comic', 'manga'];
  for (const kind of order) {
    const kindItems = byKind[kind];
    if (!kindItems?.length) continue;
    // books/audio: author → series → books grouping
    if (kind === 'audiobook' || kind === 'ebook' || kind === 'music') {
      main.appendChild(groupedByAuthor(kind, kindItems));
    } else {
      main.appendChild(section(kind[0].toUpperCase() + kind.slice(1) + 's', kindItems, false));
    }
  }
  if (!items.length) main.appendChild(el('p', '', '<br>No media found. Set FRAYD_LIBRARY and restart the server.'));
}

// author → series → books: one section per author, sub-rows per series
function groupedByAuthor(kind, items) {
  const label = kind[0].toUpperCase() + kind.slice(1) + 's';
  const wrap = el('section');
  const byAuthor = {};
  items.forEach(i => (byAuthor[i.author || 'Unknown'] ||= []).push(i));
  for (const [author, authorItems] of Object.entries(byAuthor)) {
    const head = el('div', 'row-head', `<h3>${esc(label)} — ${esc(author)}</h3>`);
    wrap.appendChild(head);
    const bySeries = {};
    authorItems.forEach(i => (bySeries[i.series || ''] ||= []).push(i));
    for (const [series, seriesItems] of Object.entries(bySeries)) {
      if (series) {
        const sh = el('div', 'row-head series-head', `<h4>${esc(series)}</h4>`);
        wrap.appendChild(sh);
      }
      const cards = el('div', 'cards');
      seriesItems.forEach(it => cards.appendChild(card(it, false)));
      wrap.appendChild(cards);
    }
  }
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
  const verb = ['movie', 'show'].includes(it.kind) ? 'Play' : ['comic', 'manga', 'ebook'].includes(it.kind) ? 'Read' : 'Listen';
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
  if (it.kind === 'movie' || it.kind === 'show') openVideo(it);
  else if (it.kind === 'music' || it.kind === 'audiobook') playAudio(it);
  else if (it.kind === 'comic' || it.kind === 'manga') openComic(it);
  else if (it.kind === 'ebook') openEbook(it);
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
  resumeFor(it.id).then(p => { if (p) showChunk(Math.min(Math.floor(p.position), spine.length - 1)); else showChunk(0); });
}

async function showChunk(n) {
  if (!ebook) return;
  ebook.chunk = Math.max(0, Math.min(n, ebook.spine.length - 1));
  const chunkUrl = ebook.spine[ebook.chunk]; // zip-relative path
  const base = chunkUrl.includes('/') ? chunkUrl.slice(0, chunkUrl.lastIndexOf('/') + 1) : '';
  try {
    let html = await (await fetch(`/api/epub/${ebook.it.id}/chunk/${ebook.chunk}`)).text();
    const PW = ebookPageW(), GAP = 40, PAD = 28;
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
        html{overflow:hidden}
        body{column-width:${PW - 2 * PAD}px;column-gap:${GAP}px;column-fill:auto;
          height:${FRAME_H}vh;width:${PW}px;padding:${PAD}px;
          margin:0;box-sizing:border-box;font-family:Georgia,serif;
          text-align:justify;overflow-wrap:break-word}
        img{max-width:100%}
      </style>
      <script>
        const PW = ${PW}, PAD = ${PAD}; // inner page padding (visual margin, kills edge-butt)
        const STEP = ${PW} - 2 * PAD + ${GAP}; // stride = column box (PW - 2*padding) + gap
        let col = 0;
        function pageCount() {
          const pages = Math.round((document.body.scrollWidth - ${PW} - 2 * PAD) / STEP) + 1;
          return Math.max(1, pages);
        }
        function report() { parent.postMessage({type:'page', page: col + 1, pages: pageCount()}, '*'); }
        window.addEventListener('message', (e) => {
          if (e.data === 'next' || e.data === 'prev') {
            const last = pageCount() - 1;
            if (e.data === 'next') {
              if (col >= last) parent.postMessage({type:'chapter', dir:1}, '*');
              else { col++; window.scrollTo({ left: Math.round(col * STEP), behavior: 'instant' }); report(); }
            } else {
              if (col <= 0) parent.postMessage({type:'chapter', dir:-1}, '*');
              else { col--; window.scrollTo({ left: Math.round(col * STEP), behavior: 'instant' }); report(); }
            }
          } else if (typeof e.data === 'object' && e.data?.type === 'goto') {
            col = Math.max(0, Math.min(e.data.col, pageCount() - 1));
            window.scrollTo({ left: Math.round(col * STEP), behavior: 'instant' }); report();
          }
        });
        window.addEventListener('load', () => { col = 0; report(); });
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
                const pw = PW + GAP;
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
            const pw = PW + GAP;
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
          parent.postMessage({type:'highlight', cfi: 'char:' + offset + ':' + text.length, text, page: Math.round(window.scrollX / (PW + GAP))}, '*');
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
    if (inject.length) {
      $('#em-frame').addEventListener('load', () => {
        const w = $('#em-frame').contentWindow;
        if (!w) return;
        try {
          const full = w.document.body.textContent;
          const walker = w.document.createTreeWalker(w.document.body, w.NodeFilter.SHOW_TEXT);
          const nodes = [];
          while (walker.nextNode()) nodes.push(walker.currentNode);
          let pos = 0;
          const nodeOffsets = nodes.map(n => { const st = pos; pos += n.nodeValue.length; return { n, st }; });
          for (const h of inject) {
            for (const { n, st } of nodeOffsets) {
              const en = st + n.nodeValue.length;
              if (h.off >= st && h.off + h.len <= en) {
                const r = w.document.createRange();
                r.setStart(n, h.off - st);
                r.setEnd(n, h.off - st + h.len);
                const mark = w.document.createElement('mark');
                mark.className = 'user-highlight';
                mark.style.setProperty('--hl', h.color);
                try { r.surroundContents(mark); } catch {}
                break;
              }
            }
          }
        } catch (err) { console.warn('highlight restore failed', err); }
      }, { once: true });
    }
  } catch {
    $('#em-frame').srcdoc = '<p style="font-family:sans-serif;color:#333">Failed to load chapter.</p>';
  }
  updateEbookInfo();
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
    if (n >= 0 && n < ebook.spine.length) showChunk(n);
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
$('#em-close').onclick = closeEbook;
// TOC drawer
$('#em-toc-btn').onclick = () => { renderToc(); $('#em-toc').classList.toggle('on'); };
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
  if (ebook) { syncProgress(ebook.it.id, ebook.chunk); ebook = null; }
  $('#em-frame').src = 'about:blank';
  $('#em-search').value = '';
  $('#em-searchinfo').textContent = '';
}

function playAudio(it) {
  $('#video-modal').classList.remove('on');
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
const isVideoId = (id) => (window.LIBRARY || []).some(x => x.id === id && (x.kind === 'movie' || x.kind === 'show'));

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
