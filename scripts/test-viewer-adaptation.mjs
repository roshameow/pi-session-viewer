// Offline mock-only regression runner. No desktop, Pi, MCP or rmux is started.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import path from 'node:path';

const root = path.resolve(new URL('..', import.meta.url).pathname);
// Bundle real React components. External React uses the project's node_modules;
// the scratch module itself is removed even when assertions fail.
const scratch = await mkdtemp(path.join(root, 'node_modules/.viewer-adaptation-'));
try {
  const outfile = path.join(scratch, 'components.mjs');
  await build({ stdin: {
    contents: `export * from './src/components/Thread'; export * from './src/components/ConfigPanel'; export * from './src/components/Sidebar';`,
    resolveDir: root, loader: 'tsx',
  }, bundle: true, platform: 'node', format: 'esm', outfile,
    external: ['react', 'react-dom', 'react-dom/server'], logLevel: 'silent' });
  const { buildLiveBlocks, appendLiveEvents, McpCard, Thread, Sidebar } = await import(pathToFileURL(outfile));
  const React = await import('react');
  const { renderToStaticMarkup } = await import('react-dom/server');
  const render = (component, props) => renderToStaticMarkup(React.createElement(component, props));
  const events = [
    {type:'tool_execution_start', toolCallId:'parent', toolName:'codemode', args:{code:'return tools.mcp__demo__read({})'}},
    {type:'tool_execution_start', toolCallId:'parent/1', parentToolCallId:'parent', toolName:'mcp__demo__read', args:{}},
    {type:'tool_execution_start', toolCallId:'parent/2', parentToolCallId:'parent', toolName:'mcp__demo__write', args:{}},
    {type:'tool_execution_update', toolCallId:'parent/1', partialResult:'partial read'},
    {type:'tool_execution_end', toolCallId:'parent/2', result:'write error', isError:true},
    {type:'tool_execution_end', toolCallId:'parent/1', result:'read ok'},
    {type:'tool_execution_update', toolCallId:'parent', partialResult:'parent progress'},
    {type:'tool_execution_end', toolCallId:'parent', result:'script complete'},
  ];
  const blocks = buildLiveBlocks(events);
  assert.deepEqual(blocks.map(b=>[b.toolCallId,b.result,b.isError,b.done]), [
    ['parent','script complete',false,true], ['parent/1','read ok',false,true], ['parent/2','write error',true,true],
  ]);
  assert.deepEqual(appendLiveEvents(buildLiveBlocks(events.slice(0,4)),events.slice(4)),blocks);
  assert.equal(buildLiveBlocks([{type:'tool_execution_start',toolName:'mcp',args:{}},
    {type:'tool_execution_end',result:'legacy ok'}])[0].result,'legacy ok');
  console.log('PASS nested/parallel native MCP codemode events + adapter legacy stream');

  const shared = {name:'same',command:'mock',args:[],env:[],source:'global',socket:null,url:null,enabled:null};
  const native = render(McpCard,{s:{...shared, enabled:false, exposure:'deferred',toolExposure:{'get_*':'direct'},configPath:'/project/.pi/mcp.json'}});
  assert.match(native,/native disabled/); assert.match(native,/native exposure: deferred/);
  assert.match(native,/toolExposure: get_\* → direct/); assert.match(native,/\.pi\/mcp\.json/);
  const adapter = render(McpCard,{s:{...shared,dialect:'adapter',disabled:true,directTools:['read'],socket:'/mock.sock'}});
  assert.match(adapter,/adapter disabled/); assert.match(adapter,/adapter directTools:/); assert.match(adapter,/socket:/);
  assert.doesNotMatch(adapter,/native disabled/);
  assert.match(render(McpCard,{s:shared}),/codemode \(default\)/);
  console.log('PASS native exposure/enable vs adapter disabled/socket/directTools rendering');

  // Thread's generic rows intentionally preserve native and adapter names.
  const detail = {id:'mock',cwd:'/mock',path:'/mock/session.jsonl',taskId:null,entries:[],active:[],
    stats:{provider:'mock',model:'mock',messageCount:0,tokenCount:0,costTotal:0,thinkingLevel:null,contextTokens:null,contextLimit:null}};
  const html = render(Thread,{detail,liveBlocks:blocks,running:false,preview:true});
  assert.match(html,/codemode/); assert.match(html,/mcp__demo__read/); assert.match(html,/mcp__demo__write/);
  console.log('PASS native MCP full names and codemode rows');

  const session = {id:'mock',path:'/mock/session.jsonl',cwd:'/mock',name:'Mock',firstMessage:'Mock',updatedAt:0,
    isSubagent:true,taskId:'mock-task',parentSessionId:null,running:false,sleeping:false,interrupted:false,
    inRmux:true,rmuxTarget:'pi-agents:mock.0',rmuxAttached:false,rmuxDead:false,termAlive:false};
  globalThis.localStorage = {getItem:()=>null, setItem:()=>{}};
  const sidebar = (rmuxPiAlive) => render(Sidebar,{projects:[{key:'mock',cwd:'/mock',sessionCount:1,subagentCount:1,
    updatedAt:0,runningCount:0,rmuxCount:0,termCount:0}], sessions:[{...session,rmuxPiAlive}], selectedProject:'mock',
    selectedSessionPath:session.path, finishedAt:{}, remoteHosts:[], onSelectProject:()=>{}, onSelectSession:()=>{}});
  assert.match(sidebar(false),/ended rmux/); assert.doesNotMatch(sidebar(false),/running in background/);
  assert.match(sidebar(null),/\? rmux/); assert.match(sidebar(null),/identity unknown/);
  console.log('PASS ended vs unknown historical rmux location chips');
} finally { await rm(scratch,{recursive:true,force:true}); }
