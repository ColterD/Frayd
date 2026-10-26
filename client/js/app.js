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

// ---------- nav ----------
document.querySelectorAll('.nav-item').forEach(btn => {
  btn.onclick = () => {
    document.querySelectorAll('.nav-item').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    filterKind(btn.dataset.kind);
  };
});
$('#search input').addEventListener('input', (e) => filterKind(null, e.target.value));

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

function renderHero(it) {
  const hero = $('#hero');
  hero.style.display = 'block';
  hero.innerHTML = `
    <div class="hero-tags"><span class="tag">${esc(it.kind)}</span>${it.author ? `<span class="tag">${esc(it.author)}</span>` : ''}</div>
    <div class="hero-copy">
      <h2>${esc(it.title)}</h2>
      <p>Resume where you left off, or start fresh — progress syncs across your devices.</p>
      <div class="cta-row">
        <button class="btn-play" id="hero-play">▶ Play</button>
        <button class="btn-ghost" id="hero-info">More info</button>
      </div>
    </div>`;
  $('#hero-play').onclick = () => openItem(it);
}

// ---------- playback ----------
const pbAudio = $('#pb-audio');

function openItem(it) {
  if (it.kind === 'movie' || it.kind === 'show') openVideo(it);
  else if (it.kind === 'music' || it.kind === 'audiobook') playAudio(it);
  // ebook / comic handled by dedicated readers later
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
