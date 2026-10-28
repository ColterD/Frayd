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
    await until(() => ebook?.document?.ready, 'current reader ready');
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
    const retiredMetadata = event => {
      trace.push({ event: 'retired-native-metadata', trusted: event.isTrusted,
        retired: previous !== plyr.media, src: previous.currentSrc,
        currentDuration: plyr.media.duration, currentTime: plyr.currentTime });
    };
    previous.addEventListener('loadedmetadata', retiredMetadata, { once: true });
    try {
      if (decodableBlank) await api('hold-media', { path: `/api/stream/${id}` });
      openVideo({ id, kind: 'movie', title: 'Native resume' });
      if (decodableBlank) {
        await until(async () => {
          const state = await api('media-gate');
          check(state.pending > 0 || !trace.some(event => event.event === 'native:loadedmetadata:trusted=true'),
            'Requested movie metadata bypassed the held server stream (browser media cache)');
          return state.pending > 0;
        }, 'requested movie byte request at the held server stream');
        await until(() => trace.some(event => event.event === 'retired-native-metadata' && event.trusted && event.retired),
          'retired cancellation blank metadata before requested movie release');
        const release = await api('release-media', {});
        check(release.released && release.waited > 0, 'Requested native bytes must be held and explicitly released');
        trace.push({ event: 'requested-stream-released', waited: release.waited });
      }
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
      try { if (decodableBlank) await api('release-media', {}); }
      finally {
        previous.removeEventListener('loadedmetadata', retiredMetadata);
        document.removeEventListener('loadedmetadata', nativeMetadata, true);
        plyr.off('loadedmetadata', wrappedMetadata);
        document.getElementById('vm-back').click();
        plyr.config.blankVideo = originalBlank;
      }
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
  await run('decoded native video suppresses queued pause after rapid external round trip and keeps user pause', async () => {
    const saved = { player: ytPlayer, id: ytId, apiReady: ytReady, playerReady: ytPlayerReady };
    const forwarded = [];
    const pauses = [];
    let media;
    const observe = event => pauses.push({ trusted: event.isTrusted, pausedWhenDelivered: media.paused });
    try {
      LIBRARY = [{ id: 20, kind: 'movie' }];
      openVideo({ id: 20, kind: 'movie', title: 'Decoded queued pause fixture' });
      await until(() => plyr.media.readyState === 4 && !plyr.media.paused
        && plyr.media.videoWidth === 160
        && plyr.media.getVideoPlaybackQuality().totalVideoFrames > 0, 'decoded native playback before queued pause');
      media = plyr.media;
      check(media.getVideoPlaybackQuality().totalVideoFrames > 0, 'Queue scenario decodes owned video frames');
      media.addEventListener('pause', observe);
      partyHostHook(command => forwarded.push(command));
      partyFollowId = 20;
      ytReady = true;
      ytPlayerReady = true;
      ytId = 'native-queued-roundtrip';
      ytPlayer = { seekTo() {}, pauseVideo() {}, getCurrentTime: () => 0, getPlayerState: () => 2 };
      forwarded.length = 0;
      partyFollowExternal('native-queued-roundtrip', 'Mock external', 0, false, Date.now() / 1000, false);
      partyFollow(20, 'Native return', 12, true, Date.now() / 1000);
      await until(() => pauses.length > 0 && !media.paused, 'actual queued pause arrives after native resume');
      await delay(80);
      check(pauses[0].trusted && !pauses[0].pausedWhenDelivered, 'Trusted stale pause is delivered while native video is playing');
      equal(forwarded.filter(command => command.type === 'pause'), [], 'Queued stale native pause cannot overwrite resumed party');
      forwarded.length = 0;
      media.pause();
      await until(() => forwarded.some(command => command.type === 'pause'), 'legitimate native pause is broadcast');
      equal(forwarded.filter(command => command.type === 'pause').length, 1, 'Legitimate native pause broadcasts once');
      check(pauses.at(-1).trusted && pauses.at(-1).pausedWhenDelivered, 'Legitimate native pause is trusted and currently paused');
      return { pauses, decodedFrames: media.getVideoPlaybackQuality().totalVideoFrames,
        actualNativeEvents: true, externalAdapterMocked: true, legitimatePauseBroadcasts: 1 };
    } finally {
      if (media) media.removeEventListener('pause', observe);
      document.getElementById('vm-back').click();
      ytPlayer = saved.player; ytId = saved.id; ytReady = saved.apiReady; ytPlayerReady = saved.playerReady;
    }
  });
  await run('saved marks survive parser-gated reader startup', async () => {
    const saved = [{ kind: 'highlight', spine_idx: 0, cfi: 'char:7:6', text: 'target', color: 'yellow' }];
    await api('configure', { body: '<p id="target">prefix target suffix</p>', annotations: saved, parserGate: true });
    ebook = { it: { id: 7 }, spine: ['one.xhtml', 'two.xhtml'], chunk: 0, annots: saved };
    document.getElementById('ebook-modal').classList.add('on');
    const loaded = new Promise(resolve => frame.addEventListener('load', resolve, { once: true }));
    await showChunk(0, { frac: 0.5 });
    await until(async () => (await api('parser-state')).held, 'parser blocked before generated listener');
    await delay(240); // Negative control: the old one-shot 100 ms restore expires here.
    await api('release-parser', {});
    await loaded;
    await delay(80);
    const observed = await probe();
    equal(observed.marks, ['target'], 'Saved mark restored after actual listener readiness');
    equal((await api('state')).posts, [], 'Restore requires no new annotation POST');
    const retiredGeneration = ebook.document.generation;
    frame.contentWindow.postMessage({ type: 'inject-hl', generation: retiredGeneration,
      items: [{ off: 7, len: 6, color: 'yellow' }] }, '*');
    equal((await probe()).marks, ['target'], 'Repeated restore does not nest marks');
    document.getElementById('em-close').click();
    await api('configure', { body: '<p id="target">prefix target suffix</p>', annotations: saved });
    await openEbook({ id: 7, title: 'Reopened saved book' });
    await until(() => ebook?.document?.ready, 'reopened document ready');
    equal((await probe()).marks, ['target'], 'Account reopen restores saved annotation');
    const reopenedGeneration = ebook.document.generation;
    await showChunk(1);
    await until(() => ebook.document.ready, 'second TOC chapter ready');
    equal((await probe()).marks, [], 'Other chapter has no saved mark');
    await showChunk(0);
    await until(() => ebook.document.ready, 'first TOC chapter ready');
    equal((await probe()).marks, ['target'], 'TOC revisit restores exactly once');
    // The opaque WindowProxy remains the same across these documents.
    await probe('ready', { generation: reopenedGeneration });
    equal((await probe()).marks, ['target'], 'Stale ready cannot duplicate current marks');
    return { marks: observed.marks, parserHeldBeyondOldTimer: true, posts: 0,
      reopen: true, tocRevisit: true, duplicateRestoreIdempotent: true, staleReadyIgnored: true };
  });
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
      active.dispatchEvent(new Event('play'));
      check(!forwarded.some(message => message.type === 'play'), 'A play event while the native element is paused is stale');
      active.dispatchEvent(new Event('pause'));
      active.dispatchEvent(new Event('seeked'));
      equal(forwarded.map(message => message.type), ['pause', 'seek'], 'Active host event forwarding respects actual paused state');
      partyFollowId = null;
      window.partyFollow(id, 'Fixture movie', 12, false, Date.now() / 1000);
      check(active.getAttribute('src') === `/api/stream/${id}`, 'Follower uses current Plyr media');
      // HTTP204 supplies no metadata: model readiness explicitly for this synthetic event.
      const readyDescriptor = Object.getOwnPropertyDescriptor(active, 'readyState');
      try {
        Object.defineProperty(active, 'readyState', { configurable: true, value: 1 });
        active.dispatchEvent(new Event('loadedmetadata'));
        equal(active.currentTime, 12, 'Follower metadata callback seeks current media');
        window.partyFollow(id, 'Fixture movie', 24, false, Date.now() / 1000);
        equal(active.currentTime, 24, 'Follower drift seeks current media');
      } finally {
        if (readyDescriptor) Object.defineProperty(active, 'readyState', readyDescriptor);
        else delete active.readyState;
      }
      retired.push(active);
    }
    return { replacements: 2, activeEvents: ['play', 'pause', 'seek'], detachedSyntheticEvents,
      followerPositions: [12, 24], syntheticEventsOnly: true, metadataReadinessModeledAndRestored: true };
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
    if (this.paused) return;
    playback.set(this, true);
    setTimeout(() => this.dispatchEvent(new Event('pause')), 0);
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
    const cachedActivations = [];
    for (const transition of ['close', 'local audio']) {
      partyFollow(10, 'Cached movie', 12, true, Date.now() / 1000);
      equal(partyFollowId, 10, 'Video follow ID remains cached before native transition');
      const sourceMutations = [];
      let audioSourceInstalled = false;
      const observer = new MutationObserver(records => {
        sourceMutations.push(...records.filter(record => record.target === media));
        if (records.some(record => record.target === audio)) audioSourceInstalled = true;
      });
      observer.observe(media, { attributes: true, attributeFilter: ['src'] });
      const audioReadyDescriptor = Object.getOwnPropertyDescriptor(audio, 'readyState');
      try {
        if (transition === 'close') document.getElementById('vm-back').click();
        else {
          observer.observe(audio, { attributes: true, attributeFilter: ['src'] });
          Object.defineProperty(audio, 'readyState', { configurable: true, value: 0 });
          playAudio({ id: 11, kind: 'music', title: 'Local audio before cached return' });
          await until(() => audioSourceInstalled, 'local audio source assigned after progress response');
          // HTTP204 has no decoded metadata; deliver modeled readiness after its listener is installed.
          Object.defineProperty(audio, 'readyState', { configurable: true, value: 1 });
          audio.dispatchEvent(new Event('loadedmetadata'));
          await until(() => !audio.paused, 'local audio starts before cached video return');
        }
        observer.takeRecords();
        sourceMutations.length = 0;
        partyFollow(10, 'Cached movie', 24, true, Date.now() / 1000);
        await delay(10);
        check(document.getElementById('video-modal').classList.contains('on'), 'Cached video follow reopens the native modal');
        equal(audio.paused, true, 'Cached video follow pauses local audio');
        equal(media.paused, false, 'Cached video resumes playback');
        equal(videoItem.id, 10, 'Cached video restores progress identity');
        equal(sourceMutations.length, 0, 'Cached video activation does not reassign the stream');
        cachedActivations.push({ transition, modalOpen: true, audioPaused: audio.paused,
          nativePlaying: !media.paused, progressMediaId: videoItem.id, sourceAssignments: sourceMutations.length });
      } finally {
        observer.disconnect();
        if (audioReadyDescriptor) Object.defineProperty(audio, 'readyState', audioReadyDescriptor);
        else delete audio.readyState;
      }
    }
    return { beforeKind: 'audio', afterKind: partyPlaybackKind, audioPaused: audio.paused,
      videoPaused: media.paused, videoPosition: plyr.currentTime, productionHostHandler: true, cachedActivations, localAudioMetadataModeledAndRestored: true };
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
  for (const isHost of [true, false]) for (const kind of ['music', 'movie']) {
    await run(`${isHost ? 'host' : 'guest'} external YouTube to fresh and retained ${kind} stops external playback`, async () => {
      const originalYT = window.YT;
      const originalReady = ytReady;
      const observations = [];
      let mock;
      let playerCreations = 0;
      window.YT = { Player: function (id, config) {
        playerCreations++;
        mock = { state: 2, position: 0, pauses: 0, ready: false, commands: [],
          requireReady() { if (!this.ready) throw new Error('Mock YouTube player is not ready'); },
          emitReady() { this.ready = true; config.events.onReady({ target: this }); },
          seekTo(position) { this.requireReady(); this.commands.push('seek'); this.position = position; },
          getCurrentTime() { this.requireReady(); return this.position; },
          getPlayerState() { this.requireReady(); return this.state; },
          playVideo() { this.requireReady(); this.commands.push('play'); this.state = 1; config.events.onStateChange({ target: this, data: 1 }); },
          pauseVideo() { this.requireReady(); this.pauses++; this.state = 2; config.events.onStateChange({ target: this, data: 2 }); },
          destroy() {},
        };
        return mock;
      } };
      ytReady = true;
      try {
        for (const retained of [false, true]) for (const readyBeforeTransition of [true, false]) {
          PartyUI.close();
          delete window.partySend;
          LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'music' }];
          const id = kind === 'movie' ? 10 : 11;
          partyFollowId = retained ? id : null;
          const native = kind === 'movie' ? plyr.media : document.getElementById('pb-audio');
          native.src = `/api/stream/${id}`;
          Object.defineProperty(native, 'readyState', { configurable: true, value: 1 });
          partyPlaybackKind = kind === 'movie' ? 'video' : 'audio';
          await native.play();
          equal(native.paused, false, 'Native player is running before external handoff');
          PartyUI.open();
          document.getElementById('party-code-input').value = 'ABCD';
          document.getElementById('pt-join').click();
          socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: isHost,
            state: { external_url: `mock-${kind}-${isHost}-${retained}-${readyBeforeTransition}`, title: 'Mock YouTube',
              media_id: null, queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 } }) });
          equal(document.getElementById('pb-audio').paused, true, 'External handoff pauses native audio');
          equal(plyr.paused, true, 'External handoff pauses native video');
          check(!socket.sent.some(command => command.type === 'pause'), 'Native pause cannot broadcast a stale host pause');
          if (readyBeforeTransition) mock.emitReady();
          await delay(0);
          equal(mock.state, readyBeforeTransition ? 1 : 2, 'External mock has the expected readiness and playback');
          const modal = document.getElementById('video-modal');
          check(modal.classList.contains('yt'), 'External presentation is active');
          socket.sent.length = 0;
          socket.onmessage({ data: JSON.stringify({ type: 'state',
            state: { external_url: null, media_id: id, title: 'Native fixture', queue: [],
              position: 24, playing: true, updated_at: Date.now() / 1000 } }) });
          equal(mock.state, 2, 'External playback is stopped after native transition');
          equal(mock.pauses, readyBeforeTransition ? 1 : 0, 'Only a ready external player is paused');
          if (!readyBeforeTransition) {
            mock.emitReady();
            equal(mock.commands, [], 'Late readiness cannot seek or start YouTube after native transition');
            equal(mock.state, 2, 'Late readiness leaves external playback stopped');
          }
          check(!socket.sent.some(command => command.type === 'pause'), 'External pause cannot broadcast a stale host pause');
          check(!modal.classList.contains('yt'), 'Native player presentation is restored');
          equal(modal.classList.contains('on'), kind === 'movie', 'Native modal visibility');
          equal(native.paused, false, 'Native playback follows playing state');
          const expectedPosition = isHost ? 0 : 24;
          check(native.currentTime >= expectedPosition && native.currentTime < expectedPosition + 2,
            'Native position follows role authority');
          observations.push({ retained, readyBeforeTransition, nativePausedForExternal: true, externalPauses: mock.pauses, nativePlaying: !native.paused,
            position: native.currentTime, modalOpen: modal.classList.contains('on'), youtubeClass: false });
        }
        const queuedAudio = document.getElementById('pb-audio');
        const queueCommands = [];
        partyHostHook(command => queueCommands.push(command));
        LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'music' }];
        Object.defineProperty(queuedAudio, 'readyState', { configurable: true, value: 1 });
        partyFollow(11, 'Queue audio', 24, true, Date.now() / 1000);
        await delay(0);
        queueCommands.length = 0;
        partyFollowExternal(`queued-audio-${kind}-${isHost}`, 'Mock external', 0, false, Date.now() / 1000, false);
        partyFollow(11, 'Queue audio return', 24, true, Date.now() / 1000);
        await delay(10);
        equal(queueCommands.filter(command => command.type === 'pause'), [], 'Queued audio pause cannot overwrite resumed party');
        queuedAudio.pause();
        await delay(10);
        equal(queueCommands.filter(command => command.type === 'pause').length, 1, 'Legitimate queued audio pause broadcasts once');
        const pendingMedia = plyr.media;
        Object.defineProperty(pendingMedia, 'readyState', { configurable: true, value: 0 });
        partyFollowId = null;
        partyFollow(10, 'Pending native video', 24, true, Date.now() / 1000);
        partyFollowExternal(`pending-video-${kind}-${isHost}`, 'Mock external', 0, false, Date.now() / 1000, false);
        Object.defineProperty(pendingMedia, 'readyState', { configurable: true, value: 1 });
        pendingMedia.dispatchEvent(new Event('loadedmetadata'));
        await delay(10);
        equal(pendingMedia.paused, true, 'Retired native metadata cannot restart video under external ownership');
        const pendingId = `latest-state-${kind}-${isHost}`;
        partyFollowExternal(pendingId, 'First pending state', 5, true, Date.now() / 1000, isHost);
        partyFollowExternal(pendingId, 'Latest pending state', 20, false, Date.now() / 1000, isHost);
        mock.emitReady();
        equal(mock.commands, ['seek'], 'Ready callback applies the latest paused request without playing');
        equal(mock.position, 20, 'Ready callback seeks to the latest same-video position');
        equal(mock.state, 2, 'Latest pending paused request remains paused');
        const localActivations = [];
        for (const localKind of ['audio', 'video']) {
          partyFollowExternal(`local-${kind}-${isHost}-${localKind}`, 'External before local action',
            5, true, Date.now() / 1000, isHost);
          mock.emitReady();
          equal(mock.state, 1, 'External player is running before local native activation');
          const previousMedia = plyr.media;
          socket.sent.length = 0;
          if (localKind === 'audio') playAudio({ id: 11, kind: 'music', title: 'Local audio' });
          else openVideo({ id: 10, kind: 'movie', title: 'Local movie' });
          equal(mock.state, 2, 'Local native entry pauses external playback synchronously');
          equal(mock.pauses, 1, 'Local native entry pauses external player once');
          check(!socket.sent.some(command => command.type === 'pause'), 'Local handoff suppresses external pause broadcast');
          if (localKind === 'video') {
            await until(() => plyr.media !== previousMedia, 'local movie installs current Plyr source');
            Object.defineProperty(plyr.media, 'duration', { configurable: true, value: 634 });
            Object.defineProperty(plyr.media, 'readyState', { configurable: true, value: 1 });
            plyr.media.dispatchEvent(new Event('loadedmetadata'));
          }
          const localMedia = localKind === 'audio' ? document.getElementById('pb-audio') : plyr.media;
          await until(() => !localMedia.paused, 'local native playback starts');
          equal(partyPlaybackKind, localKind, 'Local player owns playback');
          check(!document.getElementById('video-modal').classList.contains('yt'), 'Local entry restores native presentation');
          localActivations.push({ kind: localKind, externalPauses: mock.pauses,
            externalStopped: mock.state === 2, nativePlaying: !localMedia.paused });
        }
        let roleReuse = null;
        if (isHost && kind === 'music') {
          function openRole(host) {
            PartyUI.close();
            PartyUI.open();
            document.getElementById('party-code-input').value = 'ABCD';
            document.getElementById('pt-join').click();
            socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: host,
              state: { external_url: 'same-role-video', media_id: null, title: 'Role reuse fixture',
                queue: [], position: 0, playing: false, updated_at: Date.now() / 1000 } }) });
            return socket;
          }
          const beforeRolePlayerCount = playerCreations;
          const guestSocket = openRole(false);
          mock.emitReady();
          const reusedPlayer = mock;
          mock.playVideo(); mock.pauseVideo();
          equal(guestSocket.sent, [], 'Guest-created external player cannot broadcast host commands');
          const hostSocket = openRole(true);
          check(mock === reusedPlayer && playerCreations === beforeRolePlayerCount + 1, 'Host session reuses the guest-created iframe');
          hostSocket.sent.length = 0;
          mock.position = 37;
          mock.playVideo(); mock.pauseVideo(); mock.playVideo();
          equal(hostSocket.sent.map(command => command.type), ['play', 'pause', 'play'], 'Current host sender receives one command per reused-player event');
          await until(() => hostSocket.sent.some(command => command.type === 'seek'), 'reused guest player gets current host seek poller');
          equal(hostSocket.sent.filter(command => command.type === 'seek').length, 1, 'Reused host player has one seek poller');
          check(hostSocket.sent.every(command => command.position === 37), 'Current host events use current external position');
          const hostCommands = hostSocket.sent.map(command => command.type);
          const nextGuest = openRole(false);
          mock.playVideo(); mock.pauseVideo(); mock.playVideo();
          await delay(2200);
          equal(nextGuest.sent, [], 'Host-to-guest reuse cannot publish state or seek');
          equal(hostSocket.sent.map(command => command.type), hostCommands, 'Departed host socket receives no reused-player events');
          PartyUI.close();
          mock.playVideo(); mock.pauseVideo();
          equal(nextGuest.sent, [], 'Leave clears the current external sender');
          equal(playerCreations, beforeRolePlayerCount + 1, 'Role changes never create a duplicate player');
          roleReuse = { samePlayer: true, hostCommands, guestCommands: 0, departedHostCommands: 0,
            playerCreations: 1, realSeekPoller: true, externalPlaybackMocked: true };
        }
        return { queuedAudioControl: { stalePauseBroadcasts: 0, legitimatePauseBroadcasts: 1 },
          retiredVideoMetadataIgnored: true, observations, latestPendingStateViaAppFollow: { position: 20, playing: false }, localActivations, roleReuse,
          productionPartyStateHandler: true, mockedYouTubeAPI: true, actualYouTubePlayback: false };
      } finally {
        PartyUI.close();
        clearInterval(window._ytSeekTimer);
        ytPlayer = null;
        ytPlayerReady = false;
        ytId = null;
        ytReady = originalReady;
        window.YT = originalYT;
      }
    });
  }
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
