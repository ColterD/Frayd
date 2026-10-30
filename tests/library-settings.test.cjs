const assert = require('node:assert/strict');
const { test } = require('node:test');
const { browser, settle, reply } = require('./fixtures/client-browser.cjs');

test('authoritative roots, episode permissions and explicit null clearing', () => {
  const b = browser();
  assert.equal(b.run('KINDS.includes("episode")'), true);
  assert.equal(b.run('JSON.stringify(settingsRoots({library_paths: [], library_path:"/legacy"}))'), '[]');
  assert.equal(b.run('JSON.stringify(settingsRoots({library_path:"/legacy"}))'), '["/legacy"]');
  assert.equal(b.run('JSON.stringify(settingsRoots({library_locked:true,library_paths:["/db"],effective_library_paths:["/env"]}))'), '["/env"]');
  assert.equal(b.run('permissionKinds({querySelectorAll: selector => { if(selector !== ".p-kinds-box input:checked") throw Error(selector); return KINDS.map(value => ({value})); }})'), null);
});

test('newer library request owns publication and preserves active filter', async () => {
  const b = browser();
  let release;
  b.respond(url => url === '/api/library' ? new Promise(resolve => { release = resolve; }) : Promise.resolve(reply(null)));
  const old = b.run('loadLibrary()');
  b.respond(async () => ({ok:true,json:async()=>[{id:2,kind:'music',title:'New'}]}));
  await b.run('loadLibrary()');
  release({ok:true,json:async()=>[{id:9,kind:'movie',title:'Old'}]});
  await old;
  assert.equal(b.run('JSON.stringify(LIBRARY.map(i=>i.id))'), '[2]');
});

test('deletion invalidates pending loads and retains rows on refusal or cancel', async () => {
  const b = browser();
  b.run('LIBRARY=[{id:9,kind:"movie",title:"Movie"}]; confirm=()=>false');
  await b.run('deleteMedia(LIBRARY[0])');
  assert.equal(b.requests.length, 0);
  b.run('confirm=()=>true');
  b.respond(async () => ({ok:false,status:409}));
  await b.run('deleteMedia(LIBRARY[0])');
  assert.equal(b.run('LIBRARY.length'), 1);
  let release;
  b.respond(() => new Promise(resolve => {release=resolve;}));
  const pending=b.run('loadLibrary()');
  b.respond(async()=>({ok:true,status:204}));
  await b.run('deleteMedia(LIBRARY[0])');
  release({ok:true,json:async()=>[{id:9,kind:'movie'}]});
  await pending;
  assert.equal(b.run('LIBRARY.length'),0);
});

test('settings account retirement cancels timer and rejects late status publication', async () => {
  const b=browser();
  let release;
  b.respond(url => url === '/api/scan/status' ? new Promise(resolve=>{release=resolve;}) : Promise.resolve(reply(null)));
  b.run('settingsGeneration=1; pollSettings(1, "scan", {textContent:"waiting",disabled:true})');
  const poll=[...b.intervals.values()][0];
  const pending=poll();
  b.context.location.reload = () => {};
  b.nodes.get('#logout').emit('click');
  release(reply({scanning:false}));
  await pending;
  assert.equal(b.intervals.size,0);
  assert.equal(b.requests.filter(request => request.url === "/api/library").length,0);
});

test('settings scan refusal reports failure without refreshing the library', async () => {
  const b=browser();
  b.context.button={textContent:'Scanning…',disabled:true};
  b.run('settingsGeneration=1;pollSettings(1,"scan",button)');
  b.respond(async()=>reply({scanning:false,scan_error:'Library root unavailable'}));
  await [...b.intervals.values()][0]();
  assert.equal(b.context.button.textContent,'Scan failed');
  assert.equal(b.context.button.disabled,false);
  assert.equal(b.intervals.size,0);
  assert.equal(b.requests.some(request=>request.url==='/api/library'),false);
});

