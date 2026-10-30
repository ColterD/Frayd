const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');

const { randomUUID } = require('node:crypto');
const hostResponse = code => ({ code, host_capability: randomUUID() });

function panel(fetch) {
  const nodes = new Map();
  const requests = [];
  function parse(html) {
    for (const match of html.matchAll(/<[^>]+\bid="([^"]+)"[^>]*>/g)) nodes.set('#' + match[1], element());
    for (const match of html.matchAll(/<button data-media="(\d+)" data-title="([^"]*)" class="pt-request"/g)) {
      const button = element();
      button.dataset = { media: match[1], title: match[2] };
      requests.push(button);
    }
  }
  function element() {
    const classes = new Set();
    let html = '';
    return {
      value: '', dataset: {},
      classList: { add: name => classes.add(name), remove: name => classes.delete(name), contains: name => classes.has(name) },
      get innerHTML() { return html; },
      set innerHTML(value) { html = value; requests.length = 0; parse(value); },
      insertAdjacentHTML(position, value) { html = value + html; },
      focus() { context.document.activeElement = this; },
      setSelectionRange(start, end, direction) { this.selectionStart = start; this.selectionEnd = end; this.selectionDirection = direction; },
      addEventListener() {},
    };
  }
  parse(readFileSync(join(__dirname, '../client/index.html'), 'utf8'));
  const sockets = [];
  let loaded;
  let hookCount = 0;
  let hostSend;
  const context = vm.createContext({
    document: {
      querySelector: selector => nodes.get(selector) || null,
      querySelectorAll: selector => selector === '.pt-request' ? requests : [],
      addEventListener: (name, handler) => { if (name === 'DOMContentLoaded') loaded = handler; },
    },
    fetch,
    localStorage: { frayd_identity: 'panel-test' },
    esc: value => String(value),
    WebSocket: class {
      constructor(url, protocols) { this.url = url; this.protocols = Array.from(protocols || []); this.readyState = 1; this.closed = 0; this.sent = []; sockets.push(this); }
      close() { this.closed++; this.readyState = 3; }
      send(value) { this.sent.push(JSON.parse(value)); }
    },
    getLibrary: () => [{ id: 8, kind: 'music', title: 'Song' }],
    partyHostHook: send => { hookCount++; hostSend = send; },
    partyFollow() {},
  });
  context.window = context;
  context.$ = selector => nodes.get(selector) || null;
  vm.runInContext(readFileSync(join(__dirname, '../client/js/party.js'), 'utf8'), context);
  loaded();
  const run = source => vm.runInContext(source, context);
  function joinParty(code = 'ABCD', host = false) {
    run('PartyUI.open()');
    nodes.get('#party-code-input').value = code;
    nodes.get('#pt-join').onclick();
    const socket = sockets.at(-1);
    socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: host, state: { title: 'Song', media_id: 8, position: 2, playing: true, queue: [] } }) });
    return socket;
  }
  return { run, nodes, sockets, context, requests, joinParty, hookCount: () => hookCount, hostSend: value => hostSend(value) };
}

test('Hide keeps the session and updates; the Party opener restores the code without another socket', () => {
  const p = panel();
  const socket = p.joinParty();
  assert.equal(typeof p.nodes.get('#pt-hide')?.onclick, 'function');
  p.nodes.get('#pt-hide').onclick();
  assert.equal(p.nodes.get('#party-modal').classList.contains('on'), false);
  assert.equal(socket.closed, 0);
  socket.onmessage({ data: JSON.stringify({ type: 'members', members: ['hidden guest'] }) });
  p.nodes.get('#party-btn').onclick();
  assert.equal(p.sockets.length, 1);
  assert.match(p.nodes.get('#party-stage').innerHTML, /Code: <b>ABCD<\/b>/);
  assert.match(p.nodes.get('#party-stage').innerHTML, /hidden guest/);
  p.requests[0].onclick();
  assert.deepEqual(socket.sent, [{ type: 'request', media_id: 8, title: 'Song' }]);
});

