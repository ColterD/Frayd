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
const evidenceDir = join(root, '.codex/evidence/pr2-real-dom');
const productionFiles = ['client/index.html', 'client/js/app.js', 'client/js/party.js',
  'client/vendor/plyr/plyr.min.js', 'client/vendor/plyr/plyr.min.css', 'client/vendor/plyr/plyr.svg', 'client/css/frayd.css'];
const harnessFiles = ['tests/browser-regression.cjs', 'tests/fixtures/browser-driver.js', 'tests/fixtures/reader-probe.js', 'tests/fixtures/native-resume.mp4', 'tests/fixtures/generate-native-resume.sh'];
function hash(path) { return createHash('sha256').update(readFileSync(path)).digest('hex'); }
/**
 * Resolve an existing BROWSER_BIN or PATH browser without installing or downloading
 * software. Version probing and launch failures remain visible to the caller.
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
 * Escalate TERM to KILL and confirm that the owned process group no longer exists.
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
  if (!await waitForExit(2000)) throw new Error(`Owned browser process group ${browser.pid} did not exit after SIGKILL; profile retained`);
}
/**
 * Remove DevTools listening lines and WebSocket capability URLs from diagnostics.
 * This targets debugger capabilities; it is not a general secret or log sanitizer.
 */
function redactDebuggerCapabilities(message) {
  return String(message).replace(/DevTools listening on [^\r\n]*/g, 'Owned DevTools endpoint redacted')
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
 * Spend one shared startup deadline discovering the owned blank page and configuring
 * its cache before navigation. Retry transient HTTP readiness only; unexpected targets
 * or debugger ownership fail closed. Keep the protocol open until caller teardown.
 */
async function configureOwnedBrowserCache({ profile, fixtureUrl, browser }) {
  const startedAt = Date.now();
  const deadline = startedAt + 15000;
  /**
   * Return time left on the single startup deadline; advancing to another phase or
   * retrying discovery never resets it. Reject before starting work after expiration.
   */
  function remainingStartupBudget() {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error('Owned browser startup deadline exceeded');
    return remaining;
  }
  const portFile = join(profile, 'DevToolsActivePort');
  let port;
  while (true) {
    if (browser.exitCode !== null || browser.signalCode !== null) throw new Error('Owned browser exited before cache setup');
    if (Date.now() >= deadline) throw new Error('Owned browser did not expose its cache-control endpoint');
    if (existsSync(portFile)) {
      const contents = readFileSync(portFile, 'utf8');
      const firstLine = contents.split('\n')[0].trim();
      port = contents.includes('\n') && /^\d+$/.test(firstLine) ? Number(firstLine) : NaN;
      if (Number.isInteger(port) && port >= 1 && port <= 65535) break;
    }
    await delay(20);
  }
  let pages;
  const browserExited = new AbortController();
  const onBrowserExit = () => browserExited.abort();
  browser.once('exit', onBrowserExit);
  try {
    while (true) {
      if (browser.exitCode !== null || browser.signalCode !== null) throw new Error('Owned browser exited before cache setup');
      const signal = AbortSignal.any([AbortSignal.timeout(remainingStartupBudget()), browserExited.signal]);
      let response;
      let targets;
      try {
        response = await fetch(`http://127.0.0.1:${port}/json/list`, { signal });
        if (response.ok) {
          targets = await bounded(response.json(), 'Owned browser page discovery body timed out', remainingStartupBudget());
        }
      } catch (error) {
        if (browser.exitCode !== null || browser.signalCode !== null) throw new Error('Owned browser exited before cache setup');
        remainingStartupBudget();
        if (!['TypeError', 'AbortError', 'TimeoutError'].includes(error.name)) throw error;
        await delay(20);
        continue;
      }
      remainingStartupBudget();
      if (!response.ok) {
        if (response.status === 408 || response.status === 429 || response.status >= 500) {
          await delay(20);
          continue;
        }
        throw new Error('Owned browser page discovery failed');
      }
      if (!Array.isArray(targets)) throw new Error('Invalid owned browser page discovery response');
      pages = targets.filter(page => page.type === 'page');
      if (pages.length === 0) {
        await delay(20);
        continue;
      }
      if (pages.length !== 1 || pages[0].url !== 'about:blank') throw new Error('Owned browser must have exactly one blank page before navigation');
      break;
    }
  } finally { browser.removeListener('exit', onBrowserExit); }
  const endpoint = new URL(pages[0].webSocketDebuggerUrl);
  if (endpoint.protocol !== 'ws:' || endpoint.hostname !== '127.0.0.1' || endpoint.port !== String(port)
    || !/^\/devtools\/page\/[a-zA-Z0-9-]+$/.test(endpoint.pathname)) throw new Error('Unexpected owned browser page endpoint');
  const openBudget = remainingStartupBudget();
  const socket = new WebSocket(endpoint.href);
  const pending = new Map();
  let sequence = 0;
  socket.addEventListener('message', event => {
    let message;
    try { message = JSON.parse(event.data); }
    catch { for (const waiter of pending.values()) waiter.reject(new Error('Invalid owned browser protocol response')); return; }
    const waiter = pending.get(message.id);
    if (!waiter) return;
    if (message.error) waiter.reject(new Error(`Owned browser cache RPC failed (${message.error.code})`));
    else waiter.resolve(message.result);
  });
  socket.addEventListener('error', () => {
    for (const waiter of pending.values()) waiter.reject(new Error('Owned browser cache protocol connection failed'));
  });
  /**
   * Send one bootstrap command on the owned protocol session and correlate its reply
   * by ID. Always remove the pending entry; failures propagate without command retry.
   */
  async function rpc(method, params = {}) {
    const rpcBudget = remainingStartupBudget();
    const id = ++sequence;
    try {
      const result = new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); });
      socket.send(JSON.stringify({ id, method, params }));
      return await bounded(result, `Owned browser cache RPC ${method} timed out`, rpcBudget);
    } finally { pending.delete(id); }
  }
  const receipt = { cacheDisabled: false, browserCacheCleared: false, configuredBeforeFixtureNavigation: false, protocolClosed: false };
  /**
   * Close only this owned protocol session after tests, keeping its cache overrides
   * active until then. Teardown has a separate three-second bound, outside startup.
   */
  async function close() {
    if (socket.readyState !== WebSocket.CLOSED) {
      const closed = new Promise(resolve => socket.addEventListener('close', resolve, { once: true }));
      socket.close();
      await bounded(closed, 'Owned browser cache protocol close timed out');
    }
    receipt.protocolClosed = true;
  }
  try {
    await bounded(new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', () => reject(new Error('Owned browser cache protocol failed to open')), { once: true });
    }), 'Owned browser cache protocol open timed out', openBudget);
    await rpc('Network.enable');
    await rpc('Network.setCacheDisabled', { cacheDisabled: true });
    await rpc('Network.clearBrowserCache');
    const navigation = await rpc('Page.navigate', { url: fixtureUrl });
    if (navigation.errorText) throw new Error('Owned browser fixture navigation failed');
    Object.assign(receipt, { cacheDisabled: true, browserCacheCleared: true, configuredBeforeFixtureNavigation: true });
    // Keep Network overrides active until the runner finishes all media cases.
    return { receipt, close };
  } catch (error) {
    await close();
    throw error;
  }
}
/**
 * Run the production client against an owned loopback fixture, process group and fresh
 * profile. Record original and cleanup failures separately; remove the profile only
 * after confirmed group shutdown, retaining it when shutdown cannot be established.
 */
