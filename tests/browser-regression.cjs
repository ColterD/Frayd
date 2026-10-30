#!/usr/bin/env node
'use strict';

// Zero npm dependencies. Runs actual client scripts in a fresh, disposable browser.
const { createServer } = require('node:http');
const { spawn, execFileSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const { readFileSync, existsSync, mkdtempSync, rmSync, mkdirSync, writeFileSync, realpathSync } = require('node:fs');
const { join, resolve, extname, delimiter } = require('node:path');
const { tmpdir } = require('node:os');
const { setTimeout: delay } = require('node:timers/promises');

const root = resolve(__dirname, '..');
// Every full run must report exactly this many uniquely named cases.
const EXPECTED_CASES = 45;
const productionFiles = ['client/index.html', 'client/js/app.js', 'client/js/party.js', 'client/js/build-info.js',
  'client/vendor/plyr/plyr.min.js', 'client/vendor/plyr/plyr.min.css', 'client/vendor/plyr/plyr.svg', 'client/css/frayd.css', 'client/install/index.html'];
const harnessFiles = ['tests/browser-regression.cjs', 'tests/fixtures/browser-driver.js', 'tests/fixtures/reader-probe.js', 'tests/fixtures/native-resume.mp4', 'tests/fixtures/generate-native-resume.sh'];
function hash(path) { return createHash('sha256').update(readFileSync(path)).digest('hex'); }
/** Configure only the one app bootstrap without altering its exact cache-busted tag. */
function withLocalPlyrIcons(source) {
  const tags = source.match(/<script src="js\/app\.js(?:\?[^"<>]*)?"><\/script>/g) || [];
  if (tags.length !== 1) throw new Error('Expected exactly one production app script');
  return source.replace(tags[0], '<script>Plyr.defaults.iconUrl="/vendor/plyr/plyr.svg";</script>' + tags[0]);
}
function responsiveAudioFixture() {
  // Deterministic 120 s mono PCM; no file, dependency, download or physical output.
  const samples = 120 * 8000;
  const bytes = Buffer.alloc(44 + samples * 2);
  bytes.write('RIFF'); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write('WAVEfmt ', 8);
  bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(1, 22);
  bytes.writeUInt32LE(8000, 24); bytes.writeUInt32LE(16000, 28); bytes.writeUInt16LE(2, 32);
  bytes.writeUInt16LE(16, 34); bytes.write('data', 36); bytes.writeUInt32LE(samples * 2, 40);
  // Buffer.alloc leaves all PCM samples silent; only the WAV header is nonzero.
  return bytes;
}
function responsivePreload(width) {
  const originalFetch = window.fetch.bind(window);
  const fixture = window.__responsiveFixture = { width, progressResponses: [], mediaEvents: [], cardClicks: [], requests: [] };
  fixture.items = [
    ...Array.from({ length: 8 }, (_, n) => ({ id: 200 + n, kind: 'episode', title: `Show ${n} S01E01`, series: `Fixture show ${n}`, created_at: [4, 1, 8, 2, 7, 3, 6, 5][n] })),
    ...Array.from({ length: 6 }, (_, n) => ({ id: 60 + n, kind: 'movie', title: n ? `Movie ${n}` : 'Date With Your Family, A - http://www.archive.org/details/DateWith1950', author: n ? '' : 'UnbrokenAuthor'.repeat(15), created_at: n })),
    ...Array.from({ length: 8 }, (_, n) => ({ id: 23 + n, kind: 'music', title: n ? `Audio ${n}` : 'Long player title '.repeat(8), author: 'Fixture artist', created_at: n })),
    ...['ebook', 'comic', 'manga', 'audiobook'].flatMap((kind, k) => Array.from({ length: 8 }, (_, n) => ({
      id: 70 + k * 10 + n, kind, title: `${kind} book ${n}`, author: 'Fixture author', created_at: n,
    }))),
  ];
  // Per-document auth double: never mutate the parent profile's storage or account.
  Object.defineProperty(window, 'localStorage', { value: { frayd_identity: `owned-responsive-${width}`, frayd_token: 'owned-fixture-model' } });
  document.addEventListener('click', event => {
    const card = event.target.closest?.('.media-card');
    if (!card) return;
    const item = fixture.items.find(item => card.title.startsWith(item.title));
    fixture.cardClicks.push({ id: item?.id || null, isTrusted: event.isTrusted, title: card.title });
  }, true);
  const reply = (value, status = 200) => new Response(status === 204 ? null : JSON.stringify(value), {
    status, headers: { 'Content-Type': 'application/json' },
  });
  window.fetch = async (url, options = {}) => {
    const path = new URL(String(url), location.origin).pathname;
    if (path.startsWith('/__fixture/')) return originalFetch(url, options);
    fixture.requests.push({ path, method: options.method || 'GET' });
    if (path === '/api/build') return reply({ version: '0.1.1', commit: 'a'.repeat(40), commits_since_tag: 131, dirty: false });
    if (path === '/api/library') return reply(fixture.items);
    if (path === '/api/auth/me') return reply({ name: 'Responsive admin', is_admin: true });
    if (path === '/api/progress/23') {
      fixture.progressResponses.push({ url: String(url), id: 23, status: 200, position: 0 });
      return reply({ position: 0 });
    }
    if (path.startsWith('/api/progress/')) return reply(null);
    if (path === '/api/progress') return reply(null, 204);
    if (path === '/api/scan') return reply(null, 409);
    if (path === '/api/scan/status') return reply({ scanning: false });
    if (path === '/api/admin/perms') return reply([]);
    if (path === '/api/admin/settings') return reply({ library_paths: [], library_locked: false, server_version: 'fixture' });
    throw new Error('Unknown responsive API fixture request: ' + path);
  };
  for (const name of ['loadstart', 'loadedmetadata', 'loadeddata', 'canplay', 'playing', 'pause', 'seeking', 'seeked', 'ended', 'error']) {
    document.addEventListener(name, event => {
      const media = event.target;
      if (media.id !== 'pb-audio' || fixture.mediaEvents.length >= 80) return;
      fixture.mediaEvents.push({ event: name, currentTime: media.currentTime, duration: Number.isFinite(media.duration) ? media.duration : null,
        readyState: media.readyState, paused: media.paused, source: media.currentSrc || media.src,
        mediaError: media.error?.code || null });
    }, true);
  }
}
/** Preload only in the installer fixture; production inline script stays byte-identical.
 * CSP additionally forbids network connections even if this adapter is bypassed.
 */
