const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');

const clientPath = join(__dirname, '..', 'client');

// Only browser boundaries are doubled: the complete application scripts run as-is.
function browser() {
  const nodes = new Map();
  const listeners = new Map();
  function element() {
    const classes = new Set();
    const attributes = new Map();
    return {
      style: {}, dataset: {}, children: [], value: '', type: '', innerHTML: '', textContent: '',
      src: '', paused: true, readyState: 1, currentTime: 0, clientWidth: 760, clientHeight: 600,
      classList: {
        add: name => classes.add(name), remove: name => classes.delete(name),
        contains: name => classes.has(name),
      },
      setAttribute: (name, value) => attributes.set(name, value),
      getAttribute: name => attributes.get(name),
      addEventListener() {},
      appendChild(child) { this.children.push(child); },
      append(...children) { this.children.push(...children); },
      pause() { this.paused = true; },
      play() { this.paused = false; return Promise.resolve(); },
    };
  }
  const html = readFileSync(join(clientPath, 'index.html'), 'utf8');
  for (const match of html.matchAll(/<[^>]+\bid="([^"]+)"[^>]*>/g)) {
    const node = element();
    for (const attr of match[0].matchAll(/([\w-]+)="([^"]*)"/g)) {
      node.setAttribute(attr[1], attr[2]);
      if (attr[1] === 'type') node.type = attr[2];
    }
    nodes.set('#' + match[1], node);
  }
  for (const selector of ['#search input', '.avatar', '#hero-play', '#hero-backdrop', '#pt-host', '#pt-join', '#party-code-input']) nodes.set(selector, element());
  nodes.get('#em-frame').contentWindow = { postMessage() {} };
  nodes.get('#add-media').innerHTML = html.match(/<button id="add-media">([\s\S]*?)<\/button>/)[1];
  const requests = [];
  const intervals = new Map();
  let response = async () => ({ ok: true, status: 200, json: async () => null });
  const document = {
    body: element(), querySelector: selector => nodes.get(selector) || null,
    getElementById: id => nodes.get('#' + id), querySelectorAll: () => [],
    createElement: element, addEventListener() {},
  };
  const sockets = [];
  const context = vm.createContext({
    document, localStorage: {}, location: { hostname: 'test.invalid' },
    console: { log() {}, warn() {} }, innerWidth: 1000, innerHeight: 800,
    fetch: async (url, options) => { requests.push({ url, options }); return response(url, options); },
    addEventListener: (name, fn) => listeners.set(name, fn),
    setTimeout() {}, clearTimeout() {},
    setInterval: fn => { const id = intervals.size + 1; intervals.set(id, fn); return id; },
    clearInterval: id => intervals.delete(id),
    requestAnimationFrame: fn => { try { fn(); } catch {} },
    confirm: () => false,
    Plyr: class {}, Image: class {},
    WebSocket: class { constructor() { sockets.push(this); } },
  });
  context.window = context;
  vm.runInContext(readFileSync(join(clientPath, 'js/app.js'), 'utf8'), context);
  return {
    context, nodes, requests, intervals, sockets,
    run: source => vm.runInContext(source, context),
    respond: fn => { response = fn; },
    page: (page, pages) => listeners.get('message')({
      data: { type: 'page', page, pages }, origin: 'null', source: nodes.get('#em-frame').contentWindow,
    }),
  };
}

function progress(browser) {
  return browser.requests.filter(r => r.url === '/api/progress').map(r => JSON.parse(r.options.body).position);
}

test('closing an ebook preserves the last reported fractional chapter position', () => {
  const b = browser();
  b.run('ebook = { it: { id: 7 }, chunk: 2, spine: ["a", "b", "c"] }');
  b.page(3, 5);
  b.nodes.get('#em-close').onclick();
  assert.deepEqual(progress(b), [2.5, 2.5]);
  assert.equal(b.nodes.get('#em-frame').src, 'about:blank');
});

test('closing before any page report saves a finite chapter position', () => {
  const b = browser();
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a"] }');
  b.nodes.get('#em-close').onclick();
  assert.deepEqual(progress(b), [0]);
});

test('changing chapters resets the previous fractional position before layout reports', async () => {
  const b = browser();
  b.respond(async () => ({ text: async () => '<html><head></head><body>Chapter</body></html>' }));
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a", "b"] }');
  b.page(3, 5);
  await b.run('showChunk(1)');
  b.nodes.get('#em-close').onclick();
  assert.deepEqual(progress(b), [0.5, 1]);
});