test('Leave disconnects and reopening starts a fresh panel', () => {
  const p = panel();
  const socket = p.joinParty('ABCD', true);
  p.nodes.get('#pt-close').onclick();
  assert.equal(socket.closed, 1);
  assert.equal(p.context.partySend, null);
  p.run('PartyUI.open()');
  assert.match(p.nodes.get('#party-stage').innerHTML, /Start a party/);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /ABCD|Song/);
});

test('remote close while hidden clears host state and permits a new session', () => {
  const p = panel();
  const socket = p.joinParty('ABCD', true);
  p.nodes.get('#party-modal').classList.remove('on');
  socket.onclose();
  assert.equal(p.context.partySend, null);
  assert.match(p.nodes.get('#party-stage').innerHTML, /Party ended/);
  p.run('PartyUI.open()');
  assert.match(p.nodes.get('#party-stage').innerHTML, /Start a party/);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /ABCD|Song/);
});

test('error followed by close before admission keeps the join rejection visible', () => {
  const p = panel();
  p.run('PartyUI.open()');
  p.nodes.get('#party-code-input').value = 'ABCD';
  p.nodes.get('#pt-join').onclick();
  const socket = p.sockets[0];
  socket.onerror();
  assert.match(p.nodes.get('#party-stage').innerHTML, /Party not found or rejected/);
  socket.onclose();
  assert.match(p.nodes.get('#party-stage').innerHTML, /Start a party/);
  assert.match(p.nodes.get('#party-stage').innerHTML, /Party not found or rejected/);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /Party ended/);
});

test('old socket callbacks cannot clear or change a replacement connection', () => {
  const p = panel();
  const old = p.joinParty('ABCD');
  p.run('PartyUI.close()');
  const current = p.joinParty('EFGH');
  old.onclose();
  old.onerror();
  old.onmessage({ data: JSON.stringify({ type: 'ended' }) });
  p.run('PartyUI.open()');
  assert.match(p.nodes.get('#party-stage').innerHTML, /Code: <b>EFGH<\/b>/);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /Party ended|rejected/);
  assert.equal(current.closed, 0);
});

test('rehosting rebinds the sender once; old host hooks cannot broadcast as a guest', () => {
  const p = panel();
  const first = p.joinParty('ABCD', true);
  p.run('PartyUI.close()');
  const second = p.joinParty('EFGH', true);
  assert.equal(typeof p.context.partySend, 'function');
  assert.equal(p.hookCount(), 1);
  p.context.partySend({ type: 'play', position: 3 });
  assert.equal(first.sent.length, 0);
  assert.deepEqual(second.sent, [{ type: 'play', position: 3 }]);
  p.run('PartyUI.close()');
  const guest = p.joinParty('IJKL');
  p.hostSend({ type: 'pause', position: 4 });
  assert.deepEqual(guest.sent, []);
  p.requests[0].onclick();
  assert.deepEqual(guest.sent, [{ type: 'request', media_id: 8, title: 'Song' }]);
});

test('reopening during connection preserves the connecting code and socket', () => {
  const p = panel();
  p.run('PartyUI.open()');
  p.nodes.get('#party-code-input').value = 'ABCD';
  p.nodes.get('#pt-join').onclick();
  p.run('PartyUI.hide(); PartyUI.open()');
  assert.equal(p.sockets.length, 1);
  assert.match(p.nodes.get('#party-stage').innerHTML, /Connecting to <b>ABCD<\/b>/);
  assert.equal(p.sockets[0].closed, 0);
});

test('ended message clears a hidden connection and closes its socket', () => {
  const p = panel();
  const socket = p.joinParty('ABCD', true);
  p.run('PartyUI.hide()');
  socket.onmessage({ data: JSON.stringify({ type: 'ended' }) });
  assert.equal(p.nodes.get('#party-modal').classList.contains('on'), false);
  assert.equal(p.context.partySend, null);
  assert.equal(socket.closed, 1);
  socket.onclose();
  p.run('PartyUI.open()');
  assert.match(p.nodes.get('#party-stage').innerHTML, /Start a party/);
});

