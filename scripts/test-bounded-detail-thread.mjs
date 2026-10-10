// Offline real Thread/hooks/API tests. Synthetic records only; no Pi/SSH/desktop.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';
const root = fileURLToPath(new URL('..', import.meta.url));
const require = createRequire(import.meta.url);
const scratch = await mkdtemp(path.join(root, 'node_modules/.bounded-thread-'));
const hooks = `
import React from ${JSON.stringify(require.resolve('react'))};
let cells = [], cursor = 0, effects = [];
export function reset() { for (const c of cells) c?.cleanup?.(); cells = []; effects = []; }
export function begin() { cursor = 0; }
export function flushEffects() { const batch = effects.splice(0); batch.forEach(fn => fn()); }
export function useState(initial) {
  const i = cursor++;
  if (!(i in cells)) cells[i] = { value: typeof initial === 'function' ? initial() : initial };
  return [cells[i].value, next => { cells[i].value = typeof next === 'function' ? next(cells[i].value) : next; }];
}
export function useRef(initial) { return useState(() => ({ current: initial }))[0]; }
const same = (a,b) => a && b && a.length === b.length && a.every((v,i) => Object.is(v,b[i]));
export function useMemo(fn, deps) { const i = cursor++; if (!cells[i] || !same(cells[i].deps, deps)) cells[i] = { value: fn(), deps }; return cells[i].value; }
export function useCallback(fn, deps) { return useMemo(() => fn, deps); }
export function useEffect(fn, deps) {
  const i = cursor++;
  if (!cells[i] || !same(cells[i].deps, deps)) {
    const previous = cells[i]; const cell = { deps }; cells[i] = cell;
    effects.push(() => { previous?.cleanup?.(); cell.cleanup = fn(); });
  }
}
export default React;
`;

const entry = (id, role='user', content=[{kind:'text',text:id}]) => ({kind:'message',id,parentId:null,ts:null,role,content,
  model:null,toolName:null,toolCallId:null,isError:false,summary:null,name:null,label:null});
const detail = (entries, page) => ({id:'synthetic',path:'/synthetic/52mb.jsonl',cwd:'/synthetic',taskId:null,createdIso:'',
  entries,active:entries.map((_,i)=>i),size:52*1024*1024,updatedAt:1,
  stats:{messageCount:4000,tokenCount:200,model:'fixture',provider:'offline',thinkingLevel:'high',contextTokens:10,contextLimit:100,costTotal:1},page});
const page = (overrides={}) => ({generation:'fixture-g1',previousCursor:'opaque-cursor',totalEntries:4000,branchEntries:3800,
  matchedEntries:3700,returnedEntries:1,hasMore:true,incompleteTail:false,malformedLines:0,
  counters:{messages:3800,user:2000,assistant:1000,toolResult:800,labeled:2},toolPairs:[],payloadBytes:2000,...overrides});
