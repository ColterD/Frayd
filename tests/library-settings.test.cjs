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
  const control=selector=>{if(!controls.has(selector)) controls.set(selector,{value:'',textContent:'',dataset:{},addEventListener(){}});return controls.get(selector);};
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
  const replacement = [...b.intervals.keys()][0];
  release(reply({scanning:false}));
  await old;
  assert.equal(b.intervals.has(replacement), true);
  assert.equal([...b.intervals.keys()][0], replacement);
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
  const replacement=[...b.intervals.keys()][0];
  reject(Error('retired connection'));
  await pending;
  assert.equal(b.intervals.has(replacement),true);
  assert.equal([...b.intervals.keys()][0],replacement);
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

// Dynamic DOM is the browser boundary; the complete application and fetch handlers run unchanged.
function passwordPanel(b) {
  b.respond(async url => reply(url.startsWith('/api/library') ? [] : null));
  const controls = new Map();
  const control = selector => {
    if (!controls.has(selector)) controls.set(selector, b.context.document.createElement('input'));
    return controls.get(selector);
  };
  const stage = b.nodes.get('#settings-stage');
  stage.querySelector = control;
  stage.querySelectorAll = () => [];
  for (const selector of ['#s-site-save', '#s-net-save', '#s-pw-save']) b.nodes.set(selector, control(selector));
  return { stage, control };
}

test('signed-in nonadmin can open own password panel without admin requests; guest has no entry', async () => {
  const b = browser();
  await settle();
  const { stage } = passwordPanel(b);
  b.run('account={name:"Member",isAdmin:false};enterApp()');
  await settle();
  assert.equal(b.nodes.get('#settings-btn').style.display, 'none');
  assert.equal(b.nodes.get('#account-btn').style.display, '');
  b.requests.length = 0;
  b.nodes.get('#account-btn').emit('click');
  assert.equal(b.requests.length, 0);
  assert.equal(b.nodes.get('#settings-modal').classList.contains('on'), true);
  assert.equal(b.nodes.get('#settings-title').textContent, 'Account');
  assert.match(stage.innerHTML, /Your password/);
  assert.doesNotMatch(stage.innerHTML, /Library folders|Accounts & permissions|s-net-save|p-admin/);
  b.run('account=null;enterApp()');
  await settle();
  assert.equal(b.nodes.get('#account-btn').style.display, 'none');
  assert.equal(b.nodes.get('#settings-btn').style.display, 'none');
  b.requests.length = 0;
  b.nodes.get('#account-btn').emit('click');
  assert.equal(b.requests.length, 0);
  assert.equal(b.nodes.get('#settings-modal').classList.contains('on'), false);
});

test('admin settings and account reentry share password submission and reset modal title', async () => {
  const b = browser({ storage: { frayd_token: 'public-test-token' } });
  await settle();
  const { stage, control } = passwordPanel(b);
  b.run('account={name:"Admin",isAdmin:true};enterApp()');
  await settle();
  assert.equal(b.nodes.get('#settings-btn').style.display, '');
  b.nodes.get('#account-btn').emit('click');
  b.respond(async url => reply(url.endsWith('/perms') ? [] : { library_paths: [] }));
  await b.run('openSettings()');
  assert.equal(b.nodes.get('#settings-title').textContent, 'Settings');
  assert.match(stage.innerHTML, /s-net-save/);
  assert.ok(b.requests.some(r => r.url === '/api/admin/settings'));
  assert.ok(b.requests.some(r => r.url === '/api/admin/perms'));
  for (const panel of ['settings', 'account']) {
    if (panel === 'account') b.nodes.get('#account-btn').emit('click');
    control('.pw-old').value = 'public-old-password';
    control('.pw-new').value = 'public-new-password';
    b.respond(async () => ({ ok: false, json: async () => ({ error: 'Current password incorrect' }) }));
    control('#s-pw-save').emit('click');
    await settle();
    assert.equal(control('#s-pw-save').textContent, 'Current password incorrect');
    assert.equal(control('.pw-new').value, 'public-new-password', 'failure does not claim success or clear input');
    b.respond(async () => reply({}));
    control('#s-pw-save').emit('click');
    await settle();
    const request = b.requests.at(-1);
    assert.equal(request.url, '/api/auth/password');
    assert.equal(request.options.method, 'POST');
    assert.equal(request.options.headers.Authorization, 'Bearer public-test-token');
    assert.deepEqual(JSON.parse(request.options.body), { old: 'public-old-password', new: 'public-new-password' });
    assert.equal(control('.pw-old').value, '');
    assert.equal(control('.pw-new').value, '');
    assert.equal(control('#s-pw-save').textContent, '✓ Changed');
  }
  assert.equal(b.nodes.get('#settings-title').textContent, 'Account');
});

