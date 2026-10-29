// Frayd web client — auth, dashboard, playback
'use strict';
console.log('frayd app.js v4');

/**
 * Return the first DOM node matching a selector.
 * @param {string} s
 * @returns {Element|null}
 */
const $ = (s) => document.querySelector(s);
/**
 * Create an element with optional class and trusted HTML.
 * @param {string} tag
 * @param {string} cls
 * @param {string} html
 * @returns {HTMLElement}
 */
const el = (tag, cls, html) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html != null) e.innerHTML = html;
  return e;
};
/**
 * Escape untrusted text before inserting it into HTML templates.
 * @param {*} s
 * @returns {string}
 */
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, c =>
  ({ '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;' }[c]));

// ---------- toast ----------
/**
 * Show an escaped transient message; errors remain visible longer.
 * @param {string} msg
 * @param {boolean} [err=false]
 */
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
/**
 * Copy extra HTTP headers and add the current bearer token when present.
 * @param {Object} [extra={}]
 * @returns {Object}
 */
function authHeaders(extra = {}) {
  const h = { ...extra };
  if (localStorage.frayd_token) h['Authorization'] = 'Bearer ' + localStorage.frayd_token;
  return h;
}
let account = null;
let accountGeneration = 0;

/**
 * Restore a stored LAN identity and optional account state if the library endpoint accepts it.
 * @returns {Promise<boolean>}
 */
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

/** Enter the dashboard for the current account generation and start loading the library. */
function enterApp() {
  ++accountGeneration;
  stopSettingsMonitor();
  disposeSettings();
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
const KINDS = ['movie', 'episode', 'music', 'audiobook', 'ebook', 'comic', 'manga'];
let settingsGeneration = 0;
let settingsPoll = null;
let settingsMonitor = null;
/** Retire panel callbacks without abandoning an admitted account-owned job. */
function disposeSettings() {
  ++settingsGeneration;
}
/** Stop the account's job observer; retired callbacks cannot clear its successor. */
function stopSettingsMonitor() {
  clearInterval(settingsPoll);
  settingsPoll = null;
  settingsMonitor = null;
}
/**
 * Return effective locked roots or editable persisted roots, including the legacy single path.
 * @param {Object} settings
 * @returns {string[]}
 */
function settingsRoots(settings) {
  if (settings.library_locked && Array.isArray(settings.effective_library_paths)) return [...settings.effective_library_paths];
  return Array.isArray(settings.library_paths) ? [...settings.library_paths]
    : settings.library_path ? [settings.library_path] : [];
}
/**
 * Read allowed media kinds from one permission row; null means unrestricted.
 * @param {Element} row
 * @returns {string[]|null}
 */
function permissionKinds(row) {
  const checked = [...row.querySelectorAll('.p-kinds-box input:checked')].map(input => input.value);
  return KINDS.every(kind => checked.includes(kind)) ? null : checked;
}
/**
 * Translate a scan HTTP status into a message for the settings panel.
 * @param {number} status
 * @returns {string}
 */
function scanFailure(status) {
  return status === 409 ? 'Scan already running' : status === 501
    ? 'No library folder configured — set it in settings' : `Scan failed (${status})`;
}
/**
 * Observe one admitted job for this account; only its current panel may publish UI.
 * @param {number} generation
 * @param {string} mode
 * @param {HTMLButtonElement} button
 */
function pollSettings(generation, mode, button) {
  stopSettingsMonitor();
  const monitor = settingsMonitor = { account: accountGeneration, identity: IDENTITY };
  const current = () => settingsMonitor === monitor
    && monitor.account === accountGeneration && monitor.identity === IDENTITY;
  const panelCurrent = () => generation === settingsGeneration;
  let pending = false;
  settingsPoll = setInterval(async () => {
    if (pending || !current()) return;
    pending = true;
    try {
      const response = await fetch('/api/scan/status');
      if (!response.ok) throw new Error(`Status failed (${response.status})`);
      const status = await response.json();
      if (!current()) return;
      const ingest = status.ingesting;
      if (mode === 'ingest' && (ingest === 'downloading' || ingest === 'indexing')) {
        if (panelCurrent()) button.textContent = ingest === 'indexing' ? 'Indexing…' : 'Downloading…';
        return;
      }
      if (mode === 'scan' && status.scanning) return;
      stopSettingsMonitor();
      const failure = mode === 'scan' ? status.scan_error : ingest !== 'done';
      if (panelCurrent()) {
        button.disabled = false;
        button.textContent = failure ? (mode === 'scan' ? 'Scan failed' : 'Download failed')
          : mode === 'scan' ? 'Scan complete' : 'Download complete';
        if (failure) toast(mode === 'scan' ? `Scan failed — ${status.scan_error}` : 'Download or indexing failed', true);
      }
      if (!failure) await loadLibrary();
    } catch (error) {
      if (!current()) return;
      stopSettingsMonitor();
      if (panelCurrent()) {
        button.disabled = false;
        button.textContent = 'Status unavailable';
        toast(error.message || 'Status unavailable', true);
      }
    } finally { pending = false; }
  }, 1000);
}
/**
 * Request a scan for this account; completion monitoring survives panel retirement.
 * @param {number} generation
 * @param {HTMLButtonElement} button
 */
async function startSettingsScan(generation, button) {
  const owner = accountGeneration, identity = IDENTITY;
  button.disabled = true;
  button.textContent = 'Starting scan…';
  try {
    const response = await fetch('/api/scan', { method: 'POST', headers: authHeaders() });
    if (owner !== accountGeneration || identity !== IDENTITY) return;
    if (response.status !== 202) throw new Error(scanFailure(response.status));
    if (generation === settingsGeneration) button.textContent = 'Scanning…';
    pollSettings(generation, 'scan', button);
  } catch (error) {
    if (owner !== accountGeneration || identity !== IDENTITY || generation !== settingsGeneration) return;
    button.disabled = false;
    button.textContent = error.message || 'Scan failed — server unreachable';
    toast(button.textContent, true);
  }
}
/**
 * Persist an admin settings payload; reject HTTP failure and report whether its panel still owns the result.
 * @param {Object} payload
 * @param {number} generation
 * @returns {Promise<boolean>}
 */
async function saveSettings(payload, generation) {
  const response = await fetch('/api/admin/settings', {
    method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }), body: JSON.stringify(payload),
  });
  if (!response.ok) throw new Error(response.status === 409
    ? 'Settings refused: library override or active job' : `Settings failed (${response.status})`);
  return generation === settingsGeneration;
}
/**
 * Reload the settings panel after a successful change.
 * @returns {Promise<void>}
 */
function reopenSettings() { return openSettings(); }
/**
 * Fetch permissions and library settings, then render controls guarded by a panel generation.
 * @returns {Promise<void>}
 */
