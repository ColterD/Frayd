'use strict';
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { createServer } = require('node:http');
const { mkdtempSync, readFileSync, existsSync, rmSync, writeFileSync, mkdirSync } = require('node:fs');
const { join, resolve, dirname } = require('node:path');
const { tmpdir } = require('node:os');
const { once, EventEmitter } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');
const { test } = require('node:test');
const vm = require('node:vm');
const { stopBrowserGroup } = require('./browser-regression.cjs');
const executable = join(__dirname, 'fixtures/browser-child-writer.cjs');

test('interception diagnostics preserve fatal errors and safely correlate retired frames without copying payloads', async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const setup = source.slice(source.indexOf('async function bounded('), source.indexOf('async function main('));
  for (const scenario of ['active', 'canceled', 'wrong-network', 'canceled-generic', 'canceled-other-code',
    'canceled-nonboolean', 'blocked-canceled', 'blocked-unrecognized']) {
    const cancellationEvent = scenario !== 'active' && scenario !== 'blocked-unrecognized';
    const exactCancellation = scenario === 'canceled' || scenario === 'blocked-canceled';
    class ProtocolSocket extends EventTarget {
      static CLOSED = 3;
      constructor() {
        super(); this.readyState = 1;
        queueMicrotask(() => this.dispatchEvent(new Event('open')));
      }
      receive(value) { this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(value) })); }
      send(bytes) {
        const command = JSON.parse(bytes);
        queueMicrotask(() => {
          if (command.method.startsWith('Fetch.') && command.method !== 'Fetch.enable') {
            if (cancellationEvent) {
              this.receive({ method: 'Network.loadingFailed', params: { requestId: scenario === 'wrong-network' ? 'different-network' : 'owned-network',
                canceled: scenario === 'canceled-nonboolean' ? 'true' : true } });
              this.receive({ method: 'Page.frameDetached', params: { frameId: 'owned-frame' } });
            }
            this.receive({ id: command.id, error: { code: scenario === 'canceled-other-code' ? -32000 : -32602,
              message: scenario === 'blocked-unrecognized' ? 'diagnostic-payload-must-not-leak'
                : scenario === 'canceled-generic' ? 'Invalid parameters' : 'Invalid InterceptionId.',
              data: 'diagnostic-payload-must-not-leak' } });
            return;
          }
          this.receive({ id: command.id, result: {} });
          if (command.method === 'Page.navigate') {
            this.receive({ method: 'Page.frameNavigated', params: { frame: { id: 'owned-frame', loaderId: 'owned-loader' } } });
            this.receive({ method: 'Fetch.requestPaused', params: { requestId: 'owned-interception', networkId: 'owned-network',
              frameId: 'owned-frame', resourceType: 'Image', request: { url: scenario.startsWith('blocked-')
                ? 'https://blocked.invalid/asset' : 'http://127.0.0.1:8080/owned' } } });
          }
        });
      }
      close() { this.readyState = 3; this.dispatchEvent(new Event('close')); }
    }
    const configure = vm.runInNewContext(setup + '\nconfigureOwnedBrowserCache;', {
      join, existsSync: () => true, readFileSync: () => '32123\n/unused',
      WebSocket: ProtocolSocket, AbortSignal, AbortController, URL, setTimeout, clearTimeout, delay,
      fetch: async () => ({ ok: true, json: async () => [{ type: 'page', url: 'about:blank',
        webSocketDebuggerUrl: 'ws://127.0.0.1:32123/devtools/page/owned-protocol' }] }),
    });
    const browser = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
    const session = await configure({ profile: '/owned/profile', fixtureUrl: 'http://127.0.0.1:8080/', browser });
    try {
      await delay(0);
      assert.equal(session.receipt.protocolErrors.length, exactCancellation ? 0 : 1, 'Only exact correlated cancellation retires; every other error stays fatal');
      assert.equal(session.receipt.canceledInterceptions.length, exactCancellation ? 1 : 0);
      const error = (exactCancellation ? session.receipt.canceledInterceptions : session.receipt.protocolErrors)[0];
      assert.equal(error.method, scenario.startsWith('blocked-') ? 'Fetch.failRequest' : 'Fetch.continueRequest');
      assert.equal(error.code, scenario === 'canceled-other-code' ? -32000 : -32602);
      assert.equal(error.message, scenario === 'blocked-unrecognized' ? 'unrecognized CDP message'
        : scenario === 'canceled-generic' ? 'Invalid parameters' : 'Invalid InterceptionId.');
      assert.equal(error.lifecycle.matchingNetworkCanceled, cancellationEvent && scenario !== 'wrong-network' && scenario !== 'canceled-nonboolean');
      assert.equal(error.lifecycle.frameDetached, cancellationEvent);
      assert.doesNotMatch(JSON.stringify(error), /diagnostic-payload-must-not-leak/);
    } finally { await session.close(); }
  }
});