for (const retirement of ['close', 'account switch', 'settings switch']) {
  for (const responsePhase of ['success', 'error body']) {
    test(`password ${responsePhase} cannot publish after ${retirement}`, async () => {
      const b = browser();
      await settle();
      const { control } = passwordPanel(b);
      b.run('account={name:"Member",isAdmin:false};enterApp()');
      await settle();
      b.nodes.get('#account-btn').emit('click');
      control('.pw-old').value = 'public-old-password';
      control('.pw-new').value = 'public-new-password';
      let release;
      b.respond(url => url.startsWith('/api/library') ? Promise.resolve(reply([])) : responsePhase === 'success'
        ? new Promise(resolve => { release = () => resolve(reply({})); })
        : Promise.resolve({ ok: false, json: () => new Promise(resolve => { release = () => resolve({ error: 'Late error' }); }) }));
      control('#s-pw-save').emit('click');
      await settle();
      if (retirement === 'close') b.nodes.get('#settings-close').emit('click');
      if (retirement === 'account switch') b.run('account={name:"Successor",isAdmin:false};enterApp()');
      if (retirement === 'settings switch') {
        b.respond(async url => reply(url.endsWith('/perms') ? [] : { library_paths: [] }));
        await b.run('openSettings()');
      }
      assert.equal(control('.pw-old').value, '');
      assert.equal(control('.pw-new').value, '');
      b.nodes.get('#account-btn').emit('click');
      control('.pw-new').value = 'successor-unsent-password';
      control('#s-pw-save').textContent = 'Successor panel';
      release();
      await settle();
      assert.equal(control('.pw-new').value, 'successor-unsent-password');
      assert.equal(control('#s-pw-save').textContent, 'Successor panel');
    });
  }
}

for (const retirement of ['close', 'account panel']) {
  test(`pending admin settings cannot replace ${retirement}`, async () => {
    const b = browser();
    await settle();
    const { stage } = passwordPanel(b);
    b.run('account={name:"Admin",isAdmin:true}');
    let release;
    b.respond(url => url.endsWith('/settings') ? new Promise(resolve => { release = resolve; }) : Promise.resolve(reply([])));
    const opening = b.run('openSettings()');
    if (retirement === 'close') b.nodes.get('#settings-close').emit('click');
    else b.nodes.get('#account-btn').emit('click');
    const markup = stage.innerHTML;
    release(reply({ library_paths: [] }));
    await opening;
    assert.equal(stage.innerHTML, markup);
    assert.equal(b.nodes.get('#settings-modal').classList.contains('on'), retirement !== 'close');
    if (retirement === 'account panel') assert.equal(b.nodes.get('#settings-title').textContent, 'Account');
  });
}