test('ebook shortcut guard excludes inactive, editable and button focus', () => {
  const b=browser();
  assert.equal(b.run('ebookShortcutAllowed({target:{tagName:"DIV"}})'),false);
  b.run('ebook={}; document.querySelector("#ebook-modal").classList.add("on")');
  for(const tagName of ['INPUT','TEXTAREA','SELECT','BUTTON']) {
    b.context.keyTarget={tagName};
    assert.equal(b.run('ebookShortcutAllowed({target:keyTarget})'),false);
  }
  assert.equal(b.run('ebookShortcutAllowed({target:{tagName:"DIV",isContentEditable:true}})'),false);
  assert.equal(b.run('ebookShortcutAllowed({target:{tagName:"DIV"}})'),true);
});

test('scan feedback distinguishes accepted, busy, no roots, server and network failure', async () => {
  for (const status of [202,409,501,500,'network']) {
    const b=browser();
    b.context.button={textContent:'',disabled:false};
    b.run('settingsGeneration=3');
    b.respond(async()=>{if(status==='network') throw Error('server unreachable'); return {status};});
    await b.run('startSettingsScan(3,button)');
    if(status===202 || status===409) {
      assert.equal(b.context.button.textContent,status===202 ? 'Scanning…' : 'Scan already running — monitoring…');
      assert.equal(b.intervals.size,1);
    } else {
      assert.equal(b.context.button.disabled,false);
      assert.equal(b.intervals.size,0);
      assert.match(b.context.button.textContent,status===501?/No library folder/:status===500?/500/:/unreachable/);
    }
  }
});

test('settings saves fail honestly and stale success cannot publish', async () => {
  const b=browser();
  for(const status of [400,409,500]) {
    b.respond(async()=>({ok:false,status}));
    await assert.rejects(b.run('saveSettings({library_paths:[]},settingsGeneration)'), status===409?/override or active job/:new RegExp(String(status)));
  }
  b.respond(async()=>({ok:true}));
  b.run('settingsGeneration=5');
  assert.equal(await b.run('saveSettings({library_paths:[]},4)'),false);
  assert.deepEqual(JSON.parse(b.requests.at(-1).options.body),{library_paths:[]});
});

test('ingest indexing keeps its poll until failure and never refreshes library', async () => {
  const b=browser();
  b.context.button={textContent:'',disabled:true};
  b.run('settingsGeneration=1;pollSettings(1,"ingest",button)');
  const poll=[...b.intervals.values()][0];
  b.respond(async()=>reply({ingesting:'indexing'}));
  await poll();
  assert.equal(b.context.button.textContent,'Indexing…');
  assert.equal(b.intervals.size,1);
  b.respond(async()=>reply({ingesting:'failed'}));
  await poll();
  assert.equal(b.context.button.textContent,'Download failed');
  assert.equal(b.context.button.disabled,false);
  assert.equal(b.intervals.size,0);
  assert.equal(b.requests.every(request=>request.url==='/api/scan/status'),true);
});

test('stale search and continue rows cannot replace a newer filter', async () => {
  const b=browser();
  b.run('LIBRARY=[{id:1,kind:"movie",title:"Movie"},{id:2,kind:"music",title:"Song"}]');
  let release;
  b.respond(url=>url.startsWith('/api/library?') ? new Promise(resolve=>{release=resolve;}) : Promise.resolve(reply(null)));
  const pending=b.run('serverSearch("old")');
  await b.run('filterKind("music","")');
  const html=b.nodes.get('#main').innerHTML;
  release({ok:true,json:async()=>[{id:9,kind:'movie',title:'Old'}]});
  await pending;
  assert.equal(b.run('activeKind'),'music');
  assert.equal(b.nodes.get('#main').innerHTML,html);
  assert.equal(b.run('activeQuery'),'');
});

test('admin delete survives card markup and stops click and key bubbling', async () => {
  const b=browser();
  b.run('account={isAdmin:true}; testCard=card({id:9,title:"Movie",kind:"movie"},resumeFor)');
  const card=b.run('testCard');
  const button=card.children.find(child=>child.className==='card-del');
  assert.ok(button);
  let stopped=0;
  button.onkeydown({stopPropagation(){stopped++;}});
  await button.onclick({stopPropagation(){stopped++;}});
  assert.equal(stopped,2);
  assert.equal(b.requests.filter(request=>request.options?.method==='DELETE').length,0);
});

