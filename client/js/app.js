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
    if (r.ok) { enterApp(); return true; }
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
  loadLibrary();
}

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
    account = { name: data.name || email.split('@')[0], token: data.token };
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
$('#logout').onclick = () => {
  fetch('/api/auth/logout', { method: 'POST', headers: authHeaders() }).catch(() => {});
  account = null;
  delete localStorage.frayd_token;
  location.reload();
};
$('#guest').onclick = (e) => {
  e.preventDefault();
  IDENTITY = localStorage.frayd_identity ||= location.hostname + ':' + crypto.randomUUID();
  enterApp();
};

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
        renderLibrary();
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
  return fetch(`/api/progress/${id}`, { headers: authHeaders() })
    .then(r => r.ok ? r.json() : null).catch(() => null);
}

function card(it, portrait) {
  const c = el('div', 'media-card ' + (portrait ? 'portrait' : 'landscape'));
  const isVideo = it.kind === 'movie' || it.kind === 'show';
  c.innerHTML = `
    ${isVideo
      ? `<img class="thumb" loading="lazy" src="/api/thumb/${it.id}" onerror="this.outerHTML='<div class=&quot;thumb-fallback${portrait?' portrait':''}&quot;>▶</div>'">`
      : `<div class="thumb-fallback${portrait ? ' portrait' : ''}">${it.kind === 'music' ? '♪' : it.kind === 'audiobook' ? '🎙' : '📖'}</div>`}
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
  main.innerHTML = '';
  // hero: first movie with backdrop, else first item
  const heroItem = items.find(i => i.kind === 'movie') || items[0];
  if (heroItem) renderHero(heroItem);
  const byKind = {};
  items.forEach(i => (byKind[i.kind] ||= []).push(i));
  const order = ['movie', 'music', 'audiobook', 'ebook', 'comic'];
  for (const kind of order) {
    if (byKind[kind]?.length) main.appendChild(section(kind[0].toUpperCase() + kind.slice(1) + 's', byKind[kind], false));
  }
  if (!items.length) main.appendChild(el('p', '', '<br>No media found. Set FRAYD_LIBRARY and restart the server.'));
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
  const verb = ['movie', 'show'].includes(it.kind) ? 'Play' : it.kind === 'comic' || it.kind === 'ebook' ? 'Read' : 'Listen';
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
  else if (it.kind === 'comic') openComic(it);
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
  if (e.key === 'ArrowRight' || e.key === ' ') { e.preventDefault(); ebook && showChunk(ebook.chunk + 1); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); ebook && showChunk(ebook.chunk - 1); }
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
  ebook = { it, spine, chunk: 0 };
  $('#em-title').textContent = it.title;
  $('#ebook-modal').classList.add('on');
  resumeFor(it.id).then(p => { if (p) showChunk(Math.min(Math.floor(p.position), spine.length - 1)); else showChunk(0); });
}

async function showChunk(n) {
  if (!ebook) return;
  ebook.chunk = Math.max(0, Math.min(n, ebook.spine.length - 1));
  const chunkUrl = ebook.spine[ebook.chunk]; // zip-relative path
  const base = chunkUrl.includes('/') ? chunkUrl.slice(0, chunkUrl.lastIndexOf('/') + 1) : '';
  try {
    let html = await (await fetch(`/api/epub/${ebook.it.id}/chunk/${ebook.chunk}`)).text();
    // rewrite relative src/href to asset endpoint (skip absolute, data:, #anchors, mailto)
    html = html.replace(/(src|href)="([^"]*)"/g, (m, attr, url) => {
      if (/^(https?:|data:|#|mailto:|javascript:)/i.test(url)) return m;
      const resolved = base + url.replace(/^\.\//, '');
      return `${attr}="/api/epub/${ebook.it.id}/asset/${resolved}"`;
    });
    $('#em-frame').srcdoc = html;
  } catch {
    $('#em-frame').srcdoc = '<p style="font-family:sans-serif;color:#333">Failed to load chapter.</p>';
  }
  $('#em-pageinfo').textContent = `${ebook.chunk + 1} / ${ebook.spine.length}`;
  $('#em-prev').disabled = ebook.chunk === 0;
  $('#em-next').disabled = ebook.chunk === ebook.spine.length - 1;
  syncProgress(ebook.it.id, ebook.chunk);
}

$('#em-prev').onclick = () => ebook && showChunk(ebook.chunk - 1);
$('#em-next').onclick = () => ebook && showChunk(ebook.chunk + 1);
$('#em-close').onclick = closeEbook;
function closeEbook() {
  $('#ebook-modal').classList.remove('on');
  if (ebook) { syncProgress(ebook.it.id, ebook.chunk); ebook = null; }
  $('#em-frame').src = 'about:blank';
}

function playAudio(it) {
  $('#video-modal').classList.remove('on');
  $('#playerbar').classList.add('on');
  $('#pb-title').textContent = it.title;
  $('#pb-sub').textContent = it.author || it.kind;
  resumeFor(it.id).then(p => {
    pbAudio.src = `/api/stream/${it.id}`;
    if (p && p.position > 5) pbAudio.currentTime = p.position;
    pbAudio.play();
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

function syncProgress(mediaId, position) {
  fetch('/api/progress', {
    method: 'POST',
    headers: authHeaders({ 'Content-Type': 'application/json' }),
    body: JSON.stringify({ identity: IDENTITY, media_id: mediaId, position })
  });
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
}

$('#vm-close').onclick = () => {
  if (plyr.currentTime > 0 && videoItem) syncProgress(videoItem.id, plyr.currentTime);
  plyr.stop();
  $('#video-modal').classList.remove('on');
  videoItem = null;
};
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') $('#vm-close').click();
});

// ---------- boot ----------
tryResume().then(resumed => {
  if (!resumed) {
    document.body.classList.add('auth');
    $('#login').style.display = 'flex';
  }
});
