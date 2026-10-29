const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');
const { spawnSync } = require('node:child_process');
const page = readFileSync(process.env.FRAYD_INSTALL_PAGE_SOURCE || join(__dirname, '../client/install/index.html'), 'utf8');
const scriptOpening = '<script>';
const scriptStart = page.indexOf(scriptOpening);
assert.ok(scriptStart >= 0, 'installer fixture must contain its inline script');
const scriptEnd = page.indexOf('</script>', scriptStart + scriptOpening.length);
assert.ok(scriptEnd > scriptStart + scriptOpening.length, 'installer fixture must close its nonempty inline script');
const script = page.slice(scriptStart + scriptOpening.length, scriptEnd);
const json = value => ({ok:true,type:'cors',headers:{get:()=> 'application/json'},json:async()=>value});
const flush = () => new Promise(resolve => setImmediate(resolve));
function installer(response=async url=>json(url.endsWith('/health')?{status:'ok'}:{needed:true})) {
  const nodes=new Map();
  const node=selector=>{if(!nodes.has(selector)) nodes.set(selector,{value:'',checked:false,dataset:{},style:{},disabled:false,textContent:'',innerHTML:'',classList:{add(){},remove(){}},addEventListener(name,fn){this['on'+name]=fn;}});return nodes.get(selector);};
  for(const match of page.matchAll(/\bid="([^"]+)"/g)) node('#'+match[1]);
  const requests=[],timers=new Map(),events=new Map();
  let nextTimer=0;
  const location=new Proxy({hostname:'installer.example',href:'https://installer.example/install'}, {set(){throw Error('Automatic navigation forbidden');}});
  const context=vm.createContext({
    document:{querySelector:node,querySelectorAll:()=>[]},navigator:{userAgent:'Linux',clipboard:{writeText:async()=>{}}},
    location,URLSearchParams,AbortController,AbortSignal,
    fetch:async(url,options={})=>{requests.push({url,options});return response(url,options);},
    setTimeout:(fn,delay)=>{const id=++nextTimer;timers.set(id,{fn,delay});return id;},clearTimeout:id=>timers.delete(id),
    addEventListener:(name,fn)=>events.set(name,fn),
  });
  for(const name of ['sessionStorage','localStorage']) Object.defineProperty(context,name,{get(){throw Error('Credential storage forbidden');}});
  vm.runInContext(script,context);
  return {nodes,requests,timers,events,context,run:source=>vm.runInContext(source,context),respond:fn=>{response=fn;}};
}

test('installer requires typed health and setup JSON and keeps setup inside the app',async()=>{
  for(const needed of [true,false]) {
    const b=installer(async url=>json(url.endsWith('/health')?{status:'ok'}:{needed}));
    await flush();
    assert.match(b.nodes.get('#stat').textContent,/verified/i);
    assert.match(b.nodes.get('#s-stat').textContent,needed?/create.*owner.*in.*Frayd/i:/sign in.*Frayd/i);
    assert.match(page,/<a[^>]+href="http:\/\/127\.0\.0\.1:8484\/"[^>]*>Open Frayd/i);
    assert.equal(b.requests.length,2);
    assert.equal(b.requests.every(({url,options})=>url.startsWith('http://127.0.0.1:8484/') && !options.method && options.credentials==='omit'),true);
    assert.equal(b.timers.size,0);
    assert.equal(b.context.location.href,'https://installer.example/install');
    assert.doesNotMatch(page,/<input[^>]+(?:type="(?:email|password)"|id="s-(?:email|pw)")/i);
    assert.equal(b.nodes.has('#s-go'),false);
  }
});

test('opaque, unrelated HTML, malformed health and nonboolean setup never verify readiness',async()=>{
  const cases=[
    async()=>({ok:true,type:'opaque',headers:{get:()=>null},json:async()=>({status:'ok'})}),
    async()=>({ok:true,type:'cors',headers:{get:()=> 'text/html'},json:async()=>({status:'ok'})}),
    async()=>json({status:'other'}),
    async url=>json(url.endsWith('/health')?{status:'ok'}:{needed:'false'}),
    async()=>{throw Error('CORS unreadable');},
  ];
  for(const response of cases){const b=installer(response);await flush();assert.match(b.nodes.get('#stat').textContent,/unverified/i);assert.doesNotMatch(b.nodes.get('#stat').textContent,/^.*(?<!un)verified/i);assert.ok([...b.timers.values()].some(timer=>timer.delay===3000));}
});

test('checks are bounded and explicit retry starts a fresh generation',async()=>{
  const b=installer(async()=>{throw Error('offline');});await flush();
  for(let i=0;i<4;i++){
    const [id,timer]=[...b.timers].find(([,timer])=>timer.delay===3000);
    b.timers.delete(id);await timer.fn();await flush();
  }
  assert.equal(b.requests.length,5);assert.equal(b.timers.size,0);
  assert.match(b.nodes.get('#stat').textContent,/Retry/);
  b.respond(async url=>json(url.endsWith('/health')?{status:'ok'}:{needed:true}));
  b.nodes.get('#retry').onclick();await flush();
  assert.equal(b.requests.length,7);assert.match(b.nodes.get('#stat').textContent,/^Server protocol verified/);assert.equal(b.timers.size,0);
});

