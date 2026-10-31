'use strict';
// Decode the actual Frayd stream in an existing Chromium browser; no installation or mock fetch.
const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { spawn } = require('node:child_process');
const { existsSync, mkdtempSync, rmSync, realpathSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { delimiter, join } = require('node:path');
const { stopBrowserGroup } = require('../tests/browser-regression.cjs');

function browserPath() {
  if (process.env.BROWSER_BIN) return realpathSync(process.env.BROWSER_BIN);
  for (const name of ['google-chrome', 'chromium', 'chromium-browser', 'chrome', 'chrome-headless-shell']) {
    for (const folder of (process.env.PATH || '').split(delimiter)) {
      if (existsSync(join(folder, name))) return realpathSync(join(folder, name));
    }
  }
  throw new Error('Set BROWSER_BIN to an existing Chromium browser; no download is performed');
}

async function main() {
  const source = new URL(process.argv[2]);
  assert.equal(source.protocol, 'http:');
  assert.equal(source.hostname, '127.0.0.1');
  assert.match(source.pathname, /^\/api\/stream\/\d+$/);
  assert.equal(source.search + source.username + source.password + source.hash, '');
  let resolveResult;
  const result = new Promise(resolve => { resolveResult = resolve; });
  const server = createServer((request, response) => {
    if (request.method === 'POST' && request.url === '/result') {
      let body = '';
      request.on('data', data => { body += data; if (body.length > 4096) request.destroy(); });
      request.on('end', () => { resolveResult(JSON.parse(body)); response.end('ok'); });
      return;
    }
    if (request.url !== '/') { response.writeHead(404); response.end(); return; }
    response.setHeader('Content-Type', 'text/html');
    response.end(`<!doctype html><video muted playsinline></video><script>
      const video = document.querySelector('video'); video.muted = true;
      video.src = ${JSON.stringify(source.href)};
      let sent = false;
      function report(value) { if (!sent) { sent = true; fetch('/result', {method:'POST',body:JSON.stringify(value)}); } }
      video.addEventListener('error', () => report({error:'media error',code:video.error?.code}));
      video.play().catch(error => report({error:error.name}));
      const poll = setInterval(() => {
        const frames = video.getVideoPlaybackQuality().totalVideoFrames;
        if (video.currentTime > 0.2 && frames > 0 && video.videoWidth > 0) {
          clearInterval(poll); report({muted:video.muted,currentTime:video.currentTime,frames,width:video.videoWidth,height:video.videoHeight});
        }
      }, 50);
      </script>`);
  });
  let browser;
  let profile;
  let stopped = false;
  let timer;
  try {
    const executable = browserPath();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    profile = mkdtempSync(join(tmpdir(), 'frayd-download-playback-'));
    browser = spawn(executable, ['--headless', '--no-sandbox', '--disable-gpu', '--mute-audio',
      '--autoplay-policy=no-user-gesture-required', '--disable-background-networking', '--disable-component-update',
      '--disable-sync', '--no-first-run', '--no-default-browser-check', '--disable-features=MediaRouter',
      '--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1', `--user-data-dir=${profile}`,
      `http://127.0.0.1:${server.address().port}/`], { detached: true, stdio: 'ignore' });
    const failed = new Promise((_, reject) => {
      browser.once('error', reject);
      browser.once('exit', () => reject(new Error('Browser exited before playback proof')));
      timer = setTimeout(() => reject(new Error('Real stream playback exceeded 25 seconds')), 25000);
    });
    const observed = await Promise.race([result, failed]);
    assert.equal(observed.muted, true, JSON.stringify(observed));
    assert(observed.currentTime > 0.2 && observed.frames > 0 && observed.width > 0 && observed.height > 0);
    console.log('PASS: actual Frayd API muted browser playback ' + JSON.stringify(observed));
  } finally {
    clearTimeout(timer);
    try { await stopBrowserGroup(browser); stopped = true; }
    finally {
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
      if (stopped && profile) rmSync(profile, { recursive: true, force: false });
    }
  }
}
if (require.main === module) main().catch(error => { console.error(error.message); process.exitCode = 1; });
