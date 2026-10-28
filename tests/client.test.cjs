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
    const events = new Map();
    return {
      style: {}, dataset: {}, children: [], value: '', type: '', innerHTML: '', textContent: '',
      src: '', paused: true, readyState: 1, currentTime: 0, clientWidth: 760, clientHeight: 600,
      classList: {
        add: (...names) => names.forEach(name => classes.add(name)),
        remove: (...names) => names.forEach(name => classes.delete(name)),
        contains: name => classes.has(name),
      },
      querySelector: () => null,
      setAttribute: (name, value) => attributes.set(name, value),
      getAttribute: name => attributes.get(name),
      addEventListener(name, callback, options) {
        const handlers = events.get(name) || [];
        handlers.push({ callback, once: options?.once });
        events.set(name, handlers);
      },
      emit(name) {
        const handlers = events.get(name) || [];
        events.set(name, handlers.filter(handler => !handler.once));
        for (const handler of handlers) handler.callback();
        this['on' + name]?.();
      },
      appendChild(child) { this.children.push(child); },
      append(...children) { this.children.push(...children); },
      pause() { if (!this.paused) { this.paused = true; queueMicrotask(() => this.emit('pause')); } },
      play() { if (this.paused) { this.paused = false; queueMicrotask(() => this.emit('play')); } return Promise.resolve(); },
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
  for (const selector of ['#search input', '.avatar', '#hero-play', '#hero-backdrop', '#pt-host', '#pt-join', '#party-code-input', '#pt-play', '#pt-pause', '#pt-skip']) nodes.set(selector, element());
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
    document, localStorage: {}, URL, location: { hostname: 'test.invalid', origin: 'https://test.invalid' },
    console: { log() {}, warn() {} }, innerWidth: 1000, innerHeight: 800,
    fetch: async (url, options) => { requests.push({ url, options }); return response(url, options); },
    addEventListener: (name, fn) => listeners.set(name, fn),
    setTimeout() {}, clearTimeout() {},
    setInterval: fn => { const id = intervals.size + 1; intervals.set(id, fn); return id; },
    clearInterval: id => intervals.delete(id),
    Plyr: class {
      constructor() { this.events = element(); this.duration = 0; this.bindMedia(nodes.get('#plyr-video')); }
      bindMedia(media) {
        this.media = media;
        for (const name of ['play', 'pause', 'seeked', 'loadedmetadata', 'ready'])
          media.addEventListener(name, () => this.events.emit(name));
      }
      set source(value) {
        this.sourceValue = value;
        this.media.paused = true;
        const media = element();
        media.src = value.sources[0].src;
        nodes.set('#plyr-video', media);
        this.bindMedia(media);
        this.duration = 0;
      }
      set currentTime(value) { if (this.duration) this.media.currentTime = Math.min(value, this.duration); }
      get currentTime() { return this.media.currentTime; }
      get paused() { return this.media.paused; }
      on(name, callback) { this.events.addEventListener(name, callback); }
      once(name, callback) { this.events.addEventListener(name, callback, { once: true }); }
      emit(name) { this.media.emit(name); }
      play() { return this.media.play(); }
      pause() { this.media.pause(); }
      stop() { this.pause(); this.currentTime = 0; }
    }, Image: class {},
    WebSocket: class {
      constructor() { this.readyState = 1; this.sent = []; sockets.push(this); }
      send(message) { this.sent.push(JSON.parse(message)); }
    },
  });
  context.window = context;
  vm.runInContext(readFileSync(join(clientPath, 'js/app.js'), 'utf8'), context);
  return {
    context, nodes, requests, intervals, sockets,
    run: source => vm.runInContext(source, context),
    respond: fn => { response = fn; },
    message: (data, overrides = {}) => listeners.get('message')({ data: { generation: vm.runInContext('ebook?.document?.generation', context), ...data }, origin: 'null', source: nodes.get('#em-frame').contentWindow, ...overrides }),
    page: (page, pages) => listeners.get('message')({
      data: { type: 'page', page, pages, generation: vm.runInContext('ebook?.document?.generation', context) }, origin: 'null', source: nodes.get('#em-frame').contentWindow,
    }),
  };
}

function progress(browser) {
  return browser.requests.filter(r => r.url === '/api/progress').map(r => JSON.parse(r.options.body).position);
}

test('closing an ebook preserves the last reported fractional chapter position', () => {
  const b = browser();
  b.run('ebook = { it: { id: 7 }, chunk: 2, spine: ["a", "b", "c"], document: { generation: 1, ready: true } }');
  b.page(3, 5);
  b.nodes.get('#em-close').onclick();
  assert.deepEqual(progress(b), [2.5, 2.5]);
  assert.equal(b.nodes.get('#em-frame').src, 'about:blank');
});

test('closing before any page report saves a finite chapter position', () => {
  const b = browser();
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a"], document: { generation: 1, ready: true } }');
  b.nodes.get('#em-close').onclick();
  assert.deepEqual(progress(b), [0]);
});