test('closing while the next chapter fetch is pending saves the new chapter position', async () => {
  const b = browser();
  let finishFetch;
  b.respond(url => url.includes('/chunk/')
    ? new Promise(resolve => { finishFetch = resolve; })
    : Promise.resolve({ ok: true }));
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a", "b"] }');
  b.page(3, 5);
  const loading = b.run('showChunk(1)');
  b.nodes.get('#em-close').onclick();
  assert.deepEqual(progress(b), [0.5, 1]);
  finishFetch({ text: async () => '<html><head></head><body>Chapter</body></html>' });
  await loading;
});

test('closing a resumed chapter before layout preserves the requested fraction', async () => {
  const b = browser();
  b.respond(async () => ({ text: async () => '<html><head></head><body>Chapter</body></html>' }));
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a", "b"] }');
  await b.run('showChunk(1, { frac: 0.75 })');
  b.nodes.get('#em-close').onclick();
  assert.deepEqual(progress(b), [1.75]);
});

test('password reveal advertises the action for the current visibility in both directions', () => {
  const b = browser();
  const button = b.nodes.get('#pw-toggle');
  const password = b.nodes.get('#password');
  assert.equal(button.getAttribute('aria-label'), 'Show password');
  button.onclick();
  assert.equal(password.type, 'text');
  assert.equal(button.getAttribute('aria-label'), 'Hide password');
  button.onclick();
  assert.equal(password.type, 'password');
  assert.equal(button.getAttribute('aria-label'), 'Show password');
});

test('scan disables repeat clicks, polls until idle, refreshes and restores the control', async () => {
  const b = browser();
  let scanning = true;
  b.respond(async url => ({
    status: 202,
    json: async () => url === '/api/scan/status' ? { scanning } : [],
  }));
  const button = b.nodes.get('#add-media');
  const original = button.innerHTML;
  await button.onclick();
  await button.onclick();
  assert.equal(button.disabled, true);
  assert.deepEqual(b.requests.map(r => [r.url, r.options?.method]), [['/api/scan', 'POST']]);
  const poll = [...b.intervals.values()][0];
  await poll();
  assert.equal(button.disabled, true);
  scanning = false;
  await poll();
  assert.equal(button.disabled, false);
  assert.equal(button.innerHTML, original);
  assert.equal(b.intervals.size, 0);
  assert.deepEqual(b.requests.map(r => r.url), ['/api/scan', '/api/scan/status', '/api/scan/status', '/api/library']);
});

test('party follow detects a video after each library replacement', async () => {
  const b = browser();
  b.respond(async url => ({ ok: true, json: async () => url === '/api/library' ? [{ id: 9, kind: 'movie', title: 'Movie' }] : null }));
  await b.run('loadLibrary()');
  b.run('partyFollow(9, "Movie", 12, false, 0)');
  assert.equal(b.nodes.get('#plyr-video').src, '/api/stream/9');
  assert.equal(b.nodes.get('#pb-audio').src, '');
  b.respond(async url => ({ ok: true, json: async () => url === '/api/library' ? [{ id: 10, kind: 'show', title: 'Episode' }] : null }));
  await b.run('loadLibrary()');
  b.run('partyFollow(10, "Episode", 4, false, 0)');
  assert.equal(b.nodes.get('#plyr-video').src, '/api/stream/10');
});

test('party guest request picker follows library replacements', async () => {
  const b = browser();
  b.respond(async url => ({ ok: true, json: async () => url === '/api/library' ? [{ id: 9, kind: 'movie', title: 'Movie' }] : null }));
  await b.run('loadLibrary()');
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
  b.nodes.get('#party-code-input').value = 'ABCD';
  b.run('PartyUI.open()');
  b.nodes.get('#pt-join').onclick();
  b.sockets[0].onmessage({ data: JSON.stringify({ type: 'state', is_host: false, state: { queue: [], position: 0 } }) });
  assert.match(b.nodes.get('#party-stage').innerHTML, /data-media="9"/);
  assert.match(b.nodes.get('#party-stage').innerHTML, /Movie/);
  b.respond(async url => ({ ok: true, json: async () => url === '/api/library' ? [{ id: 10, kind: 'movie', title: 'Replacement' }] : null }));
  await b.run('loadLibrary()');
  b.sockets[0].onmessage({ data: JSON.stringify({ type: 'members', members: [] }) });
  assert.match(b.nodes.get('#party-stage').innerHTML, /data-media="10"/);
  assert.doesNotMatch(b.nodes.get('#party-stage').innerHTML, /data-media="9"/);
});