test('PID-only cleanup leaves a profile writer; owned process-group cleanup stops it before removal', { timeout: 8000 }, async () => {
  const ownedParent = mkdtempSync(join(tmpdir(), 'frayd-cleanup-oracle-'));
  const profile = join(ownedParent, 'frayd=ci/profile');
  mkdirSync(profile, { recursive: true });
  let resolveReady;
  const ready = new Promise(resolve => { resolveReady = resolve; });
  const server = createServer(async (request, response) => {
    let input = '';
    for await (const chunk of request) input += chunk;
    response.end('{}');
    resolveReady(JSON.parse(input));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = spawn(executable, [`--user-data-dir=${profile}`, `http://127.0.0.1:${server.address().port}/`],
    { detached: true, stdio: 'ignore' });
  let readyTimer;
  try {
    const reported = await Promise.race([ready, new Promise((_, reject) => {
      readyTimer = setTimeout(() => reject(new Error('Child-writer fixture did not become ready within 2 seconds')), 2000);
    })]);
    clearTimeout(readyTimer);
    assert.equal(reported.profile, profile, 'Launcher preserves the complete owned profile path');
    const stopped = once(browser, 'exit');
    browser.kill('SIGKILL'); // Original runner behavior: only the launcher exits.
    await stopped;
    const writerFile = join(profile, 'Default/writer');
    const before = readFileSync(writerFile, 'utf8');
    await delay(40);
    assert.notEqual(readFileSync(writerFile, 'utf8'), before, 'Child continues writing after launcher exits');
    try { rmSync(profile, { recursive: true, force: true }); }
    catch (error) { if (error.code !== 'ENOTEMPTY') throw error; }
    await delay(40);
    assert.equal(existsSync(writerFile), true, 'Surviving child recreates removed profile');
    await stopBrowserGroup(browser);
    rmSync(profile, { recursive: true, force: true });
    await delay(40);
    assert.equal(existsSync(profile), false, 'No child recreates profile after group cleanup');
    assert.throws(() => process.kill(-browser.pid, 0), { code: 'ESRCH' });
  } finally {
    clearTimeout(readyTimer);
    await stopBrowserGroup(browser);
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
    rmSync(ownedParent, { recursive: true, force: true });
  }
});

test('original browser failure remains visible alongside an injected profile-removal failure', { timeout: 8000 }, async () => {
  const runner = resolve(__dirname, 'browser-regression.cjs');
  const resultDirectory = mkdtempSync(join(tmpdir(), 'frayd-cleanup-result-'));
  let retainedProfile;
  const invocation = spawn(process.execPath, ['-e', `
    require(${JSON.stringify(runner)}).main({ evidenceDirectory: ${JSON.stringify(resultDirectory)}, cacheSetup: async () => {
      const receipt = { injectedTestDouble: true, protocolClosed: false };
      return { receipt, close: async () => { receipt.protocolClosed = true; } };
    }, removeProfile(profile) {
      throw new Error('cleanup sentinel: ' + profile);
    }}).catch(error => { console.error(error.stack); process.exitCode = 1; });
  `], { env: { ...process.env, BROWSER_BIN: executable, FRAYD_CHILD_FIXTURE_EARLY_EXIT: '1' }, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  let stderr = '';
  invocation.stdout.on('data', bytes => { stdout += bytes; });
  invocation.stderr.on('data', bytes => { stderr += bytes; });
  try {
    const [code] = await once(invocation, 'exit');
    assert.equal(code, 1, stderr);
    const result = JSON.parse(stdout);
    assert.match(result.runnerError, /Browser exited before result: 7\/null; original browser exit sentinel/);
    assert.equal(result.cleanupErrors.length, 1);
    assert.equal(result.cacheControl.protocolClosed, true, 'Confirmed group shutdown closes the owned session');
    assert.match(result.cleanupErrors[0], /cleanup sentinel: /);
    retainedProfile = result.cleanupErrors[0].match(/cleanup sentinel: ([^\n]+)/)[1];
    assert.equal(dirname(retainedProfile), tmpdir());
    assert.match(retainedProfile, /frayd-browser-regression-[^/]+$/);
    assert.deepEqual(result.cleanup, { browserExited: true, browserGroupStopped: true, profileRemoved: false, serverClosed: true });
    assert.equal(result.complete, false);
    assert.equal(existsSync(retainedProfile), true, 'Failed cleanup retains the path for diagnosis');
  } finally {
    rmSync(resultDirectory, { recursive: true, force: true });
    if (retainedProfile) rmSync(retainedProfile, { recursive: true, force: true });
    if (invocation.exitCode === null) invocation.kill('SIGKILL');
  }
});


test('runner cleanup releases an active native stream gate after browser abort', { timeout: 8000 }, async () => {
  const fixtureDirectory = mkdtempSync(join(tmpdir(), 'frayd-native-gate-abort-'));
  const browserScript = join(fixtureDirectory, 'browser.cjs');
  const resultDirectory = join(fixtureDirectory, 'result');
  const script = `#!/usr/bin/env node
if (process.argv.includes('--version')) { console.log('Owned native-gate abort fixture'); process.exit(0); }
(async () => {
  const base = process.argv.at(-1);
  await fetch(new URL('/__fixture/hold-media', base), {
    method: 'POST', body: JSON.stringify({ path: '/api/stream/20' })
  });
  fetch(new URL('/api/stream/20', base)).catch(() => {});
  const deadline = Date.now() + 2000;
  while (true) {
    const state = await (await fetch(new URL('/__fixture/media-gate', base))).json();
    if (state.pending === 1) break;
    if (Date.now() >= deadline) throw new Error('Native gate never held the requested bytes');
  }
  // Nonfunctional marker exercises redaction, never an actual debugger capability.
  console.error('DevTools listening on ws://127.0.0.1:1/devtools/browser/fixture-only-debugger-marker');
  console.error('native held-stream abort sentinel');
  process.exit(7);
})().catch(error => { console.error(error); process.exit(9); });
`;
  writeFileSync(browserScript, script, { mode: 0o700 });
  const runner = resolve(__dirname, 'browser-regression.cjs');
  const invocation = spawn(process.execPath, ['-e', `
    require(${JSON.stringify(runner)}).main({ evidenceDirectory: ${JSON.stringify(resultDirectory)}, cacheSetup: async () => ({ receipt: { injectedTestDouble: true }, close: async () => {} }) })
      .catch(error => { console.error(error.stack); process.exitCode = 1; });
  `], { env: { ...process.env, BROWSER_BIN: browserScript }, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  let stderr = '';
  invocation.stdout.on('data', bytes => { stdout += bytes; });
  invocation.stderr.on('data', bytes => { stderr += bytes; });
  try {
    const [code] = await once(invocation, 'exit');
    assert.equal(code, 1, stderr);
    const result = JSON.parse(stdout);
    assert.match(result.runnerError, /Browser exited before result: 7/);
    assert.match(result.runnerError, /native held-stream abort sentinel/);
    assert.match(result.runnerError, /Owned DevTools endpoint redacted/);
    assert.doesNotMatch(stdout, /fixture-only-debugger-marker/);
    assert.equal(result.mediaGate.held, false);
    assert.equal(result.mediaGate.pending, 0);
    assert.deepEqual(result.mediaGate.events, [
      { event: 'hold', path: '/api/stream/20' },
      { event: 'held-request', path: '/api/stream/20' },
      { event: 'release', path: '/api/stream/20', waited: 1, reason: 'runner-cleanup' },
    ]);
    assert.equal(Object.values(result.cleanup).every(Boolean), true);
    assert.deepEqual(result.cleanupErrors, []);
  } finally {
    if (invocation.exitCode === null) invocation.kill('SIGKILL');
    rmSync(fixtureDirectory, { recursive: true, force: true });
  }
});


test('native driver releases held bytes when retired metadata never arrives', { timeout: 8000 }, async () => {
  const driver = readFileSync(join(__dirname, 'fixtures/browser-driver.js'), 'utf8');
  const untilStart = driver.indexOf('  async function until(');
  const untilEnd = driver.indexOf('  async function api(', untilStart);
  const nativeStart = driver.indexOf('  async function nativeResume(');
  const nativeEnd = driver.indexOf("  await run('native decoded plain movie", nativeStart);
  assert.ok(untilStart >= 0 && untilEnd > untilStart && nativeStart >= 0 && nativeEnd > nativeStart);
  let held = false;
  let releases = 0;
  const media = { addEventListener() {}, removeEventListener() {} };
  const plyr = { media, config: { blankVideo: 'original' }, on() {}, off() {} };
  const context = {
    plyr, delay, nativeMedia: [], HTMLVideoElement: class {},
    check: (value, message) => assert.ok(value, message),
    document: {
      createElement: () => ({ canPlayType: () => 'probably' }),
      addEventListener() {}, removeEventListener() {}, getElementById: () => ({ click() {} }),
    },
    openVideo: () => { plyr.media = {}; },
    async api(action) {
      if (action === 'hold-media') { held = true; return {}; }
      if (action === 'media-gate') return { held, pending: 1 };
      if (action === 'release-media') { releases++; held = false; return {}; }
      throw new Error('Unexpected native fixture action');
    },
  };
  // Run the actual driver's bounded wait and finally block; no native metadata fires.
  const nativeResume = vm.runInNewContext(driver.slice(untilStart, untilEnd)
    + driver.slice(nativeStart, nativeEnd) + '\nnativeResume;', context);
  await assert.rejects(nativeResume('missing retired metadata', true),
    /Timed out: retired cancellation blank metadata before requested movie release/);
  assert.equal(held, false);
  assert.equal(releases, 1);
  assert.equal(plyr.config.blankVideo, 'original');
});


test('cache startup failure retains its cause and bounded redacted stderr', { timeout: 20000 }, async () => {
  const fixtureDirectory = mkdtempSync(join(tmpdir(), 'frayd-cache-startup-failure-'));
  const browserScript = join(fixtureDirectory, 'browser.cjs');
  const resultDirectory = join(fixtureDirectory, 'result');
  writeFileSync(browserScript, `#!/usr/bin/env node
if (process.argv.includes('--version')) { console.log('Owned startup stderr fixture'); process.exit(0); }
console.error('x'.repeat(9000));
// A nonfunctional marker, never an actual browser debugging capability.
console.error('DevTools listening on ws://127.0.0.1:1/devtools/browser/owned-startup-debugger-marker');
console.error('harmless owned startup diagnostic');
setInterval(() => {}, 1000);
`, { mode: 0o700 });
  const runner = resolve(__dirname, 'browser-regression.cjs');
  const invocation = spawn(process.execPath, ['-e', `
    require(${JSON.stringify(runner)}).main({ evidenceDirectory: ${JSON.stringify(resultDirectory)}, startupBudgetMs: 200 })
      .catch(error => { console.error(error.stack); process.exitCode = 1; });
  `], { env: { ...process.env, BROWSER_BIN: browserScript }, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  let stderr = '';
  invocation.stdout.on('data', bytes => { stdout += bytes; });
  invocation.stderr.on('data', bytes => { stderr += bytes; });
  try {
    const [code] = await once(invocation, 'exit');
    assert.equal(code, 1, stderr);
    const result = JSON.parse(stdout);
    assert.match(result.runnerError, /Owned browser did not expose its cache-control endpoint|Browser startup exceeded 200 milliseconds/);
    assert.equal(result.cases.length, 0);
    assert.equal(result.complete, false);
    assert.equal(result.cacheControl, undefined);
    assert.equal(typeof result.startupStderr, 'string');
    assert.ok(result.startupStderr.length <= 8192);
    assert.match(result.startupStderr, /harmless owned startup diagnostic/);
    assert.match(result.startupStderr, /Owned DevTools endpoint redacted/);
    assert.doesNotMatch(stdout, /owned-startup-debugger-marker|ws:\/\/127/);
    assert.equal(Object.values(result.cleanup).every(Boolean), true);
    assert.deepEqual(result.cleanupErrors, []);
  } finally {
    if (invocation.exitCode === null) invocation.kill('SIGKILL');
    rmSync(fixtureDirectory, { recursive: true, force: true });
  }
});


test('cache discovery waits for a complete valid port and stops on exit or deadline', async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const start = source.indexOf('async function configureOwnedBrowserCache(');
  const end = source.indexOf('async function main(', start);
  for (const scenario of ['complete', 'exit', 'deadline']) {
    let now = 0;
    let reads = 0;
    const browser = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
    const fetched = [];
    const context = {
      Date: { now: () => now }, join, AbortSignal, AbortController,
      existsSync: () => true,
      readFileSync: () => ['', '12', '12partial\n/unused', '70000\n/unused', '32123\n/unused'][Math.min(reads++, 4)],
      async delay() {
        now += scenario === 'deadline' ? 30000 : 20;
        if (scenario === 'exit') browser.exitCode = 7;
      },
      async fetch(url) { fetched.push(url); throw new Error('valid discovery reached'); },
    };
    const configure = vm.runInNewContext(source.slice(start, end) + '\nconfigureOwnedBrowserCache;', context);
    await assert.rejects(configure({ profile: '/owned/profile', fixtureUrl: 'unused', browser }),
      scenario === 'complete' ? /valid discovery reached/ : scenario === 'exit' ? /exited before cache setup/ : /did not expose/);
    assert.deepEqual(fetched, scenario === 'complete' ? ['http://127.0.0.1:32123/json/list'] : []);
  }
});

test('browser runner rejects unsupported Node and missing global WebSocket before starting resources', async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const start = source.indexOf('async function main(');
  const end = source.indexOf('module.exports =', start);
  for (const scenario of [{ node: '20.0.0', WebSocket: class {} }, { node: '22.0.0', WebSocket: undefined }]) {
    const main = vm.runInNewContext(source.slice(start, end) + '\nmain;', {
      process: { versions: { node: scenario.node } }, WebSocket: scenario.WebSocket,
      rmSync() {}, evidenceDir: 'unused', configureOwnedBrowserCache() {},
      createServer() { throw new Error('Unsupported runtime started fixture resources'); },
    });
    await assert.rejects(main(), /require Node.js 22\+ with the global WebSocket API/);
  }
});

test('browser setup shares its remaining startup budget across port and HTTP discovery', async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const start = source.indexOf('async function bounded(');
  const end = source.indexOf('async function main(', start);
  for (const expireAfterHttp of [false, true]) {
    let now = 0;
    let portReady = false;
    let httpBudget;
    const context = {
      Date: { now: () => now }, join, URL, AbortController,
      existsSync: () => portReady,
      readFileSync: () => '32123\n/unused',
      async delay() { now += 4000; portReady = true; },
      AbortSignal: { timeout(milliseconds) { httpBudget = milliseconds; return {}; }, any() { return {}; } },
      async fetch() {
        if (!expireAfterHttp) throw new Error('HTTP boundary reached');
        now = 30001;
        return { ok: true, json: async () => [{ type: 'page', url: 'about:blank', webSocketDebuggerUrl: 'ws://127.0.0.1:32123/devtools/page/owned-test' }] };
      },
      WebSocket: class { constructor() { throw new Error('Expired startup opened a protocol connection'); } },
    };
    const configure = vm.runInNewContext(source.slice(start, end) + '\nconfigureOwnedBrowserCache;', context);
    await assert.rejects(configure({ profile: '/owned/profile', fixtureUrl: 'unused', browser: Object.assign(new EventEmitter(), { exitCode: null, signalCode: null }) }),
      expireAfterHttp ? /startup deadline exceeded/ : /HTTP boundary reached/);
    assert.equal(httpBudget, 1000, 'One HTTP attempt is bounded within the remaining shared startup budget');
  }
});


test('page discovery retries temporary HTTP failures and empty lists but rejects unsafe targets', async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const setupSource = source.slice(source.indexOf('async function bounded('), source.indexOf('async function main('));
  for (const scenario of ['transient', 'multiple', 'nonblank', 'foreign']) {
    let calls = 0;
    const page = { type: 'page', url: 'about:blank', webSocketDebuggerUrl: 'ws://127.0.0.1:32123/devtools/page/owned-test' };
    const browser = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
    const configure = vm.runInNewContext(setupSource + '\nconfigureOwnedBrowserCache;', {
      Date, join, URL, AbortSignal, AbortController, setTimeout, clearTimeout, delay,
      existsSync: () => true, readFileSync: () => '32123\n/unused',
      async fetch() {
        calls++;
        if (scenario === 'transient' && calls === 1) throw new TypeError('fetch failed');
        if (scenario === 'transient' && calls === 2) return { ok: false, status: 503 };
        const pages = scenario === 'transient' && calls === 3 ? []
          : scenario === 'multiple' ? [page, page]
          : [{ ...page, ...(scenario === 'nonblank' ? { url: 'https://example.invalid' }
            : scenario === 'foreign' ? { webSocketDebuggerUrl: 'ws://example.invalid/devtools/page/owned-test' } : {}) }];
        return { ok: true, json: async () => pages };
      },
      WebSocket: class { constructor() { throw new Error('Validated owned protocol boundary reached'); } },
    });
    await assert.rejects(configure({ profile: '/owned/profile', fixtureUrl: 'unused', browser }),
      scenario === 'transient' ? /Validated owned protocol boundary reached/ : scenario === 'foreign' ? /Unexpected owned browser page endpoint/ : /exactly one blank page/);
    assert.equal(calls, scenario === 'transient' ? 4 : 1);
    assert.equal(browser.listenerCount('exit'), 0, 'Discovery releases its exit listener');
  }
});

test('unresponsive owned discovery stops at the shared deadline or browser exit and cleans up', { timeout: 20000 }, async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const setupSource = source.slice(source.indexOf('async function bounded('), source.indexOf('async function main('));
  for (const scenario of ['deadline', 'exit']) {
    const profile = mkdtempSync(join(tmpdir(), 'frayd-discovery-never-'));
    const browser = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
    let requested = false;
    let exitTimer;
    const server = createServer((request, response) => {
      assert.equal(request.url, '/json/list');
      requested = true; // Keep the actual HTTP response open until discovery aborts.
      if (scenario === 'exit') exitTimer = setTimeout(() => {
        browser.exitCode = 7;
        browser.emit('exit', 7, null);
      }, 20);
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    writeFileSync(join(profile, 'DevToolsActivePort'), `${server.address().port}\n/unused`);
    const configure = vm.runInNewContext(setupSource + '\nconfigureOwnedBrowserCache;', {
      Date, join, URL, AbortSignal, AbortController, setTimeout, clearTimeout, delay,
      existsSync, readFileSync, fetch,
      WebSocket: class { constructor() { throw new Error('Unresponsive discovery opened a protocol connection'); } },
    });
    const startedAt = Date.now();
    try {
      await assert.rejects(configure({ profile, fixtureUrl: 'unused', browser, budgetMs: 200 }),
        scenario === 'deadline' ? /startup deadline exceeded/ : /exited before cache setup/);
      const elapsed = Date.now() - startedAt;
      assert.equal(requested, true, 'The real owned HTTP endpoint received discovery');
      if (scenario === 'deadline') assert.ok(elapsed >= 180 && elapsed < 1500, `Shared deadline elapsed ${elapsed}ms`);
      else assert.ok(elapsed < 3000, `Browser exit must abort the pending request: ${elapsed}ms`);
      assert.equal(browser.listenerCount('exit'), 0);
    } finally {
      clearTimeout(exitTimer);
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
      rmSync(profile, { recursive: true, force: true });
    }
    assert.equal(existsSync(profile), false);
    assert.equal(server.listening, false);
  }
});


test('owned discovery retries a stalled HTTP attempt within the shared startup budget', { timeout: 8000 }, async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const setupSource = source.slice(source.indexOf('async function bounded('), source.indexOf('async function main('));
  for (const scenario of ['headers', 'body']) {
    let attempts = 0;
    const browser = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
    const server = createServer((request, response) => {
      assert.equal(request.url, '/json/list');
      attempts++;
      if (attempts === 1) {
        if (scenario === 'body') { response.writeHead(200, { 'Content-Type': 'application/json' }); response.write('['); }
        return;
      }
      response.end(JSON.stringify([{ type: 'page', url: 'about:blank',
        webSocketDebuggerUrl: `ws://127.0.0.1:${server.address().port}/devtools/page/owned-test` }]));
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const configure = vm.runInNewContext(setupSource + '\nconfigureOwnedBrowserCache;', {
      Date, join, URL, AbortSignal, AbortController, setTimeout, clearTimeout, delay, fetch,
      existsSync: () => true, readFileSync: () => `${server.address().port}\n/unused`,
      WebSocket: class { constructor() { throw new Error('Validated owned protocol boundary reached'); } },
    });
    const diagnostics = {};
    // Bound the baseline failure too: release its stuck response without leaving sockets.
    const failSafe = setTimeout(() => server.closeAllConnections(), 2500);
    const startedAt = Date.now();
    try {
      await assert.rejects(configure({ profile: '/owned/profile', fixtureUrl: 'unused', browser, diagnostics }),
        /Validated owned protocol boundary reached/);
      assert.equal(attempts, 2);
      assert.equal(diagnostics.attempts, 2);
      assert.equal(diagnostics.transientFailures, 1);
      assert.equal(diagnostics.budgetMs, 30000);
      assert.equal(diagnostics.phase, 'socket-open');
      assert.doesNotMatch(JSON.stringify(diagnostics), /127\.0\.0\.1|devtools|unused/);
      assert.ok(Date.now() - startedAt < 2000, 'A stalled attempt must leave time for subsequent readiness probes');
      assert.equal(browser.listenerCount('exit'), 0);
    } finally {
      clearTimeout(failSafe);
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
    }
  }
});


test('retry fixture HTTP renders only trusted mode literals and rejects hostile input without reflection', { timeout: 8000 }, async () => {
  const fixtureDirectory = mkdtempSync(join(tmpdir(), 'frayd-mode-input-'));
  const browserScript = join(fixtureDirectory, 'browser.cjs');
  writeFileSync(browserScript, `#!/usr/bin/env node
if (process.argv.includes('--version')) { console.log('Owned idle input fixture'); process.exit(0); }
setInterval(() => {}, 1000);
`, { mode: 0o700 });
  const runner = resolve(__dirname, 'browser-regression.cjs');
  const invocation = spawn(process.execPath, ['-e', `
    const assert = require('node:assert/strict');
    require(${JSON.stringify(runner)}).main({ evidenceDirectory: ${JSON.stringify(join(fixtureDirectory, 'result'))},
      async cacheSetup({ fixtureUrl }) {
        const controls = [
          ['responsive?width=320', 200, 'null'],
          ['responsive?width=320&mode=ignored', 200, 'null'],
          ['toolbar-retry?width=768', 200, 'null'],
          ['toolbar-retry?width=1024&mode=transient', 200, '"transient"'],
          ['toolbar-retry?width=1440&mode=reattach', 200, '"reattach"'],
          ['toolbar-retry?width=320&mode=other', 400],
          ['toolbar-retry?width=320&mode=', 400],
          ['toolbar-retry?width=320&mode=__proto__', 400],
          ['toolbar-retry?width=320&mode=constructor', 400],
          ['toolbar-retry?width=319&mode=transient', 400],
          ['toolbar-retry?width=320&mode=' + encodeURIComponent('</script><script>globalThis.hostileInput=true</script>'), 400],
        ];
        for (const [route, status, literal] of controls) {
          const response = await fetch(fixtureUrl + '__fixture/' + route);
          const body = await response.text();
          assert.equal(response.status, status, route);
          if (status === 200) assert.ok(body.includes('window.__toolbarRetryMode=' + literal + '</script>'), route);
          else assert.equal(body, '{}', route);
          assert.ok(!body.includes('globalThis.hostileInput'), route);
        }
        console.error('Verified 11 real HTTP fixture input controls');
        throw new Error('Owned fixture input verification complete');
      }
    });
  `], { env: { ...process.env, BROWSER_BIN: browserScript }, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  let stderr = '';
  invocation.stdout.on('data', bytes => { stdout += bytes; });
  invocation.stderr.on('data', bytes => { stderr += bytes; });
  try {
    const [code] = await once(invocation, 'exit');
    assert.equal(code, 1, stderr);
    assert.match(stderr, /Verified 11 real HTTP fixture input controls/);
    const result = JSON.parse(stdout);
    assert.match(result.runnerError, /Owned fixture input verification complete/);
    assert.equal(result.cases.length, 0);
    assert.equal(Object.values(result.cleanup).every(Boolean), true);
    assert.deepEqual(result.cleanupErrors, []);
  } finally {
    if (invocation.exitCode === null) invocation.kill('SIGKILL');
    rmSync(fixtureDirectory, { recursive: true, force: true });
  }
});


test('startup and cases have separate finite budgets and preserve partial progress on failure', { timeout: 8000 }, async () => {
  const fixtureDirectory = mkdtempSync(join(tmpdir(), 'frayd-phase-budgets-'));
  const browserScript = join(fixtureDirectory, 'browser.cjs');
  writeFileSync(browserScript, `#!/usr/bin/env node
if (process.argv.includes('--version')) { console.log('Owned phase-budget fixture'); process.exit(0); }
setInterval(() => {}, 1000);
`, { mode: 0o700 });
  const runner = resolve(__dirname, 'browser-regression.cjs');
  try {
    for (const scenario of ['slow-success', 'startup-timeout', 'case-timeout']) {
      const invocation = spawn(process.execPath, ['-e', `
        require(${JSON.stringify(runner)}).main({ evidenceDirectory: ${JSON.stringify(fixtureDirectory)},
          startupBudgetMs: 600, caseBudgetMs: 900,
          async cacheSetup({ fixtureUrl }) {
            if (${JSON.stringify(scenario)} === 'startup-timeout') return new Promise(() => {});
            await new Promise(resolve => setTimeout(resolve, 400));
            await fetch(fixtureUrl + '__fixture/progress', { method: 'POST', body: JSON.stringify({
              completed: 1, passed: 1, failed: 0, blocked: 0, currentCase: 2 }) });
            if (${JSON.stringify(scenario)} === 'slow-success') setTimeout(() => {
              fetch(fixtureUrl + '__fixture/result', { method: 'POST', body: JSON.stringify({
                cases: Array.from({ length: 49 }, (_, n) => ({ name: 'controlled ' + n, status: 'PASS' })),
                passed: 49, failed: 0, errors: [] }) }).catch(() => {});
            }, 650);
            return { receipt: { injectedTestDouble: true }, close: async () => {} };
          }
        }).catch(error => { console.error(error.stack); process.exitCode = 1; });
      `], { env: { ...process.env, BROWSER_BIN: browserScript }, stdio: ['ignore', 'pipe', 'pipe'] });
      let stdout = '', stderr = '';
      invocation.stdout.on('data', bytes => { stdout += bytes; });
      invocation.stderr.on('data', bytes => { stderr += bytes; });
      const [code] = await once(invocation, 'exit');
      const result = JSON.parse(stdout);
      assert.equal(code, scenario === 'slow-success' ? 0 : 1, stderr);
      assert.equal(result.complete, scenario === 'slow-success');
      assert.deepEqual(result.timeBudgets, { startupMs: 600, casesMs: 900 });
      assert.deepEqual(result.phaseTimeline.map(item => item.phase), scenario === 'startup-timeout'
        ? ['startup', 'cleanup', 'finished'] : ['startup', 'cases', 'cleanup', 'finished']);
      assert.equal(Object.values(result.cleanup).every(Boolean), true);
      assert.deepEqual(result.cleanupErrors, []);
      if (scenario === 'slow-success') {
        assert.ok(result.phaseTimeline[2].elapsedMs > 900, 'Combined work exceeds the case budget yet passes');
        assert.equal(result.cases.length, 49);
      } else {
        assert.match(result.runnerError, scenario === 'startup-timeout' ? /startup exceeded 600 milliseconds/ : /cases exceeded 900 milliseconds/);
        assert.equal(result.cases.length, 0, 'Progress never substitutes for terminal case evidence');
        assert.equal(result.caseProgress.completed, scenario === 'case-timeout' ? 1 : 0);
        assert.equal(result.caseProgress.currentCase, scenario === 'case-timeout' ? 2 : null);
      }
    }
  } finally { rmSync(fixtureDirectory, { recursive: true, force: true }); }
});


test('transient discovery response bodies are canceled before another attempt', { timeout: 8000 }, async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const setup = source.slice(source.indexOf('async function bounded('), source.indexOf('async function main('));
  for (const status of [408, 429, 503]) {
    let attempts = 0;
    let canceled = false;
    let responseClosed;
    const closed = new Promise(resolve => { responseClosed = resolve; });
    const server = createServer((request, response) => {
      assert.equal(request.url, '/json/list');
      if (++attempts === 1) {
        response.on('close', responseClosed);
        response.writeHead(status);
        response.write('unfinished readiness response');
        return;
      }
      response.end(JSON.stringify([{ type: 'page', url: 'about:blank',
        webSocketDebuggerUrl: `ws://127.0.0.1:${server.address().port}/devtools/page/owned-test` }]));
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const configure = vm.runInNewContext(setup + '\nconfigureOwnedBrowserCache;', {
      Date, join, URL, AbortSignal, AbortController, setTimeout, clearTimeout, delay,
      existsSync: () => true, readFileSync: () => `${server.address().port}\n/unused`,
      async fetch(...args) {
        if (attempts) assert.equal(canceled, true, 'Previous body must be canceled before retry');
        const response = await fetch(...args);
        if (!response.ok) {
          const cancel = response.body.cancel.bind(response.body);
          response.body.cancel = async () => { await cancel(); canceled = true; };
        }
        return response;
      },
      WebSocket: class { constructor() { throw new Error('Validated owned protocol boundary reached'); } },
    });
    const browser = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
    try {
      await assert.rejects(configure({ profile: '/owned/profile', fixtureUrl: 'unused', browser, budgetMs: 1000 }),
        /Validated owned protocol boundary reached/);
      await Promise.race([closed, delay(1000).then(() => { throw new Error('Canceled response remained open'); })]);
      assert.equal(attempts, 2);
      assert.equal(canceled, true);
      assert.equal(browser.listenerCount('exit'), 0);
    } finally {
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
    }
  }
});

test('unconfirmed browser shutdown retains its active interception session and profile', { timeout: 8000 }, async () => {
  const runner = resolve(__dirname, 'browser-regression.cjs');
  const resultDirectory = mkdtempSync(join(tmpdir(), 'frayd-shutdown-held-'));
  const invocation = spawn(process.execPath, ['-e', `
    const assert = require('node:assert/strict');
    const fs = require('node:fs');
    const source = fs.readFileSync(${JSON.stringify(runner)}, 'utf8').replace(/^#!.*\\n/, '');
    const ownedModule = { exports: {} };
    const controls = new Function('require', 'module', '__dirname', source + \`
      const actualStop = stopBrowserGroup;
      let ownedBrowser;
      stopBrowserGroup = async browser => {
        ownedBrowser = browser;
        process.kill(-browser.pid, 0);
        throw new Error('injected group shutdown failure');
      };
      return { actualStop, browser: () => ownedBrowser };
    \`)(require, ownedModule, ${JSON.stringify(__dirname)});
    let profile;
    let guardActive = true;
    let shutdownBegun = false;
    let removals = 0;
    (async () => {
      try {
        await ownedModule.exports.main({ evidenceDirectory: ${JSON.stringify(resultDirectory)},
          async cacheSetup(options) {
            profile = options.profile;
            return { receipt: { injectedTestDouble: true }, beginShutdown() { shutdownBegun = true; },
              async close() { guardActive = false; } };
          },
          removeProfile() { removals++; }
        });
        const result = JSON.parse(fs.readFileSync(${JSON.stringify(join(resultDirectory, 'result.json'))}, 'utf8'));
        assert.equal(guardActive, true, 'Live browser must retain interception');
        assert.equal(shutdownBegun, true);
        assert.equal(removals, 0);
        assert.equal(fs.existsSync(profile), true);
        process.kill(-controls.browser().pid, 0);
        assert.equal(result.cleanup.browserGroupStopped, false);
        assert.equal(result.cleanup.profileRemoved, false);
        assert.equal(result.cleanup.serverClosed, true);
        assert.equal(result.cleanupErrors.length, 1);
        assert.match(result.cleanupErrors[0], /injected group shutdown failure/);
        assert.equal(result.complete, false);
        console.error('Verified active guard and profile held while owned group remains alive');
      } finally {
        await controls.actualStop(controls.browser());
        if (profile) fs.rmSync(profile, { recursive: true, force: true });
      }
    })().catch(error => { console.error(error.stack); process.exitCode = 2; });
  `], { env: { ...process.env, BROWSER_BIN: executable }, stdio: ['ignore', 'pipe', 'pipe'] });
  let stderr = '';
  invocation.stdout.resume();
  invocation.stderr.on('data', bytes => { stderr += bytes; });
  try {
    const [code] = await once(invocation, 'exit');
    assert.equal(code, 1, stderr); // Held cleanup must remain a failed runner outcome.
    assert.match(stderr, /Verified active guard and profile held/);
  } finally {
    if (invocation.exitCode === null) invocation.kill('SIGKILL');
    rmSync(resultDirectory, { recursive: true, force: true });
  }
});

test('discovery cancellation failures propagate without retries or a fresh deadline', { timeout: 3000 }, async () => {
  const source = readFileSync(join(__dirname, 'browser-regression.cjs'), 'utf8');
  const setup = source.slice(source.indexOf('async function bounded('), source.indexOf('async function main('));
  for (const scenario of ['reject', 'stall']) {
    let attempts = 0;
    let cancellations = 0;
    const browser = Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
    const configure = vm.runInNewContext(setup + '\nconfigureOwnedBrowserCache;', {
      Date, join, URL, AbortSignal, AbortController, setTimeout, clearTimeout, delay,
      existsSync: () => true, readFileSync: () => '32123\n/unused',
      async fetch() {
        attempts++;
        return { ok: false, status: 503, body: { cancel() {
          cancellations++;
          return scenario === 'stall' ? new Promise(() => {}) : Promise.reject(new Error('body cancellation sentinel'));
        } } };
      },
      WebSocket: class { constructor() { throw new Error('Failed cancellation opened a protocol connection'); } },
    });
    const startedAt = Date.now();
    await assert.rejects(configure({ profile: '/owned/profile', fixtureUrl: 'unused', browser, budgetMs: 100 }),
      scenario === 'stall' ? /body cancellation timed out/ : /body cancellation sentinel/);
    assert.equal(attempts, 1);
    assert.equal(cancellations, 1);
    assert.ok(Date.now() - startedAt < 1000, 'Cancellation stays inside the original startup deadline');
    assert.equal(browser.listenerCount('exit'), 0);
  }
});

test('installer poll observer keeps its own deadline and retires timers, observers and frame', async () => {
  const driver = readFileSync(join(__dirname, 'fixtures/browser-driver.js'), 'utf8');
  const start = driver.indexOf('  function observeInstallerPolling(');
  const end = driver.indexOf('  // Independent iframe', start);
  assert.ok(start >= 0 && end > start);
  for (const scenario of ['complete', 'late-complete', 'deadline', 'dispose']) {
    let now = 1000;
    let inspect;
    let disconnected = 0;
    let removed = 0;
    let timer;
    const status = { textContent: 'Checking again…' };
    const node = { contentDocument: { getElementById(id) { assert.equal(id, 'stat'); return status; } } };
    const observe = vm.runInNewContext(driver.slice(start, end) + '\nobserveInstallerPolling;', {
      Date: { now: () => now },
      MutationObserver: class {
        constructor(callback) { inspect = callback; }
        observe(target) { assert.equal(target, status, 'Observe only the owned iframe status'); }
        disconnect() { disconnected++; }
      },
      setTimeout(callback, milliseconds) { timer = { callback, milliseconds }; return timer; },
      clearTimeout(id) { if (id === timer) timer = undefined; },
      closeInstaller(owned) { assert.equal(owned, node); removed++; },
    });
    const polling = observe(node);
    assert.equal(timer.milliseconds, 15000, 'The original five-poll deadline starts immediately');
    if (scenario === 'dispose') polling.dispose();
    else if (scenario === 'deadline') { now += 15000; timer.callback(); }
    else {
      now += scenario === 'complete' ? 12000 : 15001;
      status.textContent = 'Server unverified. Open Frayd manually, or choose Retry server check.';
      inspect();
    }
    now += 30000; // Another case may consume the result much later; never reset its clock.
    const result = await polling.completion;
    if (scenario === 'complete') {
      assert.equal(result.elapsedMs, 12000);
      assert.equal(result.error, undefined);
    } else assert.match(result.error.message, scenario === 'dispose' ? /disposed before completion/ : /exceeded15s/);
    assert.equal(timer, undefined, 'Settled observation retires its timer');
    assert.equal(disconnected, 1, 'Settled observation retires its observer once');
    polling.dispose();
    polling.dispose();
    assert.equal(removed, 1, 'Every completion/failure/disposal path closes its exact frame once');
  }
});