test('settings render locks root controls, sends scoped permissions and cancels user deletion', async () => {
  const b=browser();
  const controls=new Map();
  const control=selector=>{if(!controls.has(selector)) controls.set(selector,{value:'',textContent:'',addEventListener(){}});return controls.get(selector);};
  let permissionRow;
  const save={closest:()=>permissionRow};
  const remove={dataset:{name:'User'},closest:()=>({dataset:{id:'3'}})};
  const stage=b.nodes.get('#settings-stage');
  stage.querySelector=control;
  stage.querySelectorAll=selector=>selector==='.p-save'?[save]:selector==='.p-remove'?[remove]:[];
  for(const selector of ['#s-site-save','#s-net-save']) b.nodes.set(selector,control(selector));
  b.respond(async url=>({ok:true,json:async()=>url.endsWith('/perms')?[{identity_id:3,key:'User',is_admin:true,allowed_kinds:null}]:{library_paths:[],library_path:'/obsolete',library_locked:true,server_version:'0.1.1'}}));
  await b.run('openSettings()');
  assert.match(stage.innerHTML,/locked by FRAYD_LIBRARY/);
  assert.match(stage.innerHTML,/class="lib-add" disabled/);
  assert.doesNotMatch(stage.innerHTML,/obsolete/);
  permissionRow={dataset:{id:'3'},querySelector:()=>({checked:true}),querySelectorAll:selector=>{
    assert.equal(selector,'.p-kinds-box input:checked');
    return ['movie','episode','music','audiobook','ebook','comic','manga'].map(value=>({value}));
  }};
  b.respond(async()=>({ok:true}));
  await save.onclick();
  assert.deepEqual(JSON.parse(b.requests.at(-1).options.body),{identity_id:3,is_admin:true,allowed_kinds:null});
  const count=b.requests.length;
  await remove.onclick();
  assert.equal(b.requests.length,count);
  await control('.lib-add').onclick({target:control('.lib-add')});
  assert.equal(b.requests.length,count);
});

for (const rerender of ['fails', 'is closed']) {
  test(`settings Add + scan starts the saved folder scan when its re-render ${rerender}`, async () => {
    const b = browser();
    const controls = new Map();
    const control = selector => {
      if (!controls.has(selector)) controls.set(selector, { value: '', textContent: '', addEventListener() {} });
      return controls.get(selector);
    };
    const stage = b.nodes.get('#settings-stage');
    // The boundary double does not parse innerHTML; only a rendered panel owns a scan button.
    stage.querySelector = selector => selector === '.p-scan' && !stage.innerHTML.includes('class="p-scan"')
      ? null : control(selector);
    stage.querySelectorAll = () => [];
    for (const selector of ['#s-site-save', '#s-net-save']) b.nodes.set(selector, control(selector));
    let reads = 0;
    let releaseRerender;
    b.respond(async (url, options) => {
      if (url === '/api/scan') return { ok: true, status: 202 };
      if (url === '/api/scan/status') return { ok: true, status: 200, json: async () => ({ scanning: true }) };
      if (options?.method === 'POST') return { ok: true, status: 200 };
      if (url === '/api/admin/settings' && ++reads === 2) {
        if (rerender === 'fails') return { ok: false, status: 500 };
        await new Promise(resolve => { releaseRerender = resolve; });
      }
      return { ok: true, status: 200, json: async () => url.endsWith('/perms') ? [] : { library_paths: [] } };
    });
    await b.run('openSettings()');
    control('.s-lib-add').value = '/media/new';
    const adding = control('.lib-add').onclick();
    if (rerender === 'is closed') {
      await settle();
      b.nodes.get('#settings-close').emit('click');
      releaseRerender();
    }
    await adding;
    assert.deepEqual(JSON.parse(b.requests.find(r => r.options?.method === 'POST').options.body),
      { library_paths: ['/media/new'] });
    assert.equal(b.requests.filter(r => r.url === '/api/scan' && r.options?.method === 'POST').length, 1);
    assert.equal(b.intervals.size, 1, 'the requested scan is monitored');
    const notices = b.notices();
    assert.match(notices, /Scanning library/);
  });
}

