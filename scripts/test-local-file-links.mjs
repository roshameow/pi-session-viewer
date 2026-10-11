// Real react-markdown + Thread/App behavior with deterministic hooks and fake IPC.
// No filesystem reads of linked paths, native opener, SSH, Pi, or new dependencies.
// Verified method: walk the actual parser's JSX and retain component hook cells;
// context-only updates must reach anchors through memo without rerunning the parser.
// Pitfall: overriding every URL with identity admits unsafe schemes and file: images.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import React from 'react';
const root = fileURLToPath(new URL('..', import.meta.url));
const require = createRequire(import.meta.url);
const scratch = await mkdtemp(path.join(root, 'node_modules/.local-link-test-'));
const hooks = `
import React from ${JSON.stringify(require.resolve('react'))};
export const records = new Map();
let active, cursor = 0, contexts, effects = [];
export function begin(key, values) {
  if (!records.has(key)) records.set(key, { cells: [], dirty: false, consumed: new Map() });
  active = records.get(key); cursor = 0; contexts = values; active.consumed = new Map();
}
export function reset() {
  for (const r of records.values()) for (const c of r.cells) c?.cleanup?.();
  records.clear(); effects = [];
}
export function flushEffects() { effects.splice(0).forEach(fn => fn()); }
export function useState(initial) {
  const r = active, i = cursor++;
  if (!(i in r.cells)) r.cells[i] = { value: typeof initial === 'function' ? initial() : initial };
  return [r.cells[i].value, next => {
    r.cells[i].value = typeof next === 'function' ? next(r.cells[i].value) : next; r.dirty = true;
  }];
}
export function useRef(initial) { return useState(() => ({ current: initial }))[0]; }
const same = (a,b) => a && b && a.length === b.length && a.every((v,i) => Object.is(v,b[i]));
export function useMemo(fn,deps) {
  const i = cursor++;
  if (!active.cells[i] || !same(active.cells[i].deps,deps)) active.cells[i] = { value: fn(), deps };
  return active.cells[i].value;
}
export function useCallback(fn,deps) { return useMemo(() => fn,deps); }
export function useEffect(fn,deps) {
  const r = active, i = cursor++;
  if (!r.cells[i] || !same(r.cells[i].deps,deps)) {
    const old = r.cells[i], cell = { deps }; r.cells[i] = cell;
    effects.push(() => { old?.cleanup?.(); cell.cleanup = fn(); });
  }
}
function useContext(context) {
  const value = contexts.has(context) ? contexts.get(context) : context._currentValue;
  active.consumed.set(context,value); return value;
}
export default { ...React, useContext };
`;
const shallow = (a,b) => a && b && Object.keys(a).length === Object.keys(b).length && Object.keys(a).every(k => Object.is(a[k],b[k]));
const text = n => Array.isArray(n) ? n.map(text).join('') : n && typeof n === 'object' && n.props ? text(n.props.children) : n == null || typeof n === 'boolean' ? '' : String(n);
const nodes = (n,out=[]) => { if(Array.isArray(n)) n.forEach(x=>nodes(x,out)); else if(n?.props){out.push(n);nodes(n.props.children,out);} return out; };
const entry = (id,body) => ({kind:'message',id,parentId:null,role:'assistant',content:[{kind:'text',text:body}]});
const detail = (p='/sessions/actual.jsonl', body='[artifact](./artifacts/report.pdf)') => ({
  path:p,id:'synthetic',cwd:'/actual/header/cwd-not-derived-from-session-path',entries:[entry('stable',body)],active:[0],size:100,updatedAt:1,
  stats:{messageCount:1,tokenCount:1,costTotal:0},
});
const deferred = () => {let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
try {
  const outfile = path.join(scratch,'fixture.mjs');
  await build({stdin:{contents:`export { Markdown, MemoMarkdown, LocalFileLinkContext } from './src/components/Markdown'; export { Thread } from './src/components/Thread'; export { default as App } from './src/App'; export { isLocalFileHref, localFileUrlTransform } from './src/localFileLinks'; export { begin, reset, records, flushEffects } from 'react';`,resolveDir:root,loader:'tsx'},
    bundle:true,platform:'node',format:'esm',outfile,external:[require.resolve('react'),'react/jsx-runtime'],logLevel:'silent',plugins:[{name:'offline-local-links',setup(b){
      b.onResolve({filter:/^react$/},()=>({path:'react',namespace:'fixture'}));
      b.onResolve({filter:/^react-markdown$/},()=>({path:'markdown',namespace:'fixture'}));
      b.onResolve({filter:/^@tauri-apps\/api\//},args=>({path:args.path,namespace:'fixture'}));
      b.onResolve({filter:/^\.\/components\/(Sidebar|Thread|Composer|ConfigPanel|DataAccessPanel)$/},args=>({path:args.path,namespace:'fixture'}));
      b.onLoad({filter:/.*/,namespace:'fixture'},({path:p})=>{
        if(p==='react')return {contents:hooks};
        if(p==='markdown')return {resolveDir:root,contents:`import Real from ${JSON.stringify(require.resolve('react-markdown'))}; export { defaultUrlTransform } from ${JSON.stringify(require.resolve('react-markdown'))}; export default function ActualMarkdown(p) { globalThis.fixture.parses++; return Real(p); }`};
        if(p.endsWith('/core'))return {contents:'export class Channel {} export const invoke=(name,args)=>globalThis.fixture.invoke(name,args);'};
        if(p.endsWith('/window'))return {contents:'export const getCurrentWindow=()=>({onFocusChanged:()=>Promise.resolve(()=>{})});'};
        const name=p.split('/').pop();return {contents:`export function ${name}(props) {globalThis.fixture.${name}=props;return null;} ${name==='Thread'?'export const appendLiveEvents=(prev,batch)=>[...prev,...batch];':''}`};
      });
    }}]});
  const {Markdown,MemoMarkdown,LocalFileLinkContext,Thread,App,isLocalFileHref,localFileUrlTransform,begin,reset,records,flushEffects}=await import(pathToFileURL(outfile));
  let tree, rootNode, seen, f;
  function walk(node,key='root',values=new Map(),skip=false){
    if(Array.isArray(node))return node.map((n,i)=>walk(n,`${key}/${n?.key ?? i}`,values,skip));
    if(!node?.props)return node;
    let type=node.type;
    if(type?.$$typeof===Symbol.for('react.provider')){
      const next=new Map(values);next.set(type._context,node.props.value);
      return walk(node.props.children,key+'/provider',next,skip);
    }
    const memo=type?.$$typeof===Symbol.for('react.memo');if(memo)type=type.type;
    if(typeof type==='function'){
      const id=key+':'+type.name;seen.add(id);let record=records.get(id);
      const unchanged=record?.type===type && shallow(record.props,node.props);
      const contextChanged=record && [...record.consumed].some(([c,v])=>!Object.is(v,values.has(c)?values.get(c):c._currentValue));
      const reuse=record && unchanged && !record.dirty && !contextChanged && (skip || memo);
      if(!reuse){begin(id,values);record=records.get(id);record.output=type(node.props);record.type=type;record.props=node.props;record.dirty=false;}
      return walk(record.output,key+'/output',values,reuse);
    }
    if(type===React.Fragment)return walk(node.props.children,key+'/fragment',values,skip);
    return React.createElement(type,{...node.props,children:walk(node.props.children,key+'/children',values,skip)});
  }
  function render(){f.Thread=undefined;seen=new Set();tree=walk(rootNode);for(const [id,r] of records){if(!seen.has(id)){for(const c of r.cells)c?.cleanup?.();records.delete(id);}}flushEffects();}
  async function pump(){for(let i=0;i<25;i++){await Promise.resolve();render();}}
  function fixture(node,native=true){
    reset();f={calls:[],parses:0,invoke:async(name,args)=>{f.calls.push({name,args});assert.equal(name,'open_local_file');return '/resolved/by/backend';}};
    globalThis.fixture=f;
    globalThis.window={...(native?{__TAURI_INTERNALS__:{}}:{}),setTimeout:()=>1,clearTimeout:()=>{},addEventListener:()=>{},removeEventListener:()=>{}};
    globalThis.document={hidden:false,activeElement:null,addEventListener:()=>{},removeEventListener:()=>{}};
    globalThis.requestAnimationFrame=()=>1;globalThis.cancelAnimationFrame=()=>{};
    rootNode=node;render();return f;
  }
  const scope=(sessionPath='/sessions/actual.jsonl',allowed=true)=>({sessionPath,allowed});
  const markdown=(body,value=scope())=>React.createElement(LocalFileLinkContext.Provider,{value},React.createElement(MemoMarkdown,{text:body}));
  const anchor=label=>nodes(tree).find(n=>n.type==='a'&&text(n)===label);
  async function click(label){const a=anchor(label);assert.ok(a,`missing anchor ${label}`);let prevented=false;await a.props.onClick({preventDefault(){prevented=true;}});assert.equal(prevented,true);render();}

  const local=['file:///tmp/report%20one.pdf','file://localhost/tmp/report.pdf','/tmp/report.pdf','~/reports/report.pdf','./artifacts/report.pdf','../output.csv','notes.md','README.md:12:3','src/file.ts:12','artifacts/report.html','file:///tmp/report%20with%20images.htm'];
  fixture(markdown(local.map((href,i)=>`[local${i}](${href})`).join(' ')));
  assert.equal(f.calls.length,0,'render must never open files');
  for(const [i,href] of local.entries()){
    const a=anchor('local'+i);assert.equal(a.props.href,href,'actual parser must not strip local href');
    assert.equal(a.props.target,undefined);assert.equal(a.props['aria-disabled'],false);assert.match(a.props.title,/default app/);
    await click('local'+i);assert.deepEqual(f.calls.at(-1),{name:'open_local_file',args:{href,sessionPath:'/sessions/actual.jsonl'}});
  }
  assert.equal(f.calls.length,local.length);
  assert.match(anchor('local'+(local.length-1)).props.title,/HTML: default browser/);
  fixture(markdown('[space](<file:///tmp/spaced file.pdf>)'));
  assert.equal(anchor('space').props.href,'file:///tmp/spaced%20file.pdf');
  await click('space');
  assert.equal(f.calls[0].args.href,'file:///tmp/spaced%20file.pdf');
  assert.equal(isLocalFileHref('javascript:12'),false);
  console.log('PASS actual Markdown parser retains file/localhost/POSIX/home/relative anchors; click-only exact href+session IPC');

  fixture(markdown('[web](https://example.test/x) [http](http://example.test) [mail](mailto:test@example.test) [fragment](#section) [js](javascript:alert%281%29) [data](data:text/html,evil) [scheme](custom:evil) [network](//evil.test/file) [host](file://evil.test/tmp/a) ![file-image](file:///tmp/private.png) ![data-image](data:image/png,evil) ![js-image](javascript:evil) ![network-image](//evil.test/a.png) ![web-image](https://example.test/a.png)'));
  for(const label of ['web','http','mail','fragment']){
    const a=anchor(label);assert.ok(a.props.href);assert.equal(a.props.target,'_blank');assert.equal(a.props.rel,'noreferrer');assert.equal(a.props.onClick,undefined);
  }
  for(const label of ['js','data','scheme','network','host'])assert.equal(anchor(label).props.href,undefined,`${label} rejected`);
  const images=nodes(tree).filter(n=>n.type==='img');assert.equal(images.find(n=>n.props.alt==='file-image').props.src,'');assert.equal(images.find(n=>n.props.alt==='web-image').props.src,'https://example.test/a.png');
  for(const alt of ['data-image','js-image','network-image'])assert.equal(images.find(n=>n.props.alt===alt).props.src,'');
  assert.equal(f.calls.length,0);
  for(const bad of ['#fragment','//host/x','javascript:evil','data:text/html,x','file://server/x','file://user@localhost/x','file:relative','file://localhost:80/x','file:////server/x','\\server\\x',' /tmp/a'])assert.equal(isLocalFileHref(bad),false,bad);
  assert.equal(localFileUrlTransform('file:///tmp/a','src',{tagName:'img'}),'');
  console.log('PASS normal HTTP/mailto/fragment links, unsafe schemes/network authorities rejected, file: image still sanitized');

  fixture(markdown('[failed](./not-there.pdf)'));
  f.invoke=async(name,args)=>{f.calls.push({name,args});throw new Error('native opener failed');};
  await click('failed');assert.match(text(tree),/Could not open local file: Error: native opener failed/);assert.equal(nodes(tree).filter(n=>n.props.role==='alert').length,1);
  console.log('PASS failed native IPC produces visible inline accessible error');

  fixture(markdown('[browser](./report.pdf)'),false);await click('browser');assert.equal(f.calls.length,0);assert.match(text(tree),/desktop app.*browser\/demo/);
  fixture(React.createElement(Markdown,{text:'[unset](./report.pdf)'}));await click('unset');assert.equal(f.calls.length,0);assert.match(text(tree),/ready local session/);
  for(const props of [{preview:true,localFilesAllowed:true},{localFilesAllowed:false},{}]){
    fixture(React.createElement(Thread,{detail:detail(),liveBlocks:[],running:false,...props}));
    assert.equal(anchor('artifact').props['aria-disabled'],true);await click('artifact');assert.equal(f.calls.length,0);assert.match(text(tree),props.preview?/preview\/demo/:/remote or not-ready/);
  }
  console.log('PASS plain browser, missing context/prop, preview and remote never invoke opener and show reason');

  let threadProps={detail:detail('/sessions/path-does-not-represent-cwd.jsonl'),liveBlocks:[{kind:'assistant',text:'[live](../live.pdf)',thinking:'',done:true}],running:false,localFilesAllowed:true};
  fixture(React.createElement(Thread,threadProps));await click('artifact');await click('live');
  assert.deepEqual(f.calls.map(c=>c.args.sessionPath),[threadProps.detail.path,threadProps.detail.path]);assert.deepEqual(Object.keys(f.calls[0].args).sort(),['href','sessionPath']);
  const parseCount=f.parses;
  threadProps={...threadProps,localFilesAllowed:false};rootNode=React.createElement(Thread,threadProps);render();
  assert.equal(f.parses,parseCount,'context update must not reparse memoized Markdown');assert.equal(anchor('artifact').props['aria-disabled'],true);assert.equal(anchor('live').props['aria-disabled'],true);
  await click('artifact');assert.equal(f.calls.length,2);
  console.log('PASS real Thread entry+live scope routes session identity (not guessed cwd); context updates cross memo without reparse');

  const first=scope('/sessions/A.jsonl');fixture(markdown('[pending](./a.pdf)',first));
  const waiting=deferred();f.invoke=(name,args)=>{f.calls.push({name,args});return waiting.promise;};
  let prevented=false;const stale=anchor('pending').props.onClick;
  const operation=stale({preventDefault(){prevented=true;}});assert.equal(prevented,true);
  rootNode=markdown('[pending](./a.pdf)',scope('/sessions/A.jsonl',false));render();
  await stale({preventDefault(){}});assert.equal(f.calls.length,1,'stale handlers cannot open');
  rootNode=markdown('[pending](./a.pdf)',scope('/sessions/A.jsonl',true));render();
  waiting.reject('late old source error');await operation;render();assert.doesNotMatch(text(tree),/late old source error/);
  const unmount=deferred();f.invoke=()=>unmount.promise;const pending=anchor('pending').props.onClick({preventDefault(){}});
  rootNode=null;render();unmount.reject('unmounted error');await pending;assert.equal(records.size,0);
  console.log('PASS local→remote→local invalidates in-flight errors/stale handlers; unmount discards late failure');

  // Guard source changes before any render/context update, not just after it.
  let allowedNow=true;
  fixture(markdown('[guarded](./guarded.pdf)',{...scope(),validate:()=>allowedNow}));
  const staleGuarded=anchor('guarded').props.onClick;
  allowedNow=false;
  await staleGuarded({preventDefault(){}});render();
  assert.equal(f.calls.length,0);
  assert.match(text(tree),/Session\/source changed/);
  console.log('PASS synchronous scope validator rejects a stale click before context rerender');

  // Real App source sequencing; replace its heavy children only to inspect props.
  fixture(null);
  rootNode=React.createElement(App);
  f.invoke=async(name,args)=>{
    f.calls.push({name,args});
    switch(name){
      case 'get_remote_host': return null;
      case 'list_remote_hosts': return ['remote'];
      case 'pi_bin_path': return '/mock/pi';
      case 'pi_version': return 'mock';
      case 'list_projects': return [{key:'project',cwd:'/synthetic'}];
      case 'list_sessions': return [{path:'/sessions/A.jsonl',id:'A',size:1,updatedAt:1,running:false}];
      case 'session_detail_page': return detail(args.path);
      case 'list_running': return [];
      case 'session_status': return 'finished';
      case 'set_remote_host': return;
      case 'remote_sync_status': return {host:args.host,phase:'ready',usableCache:true,lastSuccessAt:1};
      default: throw Error('unmocked IPC '+name);
    }
  };
  render();await pump();assert.equal(f.Thread.localFilesAllowed,true);
  const switchWait=deferred();const normal=f.invoke;
  f.invoke=(name,args)=>name==='set_remote_host'?switchWait.promise:normal(name,args);
  const readyGuard=f.Thread.canOpenLocalFile;
  assert.equal(readyGuard(),true);
  const switching=f.Sidebar.onSwitchSource('remote');
  assert.equal(readyGuard(),false,'source epoch invalidates old link before render');
  await pump();
  if(f.Thread)assert.equal(f.Thread.localFilesAllowed,false,'pending source must disable');
  switchWait.resolve();await switching;await pump();assert.equal(f.Thread.localFilesAllowed,false,'remote source must disable');
  f.invoke=normal;await f.Sidebar.onSwitchSource(null);await pump();assert.equal(f.Thread.localFilesAllowed,true);
  assert.equal(readyGuard(),false,'local-remote-local must not revive first link validator');
  assert.equal(f.Thread.canOpenLocalFile(),true);
  console.log('PASS actual App allows only ready Local, disables pending/remote, re-enables Local');
  reset();console.log('All local-file-link tests passed (mock-only; no native opener/native cwd E2E).');
} finally {await rm(scratch,{recursive:true,force:true});}