test('changing chapters resets the previous fractional position before layout reports', async () => {
  const b = browser();
  b.respond(async () => ({ text: async () => '<html><head></head><body>Chapter</body></html>' }));
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a", "b"], document: { generation: 1, ready: true } }');
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
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a", "b"], document: { generation: 1, ready: true } }');
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
  b.run('ebook = { it: { id: 7 }, chunk: 0, spine: ["a", "b"], document: { generation: 1, ready: true } }');
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

/** Load the real client scripts and render a joined guest's request picker. */
async function guestPicker(library) {
  const b = browser();
  b.respond(async url => ({ ok: true, json: async () => url === '/api/library' ? library : null }));
  await b.run('loadLibrary()');
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
  b.nodes.get('#party-code-input').value = 'ABCD';
  b.run('PartyUI.open()');
  b.nodes.get('#pt-join').onclick();
  b.sockets[0].onmessage({ data: JSON.stringify({ type: 'state', is_host: false, state: { queue: [], position: 0 } }) });
  return b;
}

test('party guest picker finds a song after twenty comics', async () => {
  const library = Array.from({ length: 20 }, (_, id) => ({ id, kind: 'comic', title: 'Comic ' + id }));
  library.push({ id: 21, kind: 'music', title: 'Song' });
  const b = await guestPicker(library);
  const html = b.nodes.get('#party-stage').innerHTML;
  assert.match(html, /data-media="21"/);
  assert.doesNotMatch(html, /Comic/);
});

test('party guest picker preserves playable order, escapes titles and limits playable entries', async () => {
  const library = [
    { id: 1, kind: 'ebook', title: 'Book' },
    { id: 2, kind: 'manga', title: 'Manga' },
    { id: 3, kind: 'unknown', title: 'Unknown' },
    { id: 4, kind: 'episode', title: 'Unsupported episode' },
    { id: 5, kind: 'movie', title: '<Movie> & "title"' },
    { id: 6, kind: 'show', title: 'Show' },
    { id: 7, kind: 'music', title: 'Music' },
    { id: 8, kind: 'audiobook', title: 'Audiobook' },
    ...Array.from({ length: 18 }, (_, index) => ({ id: index + 9, kind: 'music', title: 'Track ' + index })),
  ];
  const b = await guestPicker(library);
  const html = b.nodes.get('#party-stage').innerHTML;
  assert.deepEqual([...html.matchAll(/data-media="(\d+)"/g)].map(match => Number(match[1])),
    [5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24]);
  assert.match(html, /data-title="&lt;Movie&gt; &amp; &quot;title&quot;"/);
  assert.doesNotMatch(html, /<Movie>/);
  b.run('partyFollow(7, "Music", 12, false, 0)');
  assert.equal(b.nodes.get('#pb-audio').src, '/api/stream/7');
  b.run('partyFollow(8, "Audiobook", 4, false, 0)');
  assert.equal(b.nodes.get('#pb-audio').src, '/api/stream/8');
  assert.equal(b.nodes.get('#plyr-video').src, '');
});

// Plyr's ready event precedes metadata; its seek setter ignores unknown duration.
test('movie reopen restores saved time after metadata, without undoing later seeks', async () => {
  const b = browser();
  let savedPosition = 203;
  b.respond(async () => ({ ok: true, json: async () => ({ position: savedPosition }) }));
  for (const [position, readyFirst] of [[203, true], [50, false]]) {
    savedPosition = position;
    b.run('openVideo({ id: 1, kind: "movie", title: "Movie" })');
    await new Promise(resolve => setImmediate(resolve));
    if (readyFirst) b.run('plyr.emit("ready")');
    assert.equal(b.run('plyr.currentTime'), 0);
    b.run('plyr.duration = 634; plyr.emit("loadedmetadata")');
    assert.equal(b.run('plyr.currentTime'), position);
    if (!readyFirst) b.run('plyr.emit("ready")');
    b.run('plyr.currentTime = 300; plyr.emit("loadedmetadata")');
    assert.equal(b.run('plyr.currentTime'), 300);
    b.nodes.get('#vm-back').onclick();
  }
});

test('retired video metadata cannot consume the current movie resume', async () => {
  const b = browser();
  const retired = b.nodes.get('#plyr-video');
  b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
  b.run('openVideo({ id: 1, kind: "movie", title: "Movie" })');
  await new Promise(resolve => setImmediate(resolve));
  retired.emit('loadedmetadata'); // Vendored Plyr forwards even a retired media event.
  assert.equal(b.run('plyr.currentTime'), 0);
  b.run('plyr.duration = 120; plyr.emit("loadedmetadata")');
  assert.equal(b.run('plyr.currentTime'), 64);
});

test('late movie metadata cannot seek a replacement or a closed video', async () => {
  const b = browser();
  b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
  b.run('openVideo({ id: 1, kind: "movie", title: "First" })');
  await new Promise(resolve => setImmediate(resolve));
  const first = b.nodes.get('#plyr-video');
  b.respond(async () => ({ ok: true, json: async () => null }));
  b.run('openVideo({ id: 2, kind: "movie", title: "Second" })');
  await new Promise(resolve => setImmediate(resolve));
  b.run('plyr.duration = 120');
  first.emit('loadedmetadata');
  assert.equal(b.run('plyr.currentTime'), 0);
  b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
  b.run('openVideo({ id: 3, kind: "movie", title: "Closed" })');
  await new Promise(resolve => setImmediate(resolve));
  b.nodes.get('#vm-back').onclick();
  b.run('plyr.duration = 120; plyr.emit("loadedmetadata")');
  assert.equal(b.run('plyr.currentTime'), 0);
});

for (const next of ['close', 'replacement']) {
  test(`a movie progress response after ${next} cannot load the old source`, async () => {
    const b = browser();
    let finish;
    b.respond(() => new Promise(resolve => { finish = resolve; }));
    b.run('openVideo({ id: 1, kind: "movie", title: "First" })');
    if (next === 'close') b.nodes.get('#vm-back').onclick();
    else {
      b.respond(async () => ({ ok: true, json: async () => null }));
      b.run('openVideo({ id: 2, kind: "movie", title: "Second" })');
      await new Promise(resolve => setImmediate(resolve));
    }
    const media = b.nodes.get('#plyr-video');
    finish({ ok: true, json: async () => ({ position: 64 }) });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(b.nodes.get('#plyr-video'), media);
    assert.equal(media.src, next === 'close' ? '' : '/api/stream/2');
  });
}

test('an old progress response cannot restart a quick same-item reopen', async () => {
  const b = browser();
  const pending = [];
  b.respond(() => new Promise(resolve => pending.push(resolve)));
  b.run('sameMovie = { id: 1, kind: "movie", title: "Movie" }; openVideo(sameMovie)');
  b.nodes.get('#vm-back').onclick();
  b.run('openVideo(sameMovie)');
  pending[1]({ ok: true, json: async () => null });
  await new Promise(resolve => setImmediate(resolve));
  const media = b.nodes.get('#plyr-video');
  pending[0]({ ok: true, json: async () => ({ position: 64 }) });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(b.nodes.get('#plyr-video'), media);
  b.run('plyr.duration = 120; plyr.emit("loadedmetadata")');
  assert.equal(b.run('plyr.currentTime'), 0);
});

test('metadata from a reused video with a different stream cannot apply old resume', async () => {
  const b = browser();
  b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
  b.run('openVideo({ id: 1, kind: "movie", title: "First" })');
  await new Promise(resolve => setImmediate(resolve));
  b.nodes.get('#plyr-video').src = '/api/stream/2';
  b.run('plyr.duration = 120; plyr.emit("loadedmetadata")');
  assert.equal(b.run('plyr.currentTime'), 0);
});

test('movie without meaningful saved progress starts at zero', async () => {
  for (const saved of [null, { position: 5 }]) {
    const b = browser();
    b.respond(async () => ({ ok: true, json: async () => saved }));
    b.run('openVideo({ id: 1, kind: "movie", title: "Movie" })');
    await new Promise(resolve => setImmediate(resolve));
    b.run('plyr.emit("ready"); plyr.duration = 634; plyr.emit("loadedmetadata")');
    assert.equal(b.run('plyr.currentTime'), 0);
  }
});

test('audio resume preserves saved time with available or delayed metadata', async () => {
  for (const readyState of [0, 1]) {
    const b = browser();
    const audio = b.nodes.get('#pb-audio');
    audio.readyState = readyState;
    b.respond(async () => ({ ok: true, json: async () => ({ position: 106 }) }));
    b.run('playAudio({ id: 2, kind: "music", title: "Music" })');
    await new Promise(resolve => setImmediate(resolve));
    if (!readyState) {
      assert.equal(audio.currentTime, 0);
      assert.equal(audio.paused, true);
      audio.readyState = 1;
      audio.emit('loadedmetadata');
    }
    assert.equal(audio.currentTime, 106);
    assert.equal(audio.paused, false);
    audio.currentTime = 120;
    audio.emit('loadedmetadata');
    assert.equal(audio.currentTime, 120);
  }
});


function highlightReader() {
  const b = browser();
  b.run('ebook = { it: { id: 7 }, chunk: 1, spine: ["a", "b"], annots: [], document: { generation: 1, ready: true, items: [] } }');
  const posted = [];
  b.nodes.get('#em-frame').contentWindow.postMessage = data => posted.push(data);
  return { b, posted };
}

const highlightMessage = { type: 'highlight', cfi: 'char:0:5', text: 'Hello' };
const settle = () => new Promise(resolve => setImmediate(resolve));

for (const [name, response, expected] of [
  ['guest denial', async () => ({ status: 401 }), 'Sign in to save highlights'],
  ['HTTP failure', async () => ({ status: 500 }), 'Could not save highlight. Try again.'],
  ['network failure', async () => { throw new Error('offline'); }, 'Could not save highlight. Try again.'],
]) {
  test(`highlight ${name} reports failure without saving state or painting`, async () => {
    const { b, posted } = highlightReader();
    b.respond(response);
    b.message(highlightMessage);
    await settle();
    assert.equal(b.nodes.get('#em-searchinfo').textContent, expected);
    assert.equal(b.run('ebook.annots.length'), 0);
    assert.deepEqual(posted, []);
  });
}

test('highlight waits for creation and retains saved state for chapter revisit without duplicate paint', async () => {
  const { b, posted } = highlightReader();
  let finish;
  b.respond(() => new Promise(resolve => { finish = resolve; }));
  b.message(highlightMessage);
  assert.equal(b.nodes.get('#em-searchinfo').textContent, 'Saving highlight…');
  assert.equal(b.run('ebook.annots.length'), 0);
  assert.deepEqual(posted, []);
  finish({ status: 201 });
  await settle();
  assert.equal(b.nodes.get('#em-searchinfo').textContent, 'Highlight saved');
  assert.equal(b.run('ebook.annots.length'), 1);
  assert.equal(posted.length, 1);
  assert.equal(posted[0].type, 'inject-hl');
  b.respond(async () => ({ status: 201 }));
  b.message(highlightMessage);
  await settle();
  assert.equal(b.requests.filter(request => request.url === '/api/annotations').length, 1);
  assert.equal(b.nodes.get('#em-searchinfo').textContent, 'Highlight saved');
  assert.equal(b.run('ebook.annots.length'), 1);
  assert.equal(posted.length, 1);
  b.respond(async () => ({ text: async () => '<html><head></head><body>Hello world</body></html>' }));
  await b.run('showChunk(0)');
  await b.run('showChunk(1)');
  assert.equal(posted.length, 1);
  b.message({ type: 'reader-ready' });
  assert.equal(posted.length, 3);
  assert.equal(posted[2].items[0].off, 0);
  assert.equal(posted[2].items[0].len, 5);
});

test('an already loaded highlight reports saved without posting or painting again', async () => {
  const { b, posted } = highlightReader();
  b.run('ebook.annots = [{ kind: "highlight", spine_idx: 1, cfi: "char:0:5" }]');
  b.nodes.get('#em-searchinfo').textContent = 'Search results';
  await b.message(highlightMessage);
  assert.deepEqual(b.requests, []);
  assert.deepEqual(posted, []);
  assert.equal(b.run('ebook.annots.length'), 1);
  assert.equal(b.nodes.get('#em-searchinfo').textContent, 'Highlight saved');
});

for (const [name, annotation] of [
  ['another chapter', { kind: 'highlight', spine_idx: 0, cfi: 'char:0:5' }],
  ['another range', { kind: 'highlight', spine_idx: 1, cfi: 'char:6:5' }],
  ['another kind', { kind: 'bookmark', spine_idx: 1, cfi: 'char:0:5' }],
]) {
  test(`an annotation for ${name} does not block a new highlight`, async () => {
    const { b, posted } = highlightReader();
    b.run(`ebook.annots = [${JSON.stringify(annotation)}]`);
    b.respond(async () => ({ status: 201 }));
    await b.message(highlightMessage);
    assert.equal(b.requests.length, 1);
    assert.equal(b.requests[0].options.method, 'POST');
    assert.equal(JSON.parse(b.requests[0].options.body).cfi, 'char:0:5');
    assert.equal(b.run('ebook.annots.length'), 2);
    assert.equal(posted.length, 1);
    assert.equal(b.nodes.get('#em-searchinfo').textContent, 'Highlight saved');
  });
}

test('pending highlight completion keeps captured book and chapter without touching a new reader', async () => {
  for (const status of [201, 401]) {
    const { b, posted } = highlightReader();
    let finish;
    b.respond(() => new Promise(resolve => { finish = resolve; }));
    b.message(highlightMessage);
    b.run('previousBook = ebook; ebook = { it: { id: 8 }, chunk: 0, spine: ["c"], annots: [] }');
    b.nodes.get('#em-searchinfo').textContent = 'New book';
    finish({ status });
    await settle();
    assert.equal(b.run('ebook.annots.length'), 0);
    assert.equal(b.nodes.get('#em-searchinfo').textContent, 'New book');
    assert.deepEqual(posted, []);
    assert.equal(b.run('previousBook.annots.length'), status === 201 ? 1 : 0);
    assert.equal(JSON.parse(b.requests[0].options.body).spine_idx, 1);
    assert.equal(JSON.parse(b.requests[0].options.body).media_id, 7);
  }
});


async function generatedSelection(options = {}) {
  const { b } = highlightReader();
  b.respond(async () => ({ text: async () => '<html><head></head><body>prefix Hello world</body></html>' }));
  await b.run('showChunk(1)');
  b.message({ type: 'reader-ready' });
  const generatedHtml = b.nodes.get('#em-frame').srcdoc;
  const scriptOpen = generatedHtml.indexOf('<script>');
  assert.ok(scriptOpen >= 0, 'generated reader script opening wrapper is missing');
  const scriptStart = scriptOpen + '<script>'.length;
  const scriptClose = generatedHtml.indexOf('</script>', scriptStart);
  assert.ok(scriptClose >= 0, 'generated reader script closing wrapper is missing');
  const script = generatedHtml.slice(scriptStart, scriptClose);
  const messages = [];
  const frameListeners = [];
  const documentListeners = new Map();
  const text = options.text ?? 'Hello';
  const textNode = { nodeType: options.nodeType ?? 3, nodeValue: 'prefix ' + text + ' world' };
  let painted = 0;
  let selectionCleared = false;
  const selection = {
    isCollapsed: false, rangeCount: 1,
    getRangeAt: () => ({
      startContainer: textNode, endContainer: options.crossNode ? { nodeType: 3 } : textNode,
      startOffset: 7, endOffset: 7 + text.length, toString: () => text,
    }),
    toString: () => text, removeAllRanges: () => { selectionCleared = true; },
  };
  const frame = {
    addEventListener: (name, fn) => { if (name === 'message') frameListeners.push(fn); },
    getSelection: () => selection, scrollX: 0,
  };
  const document = {
    body: {}, head: { appendChild() {} },
    addEventListener: (name, fn) => documentListeners.set(name, fn),
    createElement: () => ({ style: { setProperty() {} } }),
    createRange: () => ({
      selectNodeContents() {}, setStart() {}, setEnd() {}, toString: () => 'prefix ',
      surroundContents: () => { painted++; },
    }),
    createTreeWalker: () => {
      let visited = false;
      return { currentNode: textNode, nextNode: () => { if (visited) return false; visited = true; return true; } };
    },
  };
  const frameParent = { postMessage: data => messages.push(data) };
  vm.runInNewContext(script, {
    window: frame, document, parent: frameParent,
    NodeFilter: { SHOW_TEXT: 4 }, Node: { TEXT_NODE: 3 },
  });
  return {
    b, messages, select: () => documentListeners.get('mouseup')(),
    painted: () => painted, selectionCleared: () => selectionCleared,
    inject: data => { for (const listener of frameListeners) listener({ data: { ...data, generation: b.run('ebook.document.generation') }, source: frameParent }); },
  };
}

test('generated reader selection paints only after a saved highlight is injected', async () => {
  const reader = await generatedSelection();
  reader.select();
  assert.equal(reader.painted(), 0);
  assert.equal(reader.selectionCleared(), true);
  assert.equal(reader.messages[0].cfi, 'char:7:5');
  reader.inject({ type: 'inject-hl', items: [{ off: 7, len: 5, color: 'yellow' }] });
  assert.equal(reader.painted(), 1);
});

for (const [name, options] of [['cross-node', { crossNode: true }], ['element-node', { nodeType: 1 }]]) {
  test(`generated reader rejects ${name} selection before annotation save`, async () => {
    const reader = await generatedSelection(options);
    reader.select();
    assert.equal(reader.selectionCleared(), true);
    assert.equal(reader.painted(), 0);
    assert.equal(reader.messages[0].type, 'highlight-selection-error');
    await reader.b.message(reader.messages[0]);
    assert.equal(reader.b.requests.filter(request => request.url === '/api/annotations').length, 0);
    assert.equal(reader.b.nodes.get('#em-searchinfo').textContent, 'Select text without crossing formatting or paragraph breaks.');
  });
}

test('generated reader preserves exact leading whitespace text and range length', async () => {
  const reader = await generatedSelection({ text: '  Hello' });
  reader.select();
  assert.equal(reader.messages[0].cfi, 'char:7:7');
  assert.equal(reader.messages[0].text, '  Hello');
  reader.b.respond(async () => ({ status: 201 }));
  await reader.b.message(reader.messages[0]);
  const saved = JSON.parse(reader.b.requests.find(request => request.url === '/api/annotations').options.body);
  assert.equal(saved.cfi, 'char:7:7');
  assert.equal(saved.text, '  Hello');
  reader.inject({ type: 'inject-hl', items: [{ off: 7, len: 7, color: 'yellow' }] });
  assert.equal(reader.painted(), 1);
});

test('generated reader ignores whitespace-only selections', async () => {
  const reader = await generatedSelection({ text: '   ' });
  reader.select();
  assert.deepEqual(reader.messages, []);
  assert.equal(reader.painted(), 0);
});

test('highlight selection errors require the active opaque reader and use constant UI text', async () => {
  const { b } = highlightReader();
  const message = { type: 'highlight-selection-error', text: 'untrusted text' };
  await b.message(message, { source: {} });
  await b.message(message, { origin: 'https://attacker.invalid' });
  assert.equal(b.nodes.get('#em-searchinfo').textContent, '');
  await b.message(message);
  assert.equal(b.nodes.get('#em-searchinfo').textContent, 'Select text without crossing formatting or paragraph breaks.');
  assert.deepEqual(b.requests, []);
});


for (const [isHost, metadataFirst, priorFollowId] of [
  [true, true, null], [true, false, null], [false, true, null], [false, false, null],
  [true, true, 9], [true, false, 9],
]) {
  test(`${isHost ? 'host' : 'guest'} first party state ${metadataFirst ? 'after' : 'before'} metadata with prior follow ID ${priorFollowId} preserves the role's video position`, async () => {
    const b = browser();
    b.respond(async () => ({ ok: true, json: async () => ({ position: 203 }) }));
    b.run('LIBRARY = [{ id: 9, kind: "movie", title: "Movie" }]; openVideo(LIBRARY[0])');
    await settle();
    const media = b.nodes.get('#plyr-video');
    // Plyr installs a source child; the video src property can stay empty.
    media.querySelector = selector => selector === 'source' ? { src: '/api/stream/9' } : null;
    media.src = '';
    media.readyState = metadataFirst ? 1 : 0;
    if (metadataFirst) b.run('plyr.duration = 634; plyr.emit("loadedmetadata")');
    b.run(`partyFollowId = ${JSON.stringify(priorFollowId)}`);
    vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
    b.nodes.get('#party-code-input').value = 'ABCD';
    b.run('PartyUI.open()');
    b.nodes.get('#pt-join').onclick();
    b.sockets[0].onmessage({ data: JSON.stringify({ type: 'state', is_host: isHost,
      state: { media_id: 9, title: 'Movie', queue: [], position: isHost ? 0 : 24,
        playing: false, updated_at: Date.now() / 1000 } }) });
    if (!metadataFirst) {
      media.readyState = 1;
      b.run('plyr.duration = 634; plyr.emit("loadedmetadata")');
    }
    await settle();
    assert.equal(b.nodes.get('#plyr-video'), media);
    assert.equal(media.currentTime, isHost ? 203 : 24);
    assert.equal(media.paused, !isHost);
    if (isHost) assert.equal(media.src, '');
  });
}

for (const [name, src, child, currentSrc, preserve] of [
  ['same-origin direct source', 'https://test.invalid/api/stream/9', '', '', true],
  ['cross-origin suffix lookalike', 'https://other.invalid/api/stream/9', '', '', false],
  ['direct source before matching child', '/api/stream/10', '/api/stream/9', '', false],
  ['currentSrc fallback', '', '', 'https://test.invalid/api/stream/9', true],
]) {
  test(`host preservation checks the effective stream: ${name}`, () => {
    const b = browser();
    b.run('LIBRARY = [{ id: 9, kind: "movie" }]; partyPlaybackKind = "video"');
    b.nodes.get('#video-modal').classList.add('on');
    const media = b.nodes.get('#plyr-video');
    media.src = src;
    media.currentSrc = currentSrc;
    media.querySelector = selector => selector === 'source' && child ? { src: child } : null;
    media.currentTime = 203;
    b.run('partyFollow(9, "Movie", 0, false, 0, { preserveActiveVideo: true })');
    assert.equal(media.currentTime, preserve ? 203 : 0);
    assert.equal(media.src, preserve ? src : '/api/stream/9');
  });
}

for (const useHostHandler of [false, true]) {
  for (const inactiveState of ['audio', 'closed video', 'audio with video modal open']) {
    test(`${useHostHandler ? 'host state handler' : 'direct follow'} reactivates a retained video from ${inactiveState}`, async () => {
      const b = browser();
      b.respond(async () => ({ ok: true, json: async () => ({ position: 203 }) }));
      b.run('LIBRARY = [{ id: 9, kind: "movie" }, { id: 10, kind: "music" }]; openVideo(LIBRARY[0])');
      await settle();
      b.run('plyr.duration = 634; plyr.emit("loadedmetadata")');
      assert.equal(b.run('plyr.currentTime'), 203);
      b.nodes.get('#vm-back').onclick();
      await settle();
      let socket;
      if (useHostHandler) {
        vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
        b.nodes.get('#party-code-input').value = 'ABCD';
        b.run('PartyUI.open()');
        b.nodes.get('#pt-join').onclick();
        socket = b.sockets[0];
      }
      const follow = id => useHostHandler
        ? socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
          state: { media_id: id, title: 'Fixture', queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 } }) })
        : b.run(`partyFollow(${id}, "Fixture", 0, true, Date.now()/1000, { preserveActiveVideo: true })`);
      follow(10);
      await settle();
      if (inactiveState === 'closed video') b.run('partyPlaybackKind = "video"');
      if (inactiveState === 'audio with video modal open') b.nodes.get('#video-modal').classList.add('on');
      assert.equal(b.nodes.get('#pb-audio').paused, false);
      follow(9);
      await settle();
      assert.equal(b.nodes.get('#pb-audio').paused, true);
      assert.equal(b.nodes.get('#video-modal').classList.contains('on'), true);
      assert.equal(b.run('plyr.paused'), false);
      assert.ok(b.run('plyr.currentTime') < 2);
      assert.equal(b.run('partyPlaybackKind'), 'video');
    });
  }
}