test('late progress does not paint detached or superseded cards; row sorting is stable across refresh', async () => {
  const b=browser();
  let release;
  b.respond(()=>new Promise(resolve=>{release=resolve;}));
  b.run('testCard=card({id:9,title:"Movie",kind:"movie",duration_secs:100},resumeFor)');
  const card=b.run('testCard');
  const strip={style:{},firstElementChild:{style:{}}};
  card.isConnected=true;
  card.querySelector=()=>strip;
  b.run('++renderGeneration');
  release(reply({position:30}));
  await settle();
  assert.equal(strip.style.display,undefined);
  b.respond(async()=>reply(null));
  b.run('sortItems=[{id:2,title:"B",kind:"movie",created_at:1},{id:1,title:"A",kind:"movie",created_at:1},{id:3,title:"C",kind:"movie",created_at:2}];testSection=section("movie",sortItems,resumeFor)');
  const section=b.run('testSection');
  const cards=section.children[1];
  assert.deepEqual(Array.from(cards.children,item=>item.title),['C','A','B']);
  section.children[0].children[0].onclick();
  // The boundary double does not parse innerHTML; use a fresh section to observe persisted ordering.
  const refreshed=b.run('section("movie",sortItems,resumeFor)');
  assert.deepEqual(Array.from(refreshed.children[1].children,item=>item.title),['A','B','C']);
});

test('production library rendering assigns book portraits and retains music/video landscapes', async () => {
  const b=browser();
  b.respond(async()=>reply(null));
  b.run('account={isAdmin:true}; LIBRARY=["movie","music","audiobook","ebook","comic","manga"].map((kind,id)=>({id:id+1,title:kind,kind,author:"Author"})); filtering=false; authorDrill.music={author:"Author"}; authorDrill.ebook={author:"Author"}; authorDrill.audiobook={author:"Author"}');
  await b.run('renderRows(LIBRARY)');
  const cards=[];
  const visit=node=>{if(node.className?.startsWith('media-card ')) cards.push(node);for(const child of node.children||[])visit(child);};
  visit(b.nodes.get('#main'));
  assert.equal(cards.length,6);
  for(const card of cards){
    const kind=card.title.split(' — ')[0];
    assert.equal(card.className,['movie','music'].includes(kind)?'media-card landscape':'media-card portrait',kind);
    assert.equal(card.children.filter(child=>child.className==='card-del').length,1);
    assert.equal(typeof card.onclick,'function');
  }
});

test('continue row derives audiobook portrait while retaining playable music/video landscapes', async () => {
  const b=browser();
  b.respond(async()=>reply({position:20}));
  b.run('LIBRARY=["movie","music","audiobook","ebook","comic","manga"].map((kind,id)=>({id:id+1,title:kind,kind,duration_secs:100}))');
  const row=await b.run('continueRow(LIBRARY,resumeFor)');
  const cards=row.children[1].children;
  assert.equal(cards.length,3);
  for(const card of cards) assert.equal(card.className,card.title==='audiobook'?'media-card portrait':'media-card landscape',card.title);
});

for (const closeBeforeAdmission of [false, true]) {
  test(`admitted settings scan refresh survives close and reopen (beforeAdmission=${closeBeforeAdmission})`, async () => {
    const b = browser();
    b.context.button = { textContent: '', disabled: false };
    let release;
    b.respond(url => url === '/api/scan' ? new Promise(resolve => { release = resolve; })
      : Promise.resolve({ ok: true, json: async () => url === '/api/scan/status' ? { scanning: false } : [] }));
    const admission = b.run('startSettingsScan(settingsGeneration, button)');
    if (closeBeforeAdmission) b.run('disposeSettings()');
    release({ status: 202 });
    await admission;
    if (!closeBeforeAdmission) b.run('disposeSettings()');
    await b.run('openSettings()');
    assert.equal(b.intervals.size, 1, 'one account-owned completion observer survives');
    await [...b.intervals.values()][0]();
    assert.equal(b.requests.filter(request => request.url === '/api/library').length, 1);
    assert.equal(b.context.button.disabled, true, 'retired button is untouched');
    assert.equal(b.intervals.size, 0);
  });
}