for (const first of ['scan', 'ingest']) {
  test(`overlapping scan and download both finish when ${first} completes first`, async () => {
    const b = browser();
    b.context.scanButton = { disabled: true, textContent: 'Scanning…' };
    b.context.ingestButton = { disabled: true, textContent: 'Downloading…' };
    b.run('pollSettings(settingsGeneration,"ingest",ingestButton)');
    const ingestPoll = [...b.intervals.values()][0];
    b.run('pollSettings(settingsGeneration,"scan",scanButton)');
    const scanPoll = [...b.intervals.values()].at(-1);
    assert.equal(b.intervals.size, 2, 'scan admission preserves the older download observer');
    let status = { scanning: first !== 'scan', ingesting: first === 'ingest' ? 'done' : 'indexing' };
    b.respond(async url => reply(url === '/api/scan/status' ? status : []));
    await (first === 'scan' ? scanPoll : ingestPoll)();
    assert.equal(b.intervals.size, 1, 'completing either mode preserves the other observer');
    assert.equal(b.context[first + 'Button'].disabled, false);
    assert.equal(b.context[first === 'scan' ? 'ingestButton' : 'scanButton'].disabled, true);
    status = { scanning: false, ingesting: 'done' };
    await (first === 'scan' ? ingestPoll : scanPoll)();
    assert.equal(b.intervals.size, 0);
    assert.equal(b.context.scanButton.textContent, 'Scan complete');
    assert.equal(b.context.ingestButton.textContent, 'Download complete');
    assert.equal(b.context.scanButton.disabled, false);
    assert.equal(b.context.ingestButton.disabled, false);
    assert.equal(b.requests.filter(r => r.url === '/api/library').length, 2);
  });
}

test('same-mode replacement retires only that mode and rejects its late status', async () => {
  const b = browser();
  b.context.oldButton = { disabled: true, textContent: 'Old scan' };
  let release;
  b.respond(url => url === '/api/scan/status' ? new Promise(resolve => { release = resolve; }) : Promise.resolve(reply([])));
  b.run('pollSettings(settingsGeneration,"ingest",{})');
  const ingestTimer = [...b.intervals.keys()][0];
  b.run('pollSettings(settingsGeneration,"scan",oldButton)');
  const oldTimer = [...b.intervals.keys()].at(-1);
  const pending = b.intervals.get(oldTimer)();
  b.run('pollSettings(settingsGeneration,"scan",{})');
  const replacement = [...b.intervals.keys()].at(-1);
  assert.equal(b.intervals.size, 2);
  assert.equal(b.intervals.has(ingestTimer), true);
  assert.equal(b.intervals.has(oldTimer), false);
  release(reply({ scanning: false, ingesting: 'done' }));
  await pending;
  assert.equal(b.intervals.has(replacement), true);
  assert.equal(b.intervals.has(ingestTimer), true);
  assert.equal(b.context.oldButton.textContent, 'Old scan');
  assert.equal(b.requests.filter(r => r.url === '/api/library').length, 0);
});

for (const mode of ['scan', 'ingest']) {
  test(`completed ${mode} reports library refresh failure and releases its button`, async () => {
    const b = browser();
    b.context.button = { disabled: true, textContent: 'Busy' };
    b.respond(async url => url === '/api/scan/status' ? reply({ scanning: false, ingesting: 'done' }) : { ok: false, status: 500 });
    b.run(`pollSettings(settingsGeneration,"${mode}",button)`);
    await [...b.intervals.values()][0]();
    assert.equal(b.intervals.size, 0);
    assert.equal(b.context.button.disabled, false);
    assert.match(b.context.button.textContent, /complete.*refresh failed/i);
    assert.doesNotMatch(b.context.button.textContent, /Scan failed|Download failed|Status unavailable/);
    assert.match(b.notices(), /refresh failed.*Library failed \(500\)/i);
  });
}