test('host preserves an active paused video and its position', async () => {
  const b = browser();
  b.respond(async () => ({ ok: true, json: async () => ({ position: 203 }) }));
  b.run('LIBRARY = [{ id: 9, kind: "movie" }]; openVideo(LIBRARY[0])');
  await settle();
  b.run('plyr.duration = 634; plyr.emit("loadedmetadata"); plyr.pause()');
  b.run('partyFollow(9, "Movie", 0, true, Date.now()/1000, { preserveActiveVideo: true })');
  assert.equal(b.run('plyr.currentTime'), 203);
  assert.equal(b.run('plyr.paused'), true);
  assert.equal(b.nodes.get('#video-modal').classList.contains('on'), true);
});

test('host first party state still loads a different video', async () => {
  const b = browser();
  b.respond(async () => ({ ok: true, json: async () => ({ position: 203 }) }));
  b.run('LIBRARY = [{ id: 9, kind: "movie" }, { id: 10, kind: "movie" }]; openVideo(LIBRARY[0])');
  await settle();
  b.run('plyr.duration = 634; plyr.emit("loadedmetadata")');
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
  b.nodes.get('#party-code-input').value = 'ABCD';
  b.run('PartyUI.open()');
  b.nodes.get('#pt-join').onclick();
  b.sockets[0].onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
    state: { media_id: 10, title: 'Next', queue: [], position: 90, playing: true, updated_at: Date.now() / 1000 } }) });
  assert.equal(b.nodes.get('#plyr-video').src, '/api/stream/10');
  assert.ok(b.run('plyr.currentTime') < 2);
});

