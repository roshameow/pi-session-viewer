// Offline behavioral regression of REAL ConfigPanel/SkillCard, parser, API and App.
// Run: node scripts/test-config-skills.mjs (existing esbuild/React dependencies).
// No private config reads, network, browser/native opener or filesystem broker.
// Backend allowlist validation is deliberately NOT claimed by mocked IPC tests.
// Method: bundle TSX, retain keyed hook cells, walk actual react-markdown output,
// and explicitly settle deferred requests across Back/unmount/source A->B->A.
// Reproducible harness pitfall: one shared hook cursor cannot test child cards or
// source-key remounts. Component type + React key must own separate hook cells.
// Verified safety pitfall: shared session Markdown renders remote img elements;
// skill previews must opt out. Mocking fetch alone cannot catch browser image
// loading, so this regression asserts the actual parsed host tree has no img.
// Verified guard method: invoke the real section Escape handler with explicit
// defaultPrevented, then settle its abandoned body request. This checks handler
// behavior/late-write suppression, NOT browser keyboard dispatch or focus.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import React from 'react';
const root = fileURLToPath(new URL('..', import.meta.url));
const require = createRequire(import.meta.url);
const scratch = await mkdtemp(path.join(root, 'node_modules/.config-skills-test-'));
const hooks = `
import React from ${JSON.stringify(require.resolve('react'))};
export const records = new Map();
let active, cursor = 0, contexts, effects = [];
export function begin(key, values) {
  if (!records.has(key)) records.set(key, { cells: [], dirty: false, mounted: true, consumed: new Map() });
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
    if (!r.mounted) { globalThis.fixture.staleWrites++; return; }
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
export default { ...React, useContext, useState, useRef, useMemo, useCallback, useEffect };
`;
const text = n => Array.isArray(n) ? n.map(text).join('') : n?.props ? text(n.props.children) : n == null || typeof n === 'boolean' ? '' : String(n);
const nodes = (n, out = []) => { if (Array.isArray(n)) n.forEach(x => nodes(x, out)); else if (n?.props) { out.push(n); nodes(n.props.children, out); } return out; };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const skill = (file, name = 'same-name', source = '/synthetic/project/.pi/skills') => ({ name, description: `Inventory description for ${file}`, source, file });
const A = skill('/synthetic/skills/α/SKILL.md');
const B = skill('/synthetic/skills/β/SKILL.md');
const inventory = (skills = [A, B]) => ({ mcp: [], agents: [], skills });
const documentFor = (s, content = `# Body for ${s.file}\n\nUTF-8 中文 🦉 café`) => ({ file: s.file, content });
try {
  const outfile = path.join(scratch, 'fixture.mjs');
  await build({ stdin: { contents: `export { ConfigPanel } from './src/components/ConfigPanel'; export { default as App } from './src/App'; export { begin, reset, records, flushEffects } from 'react';`, resolveDir: root, loader: 'tsx' },
    bundle: true, platform: 'node', format: 'esm', outfile,
    external: [require.resolve('react'), 'react/jsx-runtime'], logLevel: 'silent',
    plugins: [{ name: 'offline-config-skills', setup(b) {
      b.onResolve({ filter: /^react$/ }, () => ({ path: 'react', namespace: 'fixture' }));
      b.onResolve({ filter: /^@tauri-apps\/api\// }, args => ({ path: args.path, namespace: 'fixture' }));
      // ConfigPanel, its child cards, Markdown/parser and api.ts stay REAL.
      b.onResolve({ filter: /^\.\/components\/(Sidebar|Thread|Composer|DataAccessPanel)$/ }, args => ({ path: args.path, namespace: 'fixture' }));
      b.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path: p }) => {
        if (p === 'react') return { contents: hooks };
        if (p.endsWith('/core')) return { contents: 'export class Channel {} export const invoke = (name, args) => globalThis.fixture.invoke(name, args);' };
        if (p.endsWith('/window')) return { contents: 'export const getCurrentWindow = () => ({ onFocusChanged: () => Promise.resolve(() => {}) });' };
        const name = p.split('/').pop();
        assert.ok(['Sidebar', 'Thread', 'Composer', 'DataAccessPanel'].includes(name), `unexpected native module ${p}`);
        return { contents: `export function ${name}(props) { globalThis.fixture.${name} = props; return null; } ${name === 'Thread' ? 'export const appendLiveEvents = (prev, batch) => [...prev, ...batch];' : ''}` };
      });
    } }],
  });
  const { ConfigPanel, App, begin, reset, records, flushEffects } = await import(pathToFileURL(outfile));
  let tree, rootNode, seen, f;
  const componentInputs = [];
  const typeIds = new WeakMap(); let nextTypeId = 0;
  function walk(node, key = 'root', values = new Map(), skip = false) {
    if (Array.isArray(node)) return node.map((n, i) => walk(n, `${key}/${n?.key ?? i}`, values, skip));
    if (!node?.props) return node;
    if (node.key != null) key += '/key=' + node.key;
    let type = node.type;
    if (type?.$$typeof === Symbol.for('react.provider')) {
      const next = new Map(values); next.set(type._context, node.props.value);
      return walk(node.props.children, key + '/provider', next, skip);
    }
    const memo = type?.$$typeof === Symbol.for('react.memo');
    if (memo) type = type.type;
    if (typeof type === 'function') {
      if (!typeIds.has(type)) typeIds.set(type, ++nextTypeId);
      const id = `${key}:${typeIds.get(type)}`;
      componentInputs.push({ type, props: node.props, key: node.key, id });
      seen.add(id);
      let r = records.get(id);
      const shallow = (a, b) => a && b && Object.keys(a).length === Object.keys(b).length && Object.keys(a).every(k => Object.is(a[k], b[k]));
      const contextChanged = r && [...r.consumed].some(([c, v]) => !Object.is(v, values.has(c) ? values.get(c) : c._currentValue));
      const reuse = r && shallow(r.props, node.props) && !r.dirty && !contextChanged && (skip || memo);
      if (!reuse) {
        begin(id, values); r = records.get(id);
        r.output = type(node.props); r.props = node.props; r.dirty = false;
      }
      return walk(r.output, id + '/output', values, reuse);
    }
    if (type === React.Fragment) return walk(node.props.children, key + '/fragment', values, skip);
    const children = walk(node.props.children, key + '/children', values, skip);
    // Lightweight host refs only; not a browser/layout/accessibility simulator.
    const host = { style: {}, focus() {}, getBoundingClientRect: () => ({ left: 0, bottom: 0, width: 0, height: 0 }), querySelector: () => null };
    if (typeof node.ref === 'function') node.ref(host); else if (node.ref) node.ref.current = host;
    return React.createElement(type, { ...node.props, children });
  }
  function render() {
    componentInputs.length = 0; seen = new Set(); tree = walk(rootNode);
    for (const [id, r] of records) if (!seen.has(id)) {
      r.mounted = false;
      for (const c of r.cells) c?.cleanup?.();
      records.delete(id);
    }
    flushEffects();
  }
  async function pump() { for (let i = 0; i < 25; i++) { await Promise.resolve(); render(); } }
  const calls = name => f.calls.filter(c => c.name === name);
  const buttonMatching = matcher => {
    const matches = nodes(tree).filter(n => n.type === 'button' && (typeof matcher === 'string' ? text(n) === matcher : matcher.test(text(n))));
    assert.equal(matches.length, 1, `expected one button ${matcher}, got ${matches.length}: ${nodes(tree).filter(n => n.type === 'button').map(text).join(' | ')}`);
    return matches[0];
  };
  const back = () => buttonMatching(/Back|返回/i);
  const raw = () => buttonMatching(/^(Raw(?: source)?|原文|原始)(\s*\/.*)?$/i);
  const preview = () => buttonMatching(/^(Preview|预览)(\s*\/.*)?$/i);
  const retry = () => buttonMatching(/Retry|重试/i);
  function card(file) {
    const component = componentInputs.find(n => n.props.s?.file === file && /SkillCard/.test(n.type.name));
    assert.ok(component, `real SkillCard missing for ${file}`);
    // The actual accessible card is identified by its unique full-file metadata.
    const matches = nodes(tree).filter(n => n.type === 'button' && (n.props.title?.includes(file) || text(n).includes(file) || n.props['aria-label']?.includes(file)));
    assert.equal(matches.length, 1, `expected native skill button with path ${file}`);
    return matches[0];
  }
  async function click(node) {
    assert.equal(!!node.props.disabled, false, `button disabled: ${text(node)}`);
    const operation = node.props.onClick(); await pump(); return operation;
  }
  const panel = (props = {}, key = 'source-A') => React.createElement(ConfigPanel, { ...props, key });
  async function fixture(overrides = {}, props = {}, node) {
    reset();
    const forbid = action => () => { throw Error(`forbidden offline side effect: ${action}`); };
    globalThis.fetch = forbid('fetch');
    globalThis.XMLHttpRequest = forbid('XMLHttpRequest');
    globalThis.Image = forbid('Image');
    globalThis.window = { __TAURI_INTERNALS__: {}, open: forbid('window.open'), setTimeout: () => 1, clearTimeout() {}, addEventListener() {}, removeEventListener() {} };
    globalThis.document = { hidden: false, activeElement: null, addEventListener() {}, removeEventListener() {} };
    globalThis.requestAnimationFrame = () => 1; globalThis.cancelAnimationFrame = () => {};
    f = { calls: [], staleWrites: 0, selected: null };
    const defaults = {
      list_config: async () => inventory(), read_config_skill: async ({ file }) => documentFor(skill(file)),
      get_remote_host: async () => null, list_remote_hosts: async () => ['remote'], pi_bin_path: async () => '/synthetic/pi', pi_version: async () => 'test',
      list_projects: async () => [], list_sessions: async () => [], list_running: async () => [],
      remote_sync_status: async ({ host }) => ({ host, phase: 'ready', usableCache: true, lastSuccessAt: 1 }),
      set_remote_host: async ({ host }) => { f.selected = host; }, refresh_remote: async () => {},
    };
    f.api = { ...defaults, ...overrides };
    f.invoke = (name, args) => {
      f.calls.push({ name, args });
      assert.ok(Object.hasOwn(f.api, name), `unmocked/forbidden IPC ${name}`);
      return f.api[name](args);
    };
    globalThis.fixture = f;
    rootNode = node ?? panel(props); render(); await pump(); return f;
  }
  function assertReadOnly() {
    for (const n of nodes(tree)) {
      if (n.type === 'button') assert.match(text(n), /Back|Preview|Raw source|Retry|返回|预览|原文|重试/i, 'no editor/opener/reveal/save/send action in read-only detail');
      assert.ok(!['input', 'textarea', 'select', 'form', 'script', 'iframe', 'object', 'embed', 'img', 'video', 'audio', 'source'].includes(n.type), `read-only detail rendered ${n.type}`);
      assert.ok(!n.props.contentEditable && !n.props.dangerouslySetInnerHTML, 'no HTML injection/editor');
    }
    assert.ok(!f.calls.some(c => /open|reveal|send|write|save|import|authorized/.test(c.name)), 'detail must never invoke opener/filesystem/send IPC');
  }

  await fixture();
  assert.equal(calls('list_config').length, 1, 'default ready=true inventories once');
  assert.equal(calls('read_config_skill').length, 0, 'inventory never reads bodies');
  for (const s of [A, B]) {
    const n = card(s.file);
    assert.equal(n.props.type, 'button', 'native keyboard-activatable button, not clickable div');
    assert.match(text(n), /same-name/);
    assert.ok(text(n).includes(s.description));
    assert.ok(text(n).includes(s.file));
    assert.ok(n.props['aria-label']);
  }
  const cardInputs = componentInputs.filter(n => /SkillCard/.test(n.type.name));
  assert.deepEqual(cardInputs.map(n => n.key), [A.file, B.file], 'same-name/source rows keyed by file');
  // Reorder the actual rendered inventory data without changing identities.
  // State is deliberately the exact mock result, so mutate only this synthetic array.
  const reordered = inventory([A, B]);
  await fixture({ list_config: async () => reordered });
  const beforeReorder = componentInputs.find(n => n.props.s?.file === A.file).id;
  reordered.skills.reverse(); render();
  assert.equal(componentInputs.find(n => n.props.s?.file === A.file).id, beforeReorder);
  for (const s of [A, B]) {
    await click(card(s.file));
    assert.deepEqual(calls('read_config_skill').at(-1).args, { file: s.file });
    assert.ok(text(tree).includes(s.file)); assert.ok(text(tree).includes(s.source));
    assert.match(text(tree), /Read.only/i); assertReadOnly();
    const count = calls('read_config_skill').length;
    await click(raw()); await click(preview());
    assert.equal(calls('read_config_skill').length, count, 'view toggles do not re-fetch');
    await click(back());
    assert.equal(calls('read_config_skill').length, count, 'Back does not fetch');
    assert.equal(calls('list_config').length, 1, 'Back restores inventory without re-fetch');
  }
  console.log('PASS real native SkillCards, same-name file keys/reorder, exact API arguments, internal detail, Back and click-only reads');

  const bodyWait = deferred();
  await fixture({ read_config_skill: () => bodyWait.promise });
  const doubleOpen = card(A.file).props.onClick;
  doubleOpen(); doubleOpen(); await pump();
  assert.equal(calls('read_config_skill').length, 1, 'duplicate captured clicks issue exactly one body IPC');
  assert.match(text(tree), /Loading skill/i);
  assert.ok(nodes(tree).some(n => n.props.role === 'status' && /Loading skill/i.test(text(n))));
  assert.ok(text(tree).includes(A.file)); assert.ok(text(tree).includes(A.source));
  assertReadOnly();
  bodyWait.reject(new Error('synthetic skill read denied')); await pump();
  assert.ok(nodes(tree).some(n => n.props.role === 'alert' && /synthetic skill read denied/.test(text(n))));
  assert.equal(calls('read_config_skill').length, 1, 'failure does not auto-retry');
  const retryWait = deferred(); f.api.read_config_skill = () => retryWait.promise;
  const doubleRetry = retry().props.onClick;
  doubleRetry(); doubleRetry(); await pump();
  assert.equal(calls('read_config_skill').length, 2, 'duplicate retry issues exactly one further IPC');
  retryWait.resolve(documentFor(A, '# Recovered skill')); await pump();
  assert.match(text(tree), /Recovered skill/); assert.doesNotMatch(text(tree), /synthetic skill read denied/);
  assertReadOnly();
  console.log('PASS body loading/status, visible failure, explicit retry recovery and synchronous duplicate click/retry guards');

  // Exercise the real host handler, not synthetic browser dispatch/focus.
  for (const outcome of ['resolve', 'reject']) {
    const pending = deferred();
    await fixture({ read_config_skill: () => pending.promise });
    card(A.file).props.onClick(); await pump();
    const section = nodes(tree).find(n => n.type === 'section' && n.props['data-skill-file'] === A.file);
    assert.equal(typeof section?.props.onKeyDown, 'function', 'real detail section owns Escape handling');
    const beforeCalls = f.calls.length;
    const detailText = text(tree);
    const event = (key, defaultPrevented = false) => ({
      key, defaultPrevented, prevented: 0, stopped: 0,
      preventDefault() { this.defaultPrevented = true; this.prevented++; },
      stopPropagation() { this.stopped++; },
    });
    for (const ignored of [event('Escape', true), event('Enter')]) {
      section.props.onKeyDown(ignored); await pump();
      assert.equal(ignored.prevented, 0, 'already-handled Escape/unrelated key is not intercepted');
      assert.equal(ignored.stopped, 0);
      assert.equal(text(tree), detailText, 'already-handled Escape/unrelated key leaves detail open');
      assert.equal(f.calls.length, beforeCalls);
    }
    const escape = event('Escape');
    section.props.onKeyDown(escape); await pump();
    assert.equal(escape.prevented, 1); assert.equal(escape.stopped, 1);
    assert.ok(card(A.file), 'unhandled Escape returns to inventory');
    assert.equal(nodes(tree).filter(n => n.type === 'section' && n.props['data-skill-file']).length, 0);
    assert.equal(f.calls.length, beforeCalls, 'Escape Back does not invoke any IPC');
    const inventoryText = text(tree);
    if (outcome === 'resolve') pending.resolve(documentFor(A, 'ESCAPE-STALE-BODY'));
    else pending.reject(new Error('ESCAPE-STALE-ERROR'));
    await pump();
    assert.equal(text(tree), inventoryText, 'abandoned body result/error cannot reopen Escape-dismissed detail');
    assert.equal(f.calls.length, beforeCalls, 'late completion after Escape causes no follow-up IPC');
    assert.equal(f.staleWrites, 0, 'Escape-unmounted detail receives no late state writes');
  }
  console.log('PASS real detail Escape Back respects defaultPrevented, ignores other keys, and suppresses late body success/error without IPC (not browser focus)');

  for (const badDocument of [
    { file: B.file, content: '# WRONG-TARGET-BODY' },
    { file: A.file, content: { invalid: 'WRONG-NONSTRING-BODY' } },
  ]) {
    await fixture({ read_config_skill: async () => badDocument });
    await click(card(A.file));
    assert.deepEqual(calls('read_config_skill').map(c => c.args), [{ file: A.file }]);
    assert.ok(nodes(tree).some(n => n.props.role === 'alert' && /does not match the selected file/i.test(text(n))), 'invalid document response produces a visible target-validation error');
    assert.doesNotMatch(text(tree), /WRONG-TARGET-BODY|WRONG-NONSTRING-BODY/);
    assert.equal(nodes(tree).filter(n => n.type === 'pre' || n.props['aria-label'] === 'Skill display').length, 0, 'invalid body exposes neither raw view nor preview controls');
    const detail = nodes(tree).find(n => n.type === 'section' && n.props['data-skill-file'] === A.file);
    assert.ok(detail); assert.equal(detail.props['data-skill-document-file'], undefined, 'wrong document never becomes selected document');
    assertReadOnly();
    f.api.read_config_skill = async () => documentFor(A, '# Correct target after retry');
    await click(retry());
    assert.deepEqual(calls('read_config_skill').map(c => c.args), [{ file: A.file }, { file: A.file }], 'Retry keeps the selected file, never the mismatched response target');
    assert.match(text(tree), /Correct target after retry/);
    assert.doesNotMatch(text(tree), /does not match|WRONG-TARGET-BODY|WRONG-NONSTRING-BODY/);
    assert.equal(nodes(tree).find(n => n.type === 'section').props['data-skill-document-file'], A.file);
    assertReadOnly();
  }
  console.log('PASS mismatched-file/non-string body responses show visible error without wrong content; explicit Retry reads the correct selected file');

  // Loading and empty inventory, failure+explicit retry; no implicit body read.
  const listWait = deferred();
  await fixture({ list_config: () => listWait.promise });
  assert.match(text(tree), /Loading/); assert.equal(calls('list_config').length, 1);
  assert.equal(calls('read_config_skill').length, 0);
  listWait.reject(new Error('inventory unavailable')); await pump();
  assert.ok(nodes(tree).some(n => n.props.role === 'alert' && /inventory unavailable/.test(text(n))));
  const listRetryWait = deferred(); f.api.list_config = () => listRetryWait.promise;
  const listRetry = retry().props.onClick; listRetry(); listRetry(); await pump();
  assert.equal(calls('list_config').length, 2);
  listRetryWait.resolve(inventory([])); await pump();
  assert.match(text(tree), /No skills found/); assert.equal(calls('read_config_skill').length, 0);
  console.log('PASS pending/failed/empty inventory and explicit duplicate-guarded inventory retry');

  // Each outcome is tested after both navigation away and A->B->A re-selection.
  for (const outcome of ['resolve', 'reject']) {
    const oldA = deferred(); let readsA = 0;
    await fixture({ read_config_skill: ({ file }) => file === A.file && ++readsA === 1 ? oldA.promise : Promise.resolve(documentFor(skill(file), `# fresh ${file}`)) });
    card(A.file).props.onClick(); await pump();
    await click(back()); await click(card(B.file));
    assert.match(text(tree), /fresh/); assert.ok(text(tree).includes(B.file));
    await click(back()); await click(card(A.file));
    const freshText = text(tree);
    if (outcome === 'resolve') oldA.resolve(documentFor(A, 'OBSOLETE-A-B-A-BODY'));
    else oldA.reject(new Error('OBSOLETE-A-B-A-ERROR'));
    await pump();
    assert.equal(text(tree), freshText, 'first A completion must not replace the second A result');
    assert.equal(calls('read_config_skill').length, 3);
  }
  console.log('PASS late A results/errors suppressed across Back -> B -> Back -> A');

  for (const outcome of ['resolve', 'reject']) {
    const pending = deferred();
    await fixture({ read_config_skill: () => pending.promise });
    const staleClick = card(A.file).props.onClick;
    staleClick(); await pump(); await click(back());
    if (outcome === 'resolve') pending.resolve(documentFor(A, 'BACK-STALE-BODY'));
    else pending.reject(new Error('BACK-STALE-ERROR'));
    await pump();
    assert.ok(card(A.file)); assert.doesNotMatch(text(tree), /BACK-STALE/);
    assert.equal(calls('read_config_skill').length, 1);
  }
  for (const requestKind of ['inventory', 'body']) for (const outcome of ['resolve', 'reject']) {
    const pending = deferred();
    await fixture(requestKind === 'inventory' ? { list_config: () => pending.promise } : { read_config_skill: () => pending.promise });
    const savedClick = requestKind === 'body' ? card(A.file).props.onClick : null;
    if (savedClick) { savedClick(); await pump(); }
    rootNode = null; render(); assert.equal(records.size, 0);
    const before = f.calls.length;
    savedClick?.();
    assert.equal(f.calls.length, before, 'captured click after unmount cannot invoke IPC');
    if (outcome === 'resolve') pending.resolve(requestKind === 'inventory' ? inventory() : documentFor(A, 'UNMOUNTED-BODY'));
    else pending.reject(new Error('UNMOUNTED-ERROR'));
    await pump(); assert.equal(records.size, 0); assert.equal(f.staleWrites, 0, 'late completion performs no unmounted state writes');
  }
  console.log('PASS Back and unmount discard late inventory/body results AND errors; no post-unmount IPC/state writes');

  let allowed = true;
  const validator = () => allowed;
  await fixture({}, { validate: validator });
  const staleClick = card(A.file).props.onClick;
  allowed = false; // Intentionally BEFORE render, to catch effect-only guards.
  staleClick();
  assert.equal(calls('read_config_skill').length, 0, 'synchronous validator blocks stale click');
  await pump(); assert.match(text(tree), /Waiting for source/);
  await fixture({}, { ready: false });
  assert.equal(calls('list_config').length, 0);
  assert.equal(calls('read_config_skill').length, 0);
  rootNode = panel({ ready: true }); render(); await pump();
  assert.equal(calls('list_config').length, 1);
  const readyClick = card(A.file).props.onClick;
  rootNode = panel({ ready: false }); render(); readyClick(); await pump();
  assert.equal(calls('read_config_skill').length, 0, 'old handler blocked after readiness loss');
  await fixture({}, { validate: () => false });
  assert.equal(calls('list_config').length, 0); assert.equal(calls('read_config_skill').length, 0);
  console.log('PASS default ready/validator, pending-ready=false blocks all Config IPC, and validator=false rejects captured click before render');

  for (const requestKind of ['inventory', 'body']) for (const outcome of ['resolve', 'reject']) {
    let allowedNow = true;
    const validNow = () => allowedNow;
    const pending = deferred();
    await fixture(requestKind === 'inventory' ? { list_config: () => pending.promise } : { read_config_skill: () => pending.promise }, { validate: validNow });
    if (requestKind === 'body') { card(A.file).props.onClick(); await pump(); }
    allowedNow = false; // Reject completion BEFORE a context/readiness rerender.
    if (outcome === 'resolve') pending.resolve(requestKind === 'inventory' ? inventory([skill('/synthetic/VALIDATOR-STALE/SKILL.md')]) : documentFor(A, 'VALIDATOR-STALE-BODY'));
    else pending.reject(new Error('VALIDATOR-STALE-ERROR'));
    await pump();
    allowedNow = true; render(); await pump();
    assert.doesNotMatch(text(tree), /VALIDATOR-STALE/, 'false synchronous validator suppresses pending result/error, not merely its display');
  }
  for (const requestKind of ['inventory', 'body']) for (const outcome of ['resolve', 'reject']) {
    const pending = deferred();
    await fixture(requestKind === 'inventory' ? { list_config: () => pending.promise } : { read_config_skill: () => pending.promise });
    if (requestKind === 'body') { card(A.file).props.onClick(); await pump(); }
    rootNode = panel({ ready: false }); render(); await pump();
    const before = f.calls.length;
    if (outcome === 'resolve') pending.resolve(requestKind === 'inventory' ? inventory([skill('/synthetic/NOT-READY-STALE/SKILL.md')]) : documentFor(A, 'NOT-READY-STALE-BODY'));
    else pending.reject(new Error('NOT-READY-STALE-ERROR'));
    await pump(); assert.equal(f.calls.length, before, 'readiness loss causes no follow-up IPC');
    f.api.list_config = async () => inventory();
    rootNode = panel({ ready: true }); render(); await pump();
    assert.ok(card(A.file)); assert.doesNotMatch(text(tree), /NOT-READY-STALE/);
  }
  allowed = true;
  await fixture({ read_config_skill: async () => { throw Error('retry scope failure'); } }, { validate: validator });
  await click(card(A.file));
  const staleRetry = retry().props.onClick;
  allowed = false; staleRetry();
  assert.equal(calls('read_config_skill').length, 1, 'validator rejects retry before render');
  console.log('PASS in-place readiness loss and pre-render validator invalidation suppress pending inventory/body success/error and stale retry');

  // Source epoch/key remount + readiness change each invalidate BOTH request kinds.
  for (const requestKind of ['inventory', 'body']) for (const outcome of ['resolve', 'reject']) {
    const pending = deferred();
    let epoch = 1;
    const validA = () => epoch === 1;
    await fixture(requestKind === 'inventory' ? { list_config: () => pending.promise } : { read_config_skill: () => pending.promise }, { validate: validA });
    const savedClick = requestKind === 'body' ? card(A.file).props.onClick : null;
    if (savedClick) { savedClick(); await pump(); }
    epoch = 2;
    f.api.list_config = async () => inventory([skill('/synthetic/source-B/SKILL.md', 'source-B')]);
    rootNode = panel({ ready: false, validate: () => epoch === 2 }, 'source-B'); render(); await pump();
    const before = f.calls.length; savedClick?.(); assert.equal(f.calls.length, before);
    assert.equal(calls('list_config').length, 1, 'pending source does not read inventory');
    rootNode = panel({ ready: true, validate: () => epoch === 2 }, 'source-B'); render(); await pump();
    assert.match(text(tree), /source-B/);
    epoch = 3;
    f.api.list_config = async () => inventory();
    f.api.read_config_skill = async ({ file }) => documentFor(skill(file), 'LATEST-SOURCE-A');
    rootNode = panel({ validate: () => epoch === 3 }, 'source-A-epoch-3'); render(); await pump();
    await click(card(A.file)); const fresh = text(tree);
    if (outcome === 'resolve') pending.resolve(requestKind === 'inventory' ? inventory([skill('/synthetic/OBSOLETE/SKILL.md')]) : documentFor(A, 'OBSOLETE-SOURCE-A'));
    else pending.reject(new Error('OBSOLETE-SOURCE-A-ERROR'));
    await pump(); assert.equal(text(tree), fresh); assert.equal(f.staleWrites, 0);
  }
  console.log('PASS source A->pending B->ready B->A key epochs suppress stale inventory/body success/error, even for identical skill file');

  // Real App + real ConfigPanel: only unrelated heavy views are mocked.
  await fixture({}, {}, React.createElement(App));
  f.Sidebar.onOpenConfig(); await pump();
  assert.equal(calls('list_config').length, 1);
  const firstConfig = componentInputs.find(n => n.type === ConfigPanel);
  assert.equal(firstConfig.props.ready, true); assert.equal(firstConfig.props.validate(), true);
  const oldConfigGuard = firstConfig.props.validate;
  const oldAppClick = card(A.file).props.onClick;
  const selectionWait = deferred();
  f.api.set_remote_host = ({ host }) => { f.selected = host; return selectionWait.promise; };
  const switching = f.Sidebar.onSwitchSource('remote');
  assert.equal(oldConfigGuard(), false, 'App source epoch invalidates Config synchronously');
  oldAppClick();
  assert.equal(calls('read_config_skill').length, 0);
  await pump();
  // App intentionally closes Config when switching. Reopen while selection IPC
  // is pending to exercise its ready=false contract, not a visibility assumption.
  f.Sidebar.onOpenConfig(); await pump();
  const pendingConfig = componentInputs.find(n => n.type === ConfigPanel);
  assert.ok(pendingConfig, 'Config can be opened during source switch');
  assert.equal(pendingConfig.props.ready, false);
  assert.notEqual(pendingConfig.key, firstConfig.key, 'Config keyed by source epoch, not constant source name');
  assert.equal(calls('list_config').length, 1, 'App pending source must not inventory');
  selectionWait.resolve(); await switching; await pump();
  assert.equal(componentInputs.find(n => n.type === ConfigPanel).props.ready, true);
  assert.equal(calls('list_config').length, 2);
  f.api.set_remote_host = async ({ host }) => { f.selected = host; };
  await f.Sidebar.onSwitchSource(null); await pump();
  f.Sidebar.onOpenConfig(); await pump();
  const latestConfig = componentInputs.find(n => n.type === ConfigPanel);
  assert.notEqual(latestConfig.key, firstConfig.key);
  assert.equal(oldConfigGuard(), false, 'local->remote->local never revives first guard');
  assert.equal(latestConfig.props.validate(), true);
  assert.equal(calls('list_config').length, 3);
  assert.equal(calls('read_config_skill').length, 0);
  await click(card(B.file)); assert.equal(calls('read_config_skill').length, 1);
  assert.deepEqual(calls('read_config_skill')[0].args, { file: B.file });
  assertReadOnly();
  console.log('PASS real App/Config integration: source-epoch keys, pre-render validator, zero pending-source inventory/body IPC, fresh Local->remote->Local');
  for (const [source, expectedPreview] of [
    ['---\nname: hidden-LF-frontmatter\n...\n# LF body\n\n---\n\nTAIL', '# LF body\n\n---\n\nTAIL'],
    ['---\nname: unclosed-frontmatter\n# Still content', '---\nname: unclosed-frontmatter\n# Still content'],
    ['prefix\n---\nnot-leading-frontmatter', 'prefix\n---\nnot-leading-frontmatter'],
    ['', ''],
  ]) {
    await fixture({ read_config_skill: async () => documentFor(A, source) });
    await click(card(A.file));
    const markdownInput = componentInputs.find(n => /Markdown/.test(n.type.name) && typeof n.props.text === 'string');
    assert.ok(markdownInput, 'real Markdown component receives preview body');
    assert.equal(markdownInput.props.text, expectedPreview, 'only complete leading frontmatter is removed');
    await click(raw());
    assert.equal(text(nodes(tree).find(n => n.type === 'pre')), source);
    assertReadOnly(); assert.equal(calls('read_config_skill').length, 1);
  }
  console.log('PASS LF/dot-terminated frontmatter, non-leading/unclosed delimiters, empty document and exact raw view through real components');

  const frontmatter = '\uFEFF---\r\nname: raw-frontmatter-only\r\ndescription: "中文 🧪"\r\n---\r\n';
  const content = frontmatter + '# Skill preview heading\r\n\r\nUTF-8 中文 🦉 café\r\n\r\n' +
    '<script>globalThis.__configSkillExecuted = true</script>\n<div onclick="globalThis.__configSkillExecuted=true">HTML must remain inert</div>\n\n' +
    '[local-file](./private.txt) [unsafe](javascript:alert%281%29)\n\n' +
    '![tracking](https://offline.invalid/pixel.png) ![local-image](file:///synthetic/private.png)\n\n' +
    '```yaml\n---\nkeep-code-fence-frontmatter\n---\n```\n\n' +
    ('Large UTF-8 line 中文🦉é\n'.repeat(8000)) + 'END-OF-LARGE-SKILL\n';
  assert.ok(Buffer.byteLength(content, 'utf8') > 200_000);
  globalThis.__configSkillExecuted = false;
  await fixture({ read_config_skill: async () => documentFor(A, content) });
  await click(card(A.file));
  assert.doesNotMatch(text(tree), /raw-frontmatter-only/);
  assert.match(text(tree), /Skill preview heading/);
  assert.match(text(tree), /keep-code-fence-frontmatter/);
  assert.match(text(tree), /END-OF-LARGE-SKILL/);
  assert.ok(nodes(tree).some(n => n.type === 'h1' && text(n) === 'Skill preview heading'), 'real parser renders Markdown heading');
  assert.equal(globalThis.__configSkillExecuted, false);
  for (const anchor of nodes(tree).filter(n => n.type === 'a')) {
    assert.ok(!/^javascript:/i.test(anchor.props.href ?? ''), 'unsafe URL sanitized by real parser');
  }
  const localLink = nodes(tree).find(n => n.type === 'a' && text(n) === 'local-file');
  if (localLink?.props.onClick) {
    await localLink.props.onClick({ preventDefault() {} }); await pump();
    assert.equal(f.calls.length, 2, 'local preview link cannot invoke filesystem/opener IPC');
  } else assert.ok(!localLink?.props.href, 'read-only link must be inert if no guarded handler');
  await click(raw());
  const rawBlock = nodes(tree).find(n => n.type === 'pre' && text(n) === content);
  assert.ok(rawBlock, 'raw view preserves the complete BOM/CRLF/frontmatter/HTML/UTF-8 content exactly');
  assert.equal(text(rawBlock), content);
  assert.equal(raw().props['aria-pressed'], true);
  assertReadOnly();
  await click(preview());
  assert.equal(preview().props['aria-pressed'], true);
  assert.equal(calls('read_config_skill').length, 1);
  assertReadOnly();
  delete globalThis.__configSkillExecuted;
  console.log('PASS >200 KB UTF-8, exact BOM/CRLF raw/frontmatter, real Markdown preview strips only leading frontmatter, inert HTML/scripts/unsafe URLs and no automatic images');

  reset();
  console.log('All config-skill tests passed (offline mock IPC; not native/browser E2E or backend file-allowlist verification).');
} finally {
  await rm(scratch, { recursive: true, force: true });
  delete globalThis.fixture;
}