for (const retirement of ['panel close', 'account switch', 'same-mode replacement']) {
  test(`completion refresh ownership survives until response and rejects stale publication after ${retirement}`, async () => {
    const b = browser();
    b.context.button = { disabled: true, textContent: 'Scanning…' };
    let release;
    b.respond(url => url === '/api/scan/status' ? Promise.resolve(reply({ scanning: false }))
      : new Promise(resolve => { release = resolve; }));
    b.run('pollSettings(settingsGeneration,"ingest",{});pollSettings(settingsGeneration,"scan",button)');
    const pending = [...b.intervals.values()].at(-1)();
    await settle();
    assert.equal(b.intervals.size, 2, 'completion retains ownership while refresh is pending');
    assert.equal(b.context.button.disabled, true);
    if (retirement === 'panel close') b.nodes.get('#settings-close').emit('click');
    if (retirement === 'account switch') {
      b.respond(async () => reply([]));
      b.run('IDENTITY="successor-account";enterApp()');
      await settle();
    }
    if (retirement === 'same-mode replacement') b.run('pollSettings(settingsGeneration,"scan",{})');
    const notices = b.notices();
    const text = b.context.button.textContent;
    release({ ok: false, status: 500 });
    await pending;
    assert.equal(b.context.button.textContent, text);
    assert.equal(b.context.button.disabled, true);
    assert.equal(b.notices(), notices);
    assert.equal(b.intervals.size, retirement === 'account switch' ? 0 : retirement === 'panel close' ? 1 : 2);
  });
}

function sidebarNavigation(b, lineEnding) {
  // The fixture does not parse shelf buttons. Rebind the actual production nav block
  // against these DOM boundary nodes so both Account's listener and shelf onclick run.
  const source = require('node:fs').readFileSync(require('node:path').join(__dirname, '..', 'client/js/app.js'), 'utf8');
  const navigation = source.replace(/\r?\n/g, lineEnding).match(/\/\/ ---------- nav ----------\r?\n([\s\S]*?)\r?\nlet searchTimer;/);
  assert.ok(navigation, 'production navigation block is available');
  const shelves = ['all', 'movie', 'show', 'ebook'].map(kind => {
    const node = b.context.document.createElement('button');
    node.dataset.kind = kind;
    return node;
  });
  const buttons = [...shelves, b.nodes.get('#account-btn'), b.nodes.get('#settings-btn')];
  const select = selector => {
    if (!selector.startsWith('.nav-item')) return [];
    return buttons.filter(node => (!selector.includes('[data-kind]') || 'kind' in node.dataset)
      && (!selector.includes('.active') || node.classList.contains('active')));
  };
  const query = b.context.document.querySelector;
  b.context.document.querySelectorAll = select;
  b.context.document.querySelector = selector => selector.startsWith('.nav-item') ? select(selector)[0] || null : query(selector);
  b.run(navigation[1]);
  return shelves;
}

for (const lineEnding of ['\n', '\r\n']) {
  const ending = lineEnding === '\n' ? 'LF' : 'CRLF';
  for (const kind of ['all', 'movie', 'show', 'ebook']) {
    test(`opening Account preserves ${kind} shelf, active navigation and drill state (${ending})`, async () => {
      const b = browser();
      await settle();
      passwordPanel(b);
      const shelves = sidebarNavigation(b, lineEnding);
      const active = shelves.find(node => node.dataset.kind === kind);
      active.classList.add('active');
      b.context.shelf = kind;
      b.run('account={name:"Member",isAdmin:false};activeKind=libraryKind(shelf);activeQuery="retained search";filtering=true;showsDrill="Retained series";authorDrill.ebook={author:"Retained author",series:"Retained books"}');
      const state = b.run('JSON.stringify({activeKind,activeQuery,filtering,showsDrill,authorDrill})');
      const main = b.nodes.get('#main');
      main.innerHTML = '<div>Retained current view</div>';
      main.scrollTop = 321;
      b.requests.length = 0;
      b.nodes.get('#account-btn').emit('click');
      await settle();
      assert.equal(b.nodes.get('#settings-modal').classList.contains('on'), true);
      assert.equal(b.nodes.get('#settings-title').textContent, 'Account');
      assert.equal(b.run('JSON.stringify({activeKind,activeQuery,filtering,showsDrill,authorDrill})'), state);
      assert.equal(b.run('currentKind()'), kind);
      assert.equal(active.classList.contains('active'), true);
      assert.equal(b.nodes.get('#account-btn').classList.contains('active'), false);
      assert.equal(main.innerHTML, '<div>Retained current view</div>');
      assert.equal(main.scrollTop, 321);
      assert.equal(b.requests.length, 0);
    });
  }

  test(`Dashboard remains shelf navigation and clears retained drill state (${ending})`, async () => {
    const b = browser();
    await settle();
    const shelves = sidebarNavigation(b, lineEnding);
    const dashboard = shelves.find(node => node.dataset.kind === 'all');
    shelves.find(node => node.dataset.kind === 'ebook').classList.add('active');
    b.run('activeKind="ebook";showsDrill="Retained show";authorDrill.ebook={author:"Retained author"}');
    dashboard.emit('click');
    await settle();
    assert.equal(b.run('activeKind'), 'all');
    assert.equal(b.run('currentKind()'), 'all');
    assert.equal(b.run('showsDrill'), null);
    assert.equal(b.run('Object.values(authorDrill).every(value=>value===null)'), true);
    assert.equal(dashboard.classList.contains('active'), true);
    assert.equal(shelves.filter(node => node.classList.contains('active')).length, 1);
  });
}