test('Hide and Leave labels describe their distinct actions', () => {
  const html = readFileSync(join(__dirname, '../client/index.html'), 'utf8');
  assert.match(html, /id="pt-hide" aria-label="Hide controls \(stay connected\)"[^>]*>Hide<\/button>/);
  assert.match(html, /id="pt-close" aria-label="Leave party"[^>]*>Leave<\/button>/);
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

for (const phase of ['HTTP', 'JSON']) {
  test(`overlapping Start clicks during pending ${phase} keep the first host socket alive`, async () => {
    const responses = [];
    const body = deferred();
    const bodyStarted = deferred();
    const p = panel(() => {
      const response = deferred();
      responses.push(response);
      return response.promise;
    });
    p.run('PartyUI.open()');
    const start = p.nodes.get('#pt-host').onclick;
    const first = start();
    if (phase === 'JSON') {
      responses[0].resolve({ ok: true, json: () => { bodyStarted.resolve(); return body.promise; } });
      await bodyStarted.promise;
    }
    p.run('PartyUI.hide(); PartyUI.open()');
    const second = p.nodes.get('#pt-host').onclick();
    if (phase === 'HTTP') responses[0].resolve({ ok: true, json: async () => hostResponse('ABCD') });
    else body.resolve(hostResponse('ABCD'));
    await first;
    const socket = p.sockets[0];
    socket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true, state: { title: 'Song', position: 0 } }) });
    for (const response of responses.slice(1)) response.resolve({ ok: true, json: async () => hostResponse('ABCD') });
    await second;
    assert.equal(socket.closed, 0, 'second Start must not close the first host socket and end its party');
    assert.equal(p.sockets.length, 1);
    assert.equal(responses.length, 1);
    assert.match(p.nodes.get('#party-stage').innerHTML, /you are host/);
    p.run('PartyUI.close(); PartyUI.open()');
    const next = p.nodes.get('#pt-host').onclick();
    responses.at(-1).resolve({ ok: true, json: async () => hostResponse('EFGH') });
    await next;
    assert.equal(p.sockets.length, 2);
    assert.match(p.sockets[1].url, /EFGH/);
  });
}

for (const failure of ['HTTP', 'network', 'JSON']) {
  test(`Start can retry after ${failure} failure`, async () => {
    let calls = 0;
    const p = panel(async () => {
      calls++;
      if (calls > 1) return { ok: true, json: async () => hostResponse('ABCD') };
      if (failure === 'network') throw new Error('offline');
      return { ok: failure !== 'HTTP', status: 503, json: async () => { throw new Error('invalid JSON'); } };
    });
    p.run('PartyUI.open()');
    await p.nodes.get('#pt-host').onclick();
    assert.equal(p.sockets.length, 0);
    assert.match(p.nodes.get('#party-stage').innerHTML, /Could not start party/);
    await p.nodes.get('#pt-host').onclick();
    assert.equal(p.sockets.length, 1);
  });
}

for (const phase of ['HTTP', 'JSON']) {
  for (const action of ['Leave', 'Join']) {
    test(`${action} cancels a Start pending ${phase} without stale connection resurrection`, async () => {
      const response = deferred();
      const body = deferred();
      const bodyStarted = deferred();
      const p = panel(() => response.promise);
      p.run('PartyUI.open()');
      const start = p.nodes.get('#pt-host').onclick();
      if (phase === 'JSON') {
        response.resolve({ ok: true, json: () => { bodyStarted.resolve(); return body.promise; } });
        await bodyStarted.promise;
      }
      let guest;
      if (action === 'Leave') p.nodes.get('#pt-close').onclick();
      else guest = p.joinParty('EFGH');
      if (phase === 'HTTP') response.resolve({ ok: true, json: async () => hostResponse('ABCD') });
      else body.resolve(hostResponse('ABCD'));
      await start;
      assert.equal(p.sockets.length, action === 'Join' ? 1 : 0);
      if (guest) {
        assert.equal(guest.closed, 0);
        assert.deepEqual(guest.protocols, ['frayd-party'], 'cancelled Start must never transfer its proof to Join');
        assert.match(p.nodes.get('#party-stage').innerHTML, /Code: <b>EFGH/);
      }
    });
  }
}