for (const audioWasLoaded of [false, true]) {
  test(`guest video to ${audioWasLoaded ? 'retained' : 'new'} audio activates only the audio player`, async () => {
    const b = browser();
    b.run('LIBRARY = [{ id: 9, kind: "movie" }, { id: 10, kind: "music" }]');
    vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
    b.nodes.get('#party-code-input').value = 'ABCD';
    b.run('PartyUI.open()');
    b.nodes.get('#pt-join').onclick();
    const socket = b.sockets[0];
    const follow = id => socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: false,
      state: { media_id: id, title: 'Fixture', queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 } }) });
    if (audioWasLoaded) follow(10);
    follow(9);
    await settle();
    assert.equal(b.run('plyr.paused'), false);
    b.nodes.get('#video-modal').classList.add('yt');
    b.nodes.get('#playerbar').classList.remove('on');
    follow(10);
    await settle();
    assert.equal(b.run('plyr.paused'), true);
    assert.equal(b.nodes.get('#pb-audio').paused, false);
    assert.equal(b.nodes.get('#video-modal').classList.contains('on'), false);
    assert.equal(b.nodes.get('#video-modal').classList.contains('yt'), false);
    assert.equal(b.nodes.get('#playerbar').classList.contains('on'), true);
  });
}

test('host video to audio cleanup cannot publish an obsolete video pause', async () => {
  const { b, socket, state } = await hostParty('movie');
  b.run('LIBRARY.push({ id: 10, kind: "music" })');
  socket.onmessage({ data: JSON.stringify({ type: 'state', state: { ...state, media_id: 10 } }) });
  await settle();
  assert.equal(b.run('plyr.paused'), true);
  assert.equal(b.nodes.get('#pb-audio').paused, false);
  assert.deepEqual(socket.sent.map(message => message.type), ['play']);
});