test('retired settings status cannot clear a replacement monitor', async () => {
  const b = browser();
  let release;
  b.respond(url => url === '/api/scan/status' ? new Promise(resolve => { release = resolve; }) : Promise.resolve(reply([])));
  b.run('pollSettings(settingsGeneration,"scan",{})');
  const old = [...b.intervals.values()][0]();
  b.run('pollSettings(settingsGeneration,"scan",{})');
  const replacement = b.run('settingsPoll');
  release(reply({scanning:false}));
  await old;
  assert.equal(b.intervals.has(replacement), true);
  assert.equal(b.run('settingsPoll'), replacement);
});

for (const phase of ['admission', 'status']) {
  test(`account switch rejects old settings scan ${phase} and stops monitoring`, async () => {
    const b = browser();
    let release;
    b.context.button = {textContent:'',disabled:false};
    b.respond(url => url === (phase === 'admission' ? '/api/scan' : '/api/scan/status')
      ? new Promise(resolve => { release = resolve; })
      : Promise.resolve({status:202,ok:true,json:async()=>[]}));
    const admission = b.run('startSettingsScan(settingsGeneration, button)');
    let pending;
    if (phase === 'status') { await admission; pending = [...b.intervals.values()][0](); }
    b.run('IDENTITY="new-account";enterApp()');
    b.requests.length = 0;
    release({status:202,ok:true,json:async()=>({scanning:false})});
    await (pending || admission);
    assert.equal(b.intervals.size, 0);
    assert.equal(b.requests.filter(request => request.url === '/api/library').length, 0);
    assert.notEqual(b.context.button.textContent, 'Scan complete');
  });
}

for (const failure of ['network', 'server']) {
  test(`settings scan status retries ${failure} and refreshes once after recovery`, async () => {
    const b = browser();
    b.context.button = {disabled:true,textContent:'Scanning…'};
    let reads = 0;
    b.respond(async url => {
      if (url !== '/api/scan/status') return reply([]);
      if (++reads === 1) {
        if (failure === 'network') throw Error('connection reset');
        return {ok:false,status:503};
      }
      return reply({scanning:false});
    });
    b.run('pollSettings(settingsGeneration,"scan",button)');
    const poll = [...b.intervals.values()][0];
    await poll();
    assert.equal(b.intervals.size,1,'transient failure retains observer');
    assert.equal(b.context.button.disabled,true);
    await poll();
    assert.equal(b.intervals.size,0);
    assert.equal(reads,2);
    assert.equal(b.requests.filter(r=>r.url==='/api/library').length,1);
    assert.equal(b.context.button.disabled,false);
  });
}

test('settings scan status stops after three consecutive failures; valid busy status resets the count', async () => {
  const b = browser();
  b.context.button = {disabled:true,textContent:'Scanning…'};
  let reads = 0;
  b.respond(async()=>++reads === 3 ? reply({scanning:true}) : {ok:false,status:502});
  b.run('pollSettings(settingsGeneration,"scan",button)');
  const poll = [...b.intervals.values()][0];
  for (let n=0;n<5;++n) {
    await poll();
    assert.equal(b.intervals.size,1,`observer survives read ${n+1}`);
    assert.equal(b.context.button.disabled,true);
  }
  await poll();
  assert.equal(reads,6);
  assert.equal(b.intervals.size,0);
  assert.equal(b.context.button.disabled,false);
  assert.equal(b.context.button.textContent,'Status unavailable');
  assert.equal(b.requests.filter(r=>r.url==='/api/library').length,0);
});

for (const mode of ['scan','ingest']) {
  test(`settings ${mode} status never overlaps requests and deterministic HTTP failure stops`, async () => {
    const b=browser();
    b.context.button={disabled:true,textContent:'Busy'};
    let release;
    b.respond(()=>new Promise(resolve=>{release=resolve;}));
    b.run(`pollSettings(settingsGeneration,"${mode}",button)`);
    const poll=[...b.intervals.values()][0];
    const pending=poll();
    await poll();
    assert.equal(b.requests.length,1);
    release({ok:false,status:403});
    await pending;
    assert.equal(b.intervals.size,0);
    assert.equal(b.context.button.disabled,false);
  });
}

test('settings ingest network failure stops immediately', async () => {
  const b=browser();
  b.context.button={disabled:true,textContent:'Busy'};
  b.respond(async()=>{throw Error('offline');});
  b.run('pollSettings(settingsGeneration,"ingest",button)');
  await [...b.intervals.values()][0]();
  assert.equal(b.intervals.size,0);
  assert.equal(b.context.button.disabled,false);
});

