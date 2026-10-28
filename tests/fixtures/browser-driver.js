/* global ebook, showChunk, LIBRARY, plyr, partyFollowId, partyPlaybackKind */
'use strict';
(async () => {
  const cases = [];
  const errors = [];
  const mediaDiagnostics = [];
  window.addEventListener('error', event => {
    if (event instanceof ErrorEvent) errors.push({ message: event.message, stack: event.error?.stack, filename: event.filename, line: event.lineno });
    else mediaDiagnostics.push({ type: event.type, target: event.target?.tagName, detail: 'Empty fixture streams exercise DOM events without decoded playback.' });
  });
  window.addEventListener('unhandledrejection', event => errors.push({ rejection: String(event.reason), stack: event.reason?.stack }));
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  function check(value, message) { if (!value) throw new Error(message); }
  function equal(actual, expected, message) {
    check(JSON.stringify(actual) === JSON.stringify(expected), `${message}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
  }
  async function until(predicate, label) {
    const deadline = Date.now() + 4000;
    while (!(await predicate())) {
      if (Date.now() >= deadline) throw new Error(`Timed out: ${label}`);
      await delay(20);
    }
  }
  async function api(path, body) {
    const response = await fetch('/__fixture/' + path, body === undefined ? {} : {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    });
    check(response.ok, `Fixture ${path}: ${response.status}`);
    return response.json();
  }
  const frame = document.getElementById('em-frame');
  let sequence = 0;
  function probe(action = 'inspect', details = {}) {
    return new Promise((resolve, reject) => {
      const id = ++sequence;
      const timeout = setTimeout(() => { window.removeEventListener('message', receive); reject(new Error('Probe timeout')); }, 4000);
      function receive(event) {
        if (event.source !== frame.contentWindow || event.origin !== 'null'
          || event.data?.type !== 'regression-reply' || event.data.id !== id) return;
        clearTimeout(timeout);
        window.removeEventListener('message', receive);
        if (event.data.error) reject(new Error(event.data.error));
        else resolve(event.data);
      }
      window.addEventListener('message', receive);
      frame.contentWindow.postMessage({ type: 'regression-probe', id, action, ...details }, '*');
    });
  }
  async function load() {
    const loaded = new Promise(resolve => frame.addEventListener('load', resolve, { once: true }));
    await showChunk(0);
    await loaded;
    // Production restores saved highlights after its own 100 ms timer.
    await delay(160);
    equal((await probe()).opaqueOrigin, 'null', 'Reader must have opaque sandbox origin');
  }
  async function run(name, fn) {
    try { cases.push({ name, status: 'PASS', observed: await fn() }); }
    catch (error) { cases.push({ name, status: 'FAIL', error: error.stack }); }
  }
  async function highlight({ body, start, end, text, offset, search, saved = [] }) {
    await api('configure', { body, annotations: saved });
    ebook = { it: { id: 7 }, spine: ['chapter.xhtml'], chunk: 0, annots: saved.slice() };
    document.getElementById('ebook-modal').classList.add('on');
    await load();
    if (search) {
      frame.contentWindow.postMessage({ type: 'search', query: search }, '*');
      await until(async () => (await probe()).searchMarks.length > 0, 'production search mark');
    }
    const before = (await probe()).marks;
    await probe('select', { start, end });
    await until(async () => (await api('state')).posts.length === 1, 'annotation request');
    const state = await api('state');
    equal(state.posts[0], { media_id: 7, kind: 'highlight', spine_idx: 0,
      cfi: `char:${offset}:${end - start}`, text, color: 'yellow' }, 'Saved exact annotation');
    equal((await probe()).marks, before, 'No new paint before HTTP 201');
    await api('release', {});
    await until(async () => (await probe()).marks.includes(text), 'paint after HTTP 201');
    equal(ebook.annots.length, saved.length + 1, 'Saved in reader state');
    ebook.annots = await (await fetch('/api/annotations/7?identity=fixture')).json();
    await load();
    const restored = (await probe()).marks;
    equal(restored.slice().sort(), [...before, text].sort(), 'Reload restores exact mark text');
    await probe('select', { start, end });
    await delay(80);
    equal((await api('state')).posts.length, 1, 'Duplicate selection sends no second POST');
    return { cfi: state.posts[0].cfi, savedText: text, restoredMarks: restored, posts: 1, paintHeldUntil201: true };
  }
  await run('plain text save, 201 paint, reload and duplicate', () => highlight({
    body: '<p id="target">prefix target suffix</p>', start: 7, end: 13, text: 'target', offset: 7,
  }));
  await run('leading and trailing selection whitespace preserved', () => highlight({
    body: '<p id="target">prefix   target  suffix</p>', start: 6, end: 17, text: '   target  ', offset: 6,
  }));
  await run('production search mark splits preceding text', () => highlight({
    body: '<p id="target">prefix target suffix</p>', start: 7, end: 13, text: 'target', offset: 7, search: 'prefix',
  }));
  await run('existing saved mark splits preceding text', () => highlight({
    body: '<p id="target">prefix target suffix</p>', start: 7, end: 13, text: 'target', offset: 7,
    saved: [{ kind: 'highlight', spine_idx: 0, cfi: 'char:0:6', text: 'prefix', color: 'yellow' }],
  }));
  await run('body style and script text use the same offset space', () => highlight({
    body: '<style>.prefix{color:red}</style><script>void 0;</script><p id="target">prefix target suffix</p>',
    start: 7, end: 13, text: 'target', offset: '.prefix{color:red}'.length + 'void 0;'.length + 7,
  }));
  for (const fixture of [
    { name: 'cross-node selection rejected', body: '<p id="target">one <b>two</b></p>', start: 0, end: 7, rejection: true },
    { name: 'selection ending at next text node offset zero rejected', body: '<p id="target">one</p><p id="next">two</p>', start: 0, end: 3, nextNode: true, rejection: true },
    { name: 'whitespace-only selection ignored', body: '<p id="target">   </p>', start: 0, end: 3, rejection: false },
  ]) await run(fixture.name, async () => {
    await api('configure', { body: fixture.body, annotations: [] });
    ebook = { it: { id: 7 }, spine: ['chapter.xhtml'], chunk: 0, annots: [] };
    await load();
    document.getElementById('em-searchinfo').textContent = '';
    await probe('select', fixture);
    await delay(80);
    equal((await api('state')).posts, [], 'Rejected selection has no annotation POST');
    equal((await probe()).marks, [], 'Rejected selection has no paint');
    equal(document.getElementById('em-searchinfo').textContent,
      fixture.rejection ? 'Select text without crossing formatting or paragraph breaks.' : '', 'Selection status');
    return { posts: 0, marks: 0 };
  });
  await run('actual vendored Plyr replaces media twice; active host events and follower seek remain connected', async () => {
    LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'movie' }];
    const forwarded = [];
    window.partyHostHook(message => forwarded.push(message));
    partyPlaybackKind = 'video';
    const original = plyr.media;
    const retired = [original];
    const detachedSyntheticEvents = [];
    for (const id of [10, 11]) {
      const previous = plyr.media;
      plyr.source = { type: 'video', sources: [{ src: `/api/stream/${id}`, type: 'video/mp4' }] };
      const active = plyr.media;
      check(active instanceof HTMLVideoElement && active !== previous && active.isConnected && !previous.isConnected,
        'Vendored source setter must replace and detach the prior media');
      await delay(80);
      forwarded.length = 0;
      for (const old of retired) for (const name of ['play', 'pause', 'seeked']) old.dispatchEvent(new Event(name));
      detachedSyntheticEvents.push({ retiredMedia: retired.length, forwarded: forwarded.slice() });
      check(forwarded.every(message => message.position === plyr.currentTime), 'Any retained vendor listeners must read current player position');
      forwarded.length = 0;
      for (const name of ['play', 'pause', 'seeked']) active.dispatchEvent(new Event(name));
      equal(forwarded.map(message => message.type), ['play', 'pause', 'seek'], 'Active host event forwarding');
      partyFollowId = null;
      window.partyFollow(id, 'Fixture movie', 12, false, Date.now() / 1000);
      check(active.getAttribute('src') === `/api/stream/${id}`, 'Follower uses current Plyr media');
      active.dispatchEvent(new Event('loadedmetadata'));
      equal(active.currentTime, 12, 'Follower metadata callback seeks current media');
      window.partyFollow(id, 'Fixture movie', 24, false, Date.now() / 1000);
      equal(active.currentTime, 24, 'Follower drift seeks current media');
      retired.push(active);
    }
    return { replacements: 2, activeEvents: ['play', 'pause', 'seek'], detachedSyntheticEvents,
      followerPositions: [12, 24], syntheticEventsOnly: true };
  });
  await api('result', { cases, errors, mediaDiagnostics, passed: cases.filter(item => item.status === 'PASS').length,
    failed: cases.filter(item => item.status === 'FAIL').length,
    limits: ['Synthetic media events do not prove decoded playback, codec support, or two-client synchronization.',
      'Vendored Plyr retains synthetic event forwarding from detached media; no naturally occurring late event user path is established.',
      'Disposable loopback API; no deployed service, user profile, account, or existing browser session accessed.'] });
})().catch(error => fetch('/__fixture/result', { method: 'POST', headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ cases: [], errors: [error.stack], failed: 1, passed: 0 }) }));