test('video party button renders and reopens the connected panel through PartyUI', () => {
  const b = browser();
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
  b.nodes.get('#vm-party').onclick();
  assert.equal(typeof b.nodes.get('#pt-join').onclick, 'function');
  assert.equal(b.nodes.get('#party-modal').style.zIndex, 200);
  b.nodes.get('#party-code-input').value = 'ABCD';
  b.nodes.get('#pt-join').onclick();
  b.sockets[0].onmessage({ data: JSON.stringify({ type: 'state', is_host: false,
    state: { media_id: null, title: '', queue: [], position: 0, playing: false } }) });
  b.run('PartyUI.hide()');
  b.nodes.get('#vm-party').onclick();
  assert.equal(b.sockets.length, 1);
  assert.match(b.nodes.get('#party-stage').innerHTML, /ABCD/);
  assert.equal(b.nodes.get('#party-modal').classList.contains('on'), true);
});

async function hostParty(kind = 'music') {
  const b = browser();
  b.run(`LIBRARY = [{ id: 9, kind: "${kind}", title: "Track" }]`);
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
  b.nodes.get('#party-code-input').value = 'ABCD';
  b.run('PartyUI.open()');
  b.nodes.get('#pt-join').onclick();
  const socket = b.sockets[0];
  const state = { media_id: 9, title: 'Track', queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 };
  socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true, state }) });
  await settle();
  socket.sent.length = 0;
  return { b, socket, state, media: b.nodes.get(kind === 'movie' ? '#plyr-video' : '#pb-audio') };
}