async function openSettings() {
  disposeSettings();
  const generation = settingsGeneration;
  const current = () => generation === settingsGeneration;
  $('#settings-modal').classList.add('on');
  const stage = $('#settings-stage');
  stage.innerHTML = '<div class="party-sec">Accounts & permissions</div>';
  try {
    const [perms, settings] = await Promise.all([
      fetch('/api/admin/perms', { headers: authHeaders() }).then(r => r.ok ? r.json() : Promise.reject(r.status)),
      fetch('/api/admin/settings', { headers: authHeaders() }).then(r => r.ok ? r.json() : Promise.reject(r.status)),
    ]);
    if (!current()) return;
    const libs = settingsRoots(settings);
    const locked = !!settings.library_locked;
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
      + `<div class="party-sec">Media folders${locked ? ' — locked by FRAYD_LIBRARY' : ''}</div>`
      + (libs.length ? libs.map((l, i) => `
      <div class="perm-row"><span class="k">${esc(l)}</span><button class="lib-remove" ${locked ? 'disabled' : ''} data-path="${esc(l)}">Remove</button></div>`).join('') : '<div class="party-sec">No folders configured</div>')
      + `<div class="perm-row"><span class="k">Add folder</span><input class="s-lib-add" placeholder="/path/to/more/media" style="background:#0D0E12;color:#fff;border:1px solid #262833;border-radius:6px;padding:5px 8px;flex:1"><button class="lib-add" ${locked ? 'disabled' : ''}>Add + scan</button></div>`
      + `<div class="party-sec">Server settings — ${esc(settings.server_version || '')}</div>`
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
      try {
      const row = b.closest('.perm-row');
      const allowedKinds = permissionKinds(row);
      const r = await fetch('/api/admin/perms', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({
          identity_id: +row.dataset.id,
          is_admin: row.querySelector('.p-admin').checked,
          allowed_kinds: allowedKinds, // explicit null clears restrictions
        }),
      });
      if (!current()) return;
      b.textContent = r.ok ? '✓' : '✗';
      setTimeout(() => { if (current()) b.textContent = 'Save'; }, 1200);
      } catch { if (current()) toast('Permission save failed — server unreachable', true); }
    });
    $('#s-site-save').onclick = async (e) => {
      try {
        if (await saveSettings({site_name: stage.querySelector('.s-site').value}, generation)) e.target.textContent = 'Saved';
      } catch (error) { if (current()) toast(error.message || 'Save failed — server unreachable', true); }
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
      try {
        if (await saveSettings(body, generation)) e.target.textContent = 'Saved — restart to apply port/TLS';
      } catch (error) { if (current()) toast(error.message || 'Save failed — server unreachable', true); }
    };
    $('#s-pw-save')?.addEventListener('click', async (e) => {
      try {
      const old_ = stage.querySelector('.pw-old').value, nw = stage.querySelector('.pw-new').value;
      if (nw.length < 4) { e.target.textContent = 'Too short (min 4)'; return; }
      const r = await fetch('/api/auth/password', {
        method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({ old: old_, new: nw }),
      });
      if (!current()) return;
      if (r.ok) { e.target.textContent = '✓ Changed'; stage.querySelector('.pw-old').value = stage.querySelector('.pw-new').value = ''; }
      else { const j = await r.json().catch(() => ({})); e.target.textContent = j.error || '✗ Failed'; }
      setTimeout(() => { if (current()) e.target.textContent = 'Change password'; }, 2500);
      } catch { if (current()) toast('Password change failed — server unreachable', true); }
    });
    const scanBtn = stage.querySelector('.p-scan');
    scanBtn.onclick = () => startSettingsScan(generation, scanBtn);
    stage.querySelector('.lib-add').onclick = async (e) => {
      if (locked || !current()) return;
      const path = stage.querySelector('.s-lib-add').value.trim();
      if (!path) return;
      try {
        if (!await saveSettings({ library_paths: [...libs, path] }, generation)) return;
        toast('Folder added');
        await reopenSettings();
        await startSettingsScan(settingsGeneration, $('#settings-stage').querySelector('.p-scan'));
      } catch (error) { if (current()) toast(error.message || 'Add failed — server unreachable', true); }
    };
    stage.querySelectorAll('.lib-remove').forEach(b => b.onclick = async () => {
      if (locked || !current() || !confirm(`Stop scanning "${b.dataset.path}"? Existing items and files remain.`)) return;
      try {
        if (!await saveSettings({ library_paths: libs.filter(path => path !== b.dataset.path) }, generation)) return;
        toast('Folder removed'); reopenSettings();
      } catch (error) { if (current()) toast(error.message || 'Remove failed — server unreachable', true); }
    });
    // remove a user: sessions, progress, highlights gone. Cannot remove self/last admin.
    stage.querySelectorAll('.p-remove').forEach(b => b.onclick = async () => {
      const row = b.closest('.perm-row');
      if (!confirm(`Remove user ${b.dataset.name}? Their progress and highlights will be deleted. This cannot be undone.`)) return;
      try {
      const r = await fetch(`/api/admin/users/${row.dataset.id}?identity=${encodeURIComponent(IDENTITY)}`, { method: 'DELETE', headers: authHeaders() });
      if (!current()) return;
      if (r.ok) { toast('User removed'); reopenSettings(); }
      else if (r.status === 409) toast('Cannot remove: last admin (or yourself)', true);
      else toast(`Remove failed (${r.status})`, true);
      } catch { if (current()) toast('Remove failed — server unreachable', true); }
    });
    const ingestBtn = stage.querySelector('.p-ingest');
    ingestBtn.onclick = async () => {
      const url = stage.querySelector('.s-ingest').value.trim();
      if (!/^https?:\/\//.test(url)) { ingestBtn.textContent = 'Invalid URL'; return; }
      ingestBtn.disabled = true;
      const owner = accountGeneration, identity = IDENTITY;
      try {
        const response = await fetch('/api/ingest', { method: 'POST', headers: authHeaders({ 'Content-Type': 'application/json' }), body: JSON.stringify({ url }) });
        if (owner !== accountGeneration || identity !== IDENTITY) return;
        if (response.status !== 202) throw new Error(response.status === 409 ? 'Download already active' : response.status === 501 ? 'Download unavailable: configure a folder and yt-dlp' : `Download failed (${response.status})`);
        if (current()) ingestBtn.textContent = 'Downloading…';
        pollSettings(generation, 'ingest', ingestBtn);
      } catch (error) {
        if (!current()) return;
        ingestBtn.disabled = false;
        ingestBtn.textContent = 'Download failed';
        toast(error.message || 'Download failed — server unreachable', true);
      }
    };
  } catch (e) {
    if (!current()) return;
    stage.innerHTML = `<div class="party-sec">Admin access required (${e}). Log out and back in to refresh your session.</div>`;
    return;
  }
}
$('#settings-btn')?.addEventListener('click', openSettings);
$('#settings-close')?.addEventListener('click', () => { disposeSettings(); $('#settings-modal').classList.remove('on'); });

/**
 * Submit the login form, store its bearer token and identity, or show the returned error.
 * @param {SubmitEvent} ev
 */
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
  ++accountGeneration;
  stopSettingsMonitor();
  disposeSettings();
  ++libraryRequestGeneration;
  ++renderGeneration;
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
  const generation = accountGeneration;
  try {
    const r = await fetch('/api/scan', { method: 'POST', headers: authHeaders() });
    if (generation !== accountGeneration) return;
    if (r.status === 202 || r.status === 409) {
      toast(r.status === 409 ? 'Scan already running — monitoring progress…' : 'Scanning library…');
      let pending = false;
      let failures = 0;
      // Keep at most one request active; allow three consecutive failures before manual retry.
      const poll = setInterval(async () => {
        if (generation !== accountGeneration) { clearInterval(poll); return; }
        if (pending) return;
        pending = true;
        try {
          const response = await fetch('/api/scan/status');
          if (response.ok === false) throw new Error('Status unavailable');
          const { scanning, scan_error } = await response.json();
          if (generation !== accountGeneration) { clearInterval(poll); return; }
          if (scanning) { failures = 0; return; }
          if (scan_error) {
            clearInterval(poll);
            btn.disabled = false;
            btn.innerHTML = ADD_BTN_HTML;
            toast(`Scan failed — ${scan_error}`, true);
            return;
          }
          const before = LIBRARY.length;
          await loadLibrary();
          clearInterval(poll);
          if (generation !== accountGeneration) return;
          toast(LIBRARY.length > before ? `Scan complete — ${LIBRARY.length - before} new item(s)` : 'Scan complete');
          btn.disabled = false;
          btn.innerHTML = ADD_BTN_HTML;
        } catch {
          if (generation !== accountGeneration) { clearInterval(poll); return; }
          if (++failures < 3) {
            if (failures === 1) toast('Scan status unavailable — retrying…', true);
            return;
          }
          clearInterval(poll);
          btn.disabled = false;
          btn.innerHTML = ADD_BTN_HTML;
          toast('Scan status unavailable — click Scan to retry monitoring', true);
        } finally {
          pending = false;
        }
      }, 1500);
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
    if (generation !== accountGeneration) return;
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
    filterKind(btn.dataset.kind);
  };
});
let searchTimer;
$('#search input').addEventListener('input', (e) => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => serverSearch(e.target.value), 250);
});