for (const closeBeforeAdmission of [false,true]) {
  test(`settings busy scan attaches one observer after POST 409 (closed=${closeBeforeAdmission})`, async () => {
    const b=browser();
    b.context.button={disabled:false,textContent:''};
    let release;
    b.respond(url=>url==='/api/scan' ? new Promise(resolve=>{release=resolve;})
      : Promise.resolve({ok:true,json:async()=>url==='/api/scan/status'?{scanning:false}:[]}));
    const admission=b.run('startSettingsScan(settingsGeneration,button)');
    if(closeBeforeAdmission)b.run('disposeSettings()');
    release({status:409});
    await admission;
    assert.equal(b.intervals.size,1);
    await [...b.intervals.values()][0]();
    assert.equal(b.requests.filter(r=>r.url==='/api/scan').length,1,'admission is never replayed');
    assert.equal(b.requests.filter(r=>r.url==='/api/library').length,1);
    assert.equal(b.context.button.disabled,closeBeforeAdmission);
    assert.equal(b.intervals.size,0);
  });
}

test('settings POST 409 monitor logout rejects late status failure without UI publication', async () => {
  const b=browser();
  b.context.button={disabled:false,textContent:''};
  b.context.location.reload=()=>{};
  let reject;
  b.respond(url=>url==='/api/scan/status' ? new Promise((resolve,fail)=>{reject=fail;})
    : Promise.resolve({status:409,ok:true,json:async()=>null}));
  await b.run('startSettingsScan(settingsGeneration,button)');
  const pending=[...b.intervals.values()][0]();
  const text=b.context.button.textContent;
  b.nodes.get('#logout').emit('click');
  reject(Error('retired connection'));
  await pending;
  assert.equal(b.intervals.size,0);
  assert.equal(b.context.button.disabled,true);
  assert.equal(b.context.button.textContent,text);
  assert.equal(b.requests.filter(r=>r.url==='/api/library').length,0);
});

test('settings retired scan retry cannot clear a replacement observer or mutate closed controls', async () => {
  const b=browser();
  b.context.button={disabled:true,textContent:'Scanning…'};
  let reject;
  b.respond(()=>new Promise((resolve,fail)=>{reject=fail;}));
  b.run('pollSettings(settingsGeneration,"scan",button)');
  const pending=[...b.intervals.values()][0]();
  b.run('disposeSettings();pollSettings(settingsGeneration,"scan",{})');
  const replacement=b.run('settingsPoll');
  reject(Error('retired connection'));
  await pending;
  assert.equal(b.intervals.has(replacement),true);
  assert.equal(b.run('settingsPoll'),replacement);
  assert.equal(b.context.button.disabled,true);
  assert.equal(b.context.button.textContent,'Scanning…');
});

test('settings scan malformed status JSON fails fast', async () => {
  const b=browser();
  b.context.button={disabled:true,textContent:'Scanning…'};
  b.respond(async()=>({ok:true,json:async()=>{throw SyntaxError('malformed status');}}));
  b.run('pollSettings(settingsGeneration,"scan",button)');
  await [...b.intervals.values()][0]();
  assert.equal(b.intervals.size,0);
  assert.equal(b.context.button.disabled,false);
  assert.equal(b.requests.length,1);
});

test('closed settings scan exhausts retry budget without mutating its retired button or publishing toasts', async () => {
  const b=browser();
  b.context.button={disabled:true,textContent:'Scanning…'};
  b.respond(async()=>({ok:false,status:503}));
  b.run('pollSettings(settingsGeneration,"scan",button);disposeSettings()');
  const poll=[...b.intervals.values()][0];
  const children=b.context.document.body.children.length;
  for(let n=0;n<3;++n)await poll();
  assert.equal(b.intervals.size,0);
  assert.equal(b.context.button.disabled,true);
  assert.equal(b.context.button.textContent,'Scanning…');
  assert.equal(b.context.document.body.children.length,children);
  assert.equal(b.requests.filter(r=>r.url==='/api/scan/status').length,3);
});