test('pagehide aborts and retires in-flight requests and timers; late success cannot publish',async()=>{
  let release;
  const b=installer(()=>new Promise(resolve=>{release=resolve;}));await flush();
  const signal=b.requests[0].options.signal;
  b.events.get('pagehide')();assert.equal(signal.aborted,true);assert.equal(b.timers.size,0);
  const before=b.nodes.get('#stat').textContent;
  release(json({status:'ok'}));await flush();
  assert.equal(b.requests.length,1);assert.equal(b.nodes.get('#stat').textContent,before);
});

test('new retry aborts prior request and ignores its later response',async()=>{
  let release;
  const b=installer(()=>new Promise(resolve=>{release=resolve;}));await flush();
  const signal=b.requests[0].options.signal;
  b.respond(async url=>json(url.endsWith('/health')?{status:'ok'}:{needed:false}));
  b.nodes.get('#retry').onclick();await flush();assert.equal(signal.aborted,true);
  const text=b.nodes.get('#s-stat').textContent;
  release(json({status:'ok'}));await flush();
  assert.equal(b.requests.length,3);assert.equal(b.nodes.get('#s-stat').textContent,text);assert.equal(b.timers.size,0);
});

test('download command links remain available for Linux, macOS and Windows',()=>{
  const b=installer();
  const shellCommand = 'curl -fsSL "https://raw.githubusercontent.com/kattiest/Frayd/main/client/install/bootstrap.sh" -o frayd-install.sh && bash frayd-install.sh';
  const commands = {
    linux: shellCommand,
    mac: shellCommand,
    win: 'cd $env:TEMP; irm "https://raw.githubusercontent.com/kattiest/Frayd/main/client/install/bootstrap.ps1" -OutFile frayd-install.ps1; & .\\frayd-install.ps1',
  };
  for(const os of ['linux','mac','win']){
    b.run(`os=${JSON.stringify(os)};gen()`);
    assert.equal(b.nodes.get('#cmd').textContent, commands[os]);
    assert.equal(b.nodes.get('#copy').style.display,'block');
  }
});

test('configured commands pass quoted library and tunnel arguments for each platform',()=>{
  const b=installer();
  const base='https://raw.githubusercontent.com/kattiest/Frayd/main/client/install';
  const cases=[
    {lib:'Media with spaces',tunnel:'tailscale',posixLib:"'Media with spaces'",posixTunnel:"'tailscale'",windowsLib:"'Media with spaces'",windowsTunnel:"'tailscale'"},
    {lib:"Owner's $Library with spaces",tunnel:"cloud'flare$Tunnel",posixLib:String.raw`'Owner'\''s $Library with spaces'`,posixTunnel:String.raw`'cloud'\''flare$Tunnel'`,windowsLib:"'Owner''s $Library with spaces'",windowsTunnel:"'cloud''flare$Tunnel'"},
  ];
  for(const fixture of cases){
    b.nodes.get('#lib').value=fixture.lib;
    b.nodes.get('#tunnel-cb').checked=true;
    b.nodes.get('#tunnel').value=fixture.tunnel;
    for(const os of ['linux','mac','win']){
      b.run(`os=${JSON.stringify(os)};gen()`);
      const expected=os==='win'
        ? `cd $env:TEMP; irm "${base}/bootstrap.ps1" -OutFile frayd-install.ps1; & .\\frayd-install.ps1 -Lib ${fixture.windowsLib} -Tunnel ${fixture.windowsTunnel}`
        : `curl -fsSL "${base}/bootstrap.sh" -o frayd-install.sh && bash frayd-install.sh ${fixture.posixLib} --tunnel ${fixture.posixTunnel}`;
      assert.equal(b.nodes.get('#cmd').textContent,expected);
    }
  }
});

test('POSIX generated commands preserve configured values at the actual shell argument boundary',()=>{
  const b=installer();
  const library="Owner's $Library with spaces";
  const tunnel="cloud'flare$Tunnel";
  b.nodes.get('#lib').value=library;
  b.nodes.get('#tunnel-cb').checked=true;
  b.nodes.get('#tunnel').value=tunnel;
  for(const os of ['linux','mac']){
    b.run(`os=${JSON.stringify(os)};gen()`);
    // Owned shell functions replace both commands; no downloader or installer executes.
    const stubs='curl() { return 0; }\nbash() { printf "%s\\0" "$@"; }\n';
    const result=spawnSync('/bin/sh',['-c',stubs+b.nodes.get('#cmd').textContent],{encoding:'utf8',env:{Library:'must-not-expand',Tunnel:'must-not-expand'}});
    assert.equal(result.status,0,result.stderr);
    assert.deepEqual(result.stdout.split('\0'),['frayd-install.sh',library,'--tunnel',tunnel,'']);
  }
});