test('a cancelled Start failure cannot clear a newer pending Start or paint its error', async () => {
  const responses = [];
  const oldBody = deferred();
  const bodyStarted = deferred();
  const p = panel(() => {
    const response = deferred();
    responses.push(response);
    return response.promise;
  });
  p.run('PartyUI.open()');
  const old = p.nodes.get('#pt-host').onclick();
  responses[0].resolve({ ok: true, json: () => { bodyStarted.resolve(); return oldBody.promise; } });
  await bodyStarted.promise;
  p.run('PartyUI.close(); PartyUI.open()');
  const current = p.nodes.get('#pt-host').onclick();
  oldBody.reject(new Error('old body failed'));
  await old;
  await p.nodes.get('#pt-host').onclick();
  assert.equal(responses.length, 2, 'old completion must not unlock the current Start');
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /Could not start party/);
  responses[1].resolve({ ok: true, json: async () => hostResponse('EFGH') });
  await current;
  assert.equal(p.sockets.length, 1);
  assert.match(p.sockets[0].url, /EFGH/);
});


test('Start transfers creator proof only in the protocol offer, with the exact creation identity', async () => {
  const created = hostResponse('ABCD');
  let creator;
  const p = panel(async (_url, options) => {
    creator = JSON.parse(options.body).identity;
    return { ok: true, json: async () => created };
  });
  delete p.context.localStorage.frayd_identity;
  p.run('PartyUI.open()');
  await p.nodes.get('#pt-host').onclick();
  const socket = p.sockets[0];
  assert.equal(socket.protocols.length, 2);
  assert.equal(socket.protocols[0], 'frayd-party');
  assert.equal(socket.protocols[1] === 'frayd-host.' + created.host_capability, true);
  assert.equal(new URL(socket.url, 'http://fixture.invalid').searchParams.get('identity') === creator, true);
  assert.equal(socket.url.includes(created.host_capability), false);
  assert.equal(JSON.stringify(p.context.localStorage).includes(created.host_capability), false);
  assert.equal(p.nodes.get('#party-stage').innerHTML.includes(created.host_capability), false);
  assert.deepEqual(socket.sent, []);
});

test('Join after a host session never reuses its proof', async () => {
  const created = hostResponse('ABCD');
  const p = panel(async () => ({ ok: true, json: async () => created }));
  p.run('PartyUI.open()');
  await p.nodes.get('#pt-host').onclick();
  const host = p.sockets[0];
  p.run('PartyUI.close()');
  const guest = p.joinParty('EFGH');
  assert.equal(host.closed, 1);
  assert.deepEqual(guest.protocols, ['frayd-party']);
  assert.equal(guest.url.includes(created.host_capability), false);
  assert.equal(JSON.stringify(p.context.localStorage).includes(created.host_capability), false);
});

for (const malformed of [undefined, '', 'invalid']) {
  test('Start with missing or malformed proof does not attempt an insecure host connection: ' + typeof malformed, async () => {
    const p = panel(async () => ({ ok: true, json: async () => ({ code: 'ABCD', host_capability: malformed }) }));
    p.run('PartyUI.open()');
    await p.nodes.get('#pt-host').onclick();
    assert.equal(p.sockets.length, 0);
    assert.match(p.nodes.get('#party-stage').innerHTML, /Could not start party/);
  });
}


