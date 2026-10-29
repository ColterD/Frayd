const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { createRequire } = require('node:module');
const vm = require('node:vm');
const { test } = require('node:test');
const harnessPath = join(__dirname, 'client.test.cjs');
const marker = "test('closing an ebook";
const source = readFileSync(harnessPath, 'utf8');
assert.ok(source.includes(marker), 'client.test.cjs harness marker is missing');
const harness = source.split(marker)[0];
const browser = vm.runInNewContext(harness + '\nbrowser;', { require: createRequire(harnessPath), __dirname, queueMicrotask, URL });

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
  b.respond(url => url === '/api/library' ? new Promise(resolve => { release = resolve; }) : Promise.resolve({ok:true,json:async()=>null}));
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

test('settings disposal cancels timer and rejects late status publication', async () => {
  const b=browser();
  let release;
  b.respond(()=>new Promise(resolve=>{release=resolve;}));
  b.run('settingsGeneration=1; pollSettings(1, "scan", {textContent:"waiting",disabled:true})');
  const poll=[...b.intervals.values()][0];
  const pending=poll();
  b.run('disposeSettings()');
  release({ok:true,json:async()=>({scanning:false})});
  await pending;
  assert.equal(b.intervals.size,0);
  assert.equal(b.requests.length,1);
});

test('settings scan refusal reports failure without refreshing the library', async () => {
  const b=browser();
  b.context.button={textContent:'Scanning…',disabled:true};
  b.run('settingsGeneration=1;pollSettings(1,"scan",button)');
  b.respond(async()=>({ok:true,json:async()=>({scanning:false,scan_error:'Library root unavailable'})}));
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
    if(status===202) {
      assert.equal(b.context.button.textContent,'Scanning…');
      assert.equal(b.intervals.size,1);
    } else {
      assert.equal(b.context.button.disabled,false);
      assert.equal(b.intervals.size,0);
      assert.match(b.context.button.textContent,status===409?/already running/:status===501?/No library folder/:status===500?/500/:/unreachable/);
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
  b.respond(async()=>({ok:true,json:async()=>({ingesting:'indexing'})}));
  await poll();
  assert.equal(b.context.button.textContent,'Indexing…');
  assert.equal(b.intervals.size,1);
  b.respond(async()=>({ok:true,json:async()=>({ingesting:'failed'})}));
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
  b.respond(url=>url.startsWith('/api/library?') ? new Promise(resolve=>{release=resolve;}) : Promise.resolve({ok:true,json:async()=>null}));
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
  b.run('account={isAdmin:true}; testCard=card({id:9,title:"Movie",kind:"movie"},false)');
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

test('late progress does not paint detached or superseded cards; row sorting is stable across refresh', async () => {
  const b=browser();
  let release;
  b.respond(()=>new Promise(resolve=>{release=resolve;}));
  b.run('testCard=card({id:9,title:"Movie",kind:"movie",duration_secs:100},false)');
  const card=b.run('testCard');
  const strip={style:{},firstElementChild:{style:{}}};
  card.isConnected=true;
  card.querySelector=()=>strip;
  b.run('++renderGeneration');
  release({ok:true,json:async()=>({position:30})});
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(strip.style.display,undefined);
  b.respond(async()=>({ok:true,json:async()=>null}));
  b.run('sortItems=[{id:2,title:"B",kind:"movie",created_at:1},{id:1,title:"A",kind:"movie",created_at:1},{id:3,title:"C",kind:"movie",created_at:2}];testSection=section("Movies",sortItems,false)');
  const section=b.run('testSection');
  const cards=section.children[1];
  assert.deepEqual(Array.from(cards.children,item=>item.title),['C','A','B']);
  section.children[0].children[0].textContent='oldest first';
  section.children[0].children[0].onclick();
  // The boundary double does not parse innerHTML; use a fresh section to observe persisted ordering.
  const refreshed=b.run('section("Movies",sortItems,false)');
  assert.deepEqual(Array.from(refreshed.children[1].children,item=>item.title),['A','B','C']);
});

test('production library rendering assigns book portraits and retains music/video landscapes', async () => {
  const b=browser();
  b.respond(async()=>({ok:true,json:async()=>null}));
  b.run('account={isAdmin:true}; LIBRARY=["movie","music","audiobook","ebook","comic","manga"].map((kind,id)=>({id:id+1,title:kind,kind,author:"Author"})); filtering=false');
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
  b.respond(async()=>({ok:true,json:async()=>({position:20})}));
  b.run('LIBRARY=["movie","music","audiobook","ebook","comic","manga"].map((kind,id)=>({id:id+1,title:kind,kind,duration_secs:100}))');
  const row=await b.run('continueRow()');
  const cards=row.children[1].children;
  assert.equal(cards.length,3);
  for(const card of cards) assert.equal(card.className,card.title==='audiobook'?'media-card portrait':'media-card landscape',card.title);
});
