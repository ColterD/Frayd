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
  let response = async () => ({ ok: true, status: 200, json: async () => null });
  const document = {
    body: element(), querySelector: selector => nodes.get(selector) || null,
    getElementById: id => nodes.get('#' + id), querySelectorAll: () => [],
    createElement: element, addEventListener() {},
  };
  const context = vm.createContext({
    document, localStorage: {}, location: { hostname: 'test.invalid' },
    console: { log() {}, warn() {} }, innerWidth: 1000, innerHeight: 800,
    fetch: async (url, options) => { requests.push({ url, options }); return response(url, options); },
    addEventListener: (name, fn) => listeners.set(name, fn),
    setTimeout() {}, clearTimeout() {},
    setInterval() {}, clearInterval() {},
    Plyr: class {}, Image: class {},
    WebSocket: class {},
  });
  context.window = context;
  vm.runInContext(readFileSync(join(clientPath, 'js/app.js'), 'utf8'), context);
  return {
    nodes, requests,
    run: source => vm.runInContext(source, context),
    respond: fn => { response = fn; },
    message: event => listeners.get('message')(event),
  };
}


const highlight = { type: 'highlight', cfi: 'char:0:5', text: 'Hello' };
function reader() {
  const b = browser();
  b.run('ebook = { it: { id: 7 }, chunk: 1, spine: ["a", "b", "c"], document: { generation: 1, ready: true, items: [] } }');
  b.send = (data, overrides = {}) => b.message({
    data: data && typeof data === "object" && !Array.isArray(data) ? { generation: b.run("ebook?.document?.generation"), ...data } : data, origin: 'null', source: b.nodes.get('#em-frame').contentWindow, ...overrides,
  });
  return b;
}

for (const [name, overrides] of [
  ['cross-origin window', { origin: 'https://attacker.invalid', source: {} }],
  ['same-origin sibling window', { origin: 'https://frayd.invalid', source: {} }],
  ['unrelated opaque sandbox', { origin: 'null', source: {} }],
  ['missing source', { source: null }],
  ['active frame with unexpected origin', { origin: 'https://attacker.invalid' }],
]) {
  test(`rejects ${name} before annotation, progress, chapter or UI effects`, () => {
    const b = reader();
    for (const data of [highlight, { type: 'page', page: 2, pages: 3 },
      { type: 'chapter', dir: 1 }, { type: 'geom' },
      { type: 'search-results', count: 8 }, { type: 'search-hit', idx: 1, total: 8 }]) {
      b.send(data, overrides);
    }
    assert.deepEqual(b.requests, []);
    assert.equal(b.run('ebook.chunk'), 1);
    assert.equal(b.nodes.get('#em-pageinfo').textContent, '');
    assert.equal(b.nodes.get('#em-searchinfo').textContent, '');
  });
}

test('legitimate opaque reader persists highlights and fractional progress, updates search, and closes', () => {
  const b = reader();
  b.send(highlight);
  assert.equal(b.requests[0].url, '/api/annotations');
  assert.deepEqual(JSON.parse(b.requests[0].options.body), {
    media_id: 7, kind: 'highlight', spine_idx: 1, cfi: 'char:0:5', text: 'Hello', color: 'yellow',
  });
  b.send({ type: 'page', page: 3, pages: 5 });
  assert.equal(JSON.parse(b.requests[1].options.body).position, 1.5);
  assert.equal(b.nodes.get('#em-pageinfo').textContent, '2 · p3/5');
  b.send({ type: 'search-results', count: 2 });
  assert.equal(b.run('ebook.searchHits'), 2);
  b.send({ type: 'search-hit', idx: 1, total: 2 });
  assert.equal(b.nodes.get('#em-searchinfo').textContent, '1/2 in ch.');
  b.nodes.get('#em-close').onclick();
  assert.equal(JSON.parse(b.requests[2].options.body).position, 1.5);
  b.send(highlight);
  assert.equal(b.requests.length, 3);
});

test('closed reader rejects messages even from its former frame', () => {
  const b = reader();
  b.run('ebook = null');
  b.send(highlight);
  assert.deepEqual(b.requests, []);
});


test('active reader rejects malformed payloads without state or network effects', () => {
  const b = reader();
  for (const data of [null, 'highlight', [], {},
    { type: 'chapter', dir: '1' }, { type: 'chapter', dir: 0 }, { type: 'chapter', dir: 0.5 },
    { type: 'highlight', cfi: {} , text: 'Hello' },
    { ...highlight, cfi: 'not-a-range' }, { ...highlight, text: {} }, { ...highlight, text: ' ' },
    { type: 'page', page: '2', pages: 3 }, { type: 'page', page: NaN, pages: 3 },
    { type: 'page', page: Infinity, pages: Infinity }, { type: 'page', page: 0, pages: 3 },
    { type: 'page', page: 4, pages: 3 }, { type: 'page', page: 1, pages: Number.MAX_SAFE_INTEGER + 1 },
    { type: 'search-results', count: -1 }, { type: 'search-results', count: '2' },
    { type: 'search-hit', idx: 0, total: 3 }, { type: 'search-hit', idx: 4, total: 3 },
    { type: 'search-hit', idx: 1, total: Infinity }, { type: 'unknown' },
  ]) b.send(data);
  assert.deepEqual(b.requests, []);
  assert.equal(b.run('ebook.chunk'), 1);
  assert.equal(b.nodes.get('#em-pageinfo').textContent, '');
  assert.equal(b.nodes.get('#em-searchinfo').textContent, '');
});