function nodes(node, out=[]) {
  if(Array.isArray(node)) node.forEach(n=>nodes(n,out));
  else if(node && typeof node === 'object' && node.props) {out.push(node);nodes(node.props.children,out);}
  return out;
}
const text = n => Array.isArray(n) ? n.map(text).join('') : n && typeof n === 'object' && n.props ? text(n.props.children) : n == null || typeof n === 'boolean' ? '' : String(n);
try {
  const outfile=path.join(scratch,'thread.mjs');
  await build({stdin:{contents:`export * from './src/components/Thread'; export * from './src/threadItems'; export { api } from './src/api'; export { begin,reset,flushEffects } from 'react';`,resolveDir:root,loader:'tsx'},
    bundle:true,platform:'node',format:'esm',outfile,external:[require.resolve('react'),'react/jsx-runtime'],logLevel:'silent',plugins:[{name:'offline-thread',setup(b){
      b.onResolve({filter:/^react$/},()=>({path:'react',namespace:'fixture'}));
      b.onResolve({filter:/^@tauri-apps\/api\/core$/},()=>({path:'ipc',namespace:'fixture'}));
      b.onLoad({filter:/.*/,namespace:'fixture'},({path:p})=>({contents:p==='react'?hooks:'export const invoke=(command,args)=>globalThis.fixture.invoke(command,args);'}));
    }}]});
  const { Thread,FullBodyView,buildThreadItems,prependDetailPage,api,begin,reset,flushEffects }=await import(pathToFileURL(outfile));
  let tree, timers, clock, nextId, props;
  const render=()=>{begin();tree=Thread(props);flushEffects();};
  async function pump(){for(let i=0;i<10;i++){await Promise.resolve();render();}}
  async function advance(ms){const end=clock+ms;while(true){const next=[...timers].filter(([,t])=>t.at<=end).sort((a,b)=>a[1].at-b[1].at)[0];if(!next)break;const[id,t]=next;timers.delete(id);clock=t.at;t.fn();await pump();}clock=end;await pump();}
  function setup(p) {
    reset(); timers=new Map();clock=0;nextId=0;props=p;
    globalThis.window={setTimeout:(fn,ms=0)=>{const id=++nextId;timers.set(id,{at:clock+ms,fn});return id;},clearTimeout:id=>timers.delete(id),addEventListener:()=>{},removeEventListener:()=>{}};
    globalThis.requestAnimationFrame=fn=>window.setTimeout(fn,16);globalThis.cancelAnimationFrame=id=>window.clearTimeout(id);
    render();return tree;
  }
  const button=label=>nodes(tree).find(n=>n.type==='button' && text(n).includes(label));
  const input=()=>nodes(tree).find(n=>n.props.className==='thread-search');
  const queryCalls=[];
  setup({detail:detail([entry('tail')],page()),liveBlocks:[],running:false,pageRequest:{filter:'default'},
    onQuery:async r=>{queryCalls.push(r);props={...props,pageRequest:r};}});
  input().props.onChange({target:{value:'ancient'}});render();await advance(299);
  assert.equal(queryCalls.length,0,'server search must be debounced');
  await advance(1);assert.deepEqual(queryCalls,[{query:'ancient',filter:'default',branchLeafId:undefined}]);
  assert.match(text(tree),/3700 matches \(full branch\)/);
  assert.match(text(tree),/3800 branch entries \/ 4000 file entries/);
  assert.match(text(tree),/800 tool results/);
  console.log('PASS real Thread 300ms debounced server-wide search and full branch/file/filter counts');

  // External navigation clears stale search/filter without reissuing old input.
  props={...props,pageRequest:{filter:'all',entryId:'old-peer'}};render();await pump();
  assert.equal(queryCalls.length,1);
  assert.equal(input().props.value,'');
  assert.equal(button('All').props.className,'filter-btn active');
  console.log('PASS entry navigation synchronizes search/filter without issuing stale query');

  // Giant bodies are server-matched even when the page preview omits the needle.
  const previewEntry={...entry('giant','toolResult'),toolCallId:'tool-1',toolName:'codemode',bodyRef:{generation:'fixture-g1',entryId:'giant',byteLength:52*1024*1024,recordOffset:123,preview:true}};
  setup({detail:detail([previewEntry],page({matchedEntries:1,toolPairs:[{toolCallId:'tool-1',callEntryId:'ancient-call',resultEntryId:'giant'}]})),
    liveBlocks:[],running:false,pageRequest:{query:'needle-outside-preview',filter:'all'},onLoadEntry:async id=>{queryCalls.push(id);}});
  assert.equal(buildThreadItems(props.detail,'all','needle-outside-preview').items.length,1);
  assert.match(text(tree),/Load related tool call · ancient-call/);
  await button('Load related tool call').props.onClick();assert.equal(queryCalls.at(-1),'ancient-call');
  const html=renderToStaticMarkup(tree);assert.match(html,/Incomplete preview/);assert.match(html,/Load full body/);
  console.log('PASS server giant-body match retained despite preview, truthful full-body control and off-page pairing action');

  // Pure linear pairing guard: prohibit indexOf/forward scans and count reads.
  const many=[];
  for(let i=0;i<2000;i++){
    many.push(entry('a'+i,'assistant',[{kind:'toolCall',id:'tc'+i,name:'codemode',arguments:'{}'}]));
    many.push({...entry('r'+i,'toolResult'),toolCallId:'tc'+i});
  }
  let reads=0;
  for(const e of many){const original=e.content;Object.defineProperty(e,'content',{get(){reads++;return original;}});}
  const big=detail(many);big.active.indexOf=()=>{throw Error('quadratic active.indexOf forbidden');};
  const items=buildThreadItems(big,'all','');
  assert.equal(items.items.length,2000);assert.equal(items.items.at(-1).inlineResults[0].id,'r1999');
  assert.ok(reads<5000,`content reads ${reads} must be linear`);
  const assistantMatches=buildThreadItems(big,'all','a1999');assert.ok(assistantMatches.items.length<=1);
  const userSearch=buildThreadItems(detail([entry('first'),entry('second')]),'user-only','second');
  assert.deepEqual(userSearch.items.map(i=>i.entry.id),['second']);
  console.log('PASS indexed O(n) 4000-entry pairing and user-only search intersection');

  const call=entry('boundary-call','assistant',[{kind:'toolCall',id:'boundary',name:'tool',arguments:'{}'}]);
  const result={...entry('boundary-result','toolResult'),toolCallId:'boundary'};
  const pairs=[{toolCallId:'boundary',callEntryId:call.id,resultEntryId:result.id}];
  const recent=detail([result],page({toolPairs:pairs}));const old=detail([call],page({toolPairs:pairs,hasMore:false,previousCursor:null}));
  const merged=prependDetailPage(recent,old);assert.equal(merged.entries[1],result);
  assert.equal(buildThreadItems(merged,'all','').items[0].inlineResults[0],result);
  assert.throws(()=>prependDetailPage(recent,detail([call],page({generation:'changed'}))),/STALE_DETAIL/);
  const emptyPreview={...call,content:[],bodyRef:{generation:'fixture-g1',entryId:call.id,recordOffset:123,byteLength:1234,preview:true}};
  assert.equal(buildThreadItems(detail([emptyPreview,result],page({toolPairs:pairs})),'all','').items.length,2,
    'result must remain visible when the call preview cannot render its tool block');
  const repeated={...result,id:'boundary-second-output'};
  const repeatedItems=buildThreadItems(detail([call,result,repeated],page({toolPairs:[...pairs,{...pairs[0],resultEntryId:repeated.id}]})),'all','').items;
  assert.equal(repeatedItems.length,2);assert.equal(repeatedItems[1].entry.id,repeated.id);
  console.log('PASS backend-index cross-page pairing, stable old Entry references and stale generation rejection');

  // Actual collapsed ToolRow rendering must never parse the giant argument.
  const giantArg=JSON.stringify({code:'z'.repeat(64*1024)+' code-evidence-tail'});
  setup({detail:detail([entry('giant-call','assistant',[{kind:'toolCall',id:'g',name:'codemode',arguments:giantArg}])]),liveBlocks:[],running:false});
  const oldParse=JSON.parse;let giantParses=0;
  JSON.parse=(value,...rest)=>{if(value===giantArg)giantParses++;return oldParse(value,...rest);};
  let collapsed;
  try{collapsed=renderToStaticMarkup(tree);}finally{JSON.parse=oldParse;}
  assert.equal(giantParses,0);assert.ok(collapsed.length<20000);assert.match(collapsed,/expand for full arguments/);
  const memoEntry=nodes(tree).find(n=>n.props.entry?.id==='giant-call');
  const assistantTree=memoEntry.type.type(memoEntry.props);
  const tool=nodes(assistantTree).find(n=>n.type?.name==='ToolRow');
  reset();begin();let toolTree=tool.type(tool.props);
  nodes(toolTree).find(n=>n.type==='button').props.onClick();begin();toolTree=tool.type(tool.props);
  assert.ok(text(toolTree).includes('code-evidence-tail'),'explicit expansion preserves complete tool arguments');
  console.log('PASS collapsed 64KiB tool args avoid eager JSON.parse and expanded arguments remain lossless');

  // Full record chunks preserve code, original unknown/image fields and UTF-8.
  reset(); const calls=[];const raw=JSON.stringify({type:'message',id:'giant',message:{role:'assistant',content:[{type:'text',text:'full evidence 🦀'},
    {type:'toolCall',id:'tc',name:'codemode',arguments:{code:'return tools.read({path:"full"})'}},{type:'image',mimeType:'image/png',data:'original-image'}]},unknown:'retained'});
  const split=raw.indexOf('🦀');const first=raw.slice(0,split),last=raw.slice(split);const bytes=Buffer.byteLength(first);const total=Buffer.byteLength(raw);
  const chunks=[{generation:'fixture-g1',entryId:'giant',recordOffset:123,offset:0,nextOffset:bytes,totalBytes:total,data:first,encoding:'utf8-jsonl',complete:false},
    {generation:'fixture-g1',entryId:'giant',recordOffset:123,offset:bytes,nextOffset:null,totalBytes:total,data:last,encoding:'utf8-jsonl',complete:true}];
  globalThis.fixture={invoke:async(command,args)=>{calls.push({command,args});if(command==='session_entry_body')return chunks.find(c=>c.offset===args.offset);throw Error(command);}};
  const bodyProps={entry:{...previewEntry,bodyRef:{...previewEntry.bodyRef,byteLength:total}},onBodyChunk:(g,id,offset,recordOffset)=>api.sessionEntryBody('/synthetic/52mb.jsonl',g,id,offset,recordOffset)};
  const bodyRender=()=>{begin();tree=FullBodyView(bodyProps);flushEffects();};bodyRender();
  assert.equal(calls.length,0,'preview cannot eagerly fetch body');
  const bodyLoad=button('Load full body').props.onClick();
  for(let i=0;i<20;i++)await Promise.resolve();await bodyLoad;bodyRender();bodyRender();
  assert.equal(calls.length,2);assert.equal(calls[1].args.offset,bytes);assert.equal(calls[0].args.maxBytes,65536);assert.equal(calls[0].args.recordOffset,123);
  assert.ok(text(tree).includes(raw));assert.match(text(tree),/Full original record/);assert.doesNotMatch(text(tree),/Incomplete preview/);
  const download=nodes(tree).find(n=>n.type==='a');assert.ok(download.props.href.startsWith('blob:'));
  const downloaded=await(await fetch(download.props.href)).text();assert.equal(downloaded,raw);
  console.log('PASS explicit chunked lossless full JSONL body, code/UTF-8/images/unknown fields and download');

  // Opt-in 52MiB stress record; routine checks use 1MiB under memory pressure.
  // PI_VIEWER_FULL_BODY_STRESS=1 node scripts/test-bounded-detail-thread.mjs
  // A synthetic backend record behind fake IPC: initial page is
  // bounded, rare suffix search is server-side, full body only crosses IPC
  // after explicit loading. This measures the frontend contract, not Rust.
  const fixtureBodyMiB=process.env.PI_VIEWER_FULL_BODY_STRESS==='1'?52:1;
  const fixture52=JSON.stringify({type:'message',id:'giant',message:{role:'toolResult',toolCallId:'tool-1',toolName:'codemode',content:[{type:'text',
    text:'x'.repeat(fixtureBodyMiB*1024*1024)+' rare-52mb-code-evidence-tail'}]},unknown:'retained'});
  const fixture52Bytes=Buffer.byteLength(fixture52);
  const giant52={...previewEntry,content:[{kind:'text',text:'x'.repeat(512)}],bodyRef:{...previewEntry.bodyRef,byteLength:fixture52Bytes}};
  const initial52=detail([giant52],page({matchedEntries:1,returnedEntries:1}));
  initial52.page.payloadBytes=Buffer.byteLength(JSON.stringify(initial52));
  let body52Calls=0, page52Calls=0;
  globalThis.fixture={invoke:async(command,args)=>{
    if(command==='session_detail_page'){
      page52Calls++;
      if(args.request?.query) assert.ok(fixture52.includes(args.request.query));
      // Serialize over the fake IPC boundary; the frontend receives no body.
      return JSON.parse(JSON.stringify(initial52));
    }
    if(command==='session_entry_body'){
      body52Calls++;
      const end=Math.min(args.offset+args.maxBytes,fixture52Bytes);
      return {generation:'fixture-g1',entryId:'giant',recordOffset:123,offset:args.offset,nextOffset:end<fixture52Bytes?end:null,
        totalBytes:fixture52Bytes,data:fixture52.slice(args.offset,end),encoding:'utf8-jsonl',complete:end===fixture52Bytes};
    }
    throw Error(command);
  }};
  const initialPage=await api.sessionDetailPage('/synthetic/52mb.jsonl',{filter:'default'});
  assert.ok(fixture52Bytes>=fixtureBodyMiB*1024*1024);assert.ok(Buffer.byteLength(JSON.stringify(initialPage))<262144);
  assert.equal(body52Calls,0);assert.ok(!JSON.stringify(initialPage).includes('rare-52mb-code-evidence-tail'));
  const searchedPage=await api.sessionDetailPage('/synthetic/52mb.jsonl',{query:'rare-52mb-code-evidence-tail',filter:'all'});
  assert.equal(buildThreadItems(searchedPage,'all','rare-52mb-code-evidence-tail').items.length,1);
  assert.equal(body52Calls,0);
  reset();const giant52Props={entry:searchedPage.entries[0],onBodyChunk:bodyProps.onBodyChunk};
  begin();tree=FullBodyView(giant52Props);flushEffects();button('Load full body').props.onClick();
  for(let i=0;i<2000;i++)await Promise.resolve();begin();tree=FullBodyView(giant52Props);flushEffects();
  const full52=nodes(tree).find(n=>n.type==='pre').props.children;
  assert.equal(full52,fixture52);assert.ok(full52.endsWith('"retained"}'));
  assert.equal(body52Calls,Math.ceil(fixture52Bytes/65536));assert.equal(page52Calls,2);
  console.log(`PASS actual ${fixture52Bytes} byte synthetic fake-IPC fixture, initial ${initialPage.page.payloadBytes} byte payload, suffix search and explicit full body (${body52Calls} chunks)`);
  reset();

  // Invalid/incomplete body never masquerades as complete.
  reset();globalThis.fixture={invoke:async()=>({...chunks[0],nextOffset:null,complete:false})};bodyRender();
  button('Load full body').props.onClick();for(let i=0;i<20;i++)await Promise.resolve();bodyRender();
  assert.match(text(tree),/no progress/);assert.match(text(tree),/Incomplete preview/);assert.doesNotMatch(text(tree),/Full original record/);
  console.log('PASS incomplete body response retains truthful preview/error, no fake completion');

  reset();globalThis.fixture={invoke:async()=>({...chunks[1],offset:0,data:'{}'})};bodyRender();
  button('Load full body').props.onClick();for(let i=0;i<20;i++)await Promise.resolve();bodyRender();
  assert.match(text(tree),/Invalid final body chunk/);assert.match(text(tree),/Incomplete preview/);
  console.log('PASS valid JSON with incomplete original byte length is not accepted as complete');

  reset();let resolveBody;let cancelledCalls=0;
  const blockedBody=new Promise(resolve=>{resolveBody=resolve;});
  globalThis.fixture={invoke:()=>{cancelledCalls++;return blockedBody;}};bodyRender();
  button('Load full body').props.onClick();bodyRender();button('Cancel body download').props.onClick();bodyRender();
  resolveBody(chunks[0]);for(let i=0;i<20;i++)await Promise.resolve();bodyRender();
  assert.equal(cancelledCalls,1);assert.match(text(tree),/Incomplete preview/);
  console.log('PASS explicit body cancellation does not download remaining giant chunks');

  const metadata=[{...entry('compact'),kind:'compaction',summary:'preserved compaction code evidence',name:'123'},
    {...entry('branch'),kind:'branch_summary',summary:'branch evidence'},
    {...entry('label'),kind:'label',label:'important',name:'off-page-labeled'},
    {...entry('model'),kind:'model_change',model:'preserved-model'}];
  setup({detail:detail(metadata,page()),liveBlocks:[],running:false,pageRequest:{filter:'all'}});
  const metaHTML=renderToStaticMarkup(tree);
  assert.match(metaHTML,/preserved compaction code evidence/);assert.match(metaHTML,/branch evidence/);
  assert.match(metaHTML,/important/);assert.match(metaHTML,/preserved-model/);assert.match(metaHTML,/thinking: high/);
  console.log('PASS compaction/branch/labels/model/context metadata preserved in bounded all mode');

  // Full export delegates to old backend export command, never page HTML.
  const exportCalls=[];globalThis.fixture={invoke:async(command,args)=>{exportCalls.push({command,args});return '/synthetic/full-export.html';}};
  setup({detail:detail([entry('one-page')],page()),liveBlocks:[],running:false});
  button('Export HTML').props.onClick();await pump();
  assert.deepEqual(exportCalls,[{command:'export_session_html',args:{sessionPath:'/synthetic/52mb.jsonl'}}]);
  assert.match(text(tree),/full-export.html/);
  button('Attach').props.onClick();await pump();button('Open TUI').props.onClick();await pump();
  assert.deepEqual(exportCalls.slice(1).map(c=>c.command),['attach_session','open_in_terminal']);
  console.log('PASS full-backend export and attach/Open TUI actions retained');

  const source=await readFile(path.join(root,'src/components/Thread.tsx'),'utf8');
  assert.match(source,/key=\{entry\.id\}/);assert.doesNotMatch(source,/key=\{entry\.id \+ idx\}/);
  console.log('PASS stable Entry ID DOM keys (prepend does not remount old Markdown)');
  console.log('All bounded Thread tests passed (offline synthetic fake IPC, not native E2E).');
} finally {await rm(scratch,{recursive:true,force:true});}