function installerPreload() {
  const fixture = window.__installerFixture = {
    mode: location.pathname.split('/').pop(), requests: [], storage: [], errors: [], held: [],
  };
  addEventListener('error', event => fixture.errors.push(event.message));
  addEventListener('unhandledrejection', event => fixture.errors.push(String(event.reason)));
  for (const name of ['localStorage', 'sessionStorage']) Object.defineProperty(window, name, {
    get() { fixture.storage.push(name); throw new Error('Forbidden installer storage access'); },
  });
  const json = (value, contentType = 'application/json') => new Response(JSON.stringify(value), {
    headers: { 'Content-Type': contentType },
  });
  window.fetch = async (url, options = {}) => {
    const text = String(url);
    const entry = { url: text, method: options.method || 'GET', credentials: options.credentials,
      cache: options.cache, aborted: false };
    fixture.requests.push(entry);
    options.signal?.addEventListener('abort', () => { entry.aborted = true; });
    if (!['http://127.0.0.1:8484/api/health', 'http://127.0.0.1:8484/api/setup'].includes(text)
      || entry.method !== 'GET' || options.credentials !== 'omit' || options.cache !== 'no-store') {
      throw new Error('Forbidden installer request');
    }
    if (fixture.mode === 'held') return new Promise(resolve => fixture.held.push(resolve));
    if (fixture.mode === 'unreadable') throw new TypeError('Modeled browser CORS refusal');
    if (fixture.mode === 'html') return new Response('<html>unrelated</html>', { headers: { 'Content-Type': 'text/html' } });
    if (fixture.mode === 'malformed') return new Response('{', { headers: { 'Content-Type': 'application/json' } });
    if (fixture.mode === 'opaque') { const result = json({ status: 'ok' }); Object.defineProperty(result, 'type', { value: 'opaque' }); return result; }
    if (text.endsWith('/api/health')) return json({ status: fixture.mode === 'unrelated' ? 'different-service' : 'ok' });
    return json({ needed: fixture.mode === 'nonboolean' ? 'true' : fixture.mode !== 'signed-in' });
  };
  fixture.release = () => fixture.held.splice(0).forEach(resolve => resolve(json({ status: 'ok' })));
  // Resolve production lexical bindings without eval or weakening fixture CSP.
  fixture.inspectRetirement = () => checkTimer === null && checkTimeout === null && checkController === null;
}
/**
 * Resolve an existing BROWSER_BIN or PATH browser without installing or downloading
 * software. Identity comes from the browser protocol after launch.
 */
function browserPath() {
  if (process.env.BROWSER_BIN) return realpathSync(process.env.BROWSER_BIN);
  for (const command of ['google-chrome', 'chromium', 'chromium-browser', 'chrome', 'chrome-headless-shell']) {
    for (const directory of (process.env.PATH || '').split(delimiter)) {
      const path = join(directory, command);
      if (existsSync(path)) return realpathSync(path);
    }
  }
  throw new Error('No browser found. Set BROWSER_BIN to an existing Chrome/Chromium executable; no download is performed.');
}
/**
 * Signal the detached browser's entire process group, including with signal 0 for
 * liveness. Only ESRCH means the group is gone; permission and other errors propagate.
 */
function signalGroup(pid, signal) {
  try { process.kill(-pid, signal); return true; }
  catch (error) { if (error.code === 'ESRCH') return false; throw error; }
}
/**
 * Escalate TERM to KILL and confirm that the process group no longer exists.
 * Reject unconfirmed shutdown so the runner retains its profile instead of deleting it.
 */
async function stopBrowserGroup(browser) {
  if (!browser?.pid) return;
  /**
   * Poll process-group liveness rather than the launcher PID: a leader can exit while
   * descendants still write the profile. Only confirmed group absence permits deletion.
   */
  async function waitForExit(milliseconds) {
    const deadline = Date.now() + milliseconds;
    while (signalGroup(browser.pid, 0)) {
      if (Date.now() >= deadline) return false;
      await delay(25);
    }
    return true;
  }
  signalGroup(browser.pid, 'SIGTERM');
  if (await waitForExit(750)) return;
  signalGroup(browser.pid, 'SIGKILL');
  if (!await waitForExit(2000)) throw new Error(`Browser process group ${browser.pid} did not exit after SIGKILL; profile retained`);
}
/**
 * Remove DevTools listening lines and WebSocket capability URLs from diagnostics.
 * This targets debugger capabilities; it is not a general secret or log sanitizer.
 */
