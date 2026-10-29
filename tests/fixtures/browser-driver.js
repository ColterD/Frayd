/* global ebook, showChunk, LIBRARY, plyr, partyPlaybackKind, openVideo, PartyUI */
'use strict';
function responsiveOverflowInventory() {
  const main = document.getElementById('main');
  const edge = main.getBoundingClientRect().left + main.clientWidth;
  function selector(node) {
    if (node.id) return '#' + node.id;
    const classes = typeof node.className === 'string' ? node.className.trim().split(/\s+/).filter(Boolean).join('.') : '';
    return node.tagName.toLowerCase() + (classes ? '.' + classes : '') + ':nth-child(' + ([...node.parentElement.children].indexOf(node) + 1) + ')';
  }
  return [main, ...main.querySelectorAll('*')].map(node => {
    const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
    let clippingAncestor = null;
    for (let parent = node.parentElement; parent && parent !== main; parent = parent.parentElement) {
      if (['hidden', 'clip'].includes(getComputedStyle(parent).overflowX)) { clippingAncestor = selector(parent); break; }
    }
    return { selector: selector(node), parent: node.parentElement ? selector(node.parentElement) : null,
      text: node.textContent.trim().slice(0, 90), rect: { left: rect.left, right: rect.right, top: rect.top, width: rect.width, height: rect.height },
      clientWidth: node.clientWidth, scrollWidth: node.scrollWidth, directMainChild: node.parentElement === main,
      width: style.width, minWidth: style.minWidth, display: style.display, flex: style.flex,
      gridTemplateColumns: style.gridTemplateColumns, overflowX: style.overflowX,
      clippingAncestor, exceedsMainRight: rect.right > edge + 1 };
  }).filter(item => !item.clippingAncestor && (item.selector === '#main' || item.directMainChild
    || item.exceedsMainRight || item.scrollWidth > item.clientWidth + 1)).slice(0, 100);
}
async function isolatedResponsiveCase(width) {
  const fixture = window.__responsiveFixture;
  const items = fixture.items;
  const measured = { viewport: { width: innerWidth, height: innerHeight }, freshDocument: true };
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  function check(value, message) { if (!value) throw new Error(message); }
  function equal(actual, expected, message) { check(JSON.stringify(actual) === JSON.stringify(expected), message); }
  async function until(predicate, label) {
    const deadline = Date.now() + 4000;
    while (!(await predicate())) { check(Date.now() < deadline, 'Timed out: ' + label); await delay(20); }
  }
  async function api(path, body) {
    const response = await fetch('/__fixture/' + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
    check(response.ok, 'Owned fixture control failed'); return response.json();
  }
  const key = (key, accept = false) => api('key', { key, accept });
    try {
      await until(() => account?.isAdmin && LIBRARY.length === items.length, 'fresh document authenticates with private fixture model');
      check(!document.getElementById('party-modal').classList.contains('on') && !document.getElementById('playerbar').classList.contains('on'), 'Fresh document starts with clean party/player state');
      // Measure existing production form nodes; only signed-out visibility is modeled.
      const login = document.getElementById('login'), app = document.getElementById('app');
      const loginStyle = login.getAttribute('style'), appClass = app.className, bodyClass = document.body.className;
      const rect = node => { const r = node.getBoundingClientRect(); return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height }; };
      const contains = (outer, inner) => inner.left >= outer.left - 1 && inner.right <= outer.right + 1 && inner.top >= outer.top - 1 && inner.bottom <= outer.bottom + 1;
      try {
        app.classList.remove('on'); login.style.display = 'flex'; document.body.classList.add('auth');
        const card = rect(document.querySelector('.auth-card'));
        measured.loginGeometry = ['email', 'password', 'pw-toggle'].map(id => ({ id, ...rect(document.getElementById(id)) }));
        check(document.documentElement.scrollWidth <= innerWidth + 1, 'Sign-in document fits viewport without hiding overflow');
        measured.loginGeometry.forEach(field => check(field.width > 0 && contains(card, field), 'Sign-in input/reveal fits auth card: ' + field.id));
        const reveal = document.getElementById('pw-toggle'), r = rect(reveal);
        check(reveal.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)), 'Password reveal accepts pointer hit');
        // Use the exact production boot display, then shrink the real iframe viewport.
        const height = width === 320 ? 568 : width === 768 ? 600 : 1000;
        try {
          await api('viewport', { width, height });
          await until(() => innerHeight === height, 'short authentication viewport settles');
          scrollTo(0, 0);
          const brandRect = rect(login.querySelector('.auth-brand')), cardRect = rect(login.querySelector('.auth-card')), footerRect = rect(login.querySelector('.auth-footer'));
          measured.authFlowGeometry = { width, height, brand: brandRect, card: cardRect, footer: footerRect, controls: [] };
          check(brandRect.top >= 0 && brandRect.bottom <= cardRect.top && cardRect.bottom <= footerRect.top, 'Authentication brand, form and guest footer never overlap at short height');
          check(document.documentElement.scrollWidth <= innerWidth + 1, 'Short authentication document stays within viewport width');
          const controls = [...login.querySelectorAll('input,button,a')];
          measured.authFlowGeometry.controlOrder = controls.map(control => control.id || control.getAttribute('type'));
          equal(measured.authFlowGeometry.controlOrder, ['email', 'password', 'pw-toggle', 'submit', 'guest'], 'Authentication offers exactly its five implemented controls');
          check(controls[0].type === 'email' && controls[0].required && controls[1].type === 'password' && controls[1].required, 'Native sign-in field validation remains enabled');
          check(controls[3].type === 'submit' && controls[3].form === document.getElementById('login-form'), 'Sign in retains native form submission');
          for (const control of controls) {
            control.scrollIntoView({ block: 'center', inline: 'nearest' });
            control.focus();
            const bounds = rect(control), hit = document.elementFromPoint(bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
            check(bounds.width > 0 && bounds.top >= -1 && bounds.bottom <= innerHeight + 1 && bounds.left >= -1 && bounds.right <= innerWidth + 1, 'Authentication control stays reachable through natural document scrolling');
            check((hit === control || control.contains(hit)) && document.activeElement === control, 'Authentication control accepts pointer hit and keyboard focus');
            if (control.id === 'pw-toggle') {
              const password = document.getElementById('password'), beforeType = password.type;
              const point = { x: bounds.left + bounds.width / 2, y: bounds.top + bounds.height / 2 };
              measured.authFlowGeometry.revealClick = await api('click', point);
              check(password.type !== beforeType, 'Trusted pointer click activates password reveal after scrolling');
              await api('click', point);
              check(password.type === beforeType, 'Second pointer click restores password masking');
            }
            measured.authFlowGeometry.controls.push({ id: control.id || control.textContent.trim(), bounds, scrollY });
          }
          controls[0].focus();
          for (const control of controls.slice(1)) {
            await key('Tab');
            check(document.activeElement === control, 'Native Tab follows the supported sign-in control order');
          }
          measured.authFlowGeometry.tabOrderVerified = true;
          measured.authFlowGeometry.needsScroll = document.documentElement.scrollHeight > innerHeight + 1;
          if (measured.authFlowGeometry.needsScroll) check(measured.authFlowGeometry.controls.some(control => control.scrollY > 0), 'Overflowing sign-in content remains reachable by natural scrolling');
          else check(footerRect.bottom <= innerHeight + 1, 'Compact sign-in content fits without requiring artificial scrolling');
        } finally {
          await api('viewport', { width, height: 1000 });
          await until(() => innerHeight === 1000, 'original viewport height restored');
          scrollTo(0, 0);
        }

      } finally {
        if (loginStyle === null) login.removeAttribute('style'); else login.setAttribute('style', loginStyle);
        app.className = appClass; document.body.className = bodyClass;
      }
      // Measure folder shelves independently before entering the real item drill paths.
      LIBRARY = items.map((item, index) => ['music', 'ebook', 'audiobook'].includes(item.kind)
        ? { ...item, author: `Folder author ${index % 4}` } : item);
      showsDrill = null;
      authorDrill.music = authorDrill.ebook = authorDrill.audiobook = null;
      await filterKind('all', '');
      const shelves = [...document.querySelectorAll('#main > section')];
      equal(shelves.map(shelf => shelf.querySelector('h3')?.textContent),
        ['Movies', 'TV Shows', 'Music', 'Audiobooks', 'Ebooks', 'Comics', 'Manga'].map(name => 'Recently Added ' + name), 'Seven separately headed dashboard siblings');
      measured.folderGrids = shelves.filter(shelf => /TV Shows|Music|Audiobooks|Ebooks/.test(shelf.querySelector('h3').textContent)).map(shelf => {
        const grid = shelf.querySelector('.cards');
        const boxes = [...grid.children].map(node => node.getBoundingClientRect());
        const columns = getComputedStyle(grid).gridTemplateColumns;
        check(boxes.length >= 4 && getComputedStyle(grid).display === 'grid', 'Multiple folders use actual production grid');
        check(grid.scrollWidth <= grid.clientWidth + 1, 'Folder grid fits available width');
        if (width >= 768) check(boxes[1].left > boxes[0].left && Math.abs(boxes[1].top - boxes[0].top) < 1, 'Multiple TV/author folders lay out horizontally');
        return { heading: shelf.querySelector('h3').textContent, columns, count: boxes.length, boxes: boxes.map(box => ({left: box.left, top: box.top, width: box.width})) };
      });
      LIBRARY = items;
      authorDrill.music = { author: 'Fixture artist' };
      authorDrill.ebook = { author: 'Fixture author' };
      authorDrill.audiobook = { author: 'Fixture author' };
      await filterKind('all', '');
      const hero = document.getElementById('hero'), heroRect = rect(hero);
      check(hero.style.display === 'block', 'Actual renderHero retains its production inline block display');
      const title = hero.querySelector('h2'), copy = hero.querySelector('.hero-copy'), tags = [...hero.querySelectorAll('.tag')];
      measured.heroGeometry = { hero: heroRect, title: rect(title), copy: rect(copy), tags: tags.map(rect), play: rect(hero.querySelector('.btn-play')) };
      for (const node of [title, copy, ...tags, hero.querySelector('.btn-play')]) {
        check(contains(heroRect, rect(node)), 'Hero text/tag/button fits growing hero bounds');
        if (node !== copy) check(node.scrollWidth <= node.clientWidth + 1, 'Hero text wraps without horizontal clipping');
      }
      check(Math.max(...tags.map(node => rect(node).bottom)) <= rect(copy).top, 'Wrapped tags do not overlap hero copy');
      const actualCards = [...document.querySelectorAll('#main .media-card')];
      const portraitItems = items.filter(item => ['ebook', 'comic', 'manga', 'audiobook'].includes(item.kind));
      for (const item of portraitItems) {
        const rendered = actualCards.find(node => node.title.startsWith(item.title));
        check(rendered?.classList.contains('portrait'), `Normal library render derives portrait layout for ${item.kind}`);
      }
      check(!Object.hasOwn(pbAudio, 'readyState'), 'Responsive native audio has no synthetic readyState override');
      measured.nativeAudioReadinessIsUnmodified = true;
      measured.overflowInventory = responsiveOverflowInventory();
      measured.overflowInventoryBeforeAudio = measured.overflowInventory;
      measured.mainBeforeAudio = { client: document.getElementById('main').clientWidth, scroll: document.getElementById('main').scrollWidth };
      const music = items.find(item => item.id === 23);
      const audioCard = actualCards.find(node => node.title.startsWith(music.title));
      check(audioCard, 'Production music card exists for native23');
      audioCard.scrollIntoView({ block: 'center', inline: 'nearest' });
      const clickRect = audioCard.getBoundingClientRect();
      const clickPoint = { x: clickRect.left + clickRect.width / 2, y: clickRect.top + clickRect.height / 2 };
      const clickHit = document.elementFromPoint(clickPoint.x, clickPoint.y);
      check(clickHit === audioCard || audioCard.contains(clickHit), 'Native user click reaches production audio card');
      measured.nativePlaybackGesture = await api('click', clickPoint);
      check(fixture.cardClicks.some(event => event.id === 23 && event.isTrusted), 'Production audio-card click isTrusted');
      await until(() => pbAudio.readyState >= 2 && !pbAudio.paused, 'owned PCM audio-element playback starts');
      const startedAt = pbAudio.currentTime;
      await until(() => !pbAudio.paused && pbAudio.currentTime > startedAt + 0.03, 'native audio time advances');
      check(pbAudio.currentTime < 5, 'Fresh source with progress0 cannot inherit stale203/end position');
      measured.nativeDiagnostics = fixture;
      measured.overflowInventory = responsiveOverflowInventory();
      const sizes = {};
      measured.sizes = sizes;
      for (const [name, element] of [['document', document.documentElement], ['body', document.body], ['main', document.getElementById('main')]]) {
        sizes[name] = { client: element.clientWidth, scroll: element.scrollWidth };
        check(element.scrollWidth <= element.clientWidth + 1, `${width} ${name} must not overflow horizontally: ${JSON.stringify(sizes)}`);
      }
      const grids = [...document.querySelectorAll('#main .cards')].map(grid => {
        const rect = grid.getBoundingClientRect();
        const cards = [...grid.querySelectorAll('.media-card')];
        check(grid.scrollWidth <= grid.clientWidth + 1, 'Card grid fits its available width');
        check(cards.every(c => c.getBoundingClientRect().left >= rect.left - 1 && c.getBoundingClientRect().right <= rect.right + 1), 'Every card fits its grid');
        check(new Set(cards.map(c => Math.round(c.getBoundingClientRect().top))).size > 1, 'Multiple cards wrap onto new rows');
        return { width: grid.clientWidth, columns: getComputedStyle(grid).gridTemplateColumns, rows: new Set(cards.map(c => Math.round(c.getBoundingClientRect().top))).size };
      });
      const ratios = [...document.querySelectorAll('.portrait .thumb,.portrait .thumb-fallback')].map(node => {
        const r = node.getBoundingClientRect(); check(Math.abs(r.width / r.height - 2 / 3) < 0.01, 'Portrait thumbnail/fallback keeps2:3');
        return { tag: node.tagName, width: r.width, height: r.height };
      });
      measured.grids = grids; measured.ratios = ratios;
      equal(ratios.length, portraitItems.length, 'Every normal production book/comic/manga/audiobook portrait measured');
      if (width === 320) {
        await api('viewport', { width, height: 568 });
        await until(() => innerHeight === 568, 'short active-audio viewport settles');
        try {
          const main = document.getElementById('main');
          const lastCard = [...main.querySelectorAll('.media-card')].at(-1);
          const title = lastCard.querySelector('.title');
          const bar = document.getElementById('playerbar');
          main.scrollTop = main.scrollHeight;
          window.scrollTo(0, document.documentElement.scrollHeight);
          const pointHit = node => {
            const box = rect(node);
            const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
            return { box, hit: hit?.id || hit?.className || hit?.tagName, reachable: hit === node || node.contains(hit) };
          };
          measured.shortAudioFooter = { viewport: { width: innerWidth, height: innerHeight }, main: rect(main),
            paddingBottom: getComputedStyle(main).paddingBottom, maxScroll: main.scrollTop, documentScroll: scrollY, documentHeight: document.documentElement.scrollHeight, card: rect(lastCard),
            title: pointHit(title), bar: rect(bar), audioControls: ['pb-play', 'pb-close'].map(id => ({id, ...pointHit(document.getElementById(id))})) };
          check(rect(lastCard).bottom <= rect(bar).top, 'Last library card clears the active wrapped player at maximum scroll');
          check(measured.shortAudioFooter.title.reachable, 'Last library card title center accepts a pointer hit above the player');
          let trustedTitleClick = false;
          title.addEventListener('click', event => { trustedTitleClick = event.isTrusted; event.stopImmediatePropagation(); }, { capture: true, once: true });
          const titleBox = rect(title);
          await api('click', { x: titleBox.left + titleBox.width / 2, y: titleBox.top + titleBox.height / 2 });
          check(trustedTitleClick, 'Trusted pointer input reaches the last library title');
          measured.shortAudioFooter.trustedTitleClick = trustedTitleClick;
          measured.shortAudioFooter.titleClickActionIntercepted = true;
          check(measured.shortAudioFooter.audioControls.every(control => control.reachable), 'Both active audio buttons remain hit-testable');
          lastCard.scrollIntoView({ block: 'center' });
          measured.shortAudioFooter.centeredCard = pointHit(lastCard);
          check(measured.shortAudioFooter.centeredCard.reachable, 'Last library card center is reachable by scrolling');
          equal([...bar.querySelectorAll('button')].map(button => button.id), ['pb-play', 'pb-close'], 'Default audio player retains exactly its two implemented buttons');
          await filterKind('episode', '');
          equal(scrollY, 0, 'Category filtering resets document scroll');
          window.scrollTo(0, document.documentElement.scrollHeight);
          document.querySelector('#main .show-grid .media-card').click();
          equal(scrollY, 0, 'Folder opening resets document scroll');
          window.scrollTo(0, document.documentElement.scrollHeight);
          document.querySelector('#main .back-row').click();
          equal(scrollY, 0, 'Folder back resets document scroll');
          measured.shortAudioFooter.categoryAndBackResetScroll = true;
          await filterKind('all', '');
        } finally {
          await api('viewport', { width, height: 1000 });
          await until(() => innerHeight === 1000, 'short active-audio viewport restores');
        }
      }
      function reachable(node) {
        node.scrollIntoView({ block: 'center', inline: 'nearest' });
        const r = node.getBoundingClientRect(); const style = getComputedStyle(node);
        check(style.display !== 'none' && style.visibility !== 'hidden' && r.width > 0 && r.height > 0, 'Control is visibly laid out');
        check(r.left >= -1 && r.right <= innerWidth + 1, 'Control stays horizontally reachable');
        check(r.top >= -1 && r.bottom <= innerHeight + 1, 'Control stays vertically reachable');
        const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        check(hit === node || node.contains(hit), `Control ${node.id || node.textContent.trim() || node.getAttribute('aria-label')} center accepts a pointer hit; actual=${hit?.id || hit?.className?.baseVal || hit?.className || hit?.tagName}`);
        node.focus(); check(document.activeElement === node, 'Control accepts focus');
      }
      const nav = [...document.querySelectorAll('#sidebar .nav-item')];
      nav.forEach(node => { check(node.textContent.trim().length > 0, 'Navigation label retained'); reachable(node); });
      nav.find(node => node.dataset.kind === 'movie').click(); equal(activeKind, 'movie', 'Visible movie navigation works');
      reachable(document.getElementById('settings-btn')); document.getElementById('settings-btn').click();
      await until(() => !!document.querySelector('.p-scan'), 'Visible Settings opens usable controls');
      const settingsPanel = document.querySelector('#settings-modal .modal-panel');
      const guide = settingsPanel.querySelector('pre');
      const guideText = 'Media/\n  Movies/    *.mp4 *.mkv\n  Shows/     S01E01 files\n  Music/     *.mp3 *.flac\n  Audiobooks/ *.m4b *.mp3\n  Ebooks/    *.epub\n  Comics/    *.cbz *.cbr\n  Manga/     folder of images under a *manga* dir';
      equal(guide.textContent, guideText, 'Suggested layout retains all guide text and indentation');
      measured.settingsGeometry = { panel: rect(settingsPanel), clientWidth: settingsPanel.clientWidth, scrollWidth: settingsPanel.scrollWidth, guideText: guide.textContent, controls: [] };
      check(settingsPanel.scrollWidth <= settingsPanel.clientWidth + 1 && guide.scrollWidth <= guide.clientWidth + 1, 'Settings guide wraps inside panel without horizontal scrolling');
      for (const control of settingsPanel.querySelectorAll('input,button,select')) {
        reachable(control);
        check(contains(rect(settingsPanel), rect(control)) && settingsPanel.scrollLeft === 0, 'Every basic settings control fits panel through vertical scrolling');
        measured.settingsGeometry.controls.push({ id: control.id || control.className, bounds: rect(control), scrollTop: settingsPanel.scrollTop });
      }
      equal(guide.textContent, guideText, 'Scrolling settings preserves the complete guide');

      document.getElementById('settings-close').click();
      reachable(document.getElementById('logout'));
      reachable(document.querySelector('#search input'));
      reachable(document.getElementById('add-media')); document.getElementById('add-media').click();
      await until(() => !document.getElementById('add-media').disabled, 'Visible scan control handles modeled busy response');
      equal([...document.querySelectorAll('#topbar button')].map(button => button.id), ['party-btn', 'add-media'], 'Main toolbar offers implemented Party and Scan controls');
      reachable(document.getElementById('party-btn')); document.getElementById('party-btn').click();
      check(document.getElementById('party-modal').classList.contains('on'), 'Visible party control opens panel');
      const originalSocket = window.WebSocket;
      let layoutSocket;
      window.WebSocket = class {
        constructor() { this.readyState = 1; layoutSocket = this; }
        send() {}
        close() { this.readyState = 3; }
      };
      try {
        const partyPanel = document.querySelector('#party-modal .party-inner');
        const checkPartyRow = (input, button) => {
          reachable(input); reachable(button);
          check(contains(rect(partyPanel), rect(input)) && contains(rect(partyPanel), rect(button)), 'Party entry controls fit their panel');
          check(partyPanel.scrollWidth <= partyPanel.clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth + 1, 'Party panel and document stay within viewport');
          return { panel: rect(partyPanel), input: rect(input), button: rect(button), clientWidth: partyPanel.clientWidth, scrollWidth: partyPanel.scrollWidth };
        };
        const joinInput = document.getElementById('party-code-input'), joinButton = document.getElementById('pt-join');
        measured.partyGeometry = { join: checkPartyRow(joinInput, joinButton), modeledTransport: true };
        joinInput.value = 'LAYO';
        let trustedJoin = false;
        joinButton.addEventListener('click', event => { trustedJoin = event.isTrusted; }, { once: true });
        const joinBounds = rect(joinButton);
        await api('click', { x: joinBounds.left + joinBounds.width / 2, y: joinBounds.top + joinBounds.height / 2 });
        check(trustedJoin && layoutSocket, 'Trusted Join click reaches production connection handler');
        layoutSocket.onmessage({ data: JSON.stringify({ type: 'state', is_host: true, self_label: 'Host', state: { media_id: null, title: '', position: 0, playing: false, queue: [] } }) });
        const ytInput = document.getElementById('pt-yt'), ytPlay = document.getElementById('pt-yt-go');
        measured.partyGeometry.host = checkPartyRow(ytInput, ytPlay);
        let trustedPlay = false;
        ytPlay.addEventListener('click', event => { trustedPlay = event.isTrusted; }, { once: true });
        const playBounds = rect(ytPlay);
        await api('click', { x: playBounds.left + playBounds.width / 2, y: playBounds.top + playBounds.height / 2 });
        check(trustedPlay && document.querySelector('.party-error')?.textContent.includes('Paste a YouTube URL'), 'Trusted host Play click reaches production URL validation');
        measured.partyGeometry.trustedJoin = trustedJoin; measured.partyGeometry.trustedPlay = trustedPlay;
      } finally { PartyUI.close(); window.WebSocket = originalSocket; }
      document.getElementById('pt-close').click();
      const playerControls = [...document.querySelectorAll('#playerbar button')];
      measured.audioToolbarControls = playerControls.map(node => ({ id: node.id, label: node.getAttribute('aria-label') }));
      equal(playerControls.map(node => node.id), ['pb-play', 'pb-close'], 'Audio toolbar exposes its functional controls without inactive Previous/Next buttons');
      playerControls.forEach(reachable);
      let trustedAudioPause = false;
      pbAudio.addEventListener('pause', event => { trustedAudioPause = event.isTrusted; }, { once: true });
      reachable(document.getElementById('pb-play')); document.getElementById('pb-play').click();
      await until(() => pbAudio.paused && trustedAudioPause, 'Visible player receives native trusted pause');
      equal(document.getElementById('pb-play').textContent, '▶', 'Paused native audio shows the play action');
      measured.audioPauseUi = { trustedPause: trustedAudioPause, glyph: document.getElementById('pb-play').textContent };
      document.getElementById('pb-play').click();
      await until(() => !pbAudio.paused && document.getElementById('pb-play').textContent === '❚❚', 'Visible player play restores pause action');
      for (const id of ['pb-vol', 'pb-bar']) {
        const node = document.getElementById(id); const r = node.getBoundingClientRect();
        check(r.width > 0 && r.left >= 0 && r.right <= innerWidth + 1, 'Volume/seek track stays reachable');
        node.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: r.left + r.width / 2 }));
      }
      check(Math.abs(pbAudio.volume - 0.5) < 0.02 && pbAudio.currentTime > 50, 'Volume and seek controls work after wrapping');
      document.getElementById('pb-close').click();
      const deleteButton = document.querySelector('#main .card-del');
      const precedingSort = document.querySelector('#main .row-head button');
      reachable(precedingSort); await key('Tab');
      check(document.activeElement === deleteButton && deleteButton.matches(':focus-visible'), 'Delete stays keyboard reachable with visible focus');
      const before = LIBRARY.length;
      const enter = await key('Enter'); const space = await key(' ');
      check(enter.dialogs.length === 1 && space.dialogs.length === 1 && LIBRARY.length === before, 'Native cancellation retains cards at this viewport');
      check(!document.getElementById('video-modal').classList.contains('on'), 'Delete keys do not open playback');
      measured.modalHeaders = [];
      async function headerAtHeights(selector, titleId, expectedTitle, native = false) {
        try {
          for (const height of width === 320 ? [1000, 568] : width === 768 ? [1000, 600] : [1000]) {
            await api('viewport', { width, height });
            await until(() => innerHeight === height, 'header viewport settles');
            const header = document.querySelector(selector), title = document.getElementById(titleId);
            const bounds = rect(header), titleBounds = rect(title);
            equal(title.textContent, expectedTitle, 'Modal retains complete title text');
            check(bounds.left >= -1 && bounds.right <= innerWidth + 1 && bounds.top >= -1 && bounds.bottom <= innerHeight + 1, 'Modal header stays in viewport');
            const controls = [...header.querySelectorAll('button,input')];
            const controlRects = controls.map(control => {
              const bounds = rect(control), hit = document.elementFromPoint(bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
              check(bounds.width > 0 && contains(rect(header), bounds) && (hit === control || control.contains(hit)), 'Every modal header action has an unobstructed hit target');
              if (!control.disabled) { control.focus(); check(document.activeElement === control, 'Enabled header action accepts focus'); }
              if (native) check(bounds.width === 44 && bounds.height === 44, 'Native header action preserves44px hit area');
              return { id: control.id, disabled: control.disabled, bounds };
            });
            if (native) {
              check(contains(bounds, titleBounds) && bounds.height <= 76, 'Native title fits a bounded one-line overlay');
            } else {
              // Inline reader titles can span lines; inspect painted fragments, not their union.
              const range = document.createRange(); range.selectNodeContents(title);
              for (const fragment of range.getClientRects()) {
                check(fragment.left >= -1 && fragment.right <= innerWidth + 1 && fragment.top >= bounds.top - 1 && fragment.bottom <= bounds.bottom + 1, 'Reader title fragment fits header');
                check(controlRects.every(control => fragment.right <= control.bounds.left + 1 || fragment.left >= control.bounds.right - 1 || fragment.bottom <= control.bounds.top + 1 || fragment.top >= control.bounds.bottom - 1), 'Reader title paint does not overlap actions');
              }
            }
            measured.modalHeaders.push({ width, height, selector, title: title.textContent, titleBounds, bounds, controls: controlRects });
          }
        } finally {
          await api('viewport', { width, height: 1000 });
          await until(() => innerHeight === 1000, 'header viewport restores');
        }
      }
      const previousReaderFetch = window.fetch;
      let releaseHeaderProgress;
      const headerReply = value => new Response(JSON.stringify(value), { headers: { 'Content-Type': 'application/json' } });
      window.fetch = (url, options = {}) => {
        const path = new URL(String(url), location.origin).pathname;
        if (path === '/api/comic/90/pages') return Promise.resolve(headerReply(['0', '1']));
        if (path === '/api/epub/7/spine') return Promise.resolve(headerReply(['one.xhtml']));
        if (path === '/api/epub/7/toc' || path === '/api/annotations/7') return Promise.resolve(headerReply([]));
        if (path === '/api/progress/90' || path === '/api/progress/7') return new Promise(resolve => { releaseHeaderProgress = () => resolve(headerReply(null)); });
        if (path === '/api/epub/7/chunk/0') return Promise.resolve(new Response('<html><head></head><body><p>Owned reader header fixture</p></body></html>', { headers: { 'Content-Type': 'text/html' } }));
        return previousReaderFetch(url, options);
      };
      try {
        const comicTitle = 'Wilhelm Busch - Max und Moritz';
        await openComic({ id: 90, kind: 'comic', title: comicTitle });
        await until(() => releaseHeaderProgress, 'comic header restore reaches held progress');
        check(comic.page === null && document.getElementById('cm-pageinfo').textContent === 'Loading…', 'Comic header uses actual pending state');
        await headerAtHeights('#comic-modal .comic-head', 'cm-title', comicTitle);
        releaseHeaderProgress(); releaseHeaderProgress = null;
        await until(() => comic.page === 0, 'comic header restore completes');
        await headerAtHeights('#comic-modal .comic-head', 'cm-title', comicTitle);
        document.getElementById('cm-close').click();
        const ebookTitle = 'Robert Louis Stevenson - Dr Jekyll and Mr Hyde';
        await openEbook({ id: 7, kind: 'ebook', title: ebookTitle });
        await until(() => releaseHeaderProgress, 'ebook header restore reaches held progress');
        check(ebook.chunk === null && document.getElementById('em-pageinfo').textContent === 'Loading…', 'EPUB header uses actual pending state');
        await headerAtHeights('#ebook-modal .comic-head', 'em-title', ebookTitle);
        releaseHeaderProgress(); releaseHeaderProgress = null;
        await until(() => ebook.document?.ready, 'real sandboxed chapter establishes header ready state');
        await headerAtHeights('#ebook-modal .comic-head', 'em-title', ebookTitle);
      } finally {
        releaseHeaderProgress?.();
        document.getElementById('cm-close').click(); document.getElementById('em-close').click();
        window.fetch = previousReaderFetch;
      }
      // Real production openVideo and decoder; only media/API bytes are fixtures.
      plyr.muted = true;
      const nativeTitle = 'Duck and Cover - http://www.archive.org/details/DuckandC1951';
      openVideo({ id: 21, kind: 'movie', title: nativeTitle });
      await until(() => plyr.media.readyState === 4 && plyr.media.getVideoPlaybackQuality().totalVideoFrames > 0, 'responsive native video decodes');
      document.querySelector('.vm-overlay').classList.remove('hidden');
      await headerAtHeights('.vm-overlay', 'vm-title', nativeTitle, true);
      plyr.toggleControls(true);
      const wrapper = document.querySelector('#video-modal .plyr');
      const nativeControls = document.querySelector('.plyr__controls');
      await until(() => contains(rect(wrapper), rect(nativeControls)), 'revealed native controls settle inside wrapper');
      const viewportWidth = document.documentElement.clientWidth;
      measured.videoGeometry = { viewportWidth, wrapper: rect(wrapper), controls: rect(nativeControls), decodedFrames: plyr.media.getVideoPlaybackQuality().totalVideoFrames };
      check(rect(wrapper).width === viewportWidth && rect(wrapper).height === innerHeight, 'Native Plyr fills viewport');
      check(getComputedStyle(document.getElementById('yt-party')).display === 'none', 'Native mode hides inactive external surface');
      reachable(document.querySelector('.plyr__controls [data-plyr="play"]'));
      // Model only the external-provider layout state; no YouTube playback claim.
      const modal = document.getElementById('video-modal'), external = document.getElementById('yt-party');
      modal.classList.add('yt');
      await headerAtHeights('.vm-overlay', 'vm-title', nativeTitle, true);
      measured.youtubeLayoutModel = { wrapper: rect(wrapper), placeholder: rect(external) };
      check(rect(wrapper).width === 0 && rect(nativeControls).width === 0, 'YouTube mode hides native wrapper and controls');
      check(rect(external).width === viewportWidth && rect(external).height === innerHeight, 'External placeholder fills viewport');
      const iframe = document.createElement('iframe'); iframe.id = 'yt-party'; iframe.title = 'Modeled YouTube surface';
      external.replaceWith(iframe);
      try {
        check(rect(iframe).width === viewportWidth && rect(iframe).height === innerHeight, 'YouTube replacement iframe fills viewport');
        modal.classList.remove('yt');
        check(rect(iframe).width === 0 && rect(wrapper).width === viewportWidth, 'Returning native mode hides external iframe and restores Plyr');
      } finally { iframe.replaceWith(external); modal.classList.remove('yt'); document.getElementById('vm-back').click(); }
      return { viewport: { width: innerWidth, height: innerHeight }, sizes, grids, ratios, folderGrids: measured.folderGrids, shortAudioFooter: measured.shortAudioFooter, layoutGeometry: { login: measured.loginGeometry, authFlow: measured.authFlowGeometry, settings: measured.settingsGeometry, party: measured.partyGeometry, modalHeaders: measured.modalHeaders, hero: measured.heroGeometry, native: measured.videoGeometry, youtubeModel: measured.youtubeLayoutModel }, navigationLabels: nav.map(n => n.textContent.trim()), adminAndPlayerControlsReachable: true, realMediaControl: true, audioPauseUi: measured.audioPauseUi, deletionCancellation: true, nativeDiagnostics: fixture, overflowInventory: measured.overflowInventory };
    } catch (error) {
      error.observed = { ...measured, audio: { readyState: pbAudio.readyState, paused: pbAudio.paused,
        duration: pbAudio.duration, currentTime: pbAudio.currentTime, mediaError: pbAudio.error?.code || null }, nativeDiagnostics: fixture };
      throw error;
    } finally { document.getElementById('pb-close').click(); disposeSettings(); }
}
async function isolatedToolbarRetryCase(mode) {
  const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
  function check(value, message) { if (!value) throw new Error(message); }
  async function until(predicate, label, budget = 4000) {
    const deadline = Date.now() + budget;
    while (!predicate()) { check(Date.now() < deadline, 'Timed out: ' + label); await delay(20); }
  }
  await until(() => account?.isAdmin && LIBRARY.length > 0, 'fresh modeled account resumes');
  const previousFetch = window.fetch;
  let statusReads = 0, libraryReads = 0;
  const postStatuses = [], clicks = [];
  let releaseLibrary;
  const reply = value => new Response(JSON.stringify(value), { status: 200, headers: { 'Content-Type': 'application/json' } });
  const button = document.getElementById('add-media');
  const originalMarkup = button.innerHTML;
  button.addEventListener('click', event => clicks.push(event.isTrusted));
  const initialLibrary = LIBRARY;
  window.fetch = async (url, options = {}) => {
    if (url === '/api/scan') {
      const status = postStatuses.length ? 409 : 202;
      postStatuses.push(status);
      return new Response(null, { status });
    }
    if (url === '/api/scan/status') {
      statusReads++;
      if (statusReads <= (mode === 'transient' ? 1 : 3)) throw Error('Owned transient status transport failure');
      return reply({ scanning: false });
    }
    if (new URL(String(url), location.origin).pathname === '/api/library') {
      libraryReads++;
      if (mode === 'transient') return new Promise(resolve => { releaseLibrary = () => resolve(reply(initialLibrary)); });
      return reply(initialLibrary);
    }
    return previousFetch(url, options);
  };
  async function trustedClick() {
    button.scrollIntoView({ block: 'center' });
    const r = button.getBoundingClientRect();
    const point = { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    check(document.elementFromPoint(point.x, point.y) === button || button.contains(document.elementFromPoint(point.x, point.y)), 'Production toolbar accepts pointer hit');
    const response = await previousFetch('/__fixture/click', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(point) });
    check(response.ok && clicks.at(-1) === true, 'Production scan receives trusted browser click');
  }
  try {
    await trustedClick();
    if (mode === 'transient') {
      await until(() => statusReads === 1, 'first failure delivered');
      check(button.disabled && button.textContent === 'Scanning…', 'Transient failure keeps monitoring active');
      check(document.getElementById('toast-host').textContent.includes('retrying'), 'Transient failure reports automatic retry');
      await until(() => releaseLibrary, 'next idle status starts refresh');
      await delay(1650);
      check(statusReads === 2 && libraryReads === 1 && button.disabled, 'No overlapping polls while library refresh is pending beyond1500ms');
      releaseLibrary();
    } else {
      // Three real1500ms intervals require4500ms;6500ms is scoped to this criterion only.
      await until(() => !button.disabled, 'three failures restore retry control', 6500);
      check(statusReads === 3 && libraryReads === 0, 'Exactly three failures stop without refreshing');
      check(button.innerHTML === originalMarkup && document.getElementById('toast-host').textContent.includes('click Scan to retry monitoring'), 'Manual retry is truthful and available');
      await trustedClick();
      check(button.disabled && document.getElementById('toast-host').textContent.includes('monitoring progress'), 'POST409 reattaches monitoring');
    }
    await until(() => !button.disabled, 'valid idle status completes monitoring');
    check(libraryReads === 1 && button.innerHTML === originalMarkup, 'Successful current-account completion refreshes once and restores control');
    check(document.getElementById('toast-host').textContent.includes('Scan complete'), 'Completion feedback published');
    check(postStatuses.join(',') === (mode === 'transient' ? '202' : '202,409'), 'Expected scan admission statuses');
    return { mode, freshDocument: true, trustedClicks: clicks, postStatuses, statusReads, libraryReads,
      realIntervalMs: 1500, noOverlapDuringHeldRefresh: mode === 'transient', toolbarRestored: true,
      accountName: account.name, modeledAPI: true, actualDOMAndTimers: true };
  } finally { if (releaseLibrary) releaseLibrary(); window.fetch = previousFetch; }
}
async function freshResponsiveDocument(width, toolbarRetryMode = null) {
  const viewport = await fetch('/__fixture/viewport', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ width }) });
  if (!viewport.ok) throw Error('Owned viewport control failed');
  const frame = document.createElement('iframe');
  frame.style.cssText = 'position:fixed;inset:0;width:100vw;height:100vh;border:0;z-index:100000';
  let timer;
  let receive;
  try {
    const result = new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(Error('Fresh responsive document exceeded12s')), 12000);
      receive = event => {
        if (event.source !== frame.contentWindow || event.origin !== location.origin || event.data?.type !== 'responsive-case-result' || event.data.width !== width) return;
        resolve(event.data);
      };
      window.addEventListener('message', receive);
    });
    frame.src = toolbarRetryMode ? '/__fixture/toolbar-retry?width=' + width + '&mode=' + toolbarRetryMode : '/__fixture/responsive?width=' + width;
    document.body.appendChild(frame);
    const packet = await result;
    if (packet.status !== 'PASS' || packet.errors.length) {
      const error = Error(packet.error || JSON.stringify(packet.errors)); error.observed = { ...packet.observed, childErrors: packet.errors }; throw error;
    }
    return packet.observed;
  } finally { clearTimeout(timer); window.removeEventListener('message', receive); frame.remove(); }
}
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
  if (window.__toolbarRetryMode) {
    const width = window.__responsiveFixture.width;
    try {
      const observed = await isolatedToolbarRetryCase(window.__toolbarRetryMode);
      parent.postMessage({ type: 'responsive-case-result', width, status: 'PASS', observed, errors }, location.origin);
    } catch (error) {
      parent.postMessage({ type: 'responsive-case-result', width, status: 'FAIL', error: error.stack, errors }, location.origin);
    }
    return;
  }
  if (window.__responsiveFixture) {
    const width = window.__responsiveFixture.width;
    try {
      const observed = await isolatedResponsiveCase(width);
      parent.postMessage({ type: 'responsive-case-result', width, status: 'PASS', observed, errors }, location.origin);
    } catch (error) {
      parent.postMessage({ type: 'responsive-case-result', width, status: 'FAIL', error: error.stack, observed: error.observed, errors }, location.origin);
    }
    return;
  }
  if (window.__responsiveDiagnosticOnly) {
    await run('fresh isolated responsive diagnostic at768 CSS pixels', () => freshResponsiveDocument(768));
    return api('result', { cases, errors, mediaDiagnostics, nativeMedia, passed: cases.filter(c => c.status === 'PASS').length,
      failed: cases.filter(c => c.status !== 'PASS').length, limits: ['Diagnostic only; full-suite and installer cases are not rerun here.'] });
  }
  const logoutReceipt = sessionStorage.getItem('frayd-qa-logout');
  if (logoutReceipt) {
    const packet = JSON.parse(logoutReceipt);
    await until(() => document.getElementById('login').style.display !== 'none', 'logout returns to login');
    check(!document.getElementById('app').classList.contains('on') && document.body.classList.contains('auth'), 'Signed-out document shows only sign-in');
    check(localStorage.frayd_logout === '1', 'Sign-out remains explicit until re-entry');
    if (!packet.logoutRefreshed) {
      packet.logoutRefreshed = true;
      sessionStorage.setItem('frayd-qa-logout', JSON.stringify(packet));
      location.reload();
      return;
    }
    sessionStorage.removeItem('frayd-qa-logout');
    check(settingsPoll === null && !document.getElementById('settings-modal').classList.contains('on'), 'new document owns no old settings poll or modal');
    document.getElementById('guest').click();
    await until(() => document.getElementById('app').classList.contains('on') && document.getElementById('login').style.display === 'none', 'explicit guest re-entry after repeated sign-in refresh');
    check(localStorage.frayd_logout === undefined, 'Explicit re-entry clears sign-out marker');
    packet.cases.push({ name: 'settings logout repeated reload retains sign-in until guest re-entry and retires old poll', status: 'PASS',
      observed: { actualReload: true, repeatedRefreshSignInOnly: true, explicitGuestReentry: true, settingsPoll: null, previousDocumentGeneration: packet.logoutGeneration } });
    delete packet.logoutGeneration;
    delete packet.logoutRefreshed;
    packet.passed++;
    packet.errors.push(...errors);
    packet.mediaDiagnostics.push(...mediaDiagnostics);
    return api('result', packet);
  }
  const readerFrame = () => document.getElementById('em-frame');
  let sequence = 0;
  function probe(action = 'inspect', details = {}) {
    return new Promise((resolve, reject) => {
      const frame = readerFrame();
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
    await showChunk(0);
    const loaded = new Promise(resolve => readerFrame().addEventListener('load', resolve, { once: true }));
    await loaded;
    await until(() => ebook?.document?.ready, 'current reader ready');
    equal((await probe()).opaqueOrigin, 'null', 'Reader must have opaque sandbox origin');
  }
  function reportProgress(currentCase) {
    // Fixed numeric fields only: no case payloads, URLs, headers or errors.
    // Never await diagnostics or change the production polling cadence.
    fetch('/__fixture/progress', { method: 'POST', keepalive: true,
      headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({
        completed: cases.length, passed: cases.filter(item => item.status === 'PASS').length,
        failed: cases.filter(item => item.status === 'FAIL').length,
        blocked: cases.filter(item => item.status === 'BLOCKED').length, currentCase,
      }) }).catch(() => {});
  }
  async function run(name, fn) {
    reportProgress(cases.length + 1);
    const startedAt = performance.now();
    try { cases.push({ name, status: 'PASS', observed: await fn(), elapsedMs: performance.now() - startedAt }); }
    catch (error) { cases.push({ name, status: error.code === 'NATIVE_CODEC_UNAVAILABLE' ? 'BLOCKED' : 'FAIL', error: error.stack, elapsedMs: performance.now() - startedAt, ...(error.observed ? { observed: error.observed } : {}) }); }
    reportProgress(null);
  }
  async function highlight({ body, start, end, text, offset, search, saved = [] }) {
    await api('configure', { body, annotations: saved });
    ebook = { it: { id: 7 }, spine: ['chapter.xhtml'], chunk: 0, annots: saved.slice() };
    document.getElementById('ebook-modal').classList.add('on');
    await load();
    if (search) {
      readerFrame().contentWindow.postMessage({ type: 'search', query: search }, '*');
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
      let pauseProgress = null;
      if (id === 21 && expectedPosition === 0) {
        const originalFetch = window.fetch;
        const saves = [];
        const media = plyr.media;
        let trustedPause = false;
        const onPause = event => { trustedPause = event.isTrusted; };
        media.addEventListener('pause', onPause, { once: true });
        window.fetch = (url, options) => {
          if (String(url) === '/api/progress' && options?.method === 'POST') saves.push(JSON.parse(options.body));
          return originalFetch(url, options);
        };
        try {
          check(plyr.currentTime > 0 && plyr.currentTime < 10, 'Pause before the ten-second periodic save threshold');
          plyr.pause();
          await until(() => saves.length > 0, 'actual Plyr pause event saves local video progress');
          check(trustedPause && media.paused, 'Native pause is trusted and delivered through vendored Plyr');
          equal(saves.length, 1, 'One pause event saves progress once');
          equal(saves[0].media_id, id, 'Pause saves the indexed video identity');
          check(Math.abs(saves[0].position - media.currentTime) < 0.01 && saves[0].position > 0,
            'Pause saves the actual position before a periodic save can run');
          pauseProgress = { trustedPause, periodicThreshold: 10, saves };
        } finally {
          window.fetch = originalFetch;
          media.removeEventListener('pause', onPause);
        }
      }
      return { trace: trace.slice(), decodedWidth: plyr.media.videoWidth, nativeEvents: true, pauseProgress };
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
    const retiredFrame = readerFrame();
    const retiredWindow = retiredFrame.contentWindow;
    await showChunk(0, { frac: 0.5 });
    check(readerFrame() !== retiredFrame && !retiredFrame.isConnected, 'Chapter replaces the previous frame');
    check(readerFrame().contentWindow !== retiredWindow, 'Published chapter owns a fresh WindowProxy');
    equal(readerFrame().getAttribute('sandbox'), retiredFrame.getAttribute('sandbox'), 'Publication preserves sandbox');
    check(readerFrame().srcdoc.includes('prefix target suffix'), 'Full chapter is configured at publication');
    // Deliver the retired source with the CURRENT generation: source validation must reject it.
    window.dispatchEvent(new MessageEvent('message', { origin: 'null', source: retiredWindow,
      data: { type: 'reader-ready', generation: ebook.document.generation } }));
    check(!ebook.document.ready, 'Retired window cannot mark the current chapter ready');
    const loaded = new Promise(resolve => readerFrame().addEventListener('load', resolve, { once: true }));
    await until(async () => (await api('parser-state')).held, 'parser blocked before generated listener');
    await delay(240); // Negative control: the old one-shot 100 ms restore expires here.
    await api('release-parser', {});
    await loaded;
    await delay(80);
    const observed = await probe();
    equal(observed.marks, ['target'], 'Saved mark restored after actual listener readiness');
    equal((await api('state')).posts, [], 'Restore requires no new annotation POST');
    const retiredGeneration = ebook.document.generation;
    readerFrame().contentWindow.postMessage({ type: 'inject-hl', generation: retiredGeneration,
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
    // Also reject a retired generation sent by the current opaque WindowProxy.
    await probe('ready', { generation: reopenedGeneration });
    equal((await probe()).marks, ['target'], 'Stale ready cannot duplicate current marks');
    const beforeResize = readerFrame();
    const resizeFraction = ebook.lastFrac;
    window.dispatchEvent(new Event('resize'));
    await until(() => readerFrame() !== beforeResize && ebook.document.ready, 'resize publishes a fresh ready chapter');
    equal(ebook.lastFrac, resizeFraction, 'Resize preserves the reported position');
    equal((await probe()).marks, ['target'], 'Resize restores saved marks');
    return { freshFrame: true, retiredWindowRejected: true, resizeReady: true,
      marks: observed.marks, parserHeldBeyondOldTimer: true, posts: 0,
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
      const sourceBeforeFollow = { src: active.getAttribute('src'), child: active.querySelector('source')?.getAttribute('src') };
      window.partyFollow(id, 'Fixture movie', 12, false, Date.now() / 1000);
      equal({ src: active.getAttribute('src'), child: active.querySelector('source')?.getAttribute('src') },
        sourceBeforeFollow, 'Follower retains the current matching Plyr source without reassigning it');
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
  for (const kind of ['video', 'audio']) {
    await run(`party follow replaces the current local ${kind} stream and records its own progress`, async () => {
      delete window.partySend;
      const priorLibrary = LIBRARY;
      LIBRARY = [{ id: 9, kind: kind === 'video' ? 'movie' : 'music', title: 'Party nine' },
        { id: 10, kind: kind === 'video' ? 'movie' : 'music', title: 'Local ten' }];
      partyFollow(9, 'Party nine', 12, false, Date.now() / 1000);
      const previous = plyr.media;
      if (kind === 'video') openVideo(LIBRARY[1]);
      else playAudio(LIBRARY[1]);
      const audio = document.getElementById('pb-audio');
      await until(() => kind === 'video' ? plyr.media !== previous : audio.src.endsWith('/api/stream/10'),
        'local source installed after saved progress resolves');
      const media = kind === 'video' ? plyr.media : audio;
      const readyDescriptor = Object.getOwnPropertyDescriptor(media, 'readyState');
      const durationDescriptor = Object.getOwnPropertyDescriptor(media, 'duration');
      const priorFetch = window.fetch;
      const saves = [];
      try {
        Object.defineProperty(media, 'readyState', { configurable: true, value: 1 });
        Object.defineProperty(media, 'duration', { configurable: true, value: 634 });
        media.dispatchEvent(new Event('loadedmetadata'));
        media.currentTime = 203;
        const localSource = media.src || media.querySelector('source')?.src || media.currentSrc;
        check(localSource.endsWith('/api/stream/10'), 'Actual local source is ten');
        window.fetch = (url, options) => {
          if (String(url) === '/api/progress') saves.push(JSON.parse(options.body));
          return priorFetch(url, options);
        };
        partyFollow(9, 'Party nine', 24, true, Date.now() / 1000);
        await until(() => (media.src || media.querySelector('source')?.src || media.currentSrc).endsWith('/api/stream/9'),
          'effective party nine stream replaces local ten');
        await until(() => !media.paused, 'party nine playback starts before pause');
        let pauseObserved = false;
        const onPause = () => { pauseObserved = true; };
        media.addEventListener('pause', onPause, { once: true });
        try {
          media.pause();
          await until(() => pauseObserved, 'party nine pause event arrives');
        } finally {
          media.removeEventListener('pause', onPause);
        }
        if (kind === 'audio') check(!saves.some(save => save.media_id === 10),
          `Retired local audio tracker must not save party position as ten: ${JSON.stringify(saves)}`);
        if (kind === 'video') {
          await until(() => saves.some(save => save.media_id === 9 && save.position >= 24 && save.position < 26),
            'asynchronous pause saves party nine progress');
        }
        const source = media.src || media.querySelector('source')?.src || media.currentSrc;
        if (kind === 'video') {
          equal(videoItem.id, 9, 'Progress identity returns to party nine');
          check(!saves.some(save => save.media_id === 9 && save.position === 203),
            `Local ten position must not be saved as nine: ${JSON.stringify(saves)}`);
          check(saves.some(save => save.media_id === 9 && save.position >= 24 && save.position < 26),
            'Pause saves the party nine position');
        }
        check(source.endsWith('/api/stream/9'), `Actual stream returns to nine: ${source}`);
        check(media.currentTime >= 24 && media.currentTime < 26, 'Party clock replaces local position');
        return { source, position: media.currentTime, saves, localProgressSettled: true,
          metadataReadinessModeledAndRestored: true };
      } finally {
        LIBRARY = priorLibrary;
        window.fetch = priorFetch;
        if (readyDescriptor) Object.defineProperty(media, 'readyState', readyDescriptor);
        else delete media.readyState;
        if (durationDescriptor) Object.defineProperty(media, 'duration', durationDescriptor);
        else delete media.duration;
      }
    });
  }
  let socket;
  window.WebSocket = class {
    constructor() { this.readyState = 1; this.sent = []; socket = this; }
    send(data) { this.sent.push(JSON.parse(data)); }
    close() { this.readyState = 3; }
  };
  async function firstPartyState({ id, isHost, metadataFirst, nextId = id, localPaused = false }) {
    PartyUI.close();
    delete window.partySend;
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
    return { isHost, metadataFirst, loadedMedia: id, partyMedia: nextId, firstPosition: position,
      sourceBefore, sourceAfter: { src: media.getAttribute('src'), child: media.querySelector('source')?.getAttribute('src') },
      actualVendoredPlyr: true, productionPartyStateHandler: true, syntheticMediaTimingAndTransport: true };
  }
  for (const metadataFirst of [false, true]) {
    const id = metadataFirst ? 11 : 10;
    for (const isHost of [true, false]) await run(
      `${isHost ? 'host' : 'guest'} first party echo ${metadataFirst ? 'after' : 'before'} metadata uses correct resume authority`,
      () => firstPartyState({ id, isHost, metadataFirst }));
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
    await run(`${isHost ? 'host' : 'guest'} external YouTube to native ${kind} stops external playback`, async () => {
      const originalYT = window.YT;
      const originalReady = ytReady;
      const observations = [];
      const queuedAudioFixture = document.getElementById('pb-audio');
      const originalAudioReady = Object.getOwnPropertyDescriptor(queuedAudioFixture, 'readyState');
      let mock;
      let playerCreations = 0;
      window.YT = { Player: function (id, config) {
        playerCreations++;
        mock = { state: -1, position: 0, cuedStart: 0, pauses: 0, ready: false, commands: [],
          requireReady() { if (!this.ready) throw new Error('Mock YouTube player is not ready'); },
          setState(state) { const changed = this.state !== state; this.state = state; if (changed) config.events.onStateChange({ target: this, data: state }); },
          emitReady() { this.ready = true; config.events.onReady({ target: this }); },
          // Vendor seek from unstarted/cued starts playing; already-paused seek does not.
          seekTo(position) { this.requireReady(); this.commands.push('seek'); this.position = position; if (this.state !== 2) this.setState(1); },
          cueVideoById({ videoId, startSeconds }) { this.requireReady(); this.commands.push('cue'); this.cuedStart = startSeconds; this.position = 0; this.setState(5); },
          getCurrentTime() { this.requireReady(); return this.position; },
          getPlayerState() { this.requireReady(); return this.state; },
          playVideo() { this.requireReady(); this.commands.push('play'); if (this.state === 5) this.position = this.cuedStart; this.setState(1); },
          pauseVideo() { this.requireReady(); this.pauses++; this.setState(2); },
          elapse(seconds) { if (this.state === 1) this.position += seconds; },
          destroy() {},
        };
        return mock;
      } };
      ytReady = true;
      try {
        for (const readyBeforeTransition of [true, false]) {
          PartyUI.close();
          delete window.partySend;
          LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'music' }];
          const id = kind === 'movie' ? 10 : 11;
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
            state: { external_url: `mock-${kind}-${isHost}-${readyBeforeTransition}`, title: 'Mock YouTube',
              media_id: null, queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 } }) });
          equal(document.getElementById('pb-audio').paused, true, 'External handoff pauses native audio');
          equal(plyr.paused, true, 'External handoff pauses native video');
          check(!socket.sent.some(command => command.type === 'pause'), 'Native pause cannot broadcast a stale host pause');
          if (readyBeforeTransition) mock.emitReady();
          await delay(0);
          equal(mock.state, readyBeforeTransition ? 1 : -1, 'External mock has the expected readiness and playback');
          const modal = document.getElementById('video-modal');
          check(modal.classList.contains('yt'), 'External presentation is active');
          socket.sent.length = 0;
          socket.onmessage({ data: JSON.stringify({ type: 'state',
            state: { external_url: null, media_id: id, title: 'Native fixture', queue: [],
              position: 24, playing: true, updated_at: Date.now() / 1000 } }) });
          equal(mock.state, readyBeforeTransition ? 2 : -1, 'Ready external playback pauses; unready playback stays unstarted after native transition');
          equal(mock.pauses, readyBeforeTransition ? 1 : 0, 'Only a ready external player is paused');
          if (!readyBeforeTransition) {
            mock.emitReady();
            equal(mock.commands, [], 'Late readiness cannot seek or start YouTube after native transition');
            equal(mock.state, -1, 'Late readiness cannot start external playback');
          }
          check(!socket.sent.some(command => command.type === 'pause'), 'External pause cannot broadcast a stale host pause');
          check(!modal.classList.contains('yt'), 'Native player presentation is restored');
          equal(modal.classList.contains('on'), kind === 'movie', 'Native modal visibility');
          equal(native.paused, false, 'Native playback follows playing state');
          const expectedPosition = isHost ? 0 : 24;
          check(native.currentTime >= expectedPosition && native.currentTime < expectedPosition + 2,
            'Native position follows role authority');
          observations.push({ readyBeforeTransition, nativePausedForExternal: true, externalPauses: mock.pauses, nativePlaying: !native.paused,
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
        check(mock.state !== 1, 'Latest pending paused request does not start playing');
        const beforePausedElapsed = mock.position; mock.elapse(12);
        equal(mock.position, beforePausedElapsed, 'Latest pending paused request does not advance');
        partyFollowExternal(pendingId, 'Later host play', 20, true, Date.now() / 1000);
        check(mock.state === 1 && Math.abs(mock.position - 20) < 0.05, 'Later host play starts at the latest prepared target');
        partyFollowExternal(pendingId, 'Later host pause', mock.position, false, Date.now() / 1000);
        const afterHostPause = mock.position; mock.elapse(5);
        check(mock.state === 2 && mock.position === afterHostPause, 'Later host pause stops advancement');
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
                queue: [], position: 16.8285, playing: false, updated_at: Date.now() / 1000 } }) });
            return socket;
          }
          const beforeRolePlayerCount = playerCreations;
          const guestSocket = openRole(false);
          mock.emitReady();
          const pausedGuestTime = mock.position; mock.elapse(12);
          check(mock.state !== 1 && mock.position === pausedGuestTime, 'Fresh ordinary paused join never starts or advances');
          guestSocket.onmessage({ data: JSON.stringify({ type: 'state', state: { external_url: 'same-role-video', media_id: null,
            title: 'Host reset and Play', queue: [], position: 0, playing: true, updated_at: Date.now() / 1000 } }) });
          check(mock.state === 1 && mock.position < 0.05, 'Host reset and Play replaces a cued nonzero start despite its zero clock');
          guestSocket.onmessage({ data: JSON.stringify({ type: 'state', state: { external_url: 'same-role-video', media_id: null,
            title: 'Role reuse fixture', queue: [], position: 16.8285, playing: true, updated_at: Date.now() / 1000 } }) });
          check(mock.state === 1 && Math.abs(mock.position - 16.8285) < 0.05, 'Ordinary join follows subsequent host Play at its saved position');
          mock.elapse(3); const advancedGuestTime = mock.position;
          guestSocket.onmessage({ data: JSON.stringify({ type: 'state', state: { external_url: 'same-role-video', media_id: null,
            title: 'Role reuse fixture', queue: [], position: advancedGuestTime, playing: false, updated_at: Date.now() / 1000 } }) });
          mock.elapse(12);
          check(mock.state === 2 && mock.position === advancedGuestTime, 'Ordinary join follows subsequent host Pause');
          guestSocket.onmessage({ data: JSON.stringify({ type: 'state', state: { external_url: 'same-role-video', media_id: null,
            title: 'Role reuse fixture', queue: [], position: 42, playing: false, updated_at: Date.now() / 1000 } }) });
          mock.elapse(12);
          check(mock.state === 2 && mock.position === 42, 'Paused ordinary join follows a later host seek without playing');
          const pausedJoin = { stoppedOnReady: true, noAdvanceWhilePaused: true, laterHostPlayPause: true, pausedHostSeek: true, playingResetFromCue: true, vendorSeekStartsUnstarted: true };
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
          mock.position = 31.66337; mock.pauseVideo();
          mock.seekTo(42.333333);
          check(mock.state === 2, 'Native YouTube bar seek retains paused state');
          await until(() => hostSocket.sent.some(command => command.type === 'seek' && command.position === 42.333333),
            'paused native-bar position reaches the current host socket through the real poller');
          const pausedSeekCount = hostSocket.sent.filter(command => command.type === 'seek').length;
          await delay(2200);
          equal(hostSocket.sent.filter(command => command.type === 'seek').length, pausedSeekCount,
            'Stationary paused player does not broadcast repeated seeks');
          check(mock.state === 2, 'Paused seek mirroring never starts playback');
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
          roleReuse = { pausedJoin, pausedNativeSeek: { position: 42.333333, stayedPaused: true, noStationarySpam: true },
            samePlayer: true, hostCommands, guestCommands: 0, departedHostCommands: 0,
            playerCreations: 1, realSeekPoller: true, externalPlaybackMocked: true };
          // The earlier reader fixture is independent of the external player check.
          document.getElementById('em-close').click();
          document.getElementById('settings-modal').classList.remove('on');
          const appShell = document.getElementById('app');
          const backBeforeEntry = document.getElementById('vm-back').getBoundingClientRect();
          const shellBeforeEntry = getComputedStyle(appShell).display;
          if (shellBeforeEntry === 'none') {
            check(backBeforeEntry.width === 0 && backBeforeEntry.height === 0,
              'Hidden app shell gives its video Back control zero bounds');
            document.getElementById('guest').click();
            await until(() => getComputedStyle(appShell).display !== 'none', 'production guest entry reveals app shell');
            await until(() => LIBRARY.length === 0, 'guest entry finishes its empty fixture library load');
            LIBRARY = [{ id: 10, kind: 'movie' }, { id: 11, kind: 'music' }];
          }
          roleReuse.idleShellEntry = { beforeDisplay: shellBeforeEntry,
            beforeBackSize: { width: backBeforeEntry.width, height: backBeforeEntry.height },
            afterDisplay: getComputedStyle(appShell).display };
          PartyUI.hide();
          partyFollow(10, 'Native before iframe', 0, false, Date.now() / 1000);
          await until(() => document.getElementById('video-modal').classList.contains('on')
            && !document.getElementById('video-modal').classList.contains('yt'), 'native video presentation before external idle');
          partyFollowExternal('same-role-video', 'External idle controls', 42.333333, false, Date.now() / 1000);
          equal(document.getElementById('vm-title').textContent, 'External idle controls', 'External header replaces the previous native title');
          const backBounds = document.getElementById('vm-back').getBoundingClientRect();
          const beforeCleanupHit = document.elementFromPoint(backBounds.x + backBounds.width / 2, backBounds.y + backBounds.height / 2);
          check(backBounds.width > 0 && backBounds.height > 0 && document.getElementById('vm-back').contains(beforeCleanupHit),
            `External Back is hit-testable after guest entry: ${JSON.stringify({ bounds: { width: backBounds.width, height: backBounds.height }, hit: beforeCleanupHit?.id || beforeCleanupHit?.tagName })}`);
          const iframe = document.createElement('iframe');
          iframe.srcdoc = '<!doctype html><body style="margin:0;background:black;height:100vh">Owned iframe event boundary</body>';
          document.getElementById('yt-party').appendChild(iframe);
          try {
            await delay(2700);
            check(!document.querySelector('.vm-overlay').classList.contains('hidden'), 'External header stays visible after native idle transition and external idle');
            for (const id of ['vm-back', 'vm-party']) {
              const button = document.getElementById(id); const bounds = button.getBoundingClientRect();
              check(button.contains(document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)),
                `External idle ${id} remains pointer reachable above the iframe`);
            }
            document.getElementById('vm-party').click();
            check(document.getElementById('party-modal').classList.contains('on'), 'External idle Party opens production panel');
            PartyUI.hide();
            document.getElementById('vm-back').click();
            check(!document.getElementById('video-modal').classList.contains('on'), 'External idle Back exits the modal');
            partyFollow(10, 'Native after iframe exit', 0, false, Date.now() / 1000);
            check(document.getElementById('video-modal').classList.contains('on')
              && !document.getElementById('video-modal').classList.contains('yt'), 'Native video can reopen after external Back');
            roleReuse.idleHeader = { nativeEntry: true, nativeAutoHideControl: 'focused VM test', externalPersistent: true, iframeHitTests: true,
              partyOpens: true, backExits: true, nativeReopens: true };
          } finally { iframe.remove(); }
        }
        return { queuedAudioControl: { stalePauseBroadcasts: 0, legitimatePauseBroadcasts: 1, readinessDescriptorRestoredInFinally: true },
          retiredVideoMetadataIgnored: true, observations, latestPendingStateViaAppFollow: { position: 20, playing: false }, localActivations, roleReuse,
          productionPartyStateHandler: true, mockedYouTubeAPI: true, actualYouTubePlayback: false };
      } finally {
        if (originalAudioReady) Object.defineProperty(queuedAudioFixture, 'readyState', originalAudioReady);
        else delete queuedAudioFixture.readyState;
        equal(Object.getOwnPropertyDescriptor(queuedAudioFixture, 'readyState'), originalAudioReady, 'Synthetic producer restores its original readiness descriptor');
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
  await run('native decoded movie resume after production party end', async () => {
    const video = await nativeResume('after prior party lifecycle', true);
    const originalFetch = window.fetch;
    const audioEchoes = [];
    const audioMuted = pbAudio.muted;
    const priorLibrary = LIBRARY;
    const app = document.getElementById('app');
    const login = document.getElementById('login');
    const priorAppVisible = app.classList.contains('on');
    const priorBodyAuth = document.body.classList.contains('auth');
    const priorLoginStyle = login.getAttribute('style');
    const fixtureWebSocket = socket.constructor;
    window.WebSocket = fixtureWebSocket;
    const metadata = [];
    const observe = event => metadata.push({trusted:event.isTrusted, time:pbAudio.currentTime});
    pbAudio.addEventListener('loadedmetadata', observe);
    try {
      // This case precedes the modeled sign-in setup used by the later DOM cases.
      // Expose the production player shell before sending trusted browser input.
      app.classList.add('on');
      document.body.classList.remove('auth');
      login.style.display = 'none';
      for (const phase of ['resume', 'metadata', 'playing']) {
        PartyUI.close();
        document.getElementById('pb-close').click();
        pbAudio.removeAttribute('src'); pbAudio.load();
        await until(() => pbAudio.readyState === 0, 'native audio reset');
        let releaseResume;
        window.fetch = (url, options) => String(url).startsWith('/api/progress/23?')
          ? new Promise(resolve => { releaseResume = () => resolve(new Response(JSON.stringify({position:64}),
            {status:200,headers:{'Content-Type':'application/json'}})); }) : originalFetch(url, options);
        LIBRARY = [{id:23,kind:'music',title:'Native host audio'}];
        await api('hold-media', {path:'/api/stream/23'});
        playAudio(LIBRARY[0]);
        await until(() => releaseResume, 'typed progress response held');
        const title = document.getElementById('pb-title');
        let trustedGesture = false;
        const observeGesture = event => { trustedGesture = event.isTrusted && event.target === title; };
        title.addEventListener('click', observeGesture);
        let gesture;
        try {
          const rect = title.getBoundingClientRect();
          const point = {x:rect.left + rect.width/2, y:rect.top + rect.height/2};
          const hit = document.elementFromPoint(point.x, point.y);
          check(rect.width > 0 && rect.height > 0 && hit === title,
            `Visible audio title receives the owned browser gesture: ${JSON.stringify({point, hit:hit?.id || hit?.tagName})}`);
          gesture = await api('click', point);
          check(trustedGesture && gesture.trustedBrowserInput && navigator.userActivation.hasBeenActive,
            'Trusted document gesture precedes progress response release');
        } finally { title.removeEventListener('click', observeGesture); }
        if (phase !== 'resume') {
          releaseResume();
          await until(async () => (await api('media-gate')).pending > 0, 'native PCM metadata bytes held');
        }
        if (phase === 'playing') {
          await api('release-media', {});
          await until(() => pbAudio.readyState >= 2 && !pbAudio.paused && pbAudio.currentTime >= 64, 'native audio resumes64');
          pbAudio.currentTime = 41;
          await until(() => !pbAudio.seeking && pbAudio.currentTime >= 41, 'native user seek41');
        }
        PartyUI.open();
        document.getElementById('party-code-input').value = 'ABCD';
        document.getElementById('pt-join').click();
        socket.onmessage({data:JSON.stringify({type:'state',is_host:true,
          state:{media_id:23,title:'Own host echo',queue:[],position:0,playing:true,updated_at:Date.now()/1000}})});
        if (phase === 'resume') {
          releaseResume();
          await until(async () => (await api('media-gate')).pending > 0, 'resume PCM bytes held');
        }
        if (phase !== 'playing') await api('release-media', {});
        const expected = phase === 'playing' ? 41 : 64;
        await until(() => pbAudio.readyState >= 2 && !pbAudio.paused && pbAudio.currentTime >= expected, 'host echo retains native position');
        check(pbAudio.currentTime < expected + 3, 'Zero echo preserves local resume or seek');
        const before = pbAudio.currentTime;
        await until(() => pbAudio.currentTime > before + 0.03, 'actual PCM playback advances');
        audioEchoes.push({phase, expected, actual:pbAudio.currentTime, readyState:pbAudio.readyState,
          paused:pbAudio.paused, trustedGesture, gesture, hostStateHandler:true, source:pbAudio.currentSrc,
          duration:pbAudio.duration, muted:pbAudio.muted});
      }
      check(metadata.length === 3 && metadata.every(event => event.trusted), 'All audio phases use actual trusted native metadata');
      return {...video, audioEchoes, audioMetadata:metadata, typedResponse:true, productionHostStateHandler:true};
    } finally {
      window.fetch = originalFetch;
      await api('release-media', {});
      pbAudio.removeEventListener('loadedmetadata', observe);
      PartyUI.close();
      document.getElementById('pb-close').click();
      pbAudio.muted = audioMuted;
      window.WebSocket = originalWebSocket;
      LIBRARY = priorLibrary;
      app.classList.toggle('on', priorAppVisible);
      document.body.classList.toggle('auth', priorBodyAuth);
      if (priorLoginStyle === null) login.removeAttribute('style');
      else login.setAttribute('style', priorLoginStyle);
    }
  });
  // Actual DOM and trusted browser keyboard, with explicitly modeled API responses.
  const qaFetch = window.fetch;
  const qaAccount = account;
  const qaIdentity = IDENTITY;
  const requests = [];
  let responder;
  const reply = (value, status = 200) => new Response(status === 204 ? null : JSON.stringify(value), { status, headers: { 'Content-Type': 'application/json' } });
  window.fetch = async (url, options = {}) => {
    if (String(url).startsWith('/__fixture/')) return qaFetch(url, options);
    if (new URL(String(url), location.origin).pathname === '/api/library') {
      equal(new URL(String(url), location.origin).searchParams.get('identity'), IDENTITY, 'Library request retains current modeled identity');
      equal(options.headers?.Authorization, localStorage.frayd_token ? 'Bearer ' + localStorage.frayd_token : undefined, 'Library request retains current bearer presence');
    }
    requests.push({ url: String(url), method: options.method || 'GET', body: options.body ? JSON.parse(options.body) : null });
    return responder(String(url), options);
  };
  account = { name: 'Disposable admin', isAdmin: true };
  IDENTITY = 'qa-owned-admin';
  document.body.classList.remove('auth');
  document.getElementById('login').style.display = 'none';
  document.getElementById('app').classList.add('on');
  document.getElementById('ebook-modal').classList.remove('on');
  document.getElementById('settings-modal').classList.remove('on');
  const baseSettings = { library_paths: [], library_path: '/obsolete', library_locked: false, server_version: 'fixture' };
  let settingsValue = baseSettings;
  const defaultResponder = async (url) => url === '/api/admin/perms'
    ? reply([{ identity_id: 4, key: 'fixture-user', is_admin: true, allowed_kinds: null }])
    : url === '/api/admin/settings' ? reply(settingsValue)
      : new URL(String(url), location.origin).pathname === '/api/library' ? reply([]) : reply(null);
  responder = defaultResponder;
  const key = (key, accept = false) => api('key', { key, accept });
  const movie = { id: 9, title: 'Owned delete fixture', kind: 'movie', duration_secs: 120 };
  await run('admin DOM survives markup and trusted Tab Enter Space isolate card deletion', async () => {
    let deleteStatus = 409;
    responder = async (url, options) => options.method === 'DELETE' ? reply(null, deleteStatus) : defaultResponder(url);
    LIBRARY = [movie];
    await filterKind('movie', '');
    const button = document.querySelector('#main .card-del');
    check(button && button.type === 'button', 'Delete survives card innerHTML');
    document.querySelector('#main .row-head button').focus();
    await key('Tab');
    check(document.activeElement === button, 'Trusted Tab reaches delete button');
    check(button.matches(':focus-visible') && getComputedStyle(button).outlineStyle !== 'none', 'Keyboard focus is visible');
    const before = requests.length;
    const enter = await key('Enter');
    const space = await key(' ');
    equal(enter.dialogs.length, 1, 'Enter opens native confirmation once');
    equal(space.dialogs.length, 1, 'Space opens native confirmation once');
    check(enter.dialogs[0].message.includes('source file remains') && space.dialogs[0].accepted === false, 'Native cancel describes preserved source');
    equal(requests.slice(before).filter(r => r.method === 'DELETE'), [], 'Cancel sends no DELETE');
    check(LIBRARY.length === 1 && !document.getElementById('video-modal').classList.contains('on'), 'Cancel retains row without player');
    await key('Enter', true);
    await delay(20);
    check(LIBRARY.length === 1 && document.getElementById('toast-host').textContent.includes('referenced'), '409 retains item and explains refusal');
    deleteStatus = 204;
    await key(' ', true);
    await until(() => LIBRARY.length === 0, '204 removes modeled row');
    check(!document.querySelector('#main .card-del') && !document.getElementById('video-modal').classList.contains('on'), 'Success removes card without opening player');
    return { trustedTabEnterSpace: true, nativeConfirmations: 4, cancelledRequests: 0, modeledHttpStatuses: [409, 204], actualBackendDeletion: false };
  });
  await run('toolbar scan completion survives Settings navigation for the current account', async () => {
    let releaseStatus;
    let libraryReads = 0;
    const scanned = { ...movie, id: 10, title: 'New current-account scan item' };
    responder = async (url, options) => {
      if (url === '/api/scan') return reply(null, 202);
      if (url === '/api/scan/status') return new Promise(resolve => { releaseStatus = resolve; });
      if (new URL(String(url), location.origin).pathname === '/api/library') { libraryReads++; return reply([scanned]); }
      return defaultResponder(url, options);
    };
    const button = document.getElementById('add-media');
    const originalMarkup = button.innerHTML;
    LIBRARY = [];
    button.click();
    await until(() => releaseStatus, 'toolbar status request is pending');
    document.getElementById('settings-btn').click();
    await until(() => !!document.querySelector('.p-scan'), 'Settings navigation renders');
    check(button.disabled && button.textContent === 'Scanning…', 'Pending toolbar remains visibly busy');
    releaseStatus(reply({ scanning: false }));
    await until(() => !button.disabled, 'current-account toolbar scan completes after Settings navigation');
    equal(button.innerHTML, originalMarkup, 'Toolbar action is restored');
    equal(libraryReads, 1, 'Valid completion refreshes library exactly once');
    check(LIBRARY.some(item => item.id === scanned.id), 'Current account receives newly scanned item');
    check(document.getElementById('toast-host').textContent.includes('Scan complete — 1 new item(s)'), 'Completion feedback reports new item');
    check(document.getElementById('settings-modal').classList.contains('on'), 'Completion leaves Settings open');
    document.getElementById('settings-close').click();
    responder = defaultResponder;
    return { pendingStatusBeforeSettings: true, libraryReads, toolbarRestored: true, currentAccountRefreshed: true, modeledAPI: true, realPollingTimer: true };
  });
  await run('toolbar scan ignores an old-account status after a new account enters', async () => {
    let releaseStatus;
    let libraryReads = 0;
    responder = async (url, options) => {
      if (url === '/api/scan') return reply(null, 202);
      if (url === '/api/scan/status') return new Promise(resolve => { releaseStatus = resolve; });
      if (new URL(String(url), location.origin).pathname === '/api/library') { libraryReads++; return reply([]); }
      return defaultResponder(url, options);
    };
    const button = document.getElementById('add-media');
    const originalMarkup = button.innerHTML;
    button.click();
    await until(() => releaseStatus, 'old-account toolbar status is pending');
    const previousAccount = account;
    account = { name: 'Next disposable account', isAdmin: true };
    enterApp();
    await until(() => libraryReads === 1, 'new account loads its own library');
    await delay(20);
    const libraryBefore = LIBRARY;
    const toastBefore = document.getElementById('toast-host').textContent;
    releaseStatus(reply({ scanning: false }));
    await delay(50);
    equal(libraryReads, 1, 'Retired status cannot refresh the next account library');
    check(LIBRARY === libraryBefore, 'Retired status cannot replace current library state');
    equal(document.getElementById('toast-host').textContent, toastBefore, 'Retired status cannot publish completion feedback');
    check(button.disabled && button.textContent === 'Scanning…', 'Retired callback cannot mutate toolbar');
    // Fixture cleanup restores the deliberately retired busy control, after observing it.
    button.disabled = false; button.innerHTML = originalMarkup;
    account = previousAccount;
    responder = defaultResponder;
    return { actualEnterAppTransition: true, staleLibraryReads: 0, staleFeedback: false, staleToolbarMutation: false, modeledAccountAndAPI: true, realPollingTimer: true, logoutReloadCoveredSeparately: true };
  });
  await run('fresh toolbar transient status failure retries without overlapping refresh', () => freshResponsiveDocument(1440, 'transient'));
  await run('fresh toolbar three status failures allow trusted POST409 monitoring reattach', () => freshResponsiveDocument(1440, 'reattach'));
  await run('actual settings DOM scopes seven kind permissions and authoritative locked roots', async () => {
    responder = defaultResponder;
    await openSettings();
    const stage = document.getElementById('settings-stage');
    check(!stage.textContent.includes('/obsolete') && stage.textContent.includes('No folders configured'), 'Empty array beats obsolete legacy root');
    const row = stage.querySelector('[data-id="4"]');
    const inputs = [...row.querySelectorAll('.p-kinds-box input')];
    equal(inputs.map(i => i.value), ['movie', 'episode', 'music', 'audiobook', 'ebook', 'comic', 'manga'], 'Seven kinds rendered');
    const bodies = [];
    responder = async (url, options) => {
      if (url === '/api/admin/perms' && options.method === 'POST') { bodies.push(JSON.parse(options.body)); return reply(null, 204); }
      return defaultResponder(url);
    };
    row.querySelector('.p-save').click(); await delay(0);
    inputs.forEach(i => { i.checked = false; });
    row.querySelector('.p-save').click(); await delay(0);
    inputs.find(i => i.value === 'episode').checked = true;
    row.querySelector('.p-save').click(); await delay(0);
    equal(bodies.map(b => b.allowed_kinds), [null, [], ['episode']], 'All, none and episode payloads are distinct');
    check(bodies.every(b => b.is_admin === true), 'Admin checkbox never becomes a kind');
    row.querySelector('.p-remove').focus();
    const count = requests.filter(r => r.method === 'DELETE').length;
    const cancellation = await key('Enter');
    equal(cancellation.dialogs.length, 1, 'Native user removal confirmation');
    equal(requests.filter(r => r.method === 'DELETE').length, count, 'Cancelled user removal sends nothing');
    settingsValue = { ...baseSettings, library_locked: true, library_paths: ['/db'], effective_library_paths: ['/env/a', '/env/b'] };
    await openSettings();
    check(stage.textContent.includes('/env/a') && stage.textContent.includes('/env/b') && !stage.textContent.includes('/db'), 'Effective environment roots displayed');
    check([...stage.querySelectorAll('.lib-add,.lib-remove')].every(b => b.disabled), 'All locked root controls disabled');
    settingsValue = baseSettings;
    document.getElementById('settings-close').click();
    return { payloads: bodies, authoritativeEmpty: true, environmentControlsDisabled: true, modeledAPI: true };
  });
  await run('actual settings scan and save controls report bounded modeled failures', async () => {
    const observations = [];
    for (const status of [202, 409, 501, 500, 'network']) {
      responder = defaultResponder;
      await openSettings();
      let statusReads = 0;
      responder = async (url, options) => {
        if (url === '/api/scan') { if (status === 'network') throw Error('server unreachable'); return reply(null, status); }
        if (url === '/api/scan/status' && ++statusReads === 1 && status === 202) return reply(null, 503);
        return defaultResponder(url, options);
      };
      const button = document.querySelector('.p-scan');
      button.click(); await delay(0);
      check(status === 202 || status === 409 ? button.disabled && settingsPoll !== null : !button.disabled && settingsPoll === null, 'Accepted or already active scans attach an observer');
      check(status === 202 ? button.textContent === 'Scanning…' : status === 409 ? button.textContent.includes('already running') : status === 501 ? button.textContent.includes('No library folder') : status === 500 ? button.textContent.includes('500') : button.textContent.includes('unreachable'), 'Truthful status feedback');
      const observation = { status, text: button.textContent, disabled: button.disabled };
      document.getElementById('settings-close').click();
      if (status === 202 || status === 409) {
        check(settingsPoll !== null, 'Closing retains account-owned scan completion');
        await until(() => settingsPoll === null, 'closed scan completes and retires observer');
        equal(button.textContent, observation.text, 'Retired scan control is untouched');
        equal(statusReads, status === 202 ? 2 : 1, 'Transient server error retries once; busy admission observes completion');
      } else equal(settingsPoll, null, 'Refused scan has no observer');
      observations.push({ ...observation, statusReads });
    }
    for (const status of [409, 'network']) {
      responder = defaultResponder; await openSettings();
      responder = async (url, options) => {
        if (url === '/api/admin/settings' && options.method === 'POST') { if (status === 'network') throw Error('server unreachable'); return reply(null, status); }
        return defaultResponder(url, options);
      };
      const button = document.getElementById('s-site-save');
      button.click(); await delay(0);
      check(button.textContent !== 'Saved', 'Refused save cannot claim success');
      document.getElementById('settings-close').click();
    }
    return { observations, failedSettingsSaves: [409, 'network'], modeledAPI: true };
  });
  await run('settings ingest waits through indexing and close reopen rejects late status', async () => {
    const states = [];
    for (const terminal of ['failed', 'done']) {
      responder = defaultResponder; await openSettings();
      const button = document.querySelector('.p-ingest');
      document.querySelector('.s-ingest').value = 'https://example.invalid/owned';
      let state = 'downloading';
      let reads = 0;
      responder = async (url, options) => {
        if (url === '/api/ingest') return reply(null, 202);
        if (url === '/api/scan/status') { reads++; return reply({ scanning: state === 'indexing', ingesting: state }); }
        return defaultResponder(url, options);
      };
      button.click(); await until(() => settingsPoll !== null, 'ingest starts poll');
      await until(() => reads >= 1, 'downloading status read');
      equal(button.textContent, 'Downloading…', 'Downloading remains active');
      state = 'indexing';
      await until(() => button.textContent === 'Indexing…', 'indexing status read');
      check(button.disabled && settingsPoll !== null, 'Indexing is still busy');
      state = terminal;
      await until(() => settingsPoll === null, 'terminal status ends poll');
      equal(button.textContent, terminal === 'done' ? 'Download complete' : 'Download failed', 'Terminal feedback');
      states.push({ terminal, reads, text: button.textContent });
      document.getElementById('settings-close').click();
    }
    responder = defaultResponder; await openSettings();
    let release;
    const button = document.querySelector('.p-scan');
    responder = async (url, options) => url === '/api/scan' ? reply(null, 202)
      : url === '/api/scan/status' ? new Promise(resolve => { release = resolve; }) : defaultResponder(url, options);
    button.click();
    await until(() => release, 'held status fetch');
    document.getElementById('settings-close').click();
    responder = defaultResponder; await openSettings();
    const stageHTML = document.getElementById('settings-stage').innerHTML;
    const libraryReads = requests.filter(r => new URL(String(r.url), location.origin).pathname === '/api/library').length;
    release(reply({ scanning: false, ingesting: 'done' })); await delay(30);
    equal(document.getElementById('settings-stage').innerHTML, stageHTML, 'Old generation cannot repaint reopened settings');
    equal(requests.filter(r => new URL(String(r.url), location.origin).pathname === '/api/library').length, libraryReads + 1, 'Admitted scan refresh survives panel retirement');
    equal(settingsPoll, null, 'No surviving settings poll');
    document.getElementById('settings-close').click();
    let releaseSettings;
    responder = async (url, options) => url === '/api/admin/settings'
      ? new Promise(resolve => { releaseSettings = resolve; }) : defaultResponder(url, options);
    const oldOpen = openSettings();
    await until(() => releaseSettings, 'held settings load');
    document.getElementById('settings-close').click();
    responder = defaultResponder; await openSettings();
    const currentHTML = document.getElementById('settings-stage').innerHTML;
    releaseSettings(reply({ ...baseSettings, site_name: 'Retired settings response' }));
    await oldOpen;
    equal(document.getElementById('settings-stage').innerHTML, currentHTML, 'Late settings load cannot replace current markup');
    let releaseSave;
    responder = async (url, options) => url === '/api/admin/settings' && options.method === 'POST'
      ? new Promise(resolve => { releaseSave = resolve; }) : defaultResponder(url, options);
    const oldSaveButton = document.getElementById('s-site-save');
    oldSaveButton.click(); await until(() => releaseSave, 'held settings save');
    document.getElementById('settings-close').click();
    responder = defaultResponder; await openSettings();
    releaseSave(reply(null, 204)); await delay(20);
    check(oldSaveButton.textContent !== 'Saved' && document.getElementById('s-site-save').textContent === 'Save', 'Retired save cannot publish success');
    document.getElementById('settings-close').click();
    // A late old callback must not retire a newer account-owned observer.
    await openSettings();
    let releaseOld;
    responder = async (url, options) => url === '/api/scan/status'
      ? new Promise(resolve => { releaseOld = resolve; }) : defaultResponder(url, options);
    pollSettings(settingsGeneration, 'scan', document.querySelector('.p-scan'));
    await until(() => releaseOld, 'old observer status pending');
    pollSettings(settingsGeneration, 'scan', document.querySelector('.p-scan'));
    const replacement = settingsPoll;
    responder = defaultResponder;
    releaseOld(reply({scanning:false})); await delay(20);
    equal(settingsPoll, replacement, 'Old callback cannot clear replacement observer');
    await until(() => settingsPoll === null, 'replacement observer completes');
    document.getElementById('settings-close').click();
    return { states, retiredPanelUntouched: true, scanRefreshAfterClose: true, replacementObserverPreserved: true,
      lateSettingsLoadIgnored: true, lateSaveIgnored: true, modeledAPI: true, pollingUsesRealTimers: true };
  });
  await run('late library and progress responses cannot resurrect deleted DOM or filter sort', async () => {
    const other = { ...movie, id: 10, title: 'Remaining movie', created_at: 2 };
    let releaseLibrary;
    let releaseSearch;
    const progressReleases = [];
    responder = async (url, options) => url.startsWith('/api/library?q=') ? new Promise(resolve => { releaseSearch = resolve; })
      : new URL(String(url), location.origin).pathname === '/api/library' ? new Promise(resolve => { releaseLibrary = resolve; })
      : url.startsWith('/api/progress/') ? new Promise(resolve => progressReleases.push(resolve))
        : options.method === 'DELETE' ? reply(null, 204) : defaultResponder(url, options);
    LIBRARY = [movie, other];
    await filterKind('movie', '');
    const retiredCard = document.querySelector('#main .media-card');
    const pending = loadLibrary();
    document.querySelector('#main .row-head button').click();
    const sort = rowSort.get('movie');
    const search = document.querySelector('#search input');
    search.value = 'Owned'; search.dispatchEvent(new Event('input', { bubbles: true }));
    await until(() => releaseSearch, 'real debounced search starts held request');
    await filterKind('movie', '');
    document.querySelector('#main .card-del').focus();
    await key('Enter', true);
    await until(() => LIBRARY.length === 1, 'modeled delete completes');
    const deletedId = requests.filter(r => r.method === 'DELETE').at(-1).url.split('/media/')[1].split('?')[0];
    releaseLibrary(reply([movie, other]));
    await pending;
    releaseSearch(reply([movie, other]));
    progressReleases.forEach(resolve => resolve(reply({ position: 45 })));
    await delay(40);
    check(!LIBRARY.some(i => i.id === +deletedId), 'Held older load cannot resurrect removed item');
    equal(activeKind, 'movie', 'Filter retained'); equal(rowSort.get('movie'), sort, 'Sort retained');
    equal(activeQuery, '', 'Late search cannot restore its retired query'); search.value = '';
    equal(retiredCard.querySelector('.progress-strip').style.display, 'none', 'Detached progress never paints');
    check(!document.querySelector('#main').textContent.includes('Continue watching'), 'Filtered view has no stale continue row');
    const retainedLibrary = LIBRARY, retainedKind = activeKind, retainedQuery = activeQuery;
    const priorSort = new Map(rowSort);
    const retainedShowsDrill = showsDrill;
    const retainedAuthorDrill = { ...authorDrill };
    const tvAndAuthorItems = [
      { id: 9, kind: 'episode', title: '日本語.Show.S01E02', series: 'Unicode show', created_at: 30, artwork_version: '9.png' },
      { id: 10, kind: 'episode', title: 'Unicode 1x01', series: 'Unicode show', created_at: 10 },
      { id: 60, kind: 'episode', title: 'Unicode bonus', series: 'Unicode show' },
      { id: 61, kind: 'movie', title: 'Unicode movie', artwork_version: '61.png' },
      { id: 62, kind: 'music', title: 'Loose song', author: 'Mixed artist' },
      { id: 63, kind: 'music', title: 'Album song', author: 'Mixed artist', series: 'Named album', artwork_version: '63.png' },
      { id: 64, kind: 'ebook', title: 'Series book', author: 'Book author', series: 'Named books', artwork_version: '64.png' },
      { id: 65, kind: 'audiobook', title: 'Loose audiobook', author: 'Audio author', artwork_version: '65.png' },
      { id: 66, kind: 'music', title: 'Second song', author: 'Second artist', created_at: 20 },
      { id: 67, kind: 'episode', title: 'Second S01E01', series: 'Second show', created_at: 20 },
      { id: 68, kind: 'comic', title: 'Covered comic', artwork_version: '68.png' },
      { id: 69, kind: 'manga', title: 'Covered manga', artwork_version: '69.png' },
    ];
    let tvSearchMode = 'success';
    responder = async (url, options) => {
      const parsed = new URL(url, location.origin);
      if (parsed.pathname === '/api/library') {
        const query = parsed.searchParams.get('q');
        if (query && tvSearchMode === 'network') throw Error('Owned TV search transport failure');
        if (query && tvSearchMode === 'http') return reply(null, 500);
        return reply(query ? tvAndAuthorItems.filter(item => item.title.toLowerCase().includes(query.toLowerCase())) : tvAndAuthorItems);
      }
      if (url.startsWith('/api/progress/')) return reply(null);
      return defaultResponder(url, options);
    };
    LIBRARY = tvAndAuthorItems;
    showsDrill = null;
    Object.keys(authorDrill).forEach(kind => { authorDrill[kind] = null; });
    rowSort.clear();
    await filterKind('all', '');
    const categoryShelves = [...document.querySelectorAll('#main > section')];
    equal(categoryShelves.map(shelf => shelf.querySelector('h3')?.textContent),
      ['Movies','TV Shows','Music','Audiobooks','Ebooks','Comics','Manga'].map(label => 'Recently Added ' + label), 'Seven dashboard siblings use category headings');
    for (const shelf of categoryShelves) {
      const button = shelf.querySelector('.row-head button');
      equal(button.textContent, 'Newest first', 'Default control states current newest order');
      button.click();
      equal(button.textContent, 'Oldest first', 'Toggle states current oldest order');
      equal(button.getAttribute('aria-pressed'), 'true', 'Oldest order is accessible');
      button.click();
      equal(button.textContent, 'Newest first', 'Second toggle returns to newest');
    }
    const dashboardImages = [...document.querySelectorAll('#main > section img.thumb')];
    await until(() => dashboardImages.length === 7 && dashboardImages.every(img => img.complete && img.naturalWidth > 0), 'All seven categories load available thumbnail API artwork');
    const tvShelf = categoryShelves[1];
    equal([...tvShelf.querySelectorAll('.title')].map(node => node.textContent), ['Unicode show', 'Second show'], 'TV folders start by maximum member date');
    tvShelf.querySelector('button').click();
    equal([...tvShelf.querySelectorAll('.title')].map(node => node.textContent), ['Unicode show', 'Second show'], 'TV oldest uses minimum member date including undated bonus');
    tvShelf.querySelector('button').click();
    const tvNav = document.querySelector('#sidebar [data-kind="show"]');
    check(tvNav, 'Upstream TV navigation exists');
    tvNav.focus(); await key('Enter');
    await until(() => activeKind === 'episode' && document.querySelector('#main .show-grid'), 'trusted TV navigation reaches canonical episode shelf');
    const byTitle = title => [...document.querySelectorAll('#main .media-card')].find(node => node.querySelector('.title')?.textContent === title);
    byTitle('Unicode show').click();
    await until(() => showsDrill === 'Unicode show' && byTitle('Unicode 1x01'), 'TV folder opens episode cards');
    equal([...document.querySelectorAll('#main .series-head h4')].map(node => node.textContent), ['Season 1', 'Other episodes'], 'Known and unknown seasons have truthful labels');
    equal([...document.querySelectorAll('#main .media-card .title')].map(node => node.textContent), ['Unicode 1x01', '日本語.Show.S01E02', 'Unicode bonus'], 'Episode ordering uses parsed numbers');
    byTitle('Unicode 1x01').click();
    await until(() => document.getElementById('video-modal').classList.contains('on') && (plyr.media.src || plyr.media.querySelector('source')?.src || '').endsWith('/api/stream/10'), 'Episode card dispatches to real vendored video player');
    document.getElementById('vm-back').click();
    await loadLibrary(); await delay(0);
    equal(showsDrill, 'Unicode show', 'Library refresh retains the selected series');
    for (const mode of ['success', 'http', 'network']) {
      tvSearchMode = mode;
      activeKind = 'show';
      await serverSearch('Unicode'); await delay(0);
      equal(activeKind, 'episode', 'TV search normalizes its navigation alias');
      check(!byTitle('Unicode movie'), 'TV search and failure fallback exclude matching movies');
      check(byTitle('Unicode 1x01'), 'TV search and failure fallback preserve matching episode cards');
    }
    tvSearchMode = 'success';
    await serverSearch(''); await delay(0);
    document.querySelector('#sidebar [data-kind="music"]').click(); await delay(0);
    byTitle('Mixed artist').click(); await delay(0);
    check(byTitle('Loose song') && byTitle('Named album'), 'Mixed author retains loose song beside album folder');
    check(byTitle('Named album').querySelector('.sub').textContent.includes('Album'), 'Music series folder is labeled as an album');
    check(byTitle('Named album').querySelector('img')?.getAttribute('src') === '/api/thumb/63?v=63.png', 'Album uses verified member artwork');
    byTitle('Named album').click(); await delay(0);
    check(byTitle('Album song') && !byTitle('Loose song'), 'Album drill shows its own songs');
    document.querySelectorAll('#main .back-row')[1].click(); await delay(0);
    check(byTitle('Loose song') && byTitle('Named album'), 'Author back restores loose items and album');
    document.querySelector('#sidebar [data-kind="ebook"]').click(); await delay(0);
    byTitle('Book author').click(); await delay(0); byTitle('Named books').click(); await delay(0);
    check(byTitle('Series book')?.classList.contains('portrait'), 'All-series ebook author retains actual portrait card');
    document.querySelector('#sidebar [data-kind="audiobook"]').click(); await delay(0);
    byTitle('Audio author').click(); await delay(0);
    check(byTitle('Loose audiobook')?.classList.contains('portrait'), 'All-loose audiobook author retains actual portrait card');
    const tvAuthorWorkflows = { trustedTVNavigation: true, canonicalEpisodeSearch: ['success', 'http', 'network'], refreshRetainsSeries: true,
      episodeVideoDispatch: true, unknownSeasonLabel: 'Other episodes', mixedArtistLooseAndAlbum: true, ebookAllSeries: true, audiobookAllLoose: true, actualDOM: true, modeledAPI: true };
    const progressGets = [];
    const positions = new Map([[60, 20], [61, 80], [62, 99], [63, 0]]);
    LIBRARY = [
      { id: 60, kind: 'movie', title: 'Progress movie', duration_secs: 100 },
      { id: 61, kind: 'episode', title: 'Progress episode', duration_secs: 100 },
      { id: 62, kind: 'music', title: 'Progress song', duration_secs: 100 },
      { id: 63, kind: 'ebook', title: 'Progress book' },
    ];
    showsDrill = 'Unknown series';
    authorDrill.music = { author: 'Unknown' };
    authorDrill.ebook = { author: 'Unknown' };
    responder = async (url, options) => {
      if (url.startsWith('/api/progress/')) {
        const id = Number(new URL(url, location.origin).pathname.split('/').at(-1));
        progressGets.push(id);
        return reply({ position: positions.get(id) });
      }
      return defaultResponder(url, options);
    };
    await filterKind('all', ''); await delay(0);
    const initialGets = progressGets.length;
    equal([...progressGets].sort(), [60, 61, 62, 63], 'Each rendered media ID fetches progress once');
    const continuationCards = [...document.querySelector('#main section .cards').children];
    equal(continuationCards.map(card => card.title), ['Progress episode', 'Progress movie'], 'Continue order and eligibility retained');
    equal(continuationCards.map(card => card.querySelector('.progress-strip > div').style.width), ['80%', '20%'], 'Shared progress preserves both continuation bars');
    equal(document.querySelectorAll('#main .media-card').length, 6, 'Two continuation cards and four regular cards render');
    document.querySelector('#main .row-head button').click(); await delay(0);
    equal(progressGets.length, initialGets, 'Same-render sorting makes no additional progress request');
    positions.set(60, 60);
    await filterKind('all', ''); await delay(0);
    equal(progressGets.length - initialGets, 4, 'Next render reads a fresh snapshot');
    check([...document.querySelectorAll('#main .media-card')].filter(card => card.title === 'Progress movie')
      .every(card => card.querySelector('.progress-strip > div').style.width === '60%'), 'Both current movie cards receive updated progress');
    const beforeFreshResume = progressGets.length;
    equal((await resumeFor(60)).position, 60, 'Non-render resume stays fresh');
    equal(progressGets.length, beforeFreshResume + 1, 'Non-render resume makes its own progress GET');
    const failedGets = [];
    responder = async (url, options) => {
      if (url.startsWith('/api/progress/')) { failedGets.push(url); return reply(null, 500); }
      return defaultResponder(url, options);
    };
    await filterKind('all', ''); await delay(0);
    equal(failedGets.length, 4, 'Failed progress is shared within the render');
    check(!document.querySelector('#main').textContent.includes('Continue watching'), 'Failed progress creates no continue row');
    check([...document.querySelectorAll('#main .progress-strip')].every(strip => strip.style.display === 'none'), 'Failed progress leaves every regular bar hidden');
    responder = defaultResponder;
    LIBRARY = retainedLibrary;
    showsDrill = retainedShowsDrill;
    Object.assign(authorDrill, retainedAuthorDrill);
    rowSort.clear();
    priorSort.forEach((value, kind) => rowSort.set(kind, value));
    await filterKind(retainedKind, retainedQuery);
    return { deletedId: +deletedId, libraryIds: LIBRARY.map(i => i.id), activeKind, sort, detachedProgressIgnored: true, modeledAPI: true, tvAuthorWorkflows,
      progressReadReuse: { libraryItems: 4, initialGets, sortAdditionalGets: 0, nextRenderGets: 4, freshResumeAdditionalGets: 1, failedRenderGets: failedGets.length,
        continuationOrder: ['Progress episode', 'Progress movie'], actualDOM: true, modeledProgressAPI: true } };
  });
  await run('trusted ebook keys preserve editable inactive settings and button focus', async () => {
    document.getElementById('em-close').click();
    const comicSaved = new Map([[90, 3], [91, 2]]), comicWrites = [];
    let comicHeld = true, releaseComic;
    responder = async (url, options) => {
      const path = new URL(url, location.origin).pathname;
      if (/^\/api\/comic\/(90|91)\/pages$/.test(path)) return reply(['0', '1', '2', '3', '4']);
      if (/^\/api\/progress\/(90|91)$/.test(path)) {
        const id = Number(path.split('/').at(-1)), position = comicSaved.get(id);
        if (id === 90 && comicHeld) return new Promise(resolve => { releaseComic = () => { comicHeld = false; resolve(reply({ position })); }; });
        return reply({ position });
      }
      if (path === '/api/progress' && options.method === 'POST') {
        const body = JSON.parse(options.body);
        if (comicSaved.has(body.media_id)) { comicWrites.push({ id: body.media_id, page: body.position }); comicSaved.set(body.media_id, body.position); return reply({}); }
      }
      return defaultResponder(url, options);
    };
    await openComic({ id: 90, title: 'Owned pending comic A' });
    await until(() => releaseComic, 'comic saved response reaches held boundary');
    check(comic.page === null && document.getElementById('cm-pageinfo').textContent === 'Loading…', 'Comic exposes unknown loading position');
    check(document.getElementById('cm-prev').disabled && document.getElementById('cm-next').disabled, 'Comic navigation is disabled during restore');
    document.getElementById('cm-next').click();
    document.body.tabIndex = -1; document.body.focus();
    await key('ArrowRight'); await key(' ');
    equal(comicWrites, [], 'Pending comic button/keyboard cannot save unknown position');
    document.getElementById('cm-close').click();
    equal(comicWrites, [], 'Closing pending comic does not save zero');
    await openComic({ id: 91, title: 'Owned comic B' });
    await until(() => comic?.page === 2, 'comic B restores its own page');
    releaseComic(); await delay(20);
    equal(comic.it.id, 91, 'Late A cannot replace B'); equal(comic.page, 2, 'Late A cannot move B');
    check(document.getElementById('cm-img').src.endsWith('/api/comic/91/page/2'), 'Comic B retains its own image');
    document.getElementById('cm-close').click();
    await openComic({ id: 90, title: 'Owned pending comic A' });
    await until(() => comic?.page === 3, 'closed pending comic reopens at saved page3');
    check(document.getElementById('cm-img').src.endsWith('/api/comic/90/page/3'), 'Reopened comic image uses saved page3');
    document.getElementById('cm-prev').click(); equal(comic.page, 2, 'Ready comic previous navigation works');
    document.getElementById('cm-next').click(); equal(comic.page, 3, 'Ready comic next navigation works');
    document.getElementById('cm-close').click();
    equal(comicSaved.get(90), 3, 'Ready close preserves comic bookmark3');
    check(!comicWrites.some(write => write.id === 90 && write.page === 0), 'No initial zero write reached comic fixture');
    const epubReadiness = [];
    for (const boundary of ['toc', 'progress']) {
      let savedPosition = 2.3, responseHeld = true, releaseResponse, releaseChapter;
      const writes = [], chapterReads = [];
      responder = async (url, options) => {
        const path = new URL(url, location.origin).pathname;
        if (path === '/api/epub/7/spine') return reply(['one.xhtml', 'two.xhtml', 'three.xhtml', 'four.xhtml']);
        if (path === '/api/annotations/7') return reply([]);
        if (path === '/api/epub/7/toc') {
          if (boundary === 'toc' && responseHeld) return new Promise(resolve => { releaseResponse = () => { responseHeld = false; resolve(reply([])); }; });
          return reply([]);
        }
        if (path === '/api/progress/7') {
          const position = savedPosition;
          if (boundary === 'progress' && responseHeld) return new Promise(resolve => { releaseResponse = () => { responseHeld = false; resolve(reply({ position })); }; });
          return reply({ position });
        }
        if (/^\/api\/epub\/7\/chunk\/[0-3]$/.test(path)) {
          const chapter = Number(path.split('/').at(-1)); chapterReads.push(chapter);
          const html = () => new Response('<html><head></head><body>Owned readiness chapter</body></html>', { headers: { 'Content-Type': 'text/html' } });
          if (chapter === 2) return new Promise(resolve => { releaseChapter = () => resolve(html()); });
          return html();
        }
        if (path === '/api/progress' && options.method === 'POST') {
          const body = JSON.parse(options.body);
          if (body.media_id === 7) { savedPosition = body.position; writes.push(savedPosition); return reply({}); }
        }
        return defaultResponder(url, options);
      };
      const opening = openEbook({ id: 7, title: 'Owned pending EPUB' });
      await until(() => releaseResponse, 'EPUB initial ' + boundary + ' response reaches hold');
      check(ebook.chunk === null && document.getElementById('em-pageinfo').textContent === 'Loading…', 'EPUB exposes unknown loading chapter');
      check(document.getElementById('em-prev').disabled && document.getElementById('em-next').disabled && document.getElementById('em-toc-btn').disabled, 'EPUB pending navigation is disabled');
      document.getElementById('em-next').click(); document.getElementById('em-toc-btn').click();
      document.body.focus(); await key('ArrowRight'); await key(' '); await key('t');
      window.dispatchEvent(new Event('resize')); await delay(340);
      equal(chapterReads, [], 'Pending EPUB keyboard/resize does not coerce null to chapter0');
      equal(writes, [], 'Pending EPUB callbacks do not save unknown chapter');
      document.getElementById('em-close').click();
      equal(writes, [], 'Closing initial EPUB restore does not overwrite bookmark');
      releaseResponse(); await opening; await delay(20);
      equal(writes, [], 'Retired EPUB initial response cannot save');
      const reopening = openEbook({ id: 7, title: 'Owned pending EPUB' });
      await until(() => releaseChapter && ebook.chunk === 2, 'EPUB reopens at saved chapter before its body loads');
      check(Math.abs(ebook.lastFrac - 0.3) < 1e-9, 'Known resume fraction is retained before layout');
      document.getElementById('em-close').click();
      equal(savedPosition, 2.3, 'Known EPUB chapter close preserves saved fractional bookmark');
      releaseChapter(); await reopening; await delay(20);
      equal(writes, [2.3], 'Only known EPUB position was saved');
      epubReadiness.push({ boundary, pendingCloseWrites: 0, resizeAndKeysIgnored: true, reopenedChapter: 2, knownClosePosition: savedPosition });
    }
    const readerBookmarkReadiness = { comic: { pendingCloseWrites: 0, reopenedPage: comicSaved.get(90), lateOtherComicUnchanged: true, readyNavigation: true }, epub: epubReadiness, actualDOM: true, modeledProgressResponses: true, originalHeldBodiesPreserved: true };
    responder = defaultResponder;
    await api('configure', { body: '<p>Reader shortcut fixture</p>', annotations: [] });
    ebook = { it: { id: 7 }, spine: ['one.xhtml', 'two.xhtml'], chunk: 0, annots: [], toc: [] };
    document.getElementById('ebook-modal').classList.add('on');
    // Reader transport remains the original loopback fixture for current opaque WindowProxy.
    window.fetch = qaFetch;
    await load();
    const initialGeneration = ebook.document.generation;
    const outsiderOrigins = [];
    for (const opaque of [true, false]) {
      const outsider = document.createElement('iframe');
      if (opaque) outsider.setAttribute('sandbox', 'allow-scripts');
      let delivered = false;
      const observe = event => {
        if (event.source === outsider.contentWindow) { delivered = true; outsiderOrigins.push(event.origin); }
      };
      window.addEventListener('message', observe);
      try {
        outsider.srcdoc = '<script>parent.postMessage(' + JSON.stringify({ type: 'chapter', dir: 1, generation: initialGeneration }) + ',"*")</script>';
        document.body.appendChild(outsider);
        await until(() => delivered, 'unrelated frame delivers its actual postMessage');
        equal(ebook.chunk, 0, 'Unrelated actual WindowProxy cannot advance current reader');
        equal(ebook.document.generation, initialGeneration, 'Unrelated sender cannot replace reader document');
      } finally { window.removeEventListener('message', observe); outsider.remove(); }
    }
    equal(outsiderOrigins, ['null', location.origin], 'Both opaque and same-origin negative controls actually delivered');
    const targets = [document.querySelector('#search input'), document.getElementById('em-search')];
    for (const tag of ['textarea', 'select', 'div', 'button']) {
      const target = document.createElement(tag);
      if (tag === 'div') target.contentEditable = 'true';
      document.getElementById('ebook-modal').appendChild(target);
      targets.push(target);
    }
    for (const target of targets) {
      target.focus();
      for (const press of ['ArrowRight', ' ', 't']) await key(press);
      equal(ebook.chunk, 0, 'Editable/button target cannot flip chapter');
      equal(ebook.document.generation, initialGeneration, 'Editable/button target cannot replace reader document');
    }
    document.getElementById('settings-modal').classList.add('on');
    document.body.tabIndex = -1; document.body.focus();
    await key('ArrowRight'); equal(ebook.chunk, 0, 'Settings guard blocks body key');
    document.getElementById('settings-modal').classList.remove('on');
    document.getElementById('ebook-modal').classList.remove('on');
    await key('ArrowRight'); equal(ebook.chunk, 0, 'Inactive reader ignores key');
    document.getElementById('ebook-modal').classList.add('on');
    document.body.focus(); await key('ArrowRight');
    await until(() => ebook.chunk === 1 && ebook.document?.ready, 'active body flips once');
    equal(ebook.chunk, 1, 'Positive key flips exactly once');
    check((await probe()).opaqueOrigin === 'null', 'Current opaque WindowProxy remains functional');
    document.getElementById('em-close').click();
    targets.slice(2).forEach(target => target.remove());
    return { readerBookmarkReadiness, trustedKeyboard: true, targets: ['search INPUT', 'reader INPUT', 'TEXTAREA', 'SELECT', 'contenteditable', 'BUTTON'], positiveChapter: 1, opaqueOrigin: 'null', outsiderOrigins };
  });
  async function installer(mode, isolated = false) {
    const node = document.createElement('iframe'); node.width = '600'; node.height = '900';
    if (isolated) node.style.cssText = 'position:fixed;top:0;left:0;width:1px;height:1px;border:0;opacity:0;pointer-events:none';
    node.src = '/__fixture/installer/' + mode;
    const loaded = new Promise(resolve => node.addEventListener('load', resolve, { once: true }));
    document.body.appendChild(node); await loaded;
    check(node.contentWindow.__installerFixture, 'Preload adapter exists before installer script');
    return node;
  }
  function closeInstaller(node) {
    node.contentWindow.dispatchEvent(new PageTransitionEvent('pagehide'));
    node.remove();
  }
  /** Observe only this iframe's terminal poll decision while independent cases run. */
  function observeInstallerPolling(node) {
    const startedAt = Date.now();
    let timer;
    let observer;
    let settled = false;
    let disposed = false;
    let resolveCompletion;
    // Resolve failures as data until case48 claims them; never reject unobserved work.
    const completion = new Promise(resolve => { resolveCompletion = resolve; });
    function finish(error) {
      if (settled) return;
      settled = true;
      const elapsedMs = Date.now() - startedAt;
      if (!error && elapsedMs >= 15000) error = new Error('Five real-timer attempts exceeded15s');
      clearTimeout(timer);
      observer?.disconnect();
      resolveCompletion({ elapsedMs, error });
    }
    function inspect() {
      if (node.contentDocument.getElementById('stat').textContent.includes('or choose Retry')) finish();
    }
    observer = new MutationObserver(inspect);
    observer.observe(node.contentDocument.getElementById('stat'), { childList: true, subtree: true, characterData: true });
    timer = setTimeout(() => finish(new Error('Five real-timer attempts exceeded15s')), 15000);
    inspect();
    return { node, completion, dispose() {
      if (disposed) return;
      disposed = true;
      finish(new Error('Installer polling observation disposed before completion'));
      closeInstaller(node);
    } };
  }
  // Independent iframe owns its fetch adapter, document, timers and protocol state.
  // It contributes no layout or input target; the production3000ms polling stays real.
  const installerPolling = observeInstallerPolling(await installer('html', true));
  try {
    // Retain the original39, then exercise the new production CSS at actual CSS-pixel
    // viewports. API fixtures supply library/admin responses; native MP4 stays decoded.
    for (const width of [320, 768, 1024, 1440]) await run(`responsive production layout at ${width} CSS pixels`, () => freshResponsiveDocument(width));
    await api('viewport', { width: 1440 });
    await run('production installer DOM validates typed protocol with manual GET-only navigation', async () => {
      const observations = [];
      for (const mode of ['valid', 'signed-in', 'opaque', 'html', 'malformed', 'unrelated', 'unreadable', 'nonboolean']) {
        const node = await installer(mode);
        try {
          const w = node.contentWindow, d = node.contentDocument, fixture = w.__installerFixture;
          await until(() => d.getElementById('stat').textContent.includes(mode === 'valid' || mode === 'signed-in' ? 'verified at' : 'unverified'), 'Installer publishes protocol decision');
          check(!d.querySelector('form,input[type=email],input[type=password]'), 'No credential form exists');
          check(d.getElementById('open-frayd').href === 'http://127.0.0.1:8484/' && !d.getElementById('retry').disabled, 'Manual Open and Retry remain available');
          check(w.location.pathname === '/__fixture/installer/' + mode, 'Installer never automatically navigates');
          check(fixture.requests.every(r => r.method === 'GET' && r.credentials === 'omit' && r.cache === 'no-store'), 'All modeled requests are credential-free fixed GETs');
          equal(fixture.storage, [], 'Installer never accesses storage'); equal(fixture.errors, [], 'Installer has no uncaught errors');
          if (mode === 'valid') check(d.getElementById('s-stat').textContent.includes('Create your owner'), 'Typed true gives manual owner instruction');
          if (mode === 'signed-in') check(d.getElementById('s-stat').textContent.includes('Sign in'), 'Typed false gives manual sign-in instruction');
          d.querySelector('[data-os=linux]').click(); d.getElementById('gen').click();
          check(d.getElementById('cmd').textContent.includes('/bootstrap.sh'), 'Linux source command remains available');
          d.querySelector('[data-os=win]').click(); d.getElementById('gen').click();
          check(d.getElementById('cmd').textContent.includes('/bootstrap.ps1'), 'Windows source command remains available');
          observations.push({ mode, status: d.getElementById('stat').textContent, instruction: d.getElementById('s-stat').textContent, requests: fixture.requests, storageAccesses: 0, automaticNavigation: false });
        } finally { closeInstaller(node); }
      }
      return { actualProductionHTMLAndInlineJS: true, modeledResponsesOnly: true, real8484Requests: 0, observations };
    });
    await run('production installer actual five-poll bound retry and pagehide stale guards', async () => {
      let node = installerPolling.node;
      let result;
      try {
        const w = node.contentWindow, d = node.contentDocument, fixture = w.__installerFixture;
        const polling = await installerPolling.completion;
        if (polling.error) throw polling.error;
        equal(fixture.requests.length, 5, 'Exactly five automatic real-timer attempts');
        await delay(150); equal(fixture.requests.length, 5, 'No sixth attempt');
        fixture.mode = 'valid'; d.getElementById('retry').click();
        await until(() => d.getElementById('stat').textContent.includes('verified at'), 'Explicit Retry starts a new valid check');
        equal(fixture.requests.length, 7, 'Explicit Retry performs health and setup');
        result = { automaticAttempts: 5, retryRequests: 2, realTimers: true, automaticPollingElapsedMs: polling.elapsedMs,
          isolatedPollingOverlappedResponsiveCases: true, completionDeadlineMs: 15000 };
      } finally { installerPolling.dispose(); }
      node = await installer('held');
      try {
        const w = node.contentWindow, d = node.contentDocument, fixture = w.__installerFixture;
        await until(() => fixture.held.length === 1, 'Old request held');
        fixture.mode = 'signed-in'; d.getElementById('retry').click();
        await until(() => d.getElementById('s-stat').textContent.includes('Sign in'), 'New retry wins');
        const current = d.getElementById('stat').textContent;
        fixture.release(); await delay(30);
        equal(d.getElementById('stat').textContent, current, 'Late old request cannot publish');
        equal(fixture.requests.length, 3, 'Retired request cannot start setup');
        fixture.mode = 'held'; d.getElementById('retry').click();
        await until(() => fixture.held.length === 1, 'Pagehide request held');
        const hiddenText = d.getElementById('stat').textContent;
        // An actual iframe navigation delivers pagehide to the production page.
        // Retain only its old fixture/document references to inspect cancellation.
        const leftPage = new Promise(resolve => node.addEventListener('load', resolve, { once: true }));
        node.src = 'about:blank'; await leftPage;
        check(fixture.inspectRetirement(), 'Pagehide disposes both actual timers and controller');
        check(fixture.requests.at(-1).aborted, 'Pagehide aborts pending request');
        fixture.release(); await delay(30);
        equal(d.getElementById('stat').textContent, hiddenText, 'Late pagehide result cannot publish');
        equal(fixture.requests.length, 4, 'Pagehide stops request chain');
        equal(fixture.errors, [], 'No uncaught installer errors'); equal(fixture.storage, [], 'No installer storage access');
        return { ...result, staleRetryIgnored: true, actualPagehideNavigation: true, pagehideAbortedAndDisposed: true, modeledResponsesOnly: true, real8484Requests: 0 };
      } finally { closeInstaller(node); }
    });
  } finally { installerPolling.dispose(); }
  window.fetch = qaFetch;
  account = qaAccount; IDENTITY = qaIdentity;
  disposeSettings();
  // Actual production logout reload is the seventh new case. Carry response proof only
  // through this fresh profile's sessionStorage; no credentials or owner state exist.
  const packet = { cases, errors, mediaDiagnostics, nativeMedia,
    passed: cases.filter(item => item.status === 'PASS').length,
    failed: cases.filter(item => item.status === 'FAIL').length,
    logoutGeneration: settingsGeneration,
    limits: ['Retained playback cases include six decoded H264 cases; HTTP204 IDs9/10/11 remain modeled.',
      'New admin/settings APIs are Response fixtures; backend transactions, filesystem preservation and external YouTube are not qualified.',
      'Fresh owned headless browser and loopback fixture only; physical output and owner deployment are untested.'] };
  // Leave a real pending interval to prove production logout disposes it.
  pollSettings(settingsGeneration, 'scan', document.createElement('button'));
  reportProgress(cases.length + 1); // The final case crosses a document reload.
  sessionStorage.setItem('frayd-qa-logout', JSON.stringify(packet));
  document.getElementById('logout').click();
  return;

})().catch(error => fetch('/__fixture/result', { method: 'POST', headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ cases: [], errors: [error.stack], failed: 1, passed: 0 }) }));
