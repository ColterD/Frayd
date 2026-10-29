const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');

const clientPath = join(__dirname, '..', 'client');

const { browser } = require('./fixtures/client-browser.cjs');

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

test('toolbar scan refusal restores control without publishing completion', async () => {
  for (const admission of [202,409]) {
    const b=browser();
    b.respond(async url=>({status:admission,ok:true,json:async()=>url==='/api/scan/status' ? {scanning:false,scan_error:'Cache unavailable'} : []}));
    const button=b.nodes.get('#add-media');
    await button.onclick();
    await [...b.intervals.values()][0]();
    assert.equal(button.disabled,false);
    assert.equal(b.intervals.size,0);
    assert.equal(b.requests.some(request=>request.url==='/api/library'),false);
    const notices=b.context.document.body.children.map(node=>node.children[0]?.innerHTML||'').join(' ');
    assert.match(notices,/Scan failed/);
    assert.doesNotMatch(notices,/Scan complete/);
  }
});

test('party follow detects a video after each library replacement', async () => {
  const b = browser();
  b.respond(async url => ({ ok: true, json: async () => url === '/api/library' ? [{ id: 9, kind: 'movie', title: 'Movie' }] : null }));
  await b.run('loadLibrary()');
  b.run('partyFollow(9, "Movie", 12, false, 0)');
  assert.equal(b.nodes.get('#plyr-video').src, '/api/stream/9');
  assert.equal(b.nodes.get('#pb-audio').src, '');
  b.respond(async url => ({ ok: true, json: async () => url === '/api/library' ? [{ id: 10, kind: 'episode', title: 'Episode' }] : null }));
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
    { id: 4, kind: 'show', title: 'Unsupported legacy show' },
    { id: 5, kind: 'movie', title: '<Movie> & "title"' },
    { id: 6, kind: 'episode', title: 'Episode' },
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
  assert.doesNotMatch(html, /Unsupported legacy show/);
  b.run('partyFollow(6, "Episode", 6, false, 0)');
  assert.equal(b.nodes.get('#plyr-video').src, '/api/stream/6');
  b.run('partyFollow(7, "Music", 12, false, 0)');
  assert.equal(b.nodes.get('#pb-audio').src, '/api/stream/7');
  b.run('partyFollow(8, "Audiobook", 4, false, 0)');
  assert.equal(b.nodes.get('#pb-audio').src, '/api/stream/8');
  assert.equal(b.nodes.get('#plyr-video').src, '/api/stream/6');
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


for (const [isHost, metadataFirst] of [
  [true, true], [true, false], [false, true], [false, false],
]) {
  test(`${isHost ? 'host' : 'guest'} first party state ${metadataFirst ? 'after' : 'before'} metadata preserves the role's video position`, async () => {
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
  for (const inactiveState of ['audio', 'audio with video modal open']) {
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
        partyPlaybackKind = 'external';
        ytPlayerReady = true;
        ytPlayer = { pauseVideo() {
          external.paused = true; external.pauses++;
          if (partyPlaybackKind === 'external') partySend({ type: 'pause' });
        } };`);
      b.nodes.get('#video-modal').classList.add('on', 'yt');
      if (retained) b.nodes.get(kind === 'movie' ? '#plyr-video' : '#pb-audio').src = '/api/stream/9';
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

for (const entry of ['playAudio', 'openVideo']) {
  test(`local ${entry} after ready external playback pauses YouTube before native activation`, async () => {
    const b = browser();
    const players = youtubePlayerFixture(b);
    b.run('partyFollowExternal("external", "External", 5, true, Date.now()/1000, true)');
    players[0].emitReady();
    players[0].calls.length = 0;
    b.run(`${entry}({ id: 9, title: 'Local native', kind: '${entry === 'playAudio' ? 'music' : 'movie'}' })`);
    await settle();
    assert.deepEqual(players[0].calls, [['pause']]);
    assert.equal(b.run('partyPlaybackKind'), entry === 'playAudio' ? 'audio' : 'video');
    assert.equal(b.run(entry === 'playAudio' ? 'pbAudio.paused' : 'plyr.paused'), false);
    assert.equal(b.nodes.get('#video-modal').classList.contains('yt'), false);
  });
}

for (const kind of ['audio', 'video']) {
  test(`queued ${kind} pause after rapid external round trip cannot pause the resumed party`, async () => {
    const b = browser();
    youtubePlayerFixture(b);
    const sends = [];
    b.context.partySend = command => sends.push(command);
    b.run(`LIBRARY = [{ id: 9, kind: '${kind === 'audio' ? 'music' : 'movie'}' }];
      partyFollow(9, 'Native', 24, true, Date.now()/1000); partyHostHook(partySend)`);
    await settle();
    sends.length = 0;
    b.run(`partyFollowExternal('external', 'External', 0, false, Date.now()/1000, true);
      partyFollow(9, 'Native', 24, true, Date.now()/1000)`);
    await settle();
    assert.equal(b.run(`${kind === 'audio' ? 'pbAudio' : 'plyr'}.paused`), false);
    assert.deepEqual(sends.filter(command => command.type === 'pause'), []);
    sends.length = 0;
    b.run(`${kind === 'audio' ? 'pbAudio' : 'plyr'}.pause()`);
    await settle();
    assert.equal(sends.filter(command => command.type === 'pause').length, 1);
  });
}

for (const kind of ['audio', 'video']) for (const returnToNative of [false, true]) {
  test(`pending ${kind} party metadata cannot restart after external handoff${returnToNative ? ' and paused native return' : ''}`, async () => {
    const b = browser();
    youtubePlayerFixture(b);
    b.run(`LIBRARY = [{ id: 9, kind: '${kind === 'audio' ? 'music' : 'movie'}' }]`);
    const media = b.nodes.get(kind === 'audio' ? '#pb-audio' : '#plyr-video');
    media.readyState = 0;
    b.run(`partyFollow(9, 'Pending native', 24, true, Date.now()/1000);
      partyFollowExternal('external', 'External', 0, false, Date.now()/1000, false)`);
    if (returnToNative) b.run("partyFollow(9, 'Paused return', 42, false, Date.now()/1000)");
    media.readyState = 1;
    media.emit('loadedmetadata');
    await settle();
    assert.equal(media.paused, true);
    assert.equal(media.currentTime, returnToNative ? 42 : 0);
  });
}

for (const entry of ['playAudio', 'openVideo']) for (const phase of ['progress', 'metadata']) {
  test(`local ${entry} pending ${phase} cannot apply after external handoff`, async () => {
    const b = browser();
    youtubePlayerFixture(b);
    let release;
    b.respond(() => new Promise(resolve => { release = resolve; }));
    b.nodes.get('#pb-audio').readyState = 0;
    b.run(`${entry}({ id: 9, title: 'Pending local', kind: '${entry === 'playAudio' ? 'music' : 'movie'}' })`);
    if (phase === 'metadata') {
      release({ ok: true, json: async () => ({ position: 106 }) });
      await settle();
    }
    const media = b.nodes.get(entry === 'playAudio' ? '#pb-audio' : '#plyr-video');
    b.run("partyFollowExternal('external', 'External', 0, false, Date.now()/1000, false)");
    if (phase === 'progress') release({ ok: true, json: async () => ({ position: 106 }) });
    await settle();
    media.readyState = 1;
    b.run('plyr.duration = 634');
    media.emit('loadedmetadata');
    await settle();
    assert.equal(b.nodes.get(entry === 'playAudio' ? '#pb-audio' : '#plyr-video'), media);
    assert.equal(media.paused, true);
    assert.equal(media.currentTime, 0);
  });
}

for (const transition of ['close', 'local audio']) {
  test(`cached video follow after ${transition} reactivates presentation and progress ownership without reloading`, async () => {
    const b = browser();
    b.run('LIBRARY = [{ id: 9, kind: "movie", title: "Cached movie" }, { id: 10, kind: "music", title: "Local audio" }]; openVideo(LIBRARY[0])');
    await settle();
    b.run('partyFollow(9, "Cached movie", 12, true, Date.now()/1000)');
    const media = b.nodes.get('#plyr-video');
    if (transition === 'close') b.nodes.get('#vm-back').onclick();
    else { b.run('playAudio(LIBRARY[1])'); await settle(); }
    let sourceAssignments = 0;
    const source = media.src;
    Object.defineProperty(media, 'src', { configurable: true, get: () => source,
      set() { sourceAssignments++; } });
    b.run('partyFollow(9, "Cached movie", 24, true, Date.now()/1000)');
    await settle();
    assert.equal(b.nodes.get('#video-modal').classList.contains('on'), true);
    assert.equal(b.nodes.get('#pb-audio').paused, true);
    assert.equal(media.paused, false);
    assert.equal(b.run('videoItem.id'), 9);
    assert.equal(sourceAssignments, 0);
    b.requests.length = 0;
    media.pause();
    await settle();
    const saves = b.requests.filter(r => r.url === '/api/progress').map(r => JSON.parse(r.options.body));
    assert.ok(saves.some(save => save.media_id === 9 && save.position >= 24 && save.position < 26));
  });
}

/** Exercise role authority through the production party socket handler while keeping
 * the YouTube boundary controlled and exposing only the current seek timer tick. */
function externalRoleSession() {
  const b = browser();
  const players = youtubePlayerFixture(b);
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
  function joinSession(isHost) {
    b.run('PartyUI.open()');
    b.nodes.get('#party-code-input').value = 'ABCD';
    b.nodes.get('#pt-join').onclick();
    const socket = b.sockets.at(-1);
    socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: isHost,
      state: { external_url: 'same-role-video', media_id: null, title: 'Same external video',
        queue: [], position: 0, playing: false, updated_at: Date.now()/1000 } }) });
    return socket;
  }
  function emit(player, state) {
    player.getCurrentTime = () => 37;
    player.getPlayerState = () => state;
    player.config.events.onStateChange({ target: player, data: state });
  }
  function seekTick() { b.intervals.get(b.context._ytSeekTimer)?.(); }
  return { b, players, join: joinSession, emit, seekTick };
}

test('guest-created YouTube player follows the current host sender for play pause and seek without duplication', () => {
  const f = externalRoleSession();
  const guest = f.join(false);
  const player = f.players[0];
  player.emitReady();
  f.emit(player, 1);
  f.seekTick();
  assert.deepEqual(guest.sent, []);
  f.b.run('PartyUI.close()');
  const host = f.join(true);
  host.sent.length = 0;
  f.emit(player, 1);
  f.emit(player, 2);
  f.emit(player, 1);
  f.seekTick();
  assert.deepEqual(host.sent.map(command => command.type), ['play', 'pause', 'play', 'seek']);
  assert.ok(host.sent.every(command => command.position === 37));
  assert.equal(f.players.length, 1);
  assert.equal(f.b.intervals.size, 1);
  f.b.run('partyFollowExternal("same-role-video", "Repeated host state", 37, false, Date.now()/1000, false)');
  host.sent.length = 0;
  f.emit(player, 1);
  f.seekTick();
  assert.deepEqual(host.sent.map(command => command.type), ['play', 'seek']);
  assert.equal(f.players.length, 1);
  assert.equal(f.b.intervals.size, 1);
});

test('reused YouTube callbacks cannot send through a departed host or a guest session', () => {
  const f = externalRoleSession();
  const host = f.join(true);
  const player = f.players[0];
  player.emitReady();
  host.sent.length = 0;
  f.b.run('PartyUI.close()');
  f.emit(player, 1);
  f.emit(player, 2);
  f.seekTick();
  assert.deepEqual(host.sent, []);
  const guest = f.join(false);
  f.emit(player, 1);
  f.emit(player, 2);
  f.seekTick();
  assert.deepEqual(guest.sent, []);
  assert.deepEqual(host.sent, []);
  assert.equal(f.players.length, 1);
  assert.ok(f.b.intervals.size <= 1);
});

for (const kind of ['video', 'audio']) {
  test(`party follow replaces the current local ${kind} stream and records its own progress`, async () => {
    const b = browser();
    const entry = kind === 'video' ? 'openVideo' : 'playAudio';
    b.run(`LIBRARY = [{ id: 9, kind: '${kind === 'video' ? 'movie' : 'music'}', title: 'Party nine' },
      { id: 10, kind: '${kind === 'video' ? 'movie' : 'music'}', title: 'Local ten' }];
      partyFollow(9, 'Party nine', 12, true, Date.now()/1000); ${entry}(LIBRARY[1])`);
    await settle();
    const media = b.nodes.get(kind === 'video' ? '#plyr-video' : '#pb-audio');
    media.currentTime = 203;
    assert.equal(media.src, '/api/stream/10');
    b.run("partyFollow(9, 'Party nine', 24, true, Date.now()/1000)");
    await settle();
    b.requests.length = 0;
    media.pause();
    await settle();
    const saves = b.requests.filter(r => r.url === '/api/progress').map(r => JSON.parse(r.options.body));
    if (kind === 'video') {
      assert.equal(b.run('videoItem.id'), 9);
      assert.ok(!saves.some(save => save.media_id === 9 && save.position === 203), JSON.stringify(saves));
      assert.ok(saves.some(save => save.media_id === 9 && save.position >= 24 && save.position < 26));
    }
    if (kind === 'audio') assert.ok(!saves.some(save => save.media_id === 10), JSON.stringify(saves));
    assert.equal(media.src, '/api/stream/9');
    assert.ok(media.currentTime >= 24 && media.currentTime < 26);
  });
}


test('native audio progress preserves pause and ended UI property handlers', async () => {
  const b = browser();
  b.run('uiEnded = 0; pbAudio.onended = () => ++uiEnded');
  b.run("playAudio({ id: 10, kind: 'music', title: 'Audio UI' })");
  await settle();
  const media = b.nodes.get('#pb-audio');
  const button = b.nodes.get('#pb-play');
  assert.equal(button.textContent, '❚❚');
  media.currentTime = 4;
  media.duration = 90;
  b.requests.length = 0;
  media.pause();
  await settle();
  assert.equal(button.textContent, '▶');
  assert.deepEqual(progress(b), [4]);
  media.emit('ended');
  assert.equal(b.run('uiEnded'), 1);
  assert.deepEqual(progress(b), [4, 90]);
  await media.play();
  await settle();
  assert.equal(button.textContent, '❚❚');
});

test('native audio listener replacement removes exact handlers and preserves unrelated listeners', async () => {
  const b = browser();
  b.run('unrelatedPauses = 0; pbAudio.addEventListener("pause", () => ++unrelatedPauses)');
  b.run("playAudio({ id: 10, kind: 'music', title: 'Audio listener' })");
  await settle();
  const media = b.nodes.get('#pb-audio');
  b.run('trackProgress(pbAudio, { id: 10 })');
  assert.equal(media.listenerCount('pause'), 2);
  assert.equal(media.listenerCount('ended'), 1);
  b.run("openVideo({ id: 11, kind: 'movie', title: 'Video successor' })");
  await settle();
  assert.equal(media.listenerCount('pause'), 1);
  assert.equal(media.listenerCount('ended'), 0);
  assert.equal(b.run('unrelatedPauses'), 1);
  const timer = b.run('progressTimer');
  b.requests.length = 0;
  media.emit('ended');
  assert.deepEqual(progress(b), []);
  assert.equal(b.intervals.has(timer), true);
});

test('current local audio saves periodic pause and ended progress', async () => {
  const b = browser();
  b.run("playAudio({ id: 10, kind: 'audiobook', title: 'Current local' })");
  await settle();
  const media = b.nodes.get('#pb-audio');
  b.requests.length = 0;
  media.currentTime = 12;
  b.intervals.get(b.run('progressTimer'))();
  media.currentTime = 18;
  media.pause();
  await settle();
  media.duration = 90;
  media.emit('ended');
  assert.deepEqual(progress(b), [12, 18, 90]);
  assert.equal(b.intervals.has(b.run('progressTimer')), false);
});

test('local Plyr pause and ended events save progress before a periodic tick', async () => {
  const b = browser();
  b.run("openVideo({ id: 10, kind: 'movie', title: 'Local video' })");
  await settle();
  const media = b.run('plyr.media');
  media.currentTime = 4;
  b.run('plyr.duration = 90');
  b.requests.length = 0;
  b.run('plyr.pause()');
  await settle();
  assert.deepEqual(progress(b), [4]);
  media.emit('ended');
  assert.deepEqual(progress(b), [4, 90]);
});

test('same-ID Plyr tracker replacement removes its exact old listeners and keeps one current timer', async () => {
  const b = browser();
  b.run("openVideo({ id: 10, kind: 'movie', title: 'Same video' })");
  await settle();
  const media = b.run('plyr.media');
  const staleTick = b.intervals.get(b.run('progressTimer'));
  b.run('trackProgress(plyr, { id: 10 })');
  const timer = b.run('progressTimer');
  assert.equal(b.captureCount('pause'), 1);
  assert.equal(b.captureCount('ended'), 1);
  media.currentTime = 4;
  b.run('plyr.duration = 90');
  b.requests.length = 0;
  staleTick();
  assert.deepEqual(progress(b), []);
  assert.equal(b.intervals.has(timer), true);
  b.run('plyr.pause()');
  await settle();
  assert.deepEqual(progress(b), [4]);
  media.emit('ended');
  assert.deepEqual(progress(b), [4, 90]);
  assert.equal(b.intervals.has(timer), false);
});

test('retired native video events cannot save successor progress or clear its timer', async () => {
  const b = browser();
  b.run("openVideo({ id: 10, kind: 'movie', title: 'Previous' })");
  await settle();
  const previous = b.run('plyr.media');
  b.run("openVideo({ id: 10, kind: 'movie', title: 'Successor same ID' })");
  await settle();
  const current = b.run('plyr.media');
  const timer = b.run('progressTimer');
  b.requests.length = 0;
  current.currentTime = 24;
  current.duration = 90;
  previous.emit('pause');
  previous.emit('ended');
  assert.deepEqual(progress(b), []);
  assert.equal(b.intervals.has(timer), true);
  assert.equal(b.captureCount('pause'), 1);
  assert.equal(b.captureCount('ended'), 1);
  // A queued pause delivered after the current media resumes also has no authority.
  current.emit('pause');
  assert.deepEqual(progress(b), []);
  current.pause();
  await settle();
  assert.deepEqual(progress(b), [24]);
});

test('switching from a Plyr tracker to audio removes only progress event handlers', async () => {
  const b = browser();
  b.run('otherPauses = 0; document.addEventListener("pause", event => { if (event.target === plyr.media) ++otherPauses; }, true)');
  b.run("openVideo({ id: 10, kind: 'movie', title: 'Previous' })");
  await settle();
  const media = b.run('plyr.media');
  b.run("playAudio({ id: 11, kind: 'music', title: 'Current' })");
  await settle();
  const timer = b.run('progressTimer');
  assert.equal(b.captureCount('pause'), 1, 'unrelated pause listener remains');
  assert.equal(b.captureCount('ended'), 0);
  b.requests.length = 0;
  b.run('plyr.pause()');
  await settle();
  media.emit('ended');
  assert.equal(b.run('otherPauses'), 1);
  assert.deepEqual(progress(b), []);
  assert.equal(b.intervals.has(timer), true);
});

for (const kind of ['audio', 'video']) {
  for (const queuedPause of [false, true]) {
    test(`${kind} host close preserves one native pause and progress at 41 (queued=${queuedPause})`, async () => {
      const { b, socket, media } = await hostParty(kind === 'audio' ? 'music' : 'movie');
      if (kind === 'audio') b.run('trackProgress(pbAudio, { id: 9 })');
      if (kind === 'video') b.run('plyr.duration = 120');
      media.currentTime = 41;
      b.requests.length = 0;
      if (queuedPause) media.pause();
      b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
      await settle();
      assert.deepEqual(socket.sent, [{ type: 'pause', position: 41 }]);
      assert.ok(progress(b).length > 0);
      assert.ok(progress(b).every(position => position === 41), JSON.stringify(progress(b)));
      assert.equal(media.paused, true);
    });
  }

  test(`${kind} paused same-ID pending resume close preserves stored 64 without a zero save`, async () => {
    const b = browser();
    const entry = kind === 'audio' ? 'playAudio' : 'openVideo';
    const item = { id: 10, kind: kind === 'audio' ? 'music' : 'movie', title: 'Same' };
    b.run(`${entry}(${JSON.stringify(item)})`);
    await settle();
    const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
    media.pause();
    await settle();
    media.currentTime = 0;
    let release;
    b.respond(url => url.startsWith('/api/progress/10?')
      ? new Promise(resolve => { release = () => resolve({ ok: true, json: async () => ({ position: 64 }) }); })
      : Promise.resolve({ ok: true, json: async () => null }));
    b.run(`${entry}(${JSON.stringify(item)})`);
    b.requests.length = 0;
    b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
    release();
    await settle();
    media.emit('loadedmetadata');
    assert.deepEqual(progress(b), []);
    assert.equal(media.paused, true);
    assert.equal(media.currentTime, 0);
  });

  test(`${kind} close after external ownership sends no native pause`, async () => {
    const { b, socket } = await hostParty(kind === 'audio' ? 'music' : 'movie');
    youtubePlayerFixture(b);
    b.run("partyFollowExternal('external', 'External', 0, false, Date.now()/1000, true)");
    await settle();
    socket.sent.length = 0;
    b.requests.length = 0;
    b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
    await settle();
    assert.deepEqual(socket.sent, []);
    assert.deepEqual(progress(b), []);
  });

  test(`${kind} external handoff retires same-stream progress`, async () => {
    const b = browser();
    youtubePlayerFixture(b);
    const item = { id: 10, kind: kind === 'audio' ? 'music' : 'movie', title: 'Local' };
    b.run(`${kind === 'audio' ? 'playAudio' : 'openVideo'}(${JSON.stringify(item)})`);
    await settle();
    const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
    media.currentTime = 24;
    media.duration = 90;
    const stalePause = () => media.emit('pause');
    const staleEnded = () => media.emit('ended');
    b.requests.length = 0;
    b.run("partyFollowExternal('same', 'External', 0, true, Date.now()/1000, false)");
    await settle();
    b.intervals.get(b.run('progressTimer'))();
    stalePause();
    staleEnded();
    assert.deepEqual(progress(b), []);
  });

  test(`${kind} close retires pending resume and metadata without losing current close progress`, async () => {
    const b = browser();
    let release;
    b.respond(url => url.startsWith('/api/progress/10?')
      ? new Promise(resolve => { release = () => resolve({ ok: true, json: async () => ({ position: 37 }) }); })
      : Promise.resolve({ ok: true, json: async () => null }));
    const item = { id: 10, kind: kind === 'audio' ? 'music' : 'movie', title: 'Local' };
    b.run(`${kind === 'audio' ? 'playAudio' : 'openVideo'}(${JSON.stringify(item)})`);
    const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
    b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
    release();
    await settle();
    media.emit('loadedmetadata');
    assert.equal(media.paused, true);
    assert.equal(media.src, '');
    assert.equal(b.nodes.get(kind === 'audio' ? '#playerbar' : '#video-modal').classList.contains('on'), false);

    b.respond(async () => ({ ok: true, json: async () => null }));
    b.run(`${kind === 'audio' ? 'playAudio' : 'openVideo'}(${JSON.stringify(item)})`);
    await settle();
    const current = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
    current.currentTime = 21;
    b.requests.length = 0;
    b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
    await settle();
    assert.ok(progress(b).includes(21), JSON.stringify(progress(b)));
  });

  test(`${kind} close clears a local host load before delayed metadata can broadcast`, async () => {
    const b = browser();
    b.run('commands = []; window.partySend = command => commands.push(command); partyHostHook(window.partySend)');
    b.run(`${kind === 'audio' ? 'playAudio' : 'openVideo'}({ id: 10, kind: '${kind === 'audio' ? 'music' : 'movie'}', title: 'Host' })`);
    await settle();
    const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
    media.readyState = 0;
    b.run('commands.length = 0');
    b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
    await settle();
    media.readyState = 1;
    media.emit('loadedmetadata');
    await settle();
    assert.equal(b.run('commands.filter(command => command.type === "load").length'), 0);
    assert.equal(media.paused, true);
    assert.equal(b.run(kind === 'audio' ? 'partyLoadPending' : 'videoLoadPending'), false);
  });

  test(`${kind} close blocks delayed party metadata for the same stream`, async () => {
    const b = browser();
    const item = { id: 10, kind: kind === 'audio' ? 'music' : 'movie', title: 'Party' };
    b.run(`LIBRARY = [${JSON.stringify(item)}]`);
    const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
    media.readyState = 0;
    b.run("partyFollow(10, 'Party', 37, true, Date.now()/1000)");
    b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
    media.readyState = 1;
    media.emit('loadedmetadata');
    await settle();
    assert.equal(media.paused, true);
    assert.equal(media.currentTime, 0);
    assert.equal(b.nodes.get(kind === 'audio' ? '#playerbar' : '#video-modal').classList.contains('on'), false);
  });
}

for (const kind of ['audio', 'video']) {
  test(`inactive ${kind === 'audio' ? 'video' : 'audio'} close preserves pending ${kind} resume and progress`, async () => {
    const b = browser();
    let release;
    b.respond(url => url.startsWith('/api/progress/10?')
      ? new Promise(resolve => { release = () => resolve({ ok: true, json: async () => ({ position: 64 }) }); })
      : Promise.resolve({ ok: true, json: async () => null }));
    b.run(`${kind === 'audio' ? 'playAudio' : 'openVideo'}({ id: 10, kind: '${kind === 'audio' ? 'music' : 'movie'}', title: 'Pending' })`);
    const timer = b.run('progressTimer');
    // Escape invokes the video close handler even when its modal is hidden.
    b.nodes.get(kind === 'audio' ? '#vm-back' : '#pb-close').onclick();
    release();
    await settle();
    const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
    if (kind === 'video') b.run('plyr.duration = 120');
    media.emit('loadedmetadata');
    await settle();
    assert.equal(media.src, '/api/stream/10');
    assert.equal(media.currentTime, 64);
    assert.equal(media.paused, false);
    assert.equal(b.intervals.has(timer), true);
    b.requests.length = 0;
    media.currentTime = 74;
    b.intervals.get(timer)();
    assert.deepEqual(progress(b), [74]);
  });
}

test('audio progress rejects periodic pause and ended callbacks after its stream changes', async () => {
  const b = browser();
  b.run("playAudio({ id: 10, kind: 'audiobook', title: 'Previous local' })");
  await settle();
  const media = b.nodes.get('#pb-audio');
  const tick = b.intervals.get(b.run('progressTimer'));
  media.src = '/api/stream/9';
  media.currentTime = 24;
  media.duration = 90;
  b.requests.length = 0;
  tick();
  media.pause();
  await settle();
  media.emit('ended');
  assert.deepEqual(progress(b), []);
});

test('retired same-ID audio tracker leaves one current event listener and cannot clear its timer', async () => {
  const b = browser();
  b.run("playAudio({ id: 10, kind: 'audiobook', title: 'Local' })");
  await settle();
  const media = b.nodes.get('#pb-audio');
  const retiredTick = b.intervals.get(b.run('progressTimer'));
  b.run('trackProgress(pbAudio, { id: 10 })');
  const timer = b.run('progressTimer');
  assert.equal(media.listenerCount('pause'), 1);
  assert.equal(media.listenerCount('ended'), 1);
  media.currentTime = 24;
  media.duration = 90;
  b.requests.length = 0;
  retiredTick();
  assert.deepEqual(progress(b), []);
  assert.equal(b.intervals.has(timer), true);
  media.pause();
  await settle();
  assert.deepEqual(progress(b), [24]);
  assert.equal(b.intervals.has(timer), true);
  media.emit('ended');
  assert.deepEqual(progress(b), [24, 90]);
  assert.equal(b.intervals.has(timer), false);
});

test('local audio progress waits for metadata from the current stream', async () => {
  const b = browser();
  b.run("playAudio({ id: 10, kind: 'audiobook', title: 'Current local' })");
  await settle();
  const media = b.nodes.get('#pb-audio');
  media.readyState = 0;
  media.currentTime = 24;
  media.duration = NaN;
  b.requests.length = 0;
  b.intervals.get(b.run('progressTimer'))();
  media.emit('pause');
  media.emit('ended');
  assert.deepEqual(progress(b), []);
});

test('interval allocation preserves live callbacks after an earlier timer is cleared', () => {
  const b = browser();
  b.run('globalThis.timerCalls=[];globalThis.firstTimer=setInterval(()=>timerCalls.push("first"));globalThis.liveTimer=setInterval(()=>timerCalls.push("live"));clearInterval(firstTimer);globalThis.newTimer=setInterval(()=>timerCalls.push("new"))');
  assert.equal(b.intervals.size, 2);
  assert.ok(b.context.newTimer > b.context.liveTimer);
  for (const callback of b.intervals.values()) callback();
  assert.equal(b.run('JSON.stringify(timerCalls)'), '["live","new"]');
});

test('toolbar scan refresh survives opening and closing Settings', async () => {
  const b = browser();
  b.respond(async url => ({ status: 202, ok: true, json: async () => url === '/api/scan/status' ? { scanning: false } : url === '/api/settings' ? {} : [] }));
  await b.nodes.get('#add-media').onclick();
  const poll = [...b.intervals.values()][0];
  await b.run('openSettings()');
  b.nodes.get('#settings-close').emit('click');
  await poll();
  assert.equal(b.requests.filter(r => r.url === '/api/library').length, 1);
  assert.equal(b.nodes.get('#add-media').disabled, false);
  assert.equal(b.intervals.size, 0);
});

for (const phase of ['start', 'status', 'library']) {
  for (const rejected of [false, true]) {
    test(`toolbar scan ignores stale ${phase} ${rejected ? 'failure' : 'success'} after logout and a new account`, async () => {
      const b = browser();
      b.context.location.reload = () => {};
      let release, reject;
      b.respond(url => {
        if (url === ({ start: '/api/scan', status: '/api/scan/status', library: '/api/library' })[phase]) {
          return new Promise((resolve, fail) => { release = resolve; reject = fail; });
        }
        return Promise.resolve({ status: 202, ok: true, json: async () => ({ scanning: false }) });
      });
      let pending = b.nodes.get('#add-media').onclick();
      if (phase !== 'start') {
        await pending;
        pending = [...b.intervals.values()][0]();
      }
      // Let the scan reach the selected asynchronous boundary.
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(typeof release, 'function');
      b.nodes.get('#logout').emit('click');
      b.respond(async () => ({ ok: true, json: async () => [] }));
      b.run('account={name:"Next",isAdmin:false};enterApp()');
      await new Promise(resolve => setImmediate(resolve));
      const button = b.nodes.get('#add-media');
      button.disabled = true;
      button.innerHTML = 'new account scan';
      const toastCount = b.context.document.body.children.length;
      const requestCount = b.requests.length;
      if (rejected) reject(Error('old request failed'));
      else release({ status: 202, ok: true, json: async () => phase === 'library' ? [{ id: 9, kind: 'movie' }] : { scanning: false } });
      await pending;
      assert.equal(button.disabled, true);
      assert.equal(button.innerHTML, 'new account scan');
      assert.equal(b.context.document.body.children.length, toastCount);
      assert.equal(b.requests.length, requestCount);
      assert.equal(b.intervals.size, 0);
      assert.equal(b.run('LIBRARY.length'), 0);
    });
  }
}

for (const failure of ['network', 'http', 'json']) {
  test(`toolbar scan retries one ${failure} status failure then refreshes on completion`, async () => {
    const b = browser();
    let statusCalls = 0;
    b.respond(async url => {
      if (url === '/api/scan/status') {
        if (++statusCalls === 1) {
          if (failure === 'network') throw Error('temporary network failure');
          return { ok: failure !== 'http', json: async () => { throw Error('temporary invalid response'); } };
        }
        return { ok: true, json: async () => ({ scanning: false }) };
      }
      return { status: 202, ok: true, json: async () => [] };
    });
    const button = b.nodes.get('#add-media');
    await button.onclick();
    const poll = [...b.intervals.values()][0];
    await poll();
    assert.equal(b.intervals.size, 1);
    assert.equal(button.disabled, true);
    await poll();
    assert.equal(statusCalls, 2);
    assert.equal(b.requests.filter(r => r.url === '/api/library').length, 1);
    assert.equal(button.disabled, false);
    assert.equal(b.intervals.size, 0);
  });
}

test('toolbar scan bounds consecutive failures and retry reattaches to a busy scan', async () => {
  const b = browser();
  b.respond(async url => {
    if (url === '/api/scan/status') throw Error('offline');
    return { status: 202 };
  });
  const button = b.nodes.get('#add-media');
  await button.onclick();
  const poll = [...b.intervals.values()][0];
  for (let attempt = 0; attempt < 3; ++attempt) await poll();
  assert.equal(b.intervals.size, 0);
  assert.equal(button.disabled, false);
  assert.equal(b.requests.filter(r => r.url === '/api/scan/status').length, 3);
  const notices = b.context.document.body.children.map(node => node.children[0]?.innerHTML || '').join(' ');
  assert.match(notices, /retry/i);
  b.respond(async url => ({ status: url === '/api/scan' ? 409 : 200, ok: true, json: async () => url === '/api/scan/status' ? { scanning: false } : [] }));
  await button.onclick();
  assert.equal(button.disabled, true);
  assert.equal(b.intervals.size, 1);
  await [...b.intervals.values()][0]();
  assert.equal(b.requests.filter(r => r.url === '/api/library').length, 1);
  assert.equal(button.disabled, false);
  assert.equal(b.intervals.size, 0);
});

test('toolbar scan never overlaps pending status or completion refresh requests', async () => {
  const b = browser();
  let release;
  b.respond(url => url === '/api/scan' ? Promise.resolve({ status: 202 }) : new Promise(resolve => { release = resolve; }));
  await b.nodes.get('#add-media').onclick();
  const poll = [...b.intervals.values()][0];
  const pending = poll();
  const duplicate = poll();
  assert.equal(b.requests.filter(r => r.url === '/api/scan/status').length, 1);
  await duplicate;
  release({ ok: true, json: async () => ({ scanning: false }) });
  await new Promise(resolve => setImmediate(resolve));
  await poll();
  assert.equal(b.requests.filter(r => r.url === '/api/library').length, 1);
  assert.equal(b.requests.filter(r => r.url === '/api/scan/status').length, 1);
  release({ ok: true, json: async () => [] });
  await pending;
  assert.equal(b.intervals.size, 0);
});

for (const phase of ['resume', 'metadata', 'playing']) {
  test(`host audio first echo preserves local ${phase} ownership`, async () => {
    const b = browser();
    b.run('LIBRARY=[{id:23,kind:"music",title:"Local audio"}]');
    let release;
    b.respond(url => url.startsWith('/api/progress/23?')
      ? new Promise(resolve => { release = () => resolve({ok:true,json:async()=>({position:64})}); })
      : Promise.resolve({ok:true,json:async()=>null}));
    const media = b.nodes.get('#pb-audio');
    media.readyState = 0;
    b.run('playAudio(LIBRARY[0])');
    if (phase !== 'resume') { release(); await settle(); }
    if (phase === 'playing') { media.readyState = 1; media.emit('loadedmetadata'); await settle(); media.currentTime = 41; }
    vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
    b.nodes.get('#party-code-input').value = 'ABCD';
    b.run('PartyUI.open()');
    b.nodes.get('#pt-join').onclick();
    b.sockets[0].onmessage({data:JSON.stringify({type:'state',is_host:true,
      state:{media_id:23,title:'Local audio',queue:[],position:0,playing:true,updated_at:Date.now()/1000}})});
    if (phase === 'resume') { release(); await settle(); }
    if (phase !== 'playing') { media.readyState = 1; media.emit('loadedmetadata'); await settle(); }
    assert.equal(media.currentTime, phase === 'playing' ? 41 : 64);
    assert.equal(media.paused, false);
    assert.match(media.src, /stream\/23$/);
  });
}

test('audio host changed source and guest same source still follow the received clock', async () => {
  for (const host of [false, true]) {
    const b = browser();
    b.run('LIBRARY=[{id:23,kind:"music",title:"Old"},{id:24,kind:"music",title:"New"}]; playAudio(LIBRARY[0])');
    await settle();
    b.nodes.get('#pb-audio').currentTime = 64;
    b.run(`partyFollow(${host ? 24 : 23},"Follow",17,false,Date.now()/1000,{preserveActiveVideo:${host}})`);
    assert.equal(b.nodes.get('#pb-audio').currentTime, 17);
    assert.equal(b.nodes.get('#pb-audio').paused, true);
    assert.match(b.nodes.get('#pb-audio').src, host ? /stream\/24$/ : /stream\/23$/);
  }
});

for (const kind of ['audio', 'video']) {
  for (const scenario of ['own closed echo', 'guest closed follow', 'host changed media', 'host inactive owner']) {
    test(`${kind} first party state after local close: ${scenario}`, async () => {
      const b = browser();
      const entry = kind === 'audio' ? 'playAudio' : 'openVideo';
      const other = kind === 'audio' ? 'openVideo' : 'playAudio';
      const mediaKind = kind === 'audio' ? 'music' : 'movie';
      b.respond(async () => ({ok:true,json:async()=>({position:64})}));
      b.run(`LIBRARY=[{id:9,kind:"${mediaKind}",title:"Local"},
        {id:10,kind:"${mediaKind}",title:"Changed"},
        {id:11,kind:"${kind === 'audio' ? 'movie' : 'music'}",title:"Other owner"}];
        ${entry}(LIBRARY[0])`);
      await settle();
      const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
      if (kind === 'video') b.run('plyr.duration=120;plyr.emit("loadedmetadata")');
      media.duration = 120;
      media.currentTime = 41;
      b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
      await settle();
      assert.equal(media.paused, true, 'local close pauses the native player');
      const panel = b.nodes.get(kind === 'audio' ? '#playerbar' : '#video-modal');
      assert.equal(panel.classList.contains('on'), false);
      if (scenario === 'host inactive owner') {
        b.run(`${other}(LIBRARY[2])`);
        await settle();
        if (kind === 'audio') b.run('plyr.duration=120;plyr.emit("loadedmetadata")');
      }
      vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
      b.nodes.get('#party-code-input').value = 'ABCD';
      b.run('PartyUI.open()');
      b.nodes.get('#pt-join').onclick();
      const isHost = scenario !== 'guest closed follow';
      const id = scenario === 'host changed media' ? 10 : 9;
      b.sockets[0].onmessage({data:JSON.stringify({type:'state',is_host:isHost,
        state:{media_id:id,title:'First load echo',queue:[],position:isHost ? 0 : 24,
          playing:true,updated_at:Date.now()/1000}})});
      await settle();
      if (scenario === 'own closed echo') {
        assert.equal(panel.classList.contains('on'), false, 'late own echo cannot reopen locally closed UI');
        assert.equal(media.paused, true, 'late own echo cannot restart locally closed playback');
        assert.equal(media.currentTime, 41, 'late own echo cannot reset close position');
      } else {
        assert.equal(panel.classList.contains('on'), true, 'authoritative follow activates target UI');
        assert.equal(media.paused, false, 'authoritative follow plays the target');
        assert.ok(media.currentTime >= (isHost ? 0 : 24) && media.currentTime < (isHost ? 2 : 26));
        assert.equal(b.run(`hasNativeStream(${kind === 'audio' ? 'pbAudio' : 'plyr.media'},${id})`), true);
        assert.equal(b.run('partyPlaybackKind'), kind);
      }
    });
  }
}

async function hostedNativeBrowser(kind) {
  const clock = { milliseconds: 1700000000000 };
  const b = browser({ now: () => clock.milliseconds });
  await settle();
  vm.runInContext(readFileSync(join(clientPath, 'js/party.js'), 'utf8'), b.context);
  b.run(`LIBRARY = [{id:9,kind:"music",title:"Previous"},
    {id:10,kind:"${kind === 'audio' ? 'music' : 'movie'}",title:"Next"}]`);
  b.nodes.get('#party-code-input').value = 'ABCD';
  b.run('PartyUI.open()');
  b.nodes.get('#pt-join').onclick();
  const socket = b.sockets[0];
  socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
    state: { media_id: 9, title: 'Previous', queue: [], position: 17, playing: true, updated_at: b.run('Date.now()/1000') } }) });
  await settle();
  socket.sent.length = 0;
  b.nodes.get('#pb-audio').readyState = 0;
  return { b, socket, clock };
}

for (const kind of ['audio', 'video']) {
  for (const saved of [64, 5, null]) {
    test(`${kind} first host wire load applies saved ${saved} before publishing controls`, async () => {
      const { b, socket } = await hostedNativeBrowser(kind);
      b.respond(async () => ({ ok: true, json: async () => saved === null ? null : { position: saved } }));
      b.run(`${kind === 'audio' ? 'playAudio' : 'openVideo'}(LIBRARY[1])`);
      await settle();
      assert.deepEqual(socket.sent, [], 'A pending local load cannot publish controls against the prior party media');
      const media = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
      media.readyState = 1;
      if (kind === 'video') b.run('plyr.duration = 120');
      media.emit('loadedmetadata');
      await settle();
      const expected = saved > 5 ? saved : 0;
      assert.deepEqual(socket.sent[0], { type: 'load', media_id: 10, title: 'Next', position: expected });
      assert.equal(socket.sent.filter(command => command.type === 'load').length, 1);
      assert.equal(media.currentTime, expected);
      // The own first state must preserve the native owner and its applied resume.
      socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
        state: { media_id: 10, title: 'Next', queue: [], position: expected, playing: true, updated_at: b.run('Date.now()/1000') } }) });
      assert.equal(media.currentTime, expected);
      media.currentTime = 73;
      media.emit('loadedmetadata');
      await settle();
      assert.equal(media.currentTime, 73, 'Repeated metadata cannot reapply the saved position');
      assert.equal(socket.sent.filter(command => command.type === 'load').length, 1);
    });
  }
}


for (const kind of ['audio', 'video']) {
  for (const retire of ['close', 'foreign stream', 'external handoff', 'new local load']) {
    test(`${kind} pending host load ignores metadata after ${retire}`, async () => {
      const { b, socket } = await hostedNativeBrowser(kind);
      b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
      const entry = kind === 'audio' ? 'playAudio' : 'openVideo';
      b.run(`${entry}(LIBRARY[1])`);
      await settle();
      const retired = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
      if (retire === 'close') b.nodes.get(kind === 'audio' ? '#pb-close' : '#vm-back').onclick();
      else if (retire === 'foreign stream') retired.src = 'https://other.invalid/api/stream/10';
      else if (retire === 'external handoff') {
        youtubePlayerFixture(b);
        b.run('partyFollowExternal("abcdefghijk", "External", 0, false, Date.now()/1000)');
      }
      else {
        b.run(`LIBRARY.push({id:11,kind:"${kind === 'audio' ? 'music' : 'movie'}",title:"Successor"}); ${entry}(LIBRARY[2])`);
        await settle();
      }
      retired.readyState = 1;
      if (kind === 'video') b.run('plyr.duration = 120');
      retired.emit('loadedmetadata');
      await settle();
      assert.equal(socket.sent.filter(command => command.type === 'load' && command.media_id === 10).length, 0);
      if (retire === 'new local load') {
        const current = b.run(kind === 'audio' ? 'pbAudio' : 'plyr.media');
        current.readyState = 1;
        current.emit('loadedmetadata');
        await settle();
        assert.deepEqual(socket.sent.filter(command => command.type === 'load'),
          [{ type: 'load', media_id: 11, title: 'Successor', position: 64 }]);
      } else assert.equal(socket.sent.filter(command => command.type === 'load').length, 0);
    });
  }
}

test('metadata-ready audio publishes its resumed host load once before play', async () => {
  const { b, socket } = await hostedNativeBrowser('audio');
  b.nodes.get('#pb-audio').readyState = 1;
  b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
  b.run('playAudio(LIBRARY[1])');
  await settle();
  assert.deepEqual(socket.sent[0], { type: 'load', media_id: 10, title: 'Next', position: 64 });
  assert.ok(socket.sent.slice(1).every(command => command.type !== 'load' && command.position === 64));
  b.nodes.get('#pb-audio').emit('loadedmetadata');
  await settle();
  assert.equal(socket.sent.filter(command => command.type === 'load').length, 1);
});

for (const mediaId of [10, 11]) {
  for (const playing of [false, true]) {
    test(`party video ${mediaId === 10 ? 'same stream' : 'changed stream'} takeover retires pending local load before ${playing ? 'autoplay' : 'host Play'}`, async () => {
      const { b, socket, clock } = await hostedNativeBrowser('video');
      youtubePlayerFixture(b);
      b.run('LIBRARY.push({id:11,kind:"movie",title:"Takeover"})');
      b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
      b.run('openVideo(LIBRARY[1])');
      await settle();
      const media = b.run('plyr.media');
      assert.equal(b.run('videoLoadPending'), true);
      const state = (value) => socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
        state: { title: 'Party takeover', queue: [], position: 0, playing: false, updated_at: b.run('Date.now()/1000'), ...value } }) });
      state({ media_id: null, external_url: 'abcdefghijk' });
      await settle();
      assert.equal(b.run('partyPlaybackKind'), 'external');
      assert.equal(media.paused, true);
      socket.sent.length = 0;
      state({ media_id: mediaId, playing });
      await settle();
      assert.equal(b.run('partyPlaybackKind'), 'video');
      assert.equal(b.run(`hasNativeStream(plyr.media, ${mediaId})`), true);
      if (!playing) b.nodes.get('#pt-play').onclick();
      await settle();
      assert.equal(media.paused, false);
      assert.deepEqual(socket.sent.filter(command => command.type === 'play'), [{ type: 'play', position: 0 }],
        'The current party-owned video must publish its native play event');
      assert.equal(b.run('videoLoadPending'), false);
      media.readyState = 1;
      b.run('plyr.duration = 120');
      media.emit('loadedmetadata');
      await settle();
      assert.equal(media.currentTime, 0, 'Retired local metadata cannot restore the old saved64');
      assert.equal(socket.sent.filter(command => command.type === 'load').length, 0,
        'Party takeover cannot publish the retired local load');
      const updatedAt = b.run('Date.now()/1000');
      clock.milliseconds += 1000;
      socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: false,
        state: { media_id: mediaId, title: 'Guest clock projection', queue: [], position: 23,
          playing: true, updated_at: updatedAt } }) });
      await settle();
      assert.equal(media.currentTime, 24, 'Guest follow still projects an explicitly advanced clock');
    });
  }
}

test('own video echo preserves the pending local resume gate until its saved64 load', async () => {
  const { b, socket } = await hostedNativeBrowser('video');
  b.respond(async () => ({ ok: true, json: async () => ({ position: 64 }) }));
  b.run('openVideo(LIBRARY[1])');
  await settle();
  const media = b.run('plyr.media');
  media.readyState = 0;
  socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
    state: { media_id: 10, title: 'Own pending echo', queue: [], position: 0, playing: true, updated_at: b.run('Date.now()/1000') } }) });
  await settle();
  assert.equal(b.run('videoLoadPending'), true, 'A matching local echo must retain local resume authority');
  assert.deepEqual(socket.sent, [], 'A matching echo cannot publish initial play0');
  media.readyState = 1;
  b.run('plyr.duration = 120');
  media.emit('loadedmetadata');
  await settle();
  assert.equal(media.currentTime, 64);
  assert.deepEqual(socket.sent.filter(command => command.type === 'load'),
    [{ type: 'load', media_id: 10, title: 'Next', position: 64 }]);
  assert.equal(b.run('videoLoadPending'), false);
});

test('host audio zero echo preserves106 through progress tick and native pause saves', async t => {
  const { b, socket, clock } = await hostedNativeBrowser('audio');
  b.respond(async () => ({ ok: true, json: async () => ({ position: 106 }) }));
  const audio = b.nodes.get('#pb-audio');
  audio.readyState = 1;
  b.run('playAudio(LIBRARY[1])');
  await settle();
  assert.equal(audio.currentTime, 106);
  assert.equal(audio.paused, false);
  assert.equal(b.run('hasNativeStream(pbAudio, 10)'), true);
  const tick = b.intervals.get(b.run('progressTimer'));
  assert.equal(typeof tick, 'function');
  b.requests.length = 0;
  socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
    state: { media_id: 10, title: 'Own zero echo', queue: [], position: 0,
      playing: true, updated_at: b.run('Date.now()/1000') } }) });
  await settle();
  const positionAfterEcho = audio.currentTime;
  clock.milliseconds += 1000;
  tick();
  await settle();
  const tickPositions = progress(b);
  const beforePause = b.requests.length;
  audio.pause();
  await settle();
  const pausePosts = b.requests.slice(beforePause).filter(request => request.url === '/api/progress')
    .map(request => JSON.parse(request.options.body));
  const posts = b.requests.filter(request => request.url === '/api/progress')
    .map(request => JSON.parse(request.options.body));
  t.diagnostic(JSON.stringify({ positionAfterEcho, tickPositions, pausePositions: pausePosts.map(post => post.position),
    savedMediaIds: posts.map(post => post.media_id), allPositions: posts.map(post => post.position) }));
  assert.deepEqual(tickPositions, [106], 'The active interval must actually save the preserved resume position');
  assert.deepEqual(pausePosts.map(post => post.position), [106], 'The native pause must actually save that same position');
  assert.equal(posts.every(post => post.media_id === 10 && post.position >= 106), true,
    'Every post-echo save belongs to the current media and none rewinds below resume');
  assert.equal(positionAfterEcho, 106);
  assert.equal(audio.currentTime, 106);
  assert.equal(audio.paused, true);
});

// A logout reload must choose the sign-in layout before any LAN auto-resume.
for (const mode of ['guest', 'account']) {
  test(`${mode} sign-out reload shows only sign-in and permits explicit re-entry`, async () => {
    const storage = { frayd_identity: 'owned-logout-fixture' };
    if (mode === 'account') storage.frayd_token = 'nonfunctional-fixture-token';
    const initialResponse = async url => ({ ok: true, status: 200, json: async () => {
      if (url === '/api/auth/login') return { name: 'Fixture', token: 'nonfunctional-fixture-token' };
      if (url === '/api/auth/me') return { name: 'Fixture', is_admin: false };
      return [];
    } });
    const signedIn = browser({ storage, initialResponse });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(signedIn.nodes.get('#app').classList.contains('on'), true);
    let reloaded = false;
    signedIn.context.location.reload = () => { reloaded = true; };
    signedIn.nodes.get('#logout').emit('click');
    assert.equal(reloaded, true);
    assert.equal(storage.frayd_token, undefined);
    assert.equal(storage.frayd_logout, '1');
    assert.equal(storage.frayd_identity, 'owned-logout-fixture');

    const signedOut = browser({ storage, initialResponse });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(signedOut.nodes.get('#app').classList.contains('on'), false, 'Dashboard stays hidden after explicit sign-out');
    assert.equal(signedOut.context.document.body.classList.contains('auth'), true);
    assert.equal(signedOut.nodes.get('#login').style.display, 'flex');
    assert.equal(signedOut.requests.length, 0, 'Explicit sign-out does not resume or load the library');
    const refreshedSignedOut = browser({ storage, initialResponse });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(refreshedSignedOut.nodes.get('#app').classList.contains('on'), false, 'Refreshing sign-in must not resume the guest session');
    assert.equal(refreshedSignedOut.context.document.body.classList.contains('auth'), true);
    assert.equal(refreshedSignedOut.nodes.get('#login').style.display, 'flex');
    assert.equal(refreshedSignedOut.requests.length, 0);
    assert.equal(storage.frayd_logout, '1', 'Sign-out persists until explicit re-entry');

    if (mode === 'guest') refreshedSignedOut.nodes.get('#guest').emit('click');
    else {
      refreshedSignedOut.nodes.get('#email').value = 'fixture@example.invalid';
      refreshedSignedOut.nodes.get('#password').value = 'nonfunctional-fixture-password';
      await refreshedSignedOut.run('doLogin({ preventDefault() {} })');
    }
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(refreshedSignedOut.context.document.body.classList.contains('auth'), false);
    assert.equal(refreshedSignedOut.nodes.get('#login').style.display, 'none');
    assert.equal(refreshedSignedOut.nodes.get('#app').classList.contains('on'), true);
    assert.equal(refreshedSignedOut.run('account !== null'), mode === 'account');
    assert.equal(storage.frayd_logout, undefined, 'Explicit re-entry clears the sign-out marker');
    assert.equal(refreshedSignedOut.requests.filter(request => new URL(request.url, 'https://fixture.invalid').pathname === '/api/library').length, 1);
  });
}

for (const boundary of ['pages', 'resume']) {
  for (const replacement of ['new comic', 'close']) {
    test(`comic ${boundary} response cannot reopen or overwrite progress after ${replacement}`, async () => {
      const b = browser();
      let release;
      b.respond(async url => {
        if (url === (boundary === 'pages' ? '/api/comic/1/pages' : '/api/progress/1?identity=undefined')) {
          return new Promise(resolve => { release = resolve; });
        }
        return { ok: true, json: async () => url.includes('/pages') ? ['0', '1', '2', '3', '4'] : { position: 1 } };
      });
      const old = b.run('openComic({ id: 1, title: "Old comic" })');
      await settle();
      assert.equal(typeof release, 'function');
      if (replacement === 'new comic') {
        await b.run('openComic({ id: 2, title: "New comic" })');
        await settle();
        assert.equal(b.nodes.get('#cm-img').src, '/api/comic/2/page/1', 'Current comic restores its own saved page');
      } else b.nodes.get('#cm-close').onclick();
      const image = b.nodes.get('#cm-img').src;
      const title = b.nodes.get('#cm-title').textContent;
      b.requests.length = 0;
      release({ ok: true, json: async () => boundary === 'pages' ? ['0', '1', '2', '3', '4'] : { position: 4 } });
      await old;
      await settle();
      assert.equal(b.nodes.get('#cm-img').src, image);
      assert.equal(b.nodes.get('#cm-title').textContent, title);
      assert.equal(b.nodes.get('#comic-modal').classList.contains('on'), replacement === 'new comic');
      assert.deepEqual(b.requests.filter(request => request.url === '/api/progress'), [], 'Retired response cannot write either comic progress');
      if (replacement === 'new comic') {
        b.nodes.get('#cm-next').onclick();
        assert.equal(b.nodes.get('#cm-img').src, '/api/comic/2/page/2', 'Current reader navigation still works');
        assert.equal(JSON.parse(b.requests.at(-1).options.body).media_id, 2);
      }
    });
  }
}

for (const mode of ['guest', 'account']) {
  for (const operation of ['loadLibrary()', 'serverSearch("needle & ?")', 'tryResume()']) {
    test(`${mode} ${operation} sends the current library identity contract`, async () => {
      const b = browser();
      await settle();
      b.run('IDENTITY = "owned library identity & ?"');
      if (mode === 'account') b.context.localStorage.frayd_token = 'nonfunctional-fixture-token';
      b.respond(async url => ({ ok: true, json: async () => url.startsWith('/api/auth/me') ? { name: 'Fixture' } : [] }));
      await b.run(operation);
      await settle();
      const requests = b.requests.filter(request => new URL(request.url, 'https://fixture.invalid').pathname === '/api/library');
      assert.ok(requests.length > 0);
      for (const request of requests) {
        const url = new URL(request.url, 'https://fixture.invalid');
        assert.equal(url.searchParams.get('identity'), 'owned library identity & ?');
        assert.equal(request.options?.headers?.Authorization, mode === 'account' ? 'Bearer nonfunctional-fixture-token' : undefined);
        if (operation.startsWith('serverSearch')) assert.equal(url.searchParams.get('q'), 'needle & ?');
      }
    });
  }
}

for (const operation of ['loadLibrary()', 'serverSearch("old")', 'tryResume()']) {
  test(`${operation} cannot publish after the account changes`, async () => {
    const b = browser();
    await settle();
    b.run('IDENTITY = "old-fixture"');
    let release;
    b.respond(() => new Promise(resolve => { release = resolve; }));
    const pending = b.run(operation);
    b.run('++accountGeneration; IDENTITY = "new-fixture"; account = { name: "New account" }; LIBRARY = [{ id: 2, kind: "music" }]');
    b.nodes.get('#main').innerHTML = 'new account view';
    release({ ok: true, json: async () => [] });
    await pending;
    await settle();
    assert.equal(b.run('LIBRARY[0]?.id'), 2);
    assert.equal(b.run('account.name'), 'New account');
    assert.equal(b.nodes.get('#main').innerHTML, 'new account view');
    assert.equal(b.nodes.get('#app').classList.contains('on'), false, 'Old resume cannot enter the new account');
  });
}

test('an obsolete boot resume cannot cover a newly entered account with sign-in', async () => {
  let release;
  const storage = { frayd_identity: 'old-fixture' };
  const b = browser({ storage, initialResponse: () => new Promise(resolve => { release = resolve; }) });
  b.respond(async () => ({ ok: true, json: async () => [] }));
  b.run('IDENTITY = "new-fixture"; account = { name: "New account" }; enterApp()');
  release({ ok: true, json: async () => [] });
  await settle();
  assert.equal(b.context.document.body.classList.contains('auth'), false);
  assert.equal(b.nodes.get('#login').style.display, 'none');
  assert.equal(b.run('account.name'), 'New account');
});

test('episode renders as video, appears in library rows and opens the video player', async () => {
  const b = browser();
  b.respond(async () => ({ ok: true, json: async () => null }));
  b.run('LIBRARY = [{ id: 12, kind: "episode", title: "Episode" }]; filtering = true');
  await b.run('renderRows(LIBRARY)');
  assert.match(b.nodes.get('#hero').innerHTML, /▶ Play/);
  const section = b.nodes.get('#main').children.at(-1);
  assert.match(section.children[0].innerHTML, /Episodes/);
  const card = section.children[1].children[0];
  assert.match(card.innerHTML, /src="\/api\/thumb\/12"/);
  card.onclick();
  await settle();
  assert.equal(b.nodes.get('#comic-modal').classList.contains('on'), false);
  assert.equal(b.nodes.get('#video-modal').classList.contains('on'), true);
  assert.equal(b.nodes.get('#plyr-video').src, '/api/stream/12');
  assert.equal(b.nodes.get('#pb-audio').src, '');
});

test('late account details cannot replace a newer account during resume', async () => {
  const b = browser();
  await settle();
  b.run('IDENTITY = "old-fixture"');
  b.context.localStorage.frayd_token = 'nonfunctional-fixture-token';
  let release;
  b.respond(async url => url === '/api/auth/me' ? new Promise(resolve => { release = resolve; }) : { ok: true, json: async () => [] });
  const pending = b.run('tryResume()');
  await settle();
  assert.equal(typeof release, 'function');
  b.run('++accountGeneration; IDENTITY = "new-fixture"; account = { name: "New account" }');
  release({ ok: true, json: async () => ({ name: 'Old account', is_admin: true }) });
  await pending;
  assert.equal(b.run('account.name'), 'New account');
  assert.equal(b.nodes.get('#app').classList.contains('on'), false);
});

test('failed stale search cannot replace a newer account view with local results', async () => {
  const b = browser();
  await settle();
  let reject;
  b.respond(() => new Promise((resolve, fail) => { reject = fail; }));
  const pending = b.run('serverSearch("old")');
  b.run('++accountGeneration; IDENTITY = "new-fixture"');
  b.nodes.get('#main').innerHTML = 'new account view';
  reject(new Error('retired request failure'));
  await pending;
  assert.equal(b.nodes.get('#main').innerHTML, 'new account view');
});

function progressRenderFixture() {
  const b = browser();
  const create = b.context.document.createElement;
  b.context.document.createElement = (...args) => {
    const node = create(...args);
    const strip = { style: {}, firstElementChild: { style: {} } };
    node.isConnected = true;
    node.querySelector = selector => selector === '.progress-strip' ? strip : null;
    return node;
  };
  b.run(`IDENTITY = 'owned-render-fixture'; LIBRARY = [
    { id: 1, kind: 'movie', title: 'Movie', duration_secs: 100 },
    { id: 2, kind: 'episode', title: 'Episode', duration_secs: 100 },
    { id: 3, kind: 'music', title: 'Song', duration_secs: 100 },
    { id: 4, kind: 'ebook', title: 'Book' }
  ]; filtering = false;`);
  const positions = new Map([[1, 20], [2, 80], [3, 99], [4, 0]]);
  b.respond(async url => ({ ok: true, json: async () => new URL(url, 'https://fixture.invalid').pathname === '/api/library' ? b.run('LIBRARY') : ({ position: positions.get(Number(new URL(url, 'https://fixture.invalid').pathname.split('/').at(-1))) }) }));
  return { b, positions, reads: () => b.requests.filter(request => request.url.startsWith('/api/progress/')) };
}

test('one render shares progress reads across continue cards, library cards and sorting', async () => {
  const { b, reads } = progressRenderFixture();
  await b.run('renderRows(LIBRARY)');
  await settle();
  const counts = {};
  for (const request of reads()) {
    const id = Number(new URL(request.url, 'https://fixture.invalid').pathname.split('/').at(-1));
    counts[id] = (counts[id] || 0) + 1;
  }
  assert.deepEqual(counts, { 1: 1, 2: 1, 3: 1, 4: 1 }, 'Each media ID is read once within the owned render');
  const sections = b.nodes.get('#main').children;
  const continuing = sections[0].children[1].children;
  assert.deepEqual(continuing.map(card => card.title), ['Episode', 'Movie']);
  assert.deepEqual(continuing.map(card => card.querySelector('.progress-strip').firstElementChild.style.width), ['80.0%', '20.0%']);
  const movieSection = sections[1];
  const beforeSort = reads().length;
  movieSection.children[0].children[0].textContent = 'oldest first';
  movieSection.children[0].children[0].onclick();
  await settle();
  assert.equal(reads().length, beforeSort, 'Sorting reuses this render snapshot');
});

test('new render and explicit media opening each fetch fresh progress', async () => {
  const { b, positions, reads } = progressRenderFixture();
  await b.run('renderRows(LIBRARY)');
  await settle();
  positions.set(1, 60);
  b.nodes.get('#main').children.length = 0; // The boundary double does not parse innerHTML clearing.
  b.requests.length = 0;
  await b.run('loadLibrary()');
  await settle();
  assert.equal(reads().length, 4, 'A new render never reuses the previous snapshot');
  const continuation = b.nodes.get('#main').children[0].children[1].children;
  assert.equal(continuation.find(card => card.title === 'Movie').querySelector('.progress-strip').firstElementChild.style.width, '60.0%');
  const beforeOpen = reads().length;
  b.run('openVideo(LIBRARY[0])');
  await settle();
  assert.equal(reads().length, beforeOpen + 1, 'Playback resume does not use the render snapshot');
});

for (const failure of ['http', 'network', 'json']) {
  test(`render progress ${failure} failure stays empty and a new render retries`, async () => {
    const { b, reads } = progressRenderFixture();
    b.respond(async () => {
      if (failure === 'network') throw new Error('owned progress failure');
      return { ok: failure !== 'http', json: async () => {
        if (failure === 'json') throw new Error('owned JSON failure');
        return null;
      } };
    });
    await b.run('renderRows(LIBRARY)');
    await settle();
    assert.equal(reads().length, 4);
    assert.doesNotMatch(b.nodes.get('#main').children[0].children[0].innerHTML, /Continue watching/);
    b.respond(async () => ({ ok: true, json: async () => ({ position: 25 }) }));
    b.nodes.get('#main').children.length = 0;
    b.requests.length = 0;
    await b.run('renderRows(LIBRARY)');
    await settle();
    assert.equal(reads().length, 4);
    assert.match(b.nodes.get('#main').children[0].children[0].innerHTML, /Continue watching/);
  });
}

test('retired render progress cannot paint cards or start new reads after an account change', async () => {
  const { b, reads } = progressRenderFixture();
  const releases = [];
  b.respond(() => new Promise(resolve => releases.push(resolve)));
  const old = b.run('renderRows(LIBRARY)');
  const oldReadCount = reads().length;
  b.run('++accountGeneration; IDENTITY = "new-render-fixture"');
  b.nodes.get('#main').innerHTML = 'new account view';
  releases.forEach(release => release({ ok: true, json: async () => ({ position: 50 }) }));
  await old;
  await settle();
  assert.equal(reads().length, oldReadCount, 'Old continuation cannot issue card reads in the new account');
  assert.equal(b.nodes.get('#main').innerHTML, 'new account view');
  assert.equal(b.nodes.get('#main').children.length, 0);
});

async function pendingLoginFixture(completion) {
  const storage = { frayd_identity: 'owned-login-fixture', frayd_logout: '1' };
  const b = browser({ storage });
  await settle();
  b.nodes.get('#email').value = 'fixture@example.invalid';
  b.nodes.get('#password').value = 'nonfunctional-fixture-password';
  let resolve, reject;
  const held = new Promise((accept, fail) => { resolve = accept; reject = fail; });
  b.respond(async url => {
    if (url !== '/api/auth/login') return { ok: true, json: async () => [] };
    return completion.startsWith('body-') ? { ok: true, json: () => held } : held;
  });
  const pending = b.run('doLogin({ preventDefault() {} })');
  await settle();
  const release = () => {
    const data = { name: 'Fixture account', token: 'nonfunctional-fixture-token', is_admin: false };
    if (completion === 'http-success') resolve({ ok: true, json: async () => data });
    else if (completion === 'body-success') resolve(data);
    else if (completion === 'http-failure') resolve({ ok: false, status: 401 });
    else reject(new Error('owned fixture failure'));
  };
  return { b, storage, pending, release };
}

for (const completion of ['http-success', 'body-success', 'http-failure', 'network-failure', 'body-failure']) {
  test(`late login ${completion} cannot replace explicit guest entry or repaint its hidden sign-in`, async () => {
    const { b, storage, pending, release } = await pendingLoginFixture(completion);
    b.nodes.get('#guest').emit('click');
    await settle();
    const owner = b.run('accountGeneration');
    assert.equal(b.nodes.get('#user-name').textContent, 'Local user');
    release();
    await pending;
    await settle();
    assert.equal(b.run('accountGeneration'), owner);
    assert.equal(b.run('account'), null);
    assert.equal(Object.hasOwn(storage, 'frayd_token'), false);
    assert.equal(b.nodes.get('#user-name').textContent, 'Local user');
    assert.equal(b.nodes.get('#app').classList.contains('on'), true);
    assert.equal(b.nodes.get('#login').style.display, 'none');
    assert.equal(b.nodes.get('#auth-error').style.display, 'none');
  });
}

for (const completion of ['http-success', 'http-failure']) {
  test(`current login ${completion} still publishes its account or visible error`, async () => {
    const { b, storage, pending, release } = await pendingLoginFixture(completion);
    release();
    await pending;
    await settle();
    const success = completion === 'http-success';
    assert.equal(b.run('account !== null'), success);
    assert.equal(Object.hasOwn(storage, 'frayd_token'), success);
    assert.equal(b.nodes.get('#app').classList.contains('on'), success);
    assert.equal(b.nodes.get('#login').style.display, success ? 'none' : 'flex');
    assert.equal(b.nodes.get('#auth-error').style.display, success ? 'none' : 'block');
  });
}

for (const completionOrder of [[0, 1], [1, 0]]) {
  test(`latest explicit login owns the result when completion order is ${completionOrder.join(' then ')}`, async () => {
    const b = browser({ storage: { frayd_identity: 'owned-login-fixture', frayd_logout: '1' } });
    await settle();
    const releases = [];
    b.respond(url => url === '/api/auth/login' ? new Promise(resolve => releases.push(resolve))
      : Promise.resolve({ ok: true, json: async () => [] }));
    b.nodes.get('#email').value = 'fixture@example.invalid';
    b.nodes.get('#password').value = 'nonfunctional-fixture-password';
    const earlier = b.run('doLogin({ preventDefault() {} })');
    const latest = b.run('doLogin({ preventDefault() {} })');
    for (const index of completionOrder) {
      releases[index]({ ok: true, json: async () => ({ name: index ? 'Latest fixture' : 'Earlier fixture', token: 'nonfunctional-fixture-token' }) });
      await settle();
    }
    await Promise.all([earlier, latest]);
    assert.equal(b.run('account.name'), 'Latest fixture');
    assert.equal(b.nodes.get('#app').classList.contains('on'), true);
    assert.equal(b.nodes.get('#auth-error').style.display, 'none');

    // An empty form is not a new valid login intent and must not retire its owner.
    const owner = b.run('accountGeneration');
    b.nodes.get('#email').value = '';
    b.nodes.get('#password').value = '';
    const ignored = b.run('doLogin({ preventDefault() {} })');
    assert.equal(releases.length, 2, 'Empty form never issues another login request');
    await ignored;
    assert.equal(b.run('accountGeneration'), owner);
    assert.equal(b.run('account.name'), 'Latest fixture');
  });
}