function redactDebuggerCapabilities(message) {
  return String(message).replace(/DevTools listening on [^\r\n]*/g, 'DevTools endpoint redacted')
    .replace(/wss?:\/\/[^\s"']+/g, '[debugger endpoint redacted]');
}
/**
 * Reject when the wait exceeds its bound and always clear the timer. The underlying
 * operation is not cancelled; callers must supply cancellation or clean it up separately.
 */
async function bounded(promise, message, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}
/**
 * Spend one shared startup deadline discovering the blank page and configuring
 * its cache before navigation. Retry transient HTTP readiness only; unexpected targets
 * or debugger ownership fail closed. Keep the protocol open until caller teardown.
 */
async function configureOwnedBrowserCache({ profile, fixtureUrl, browser, diagnostics = {}, budgetMs = 30000 }) {
  const startedAt = Date.now();
  const deadline = startedAt + budgetMs;
  Object.assign(diagnostics, { budgetMs, phase: 'port-file', attempts: 0, transientFailures: 0, emptyLists: 0 });
  function phase(name) { diagnostics.phase = name; diagnostics.elapsedMs = Date.now() - startedAt; }
  /**
   * Return time left on the single startup deadline; advancing to another phase or
   * retrying discovery never resets it. Reject before starting work after expiration.
   */
  function remainingStartupBudget() {
    diagnostics.elapsedMs = Date.now() - startedAt;
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error('Browser startup deadline exceeded');
    return remaining;
  }
  const portFile = join(profile, 'DevToolsActivePort');
  let port;
  while (true) {
    if (browser.exitCode !== null || browser.signalCode !== null) throw new Error('Browser exited before cache setup');
    diagnostics.elapsedMs = Date.now() - startedAt;
    if (Date.now() >= deadline) throw new Error('Browser did not expose its cache-control endpoint');
    if (existsSync(portFile)) {
      const contents = readFileSync(portFile, 'utf8');
      const firstLine = contents.split('\n')[0].trim();
      port = contents.includes('\n') && /^\d+$/.test(firstLine) ? Number(firstLine) : NaN;
      if (Number.isInteger(port) && port >= 1 && port <= 65535) break;
    }
    await delay(20);
  }
  diagnostics.portReadyElapsedMs = Date.now() - startedAt;
  phase('http-discovery');
  let pages;
  const browserExited = new AbortController();
  const onBrowserExit = () => browserExited.abort();
  browser.once('exit', onBrowserExit);
  try {
    while (true) {
      if (browser.exitCode !== null || browser.signalCode !== null) throw new Error('Browser exited before cache setup');
      // Allow a slow valid response to finish within the one startup deadline.
      // The same signal bounds headers and body and aborts immediately on browser exit.
      const signal = AbortSignal.any([AbortSignal.timeout(remainingStartupBudget()), browserExited.signal]);
      diagnostics.attempts++;
      phase('http-headers');
      let response;
      let targets;
      try {
        response = await fetch(`http://127.0.0.1:${port}/json/list`, { signal });
        if (response.ok) {
          phase('http-body');
          targets = await response.json();
        }
      } catch (error) {
        if (browser.exitCode !== null || browser.signalCode !== null) throw new Error('Browser exited before cache setup');
        remainingStartupBudget();
        if (!['TypeError', 'AbortError', 'TimeoutError'].includes(error.name)) throw error;
        diagnostics.transientFailures++;
        await delay(20);
        continue;
      }
      remainingStartupBudget();
      if (!response.ok) {
        if (response.status === 408 || response.status === 429 || response.status >= 500) {
          if (response.body) await bounded(response.body.cancel(),
            'Browser page discovery body cancellation timed out', remainingStartupBudget());
          diagnostics.transientFailures++;
          await delay(20);
          continue;
        }
        throw new Error('Browser page discovery failed');
      }
      if (!Array.isArray(targets)) throw new Error('Invalid browser page discovery response');
      pages = targets.filter(page => page.type === 'page');
      if (pages.length === 0) {
        diagnostics.emptyLists++;
        await delay(20);
        continue;
      }
      if (pages.length !== 1 || pages[0].url !== 'about:blank') throw new Error('Browser must have exactly one blank page before navigation');
      break;
    }
  } finally { browser.removeListener('exit', onBrowserExit); }
  const endpoint = new URL(pages[0].webSocketDebuggerUrl);
  if (endpoint.protocol !== 'ws:' || endpoint.hostname !== '127.0.0.1' || endpoint.port !== String(port)
    || !/^\/devtools\/page\/[a-zA-Z0-9-]+$/.test(endpoint.pathname)) throw new Error('Unexpected browser page endpoint');
  phase('socket-open');
  const openBudget = remainingStartupBudget();
  const socket = new WebSocket(endpoint.href);
  const pending = new Map();
  let sequence = 0;
  let navigated = false;
  let acceptDialog = false;
  const dialogs = [];
  const blockedRequests = [];
  const protocolErrors = [];
  const canceledInterceptions = [];
  const frameStates = new Map();
  const canceledRequests = new Set();
  // Invalid-interception errors waiting for their network request's cancellation event.
  const unconfirmedInterceptions = new Map();
  const safeProtocolMessages = new Set(['Invalid InterceptionId.', 'Invalid parameters', 'Invalid RequestId.', 'No resource with given identifier found']);
  let shuttingDown = false;
  socket.addEventListener('message', event => {
    let message;
    try { message = JSON.parse(event.data); }
    catch { for (const waiter of pending.values()) waiter.reject(new Error('Invalid browser protocol response')); return; }
    if (message.method === 'Page.frameNavigated') {
      const frame = message.params.frame;
      const prior = frameStates.get(frame.id);
      if (prior?.loaderId && prior.loaderId !== frame.loaderId) prior.navigated = true;
      frameStates.set(frame.id, { loaderId: frame.loaderId, navigated: false, detached: false });
    }
    if (message.method === 'Page.frameDetached') {
      const frame = frameStates.get(message.params.frameId);
      if (frame) frame.detached = true;
    }
    if (message.method === 'Network.loadingFailed' && message.params.canceled === true) {
      canceledRequests.add(message.params.requestId);
      const retired = unconfirmedInterceptions.get(message.params.requestId);
      if (retired) {
        unconfirmedInterceptions.delete(message.params.requestId);
        protocolErrors.splice(protocolErrors.indexOf(retired), 1);
        retired.lifecycle.matchingNetworkCanceled = true;
        canceledInterceptions.push(retired);
      }
    }
    if (message.method === 'Fetch.requestPaused') {
      const { requestId, request } = message.params;
      const frame = frameStates.get(message.params.frameId);
      const networkId = message.params.networkId;
      const owned = new URL(request.url).origin === new URL(fixtureUrl).origin;
      if (!owned) blockedRequests.push({ url: request.url, resourceType: message.params.resourceType });
      rpc(owned ? 'Fetch.continueRequest' : 'Fetch.failRequest', owned ? { requestId }
        : { requestId, errorReason: 'BlockedByClient' }).catch(error => {
        if (shuttingDown) return;
        const diagnostic = { method: error.rpcMethod || (owned ? 'Fetch.continueRequest' : 'Fetch.failRequest'), code: error.rpcCode ?? 'transport-or-timeout',
          message: error.rpcMessage || 'protocol transport or timeout', resourceType: message.params.resourceType, ownedRequest: owned,
          lifecycle: { networkIdObserved: Boolean(networkId), matchingNetworkCanceled: canceledRequests.has(networkId),
            knownFrameAtPause: Boolean(frame), frameNavigated: frame?.navigated === true, frameDetached: frame?.detached === true } };
        // Only this exact browser response plus the same network request's explicit canceled=true event,
        // before or after this reply, proves that a paused interception has already retired.
        const retiredInterception = error.rpcCode === -32602 && error.rpcMessage === 'Invalid InterceptionId.' && networkId;
        if (retiredInterception && canceledRequests.has(networkId)) canceledInterceptions.push(diagnostic);
        else {
          protocolErrors.push(diagnostic);
          if (retiredInterception) unconfirmedInterceptions.set(networkId, diagnostic);
        }
      });
    }
    if (message.method === 'Page.javascriptDialogOpening') {
      dialogs.push({ type: message.params.type, message: message.params.message, accepted: acceptDialog });
      rpc('Page.handleJavaScriptDialog', { accept: acceptDialog }).catch(() => {});
    }
    const waiter = pending.get(message.id);
    if (!waiter) return;
    if (message.error) {
      const code = Number.isInteger(message.error.code) ? message.error.code : 'unknown';
      const detail = safeProtocolMessages.has(message.error.message) ? message.error.message : 'unrecognized CDP message';
      const error = new Error(`Browser RPC ${waiter.method} failed (${code}): ${detail}`);
      Object.assign(error, { rpcMethod: waiter.method, rpcCode: code, rpcMessage: detail });
      waiter.reject(error);
    }
    else waiter.resolve(message.result);
  });
  socket.addEventListener('error', () => {
    for (const waiter of pending.values()) waiter.reject(new Error('Browser cache protocol connection failed'));
  });
  /**
   * Send one bootstrap command on the protocol session and correlate its reply
   * by ID. Always remove the pending entry; failures propagate without command retry.
   */
  async function rpc(method, params = {}) {
    if (!navigated) phase(method);
    const rpcBudget = navigated ? 3000 : remainingStartupBudget();
    const id = ++sequence;
    try {
      const result = new Promise((resolve, reject) => { pending.set(id, { resolve, reject, method }); });
      socket.send(JSON.stringify({ id, method, params }));
      return await bounded(result, `Browser cache RPC ${method} timed out`, rpcBudget);
    } finally { pending.delete(id); }
  }
  const receipt = { cacheDisabled: false, browserCacheCleared: false, configuredBeforeFixtureNavigation: false, protocolClosed: false };
  /**
   * Close only this protocol session after tests, keeping its cache overrides
   * active until then. Teardown has a separate three-second bound, outside startup.
   */
  async function close() {
    if (socket.readyState !== WebSocket.CLOSED) {
      const closed = new Promise(resolve => socket.addEventListener('close', resolve, { once: true }));
      socket.close();
      await bounded(closed, 'Browser cache protocol close timed out');
    }
    receipt.protocolClosed = true;
  }
  try {
    await bounded(new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', () => reject(new Error('Browser cache protocol failed to open')), { once: true });
    }), 'Browser cache protocol open timed out', openBudget);
    const identity = await rpc('Browser.getVersion');
    if (typeof identity.product !== 'string' || !identity.product.trim()
      || typeof identity.revision !== 'string' || !identity.revision.trim()) {
      throw new Error('Invalid browser identity response');
    }
    Object.assign(receipt, { browserVersion: identity.product, browserRevision: identity.revision });
    await rpc('Page.enable');
    // Intercept before navigation: no request can reach an operator loopback service.
    await rpc('Fetch.enable', { patterns: [{ urlPattern: '*', requestStage: 'Request' }] });
    await rpc('Network.enable');
    await rpc('Network.setCacheDisabled', { cacheDisabled: true });
    await rpc('Network.clearBrowserCache');
    const navigation = await rpc('Page.navigate', { url: fixtureUrl });
    if (navigation.errorText) throw new Error('Browser fixture navigation failed');
    Object.assign(receipt, { cacheDisabled: true, browserCacheCleared: true, configuredBeforeFixtureNavigation: true });
    // Keep Network overrides active until the runner finishes all media cases.
    navigated = true;
    phase('ready');
    // Only fixed keyboard commands are exposed to the loopback fixture, never arbitrary RPC.
    async function key(key, accept = false) {
      const codes = { Tab: 9, Enter: 13, ' ': 32, ArrowRight: 39, t: 84 };
      if (!(key in codes)) throw new Error('Unknown fixture keyboard key');
      acceptDialog = accept;
      const before = dialogs.length;
      await rpc('Input.dispatchKeyEvent', { type: 'keyDown', key,
        windowsVirtualKeyCode: codes[key], ...(key === 'Enter' ? { text: '\r' } : key.length === 1 ? { text: key } : {}) });
      await rpc('Input.dispatchKeyEvent', { type: 'keyUp', key, windowsVirtualKeyCode: codes[key] });
      return { key, dialogs: dialogs.slice(before), trustedBrowserInput: true };
    }
    async function viewport(width, height = 1000) {
      if (![320, 768, 1024, 1440].includes(width)) throw new Error('Unknown fixture viewport');
      if (height !== 1000 && !(width === 320 && height === 568) && !(width === 768 && height === 600)) throw new Error('Unknown fixture viewport height');
      await rpc('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
      return { width, height, cssPixels: true };
    }
    async function click(x, y) {
      if (![x, y].every(Number.isFinite) || x < 0 || x > 1440 || y < 0 || y > 1000) throw new Error('Invalid fixture click coordinates');
      await rpc('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
      await rpc('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 });
      await rpc('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 });
      return { x, y, trustedBrowserInput: true };
    }
    Object.assign(receipt, { blockedRequests, protocolErrors, canceledInterceptions, networkRestrictedToOwnedFixture: true });
    return { receipt, close, key, viewport, click, beginShutdown() { shuttingDown = true; receipt.networkGuardUntilBrowserStop = true; } };
  } catch (error) {
    await close();
    throw error;
  }
}
/**
 * Run the production client against an loopback fixture, process group and fresh
 * profile. Record original and cleanup failures separately; remove the profile only
 * after confirmed group shutdown, retaining it when shutdown cannot be established.
 */
async function main({ removeProfile = rmSync, resultDirectory = process.env.FRAYD_DOM_RESULT_DIR, cacheSetup = configureOwnedBrowserCache,
  stopGroup = stopBrowserGroup, startupBudgetMs = 30000, caseBudgetMs = 60000 } = {}) {
  if (Number(process.versions.node.split('.')[0]) < 22 || typeof WebSocket !== 'function') {
    throw new Error('Browser regressions require Node.js 22+ with the global WebSocket API');
  }
  if (!Number.isInteger(startupBudgetMs) || startupBudgetMs < 1 || startupBudgetMs > 30000
    || !Number.isInteger(caseBudgetMs) || caseBudgetMs < 1 || caseBudgetMs > 60000) {
    throw new Error('Phase budgets must be positive milliseconds, at most 30000 for startup and 60000 for cases');
  }
  let browser;
  let profile;
  let pendingAnnotation;
  let parserGate = false;
  let parserResponse;
  let heldMedia = null;
  let pendingMedia = 0;
  const sockets = new Set();
  let body = '';
  let annotations = [];
  let posts = [];
  let logs = '';
  let output;
  let provenance;
  let runError;
  let cacheControl;
  let cacheSession;
  const startupDiagnostics = {};
  let phaseStartedAt;
  const phaseTimeline = [];
  function recordPhase(phase) { phaseTimeline.push({ phase, elapsedMs: Date.now() - phaseStartedAt }); }
  let caseProgress = { completed: 0, passed: 0, failed: 0, blocked: 0, currentCase: null };
  const cleanupErrors = [];
  let browserGroupStopped = false;
  let resolveResult;
  const result = new Promise(resolve => { resolveResult = resolve; });
  const unexpectedRequests = [];
  const nativeRequests = [];
  const mediaGateEvents = [];
  const responsiveAudio = responsiveAudioFixture();
  /**
   * Clear ownership of the held native stream before waking its waiters. Repeated
   * driver or cleanup releases are harmless and report whether a gate was released.
   */
  function releaseMedia(reason) {
    const gate = heldMedia;
    heldMedia = null;
    if (gate) {
      mediaGateEvents.push({ event: 'release', path: gate.path, waited: gate.waited, reason });
      gate.release();
    }
    return { released: Boolean(gate), waited: gate?.waited || 0 };
  }
  const probe = readFileSync(join(root, 'tests/fixtures/reader-probe.js'), 'utf8');
  const contentTypes = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
  const server = createServer(async (request, response) => {
    function json(value, status = 200) {
      response.writeHead(status, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify(value));
    }
    try {
      const path = new URL(request.url, 'http://fixture.invalid').pathname;
      let input = '';
      for await (const chunk of request) {
        input += chunk;
        if (input.length > 1024 * 1024) throw new Error('Fixture request exceeds size bound');
      }
      if (path === '/__fixture/viewport' && request.method === 'POST') {
        const viewport = JSON.parse(input);
        return json(await cacheSession.viewport(viewport.width, viewport.height));
      }
      if (path === '/__fixture/click' && request.method === 'POST') { const point = JSON.parse(input); return json(await cacheSession.click(point.x, point.y)); }
      if (path === '/__fixture/responsive' || path === '/__fixture/toolbar-retry') {
        const width = Number(new URL(request.url, 'http://fixture.invalid').searchParams.get('width'));
        if (![320, 768, 1024, 1440].includes(width)) return json({}, 400);
        const mode = path === '/__fixture/toolbar-retry' ? new URL(request.url, 'http://fixture.invalid').searchParams.get('mode') : null;
        // Render only source-owned literals; request strings never enter inline script.
        let modeLiteral;
        switch (mode) {
          case null: modeLiteral = 'null'; break;
          case 'transient': modeLiteral = '"transient"'; break;
          case 'reattach': modeLiteral = '"reattach"'; break;
          default: return json({}, 400);
        }
        const source = readFileSync(join(root, 'client/index.html'), 'utf8');
        response.writeHead(200, { 'Content-Type': 'text/html' });
        return response.end(withLocalPlyrIcons(source).replace('<head>', '<head><base href="/"><script>(' + responsivePreload.toString() + ')(' + width + ');window.__toolbarRetryMode=' + modeLiteral + '</script>')
          .replace('</body>', '<script src="/__fixture/browser-driver.js"></script></body>'));
      }
      if (path.startsWith('/__fixture/installer/')) {
        if (!['valid', 'signed-in', 'opaque', 'html', 'malformed', 'unrelated', 'unreadable', 'nonboolean', 'held'].includes(path.split('/').pop())) return json({}, 404);
        const source = readFileSync(join(root, 'client/install/index.html'), 'utf8');
        response.writeHead(200, { 'Content-Type': 'text/html',
          'Content-Security-Policy': "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'none'; form-action 'none'; base-uri 'none'" });
        return response.end(source.replace('<head>', '<head><script>(' + installerPreload.toString() + ')()</script>'));
      }
      if (path === '/__fixture/key' && request.method === 'POST') {
        const command = JSON.parse(input);
        return json(await cacheSession.key(command.key, command.accept));
      }
      if (path === '/__fixture/hold-media' && request.method === 'POST') {
        const target = JSON.parse(input).path;
        if (!/^\/api\/stream\/(20|22|23)$/.test(target)) return json({ error: 'Unknown requested native stream' }, 400);
        if (heldMedia) return json({ error: 'Previous native stream hold still active' }, 409);
        let release;
        const promise = new Promise(resolve => { release = resolve; });
        heldMedia = { path: target, promise, release, waited: 0 };
        mediaGateEvents.push({ event: 'hold', path: target });
        return json({ held: true, path: target });
      }
      if (path === '/__fixture/media-gate') return json({ held: Boolean(heldMedia), pending: pendingMedia });
      if (path === '/__fixture/release-media' && request.method === 'POST') return json(releaseMedia('driver'));
      if (path === '/__fixture/configure' && request.method === 'POST') {
        if (pendingAnnotation) throw new Error('Previous annotation response still pending');
        const fixture = JSON.parse(input);
        body = fixture.body;
        parserGate = Boolean(fixture.parserGate);
        annotations = fixture.annotations;
        posts = [];
        return json({ ok: true });
      }
      if (path === '/__fixture/parser.js') { parserResponse = response; return; }
      if (path === '/__fixture/parser-state') return json({ held: Boolean(parserResponse) });
      if (path === '/__fixture/release-parser' && request.method === 'POST') {
        if (parserResponse) { parserResponse.writeHead(200, { 'Content-Type': 'text/javascript' }); parserResponse.end(''); parserResponse = null; }
        return json({ ok: true });
      }
      if (path === '/__fixture/state') return json({ posts, annotations });
      if (path === '/__fixture/release' && request.method === 'POST') {
        if (!pendingAnnotation) throw new Error('No annotation response to release');
        annotations.push(posts[posts.length - 1]);
        pendingAnnotation.writeHead(201, { 'Content-Type': 'application/json' });
        pendingAnnotation.end('{}');
        pendingAnnotation = null;
        return json({ ok: true });
      }
      if (path === '/__fixture/progress' && request.method === 'POST') {
        const { completed, passed, failed, blocked, currentCase } = JSON.parse(input);
        const counts = [completed, passed, failed, blocked];
        if (!counts.every(value => Number.isInteger(value) && value >= 0 && value <= EXPECTED_CASES)
          || passed + failed + blocked !== completed
          || !(currentCase === null || currentCase === completed + 1 && currentCase <= EXPECTED_CASES)) return json({}, 400);
        // Requests may arrive out of order; progress never supplies terminal case evidence.
        if (completed > caseProgress.completed || completed === caseProgress.completed && currentCase !== null) {
          caseProgress = { completed, passed, failed, blocked, currentCase, elapsedMs: Date.now() - phaseStartedAt };
        }
        return json({ ok: true });
      }
      if (path === '/__fixture/result' && request.method === 'POST') {
        json({ ok: true });
        resolveResult(JSON.parse(input));
        return;
      }
      if (path === '/api/annotations' && request.method === 'POST') {
        if (pendingAnnotation) throw new Error('Concurrent annotation request');
        posts.push(JSON.parse(input));
        pendingAnnotation = response;
        return;
      }
      if (/^\/api\/epub\/7\/chunk\/[01]$/.test(path)) {
        response.writeHead(200, { 'Content-Type': 'text/html' });
        return response.end(`<!doctype html><html><head><meta charset="utf-8">${parserGate ? `<script src="http://127.0.0.1:${server.address().port}/__fixture/parser.js"></script>` : ''}<script>${probe}</script></head><body>${body}</body></html>`);
      }
      if (path === '/api/epub/7/spine') return json(['one.xhtml', 'two.xhtml']);
      if (path === '/api/epub/7/toc') return json([]);
      if (path === '/api/annotations/7') return json(annotations);
      if (/^\/api\/comic\/(90|91)\/page\/[0-4]$/.test(path)) {
        response.writeHead(200, { 'Content-Type': 'image/png' });
        return response.end(Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=', 'base64'));
      }
      if (path === '/api/progress/20' || path === '/api/progress/22') { nativeRequests.push({ path, status: 200, position: 64 }); return json({ position: 64 }); }
      if (path === '/api/progress/21') { nativeRequests.push({ path, status: 200, position: 0 }); return json({ position: 0 }); }
      if (path === '/api/progress/23') { nativeRequests.push({ path, status: 200, position: 0 }); return json({ position: 0 }); }
      if (path.startsWith('/api/progress/')) return json(null);
      if (path === '/api/progress') return json({});
      if (path === '/api/build') return json({ version: '0.1.1', commit: 'a'.repeat(40), commits_since_tag: 131, dirty: false });
      if (path === '/__fixture/build-unknown') {
        response.writeHead(200, { 'Content-Type': 'text/html' });
        return response.end('<!doctype html><html><body><span data-build-info></span><script>window.fetch=async()=>new Response(JSON.stringify({version:"0.1.1",commit:null,dirty:null,commits_since_tag:null}));</script><script src="/js/build-info.js"></script></body></html>');
      }
      if (path === '/api/library') return json([]);
      if (path === '/api/auth/logout' && request.method === 'POST') { response.writeHead(204); return response.end(); }
      if (path === '/api/thumb/9' || path === '/api/thumb/10' || /^\/api\/thumb\/(6[0-9]|7[0-9]|20[0-7])$/.test(path)) {
        response.writeHead(200, { 'Content-Type': 'image/png' });
        return response.end(Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=', 'base64'));
      }
      if (/^\/api\/stream\/(20|21|22|23)$/.test(path)) {
        const gate = heldMedia;
        if (gate?.path === path) {
          gate.waited++;
          pendingMedia++;
          mediaGateEvents.push({ event: 'held-request', path });
          try { await gate.promise; }
          finally { pendingMedia--; }
          if (response.destroyed) return;
        }
        const media = path === '/api/stream/23' ? responsiveAudio : readFileSync(join(root, 'tests/fixtures/native-resume.mp4'));
        const range = request.headers.range?.match(/^bytes=(\d+)-(\d*)$/);
        const start = range ? Number(range[1]) : 0;
        const end = range?.[2] ? Math.min(Number(range[2]), media.length - 1) : media.length - 1;
        if (start > end || start >= media.length) {
          response.writeHead(416, { 'Content-Range': `bytes */${media.length}` });
          return response.end();
        }
        nativeRequests.push({ path, status: range ? 206 : 200, start, end });
        response.writeHead(range ? 206 : 200, { 'Content-Type': path === '/api/stream/23' ? 'audio/wav' : 'video/mp4', 'Accept-Ranges': 'bytes', 'Cache-Control': 'no-store',
          'Content-Length': end - start + 1, ...(range ? { 'Content-Range': `bytes ${start}-${end}/${media.length}` } : {}) });
        return response.end(media.subarray(start, end + 1));
      }
      if (/^\/api\/stream\/(9|10|11)$/.test(path)) { response.writeHead(204); return response.end(); }
      if (path === '/favicon.ico') { response.writeHead(204); return response.end(); }
      const staticFile = path === '/' ? 'client/index.html'
        : path === '/__fixture/browser-driver.js' ? 'tests/fixtures/browser-driver.js'
          : 'client' + path;
      if (![...productionFiles, 'tests/fixtures/browser-driver.js'].includes(staticFile)) {
        unexpectedRequests.push({ method: request.method, path });
        return json({ error: 'Unexpected fixture route' }, 404);
      }
      let bytes = readFileSync(join(root, staticFile));
      if (path === '/') bytes = Buffer.from(withLocalPlyrIcons(bytes.toString())
        .replace('</body>', '<script src="/__fixture/browser-driver.js"></script></body>'));
      response.writeHead(200, { 'Content-Type': contentTypes[extname(staticFile)] || 'application/octet-stream' });
      response.end(bytes);
    } catch (error) {
      unexpectedRequests.push({ error: error.message });
      if (!response.headersSent) json({ error: error.message }, 500);
      else response.end();
    }
  });
  server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
  try {
    const executable = browserPath();
    provenance = {
      capturedAt: new Date().toISOString(),
      head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8', timeout: 10000 }).trim(),
      executable,
      executableSha256: hash(executable), nodeVersion: process.version,
      files: Object.fromEntries([...productionFiles, ...harnessFiles].map(file => [file, hash(join(root, file))])),
    };
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    profile = mkdtempSync(join(tmpdir(), 'frayd-browser-regression-'));
    const url = `http://127.0.0.1:${server.address().port}/`;
    const ownedCacheControl = cacheSetup === configureOwnedBrowserCache;
    phaseStartedAt = Date.now();
    recordPhase('startup');
    browser = spawn(executable, ['--headless', '--no-sandbox', '--disable-gpu', '--mute-audio', '--disable-background-networking',
      '--disable-component-update', '--disable-sync', '--no-first-run', '--no-default-browser-check',
      '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1', '--disable-features=MediaRouter',
      ...(ownedCacheControl ? ['--remote-debugging-address=127.0.0.1', '--remote-debugging-port=0'] : []),
      `--user-data-dir=${profile}`, ownedCacheControl ? 'about:blank' : url], { detached: true, stdio: ['ignore', 'ignore', 'pipe'] });
    let stderrPending = '';
    browser.stderr.on('data', bytes => {
      stderrPending += bytes.toString();
      const lines = stderrPending.split('\n');
      stderrPending = lines.pop();
      logs = (logs + lines.map(redactDebuggerCapabilities).join('\n') + '\n').slice(-12000);
    });
    const exited = new Promise((_, reject) => {
      browser.once('error', reject);
      browser.once('exit', (code, signal) => reject(new Error(`Browser exited before result: ${code}/${signal}; ${logs}`)));
    });
    // Startup includes cache configuration and the navigation acknowledgement. Only
    // then does the unchanged case budget begin; neither phase resets on retries.
    await bounded(Promise.race([(async () => {
      cacheSession = await cacheSetup({ profile, fixtureUrl: url, browser, diagnostics: startupDiagnostics, budgetMs: startupBudgetMs });
      cacheControl = cacheSession.receipt;
      provenance.browserVersion = cacheControl.browserVersion;
      provenance.browserRevision = cacheControl.browserRevision;
    })(), exited]), `Browser startup exceeded ${startupBudgetMs} milliseconds`, startupBudgetMs);
    recordPhase('cases');
    const observed = await bounded(Promise.race([result, exited]),
      `Browser regression cases exceeded ${caseBudgetMs} milliseconds`, caseBudgetMs);
    const complete = observed.cases.length === EXPECTED_CASES
      && new Set(observed.cases.map(item => item.name)).size === EXPECTED_CASES
      && observed.cases.every(item => item.status === 'PASS')
      && observed.passed === EXPECTED_CASES && observed.failed === 0;
    output = { provenance, ...observed, expectedCases: EXPECTED_CASES, complete, unexpectedRequests, nativeRequests };
    if (!complete || observed.failed || observed.errors.length || unexpectedRequests.length) process.exitCode = 1;
    if (cacheControl.protocolErrors?.length || cacheControl.blockedRequests?.some(item => item.resourceType !== 'Font'
      && !item.url.startsWith('https://fonts.googleapis.com/')
      && item.url !== 'https://cdn.plyr.io/static/blank.mp4')) process.exitCode = 1;
  } catch (error) {
    runError = error;
  } finally {
    if (phaseStartedAt !== undefined) recordPhase('cleanup');
    releaseMedia('runner-cleanup');
    // Keep interception active through group death; detaching it while a browser is
    // alive can release paused requests to operator services.
    cacheSession?.beginShutdown?.();
    try { await stopGroup(browser); browserGroupStopped = true; }
    catch (error) { cleanupErrors.push(error.stack); }
    if (browserGroupStopped) {
      try { await cacheSession?.close(); }
      catch (error) { cleanupErrors.push(redactDebuggerCapabilities(error.stack)); }
    }
    try {
      if (pendingAnnotation) pendingAnnotation.destroy();
      if (parserResponse) parserResponse.destroy();
      for (const socket of sockets) socket.destroy();
      await new Promise(resolve => server.close(resolve));
    } catch (error) { cleanupErrors.push(error.stack); }
    if (profile && browserGroupStopped) {
      try { removeProfile(profile, { recursive: true, force: true }); }
      catch (error) { cleanupErrors.push(error.stack); }
    }
  }
  output ??= { provenance, cases: [], errors: [], passed: 0, failed: 1, complete: false, unexpectedRequests };
  if (runError) {
    output.runnerError = redactDebuggerCapabilities(runError.stack);
    if (!cacheControl) output.startupStderr = redactDebuggerCapabilities(logs).slice(-8192);
  }
  output.cacheControl = cacheControl;
  output.startupDiagnostics = startupDiagnostics;
  if (phaseStartedAt !== undefined) recordPhase('finished');
  output.timeBudgets = { startupMs: startupBudgetMs, casesMs: caseBudgetMs };
  output.phaseTimeline = phaseTimeline;
  output.caseProgress = caseProgress;
  output.mediaGate = { held: Boolean(heldMedia), pending: pendingMedia, events: mediaGateEvents };
  output.cleanupErrors = cleanupErrors;
  output.cleanup = { browserExited: !browser?.pid || browser.exitCode !== null || browser.signalCode !== null,
    browserGroupStopped, profileRemoved: !profile || !existsSync(profile), serverClosed: !server.listening };
  if (runError || cleanupErrors.length || heldMedia || pendingMedia || !Object.values(output.cleanup).every(Boolean)) process.exitCode = 1;
  // Without FRAYD_DOM_RESULT_DIR, each run writes into its own new private temporary directory.
  const directory = resultDirectory || mkdtempSync(join(tmpdir(), 'frayd-dom-regression-'));
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, 'result.json'), JSON.stringify(output, null, 2) + '\n');
  console.log(JSON.stringify(output, null, 2));
}
module.exports = { main, stopBrowserGroup, withLocalPlyrIcons, EXPECTED_CASES };
if (require.main === module) main().catch(error => { console.error(error.stack); process.exitCode = 1; });