test('missing or replaced frame cannot authorize an old sender', () => {
  const b = reader();
  const previous = b.nodes.get('#em-frame').contentWindow;
  b.nodes.get('#em-frame').contentWindow = null;
  b.send(highlight);
  b.nodes.get('#em-frame').contentWindow = { postMessage() {} };
  b.send(highlight, { source: previous });
  assert.deepEqual(b.requests, []);
});

test('reader chapter reports preserve both directions and enforce spine boundaries', async () => {
  const b = reader();
  b.respond(async () => ({ text: async () => '<html><head></head><body>Chapter</body></html>' }));
  b.send({ type: 'chapter', dir: 1 });
  await new Promise(setImmediate);
  assert.equal(b.run('ebook.chunk'), 2);
  b.send({ type: 'chapter', dir: 1 });
  assert.equal(b.requests.length, 1);
  b.run('ebook.document.ready = true');
  b.send({ type: 'chapter', dir: -1 });
  await new Promise(setImmediate);
  assert.equal(b.run('ebook.chunk'), 1);
  assert.equal(b.run('ebook.lastFrac'), 0.999);
  b.run('ebook.chunk = 0; ebook.document.ready = true');
  b.send({ type: 'chapter', dir: -1 });
  assert.deepEqual(b.requests.map(r => r.url), ['/api/epub/7/chunk/2', '/api/epub/7/chunk/1']);
});

test('reader single-page, final-page, empty search and geometry reports remain accepted', () => {
  const b = reader();
  b.send({ type: 'page', page: 1, pages: 1 });
  b.send({ type: 'page', page: 3, pages: 3 });
  assert.deepEqual(b.requests.map(r => JSON.parse(r.options.body).position), [1, 1.999]);
  b.send({ type: 'search-results', count: 0 });
  assert.equal(b.nodes.get('#em-searchinfo').textContent, 'No hits');
  b.send({ type: 'geom', innerWidth: 760 });
  assert.equal(b.requests.length, 2);
});

test('ready restores resume and saved marks once, and rejects stale generations on the same WindowProxy', async () => {
  const b = reader();
  const posted = [];
  b.nodes.get('#em-frame').contentWindow.postMessage = data => posted.push(data);
  b.respond(async () => ({ text: async () => '<html><head></head><body>Hello</body></html>' }));
  b.run('ebook.annots = [{ kind: "highlight", spine_idx: 1, cfi: "char:0:5", color: "yellow" }]');
  await b.run('showChunk(1, { frac: 0.75 })');
  const generation = b.run('ebook.document.generation');
  assert.deepEqual(posted, []);
  b.send({ type: 'reader-ready', generation: generation - 1 });
  b.send({ type: 'page', page: 1, pages: 4 });
  assert.deepEqual(posted, []);
  assert.equal(b.requests.length, 1);
  b.send({ type: 'reader-ready', generation });
  assert.deepEqual(JSON.parse(JSON.stringify(posted)), [
    { type: 'resume', col: 0.75, generation },
    { type: 'inject-hl', generation, items: [{ off: 0, len: 5, color: 'yellow' }] },
  ]);
  b.send({ type: 'reader-ready', generation });
  assert.equal(posted.length, 2);
  await b.run('showChunk(1, { atEnd: true })');
  b.send({ type: 'reader-ready', generation });
  b.send({ ...highlight, generation });
  assert.equal(posted.length, 2);
  assert.equal(b.requests.length, 2);
  b.send({ type: 'reader-ready' });
  assert.equal(posted[2].col, 'end');
  b.nodes.get('#em-close').onclick();
  b.send({ type: 'reader-ready', generation: b.run('ebookDocumentGeneration') });
  assert.equal(posted.length, 4);
});

test('late chapter fetch cannot replace the latest chapter or a closed reader', async () => {
  const b = reader();
  const finish = [];
  b.respond(() => new Promise(resolve => finish.push(resolve)));
  const old = b.run('showChunk(0)');
  const current = b.run('showChunk(2)');
  finish[1]({ text: async () => '<html><head></head><body>Current</body></html>' });
  await current;
  const html = b.nodes.get('#em-frame').srcdoc;
  finish[0]({ text: async () => '<html><head></head><body>Retired</body></html>' });
  await old;
  assert.equal(b.nodes.get('#em-frame').srcdoc, html);
  const closed = b.run('showChunk(1)');
  b.nodes.get('#em-close').onclick();
  finish[2]({ text: async () => '<html><head></head><body>Closed</body></html>' });
  await closed;
  assert.equal(b.nodes.get('#em-frame').srcdoc, '');
});

test('closed or superseded book opening cannot publish its late TOC and resume', async () => {
  const b = reader();
  let finishToc;
  b.respond(async url => {
    if (url.includes('/spine')) return { json: async () => ['chapter.xhtml'] };
    if (url.includes('/annotations/')) return { ok: true, json: async () => [] };
    if (url.includes('/toc')) return new Promise(resolve => { finishToc = resolve; });
    return { ok: true, json: async () => null };
  });
  const opening = b.run('openEbook({ id: 9, title: "Old" })');
  await new Promise(setImmediate);
  b.nodes.get('#em-close').onclick();
  b.run('ebook = { it: { id: 10 }, chunk: 0, spine: ["new"], toc: [], document: { generation: 10, ready: true } }');
  finishToc({ ok: true, json: async () => [{ title: 'Old TOC' }] });
  await opening;
  await new Promise(setImmediate);
  assert.equal(b.run('ebook.it.id'), 10);
  assert.equal(b.run('ebook.toc.length'), 0);
  assert.equal(b.requests.some(request => request.url.includes('/chunk/') || request.url.includes('/progress/9')), false);
  assert.equal(b.nodes.get('#em-frame').srcdoc, '');
});
