// Real component SSR + direct event-handler checks; synthetic metadata only.
// Run: node scripts/test-history-controls.mjs (no browser, IPC or live sessions).
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import { renderToStaticMarkup } from 'react-dom/server';

const root = fileURLToPath(new URL('..', import.meta.url));
const scratch = await mkdtemp(path.join(root, 'node_modules/.history-controls-'));
const page = {
  generation: 'synthetic-generation', previousCursor: 'opaque', totalEntries: 289,
  branchEntries: 271, matchedEntries: 271, returnedEntries: 100, hasMore: true,
  incompleteTail: false, malformedLines: 0,
  counters: { messages: 268, user: 19, assistant: 118, toolResult: 131, labeled: 3 },
  payloadBytes: 12345,
  // Rendering history navigation must never walk tool pairs or entry bodies.
  toolPairs: new Proxy([], { get() { throw new Error('History controls must not inspect pairs/content'); } }),
};
const updatedNotices = [
  'History changed — refreshed bounded page. Previously loaded older pages/full bodies were released; load them again on demand.',
  'History changed — old cursor rejected. Refreshed latest bounded page; load older history again.',
];
function nodes(node, out = []) {
  if (Array.isArray(node)) node.forEach(n => nodes(n, out));
  else if (node && typeof node === 'object' && node.props) { out.push(node); nodes(node.props.children, out); }
  return out;
}
function text(node) {
  return Array.isArray(node) ? node.map(text).join('') : node && typeof node === 'object' && node.props
    ? text(node.props.children) : node == null || typeof node === 'boolean' ? '' : String(node);
}
const hasClass = (n, name) => n.props.className?.split(' ').includes(name);
const classNode = (tree, name) => nodes(tree).find(n => hasClass(n, name));
const button = (tree, label) => nodes(tree).find(n => n.type === 'button' && text(n) === label);
const input = tree => nodes(tree).find(n => n.type === 'input');