for (const selector of ['#s-site-save', '#s-net-save']) {
  test(`${selector} replaces earlier success with pending then failure on 204 followed by 500`, async () => {
    const b = browser();
    const { control } = passwordPanel(b);
    b.respond(async url => reply(url.endsWith('/perms') ? [] : {library_paths:[]}));
    await b.run('openSettings()');
    control('.s-port').value = '8484';
    const button = b.nodes.get(selector);
    b.respond(async () => ({ok:true,status:204}));
    await button.onclick({target:button});
    assert.match(button.textContent, /Saved/);
    let release;
    b.respond(() => new Promise(resolve => {release=resolve;}));
    const pending = button.onclick({target:button});
    assert.equal(button.textContent, 'Saving…');
    release({ok:false,status:500});
    await pending;
    assert.equal(button.textContent, 'Save failed');
    assert.match(b.notices(), /500/);
  });
}

for (const listed of [false, true]) {
  test(`held library configuration repair guidance handles ${listed ? 'listed invalid entries' : 'empty normalized roots'} and escapes markup`, async () => {
    const b = browser();
    const { stage } = passwordPanel(b);
    b.respond(async url => reply(url.endsWith('/perms') ? [] : {
      library_paths: listed ? ['~/invalid<folder>'] : [],
      library_config_error: 'Invalid <img src=x onerror=alert(1)> configuration',
    }));
    await b.run('openSettings()');
    assert.match(stage.innerHTML, /Invalid &lt;img src=x onerror=alert\(1\)&gt; configuration/);
    assert.doesNotMatch(stage.innerHTML, /<img|<folder>/);
    if (listed) {
      assert.match(stage.innerHTML, /Remove invalid listed entries, then add a valid folder/);
      assert.match(stage.innerHTML, /class="lib-remove"/);
      assert.match(stage.innerHTML, /~\/invalid&lt;folder&gt;/);
      assert.doesNotMatch(stage.innerHTML, /Add a valid folder to replace the held configuration/);
    } else {
      assert.match(stage.innerHTML, /Add a valid folder to replace the held configuration/);
      assert.doesNotMatch(stage.innerHTML, /Remove invalid listed entries/);
    }
    assert.match(stage.innerHTML, /class="lib-add" >Add \+ scan/);
    assert.equal(b.requests.some(request => request.options?.method === 'POST'), false);
  });
}


