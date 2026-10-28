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
        add: name => classes.add(name), remove: name => classes.delete(name),
        contains: name => classes.has(name),
      },
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
    document, localStorage: {}, location: { hostname: 'test.invalid' },
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
    message: (data, overrides = {}) => listeners.get('message')({ data, origin: 'null', source: nodes.get('#em-frame').contentWindow, ...overrides }),
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
  b.run('ebook = { it: { id: 7 }, chunk: 1, spine: ["a", "b"], annots: [] }');
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
  assert.equal(b.run('ebook.annots.length'), 1);
  assert.equal(posted.length, 1);
  b.respond(async () => ({ text: async () => '<html><head></head><body>Hello world</body></html>' }));
  const timers = [];
  b.context.setTimeout = fn => timers.push(fn);
  await b.run('showChunk(0)');
  await b.run('showChunk(1)');
  for (const timer of timers) timer();
  assert.equal(posted.length, 2);
  assert.equal(posted[1].items[0].off, 0);
  assert.equal(posted[1].items[0].len, 5);
});

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
  vm.runInNewContext(script, {
    window: frame, document, parent: { postMessage: data => messages.push(data) },
    NodeFilter: { SHOW_TEXT: 4 }, Node: { TEXT_NODE: 3 },
  });
  return {
    b, messages, select: () => documentListeners.get('mouseup')(),
    painted: () => painted, selectionCleared: () => selectionCleared,
    inject: data => { for (const listener of frameListeners) listener({ data }); },
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
