'use strict';
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { createServer } = require('node:http');
const { mkdtempSync, readFileSync, existsSync, rmSync, writeFileSync } = require('node:fs');
const { join, resolve, dirname } = require('node:path');
const { tmpdir } = require('node:os');
const { once } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');
const { test } = require('node:test');
const vm = require('node:vm');
const { stopBrowserGroup } = require('./browser-regression.cjs');
const executable = join(__dirname, 'fixtures/browser-child-writer.cjs');

test('PID-only cleanup leaves a profile writer; owned process-group cleanup stops it before removal', { timeout: 8000 }, async () => {
  const profile = mkdtempSync(join(tmpdir(), 'frayd-cleanup-oracle-'));
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
    await Promise.race([ready, new Promise((_, reject) => {
      readyTimer = setTimeout(() => reject(new Error('Child-writer fixture did not become ready within 2 seconds')), 2000);
    })]);
    clearTimeout(readyTimer);
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
    rmSync(profile, { recursive: true, force: true });
  }
});

test('original browser failure remains visible alongside an injected profile-removal failure', { timeout: 8000 }, async () => {
  const runner = resolve(__dirname, 'browser-regression.cjs');
  const resultDirectory = mkdtempSync(join(tmpdir(), 'frayd-cleanup-result-'));
  let retainedProfile;
  const invocation = spawn(process.execPath, ['-e', `
    require(${JSON.stringify(runner)}).main({ evidenceDirectory: ${JSON.stringify(resultDirectory)}, cacheSetup: async () => ({ receipt: { injectedTestDouble: true }, close: async () => {} }), removeProfile(profile) {
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