async function main({ removeProfile = rmSync, evidenceDirectory = evidenceDir, cacheSetup = configureOwnedBrowserCache } = {}) {
  if (Number(process.versions.node.split('.')[0]) < 22 || typeof WebSocket !== 'function') {
    throw new Error('Browser regressions require Node.js 22+ with the global WebSocket API');
  }
  let browser;
  let profile;
  let timer;
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
  const cleanupErrors = [];
  let browserGroupStopped = false;
  let resolveResult;
  const result = new Promise(resolve => { resolveResult = resolve; });
  const unexpectedRequests = [];
  const nativeRequests = [];
  const mediaGateEvents = [];
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
      if (path === '/__fixture/hold-media' && request.method === 'POST') {
        const target = JSON.parse(input).path;
        if (!/^\/api\/stream\/(20|22)$/.test(target)) return json({ error: 'Unknown requested native stream' }, 400);
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
      if (path === '/api/progress/20' || path === '/api/progress/22') { nativeRequests.push({ path, status: 200, position: 64 }); return json({ position: 64 }); }
      if (path === '/api/progress/21') { nativeRequests.push({ path, status: 200, position: 0 }); return json({ position: 0 }); }
      if (path.startsWith('/api/progress/')) return json(null);
      if (path === '/api/progress') return json({});
      if (path === '/api/library') return json([]);
      if (/^\/api\/stream\/(20|21|22)$/.test(path)) {
        const gate = heldMedia;
        if (gate?.path === path) {
          gate.waited++;
          pendingMedia++;
          mediaGateEvents.push({ event: 'held-request', path });
          try { await gate.promise; }
          finally { pendingMedia--; }
          if (response.destroyed) return;
        }
        const media = readFileSync(join(root, 'tests/fixtures/native-resume.mp4'));
        const range = request.headers.range?.match(/^bytes=(\d+)-(\d*)$/);
        const start = range ? Number(range[1]) : 0;
        const end = range?.[2] ? Math.min(Number(range[2]), media.length - 1) : media.length - 1;
        if (start > end || start >= media.length) {
          response.writeHead(416, { 'Content-Range': `bytes */${media.length}` });
          return response.end();
        }
        nativeRequests.push({ path, status: range ? 206 : 200, start, end });
        response.writeHead(range ? 206 : 200, { 'Content-Type': 'video/mp4', 'Accept-Ranges': 'bytes', 'Cache-Control': 'no-store',
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
      if (path === '/') bytes = Buffer.from(bytes.toString().replace('<script src="js/app.js?v=5"></script>', '<script>Plyr.defaults.iconUrl="/vendor/plyr/plyr.svg";</script><script src="js/app.js?v=5"></script>').replace('</body>', '<script src="/__fixture/browser-driver.js"></script></body>'));
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
      repository: 'ColterD/Frayd',
      head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8', timeout: 10000 }).trim(),
      executable, browserVersion: execFileSync(executable, ['--version'], { encoding: 'utf8', timeout: 10000 }).trim(),
      executableSha256: hash(executable), nodeVersion: process.version,
      files: Object.fromEntries([...productionFiles, ...harnessFiles].map(file => [file, hash(join(root, file))])),
      browserClaim: 'Observed executable identity only; browser is supplied by BROWSER_BIN or PATH, not an immutable dependency.',
    };
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    profile = mkdtempSync(join(tmpdir(), 'frayd-browser-regression-'));
    const url = `http://127.0.0.1:${server.address().port}/`;
    const ownedCacheControl = cacheSetup === configureOwnedBrowserCache;
    browser = spawn(executable, ['--headless', '--no-sandbox', '--disable-gpu', '--disable-background-networking',
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
    const timedOut = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Browser regression exceeded 60 seconds; ${logs}`)), 60000); });
    const observed = await Promise.race([(async () => {
      cacheSession = await cacheSetup({ profile, fixtureUrl: url, browser });
      cacheControl = cacheSession.receipt;
      return result;
    })(), exited, timedOut]);
    const expectedCases = 32;
    const complete = observed.cases.length === expectedCases
      && new Set(observed.cases.map(item => item.name)).size === expectedCases
      && observed.cases.every(item => item.status === 'PASS')
      && observed.passed === expectedCases && observed.failed === 0;
    output = { provenance, ...observed, expectedCases, complete, unexpectedRequests, nativeRequests,
      fixtureConfiguration: { iconUrl: '/vendor/plyr/plyr.svg', mediaResponses: 'IDs9/10/11 HTTP204; IDs20/21/22 owned120s H264 MP4 with Range support',
        browserSandbox: 'Disabled only for this disposable headless process; EPUB iframe keeps sandbox=allow-scripts.' } };
    if (!complete || observed.failed || observed.errors.length || unexpectedRequests.length) process.exitCode = 1;
  } catch (error) {
    runError = error;
  } finally {
    clearTimeout(timer);
    releaseMedia('runner-cleanup');
    try { await cacheSession?.close(); }
    catch (error) { cleanupErrors.push(redactDebuggerCapabilities(error.stack)); }
    try { await stopBrowserGroup(browser); browserGroupStopped = true; }
    catch (error) { cleanupErrors.push(error.stack); }
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
  output.mediaGate = { held: Boolean(heldMedia), pending: pendingMedia, events: mediaGateEvents };
  output.cleanupErrors = cleanupErrors;
  output.cleanup = { browserExited: !browser?.pid || browser.exitCode !== null || browser.signalCode !== null,
    browserGroupStopped, profileRemoved: !profile || !existsSync(profile), serverClosed: !server.listening };
  if (runError || cleanupErrors.length || heldMedia || pendingMedia || !Object.values(output.cleanup).every(Boolean)) process.exitCode = 1;
  mkdirSync(evidenceDirectory, { recursive: true });
  writeFileSync(join(evidenceDirectory, 'result.json'), JSON.stringify(output, null, 2) + '\n');
  console.log(JSON.stringify(output, null, 2));
}
module.exports = { main, stopBrowserGroup };
if (require.main === module) main().catch(error => { console.error(error.stack); process.exitCode = 1; });