test('server display label marks self without rendering the stored identity, and stale labels retire', () => {
  const p = panel();
  const old = p.joinParty('ABCD', true);
  const snapshot = (socket, label, host = false) => socket.onmessage({ data: JSON.stringify({
    type: 'state', is_host: host, self_label: label,
    state: { title: 'Song', queue: [] },
  }) });
  const members = socket => socket.onmessage({ data: JSON.stringify({ type: 'members', members: ['Host (host)', 'Guest 1', 'Guest 2'] }) });
  snapshot(old, 'Host', true);
  members(old);
  assert.match(p.nodes.get('#party-stage').innerHTML, /Host \(host\) <span class="s">\(you\)/);
  assert.equal(p.nodes.get('#party-stage').innerHTML.includes(p.context.localStorage.frayd_identity), false);
  p.run('PartyUI.close()');
  const current = p.joinParty('EFGH');
  members(current);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /\(you\)/);
  snapshot(current, 'Guest 2');
  members(current);
  snapshot(old, 'Guest 1');
  assert.match(p.nodes.get('#party-stage').innerHTML, /Guest 2 <span class="s">\(you\)/);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /Guest 1 <span class="s">\(you\)/);
  current.onmessage({ data: JSON.stringify({ type: 'state', state: { title: 'Broadcast', queue: [] } }) });
  assert.match(p.nodes.get('#party-stage').innerHTML, /Guest 2 <span class="s">\(you\)/);
  current.onmessage({ data: JSON.stringify({ type: 'ended' }) });
  const next = p.joinParty('IJKL');
  members(next);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /\(you\)/);
});

for (const retire of ['Leave', 'remote close', 'ended', 'replacement']) {
  test(`self label retires on ${retire} and old string membership still renders`, () => {
    const p = panel();
    const old = p.joinParty('ABCD');
    old.onmessage({ data: JSON.stringify({ type: 'state', self_label: 'Guest 1', state: { queue: [] } }) });
    if (retire === 'Leave') p.run('PartyUI.close()');
    if (retire === 'remote close') old.onclose();
    if (retire === 'ended') old.onmessage({ data: JSON.stringify({ type: 'ended' }) });
    const current = p.joinParty('EFGH');
    current.onmessage({ data: JSON.stringify({ type: 'members', members: ['Host (host)', 'Guest 1', 'legacy guest'] }) });
    const html = p.nodes.get('#party-stage').innerHTML;
    assert.match(html, /legacy guest/);
    assert.doesNotMatch(html, /\(you\)/);
    assert.equal(html.includes(p.context.localStorage.frayd_identity), false);
  });
}


test('host library to external to same library loads the returning queue item once', () => {
  const p = panel();
  const calls = [];
  p.context.partyFollow = (...args) => calls.push(['native', args[0]]);
  p.context.partyFollowExternal = (...args) => calls.push(['external', args[0]]);
  const socket = p.joinParty('ABCD', true);
  const snapshot = state => socket.onmessage({data:JSON.stringify({type:'state',state})});
  snapshot({external_url:'abcdefghijk',title:'External',queue:[],position:0,playing:true});
  snapshot({media_id:8,title:'Song',queue:[],position:0,playing:true});
  snapshot({media_id:8,title:'Song',queue:[],position:2,playing:true});
  assert.deepEqual(calls, [['native',8],['external','abcdefghijk'],['native',8]]);
});


test('host state and member echoes preserve unfinished YouTube draft, focus and caret; session replacement retires it', () => {
  const p = panel();
  const old = p.joinParty('ABCD', true);
  let input = p.nodes.get('#pt-yt');
  input.value = 'https://youtu.be/unfinished';
  input.focus();
  input.setSelectionRange(8, 13, 'backward');
  old.onmessage({data:JSON.stringify({type:'state',state:{title:'Song',media_id:8,position:4,queue:[]}})});
  input = p.nodes.get('#pt-yt');
  assert.equal(input.value, 'https://youtu.be/unfinished');
  assert.equal(p.context.document.activeElement, input);
  assert.equal(input.selectionStart, 8);
  assert.equal(input.selectionEnd, 13);
  assert.equal(input.selectionDirection, 'backward');
  old.onmessage({data:JSON.stringify({type:'members',members:['Host (host)']})});
  assert.equal(p.nodes.get('#pt-yt').value, input.value);
  p.run('PartyUI.close()');
  // This fixture map retains detached IDs; the browser removes the old host input on close.
  p.nodes.delete('#pt-yt');
  p.joinParty('EFGH', true);
  assert.equal(p.nodes.get('#pt-yt').value, '');
});
