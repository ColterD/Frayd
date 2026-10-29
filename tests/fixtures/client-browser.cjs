const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const vm = require('node:vm');

const clientPath = join(__dirname, '..', '..', 'client');

// Only browser boundaries are doubled: the complete application scripts run as-is.
function browser({ now, storage = {}, initialResponse } = {}) {
  const nodes = new Map();
  const listeners = new Map();
  const documentCaptures = new Map();
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
      removeEventListener(name, callback) {
        events.set(name, (events.get(name) || []).filter(handler => handler.callback !== callback));
      },
      listenerCount(name) { return (events.get(name) || []).length; },
      emit(name) {
        for (const handler of documentCaptures.get(name) || []) handler({ target: this, type: name });
        const handlers = events.get(name) || [];
        events.set(name, handlers.filter(handler => !handler.once));
        for (const handler of handlers) handler.callback({ target: this, type: name, preventDefault() {} });
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
  let nextIntervalId = 0;
  let response = initialResponse || (async () => ({ ok: true, status: 200, json: async () => null }));
  const document = {
    removeEventListener(name, callback, capture) {
      if (capture === true) documentCaptures.set(name, (documentCaptures.get(name) || []).filter(handler => handler !== callback));
    },
    body: element(), querySelector: selector => nodes.get(selector) || null,
    getElementById: id => nodes.get('#' + id), querySelectorAll: () => [],
    createElement: element,
    addEventListener(name, callback, capture) {
      if (capture === true) documentCaptures.set(name, [...(documentCaptures.get(name) || []), callback]);
    },
  };
  const sockets = [];
  const context = vm.createContext({
    document, localStorage: storage, URL, location: { hostname: 'test.invalid', origin: 'https://test.invalid' },
    console: { log() {}, warn() {} }, innerWidth: 1000, innerHeight: 800,
    fetch: async (url, options) => { requests.push({ url, options }); return response(url, options); },
    addEventListener: (name, fn) => listeners.set(name, fn),
    scrollTo() {},
    setTimeout() {}, clearTimeout() {},
    setInterval: fn => { const id = ++nextIntervalId; intervals.set(id, fn); return id; },
    clearInterval: id => intervals.delete(id),
    requestAnimationFrame: fn => fn(),
    confirm: () => false,
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
      close() { this.readyState = 3; }
    },
  });
  context.window = context;
  // Date is this VM's intrinsic; the host clock and other browser contexts stay independent.
  if (now) vm.runInContext('Date', context).now = now;
  vm.runInContext(readFileSync(join(clientPath, 'js/app.js'), 'utf8'), context);
  return {
    context, nodes, requests, intervals, sockets,
    captureCount: name => (documentCaptures.get(name) || []).length,
    run: source => vm.runInContext(source, context),
    respond: fn => { response = fn; },
    dispatchMessage: event => listeners.get('message')(event),
    message: (data, overrides = {}) => listeners.get('message')({ data: { generation: vm.runInContext('ebook?.document?.generation', context), ...data }, origin: 'null', source: nodes.get('#em-frame').contentWindow, ...overrides }),
    page: (page, pages) => listeners.get('message')({
      data: { type: 'page', page, pages, generation: vm.runInContext('ebook?.document?.generation', context) }, origin: 'null', source: nodes.get('#em-frame').contentWindow,
    }),
  };
}

module.exports = { browser };