/**
 * Search the server for a query, discarding stale responses and falling back to local filtering on failure.
 * @param {string} q
 */
async function serverSearch(q) {
  activeQuery = q;
  filtering = activeKind !== 'all' || !!q;
  const generation = ++searchGeneration;
  if (!q) { filterKind(activeKind, ''); return; }
  const libraryGeneration = libraryRequestGeneration;
  try {
    const response = await fetch(`/api/library?q=${encodeURIComponent(q)}`);
    if (!response.ok) throw new Error('Search failed');
    let results = await response.json();
    if (generation !== searchGeneration || libraryGeneration !== libraryRequestGeneration) return;
    if (activeKind !== 'all') results = results.filter(item => item.kind === activeKind);
    renderRows(results);
  } catch { if (generation === searchGeneration) filterKind(activeKind, q); }
}

/**
 * Read the active navigation kind, defaulting to all.
 * @returns {string}
 */
function currentKind() {
  const active = document.querySelector('.nav-item.active');
  return active ? active.dataset.kind : 'all';
}

// ---------- library ----------
let LIBRARY = [];
let libraryRequestGeneration = 0;
let renderGeneration = 0;
let searchGeneration = 0;
let activeQuery = '';
let activeKind = 'all';
const rowSort = new Map();
let filtering = false; // suppress continue-watching row while a nav filter/search is active
window.getLibrary = () => LIBRARY;

/**
 * Replace the library only if this fetch is still current; propagate HTTP errors.
 * @returns {Promise<void>}
 */
async function loadLibrary() {
  const generation = ++libraryRequestGeneration;
  const response = await fetch('/api/library');
  if (response.ok === false) throw new Error(`Library failed (${response.status})`);
  const items = await response.json();
  if (generation !== libraryRequestGeneration) return;
  LIBRARY = items;
  filterKind(activeKind, activeQuery);
}

/**
 * Apply the selected kind and text filter to the current library and render its rows.
 * @param {string} kind
 * @param {string} [query]
 * @returns {Promise<void>}
 */
function filterKind(kind, query = activeQuery) {
  ++searchGeneration;
  activeKind = kind || 'all';
  activeQuery = query || '';
  filtering = activeKind !== 'all' || !!activeQuery;
  let items = LIBRARY;
  if (activeKind !== 'all') items = items.filter(i => i.kind === activeKind);
  if (activeQuery) {
    const q = activeQuery.toLowerCase();
    items = items.filter(i => (i.title || '').toLowerCase().includes(q) || (i.author || '').toLowerCase().includes(q));
  }
  return renderRows(items);
}

/**
 * Confirm and delete one indexed media row; the source file remains and the library view refreshes on success.
 * @param {Object} it
 */
async function deleteMedia(it) {
  if (!confirm(`Remove "${it.title}" from the library? The source file remains on disk and may return after a later scan.`)) return;
  try {
    const response = await fetch(`/api/media/${it.id}?identity=${encodeURIComponent(IDENTITY)}`, {method:'DELETE', headers:authHeaders()});
    if (!response.ok) {
      toast(response.status === 409 ? 'Cannot remove: media is referenced or a job is active' : response.status === 403 ? 'Admin access required' : `Remove failed (${response.status})`, true);
      return;
    }
    ++libraryRequestGeneration;
    LIBRARY = LIBRARY.filter(item => item.id !== it.id);
    filterKind(activeKind, activeQuery);
    toast('Removed from library; source remains on disk');
  } catch { toast('Remove failed — server unreachable', true); }
}

/**
 * Fetch identity-scoped progress for one media ID, returning null on HTTP or network failure.
 * @param {number} id
 * @returns {Promise<Object|null>}
 */
function resumeFor(id) {
  // identity query needed for guest/LAN mode (no token); server auto-creates it
  return fetch(`/api/progress/${id}?identity=${encodeURIComponent(IDENTITY)}`, { headers: authHeaders() })
    .then(r => r.ok ? r.json() : null).catch(() => null);
}

/**
 * Build a media card and attach progress only while its render generation and DOM node remain current.
 * @param {Object} it
 * @returns {HTMLElement}
 */