for (const kind of ['music', 'movie']) {
  test(`party panel controls actual ${kind} once, includes position, and ignores same-media echoes`, async () => {
    const { b, socket, state, media } = await hostParty(kind);
    media.currentTime = 47;
    b.nodes.get('#pt-pause').onclick();
    assert.equal(media.paused, true);
    assert.deepEqual(socket.sent, []); // Native events arrive asynchronously.
    await settle();
    assert.deepEqual(socket.sent, [{ type: 'pause', position: 47 }]);
    socket.onmessage({ data: JSON.stringify({ type: 'state', state: { ...state, playing: false, position: 47 } }) });
    await settle();
    assert.equal(media.currentTime, 47);
    assert.equal(socket.sent.length, 1);
    b.nodes.get('#pt-pause').onclick();
    await settle();
    assert.deepEqual(socket.sent[1], { type: 'pause', position: 47 });
    b.nodes.get('#pt-play').onclick();
    await settle();
    assert.equal(media.paused, false);
    assert.deepEqual(socket.sent[2], { type: 'play', position: 47 });
    assert.equal(socket.sent.length, 3);
    const inactive = b.nodes.get(kind === 'movie' ? '#pb-audio' : '#plyr-video');
    assert.equal(inactive.paused, true);
  });
}

test('native party controls include the active audio/video timeline position', async () => {
  for (const kind of ['music', 'movie']) {
    const { socket, media } = await hostParty(kind);
    media.currentTime = 61.5;
    media.pause();
    await settle();
    media.play();
    await settle();
    assert.deepEqual(socket.sent, [{ type: 'pause', position: 61.5 }, { type: 'play', position: 61.5 }]);
  }
});


