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
const harnessFiles = ['tests/browser-regression.cjs', 'tests/fixtures/browser-driver.js', 'tests/fixtures/reader-probe.js'];
function hash(path) { return createHash('sha256').update(readFileSync(path)).digest('hex'); }
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
function signalGroup(pid, signal) {
  try { process.kill(-pid, signal); return true; }
  catch (error) { if (error.code === 'ESRCH') return false; throw error; }
}
async function stopBrowserGroup(browser) {
  if (!browser?.pid) return;
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
async function main({ removeProfile = rmSync, evidenceDirectory = evidenceDir } = {}) {
  let browser;
  let profile;
  let timer;
  let pendingAnnotation;
  const sockets = new Set();
  let body = '';
  let annotations = [];
  let posts = [];
  let logs = '';
  let output;
  let provenance;
  let runError;
  const cleanupErrors = [];
  let browserGroupStopped = false;
  let resolveResult;
  const result = new Promise(resolve => { resolveResult = resolve; });
  const unexpectedRequests = [];
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
      if (path === '/__fixture/configure' && request.method === 'POST') {
        if (pendingAnnotation) throw new Error('Previous annotation response still pending');
        const fixture = JSON.parse(input);
        body = fixture.body;
        annotations = fixture.annotations;
        posts = [];
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
      if (path === '/api/epub/7/chunk/0') {
        response.writeHead(200, { 'Content-Type': 'text/html' });
        return response.end(`<!doctype html><html><head><meta charset="utf-8"><script>${probe}</script></head><body>${body}</body></html>`);
      }
      if (path === '/api/annotations/7') return json(annotations);
      if (path.startsWith('/api/progress/')) return json(null);
      if (path === '/api/progress') return json({});
      if (path === '/api/library') return json([]);
      if (/^\/api\/stream\/(10|11)$/.test(path)) { response.writeHead(204); return response.end(); }
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
    browser = spawn(executable, ['--headless', '--no-sandbox', '--disable-gpu', '--disable-background-networking',
      '--disable-component-update', '--disable-sync', '--no-first-run', '--no-default-browser-check',
      '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1', '--disable-features=MediaRouter',
      `--user-data-dir=${profile}`, url], { detached: true, stdio: ['ignore', 'ignore', 'pipe'] });
    browser.stderr.on('data', bytes => { logs = (logs + bytes.toString()).slice(-12000); });
    const exited = new Promise((_, reject) => {
      browser.once('error', reject);
      browser.once('exit', (code, signal) => reject(new Error(`Browser exited before result: ${code}/${signal}; ${logs}`)));
    });
    const timedOut = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`Browser regression exceeded 60 seconds; ${logs}`)), 60000); });
    const observed = await Promise.race([result, exited, timedOut]);
    const expectedCases = 9;
    const complete = observed.cases.length === expectedCases
      && new Set(observed.cases.map(item => item.name)).size === expectedCases
      && observed.cases.every(item => item.status === 'PASS')
      && observed.passed === expectedCases && observed.failed === 0;
    output = { provenance, ...observed, expectedCases, complete, unexpectedRequests,
      fixtureConfiguration: { iconUrl: '/vendor/plyr/plyr.svg', mediaResponses: 'HTTP 204, no codec bytes',
        browserSandbox: 'Disabled only for this disposable headless process; EPUB iframe keeps sandbox=allow-scripts.' } };
    if (!complete || observed.failed || observed.errors.length || unexpectedRequests.length) process.exitCode = 1;
  } catch (error) {
    runError = error;
  } finally {
    clearTimeout(timer);
    try { await stopBrowserGroup(browser); browserGroupStopped = true; }
    catch (error) { cleanupErrors.push(error.stack); }
    try {
      if (pendingAnnotation) pendingAnnotation.destroy();
      for (const socket of sockets) socket.destroy();
      await new Promise(resolve => server.close(resolve));
    } catch (error) { cleanupErrors.push(error.stack); }
    if (profile && browserGroupStopped) {
      try { removeProfile(profile, { recursive: true, force: true }); }
      catch (error) { cleanupErrors.push(error.stack); }
    }
  }
  output ??= { provenance, cases: [], errors: [], passed: 0, failed: 1, complete: false, unexpectedRequests };
  if (runError) output.runnerError = runError.stack;
  output.cleanupErrors = cleanupErrors;
  output.cleanup = { browserExited: !browser?.pid || browser.exitCode !== null || browser.signalCode !== null,
    browserGroupStopped, profileRemoved: !profile || !existsSync(profile), serverClosed: !server.listening };
  if (runError || cleanupErrors.length || !Object.values(output.cleanup).every(Boolean)) process.exitCode = 1;
  mkdirSync(evidenceDirectory, { recursive: true });
  writeFileSync(join(evidenceDirectory, 'result.json'), JSON.stringify(output, null, 2) + '\n');
  console.log(JSON.stringify(output, null, 2));
}
module.exports = { main, stopBrowserGroup };
if (require.main === module) main().catch(error => { console.error(error.stack); process.exitCode = 1; });
