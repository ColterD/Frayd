/* global ebook, showChunk, LIBRARY, plyr, partyFollowId, partyPlaybackKind, openVideo, PartyUI */
'use strict';
(async () => {
  const cases = [];
  const errors = [];
  const mediaDiagnostics = [];
  const nativeMedia = [];
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
    catch (error) { cases.push({ name, status: error.code === 'NATIVE_CODEC_UNAVAILABLE' ? 'BLOCKED' : 'FAIL', error: error.stack }); }
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
  async function nativeResume(name, decodableBlank, id = 20, expectedPosition = 64) {
    if (!document.createElement('video').canPlayType('video/mp4; codecs="avc1.42E01E"')) {
      const error = new Error('Native H264 codec is unavailable; resume scenario was not executed');
      error.code = 'NATIVE_CODEC_UNAVAILABLE';
      throw error;
    }
    const trace = [];
    const snapshot = event => {
      const media = plyr.media;
      trace.push({ event, time: media.currentTime, duration: media.duration,
        plyrDuration: plyr.duration, readyState: media.readyState, width: media.videoWidth,
        paused: media.paused, currentSrc: media.currentSrc,
        decodedFrames: media.getVideoPlaybackQuality().totalVideoFrames });
    };
    const nativeMetadata = event => {
      if (event.target instanceof HTMLVideoElement) snapshot(`native:${event.type}:trusted=${event.isTrusted}`);
    };
    const wrappedMetadata = () => snapshot('plyr:loadedmetadata');
    document.addEventListener('loadedmetadata', nativeMetadata, true);
    plyr.on('loadedmetadata', wrappedMetadata);
    plyr.muted = true;
    const originalBlank = plyr.config.blankVideo;
    if (decodableBlank) plyr.config.blankVideo = '/api/stream/21';
    const previous = plyr.media;
    previous.addEventListener('loadedmetadata', event => {
      trace.push({ event: 'retired-native-metadata', trusted: event.isTrusted,
        retired: previous !== plyr.media, src: previous.currentSrc,
        currentDuration: plyr.media.duration, currentTime: plyr.currentTime });
    }, { once: true });
    try {
      openVideo({ id, kind: 'movie', title: 'Native resume' });
      await until(() => {
        if (plyr.media.error?.code === 3 || plyr.media.error?.code === 4) {
          const error = new Error(`Native H264 decode unavailable: ${plyr.media.error.message}`);
          error.code = 'NATIVE_CODEC_UNAVAILABLE';
          throw error;
        }
        return plyr.media !== previous && plyr.media.readyState === 4
          && plyr.media.videoWidth === 160 && !plyr.media.paused && plyr.currentTime > 0;
      }, 'native H264 decode and playback');
      await delay(300);
      snapshot('settled');
      check(plyr.media.getVideoPlaybackQuality().totalVideoFrames > 0, 'Requested media must decode video frames');
      const activeMetadata = trace.findIndex(event => event.event === 'native:loadedmetadata:trusted=true');
      check(activeMetadata >= 0, 'Requested media must emit native trusted metadata');
      if (decodableBlank) {
        const retiredMetadata = trace.findIndex(event => event.event === 'retired-native-metadata' && event.trusted && event.retired);
        check(retiredMetadata >= 0 && retiredMetadata < activeMetadata,
          `Cancellation blank must finish before requested metadata: ${JSON.stringify(trace)}`);
      }
      nativeMedia.push({ name, trace: trace.slice(), expectedPosition,
        decodableBlank, canPlayH264: plyr.media.canPlayType('video/mp4; codecs="avc1.42E01E"') });
      check(plyr.currentTime >= expectedPosition && plyr.currentTime < expectedPosition + 6,
        `Native saved${expectedPosition} resume: ${JSON.stringify(trace)}`);
      return { trace: trace.slice(), decodedWidth: plyr.media.videoWidth, nativeEvents: true };
    } finally {
      document.removeEventListener('loadedmetadata', nativeMetadata, true);
      plyr.off('loadedmetadata', wrappedMetadata);
      document.getElementById('vm-back').click();
      plyr.config.blankVideo = originalBlank;
    }
  }
  await run('native decoded plain movie resume before any party hooks',
    () => nativeResume('plain native resume', false));
  await run('native decoded resume with a decodable cancellation blank before party hooks',
    () => nativeResume('decodable blank before party', true, 22));
  await run('native movie close and reopen retains the saved position',
    () => nativeResume('close and reopen', true));
  await run('native different movie without saved progress starts at zero',
    () => nativeResume('new movie without saved progress', false, 21, 0));
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
  // Keep real Plyr/source/DOM behavior; empty streams need controlled media timing.
  const originalFetch = window.fetch;
  const originalWebSocket = window.WebSocket;
  const mediaPrototype = HTMLMediaElement.prototype;
  const originalPlay = mediaPrototype.play;
  const originalPause = mediaPrototype.pause;
  const originalPaused = Object.getOwnPropertyDescriptor(mediaPrototype, 'paused');
  const playback = new WeakMap();
  mediaPrototype.play = function () {
    playback.set(this, false);
    this.dispatchEvent(new Event('play'));
    return Promise.resolve();
  };
  mediaPrototype.pause = function () {
    playback.set(this, true);
    this.dispatchEvent(new Event('pause'));
  };
  Object.defineProperty(mediaPrototype, 'paused', { configurable: true,
    get() { return playback.has(this) ? playback.get(this) : originalPaused.get.call(this); } });
  window.fetch = (url, options) => /^\/api\/progress\/(10|11)\?/.test(String(url))
    ? Promise.resolve(new Response(JSON.stringify({ position: 203 }), { headers: { 'Content-Type': 'application/json' } }))
    : originalFetch(url, options);
  let socket;
  window.WebSocket = class {
    constructor() { this.readyState = 1; this.sent = []; socket = this; }
    send(data) { this.sent.push(JSON.parse(data)); }
    close() { this.readyState = 3; }
  };
  async function firstPartyState({ id, isHost, metadataFirst, nextId = id, priorFollowId = null, localPaused = false }) {
    PartyUI.close();
    delete window.partySend;
    partyFollowId = priorFollowId;
    const previous = plyr.media;
    openVideo({ id, kind: 'movie', title: 'Resume fixture' });
    await until(() => plyr.media !== previous, 'openVideo installs real Plyr source');
    const media = plyr.media;
    Object.defineProperty(media, 'duration', { configurable: true, value: 634 });
    Object.defineProperty(media, 'readyState', { configurable: true, value: metadataFirst ? 1 : 0 });
    const sourceBefore = { src: media.getAttribute('src'), child: media.querySelector('source')?.getAttribute('src') };
    equal(sourceBefore, { src: null, child: `/api/stream/${id}` }, 'Plyr installs a source child without a video src attribute');
    if (metadataFirst) {
      media.dispatchEvent(new Event('loadedmetadata'));
      equal(plyr.currentTime, 203, 'Saved resume applied before first party echo');
    }
    if (localPaused) media.pause();
    PartyUI.open();
    document.getElementById('party-code-input').value = 'ABCD';
    document.getElementById('pt-join').click();
    const state = { media_id: nextId, title: 'Party fixture', queue: [], position: isHost ? 0 : 24,
      playing: false, updated_at: Date.now() / 1000 };
    socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: isHost, state }) });
    if (!metadataFirst) {
      Object.defineProperty(media, 'readyState', { configurable: true, value: 1 });
      media.dispatchEvent(new Event('loadedmetadata'));
    }
    check(plyr.media === media, 'Party echo retains active Plyr media element');
    const position = isHost && nextId === id ? 203 : isHost ? 0 : 24;
    equal(plyr.currentTime, position, 'Role-correct first party position');
    if (isHost && nextId === id) {
      equal({ src: media.getAttribute('src'), child: media.querySelector('source')?.getAttribute('src') }, sourceBefore,
        'Host echo preserves effective stream and source structure');
      equal(media.paused, localPaused, 'Host echo preserves local playback despite party state');
    } else {
      equal(media.paused, true, 'Guest or changed-media load follows paused party state');
    }
    if (!isHost) {
      socket.onmessage({ data: JSON.stringify({ type: 'state', state: { ...state, position: 48,
        playing: true, updated_at: Date.now() / 1000 } }) });
      check(plyr.currentTime >= 48 && plyr.currentTime < 50, 'Guest same-media update follows host seek');
      equal(media.paused, false, 'Guest same-media update follows host play');
    }
    PartyUI.close();
    return { isHost, metadataFirst, priorFollowId, loadedMedia: id, partyMedia: nextId, firstPosition: position,
      sourceBefore, sourceAfter: { src: media.getAttribute('src'), child: media.querySelector('source')?.getAttribute('src') },
      actualVendoredPlyr: true, productionPartyStateHandler: true, syntheticMediaTimingAndTransport: true };
  }
  for (const metadataFirst of [false, true]) {
    const id = metadataFirst ? 11 : 10;
    for (const isHost of [true, false]) await run(
      `${isHost ? 'host' : 'guest'} first party echo ${metadataFirst ? 'after' : 'before'} metadata uses correct resume authority`,
      () => firstPartyState({ id, isHost, metadataFirst }));
  }
  for (const metadataFirst of [false, true]) {
    const id = metadataFirst ? 11 : 10;
    await run(`host with stale same-media follow ID ${metadataFirst ? 'after' : 'before'} metadata preserves saved resume`,
      () => firstPartyState({ id, isHost: true, metadataFirst, priorFollowId: id }));
  }
  await run('host changed-media first state still loads the party stream',
    () => firstPartyState({ id: 11, nextId: 10, isHost: true, metadataFirst: true }));
  await run('host reactivates a closed retained video after queued audio', async () => {
    LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'music' }];
    PartyUI.close();
    delete window.partySend;
    const previous = plyr.media;
    openVideo({ id: 10, kind: 'movie', title: 'Previous movie' });
    await until(() => plyr.media !== previous, 'reactivation fixture installs video');
    const media = plyr.media;
    Object.defineProperty(media, 'duration', { configurable: true, value: 634 });
    Object.defineProperty(media, 'readyState', { configurable: true, value: 1 });
    media.dispatchEvent(new Event('loadedmetadata'));
    equal(plyr.currentTime, 203, 'Prior movie resumes normally');
    document.getElementById('vm-back').click();
    PartyUI.open();
    document.getElementById('party-code-input').value = 'ABCD';
    document.getElementById('pt-join').click();
    const state = { title: 'Queued fixture', queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 };
    socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true, state: { ...state, media_id: 11 } }) });
    const audio = document.getElementById('pb-audio');
    // Empty fixture bytes require explicit metadata for the real audio element.
    audio.dispatchEvent(new Event('loadedmetadata'));
    equal(audio.paused, false, 'Queued audio is playing before video request');
    equal(partyPlaybackKind, 'audio', 'Audio owns playback before video request');
    check(!document.getElementById('video-modal').classList.contains('on'), 'Prior video is closed');
    socket.onmessage({ data: JSON.stringify({ type: 'state', state: { ...state, media_id: 10, updated_at: Date.now() / 1000 } }) });
    equal(audio.paused, true, 'Reactivated video pauses queued audio');
    check(document.getElementById('video-modal').classList.contains('on'), 'Reactivated video modal is visible');
    equal(media.paused, false, 'Reactivated video follows playing state');
    check(plyr.currentTime < 2, 'Reactivated video follows party load position');
    PartyUI.close();
    return { beforeKind: 'audio', afterKind: partyPlaybackKind, audioPaused: audio.paused,
      videoPaused: media.paused, videoPosition: plyr.currentTime, productionHostHandler: true };
  });
  await run('host preserves an active paused video in real Plyr', () => {
    LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'movie' }];
    return firstPartyState({ id: 11, isHost: true, metadataFirst: true, localPaused: true });
  });
  await run('guest video to fresh and retained audio pauses video and retains explicit panel control', async () => {
    const observations = [];
    for (const audioWasLoaded of [false, true]) {
      PartyUI.close();
      delete window.partySend;
      LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'music' }];
      partyFollowId = null;
      const audio = document.getElementById('pb-audio');
      audio.pause();
      audio.removeAttribute('src');
      audio.load();
      const video = plyr.media;
      Object.defineProperty(video, 'duration', { configurable: true, value: 634 });
      Object.defineProperty(video, 'readyState', { configurable: true, value: 1 });
      document.getElementById('vm-party').click();
      document.getElementById('party-code-input').value = 'ABCD';
      document.getElementById('pt-join').click();
      const state = id => ({ type: 'state', is_host: false, state: { media_id: id, title: 'Fixture',
        queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 } });
      if (audioWasLoaded) {
        socket.onmessage({ data: JSON.stringify(state(11)) });
        audio.dispatchEvent(new Event('loadedmetadata'));
      }
      socket.onmessage({ data: JSON.stringify(state(10)) });
      equal(video.paused, false, 'Guest video starts before audio follow');
      const modal = document.getElementById('video-modal');
      modal.classList.add('yt');
      document.getElementById('playerbar').classList.remove('on');
      socket.onmessage({ data: JSON.stringify(state(11)) });
      audio.dispatchEvent(new Event('loadedmetadata'));
      equal(video.paused, true, 'Audio follow pauses the actual current Plyr media');
      equal(audio.paused, false, 'Audio follow starts audio');
      check(!modal.classList.contains('on') && !modal.classList.contains('yt'), 'Audio follow closes video presentation');
      check(document.getElementById('playerbar').classList.contains('on'), 'Audio player bar is visible');
      check(document.getElementById('party-modal').classList.contains('on'), 'Media changes do not hide party controls');
      const connection = socket;
      PartyUI.hide();
      document.getElementById('vm-party').click();
      check(socket === connection && document.getElementById('party-stage').textContent.includes('ABCD'),
        'Video party button reopens connected code without reconnecting');
      observations.push({ audioWasLoaded, videoPaused: video.paused, audioPaused: audio.paused,
        videoModalOpen: modal.classList.contains('on'), connectionRetained: socket === connection });
      PartyUI.close();
    }
    return observations;
  });
  // End a party through its production state handler before the native control.
  PartyUI.open();
  document.getElementById('party-code-input').value = 'ABCD';
  document.getElementById('pt-join').click();
  socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true,
    state: { media_id: null, title: '', queue: [], position: 0, playing: false } }) });
  socket.onmessage({ data: JSON.stringify({ type: 'ended' }) });
  window.fetch = originalFetch;
  window.WebSocket = originalWebSocket;
  mediaPrototype.play = originalPlay;
  mediaPrototype.pause = originalPause;
  Object.defineProperty(mediaPrototype, 'paused', originalPaused);
  await run('native decoded movie resume after production party end',
    () => nativeResume('after prior party lifecycle', true));
  await api('result', { cases, errors, mediaDiagnostics, nativeMedia, passed: cases.filter(item => item.status === 'PASS').length,
    failed: cases.filter(item => item.status === 'FAIL').length,
    limits: ['IDs10/11 use synthetic media events; native20/21/22 decode the owned H264 fixture. Other codecs, physical output and two-client synchronization remain untested.',
      'Vendored Plyr retains synthetic event forwarding from detached media; no naturally occurring late event user path is established.',
      'Disposable loopback API; no deployed service, user profile, account, or existing browser session accessed.'] });
})().catch(error => fetch('/__fixture/result', { method: 'POST', headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ cases: [], errors: [error.stack], failed: 1, passed: 0 }) }));