test('party video follow and host hooks use replacement Plyr media', async () => {
  const { b, socket, media: oldMedia } = await hostParty('movie');
  b.run('plyr.source = { type: "video", sources: [{ src: "/api/stream/9" }] }');
  const media = b.nodes.get('#plyr-video');
  assert.notEqual(media, oldMedia);
  b.run('partyFollow(9, "Movie", 49, true, Date.now() / 1000)');
  await settle();
  assert.ok(media.currentTime >= 49 && media.currentTime < 50);
  socket.sent.length = 0;
  b.nodes.get('#pt-pause').onclick();
  await settle();
  assert.equal(media.paused, true);
  assert.equal(socket.sent.length, 1);
  assert.equal(socket.sent[0].type, 'pause');
  assert.equal(socket.sent[0].position, media.currentTime);
});

test('unready external party controls explain the failure without native fallback', async () => {
  const { b, socket, media } = await hostParty();
  media.currentTime = 47;
  await assert.rejects(b.run('partyControlPlayback("pause", { external_url: "abcdefghijk" })'), /YouTube player is not ready/);
  assert.equal(media.paused, false);
  assert.deepEqual(socket.sent, []);
});

test('party guest requests and host approval still use the queue protocol', async () => {
  const guest = browser();
  const request = { dataset: { media: '9', title: 'Requested track' } };
  guest.context.document.querySelectorAll = selector => selector === '.pt-request' ? [request] : [];
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), guest.context);
  guest.nodes.get('#party-code-input').value = 'ABCD';
  guest.run('PartyUI.open()');
  guest.nodes.get('#pt-join').onclick();
  const guestSocket = guest.sockets[0];
  guestSocket.onmessage({ data: JSON.stringify({ type: 'state', is_host: false, state: { queue: [], playing: false } }) });
  request.onclick();
  assert.deepEqual(guestSocket.sent, [{ type: 'request', media_id: 9, title: 'Requested track' }]);
  const { b, socket, state } = await hostParty();
  const approval = { dataset: { req: '0' } };
  b.context.document.querySelectorAll = selector => selector === '.pt-approve' ? [approval] : [];
  socket.onmessage({ data: JSON.stringify({ type: 'request', media_id: 9, title: 'Requested track', by: 'guest' }) });
  approval.onclick();
  assert.deepEqual(socket.sent, [{ type: 'approve', media_id: 9, title: 'Requested track', by: 'guest' }]);
  socket.onmessage({ data: JSON.stringify({ type: 'state', state: { ...state, queue: [{ title: 'Requested track', requested_by: 'guest' }] } }) });
  assert.match(b.nodes.get('#party-stage').innerHTML, /Requested track/);
});


test('inactive audio events cannot overwrite video party playback during a switch', async () => {
  const { b, socket, media: audio } = await hostParty();
  audio.currentTime = 47;
  b.run('LIBRARY.push({ id: 10, kind: "movie", title: "Movie" }); partyFollow(10, "Movie", 12, true, Date.now() / 1000)');
  await settle();
  assert.equal(audio.paused, true);
  assert.deepEqual(socket.sent.map(command => command.type), ['play']);
  assert.ok(socket.sent[0].position >= 12 && socket.sent[0].position < 13);
  audio.emit('play');
  audio.emit('pause');
  audio.emit('seeked');
  assert.equal(socket.sent.length, 1);
});

for (const kind of ['music', 'movie']) {
  for (const retained of [false, true]) {
    test(`external YouTube to ${retained ? 'retained' : 'fresh'} ${kind} pauses external playback and restores native presentation`, async () => {
      const b = browser();
      const sends = [];
      b.context.partySend = value => sends.push(value);
      b.context.external = { paused: false, pauses: 0 };
      b.run(`LIBRARY = [{ id: 9, kind: ${JSON.stringify(kind)} }];
        partyFollowId = ${retained ? 9 : 'null'};
        partyPlaybackKind = 'external';
        ytPlayerReady = true;
        ytPlayer = { pauseVideo() {
          external.paused = true; external.pauses++;
          if (partyPlaybackKind === 'external') partySend({ type: 'pause' });
        } };`);
      b.nodes.get('#video-modal').classList.add('on', 'yt');
      b.run('partyFollow(9, "Native", 24, true, Date.now()/1000, { preserveActiveVideo: true })');
      await settle();
      assert.equal(b.context.external.paused, true);
      assert.equal(b.context.external.pauses, 1);
      assert.deepEqual(sends, []);
      const modal = b.nodes.get('#video-modal');
      assert.equal(modal.classList.contains('yt'), false);
      assert.equal(modal.classList.contains('on'), kind === 'movie');
      const media = b.nodes.get(kind === 'movie' ? '#plyr-video' : '#pb-audio');
      assert.equal(media.paused, false);
      assert.ok(media.currentTime >= 24 && media.currentTime < 26);
    });
  }
}