function card(it) {
  const portrait = ['ebook', 'comic', 'manga', 'audiobook'].includes(it.kind);
  const c = el('div', 'media-card ' + (portrait ? 'portrait' : 'landscape'));
  const isVideo = it.kind === 'movie' || it.kind === 'show';
  const isPaged = it.kind === 'comic' || it.kind === 'manga';
  c.title = `${it.title}${it.author ? ' — ' + it.author : ''}`;
  c.innerHTML = `
    ${isVideo
      ? `<img class="thumb" loading="lazy" src="/api/thumb/${it.id}" onerror="this.outerHTML='<div class=&quot;thumb-fallback${portrait?' portrait':''}&quot;>▶</div>'">`
      : `<div class="thumb-fallback${portrait ? ' portrait' : ''}">${it.kind === 'music' ? '♪' : it.kind === 'audiobook' ? '🎙' : isPaged ? '📄' : '📖'}</div>`}
    <div class="card-body">
      <div class="title">${esc(it.title)}</div>
      <div class="sub">${esc(it.author || it.kind)}</div>
    </div>
    <div class="progress-strip" style="display:none"><div></div></div>`;
  if (account?.isAdmin) {
    const del = el('button', 'card-del', 'Delete');
    del.type = 'button';
    del.setAttribute('aria-label', `Remove ${it.title} from library`);
    del.onclick = event => { event.stopPropagation(); return deleteMedia(it); };
    del.onkeydown = event => { event.stopPropagation(); };
    c.appendChild(del);
  }
  const generation = renderGeneration;
  resumeFor(it.id).then(p => {
    if (generation !== renderGeneration || !c.isConnected) return;
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

/**
 * Build a sortable media row from a title and item snapshot.
 * @param {string} title
 * @param {Object[]} items
 * @returns {HTMLElement}
 */
function section(title, items) {
  const wrap = el('section');
  const cards = el('div', 'cards');
  const sortItems = () => [...items].sort((a,b) => (rowSort.get(title) ? 1 : -1) * ((a.created_at || 0) - (b.created_at || 0)) || a.id - b.id);
  let shown = sortItems();
  const head = el('div', 'row-head', `<h3>${esc(title)}</h3>`);
  const sortBtn = el('button', 'link', rowSort.get(title) ? 'newest first' : 'oldest first');
  sortBtn.onclick = () => {
    const oldest = sortBtn.textContent === 'oldest first';
    rowSort.set(title, oldest);
    shown = sortItems();
    sortBtn.textContent = oldest ? 'newest first' : 'oldest first';
    cards.innerHTML = '';
    shown.forEach(it => cards.appendChild(card(it)));
  };
  head.appendChild(sortBtn);
  shown.forEach(it => cards.appendChild(card(it)));
  wrap.append(head, cards);
  return wrap;
}

/**
 * Build the in-progress row from a library snapshot; return null when empty or superseded.
 * @param {Object[]} [snapshot]
 * @param {number} [generation]
 * @returns {Promise<HTMLElement|null>}
 */
async function continueRow(snapshot = LIBRARY, generation = renderGeneration) {
  // in-progress items, most recently touched first
  const items = snapshot.filter(i => i.kind !== 'ebook' && i.kind !== 'comic' && i.kind !== 'manga');
  const withP = await Promise.all(items.map(async it => {
    const p = await resumeFor(it.id);
    return p && p.position > 5 && it.duration_secs && p.position < it.duration_secs * 0.95
      ? { it, frac: Math.min(1, p.position / it.duration_secs) } : null;
  }));
  if (generation !== renderGeneration) return null;
  const inProgress = withP.filter(Boolean);
  if (!inProgress.length) return null;
  inProgress.sort((a, b) => b.frac - a.frac);
  const wrap = el('section');
  wrap.appendChild(el('div', 'row-head', '<h3>Continue watching</h3>'));
  const cards = el('div', 'cards');
  inProgress.forEach(({ it, frac }) => {
    const c = card(it);
    const strip = c.querySelector('.progress-strip');
    if (strip) { strip.style.display = 'block'; strip.firstElementChild.style.width = (frac * 100).toFixed(1) + '%'; }
    cards.appendChild(c);
  });
  wrap.appendChild(cards);
  return wrap;
}

/**
 * Render hero and grouped library rows; discard asynchronous continuation work after a newer render.
 * @param {Object[]} items
 * @returns {Promise<void>}
 */
async function renderRows(items) {
  const generation = ++renderGeneration;
  const snapshot = [...LIBRARY];
  const main = $('#main');
  main.innerHTML = '<div id="hero"></div>';
  // hero: first movie with backdrop, else first item
  const heroItem = items.find(i => i.kind === 'movie') || items[0];
  if (heroItem) renderHero(heroItem);
  if (!filtering) {
    const cont = await continueRow(snapshot, generation);
    if (generation !== renderGeneration) return;
    if (cont) main.appendChild(cont);
  }
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
      main.appendChild(section(kind[0].toUpperCase() + kind.slice(1) + 's', kindItems));
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
/**
 * Group media cards by author and then series within one kind.
 * @param {string} kind
 * @param {Object[]} items
 * @returns {HTMLElement}
 */
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
      seriesItems.forEach(it => cards.appendChild(card(it)));
      wrap.appendChild(cards);
    }
  }
  return wrap;
}

/**
 * Format duration seconds as rounded minutes or hours; return empty text for missing duration.
 * @param {number} s
 * @returns {string}
 */
function fmtDuration(s) {
  if (!s) return '';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/**
 * Render the selected item summary and load its backdrop only for the current render.
 * @param {Object} it
 */
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
  const generation = renderGeneration;
  img.onload = () => { if (generation !== renderGeneration || !hero.isConnected) return; bd.style.backgroundImage = `url(${img.src})`; hero.classList.add('has-backdrop'); };
  img.src = `/api/thumb/${it.id}`;
  $('#hero-play').onclick = () => openItem(it);
}

// ---------- playback ----------
const pbAudio = $('#pb-audio');
let partyPlaybackKind = null;
let nativeLoadGeneration = 0;
// Local audio intent remains identifiable while its progress response precedes src.
let localAudioLoad = null;

/**
 * Open the reader or player matching a library item kind.
 * @param {Object} it
 */
function openItem(it) {
  if (it.kind === 'movie' || it.kind === 'show') openVideo(it);
  else if (it.kind === 'music' || it.kind === 'audiobook') playAudio(it);
  else if (it.kind === 'comic' || it.kind === 'manga') openComic(it);
  else if (it.kind === 'ebook') openEbook(it);
}

// ---------- comic reader ----------
let comic = null; // { it, pages, page }

/**
 * Load page names, open the comic reader and restore saved page progress when available.
 * @param {Object} it
 * @returns {Promise<void>}
 */
async function openComic(it) {
  const pages = await (await fetch(`/api/comic/${it.id}/pages`)).json();
  if (!Array.isArray(pages) || !pages.length) return;
  comic = { it, pages, page: 0 };
  $('#cm-title').textContent = it.title;
  $('#comic-modal').classList.add('on');
  resumeFor(it.id).then(p => { if (p) showComicPage(Math.min(Math.floor(p.position), pages.length - 1)); else showComicPage(0); });
}

/**
 * Clamp a zero-based page index, update controls and save that page as progress.
 * @param {number} n
 */
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
/** Persist the current page, release comic state and close the reader. */
function closeComic() {
  $('#comic-modal').classList.remove('on');
  if (comic) { syncProgress(comic.it.id, comic.page); comic = null; }
  $('#cm-img').src = '';
}

// ---------- ebook reader ----------
let ebook = null; // { it, spine, chunk }
let ebookOpenGeneration = 0;
let ebookDocumentGeneration = 0;

/**
 * Load an EPUB spine and annotations, then open the current book only if its generation still owns the result.
 * @param {Object} it
 * @returns {Promise<void>}
 */
async function openEbook(it) {
  const opening = ++ebookOpenGeneration;
  ebook = null;
  const spine = await (await fetch(`/api/epub/${it.id}/spine`)).json();
  if (!Array.isArray(spine) || !spine.length) return;
  const annots = await fetch(`/api/annotations/${it.id}?identity=${encodeURIComponent(IDENTITY)}`, { headers: authHeaders() })
    .then(r => r.ok ? r.json() : []).catch(() => []);
  if (opening !== ebookOpenGeneration) return;
  const book = { it, spine, chunk: 0, annots, toc: [] };
  ebook = book;
  $('#em-title').textContent = it.title;
  $('#ebook-modal').classList.add('on');
  // TOC (best-effort)
  book.toc = await fetch(`/api/epub/${it.id}/toc`).then(r => r.ok ? r.json() : []).catch(() => []);
  if (ebook !== book || opening !== ebookOpenGeneration) return;
  resumeFor(it.id).then(p => {
    if (ebook !== book || opening !== ebookOpenGeneration) return;
    const ch = p ? Math.min(Math.floor(p.position), spine.length - 1) : 0;
    const frac = p ? p.position - Math.floor(p.position) : 0;
    showChunk(ch, { frac });
  });
}

/**
 * Load and paginate an EPUB chapter in a sandboxed frame; stale book/document generations cannot publish it.
 * @param {number} n
 * @param {{atEnd?: boolean, frac?: number}} [opts]
 * @returns {Promise<void>}
 */
async function showChunk(n, opts = {}) {
  if (!ebook) return;
  const book = ebook;
  const readerDocument = { generation: ++ebookDocumentGeneration, ready: false };
  book.document = readerDocument;
  const isCurrent = () => ebook === book && book.document === readerDocument;
  ebook.chunk = Math.max(0, Math.min(n, ebook.spine.length - 1));
  ebook.pendingCol = opts.atEnd ? Infinity : opts.frac != null ? opts.frac : 0;
  ebook.lastFrac = opts.atEnd ? 0.999 : (opts.frac ?? 0);
  const chunkUrl = ebook.spine[ebook.chunk]; // zip-relative path
  const base = chunkUrl.includes('/') ? chunkUrl.slice(0, chunkUrl.lastIndexOf('/') + 1) : '';
  try {
    let html = await (await fetch(`/api/epub/${ebook.it.id}/chunk/${ebook.chunk}`)).text();
    if (!isCurrent()) return;
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
        const generation = ${readerDocument.generation};
        const sendReader = data => parent.postMessage({ ...data, generation }, '*');
        const restoredHighlights = new Set();
        const PW = ${PW}, PAD = ${PAD}; // inner page padding (visual margin, kills edge-butt)
        const STEP = ${PW}; // column (PW-2*PAD) + gap (2*PAD) = exactly PW: viewport = one page, no bleed
        let col = 0;
        function report() { sendReader({type:'page', page: col + 1, pages: pageCount()}); }
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
              if (col >= last) sendReader({type:'chapter', dir:1});
              else { col++; snap(); report(); }
            } else {
              if (col <= 0) sendReader({type:'chapter', dir:-1});
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
          // All listeners and body are ready; parent now sends this document's resume and marks.
          col = 0;
          sendReader({ type: 'reader-ready' });
        });
        // Parent-driven resume and highlights are bound to this document generation.
        window.addEventListener('message', (e) => {
          if (e.source === parent && e.data?.generation === generation && e.data?.type === 'resume') {
            const lastReal = pageCount(); // filler excluded from count
            const c = e.data.col === 'end' ? lastReal - 1
              : Math.round(e.data.col * (lastReal - 1)); // fraction → column
            col = Math.max(0, Math.min(c, lastReal - 1));
            snap(); report();
          } else if (e.source === parent && e.data?.generation === generation && e.data?.type === 'inject-hl') {
            // highlight restore runs HERE (sandbox: parent can't touch our DOM)
            const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
            const nodes = [];
            while (walker.nextNode()) nodes.push(walker.currentNode);
            let pos = 0;
            const offs = nodes.map(n => { const st = pos; pos += n.nodeValue.length; return { n, st }; });
            for (const h of [...e.data.items].sort((a, b) => b.off - a.off)) {
              const key = h.off + ':' + h.len;
              if (restoredHighlights.has(key)) continue;
              for (const { n, st } of offs) {
                const en = st + n.nodeValue.length;
                if (h.off >= st && h.off + h.len <= en) {
                  const r = document.createRange();
                  r.setStart(n, h.off - st);
                  r.setEnd(n, h.off - st + h.len);
                  const m = document.createElement('mark');
                  m.className = 'user-highlight';
                  m.style.setProperty('--hl', h.color);
                  try { r.surroundContents(m); restoredHighlights.add(key); } catch {}
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
            sendReader({ type: 'geom', innerWidth: window.innerWidth,
              scrollWidth: b.scrollWidth, clientWidth: b.clientWidth,
              scrollLeft: window.scrollX, col, STEP,
              bodyPad: getComputedStyle(b).paddingLeft + '/' + getComputedStyle(b).paddingRight });
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
            sendReader({type:'search-results', count: hits.length});
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
            sendReader({type:'search-hit', idx: (cur + 1) % marks.length + 1, total: marks.length});
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
          const text = range.toString();
          if (!text.trim()) return;
          if (range.startContainer !== range.endContainer || range.startContainer.nodeType !== Node.TEXT_NODE) {
            sel.removeAllRanges();
            sendReader({ type: 'highlight-selection-error' });
            return;
          }
          const length = range.endOffset - range.startOffset;
          // Paint only after the parent confirms that the highlight was saved.
          // Serialize the original range before clearing the selection.
          // Count preceding chars in body text.
          const pre = document.createRange();
          pre.selectNodeContents(document.body);
          pre.setEnd(range.startContainer, range.startOffset);
          const offset = pre.toString().length;
          sel.removeAllRanges();
          sendReader({type:'highlight', cfi: 'char:' + offset + ':' + length, text, page: Math.round(window.scrollX / PW)});
        });
      <\/script></head>`);
    // re-apply saved highlights for this chapter (char-offset based)
    const saved = (ebook.annots || []).filter(a => a.kind === 'highlight' && a.spine_idx === ebook.chunk);
    const inject = saved.map(a => {
      const m = a.cfi.match(/^char:(\d+):(\d+)$/);
      return m ? { off: +m[1], len: +m[2], color: a.color || 'yellow' } : null;
    }).filter(Boolean).sort((a, b) => b.off - a.off); // reverse: offsets stay valid
    readerDocument.items = inject;
    readerDocument.want = ebook.pendingCol === Infinity ? 'end' : ebook.pendingCol;
    ebook.pendingCol = 0;
    $('#em-frame').srcdoc = html;
  } catch {
    if (!isCurrent()) return;
    $('#em-frame').srcdoc = '<p style="font-family:sans-serif;color:#333">Failed to load chapter.</p>';
  }
}

// page-flip: injected iframe script reports page state via postMessage
/**
 * Return the current EPUB frame width, or a viewport-based fallback before it exists.
 * @returns {number}
 */
function ebookPageW() { const f = document.getElementById("em-frame"); return Math.round(f ? f.clientWidth : Math.min(window.innerWidth * 0.85, 760)); }
/**
 * Ask the sandboxed reader to move one page in the signed direction.
 * @param {number} dir
 */
function ebookFlip(dir) {
  if (!ebook) return;
  $('#em-frame').contentWindow?.postMessage(dir > 0 ? 'next' : 'prev', '*');
}
// resize: repaginate (reload chapter at same column); debounce 300ms
let ebookResizeT = null;
window.addEventListener('resize', () => {
  if (!ebook || !$('#ebook-modal').classList.contains('on')) return;
  clearTimeout(ebookResizeT);
  const book = ebook;
  const readerDocument = book.document;
  ebookResizeT = setTimeout(() => {
    if (ebook === book && book.document === readerDocument) showChunk(book.chunk, { frac: book.lastFrac });
  }, 300);
});
window.addEventListener('message', async (e) => {
  if (!ebook) return;
  // srcdoc is sandboxed without allow-same-origin, so its origin is opaque.
  // Other opaque frames also report "null": bind messages to our reader window.
  const frameWindow = $('#em-frame').contentWindow;
  if (e.origin !== 'null' || !frameWindow || e.source !== frameWindow) return;
  if (!e.data || typeof e.data !== 'object' || Array.isArray(e.data)) return;
  const readerDocument = ebook.document;
  if (!readerDocument || e.data.generation !== readerDocument.generation) return;
  if (e.data.type === 'reader-ready') {
    if (readerDocument.ready) return;
    readerDocument.ready = true;
    frameWindow.postMessage({ type: 'resume', col: readerDocument.want, generation: readerDocument.generation }, '*');
    if (readerDocument.items.length) frameWindow.postMessage({ type: 'inject-hl', generation: readerDocument.generation, items: readerDocument.items }, '*');
    return;
  }
  if (!readerDocument.ready) return;
  if (e.data?.type === 'chapter') {
    if (e.data.dir !== -1 && e.data.dir !== 1) return;
    const n = ebook.chunk + e.data.dir;
    if (n < 0 || n >= ebook.spine.length) return;
    // entering prev chapter from its top → land on its last real page
    showChunk(n, e.data.dir < 0 ? { atEnd: true } : {});
  } else if (e.data?.type === 'geom') {
    console.log('[ebook geom]', e.data);
  } else if (e.data?.type === 'highlight-selection-error') {
    $('#em-searchinfo').textContent = 'Select text without crossing formatting or paragraph breaks.';
  } else if (e.data?.type === 'highlight') {
    if (typeof e.data.cfi !== 'string' || !/^char:\d+:\d+$/.test(e.data.cfi)
      || typeof e.data.text !== 'string' || !e.data.text.trim()) return;
    const book = ebook;
    const chapter = book.chunk;
    const annotation = {
      media_id: book.it.id, kind: 'highlight', spine_idx: chapter,
      cfi: e.data.cfi, text: e.data.text, color: 'yellow'
    };
    const isCurrent = () => ebook === book && ebook.chunk === chapter
      && book.document === readerDocument && readerDocument.ready
      && $('#em-frame').contentWindow === frameWindow;
    if ((book.annots || []).some(a => a.kind === 'highlight'
      && a.spine_idx === chapter && a.cfi === annotation.cfi)) {
      $('#em-searchinfo').textContent = 'Highlight saved';
      return;
    }
    $('#em-searchinfo').textContent = 'Saving highlight…';
    try {
      const response = await fetch('/api/annotations', {
        method: 'POST',
        headers: authHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(annotation)
      });
      if (response.status !== 201) {
        if (isCurrent()) $('#em-searchinfo').textContent = response.status === 401
          ? 'Sign in to save highlights' : 'Could not save highlight. Try again.';
        return;
      }
      book.annots ??= [];
      const exists = book.annots.some(a => a.kind === 'highlight'
        && a.spine_idx === chapter && a.cfi === annotation.cfi);
      if (!exists) book.annots.push(annotation);
      if (!isCurrent()) return;
      if (!exists) {
        const [, off, len] = annotation.cfi.match(/^char:(\d+):(\d+)$/);
        frameWindow.postMessage({ type: 'inject-hl', generation: readerDocument.generation, items: [{ off: +off, len: +len, color: annotation.color }] }, '*');
      }
      $('#em-searchinfo').textContent = 'Highlight saved';
    } catch {
      if (isCurrent()) $('#em-searchinfo').textContent = 'Could not save highlight. Try again.';
    }
  } else if (e.data?.type === 'page') {
    if (!Number.isSafeInteger(e.data.page) || !Number.isSafeInteger(e.data.pages)
      || e.data.page < 1 || e.data.pages < e.data.page) return;
    $('#em-pageinfo').textContent = `${ebook.chunk + 1} · p${e.data.page}/${e.data.pages}`;
    // progress = chunk + fractional page (fraction-of-chapter, resolution-independent)
    const frac = e.data.pages > 1 ? (e.data.page - 1) / (e.data.pages - 1) : 0;
    ebook.lastFrac = Math.min(frac, 0.999);
    syncProgress(ebook.it.id, ebook.chunk + ebook.lastFrac);
  } else if (e.data?.type === 'search-results') {
    if (!Number.isSafeInteger(e.data.count) || e.data.count < 0) return;
    ebook.searchHits = e.data.count;
    $('#em-searchinfo').textContent = e.data.count
      ? `${e.data.count} hits — Enter for next` : 'No hits';
  } else if (e.data?.type === 'search-hit') {
    if (!Number.isSafeInteger(e.data.idx) || !Number.isSafeInteger(e.data.total)
      || e.data.idx < 1 || e.data.total < e.data.idx) return;
    $('#em-searchinfo').textContent = `${e.data.idx}/${e.data.total} in ch.`;
  }
});

// ebook search UI
/**
 * Send a query to the active EPUB frame and remember it for repeat searches.
 * @param {string} q
 */
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
/** Advance to the next search hit in the current EPUB frame. */
function ebookSearchNext() {
  const w = $('#em-frame').contentWindow;
  w?.postMessage({ type: 'search-next' }, '*');
}

$('#em-prev').onclick = () => ebookFlip(-1);
$('#em-next').onclick = () => ebookFlip(1);
/**
 * Allow reader shortcuts only while the reader owns focus outside editable controls and settings.
 * @param {KeyboardEvent} event
 * @returns {boolean}
 */
function ebookShortcutAllowed(event) {
  const target = event.target;
  return !!ebook && $('#ebook-modal').classList.contains('on')
    && !$('#settings-modal').classList.contains('on')
    && !['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON'].includes(target?.tagName)
    && !target?.isContentEditable;
}
// reader shortcuts: ←/→/space flip, s search, t toc, f fullscreen, Esc closes overlays
document.addEventListener('keydown', (e) => {
  if (!ebookShortcutAllowed(e)) return;
  if (e.key === 'ArrowRight' || e.key === ' ') { e.preventDefault(); ebookFlip(1); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); ebookFlip(-1); }
  else if (e.key === 's') { e.preventDefault(); $('#em-search').focus(); }
  else if (e.key === 't') { renderToc(); $('#em-toc').classList.toggle('on'); }
  else if (e.key === 'f') { $('#em-toc').classList.remove('on'); document.fullscreenElement ? document.exitFullscreen() : $('#ebook-modal').requestFullscreen?.(); }
  else if (e.key === 'Escape') { if ($('#em-toc').classList.contains('on')) $('#em-toc').classList.remove('on'); else closeEbook(); }
});
$('#em-close').onclick = closeEbook;
// TOC drawer
$('#em-toc-btn').onclick = () => { renderToc(); $('#em-toc').classList.toggle('on'); };
$('#em-prev').ondblclick = $('#em-next').ondblclick = () =>
  $('#em-frame').contentWindow?.postMessage({ type: 'geom' }, '*');
/** Render the EPUB table of contents and bind chapter navigation. */
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
/** Retire pending EPUB work, save progress and clear the sandboxed frame. */
function closeEbook() {
  ++ebookOpenGeneration;
  clearTimeout(ebookResizeT);
  $('#ebook-modal').classList.remove('on');
  if (ebook) { syncProgress(ebook.it.id, ebook.chunk + (ebook.lastFrac ?? 0)); ebook = null; }
  $('#em-frame').srcdoc = '';
  $('#em-frame').src = 'about:blank';
  $('#em-search').value = '';
  $('#em-searchinfo').textContent = '';
}

/**
 * Give native playback ownership before pausing a ready external player, so its
 * pause callback cannot publish a stale party command. Return the prior owner so
 * party follow can preserve a native player that was already active locally.
 * @param {'audio'|'video'} kind Native player taking ownership.
 * @returns {string|null} Playback owner before this transition.
 */
function beginNativePlayback(kind) {
  const priorKind = partyPlaybackKind;
  partyPlaybackKind = kind;
  if (priorKind === 'external' && ytPlayerReady) ytPlayer?.pauseVideo();
  return priorKind;
}

/**
 * Compare the configured native source, falling back to its effective stream.
 * Plyr installs source children; native audio and party loads may set src directly.
 * @param {HTMLMediaElement} media Native element whose stream is being checked.
 * @param {number} id Expected library stream ID.
 * @returns {boolean} Whether the canonical stream URL matches.
 */
function hasNativeStream(media, id) {
  const source = media.src || media.querySelector('source')?.src || media.currentSrc || '';
  return new URL(source, window.location.origin).href
    === new URL(`/api/stream/${id}`, window.location.origin).href;
}

/**
 * Accept a delayed native load only while its generation, player and exact stream
 * still own playback. External handoff and newer native commands retire callbacks
 * even when a later command reuses the same media ID or native element.
 * @param {HTMLMediaElement} media Native element captured when scheduling the load.
 * @param {'audio'|'video'} kind Expected playback owner.
 * @param {number} id Library stream ID for this load.
 * @param {number} generation Authority generation captured by this load.
 * @returns {boolean} Whether this callback may still seek or start its element.
 */
function ownsNativeLoad(media, kind, id, generation) {
  if (nativeLoadGeneration !== generation || partyPlaybackKind !== kind
    || media !== (kind === 'video' ? plyr.media : pbAudio)) return false;
  return hasNativeStream(media, id);
}

/**
 * Open an audio item and resume only if this native load still owns playback.
 * @param {Object} it
 */
function playAudio(it) {
  beginNativePlayback('audio');
  const generation = ++nativeLoadGeneration;
  const load = localAudioLoad = { id: it.id, generation, pendingResume: true };
  $('#video-modal').classList.remove('on', 'yt');
  $('#playerbar').classList.add('on');
  if (window.partySend) { partyLoadPending = true; partyLoadId = it.id; partyLoadTitle = it.title; }
  $('#pb-title').textContent = it.title;
  $('#pb-sub').textContent = it.author || it.kind;
  resumeFor(it.id).then(p => {
    if (nativeLoadGeneration !== generation || partyPlaybackKind !== 'audio') return;
    pbAudio.src = `/api/stream/${it.id}`;
    load.pendingResume = false;
    const seek = (p && p.position > 5) ? p.position : 0;
    const onMeta = () => {
      if (!ownsNativeLoad(pbAudio, 'audio', it.id, generation)) return;
      if (seek) pbAudio.currentTime = seek;
      pbAudio.play();
    };
    if (pbAudio.readyState >= 1) onMeta();
    else pbAudio.addEventListener('loadedmetadata', onMeta, { once: true });
  });
  trackProgress(pbAudio, it);
}

let progressTimer = null;
let progressGeneration = 0;
let disposeProgressListeners = null;
/**
 * Save only while this tracker still owns a metadata-ready matching native stream.
 * Party audio may replace src without installing a tracker; its retired local ID
 * must not receive party positions. A new tracker also retires old same-ID handlers
 * so a delayed ended event cannot clear the successor's interval. Plyr resolves its
 * current native element after source replacement before checking the stream URL.
 * @param {HTMLMediaElement|Plyr} media Native player or replacing Plyr wrapper.
 * @param {{id: number}} it Library item whose progress this tracker may save.
 */
function trackProgress(media, it) {
  disposeProgressListeners?.();
  disposeProgressListeners = null;
  clearInterval(progressTimer);
  const generation = ++progressGeneration;
  const kind = media === pbAudio ? 'audio' : 'video';
  const ownsProgress = () => {
    const native = media === plyr ? plyr.media : media;
    return generation === progressGeneration && partyPlaybackKind === kind && native.readyState >= 1
      && hasNativeStream(native, it.id);
  };
  let last = 0;
  progressTimer = setInterval(() => {
    if (!ownsProgress() || media.paused) return;
    if (media.currentTime - last >= 10) {
      last = media.currentTime;
      syncProgress(it.id, media.currentTime);
    }
  }, 1000);
  const listenerTarget = media === plyr ? document : media;
  const capture = media === plyr;
  const ownsEvent = event => event.target === (media === plyr ? plyr.media : media);
  const onPause = event => {
    if (ownsEvent(event) && media.paused && ownsProgress()) syncProgress(it.id, media.currentTime);
  };
  const onEnded = event => {
    if (!ownsEvent(event) || !ownsProgress()) return;
    clearInterval(progressTimer);
    syncProgress(it.id, media.duration);
  };
  // Capture preserves Plyr element identity; native listeners preserve UI properties.
  listenerTarget.addEventListener('pause', onPause, capture);
  listenerTarget.addEventListener('ended', onEnded, capture);
  disposeProgressListeners = () => {
    listenerTarget.removeEventListener('pause', onPause, capture);
    listenerTarget.removeEventListener('ended', onEnded, capture);
  };
}

// ---------- party sync ----------
// guest: follow host playback state (drives bottom audio bar)
/**
 * Check whether an indexed ID belongs to a video item.
 * @param {number} id
 * @returns {boolean}
 */
const isVideoId = (id) => LIBRARY.some(x => x.id === id && (x.kind === 'movie' || x.kind === 'show'));

// ---- YouTube party playback (guests + host-load mirror) ----
// loads YT iframe API once; ytPlayer mirrors host state. Looser drift (YT seek latency ~2s).
let ytPlayer = null, ytId = null, ytReady = false, ytPlayerReady = false, ytDesired = null;
const YT_DRIFT = 3;
/**
 * Mirror an external party stream after the iframe API and the current player are ready.
 * Player callbacks must match the active instance; late readiness cannot take playback
 * back from native media. Readiness applies the latest request for that video; queued
 * API callbacks for superseded videos cannot create players. Host events broadcast only
 * while external playback owns it. The current party sender supplies host authority
 * across role/session changes, including when the same iframe is reused. Native players
 * pause after ownership transfers so their pause events cannot overwrite external party state.
 * @param {string} vid YouTube video ID supplied by the party state.
 * @param {string} title Display title for the external stream.
 * @param {number} position Host position at updatedAt, in seconds.
 * @param {boolean} playing Whether the host is advancing the playback clock.
 * @param {number} updatedAt Unix timestamp used to compensate a playing stream for transit.
 * @param {boolean} isHostOwn Legacy caller hint; current partySend determines host authority.
 */
window.partyFollowExternal = (vid, title, position, playing, updatedAt, isHostOwn) => {
  partyPlaybackKind = 'external';
  ++nativeLoadGeneration;
  pbAudio.pause();
  plyr.pause();
  ytDesired = { vid, position, playing, updatedAt };
  const expected = () => ytDesired.position
    + (ytDesired.playing ? Math.max(0, Date.now() / 1000 - ytDesired.updatedAt) : 0);
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
    if (partyPlaybackKind !== 'external' || ytDesired?.vid !== vid) return;
    const desired = ytDesired;
    // Rebind one seek mirror for the current session even when reusing the iframe.
    clearInterval(window._ytSeekTimer);
    window._ytSeekTimer = null;
    if (window.partySend) {
      window._ytSeekTimer = setInterval(() => {
        if (!ytPlayerReady || !ytPlayer || !ytPlayer.getCurrentTime
          || !window.partySend || partyPlaybackKind !== 'external') return;
        if (ytPlayer.getPlayerState() === 1) window.partySend({ type: 'seek', position: ytPlayer.getCurrentTime() });
      }, 2000);
    }
    if (ytId !== vid) {
      ytId = vid;
      ytPlayerReady = false;
      if (ytPlayer) ytPlayer.destroy();
      ytPlayer = new YT.Player('yt-party', {
        videoId: vid,
        playerVars: { autoplay: 0, playsinline: 1 },
        events: {
          onReady: (e) => {
            if (e.target !== ytPlayer || ytDesired?.vid !== vid) return;
            ytPlayerReady = true;
            if (partyPlaybackKind !== 'external') return;
            e.target.seekTo(expected(), true);
            if (ytDesired.playing) e.target.playVideo();
          },
          // host: mirror player state into party broadcasts
          onStateChange: (e) => {
            if (e.target !== ytPlayer || !ytPlayerReady
              || !window.partySend || partyPlaybackKind !== 'external') return;
            if (e.data === 1) window.partySend({ type: 'play', position: ytPlayer.getCurrentTime() });
            else if (e.data === 2 || e.data === 0) window.partySend({ type: 'pause', position: ytPlayer.getCurrentTime() });
          }
        }
      });
      return;
    }
    if (!ytPlayerReady || !ytPlayer || !ytPlayer.seekTo) return;
    const cur = ytPlayer.getCurrentTime ? ytPlayer.getCurrentTime() : 0;
    const drift = Math.abs(cur - expected());
    if (desired.playing && drift > YT_DRIFT) ytPlayer.seekTo(expected(), true);
    if (desired.playing && ytPlayer.getPlayerState && ytPlayer.getPlayerState() !== 1) ytPlayer.playVideo();
    if (!desired.playing && ytPlayer.getPlayerState && ytPlayer.getPlayerState() === 1) ytPlayer.pauseVideo();
  });
  void isHostOwn;
};

/**
 * Transfer party playback to native media and apply the host's authoritative clock.
 * A host's first echo preserves its matching native owner, including local resume,
 * pause and close state; guests and other playback owners follow the received state.
 * External pause callbacks run only after ownership changes to avoid a stale broadcast.
 * @param {number} mediaId Library item whose kind selects native audio or video.
 * @param {string} title Display title for the active native stream.
 * @param {number} position Host position at updatedAt, in seconds.
 * @param {boolean} playing Whether playback should advance after synchronization.
 * @param {number} updatedAt Unix timestamp used to compensate for transit.
 * @param {{preserveActiveVideo?: boolean}} options Legacy host-only native preservation flag.
 */
window.partyFollow = (mediaId, title, position, playing, updatedAt, options = {}) => {
  const expected = () => position + (playing ? Math.max(0, Date.now() / 1000 - updatedAt) : 0);
  const priorPlaybackKind = beginNativePlayback(isVideoId(mediaId) ? 'video' : 'audio');
  $('#video-modal').classList.remove('yt');
  // video: load into Plyr modal, mirror play/pause/seek on the <video> element
  if (isVideoId(mediaId)) {
    const video = plyr.media;
    // A host's first state echoes their local load, even after a local close.
    // Visibility is UI state; the matching native owner retains close/seek authority.
    if (options.preserveActiveVideo && priorPlaybackKind === 'video'
      && hasNativeStream(video, mediaId)) {
      partyFollowId = mediaId;
      return;
    }
    const generation = ++nativeLoadGeneration;
    const changed = !hasNativeStream(video, mediaId);
    const item = LIBRARY.find(entry => entry.id === mediaId);
    const activating = priorPlaybackKind !== 'video'
      || !$('#video-modal').classList.contains('on') || videoItem?.id !== mediaId;
    partyFollowId = mediaId;
    pbAudio.pause();
    $('#video-modal').classList.add('on');
    $('#vm-title').textContent = title || item.title || '—';
    if (activating) {
      videoItem = item;
      trackProgress(video, videoItem);
    }
    if (changed) video.src = `/api/stream/${mediaId}`;
    const onMeta = () => {
      if (!ownsNativeLoad(video, 'video', mediaId, generation)) return;
      if (changed || Math.abs(video.currentTime - expected()) > 2) video.currentTime = expected();
      if (playing && video.paused) video.play().catch(() => {});
      if (!playing && !video.paused) video.pause();
    };
    if (video.readyState >= 1) onMeta();
    else video.addEventListener('loadedmetadata', onMeta, { once: true });
    return;
  }
  // The host's first load echo carries zero, before local resume or metadata may
  // finish. Preserve the pending local intent as well as an already loaded stream.
  const pendingLocalAudio = localAudioLoad?.id === mediaId
    && localAudioLoad.generation === nativeLoadGeneration && localAudioLoad.pendingResume;
  if (options.preserveActiveVideo && priorPlaybackKind === 'audio'
    && (pendingLocalAudio || hasNativeStream(pbAudio, mediaId))) {
    partyFollowId = mediaId;
    return;
  }
  const generation = ++nativeLoadGeneration;
  $('#video-modal').classList.remove('on', 'yt');
  plyr.pause();
  $('#playerbar').classList.add('on');
  const changed = !hasNativeStream(pbAudio, mediaId);
  partyFollowId = mediaId;
  if (changed) {
    $('#pb-title').textContent = title || '—';
    $('#pb-sub').textContent = 'party';
    pbAudio.src = `/api/stream/${mediaId}`;
  }
  const onMeta = () => {
    if (!ownsNativeLoad(pbAudio, 'audio', mediaId, generation)) return;
    if (changed || Math.abs(pbAudio.currentTime - expected()) > 1.5) pbAudio.currentTime = expected();
    if (playing && pbAudio.paused) pbAudio.play().catch(() => {});
    if (!playing && !pbAudio.paused) pbAudio.pause();
  };
  if (pbAudio.readyState >= 1) onMeta();
  else pbAudio.addEventListener('loadedmetadata', onMeta, { once: true });
};
let partyFollowId = null;

// Panel actions use the active player; its native event is the single command.
window.partyControlPlayback = async (type, state) => {
  const playing = type === 'play';
  if (state?.external_url) {
    if (!ytPlayer?.getPlayerState || !ytPlayer?.getCurrentTime)
      throw new Error('YouTube player is not ready');
    if ((ytPlayer.getPlayerState() === 1) === playing) {
      window.partySend?.({ type, position: ytPlayer.getCurrentTime() });
    } else if (playing) ytPlayer.playVideo();
    else ytPlayer.pauseVideo();
    return;
  }
  if (state?.media_id == null) throw new Error('Choose a track first');
  const media = isVideoId(state.media_id) ? plyr : pbAudio;
  if (media.paused === !playing) {
    window.partySend?.({ type, position: media.currentTime });
  } else if (playing) await media.play();
  else media.pause();
};

// host: local player events push party state
window.partyHostHook = (send) => {
  pbAudio.addEventListener('play', () => { if (partyPlaybackKind === 'audio' && !pbAudio.paused) send({ type: 'play', position: pbAudio.currentTime }); });
  pbAudio.addEventListener('pause', () => { if (partyPlaybackKind === 'audio' && pbAudio.paused) send({ type: 'pause', position: pbAudio.currentTime }); });
  pbAudio.addEventListener('seeked', () => { if (partyPlaybackKind === 'audio') send({ type: 'seek', position: pbAudio.currentTime }); });
  // host loads a track locally → broadcast it
  pbAudio.addEventListener('loadedmetadata', () => {
    if (window.partySend && partyPlaybackKind === 'audio' && partyLoadPending) {
      partyLoadPending = false;
      send({ type: 'load', media_id: partyLoadId, title: partyLoadTitle, position: pbAudio.currentTime });
    }
  });
  // host video: mirror Plyr play/pause/seek + load
  // Capture native events: Plyr's proxy loses the original element identity.
  document.addEventListener('play', event => {
    if (event.target === plyr.media && partyPlaybackKind === 'video' && !plyr.paused)
      send({ type: 'play', position: plyr.currentTime });
  }, true);
  document.addEventListener('pause', event => {
    if (event.target === plyr.media && partyPlaybackKind === 'video' && plyr.paused)
      send({ type: 'pause', position: plyr.currentTime });
  }, true);
  document.addEventListener('seeked', event => {
    if (event.target === plyr.media && partyPlaybackKind === 'video')
      send({ type: 'seek', position: plyr.currentTime });
  }, true);
  plyr.on('loadedmetadata', () => {
    if (window.partySend && partyPlaybackKind === 'video' && videoLoadPending && videoItem) {
      videoLoadPending = false;
      send({ type: 'load', media_id: videoItem.id, title: videoItem.title, position: plyr.currentTime });
    }
  });
};
let partyLoadPending = false, partyLoadId = null, partyLoadTitle = null;
let videoLoadPending = false;

/**
 * Send identity-scoped playback or reader progress; failures are logged without interrupting playback.
 * @param {number} mediaId
 * @param {number} position
 */
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
  if (partyPlaybackKind === 'audio') {
    ++nativeLoadGeneration;
    partyLoadPending = false;
    pbAudio.pause();
    clearInterval(progressTimer);
  }
  $('#playerbar').classList.remove('on');
};

/**
 * Format finite playback seconds as m:ss; nonfinite values display 0:00.
 * @param {number} s
 * @returns {string}
 */
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
let videoOpenGeneration = 0;

/**
 * Open a library video using saved progress only for this open generation and source.
 * Plyr replaces its native element and can forward retired metadata; the resume handler
 * therefore verifies element identity, item, generation and effective stream URL before
 * seeking. A later close or open invalidates the pending progress response.
 * @param {{id: number, title: string}} it Library video to load into the native modal.
 */
function openVideo(it) {
  const generation = ++videoOpenGeneration;
  const nativeGeneration = ++nativeLoadGeneration;
  beginNativePlayback('video');
  $('#playerbar').classList.remove('on');
  pbAudio.pause();
  videoItem = it;
  if (window.partySend) videoLoadPending = true;
  $('#vm-title').textContent = it.title;
  $('#video-modal').classList.add('on');
  $('#video-modal').classList.remove('yt');
  resumeFor(it.id).then(p => {
    if (videoItem !== it || generation !== videoOpenGeneration
      || nativeGeneration !== nativeLoadGeneration || partyPlaybackKind !== 'video') return;
    const streamPath = `/api/stream/${it.id}`;
    plyr.source = {
      type: 'video',
      sources: [{ src: streamPath, type: 'video/mp4' }],
    };
    const media = plyr.media;
    // Plyr forwards metadata from retired media; only this source may resume.
    if (p && p.position > 5) media.addEventListener('loadedmetadata', () => {
      if (ownsNativeLoad(media, 'video', it.id, nativeGeneration)
        && videoItem === it && generation === videoOpenGeneration) plyr.currentTime = p.position;
    }, { once: true });
    plyr.play();
  });
  trackProgress(plyr, it);
  startChromeAutoHide();
}

// auto-hide title/back/party when mouse idle over player
let chromeHideTimer = null;
/** Reveal the video overlay and restart its idle hide timer. */
function wakeChrome() {
  const ov = document.querySelector('.vm-overlay');
  if (!ov) return;
  ov.classList.remove('hidden');
  clearTimeout(chromeHideTimer);
  chromeHideTimer = setTimeout(() => {
    if ($('#video-modal').classList.contains('on')) ov.classList.add('hidden');
  }, 2500);
}
/** Attach mouse activity to the video overlay timer and reveal controls immediately. */
function startChromeAutoHide() {
  const modal = $('#video-modal');
  modal.onmousemove = wakeChrome;
  wakeChrome();
}

$('#vm-back').onclick = () => {
  ++videoOpenGeneration;
  videoLoadPending = false;
  if (partyPlaybackKind === 'video') {
    if (videoItem && plyr.media.readyState >= 1 && hasNativeStream(plyr.media, videoItem.id)
      && plyr.currentTime > 0) syncProgress(videoItem.id, plyr.currentTime);
    ++nativeLoadGeneration;
    // Preserve the position for the queued native pause event.
    plyr.pause();
    clearInterval(progressTimer);
  }
  $('#video-modal').classList.remove('on');
  videoItem = null;
};
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') $('#vm-back').click();
});
$('#vm-party').onclick = () => {
  // party panel opens over the player
  $('#party-modal').style.zIndex = 200;
  PartyUI.open();
};

// ---------- boot ----------
tryResume().then(resumed => {
  if (!resumed || localStorage.frayd_logout) {
    delete localStorage.frayd_logout;
    document.body.classList.add('auth');
    $('#login').style.display = 'flex';
  }
});