for (const listed of [false, true]) {
  test(`held library configuration locks ${listed ? 'listed' : 'empty'} environment roots until operator repair`, async () => {
    const b = browser();
    const { stage, control } = passwordPanel(b);
    b.respond(async url => reply(url.endsWith('/perms') ? [] : {
      library_paths: listed ? ['/fixture/invalid-folder'] : [], library_locked: true,
      library_config_error: 'Invalid environment library configuration',
    }));
    await b.run('openSettings()');
    assert.match(stage.innerHTML, /Correct FRAYD_LIBRARY and restart the server\./);
    assert.doesNotMatch(stage.innerHTML, /Remove invalid listed entries|Add a valid folder/);
    assert.match(stage.innerHTML, /class="lib-add" disabled>Add \+ scan/);
    control('.s-lib-add').value = '/fixture/valid-folder';
    await control('.lib-add').onclick();
    assert.equal(b.requests.some(request => request.options?.method === 'POST'), false);
  });
}


for (const enabled of [false, true, undefined]) {
  test(`URL download availability ${String(enabled)} controls input action and admission`, async () => {
    const b = browser();
    const { stage, control } = passwordPanel(b);
    b.respond(async url => reply(url.startsWith('/api/library') || url.endsWith('/perms') ? [] : {
      library_paths:[], ...(enabled === undefined ? {} : {downloads_enabled:enabled}),
    }));
    await b.run('openSettings()');
    const input = control('.s-ingest');
    const button = control('.p-ingest');
    assert.equal(!!input.disabled, enabled === false);
    assert.equal(!!button.disabled, enabled === false);
    assert.match(stage.innerHTML, /yt-dlp/i);
    if (enabled === false) assert.match(stage.innerHTML, /Downloads are disabled.*yt-dlp.*outbound/i);
    input.value = 'https://fixture.invalid/video';
    b.respond(async url => url === '/api/ingest' ? {status:202} : reply(url === '/api/scan/status' ? {ingesting:'done'} : []));
    await button.onclick();
    assert.equal(b.requests.filter(request=>request.url==='/api/ingest').length, enabled === false ? 0 : 1);
    assert.equal(b.intervals.size, enabled === false ? 0 : 1);
    if (enabled !== false) {
      await [...b.intervals.values()][0]();
      assert.equal(button.disabled, false);
      assert.equal(button.textContent, 'Download complete');
    }
  });
}

for (const phase of ['downloading', 'indexing', 'done', 'failed', 'network']) {
  test(`capability-disabled download control stays disabled after monitor ${phase}`, async () => {
    const b = browser();
    const { control } = passwordPanel(b);
    b.respond(async url => reply(url.startsWith('/api/library') || url.endsWith('/perms') ? [] : {library_paths:[],downloads_enabled:false}));
    await b.run('openSettings()');
    const button = control('.p-ingest');
    const label = button.textContent;
    b.context.downloadButton = button;
    b.run('pollSettings(settingsGeneration,"ingest",downloadButton)');
    b.respond(async url => {
      if (phase === 'network') throw Error('offline');
      return reply(url === '/api/scan/status' ? {ingesting:phase} : []);
    });
    await [...b.intervals.values()][0]();
    assert.equal(button.disabled, true);
    assert.equal(button.textContent, label);
    assert.equal(control('.s-ingest').disabled, true);
  });
}

for (const phase of ['admission', 'monitor']) {
  for (const retire of ['panel', 'account']) {
    test(`retired enabled download ${phase} cannot alter successor disabled controls after ${retire} change`, async () => {
      const b = browser();
      let { control } = passwordPanel(b);
      b.respond(async url => reply(url.startsWith('/api/library') || url.endsWith('/perms') ? [] : {library_paths:[],downloads_enabled:true}));
      await b.run('openSettings()');
      control('.s-ingest').value = 'https://fixture.invalid/video';
      let release;
      b.respond(url => phase === 'admission' && url === '/api/ingest'
        ? new Promise(resolve=>{release=resolve;}) : Promise.resolve({status:202}));
      let pending = control('.p-ingest').onclick();
      if (phase === 'monitor') {
        await pending;
        b.respond(()=>new Promise(resolve=>{release=resolve;}));
        pending = [...b.intervals.values()][0]();
      }
      if (retire === 'account') {
        b.respond(async()=>reply([]));
        b.run('IDENTITY="replacement";account={name:"Replacement",isAdmin:true};enterApp()');
      }
      ({ control } = passwordPanel(b));
      b.respond(async url => reply(url.startsWith('/api/library') || url.endsWith('/perms') ? [] : {library_paths:[],downloads_enabled:false}));
      await b.run('openSettings()');
      const button = control('.p-ingest');
      const label = button.textContent;
      release(phase === 'admission' ? {status:500} : reply({ingesting:'done'}));
      await pending;
      assert.equal(button.disabled, true);
      assert.equal(button.textContent, label);
      assert.equal(control('.s-ingest').disabled, true);
      await button.onclick();
      assert.equal(b.requests.filter(request=>request.url==='/api/ingest').length, 1);
      await settle();
    });
  }
}


