// Offline behavioral tests of the real App and its real request coordinator.
// Deterministic React hooks, IPC, frames and timers; no desktop/SSH/dependencies added.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
const root = fileURLToPath(new URL('..', import.meta.url));
const require = createRequire(import.meta.url);
const scratch = await mkdtemp(path.join(root, 'node_modules/.app-sequencing-'));
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
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const project = key => ({ key, cwd: `/${key}`, sessionCount: 2, subagentCount: 0, runningCount: 0, rmuxCount: 0, termCount: 0, updatedAt: 1 });
const session = path => ({ path, id: path, cwd: '/fixture', name: path, firstMessage: '', lastMessage: '',
  createdIso: '', createdAt: 0, updatedAt: 1, model: null, isSubagent: false, taskId: null,
  parentSessionId: null, parentSessionPath: null, messageCount: 0, running: false, sleeping: false,
  interrupted: false, inRmux: false, rmuxTarget: null, rmuxAttached: false, rmuxDead: false, termAlive: false, size: 1 });
const detail = path => ({ path, id: path, cwd: '/fixture', entries: [], active: [], stats: {}, size: 1, updatedAt: 1 });
const status = (host, usableCache = true) => ({ host, phase: usableCache ? 'ready' : 'idle', lastSuccessAt: usableCache ? 1 : null, error: null, usableCache });
try {
  const outfile = path.join(scratch, 'fixture.mjs');
  await build({ stdin: { contents: `export { default as App } from './src/App'; export { RequestSequencer } from './src/requestSequencing'; export { begin, reset, flushEffects } from 'react';`, resolveDir: root, loader: 'tsx' },
    bundle: true, platform: 'node', format: 'esm', outfile, external: [require.resolve('react'), 'react/jsx-runtime'], logLevel: 'silent',
    plugins: [{ name: 'offline-app', setup(b) {
      b.onResolve({ filter: /^react$/ }, () => ({ path: 'react', namespace: 'fixture' }));
      b.onResolve({ filter: /^@tauri-apps\/api\// }, args => ({ path: args.path, namespace: 'fixture' }));
      b.onResolve({ filter: /^\.\/api$/ }, () => ({ path: 'api', namespace: 'fixture' }));
      b.onResolve({ filter: /^\.\/components\// }, args => ({ path: args.path, namespace: 'fixture' }));
      b.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path: p }) => {
        if (p === 'react') return { contents: hooks };
        if (p === 'api') return { contents: 'export const api = new Proxy({}, { get: (_, key) => (...args) => globalThis.fixture.api[key](...args) });' };
        if (p.endsWith('/core')) return { contents: 'export class Channel {}' };
        if (p.endsWith('/window')) return { contents: 'export const getCurrentWindow = () => ({ onFocusChanged: fn => { globalThis.fixture.focus = fn; return Promise.resolve(() => {}); } });' };
        const name = p.split('/').pop();
        return { contents: `export function ${name}() { return null; } ${name === 'Thread' ? 'export function appendLiveEvents(prev, batch) { return [...prev, ...batch]; }' : ''}` };
      });
    } }],
  });
  const { App, RequestSequencer, begin, reset, flushEffects } = await import(pathToFileURL(outfile));
  let timers, clock, timerId, tree;
  function nodes(node, out = []) {
    if (Array.isArray(node)) node.forEach(n => nodes(n, out));
    else if (node && typeof node === 'object' && node.props) { out.push(node); nodes(node.props.children, out); }
    return out;
  }
  const props = name => nodes(tree).find(n => n.type?.name === name)?.props;
  const text = node => Array.isArray(node) ? node.map(text).join('') : node && typeof node === 'object' && node.props ? text(node.props.children) : node == null || typeof node === 'boolean' ? '' : String(node);
  const render = () => { begin(); tree = App(); flushEffects(); };
  async function pump() { for (let i = 0; i < 30; i++) { await Promise.resolve(); render(); } }
  async function advance(ms) {
    const end = clock + ms;
    while (true) {
      const next = [...timers].filter(([, t]) => t.at <= end).sort((a,b) => a[1].at - b[1].at)[0];
      if (!next) break;
      const [id, t] = next; timers.delete(id); clock = t.at; t.fn(); await pump();
    }
    clock = end; await pump();
  }
  async function fixture(overrides = {}) {
    reset(); timers = new Map(); clock = 0; timerId = 0;
    const later = (fn, ms = 0) => { const id = ++timerId; timers.set(id, { at: clock + ms, fn }); return id; };
    globalThis.window = { setTimeout: later, clearTimeout: id => timers.delete(id) };
    globalThis.requestAnimationFrame = fn => later(fn, 16);
    globalThis.cancelAnimationFrame = id => timers.delete(id);
    globalThis.document = { hidden: false };
    globalThis.prompt = () => null;
    globalThis.alert = () => {};
    const f = { selected: null, calls: [], api: {}, focus: null };
    const defaults = {
      getRemoteHost: async () => f.selected, listRemoteHosts: async () => ['A', 'B', 'cold'],
      piBinPath: async () => '/fixture/pi', piVersion: async () => 'test',
      listProjects: async () => [project(`${f.selected ?? 'local'}-p`)],
      listSessions: async key => [session(`/${key}/1`), session(`/${key}/2`)],
      sessionDetail: async p => detail(p), listRunning: async () => [], sessionStatus: async () => 'finished',
      remoteSyncStatus: async h => status(h, h !== 'cold'),
      setRemoteHost: async h => { f.selected = h; }, refreshRemote: async () => {},
      openInTerminal: async () => {}, detachFromRmux: async () => {}, killRmuxSession: async () => {},
      deleteSession: async () => {}, abortMessage: async () => {}, sendMessage: async () => {},
    };
    for (const [name, fn] of Object.entries({ ...defaults, ...overrides })) {
      f.api[name] = (...args) => { f.calls.push({ name, args, source: f.selected }); return fn(...args, f); };
    }
    globalThis.fixture = f;
    render(); await pump(); return f;
  }
  const calls = (f, name) => f.calls.filter(c => c.name === name);

  // Real App: cached source and sessions render before even starting full sync.
  const blockedSync = deferred();
  let f = await fixture({ refreshRemote: () => blockedSync.promise });
  const switching = props('Sidebar').onSwitchSource('A'); await pump(); await switching; await pump();
  assert.equal(props('Sidebar').remoteHost, 'A');
  assert.equal(props('Sidebar').selectedProject, 'A-p');
  assert.equal(props('Sidebar').sessions[0].path, '/A-p/1');
  assert.equal(props('Thread').detail.path, '/A-p/1');
  assert.equal(calls(f, 'refreshRemote').length, 0, 'Full sync must wait for paint');
  await advance(16); assert.equal(calls(f, 'refreshRemote').length, 0);
  await advance(16); assert.equal(calls(f, 'refreshRemote').length, 1);
  assert.deepEqual(calls(f, 'refreshRemote')[0].args, ['A'], 'Refresh captures explicit host');
  assert.equal(props('Sidebar').syncing, false);
  assert.equal(props('Sidebar').loadingSessions, false);
  assert.match(text(tree), /Last full sync:.*old.*refreshing in background/);
  assert.match(text(tree), /not live telemetry/);
  props('Sidebar').onRefresh(); await pump();
  assert.equal(calls(f, 'refreshRemote').length, 1, 'Equivalent full refreshes coalesce');
  await props('Sidebar').onSwitchSource('B'); await pump();
  blockedSync.reject(new Error('old A sync error')); await pump();
  assert.equal(props('Sidebar').remoteHost, 'B');
  assert.doesNotMatch(text(tree), /old A sync error/);
  console.log('PASS real App cached source/projects/list/detail before blocked background sync; paint deferral, visible cache status, explicit host, coalescing and stale refresh error');

  // Phase/error UI remains visible without hiding usable cached content.
  const refreshFailure = deferred();
  let phase = 'ready';
  f = await fixture({
    refreshRemote: () => { phase = 'syncing-history'; return refreshFailure.promise; },
    remoteSyncStatus: async h => ({ ...status(h), phase }),
  });
  await props('Sidebar').onSwitchSource('A'); await pump(); await advance(32);
  assert.match(text(tree), /syncing-history/);
  phase = 'capturing-snapshots'; await advance(1000);
  assert.match(text(tree), /capturing-snapshots/);
  refreshFailure.reject(new Error('sanitized refresh failure')); await pump();
  assert.match(text(tree), /error.*sanitized refresh failure/);
  assert.equal(props('Sidebar').syncing, false);
  assert.equal(props('Sidebar').loadingSessions, false);
  assert.equal(props('Sidebar').sessions[0].path, '/A-p/1');
  assert.equal(props('Thread').detail.path, '/A-p/1');
  console.log('PASS real App phase polling and visible refresh failure preserve cached browsing');

  // No cache: not-ready while backend initial sync blocks, failure restores source.
  const initial = deferred();
  f = await fixture({ setRemoteHost: (h, f) => h === 'cold' ? initial.promise : Promise.resolve().then(() => { f.selected = h; }) });
  const cold = props('Sidebar').onSwitchSource('cold'); await pump();
  assert.equal(props('Sidebar').remoteHost, null);
  assert.match(text(tree), /Initial sync.*source not ready/);
  assert.equal(props('Sidebar').sessions.length, 0);
  assert.equal(props('Thread'), undefined);
  initial.reject(new Error('sanitized initial failure')); await cold; await pump();
  assert.equal(props('Sidebar').remoteHost, null);
  assert.equal(props('Sidebar').selectedProject, 'local-p');
  assert.match(text(tree), /Initial selection\/sync failed: .*sanitized initial failure.*Not selected/);
  assert.equal(calls(f, 'refreshRemote').length, 0);
  console.log('PASS real App no-cache initial-sync not-ready and safe failure with retry notice');

  // Successful no-cache selection completes its one initial sync; no immediate
  // redundant background full sync is scheduled afterward.
  const initialSuccess = deferred();
  f = await fixture({
    setRemoteHost: async (h, f) => { await initialSuccess.promise; f.selected = h; },
    remoteSyncStatus: async (h, f) => status(h, f.selected === h),
  });
  const coldSuccess = props('Sidebar').onSwitchSource('cold'); await pump();
  assert.match(text(tree), /Initial sync.*source not ready/);
  initialSuccess.resolve(); await coldSuccess; await pump(); await advance(32);
  assert.equal(props('Sidebar').remoteHost, 'cold');
  assert.equal(props('Sidebar').sessions[0].path, '/cold-p/1');
  assert.equal(calls(f, 'refreshRemote').length, 0);
  console.log('PASS real App successful no-cache initial sync without redundant refresh');

  // Source A -> B -> A: a list captured by the first A remains obsolete even
  // though the selected host name is A again when that list finally resolves.
  const firstSourceA = deferred(); let sourceALists = 0;
  f = await fixture({ listProjects: async (...args) => {
    const f = args.at(-1);
    if (f.selected === 'A') {
      if (++sourceALists === 1) return firstSourceA.promise;
      return [project('new-source-A')];
    }
    return [project(`${f.selected ?? 'local'}-p`)];
  } });
  const oldSourceA = props('Sidebar').onSwitchSource('A'); await pump();
  await props('Sidebar').onSwitchSource('B'); await pump();
  await props('Sidebar').onSwitchSource('A'); await pump();
  assert.equal(sourceALists, 2);
  assert.equal(props('Sidebar').selectedProject, 'new-source-A');
  firstSourceA.resolve([project('old-source-A')]); await oldSourceA; await pump();
  assert.equal(props('Sidebar').selectedProject, 'new-source-A');
  assert.equal(props('Thread').detail.path, '/new-source-A/1');
  console.log('PASS real App A-B-A source epochs discard first-source project response');

  // Two equivalent list reloads share one IPC while pending.
  const equivalentList = deferred(); let reloads = 0;
  f = await fixture({ listSessions: async key => ++reloads === 1 ? [session(`/${key}/1`)] : equivalentList.promise });
  props('Sidebar').onRefresh(); props('Sidebar').onRefresh(); await pump();
  assert.equal(reloads, 2, 'One initial list and one coalesced equivalent reload');
  equivalentList.resolve([session('/coalesced-list')]); await pump();
  assert.equal(props('Sidebar').sessions[0].path, '/coalesced-list');
  console.log('PASS real App equivalent list requests coalesce');

  // The fake backend models the production selection epoch. A slow initial
  // sync may finish later, but must not delay/overwrite a cached or Local intent.
  const commitCold = deferred(); let backendEpoch = 0;
  f = await fixture({ setRemoteHost: async (h, f) => {
    const epoch = ++backendEpoch;
    if (h === 'cold') await commitCold.promise;
    if (epoch !== backendEpoch) throw new Error('Source selection superseded');
    f.selected = h;
  } });
  const slowCold = props('Sidebar').onSwitchSource('cold'); await pump();
  assert.match(text(tree), /Initial sync/);
  assert.equal(props('Sidebar').syncing, false, 'New source intents remain available');
  await props('Sidebar').onSwitchSource('B'); await pump();
  assert.equal(f.selected, 'B'); assert.equal(props('Sidebar').remoteHost, 'B');
  assert.equal(props('Sidebar').sessions[0].path, '/B-p/1');
  await props('Sidebar').onSwitchSource(null); await pump();
  assert.equal(f.selected, null);
  commitCold.resolve(); await slowCold; await pump();
  assert.equal(f.selected, null); assert.equal(props('Sidebar').remoteHost, null);
  assert.doesNotMatch(text(tree), /Source selection superseded/);
  console.log('PASS real App blocked cold selection does not delay cached B/Local; obsolete cold commit/error discarded');

  // A successful-but-superseded selection can change backend source before its
  // promise returns. A subsequent cold failure restores actual backend identity.
  const staleSelection = deferred(); const failedSelection = deferred();
  f = await fixture({ setRemoteHost: async (h, f) => {
    if (h === 'A') { f.selected = h; await staleSelection.promise; }
    else if (h === 'cold') await failedSelection.promise;
    else f.selected = h;
  } });
  const staleA = props('Sidebar').onSwitchSource('A'); await pump();
  const failedCold = props('Sidebar').onSwitchSource('cold'); await pump();
  failedSelection.reject(new Error('sanitized cold failure')); await failedCold; await pump();
  assert.equal(props('Sidebar').remoteHost, 'A');
  assert.equal(props('Sidebar').sessions[0].path, '/A-p/1');
  staleSelection.resolve(); await staleA; await pump();
  assert.equal(props('Sidebar').remoteHost, 'A');
  console.log('PASS real App failure recovery queries actual backend source after superseded selection');

  // Real App list A -> B -> A: first A's late list must not overwrite newer A.
  const listA = deferred(); let aLists = 0;
  f = await fixture({ listSessions: async key => {
    if (key === 'project-A') { aLists++; if (aLists === 1) return listA.promise; return [session('/new-A')]; }
    return [session(`/${key}`)];
  } });
  props('Sidebar').onSelectProject(project('project-A')); await pump();
  props('Sidebar').onSelectProject(project('project-B')); await pump();
  props('Sidebar').onSelectProject(project('project-A')); await pump();
  assert.equal(aLists, 2);
  assert.equal(props('Sidebar').sessions[0].path, '/new-A');
  listA.resolve([session('/old-A')]); await pump();
  assert.equal(props('Sidebar').sessions[0].path, '/new-A');
  console.log('PASS real App A-B-A project list epochs');

  // Detail A -> B -> A and duplicate detail request coalescing.
  const detailA = deferred(); let aDetails = 0;
  f = await fixture({ sessionDetail: async p => {
    if (p === '/A') { aDetails++; if (aDetails === 1) return detailA.promise; return { ...detail(p), updatedAt: 99 }; }
    return detail(p);
  } });
  props('Sidebar').onSelectSession(session('/A')); props('Sidebar').onSelectSession(session('/A')); await pump();
  assert.equal(aDetails, 1);
  props('Sidebar').onSelectSession(session('/B')); await pump();
  props('Sidebar').onSelectSession(session('/A')); await pump();
  assert.equal(aDetails, 2); assert.equal(props('Thread').detail.updatedAt, 99);
  detailA.resolve({ ...detail('/A'), updatedAt: 2 }); await pump();
  assert.equal(props('Thread').detail.updatedAt, 99);
  console.log('PASS real App equivalent detail coalescing and A-B-A detail epochs');

  // Old source action responses cannot clear or error the selected new source.
  const deleting = deferred();
  f = await fixture({ deleteSession: () => deleting.promise });
  const deletingTask = props('Sidebar').onDeleteSession('/local-p/1'); await pump();
  await props('Sidebar').onSwitchSource('B'); await pump();
  deleting.reject(new Error('old delete error')); await deletingTask; await pump();
  assert.equal(props('Thread').detail.path, '/B-p/1'); assert.doesNotMatch(text(tree), /old delete error/);
  console.log('PASS real App stale action response isolation');
  const oldPath = '/local-p/1';
  const countBefore = calls(f, 'deleteSession').length;
  await props('Sidebar').onDeleteSession(oldPath); await pump();
  props('Sidebar').onOpenTerminal(oldPath); await pump();
  assert.equal(calls(f, 'deleteSession').length, countBefore);
  assert.equal(calls(f, 'openInTerminal').length, 0);
  console.log('PASS real App rejects actions from obsolete source rows');

  f = await fixture();
  props('Composer').onChange('unsent local draft'); await pump();
  await props('Sidebar').onSwitchSource('B'); await pump();
  await props('Sidebar').onSwitchSource(null); await pump();
  assert.equal(props('Composer').value, 'unsent local draft');
  console.log('PASS real App preserves source-qualified drafts while switching sources');


  // Poll cancellation after listRunning/status awaits: no old-source toasts.
  const oldRunning = deferred(); let polls = 0;
  f = await fixture({ listRunning: async () => { polls++; return polls === 1 ? [{ path: '/finished', title: 'old-source-toast', isSubagent: false, projectKey: 'local-p' }] : oldRunning.promise; } });
  await advance(10000); assert.equal(polls, 1);
  await advance(10000); assert.equal(polls, 2);
  await props('Sidebar').onSwitchSource('B'); await pump();
  oldRunning.resolve([]); await pump(); assert.doesNotMatch(text(tree), /old-source-toast/);
  console.log('PASS real App poll cancellation prevents stale completion toasts');

  const oldStatus = deferred(); let runningPolls = 0;
  f = await fixture({
    listRunning: async () => ++runningPolls === 1 ? [{ path: '/finished', title: 'old-status-toast', isSubagent: false, projectKey: 'local-p' }] : [],
    sessionStatus: () => oldStatus.promise,
  });
  await advance(10000); await advance(10000);
  assert.equal(calls(f, 'sessionStatus').length, 1);
  await props('Sidebar').onSwitchSource('B'); await pump();
  oldStatus.resolve('interrupted'); await pump();
  assert.doesNotMatch(text(tree), /old-status-toast/);
  assert.deepEqual(props('Sidebar').finishedAt, {});
  console.log('PASS real App poll cancellation after awaited session status');

  // Restart polling after a failed source transition even when host/project/path
  // names all return to the same values (including an empty local source).
  const emptyPoll = deferred(); const failedEmptySwitch = deferred(); let emptyPolls = 0;
  f = await fixture({ listProjects: async () => [],
    listRunning: () => ++emptyPolls === 1 ? emptyPoll.promise : Promise.resolve([]),
    setRemoteHost: () => failedEmptySwitch.promise,
  });
  await advance(10000); assert.equal(emptyPolls, 1);
  const failedEmpty = props('Sidebar').onSwitchSource('cold'); await pump();
  failedEmptySwitch.reject(new Error('initial failed')); await failedEmpty; await pump();
  emptyPoll.resolve([]); await pump(); await advance(10000);
  assert.equal(emptyPolls, 2);
  console.log('PASS real App polling restarts after failed empty-source transition');

  // Pure coordinator assertions complement (not replace) App behavior above.
  reset();
  const q = new RequestSequencer();
  const firstA = q.beginSource('A'); q.adoptSource(firstA, 'A');
  const pendingList = deferred(); let requests = 0;
  q.selectProject('p');
  const pt = q.capture('project');
  const p1 = q.request(pt, 'list', () => { requests++; return pendingList.promise; });
  const p2 = q.request(pt, 'list', () => { requests++; return pendingList.promise; });
  assert.equal(p1, p2); await Promise.resolve(); assert.equal(requests, 1);
  q.beginSource('B'); const newestA = q.beginSource('A'); q.adoptSource(newestA, 'A');
  assert.equal(q.current(firstA), false); assert.equal(q.current(pt), false);
  pendingList.resolve([]); await p1;
  console.log('PASS real coordinator list coalescing and source A-B-A epoch identity');
  console.log('All frontend sequencing tests passed (offline fake backend; no SSH/app restart).');
} finally {
  await rm(scratch, { recursive: true, force: true });
}
