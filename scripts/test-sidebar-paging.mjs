// Offline fixture: actual Sidebar + real React SSR, with deterministic hook
// state/event updates. No browser, desktop, IPC, timers or new dependencies.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

const root = fileURLToPath(new URL('..', import.meta.url));
const require = createRequire(import.meta.url);
const scratch = await mkdtemp(path.join(root, 'node_modules/.sidebar-paging-'));
const hooks = `
  import React from ${JSON.stringify(require.resolve('react'))};
  let cells = [], cursor = 0;
  export function begin(reset = false) { if (reset) cells = []; cursor = 0; }
  export function useState(initial) {
    const index = cursor++;
    if (!(index in cells)) cells[index] = typeof initial === 'function' ? initial() : initial;
    return [cells[index], next => { cells[index] = typeof next === 'function' ? next(cells[index]) : next; }];
  }
  export function useMemo(factory) { return factory(); }
  export function useRef(initial) { return useState(() => ({ current: initial }))[0]; }
  export function useEffect() {} // Deliberately no effects: reset must precede first render.
  export default { ...React, memo: component => component };
`;
try {
  const outfile = path.join(scratch, 'fixture.mjs');
  await build({ stdin: {
    contents: `export { Sidebar } from './src/components/Sidebar'; export { sessionPage } from './src/components/sidebarPaging'; export { begin } from 'react';`,
    resolveDir: root, loader: 'tsx',
  }, bundle: true, platform: 'node', format: 'esm', outfile,
    external: [require.resolve('react'), 'react/jsx-runtime'], logLevel: 'silent',
    plugins: [{ name: 'deterministic-hooks', setup(build) {
      build.onResolve({ filter: /^react$/ }, () => ({ path: 'react', namespace: 'fixture' }));
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: hooks, loader: 'js' }));
    } }],
  });
  const { Sidebar, sessionPage, begin } = await import(pathToFileURL(outfile));
  globalThis.localStorage = { getItem: () => null, setItem: () => {} };
  const noop = () => {};
  const defaults = {
    projects: [], selectedProject: 'project-a', selectedSessionPath: null, loadingSessions: false,
    finishedAt: {}, remoteHosts: ['remote'], remoteHost: null, syncing: false, showConfig: false,
    onSelectProject: noop, onSelectSession: noop, onRefresh: noop, onOpenTerminal: noop,
    onDeleteSession: noop, onDetachFromRmux: noop, onKillRmuxSession: noop,
    onTransferToRemote: noop, onOpenConfig: noop, onSwitchSource: noop,
  };
  function session(i, sub = false, parent = null) {
    return { id: `uuid-${i}`, path: `/fixture/${i}.jsonl`, cwd: '/fixture', name: `Session ${i}`,
      firstMessage: '', lastMessage: '', createdIso: '', createdAt: 0, updatedAt: 0, model: null,
      isSubagent: sub, taskId: null, parentSessionId: null, parentSessionPath: parent,
      messageCount: 0, size: 0, running: false, sleeping: false, interrupted: false,
      inRmux: false, rmuxTarget: null, rmuxAttached: false, rmuxDead: false, termAlive: false };
  }
  function text(node) {
    if (Array.isArray(node)) return node.map(text).join('');
    if (!React.isValidElement(node)) return node == null || typeof node === 'boolean' ? '' : String(node);
    return text(node.props.children);
  }
  function natives(node, output = []) {
    if (Array.isArray(node)) { node.forEach(n => natives(n, output)); return output; }
    if (!React.isValidElement(node)) return output;
    if (typeof node.type === 'function') return natives(node.type(node.props), output);
    if (typeof node.type === 'string') output.push(node);
    natives(node.props.children, output);
    return output;
  }
  function fixture(sessions, overrides = {}) {
    begin(true);
    let props = { ...defaults, sessions, ...overrides }, tree, nodes, html;
    const api = {
      render(next = {}) {
        props = { ...props, ...next }; begin(); tree = Sidebar(props);
        html = renderToStaticMarkup(tree); nodes = natives(tree); return api;
      },
      get html() { return html; },
      nodeCount() { return nodes.length; },
      rows() { return nodes.filter(n => n.type === 'button' && n.props.className?.startsWith('session-item ')); },
      click(label, index = 0) {
        const matches = nodes.filter(n => typeof n.props.onClick === 'function' && text(n) === label);
        assert.ok(matches[index], `Missing clickable ${label}`); matches[index].props.onClick(); return api.render();
      },
      more(index = 0) {
        const buttons = nodes.filter(n => n.type === 'button' && /^Show up to \d+ more/.test(text(n)));
        assert.ok(buttons[index], 'Missing show-more button'); buttons[index].props.onClick(); return api.render();
      },
      search(query) {
        nodes.find(n => n.type === 'input' && n.props.className === 'session-search')
          .props.onChange({ target: { value: query } }); return api.render();
      },
    };
    return api.render();
  }
  const main = Array.from({ length: 4000 }, (_, i) => session(i));
  const subs = Array.from({ length: 4000 }, (_, i) => session(i, true));
  const mixed = Array.from({ length: 4000 }, (_, i) => session(i, i >= 2000));
  for (const data of [main, subs, mixed]) {
    const f = fixture(data);
    assert.equal(f.rows().length, data === mixed ? 200 : 100);
    assert.match(f.html, /Lists initially show 100 sessions/);
    assert.match(f.html, /search checks all sessions/);
  }
  const initialMain = fixture(main);
  const initialNodeCount = initialMain.nodeCount();
  const twiceAsLarge = fixture(Array.from({ length: 8000 }, (_, i) => session(i)));
  assert.equal(twiceAsLarge.rows().length, 100);
  assert.equal(twiceAsLarge.nodeCount(), initialNodeCount, 'Total initial DOM elements, not only rows, are independent of N');
  const browsed = fixture(main);
  assert.match(browsed.html, /Showing 100 of 4000 sessions/);
  browsed.more(); assert.equal(browsed.rows().length, 200);
  for (let i = 0; i < 38; i++) browsed.more();
  assert.equal(browsed.rows().length, 4000);
  assert.doesNotMatch(browsed.html, /Show up to \d+ more/);
  assert.match(browsed.html, /Session 3999/);
  assert.equal(main.length, 4000, 'Pagination must not truncate the input/history');
  console.log('PASS 4000/8000 initial SSR row bound; explicit 100-row show-more reaches all 4000');

  for (const field of ['name', 'firstMessage', 'lastMessage', 'taskId', 'id']) {
    const data = main.map(s => ({ ...s })); data[3999][field] = 'outside-initial-slice';
    const f = fixture(data).search('OUTSIDE-initial-slice');
    assert.equal(f.rows().length, 1); assert.match(f.rows()[0].props.title, /3999\.jsonl/);
  }
  const selected = fixture(main, { selectedSessionPath: main[3999].path });
  assert.equal(selected.rows().length, 101);
  assert.ok(selected.rows().find(n => n.props.className.includes('selected') && n.props.title.includes('/3999.jsonl')));
  selected.more(); assert.equal(selected.rows().length, 201);
  selected.search('does-not-match');
  assert.equal(selected.rows().length, 1);
  assert.match(selected.html, /Selected session \(outside search results or in a collapsed section\)/);
  selected.search(''); assert.equal(selected.rows().length, 101, 'Search resets pages');
  selected.click('▾Main sessions4000'); assert.equal(selected.rows().length, 1);
  selected.click('▸Main sessions4000'); assert.equal(selected.rows().length, 101);
  console.log('PASS all search fields beyond initial slice; selected row stays visible under paging/search/collapse');

  const reset = fixture(main).more();
  assert.equal(reset.rows().length, 200);
  reset.render({ sessions: [...main] }); assert.equal(reset.rows().length, 200, 'Refresh retains page');
  reset.render({ selectedProject: 'project-b' }); assert.equal(reset.rows().length, 100);
  reset.more().render({ remoteHost: 'remote' }); assert.equal(reset.rows().length, 100);
  reset.more().search('Session'); assert.equal(reset.rows().length, 100);
  reset.render({ selectedProject: 'project-a', remoteHost: null });
  assert.equal(reset.rows().length, 100, 'Returning to an old source/project must not revive its large page');
  console.log('PASS project/source/query reset synchronously; same-project refresh preserves browsing');

  const parent = session(0); parent.name = 'Named parent';
  const worker = session(1, true, parent.path); worker.name = 'Named worker';
  const nested = session(2, true, worker.path); nested.name = 'Nested needle';
  const children = Array.from({ length: 3997 }, (_, i) => session(i + 3, true, parent.path));
  const lineage = [parent, worker, nested, ...children];
  lineage.some = () => { throw Error('Unfiltered parent index must not use some()'); };
  lineage.find = () => { throw Error('Unfiltered parent index must not use find()'); };
  const grouped = fixture(lineage);
  assert.equal(grouped.rows().length, 101);
  assert.match(grouped.html, /parent: Named worker · uuid-1/);
  grouped.click('▸subagents3998');
  assert.equal(grouped.rows().length, 201, 'Each expanded child group also starts at 100');
  grouped.more(0); assert.equal(grouped.rows().length, 301);
  grouped.click('▾subagents3998'); assert.equal(grouped.rows().length, 101);
  grouped.click('▾Subagent sessions3999'); assert.equal(grouped.rows().length, 1);
  grouped.click('▸Subagent sessions3999').search('Nested needle');
  assert.equal(grouped.rows().length, 1);
  assert.match(grouped.html, /parent: Named worker · uuid-1/, 'Search retains unfiltered nested parent label');
  grouped.search('Named worker');
  assert.match(grouped.html, /parent: Named parent · uuid-0/);
  grouped.search('').click('▸subagents3998');
  assert.equal(grouped.rows().length, 201, 'Search resets group pages');
  grouped.render({ remoteHost: 'remote' });
  assert.equal(grouped.rows().length, 101, 'Source reset collapses old groups immediately');
  const pinnedWorker = fixture(lineage, { selectedSessionPath: children.at(-1).path });
  assert.equal(pinnedWorker.rows().length, 102);
  pinnedWorker.click('▾Subagent sessions3999'); assert.equal(pinnedWorker.rows().length, 2);
  pinnedWorker.click('▸subagents3998');
  assert.equal(pinnedWorker.rows().length, 102, 'Selected direct child pinned in expanded group without a duplicate fallback');
  const nestedSelected = fixture(lineage, { selectedSessionPath: nested.path }).search('not-present');
  assert.match(nestedSelected.html, /parent: Named worker · uuid-1/);
  assert.equal(sessionPage(main, 100, main[3999].path).length, 101);
  const chain = Array.from({ length: 4000 }, (_, i) => session(i, i > 0, i > 0 ? `/fixture/${i - 1}.jsonl` : null));
  const tail = fixture(chain).search('Session 3999');
  assert.equal(tail.rows().length, 1);
  assert.match(tail.html, /parent: Session 3998 · uuid-399/);
  let selectedCallback = null;
  const selectable = fixture(main, { selectedSessionPath: main[3999].path, onSelectSession: s => { selectedCallback = s; } });
  selectable.rows().find(n => n.props.className.includes('selected')).props.onClick();
  assert.equal(selectedCallback, main[3999], 'Pinned row retains the original selection callback/metadata');
  console.log(`PASS initial DOM element bound: ${initialNodeCount} elements for both 4000 and 8000 main sessions`);
  const source = await readFile(path.join(root, 'src/components/Sidebar.tsx'), 'utf8');
  assert.match(source, /mainPaths\.has\(s\.parentSessionPath\)/);
  assert.match(source, /sessionsByPath\.get\(path\)/);
  assert.doesNotMatch(source, /sessions\.(some|find)\(/);
  console.log('PASS main/direct-child grouping, child paging, nested labels, collapsed sections, full counts, indexed parents');
} finally {
  await rm(scratch, { recursive: true, force: true });
}
