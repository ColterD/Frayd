const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');

function panel() {
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
    localStorage: { frayd_identity: 'panel-test' },
    esc: value => String(value),
    WebSocket: class {
      constructor(url) { this.url = url; this.readyState = 1; this.closed = 0; this.sent = []; sockets.push(this); }
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
  p.run('PartyUI.open()');
  assert.match(p.nodes.get('#party-stage').innerHTML, /Start a party/);
  assert.doesNotMatch(p.nodes.get('#party-stage').innerHTML, /ABCD|Song/);
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
