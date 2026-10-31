const assert = require('node:assert/strict');
const { test } = require('node:test');
const vm = require('node:vm');
const { browser, settle, reply } = require('./fixtures/client-browser.cjs');

const highlight = { type: 'highlight', cfi: 'char:0:5', text: 'Hello' };
function reader() {
  const b = browser();
  b.run('ebookDocumentGeneration = 1; ebook = { it: { id: 7 }, annots: [], chunk: 1, spine: ["a", "b", "c"], document: { generation: 1, ready: true, frameWindow: $("#em-frame").contentWindow, items: [] } }');
  b.send = (data, overrides = {}) => b.dispatchMessage({
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
      { type: 'chapter', dir: 1 },
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
  assert.equal(b.nodes.get('#em-searchinfo').textContent, '2 hits — Enter for next');
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
  b.respond(async () => ({ ok: true, text: async () => '<html><head></head><body>Chapter</body></html>' }));
  b.send({ type: 'chapter', dir: 1 });
  await settle();
  assert.equal(b.run('ebook.chunk'), 2);
  b.send({ type: 'chapter', dir: 1 });
  assert.equal(b.requests.length, 1);
  b.run('ebook.document.ready = true');
  b.send({ type: 'chapter', dir: -1 });
  await settle();
  assert.equal(b.run('ebook.chunk'), 1);
  assert.equal(b.run('ebook.lastFrac'), 0.999);
  b.run('ebook.chunk = 0; ebook.document.ready = true');
  b.send({ type: 'chapter', dir: -1 });
  assert.deepEqual(b.requests.map(r => r.url), ['/api/epub/7/chunk/2', '/api/epub/7/chunk/1']);
});

test('reader single-page, final-page and empty search reports remain accepted', () => {
  const b = reader();
  b.send({ type: 'page', page: 1, pages: 1 });
  b.send({ type: 'page', page: 3, pages: 3 });
  assert.deepEqual(b.requests.map(r => JSON.parse(r.options.body).position), [1, 1.999]);
  b.send({ type: 'search-results', count: 0 });
  assert.equal(b.nodes.get('#em-searchinfo').textContent, 'No hits');
  assert.equal(b.requests.length, 2);
});

test('ready restores resume and saved marks once, and rejects stale generations on the current WindowProxy', async () => {
  const b = reader();
  const posted = [];
  b.respond(async () => ({ ok: true, text: async () => '<html><head></head><body>Hello</body></html>' }));
  b.run('ebook.annots = [{ kind: "highlight", spine_idx: 1, cfi: "char:0:5", color: "yellow" }]');
  await b.run('showChunk(1, { frac: 0.75 })');
  b.nodes.get('#em-frame').contentWindow.postMessage = data => posted.push(data);
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
  b.nodes.get('#em-frame').contentWindow.postMessage = data => posted.push(data);
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
  finish[1]({ ok: true, text: async () => '<html><head></head><body>Current</body></html>' });
  await current;
  const html = b.nodes.get('#em-frame').srcdoc;
  finish[0]({ ok: true, text: async () => '<html><head></head><body>Retired</body></html>' });
  await old;
  assert.equal(b.nodes.get('#em-frame').srcdoc, html);
  const closed = b.run('showChunk(1)');
  b.nodes.get('#em-close').onclick();
  finish[2]({ ok: true, text: async () => '<html><head></head><body>Closed</body></html>' });
  await closed;
  assert.equal(b.nodes.get('#em-frame').srcdoc, '');
});

test('closed or superseded book opening cannot publish its late TOC and resume', async () => {
  const b = reader();
  let finishToc;
  b.respond(async url => {
    if (url.includes('/spine')) return { json: async () => ['chapter.xhtml'] };
    if (url.includes('/annotations/')) return reply([]);
    if (url.includes('/toc')) return new Promise(resolve => { finishToc = resolve; });
    return reply(null);
  });
  const opening = b.run('openEbook({ id: 9, title: "Old" })');
  await settle();
  b.nodes.get('#em-close').onclick();
  b.run('ebook = { it: { id: 10 }, annots: [], chunk: 0, spine: ["new"], toc: [], document: { generation: 10, ready: true } }');
  finishToc({ ok: true, json: async () => [{ title: 'Old TOC' }] });
  await opening;
  await settle();
  assert.equal(b.run('ebook.it.id'), 10);
  assert.equal(b.run('ebook.toc.length'), 0);
  assert.equal(b.requests.some(request => request.url.includes('/chunk/') || request.url.includes('/progress/9')), false);
  assert.equal(b.nodes.get('#em-frame').srcdoc, '');
});

for (const failure of [false, true]) {
  test(`chapter ${failure ? 'error' : 'body'} publishes a fresh configured frame and rejects the retired window`, async () => {
    const b = reader();
    const retired = b.nodes.get('#em-frame');
    const attributes = ['id', 'sandbox', 'title', 'referrerpolicy'];
    retired.setAttribute('title', 'Reader');
    retired.setAttribute('referrerpolicy', 'no-referrer');
    b.respond(async () => {
      if (failure) throw new Error('Fixture chapter failure');
      return { ok: true, text: async () => '<html><head></head><body>Published chapter</body></html>' };
    });
    await b.run('showChunk(1, { frac: 0.75 })');
    const current = b.nodes.get('#em-frame');
    assert.notEqual(current, retired);
    assert.notEqual(current.contentWindow, retired.contentWindow);
    for (const name of attributes) assert.equal(current.getAttribute(name), retired.getAttribute(name));
    assert.equal(retired.replacementAtPublication.srcdoc, current.srcdoc);
    assert.match(current.srcdoc, failure ? /Failed to load chapter/ : /Published chapter/);
    const generation = b.run('ebook.document.generation');
    for (const data of [{ type: 'reader-ready' }, highlight, { type: 'page', page: 2, pages: 4 }, { type: 'chapter', dir: 1 }]) {
      b.send({ ...data, generation }, { source: retired.contentWindow });
    }
    assert.equal(b.run('ebook.document.ready'), false);
    assert.equal(b.run('ebook.chunk'), 1);
    assert.equal(b.requests.length, 1);
    assert.equal(b.nodes.get('#em-pageinfo').textContent, failure ? 'Chapter unavailable' : '');
    // A queued reset on the removed node cannot overwrite the published body.
    retired.srcdoc = '';
    retired.src = 'about:blank';
    assert.match(current.srcdoc, failure ? /Failed to load chapter/ : /Published chapter/);
  });
}

async function paginationDocument({ visible = false, pages = 4, layoutPending = false } = {}) {
  const b = reader();
  b.run('ebook.spine.push("d"); ebook.chunk = 2');
  b.respond(async () => ({ ok: true, text: async () => '<html><head></head><body>Saved chapter</body></html>' }));
  await b.run('showChunk(2, { frac: 1 / 3 })');
  // Extract the bridge for VM execution; this is not an HTML sanitizer.
  const script = b.nodes.get('#em-frame').srcdoc.match(/<script>([\s\S]*?)<\/script>/i)[1];
  const events = new Map(), renders = [];
  let width = visible ? 760 : 0;
  const body = { appendChild() {}, getBoundingClientRect: () => ({ width, height: width ? 600 : 0 }) };
  const document = {
    body, head: { appendChild() {} }, fonts: { ready: Promise.resolve() },
    documentElement: {
      get clientWidth() { return width; }, get clientHeight() { return width ? 600 : 0; },
      get scrollWidth() { return width && !layoutPending ? 760 * (pages + 1) : 0; },
    },
    createElement: () => ({ style: {} }), addEventListener() {},
  };
  const parent = { postMessage: data => b.message(data) };
  const frame = {
    get innerWidth() { return width; }, get innerHeight() { return width ? 600 : 0; },
    scrollX: 0, scrollTo({ left }) { this.scrollX = left; },
    addEventListener(name, fn) { events.set(name, [...(events.get(name) || []), fn]); },
    removeEventListener(name, fn) { events.set(name, (events.get(name) || []).filter(listener => listener !== fn)); },
  };
  const emit = (name, event) => { for (const fn of [...(events.get(name) || [])]) fn(event); };
  b.nodes.get('#em-frame').contentWindow.postMessage = data => emit('message', { source: parent, data });
  vm.runInNewContext(script, { window: frame, document, parent,
    requestAnimationFrame: fn => renders.push(fn), setTimeout() {} });
  return { b, load: () => emit('load'), show: () => { width = 760; emit('resize'); },
    async render() { await Promise.resolve(); layoutPending = false; for (const fn of renders.splice(0)) fn(); },
  };
}

test('initial unmeasurable chapter retains saved 2.333 until rendered pagination restores its column', async () => {
  const reader = await paginationDocument();
  reader.load();
  await reader.render();
  assert.equal(reader.b.run('ebook.lastFrac'), 1 / 3);
  assert.equal(reader.b.run('ebook.document.ready'), false);
  assert.equal(reader.b.requests.filter(request => request.options?.method === 'POST').length, 0);
  reader.show();
  await reader.render();
  assert.equal(reader.b.run('ebook.document.ready'), true);
  assert.equal(reader.b.nodes.get('#em-pageinfo').textContent, '3 · p2/4');
  assert.equal(reader.b.run('ebook.lastFrac'), 1 / 3);
  assert.deepEqual(reader.b.requests.filter(request => request.options?.method === 'POST')
    .map(request => JSON.parse(request.options.body).position), [2 + 1 / 3]);
});

test('visible rendered single-page chapter legitimately publishes fraction zero', async () => {
  const reader = await paginationDocument({ visible: true, pages: 1 });
  reader.load(); await reader.render();
  assert.equal(reader.b.run('ebook.document.ready'), true);
  assert.equal(reader.b.nodes.get('#em-pageinfo').textContent, '3 · p1/1');
  assert.equal(reader.b.run('ebook.lastFrac'), 0);
  assert.deepEqual(reader.b.requests.filter(request => request.options?.method === 'POST')
    .map(request => JSON.parse(request.options.body).position), [2]);
});

test('visible viewport waits for a render before its first resume can overwrite the saved fraction', async () => {
  const reader = await paginationDocument({ visible: true, layoutPending: true });
  reader.load();
  assert.equal(reader.b.run('ebook.lastFrac'), 1 / 3);
  assert.equal(reader.b.run('ebook.document.ready'), false);
  await reader.render();
  assert.equal(reader.b.nodes.get('#em-pageinfo').textContent, '3 · p2/4');
  assert.equal(reader.b.run('ebook.lastFrac'), 1 / 3);
});

for (const [status, body] of [[404, ''], [500, 'Fixture server rejection'], [200, ''], [200, 'Plain chapter without a head']]) {
  test(`chapter response ${status} ${body ? 'text' : 'empty'} publishes a current error without losing the requested fraction`, async () => {
    const b = reader();
    const response = new Response(body, { status });
    b.respond(async () => response);
    b.nodes.get('#em-pageinfo').textContent = 'Loading…';
    const previous = b.nodes.get('#em-frame');
    await b.run('showChunk(1, { frac: 0.75 })');
    assert.notEqual(b.nodes.get('#em-frame'), previous);
    assert.match(b.nodes.get('#em-frame').srcdoc, /Failed to load chapter/);
    assert.equal(b.nodes.get('#em-pageinfo').textContent, 'Chapter unavailable');
    assert.equal(b.run('ebook.lastFrac'), 0.75);
    assert.equal(b.run('ebook.document.ready'), false);
    assert.equal(b.requests.filter(request => request.options?.method === 'POST').length, 0);
    assert.equal(response.bodyUsed, status === 200, 'Rejected HTTP body is not read or rendered');
  });
}

for (const status of [404, 500]) {
  test(`late HTTP ${status} cannot erase a newer ready chapter or overwrite its bookmark`, async () => {
    const b = reader();
    let release;
    b.respond(() => new Promise(resolve => { release = resolve; }));
    const stale = b.run('showChunk(0)');
    b.respond(async () => new Response('<html><head></head><body>Current ready chapter</body></html>'));
    await b.run('showChunk(2, { frac: 0.75 })');
    b.send({ type: 'reader-ready' });
    b.send({ type: 'page', page: 4, pages: 5 });
    const current = b.nodes.get('#em-frame');
    const html = current.srcdoc;
    release(new Response('Retired error', { status }));
    await stale;
    assert.equal(b.nodes.get('#em-frame'), current);
    assert.equal(current.srcdoc, html);
    assert.equal(b.nodes.get('#em-pageinfo').textContent, '3 · p4/5');
    assert.equal(b.run('ebook.lastFrac'), 0.75);
    assert.deepEqual(b.requests.filter(request => request.options?.method === 'POST').map(request => JSON.parse(request.options.body).position), [2.75]);
  });
}

test('HTTP 200 chapter with a head publishes its bridge and awaits the normal ready handshake', async () => {
  const b = reader();
  b.respond(async () => new Response('<html><head></head><body>Valid chapter</body></html>'));
  await b.run('showChunk(1, { frac: 0.75 })');
  assert.match(b.nodes.get('#em-frame').srcdoc, /reader-ready/);
  assert.match(b.nodes.get('#em-frame').srcdoc, /Valid chapter/);
  assert.equal(b.run('ebook.document.ready'), false);
  b.send({ type: 'reader-ready' });
  b.send({ type: 'page', page: 4, pages: 5 });
  assert.equal(b.nodes.get('#em-pageinfo').textContent, '2 · p4/5');
  assert.equal(b.run('ebook.lastFrac'), 0.75);
});


for (const phase of ['pending', 'failed']) {
  test(`closing a ${phase} chapter cannot save its unrendered requested position`, async () => {
    const b = reader();
    b.run('ebook.lastFrac = 1 / 3');
    let resolveChapter;
    b.respond(() => new Promise(resolve => { resolveChapter = resolve; }));
    const opening = b.run('showChunk(2, { frac: 0.75 })');
    if (phase === 'failed') {
      resolveChapter({ ok: false, status: 404 });
      await opening;
    }
    b.nodes.get('#em-close').onclick();
    assert.deepEqual(b.requests.filter(request => request.options?.method === 'POST'), []);
    if (phase === 'pending') {
      resolveChapter({ ok: true, text: async () => '<html><head></head><body>Retired request</body></html>' });
      await opening;
      assert.equal(b.run('ebook'), null);
      assert.equal(b.nodes.get('#em-frame').srcdoc, '');
    }
  });
}

for (const stale of ['generation', 'frame']) {
  test(`closing a ready chapter with a retired ${stale} cannot save progress`, () => {
    const b = reader();
    if (stale === 'generation') b.run('ebookDocumentGeneration += 1');
    else b.nodes.get('#em-frame').contentWindow = { postMessage() {} };
    b.nodes.get('#em-close').onclick();
    assert.deepEqual(b.requests.filter(request => request.options?.method === 'POST'), []);
  });
}