try {
  const outfile = path.join(scratch, 'history-controls.mjs');
  await build({
    entryPoints: [path.join(root, 'src/components/HistoryControls.tsx')], outfile,
    bundle: true, platform: 'node', format: 'esm', packages: 'external',
    loader: { '.css': 'empty' }, logLevel: 'silent',
  });
  const { HistoryControls } = await import(pathToFileURL(outfile));
  let latest = 0, viewed = 0;
  const changes = [];
  const props = {
    page, loaded: 100, busy: false, branchLeaf: '',
    onBranchLeafChange: value => changes.push(value),
    onLatest: () => latest++, onViewBranch: () => viewed++,
  };
  const render = overrides => HistoryControls({ ...props, ...overrides });
  let tree = render();
  const html = renderToStaticMarkup(tree);
  assert.equal(text(classNode(tree, 'history-controls-count')), '100 / 271');
  assert.match(classNode(tree, 'history-controls-count').props.title, /100 loaded entries \/ 271 matching entries/);
  assert.match(classNode(tree, 'history-controls-count').props['aria-label'], /current branch/);
  assert.equal(nodes(tree).filter(n => n.type === 'details').length, 1);
  assert.equal(nodes(tree).find(n => n.type === 'details').props.open, undefined);
  assert.equal(nodes(tree).filter(n => n.type === 'summary').length, 1);
  assert.equal(nodes(tree).find(n => n.type === 'summary').props['aria-label'], 'History details and branch navigation');
  assert.equal(nodes(tree).filter(n => n.type === 'button').length, 2);
  assert.equal(input(tree).props['aria-label'], 'Branch leaf entry ID');
  assert.equal(input(tree).props.value, '');
  assert.ok(nodes(classNode(tree, 'history-controls-flyout')).includes(input(tree)), 'branch controls remain mounted inside disclosure');
  assert.match(html, /271 branch entries \/ 289 file entries/);
  assert.match(html, /268 messages · 19 user · 118 assistant · 131 tool results · 3 labeled/);
  assert.match(html, /100 entries in last fetched page · 12345 payload bytes · Older history available/);
  assert.match(html, /synthetic-generation/);
  assert.ok(html.length < 6000, 'bounded metadata-only markup');
  assert.equal(classNode(tree, 'history-controls-chip'), undefined);
  let focused=0, stopped=0;
  const disclosure={open:true,querySelector:()=>({focus(){focused++;}})};
  nodes(tree).find(n=>n.type==='details').props.onKeyDown({key:'Escape',currentTarget:disclosure,stopPropagation(){stopped++;}});
  assert.equal(disclosure.open,false);
  assert.deepEqual([focused,stopped],[1,1]);
  assert.deepEqual([latest,viewed,changes.length],[0,0,0]);
  console.log('PASS compact count, native closed disclosure, accessible summary/input, Escape dismissal, mounted full statistics');

  for (const notice of updatedNotices) {
    tree = render({ notice });
    assert.equal(text(classNode(tree, 'history-controls-chip')), 'History updated');
    assert.equal(classNode(tree, 'history-controls-chip').props.title, notice);
    assert.equal(text(classNode(tree, 'history-controls-notice')), notice);
    assert.equal(hasClass(classNode(tree, 'history-controls-chip'), 'history-controls-warning'), false);
  }
  for (const notice of [
    'Could not load history: synthetic permission error\nRetry Latest; previous page retained.',
    'Search/filter failed — previous result page retained. Retry Latest; no results for the new query confirmed.',
    updatedNotices[0] + ' Additional malformed warning!',
    '<unknown diagnostic> & all evidence must remain intact',
    'x'.repeat(2000) + ' COMPLETE NOTICE TAIL',
  ]) {
    tree = render({ notice });
    assert.match(text(classNode(tree, 'history-controls-chip')), /History warning/);
    assert.equal(classNode(tree, 'history-controls-chip').props.title, notice);
    assert.equal(text(classNode(tree, 'history-controls-notice')), notice);
    assert.equal(hasClass(classNode(tree, 'history-controls-notice'), 'history-controls-warning'), true);
    const rendered = renderToStaticMarkup(tree);
    assert.ok(!rendered.includes('<unknown diagnostic>'), 'notice is safely escaped by React');
  }
  tree = render({ notice: 'Searching/filtering the full branch…', busy: true });
  assert.equal(classNode(tree, 'history-controls-chip'), undefined);
  assert.equal(text(classNode(tree, 'history-controls-notice')), 'Searching/filtering the full branch…');
  console.log('PASS exact neutral-history allowlist, complete notice/title, unknown/error warnings never hidden');

  for (const [patch, label, body] of [
    [{ incompleteTail: true }, 'Incomplete tail', 'Incomplete trailing record — awaiting file update'],
    [{ malformedLines: 7 }, 'Malformed lines', '7 malformed lines skipped'],
    [{ incompleteTail: true, malformedLines: 7 }, 'History warning', '7 malformed lines skipped'],
  ]) {
    tree = render({ page: { ...page, ...patch } });
    assert.ok(text(classNode(tree, 'history-controls-chip')).includes(label));
    assert.equal(hasClass(classNode(tree, 'history-controls-chip'), 'history-controls-warning'), true);
    assert.ok(text(classNode(tree, 'history-controls-flyout')).includes(body));
    assert.ok(classNode(tree, 'history-controls-chip').props.title.includes(body));
  }
  tree = render({ notice: updatedNotices[0], page: { ...page, incompleteTail: true, malformedLines: 2 } });
  assert.match(text(classNode(tree, 'history-controls-chip')), /History warning/);
  assert.ok(text(tree).includes(updatedNotices[0]));
  assert.ok(text(tree).includes('Incomplete trailing record'));
  assert.ok(text(tree).includes('2 malformed lines skipped'));
  tree = render({ page: { ...page, hasMore: false, matchedEntries: 0 }, loaded: 0 });
  assert.equal(text(classNode(tree, 'history-controls-count')), '0 / 0');
  assert.match(text(tree), /No older matching entries/);
  console.log('PASS incomplete/malformed warnings remain visible when closed, even with neutral updates; empty/end counts');

  assert.deepEqual([latest, viewed, changes.length], [0, 0, 0], 'rendering/SSR never initiates navigation');
  tree = render({ activeBranchLeaf: 'active-leaf', branchLeaf: 'draft-leaf' });
  assert.equal(input(tree).props.value, 'draft-leaf');
  assert.equal(text(classNode(tree, 'history-controls-branch')), 'Branch · active-leaf');
  assert.match(classNode(tree, 'history-controls-count').props.title, /selected branch ending at active-leaf/);
  assert.match(button(tree, 'Latest').props.title, /selected branch ending at active-leaf/);
  assert.match(text(classNode(tree, 'history-controls-flyout')), /Selected branch leaf: active-leaf/);
  input(tree).props.onChange({ target: { value: 'new-leaf' } });
  input(tree).props.onChange({ target: { value: '' } });
  assert.deepEqual(changes, ['new-leaf', ''], 'empty input is forwarded unchanged for parent reset semantics');
  assert.deepEqual([latest, viewed], [0, 0], 'typing is not navigation');
  button(tree, 'Latest').props.onClick();
  button(tree, 'View branch').props.onClick();
  assert.deepEqual([latest, viewed], [1, 1], 'one callback per explicit action');
  for (const state of [{ busy: true }, { disabled: true }, { busy: true, disabled: true }]) {
    tree = render(state);
    assert.equal(button(tree, 'Latest').props.disabled, true);
    assert.equal(button(tree, 'View branch').props.disabled, true);
    assert.equal(input(tree).props.disabled, true);
    button(tree, 'Latest').props.onClick();
    button(tree, 'View branch').props.onClick();
    input(tree).props.onChange({ target: { value: 'blocked' } });
    assert.deepEqual([latest, viewed, changes.length], [1, 1, 2]);
    if (state.busy) {
      assert.equal(tree.props['aria-busy'], true);
      assert.match(text(classNode(tree, 'history-controls-busy')), /Updating/);
    }
    assert.ok(nodes(tree).find(n => n.type === 'summary'), 'metadata remains keyboard-disclosable while busy/disabled');
    renderToStaticMarkup(tree);
  }
  console.log('PASS controlled draft/active scope, reset callback, explicit-only action counts and busy/disabled guards');

  const source = await readFile(path.join(root, 'src/components/HistoryControls.tsx'), 'utf8');
  assert.doesNotMatch(source, /setTimeout|setInterval|useEffect|JSON\.parse|fetch\(/, 'no timers, auto-dismiss, fetching or reparsing');
  assert.match(source, /import "\.\/HistoryControls\.css"/);
  const css = await readFile(path.join(root, 'src/components/HistoryControls.css'), 'utf8');
  const rule = selector => css.slice(css.indexOf(selector + ' {')).split('}')[0];
  assert.match(rule('.history-controls'), /flex-wrap: wrap/);
  assert.match(rule('.history-controls'), /flex-shrink: 0/);
  assert.match(rule('.history-controls'), /max-width: 100%/);
  assert.doesNotMatch(rule('.history-controls'), /margin-top:/, 'inline search-row integration adds no vertical offset');
  assert.match(rule('.history-controls-details'), /position: relative/);
  assert.match(rule('.history-controls-flyout'), /position: absolute/);
  assert.match(rule('.history-controls-flyout'), /right: 0/);
  assert.match(rule('.history-controls-flyout'), /width: min\(360px, calc\(100vw - 40px\)\)/);
  assert.match(rule('.history-controls-flyout'), /overflow: auto/);
  assert.match(css, /:focus-visible/);
  assert.doesNotMatch(css, /details[^}]*display:\s*(?:block|flex|grid)[^}]*\.history-controls-flyout/s, 'no forced-open details panel');
  console.log('PASS compact/right-anchored responsive CSS contract and no auto-dismiss/data work');
  console.log('All history controls tests passed (real component, offline synthetic metadata; browser visual acceptance not exercised).');
} finally {
  await rm(scratch, { recursive: true, force: true });
}