function youtubePlayerFixture(b) {
  const players = [];
  b.context.YT = { Player: class {
    constructor(id, config) { this.config = config; this.calls = []; this.ready = false; players.push(this); }
    requireReady() { if (!this.ready) throw new Error('YouTube player is not ready'); }
    seekTo(position) { this.requireReady(); this.calls.push(['seek', position]); }
    playVideo() { this.requireReady(); this.calls.push(['play']); }
    pauseVideo() { this.requireReady(); this.calls.push(['pause']); }
    getCurrentTime() { this.requireReady(); return 0; }
    getPlayerState() { this.requireReady(); return 2; }
    destroy() {}
    emitReady() { this.ready = true; this.config.events.onReady({ target: this }); }
  } };
  b.run('ytReady = true');
  return players;
}

for (const kind of ['music', 'movie']) {
  test(`unready YouTube to native ${kind} completes and late readiness cannot restart external playback`, async () => {
    const b = browser();
    const players = youtubePlayerFixture(b);
    b.run(`LIBRARY = [{ id: 9, kind: ${JSON.stringify(kind)} }];
      partyFollowExternal('pending', 'External', 0, true, Date.now()/1000, false)`);
    let transitionError = null;
    try { b.run('partyFollow(9, "Native", 24, true, Date.now()/1000)'); }
    catch (error) { transitionError = error.message; }
    players[0].emitReady();
    await settle();
    assert.deepEqual({ transitionError, externalCalls: players[0].calls }, { transitionError: null, externalCalls: [] });
    const native = b.nodes.get(kind === 'movie' ? '#plyr-video' : '#pb-audio');
    assert.equal(native.paused, false);
    assert.ok(native.currentTime >= 24 && native.currentTime < 26);
    assert.equal(b.nodes.get('#video-modal').classList.contains('yt'), false);
  });
}

test('replacement YouTube readiness belongs to the current player only', () => {
  const b = browser();
  const players = youtubePlayerFixture(b);
  b.run('LIBRARY = [{ id: 9, kind: "movie" }]; partyFollowExternal("old", "Old", 0, true, Date.now()/1000, false)');
  players[0].emitReady();
  players[0].calls.length = 0;
  b.run('partyFollowExternal("new", "New", 0, true, Date.now()/1000, false)');
  assert.doesNotThrow(() => players[0].emitReady());
  assert.deepEqual(players[0].calls, []);
  assert.deepEqual(players[1].calls, []);
  assert.doesNotThrow(() => b.run('partyFollowExternal("new", "New", 0, true, Date.now()/1000, false)'));
  assert.doesNotThrow(() => b.run('partyFollow(9, "Native", 24, true, Date.now()/1000)'));
  players[1].emitReady();
  assert.deepEqual(players[1].calls, []);
});

test('unready external playback applies the latest same-video paused position on readiness', () => {
  const b = browser();
  const players = youtubePlayerFixture(b);
  b.run('partyFollowExternal("same", "First", 5, true, Date.now()/1000, false)');
  b.run('partyFollowExternal("same", "Latest", 20, false, Date.now()/1000, false)');
  players[0].emitReady();
  assert.deepEqual(players[0].calls, [['seek', 20]]);
});

test('late API readiness creates only the latest requested video and ignores native handoff', () => {
  for (const nativeHandoff of [false, true]) {
    const b = browser();
    const players = youtubePlayerFixture(b);
    b.run('ytReady = false; LIBRARY = [{ id: 9, kind: "music" }]');
    b.run('partyFollowExternal("obsolete", "Old", 5, true, Date.now()/1000, false)');
    b.run('partyFollowExternal("latest", "New", 20, false, Date.now()/1000, false)');
    if (nativeHandoff) b.run('partyFollow(9, "Native", 24, false, Date.now()/1000)');
    b.run('ytReady = true');
    for (const callback of [...b.intervals.values()]) callback();
    assert.deepEqual(players.map(player => player.config.videoId), nativeHandoff ? [] : ['latest']);
    if (!nativeHandoff) {
      players[0].emitReady();
      assert.deepEqual(players[0].calls, [['seek', 20]]);
    }
  }
});

for (const nativeKind of ['audio', 'video']) {
  test(`native ${nativeKind} to external playback pauses native players without a stale host command`, async () => {
    const b = browser();
    const players = youtubePlayerFixture(b);
    const sends = [];
    b.context.partySend = command => sends.push(command);
    b.run(`partyHostHook(partySend); partyPlaybackKind = ${JSON.stringify(nativeKind)};
      ${nativeKind === 'audio' ? 'pbAudio' : 'plyr'}.play()`);
    await settle();
    assert.equal(b.run(`${nativeKind === 'audio' ? 'pbAudio' : 'plyr'}.paused`), false);
    sends.length = 0;
    b.run('partyFollowExternal("external", "External", 20, false, Date.now()/1000, true)');
    players[0].emitReady();
    await settle();
    assert.equal(b.run('pbAudio.paused'), true);
    assert.equal(b.run('plyr.paused'), true);
    assert.deepEqual(sends, []);
    assert.deepEqual(players[0].calls, [['seek', 20]]);
    assert.equal(b.run('partyPlaybackKind'), 'external');
  });
}
