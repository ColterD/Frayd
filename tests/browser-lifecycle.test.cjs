'use strict';
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { createServer } = require('node:http');
const { mkdtempSync, readFileSync, existsSync, rmSync } = require('node:fs');
const { join, resolve, dirname } = require('node:path');
const { tmpdir } = require('node:os');
const { once } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');
const { test } = require('node:test');
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
    require(${JSON.stringify(runner)}).main({ evidenceDirectory: ${JSON.stringify(resultDirectory)}, removeProfile(profile) {
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