for (const scenario of [
  {name:'direct enabled',backend:'direct',enabled:true,direct:true},
  {name:'direct disabled',backend:'direct',enabled:false,direct:true,blocked:true},
  {name:'native enabled',backend:'yt-dlp',enabled:true},
  {name:'native disabled',backend:'yt-dlp',enabled:false,blocked:true},
  {name:'legacy metadata absent'},
  {name:'invalid backend',backend:null,enabled:false,error:'Invalid <img src=x onerror=alert(1)> download backend',blocked:true,invalid:true},
  {name:'unknown backend fails closed',backend:'unexpected',enabled:true,blocked:true,invalid:true},
  {name:'configuration error fails closed',backend:'direct',enabled:true,error:'Invalid download configuration',blocked:true,invalid:true},
]) {
  test(`download mode copy ${scenario.name} has truthful requirements and guarded admission`, async () => {
    const b=browser();
    const {stage,control}=passwordPanel(b);
    b.respond(async url=>reply(url.startsWith('/api/library')||url.endsWith('/perms')?[]:{
      library_paths:[],
      ...(scenario.backend===undefined?{}:{download_backend:scenario.backend}),
      ...(scenario.enabled===undefined?{}:{downloads_enabled:scenario.enabled}),
      download_config_error:scenario.error||null,
    }));
    await b.run('openSettings()');
    const row=stage.innerHTML.match(/<div class="perm-row"><span class="k">Add from URL<\/span>[\s\S]*?<\/div>/)?.[0];
    assert.ok(row);
    if(scenario.invalid){
      assert.match(row,/configuration.*corrected/i);
      if(scenario.error) assert.match(stage.innerHTML,/Invalid.*download/i);
    }else if(scenario.direct){
      assert.match(row,/placeholder="Direct MP4 video URL"/);
      assert.match(row,/Direct MP4 video URL only/);
      assert.match(row,/outbound/i);
      assert.doesNotMatch(row,/yt-dlp|YouTube|webpage/i);
      if(scenario.blocked) assert.match(row,/optional Docker downloads setup/);
    }else{
      assert.match(row,/placeholder="video URL \(yt-dlp supported\)"/);
      assert.match(row,/yt-dlp.*outbound/i);
      if(!scenario.blocked) assert.match(row,/additional setup/);
    }
    assert.doesNotMatch(stage.innerHTML,/<img/);
    if(scenario.error?.includes('<img')) assert.match(stage.innerHTML,/&lt;img src=x onerror=alert\(1\)&gt;/);
    const button=control('.p-ingest');
    assert.equal(button.disabled,!!scenario.blocked);
    assert.equal(control('.s-ingest').disabled,!!scenario.blocked);
    control('.s-ingest').value='https://fixture.invalid/video.mp4';
    b.respond(async()=>({status:501}));
    await button.onclick();
    assert.equal(b.requests.filter(r=>r.url==='/api/ingest').length,scenario.blocked?0:1);
    if(scenario.direct&&!scenario.blocked){
      assert.match(b.notices(),/configure a folder and enable direct downloads/);
      assert.doesNotMatch(b.notices(),/yt-dlp/);
    }
  });
}
