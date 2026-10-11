// Offline behavioral tests of the REAL panel and App callbacks, using fake IPC
// and deterministic hooks. No desktop/private data, network, or new dependencies.
// Reusable method: bundle TSX with the existing esbuild dependency and replace
// only hooks/IPC/large child views. A->B->A epochs and unmount are tested explicitly.
// Reproducible pitfall: invalidating browse only after revocation IPC settles lets
// a pending directory response render during revoke; invalidate at its START.
// Verified pitfall: drafts keyed by path alone leak staged paths into same-path
// remote sessions. Keep source-qualified keys, tested with identical paths below.
// Remembered-authorization check: persisted rows can Browse without invoking
// the picker again. This headless invariant does NOT prove native OS grants or
// cross-restart bookmark restoration; those still require installed-app testing.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('..', import.meta.url));
const require = createRequire(import.meta.url);
const scratch = await mkdtemp(path.join(root, 'node_modules/.data-access-test-'));
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
const dir = (id = 'dir1') => ({ id, name: `Directory ${id}`, path: `/synthetic/protected/${id}`, createdAt: 1700000000000 });
const copy = (id = 'copy1') => ({ id, name: `${id}.txt`, path: `/private/app/imports/${id}.txt`, size: 123, sourcePath: '/synthetic/protected/private-wechat.txt' });
const entry = (name, isDirectory = false) => ({ name, relativePath: name, isDirectory, size: isDirectory ? 0 : 123 });
const page = (relativePath = '', entries = [entry('folder', true), entry('message.txt')], hasMore = false) => ({ relativePath, entries, hasMore });
const session = (p, running = false) => ({ path: p, id: p, name: p, running, size: 1, updatedAt: 1 });
const detail = p => ({ path: p, id: p, cwd: '/synthetic', entries: [], active: [], stats: {}, size: 1, updatedAt: 1 });

try {
  const outfile = path.join(scratch, 'fixture.mjs');
  await build({ stdin: { contents: `export { default as App } from './src/App'; export { DataAccessPanel, copiedFileInstruction } from './src/components/DataAccessPanel'; export { begin, reset, flushEffects } from 'react';`, resolveDir: root, loader: 'tsx' },
    bundle: true, platform: 'node', format: 'esm', outfile, external: [require.resolve('react'), 'react/jsx-runtime'], logLevel: 'silent',
    plugins: [{ name: 'offline-data-access', setup(b) {
      b.onResolve({ filter: /^react$/ }, () => ({ path: 'react', namespace: 'fixture' }));
      b.onResolve({ filter: /^@tauri-apps\/api\// }, args => ({ path: args.path, namespace: 'fixture' }));
      // Keep the panel real; replace only the heavy existing child components.
      b.onResolve({ filter: /^\.\/components\/(Sidebar|Thread|Composer|ConfigPanel)$/ }, args => ({ path: args.path, namespace: 'fixture' }));
      b.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path: p }) => {
        if (p === 'react') return { contents: hooks };
        if (p.endsWith('/core')) return { contents: 'export class Channel {} export const invoke = (name, args) => globalThis.fixture.invoke(name, args);' };
        if (p.endsWith('/window')) return { contents: 'export const getCurrentWindow = () => ({ onFocusChanged: () => Promise.resolve(() => {}) });' };
        const name = p.split('/').pop();
        return { contents: `export function ${name}() { return null; } ${name === 'Thread' ? 'export function appendLiveEvents(prev, batch) { return [...prev, ...batch]; }' : ''}` };
      });
    } }],
  });
  const { App, DataAccessPanel, copiedFileInstruction, begin, reset, flushEffects } = await import(pathToFileURL(outfile));
  let tree, component, panelProps, f;
  function nodes(node, out = []) {
    if (Array.isArray(node)) node.forEach(n => nodes(n, out));
    else if (node && typeof node === 'object' && node.props) { out.push(node); nodes(node.props.children, out); }
    return out;
  }
  const text = node => Array.isArray(node) ? node.map(text).join('') : node && typeof node === 'object' && node.props ? text(node.props.children) : node == null || typeof node === 'boolean' ? '' : String(node);
  const buttons = label => nodes(tree).filter(n => n.type === 'button' && text(n) === label);
  const button = label => { const matches = buttons(label); assert.ok(matches.length, `missing button ${label}`); return matches[0]; };
  const props = name => nodes(tree).find(n => n.type?.name === name)?.props;
  const render = () => { begin(); tree = component === App ? App() : DataAccessPanel(panelProps); flushEffects(); };
  async function pump() { for (let i = 0; i < 30; i++) { await Promise.resolve(); render(); } }
  async function click(label, index = 0) {
    const b = buttons(label)[index]; assert.ok(b, `missing button ${label}[${index}]`); assert.equal(!!b.props.disabled, false, `disabled ${label}`);
    const result = b.props.onClick(); await pump(); return result;
  }
  const calls = name => f.calls.filter(c => c.name === name);
  async function fixture(which = DataAccessPanel, overrides = {}, options = {}) {
    reset();
    const listeners = new Map();
    globalThis.document = { hidden: false, activeElement: null, addEventListener: (name, fn) => listeners.set(name, fn), removeEventListener: name => listeners.delete(name) };
    let timer = 0;
    globalThis.window = { setTimeout: () => ++timer, clearTimeout: () => {}, confirm: () => options.confirm ?? true };
    globalThis.requestAnimationFrame = () => ++timer;
    globalThis.cancelAnimationFrame = () => {};
    globalThis.prompt = () => null;
    globalThis.alert = () => {};
    f = { selected: null, calls: [], listeners, api: {}, insertions: [], closed: 0 };
    const defaults = {
      list_authorized_directories: async () => [dir()], list_imported_data_files: async () => [],
      choose_authorized_directory: async () => dir('new'), remove_authorized_directory: async () => {},
      authorized_directory_entries: async args => page(args.relativePath), import_authorized_file: async () => copy(), remove_imported_data_file: async () => {},
      get_remote_host: async () => f.selected, list_remote_hosts: async () => ['remote'], pi_bin_path: async () => '/fixture/pi', pi_version: async () => 'test',
      list_projects: async () => [{ key: 'project', cwd: '/synthetic' }], list_sessions: async () => [session('/sessionA'), session('/sessionB')],
      session_detail_page: async args => detail(args.path), list_running: async () => [], session_status: async () => 'finished',
      remote_sync_status: async args => ({ host: args.host, phase: 'ready', usableCache: true, lastSuccessAt: 1 }),
      set_remote_host: async args => { f.selected = args.host; }, refresh_remote: async () => {}, send_message: async () => {}, abort_message: async () => {},
    };
    f.api = { ...defaults, ...overrides };
    f.invoke = (name, args) => { f.calls.push({ name, args }); assert.ok(f.api[name], `unmocked IPC ${name}`); return f.api[name](args); };
    globalThis.fixture = f;
    component = which;
    panelProps = { onClose: () => f.closed++, onInsertDraft: options.allowInsert === false ? undefined : instruction => { f.insertions.push(instruction); return options.acceptInsert ?? true; }, targetName: 'Synthetic local session' };
    render(); await pump(); return f;
  }

  await fixture();
  assert.equal(calls('list_authorized_directories').length, 1);
  assert.equal(calls('list_imported_data_files').length, 1);
  assert.equal(calls('authorized_directory_entries').length, 0);
  assert.equal(calls('choose_authorized_directory').length, 0);
  assert.equal(calls('import_authorized_file').length, 0);
  assert.match(text(tree), /私人微信消息/);
  assert.match(text(tree), /不会传输或上传/);
  assert.match(text(tree), /不是任务沙箱/);
  assert.match(text(tree), /系统授权提示/);
  assert.match(text(tree), /系统级|操作系统/);
  assert.match(text(tree), /配置的模型/);
  assert.match(text(tree), /表示同意本应用保存所选目录的授权/);
  assert.match(text(tree), /security-scoped bookmark/);
  assert.match(text(tree), /系统可恢复时复用/);
  assert.match(text(tree), /恢复失败时显示错误，请重新选择/);
  assert.match(text(tree), /未启用 App Sandbox/);
  assert.match(text(tree), /重启后访问受保护微信目录是否免提示仍需验证/);
  assert.match(text(tree), /不是对 Python \/ Pi 直接访问的「始终允许」/);
  assert.match(text(tree), /已保存授权凭据 · 查看器读取入口/);
  assert.equal(button('选择目录并记住授权').props['aria-describedby'], 'data-access-remember-consent');
  assert.match(text(tree), /2023/, "data-access timestamps are epoch milliseconds, unlike session timestamps");
  await click('选择目录并记住授权');
  assert.equal(calls('choose_authorized_directory').length, 1);
  assert.equal(calls('authorized_directory_entries').length, 0);
  assert.match(text(tree), /\/synthetic\/protected\/new/);
  console.log('PASS explicit remembered-authorization consent before native chooser, broker-only reuse caveat, metadata-only open and privacy');
  const remembered = dir('new');
  await fixture(DataAccessPanel, { list_authorized_directories: async () => [remembered] });
  assert.match(text(tree), /Directory new/);
  assert.match(text(tree), /已保存授权凭据 · 查看器读取入口/);
  assert.equal(calls('choose_authorized_directory').length, 0, 'persisted directory does not reopen picker');
  await click('Browse / 浏览');
  assert.deepEqual(calls('authorized_directory_entries')[0].args, { id: 'new', relativePath: '', offset: 0 });
  assert.equal(calls('choose_authorized_directory').length, 0, 'remembered broker browsing does not reauthorize through picker');
  console.log('PASS persisted authorization receipt is reusable via explicit Browse without choosing again');

  await fixture(DataAccessPanel, { choose_authorized_directory: async () => null });
  await click('选择目录并记住授权');
  assert.match(text(tree), /已取消选择/);
  assert.equal(buttons('Browse / 浏览').length, 1);
  const chooser = deferred();
  await fixture(DataAccessPanel, { choose_authorized_directory: () => chooser.promise });
  const staleChoose = button('选择目录并记住授权').props.onClick;
  staleChoose(); staleChoose(); await pump();
  assert.equal(calls('choose_authorized_directory').length, 1);
  assert.equal(button('选择目录并记住授权').props.disabled, true);
  assert.equal(button('关闭 ×').props.disabled, undefined);
  chooser.reject(new Error('chooser failed')); await pump();
  assert.match(text(tree), /chooser failed/);
  assert.equal(button('选择目录并记住授权').props.disabled, false);
  console.log('PASS chooser cancellation, synchronous duplicate guard, pending dismissal and error recovery');

  await fixture(DataAccessPanel, { authorized_directory_entries: async args => {
    if (args.relativePath === 'folder') return page('folder', [entry('folder/nested.txt')]);
    return page('', args.offset === 200 ? [entry('page2.txt')] : [entry('folder', true), entry('message.txt')], args.offset === 0);
  } });
  await click('Browse / 浏览');
  assert.deepEqual(calls('authorized_directory_entries')[0].args, { id: 'dir1', relativePath: '', offset: 0 });
  await click('下一页');
  assert.deepEqual(calls('authorized_directory_entries').at(-1).args, { id: 'dir1', relativePath: '', offset: 200 });
  assert.match(text(tree), /page2.txt/); assert.doesNotMatch(text(tree), /文件 message.txt/);
  assert.equal(button('下一页').props.disabled, true);
  await click('上一页'); await click('打开文件夹');
  assert.equal(calls('authorized_directory_entries').at(-1).args.relativePath, 'folder');
  await click('上一级');
  assert.equal(calls('authorized_directory_entries').at(-1).args.relativePath, '');
  assert.equal(calls('import_authorized_file').length, 0);
  await click('仅复制此文件');
  assert.deepEqual(calls('import_authorized_file')[0].args, { id: 'dir1', relativePath: 'message.txt' });
  assert.match(text(tree), /副本路径：\/private\/app\/imports\/copy1.txt/);
  assert.match(text(tree), /123 bytes/);
  assert.match(text(tree), /来源路径：\/synthetic\/protected\/private-wechat.txt/);
  assert.equal(f.insertions.length, 0);
  await click('插入当前会话草稿');
  assert.equal(f.insertions.length, 1);
  assert.match(f.insertions[0], /\/private\/app\/imports\/copy1.txt/);
  assert.doesNotMatch(f.insertions[0], /\/synthetic\/protected/);
  assert.match(f.insertions[0], /不要读取原始 App/);
  assert.equal(calls('send_message').length, 0);
  console.log('PASS explicit single-level folders and bounded offset pages, exactly one file copy, full receipt and explicit staged-path-only draft');

  assert.equal(copiedFileInstruction({ ...copy(), path: '' }), null);
  assert.equal(copiedFileInstruction({ ...copy(), path: 'x'.repeat(4097) }), null);
  assert.equal(copiedFileInstruction({ ...copy(), path: '/bad\npath' }), null);
  assert.ok(copiedFileInstruction({ ...copy(), path: '/'+ 'x'.repeat(4095) }).length < 9000);
  await fixture(DataAccessPanel, { list_imported_data_files: async () => [copy()] }, { allowInsert: false });
  assert.equal(buttons('插入当前会话草稿').length, 0);
  await fixture(DataAccessPanel, { list_imported_data_files: async () => [copy()] }, { acceptInsert: false });
  await click('插入当前会话草稿');
  assert.match(text(tree), /会话或来源已改变/);
  console.log('PASS bounded path instruction, absent insertion affordance when ineligible, stale insertion rejection shown');

  await fixture(DataAccessPanel, { list_imported_data_files: async () => [copy()] }, { confirm: false });
  await click('删除副本…');
  assert.equal(calls('remove_imported_data_file').length, 0);
  assert.match(text(tree), /副本路径/);
  const removing = deferred();
  await fixture(DataAccessPanel, { list_imported_data_files: async () => [copy()], remove_imported_data_file: () => removing.promise });
  const staleDelete = button('删除副本…').props.onClick;
  staleDelete(); staleDelete(); await pump();
  assert.equal(calls('remove_imported_data_file').length, 1);
  removing.reject(new Error('delete failed')); await pump();
  assert.match(text(tree), /delete failed/); assert.match(text(tree), /副本路径/);
  f.api.remove_imported_data_file = async () => {};
  await click('删除副本…');
  assert.equal(buttons('删除副本…').length, 0);
  assert.match(text(tree), /原文件未修改/);
  console.log('PASS confirmed deletion only, duplicate guard, retained receipt on failure and copy-only delete');

  const firstBrowse = deferred();
  await fixture(DataAccessPanel, { list_authorized_directories: async () => [dir(), dir('dir2')], authorized_directory_entries: args => args.id === 'dir1' ? firstBrowse.promise : Promise.resolve(page('', [entry('fresh.txt')])) });
  button('Browse / 浏览').props.onClick(); await pump();
  await click('Browse / 浏览', 1);
  firstBrowse.resolve(page('', [entry('obsolete.txt')])); await pump();
  assert.match(text(tree), /fresh.txt/); assert.doesNotMatch(text(tree), /obsolete.txt/);
  console.log('PASS earlier browse response cannot overwrite selected directory');

  const pendingBrowse = deferred(), pendingRevoke = deferred();
  await fixture(DataAccessPanel, { list_imported_data_files: async () => [copy()], authorized_directory_entries: () => pendingBrowse.promise, remove_authorized_directory: () => pendingRevoke.promise });
  button('Browse / 浏览').props.onClick(); await pump();
  button('撤销目录授权').props.onClick(); await pump();
  pendingBrowse.resolve(page('', [entry('revoked-response.txt')])); await pump();
  assert.doesNotMatch(text(tree), /revoked-response.txt/);
  assert.equal(buttons('仅复制此文件').length, 0);
  pendingRevoke.resolve(); await pump();
  assert.equal(buttons('Browse / 浏览').length, 0);
  assert.match(text(tree), /副本路径/);
  assert.equal(calls('remove_imported_data_file').length, 0);
  console.log('PASS invalidation at revoke start suppresses pending browse; revocation preserves imported copies');

  const revokedFailure = deferred(), obsoleteFailureBrowse = deferred();
  await fixture(DataAccessPanel, { authorized_directory_entries: () => obsoleteFailureBrowse.promise, remove_authorized_directory: () => revokedFailure.promise });
  button('Browse / 浏览').props.onClick(); await pump();
  button('撤销目录授权').props.onClick(); await pump();
  revokedFailure.reject(new Error('revoke failed')); await pump();
  obsoleteFailureBrowse.resolve(page('', [entry('late-failed-revoke.txt')])); await pump();
  assert.match(text(tree), /revoke failed/); assert.doesNotMatch(text(tree), /late-failed-revoke.txt/);
  assert.equal(buttons('Browse / 浏览').length, 1);
  f.api.authorized_directory_entries = async () => page('', [entry('retry.txt')]);
  await click('Browse / 浏览'); assert.match(text(tree), /retry.txt/);
  console.log('PASS failed revoke retains authorization but cannot resurrect stale browse; explicit retry works');

  await fixture(DataAccessPanel, { authorized_directory_entries: async () => { throw new Error('browse failed'); } });
  await click('Browse / 浏览');
  assert.match(text(tree), /browse failed/);
  assert.equal(button('重新读取此页').props.disabled, false);
  f.api.authorized_directory_entries = async () => page();
  await click('重新读取此页'); assert.equal(buttons('仅复制此文件').length, 1);
  const lateMetadata = deferred();
  await fixture(DataAccessPanel, { list_authorized_directories: () => lateMetadata.promise, list_imported_data_files: async () => { throw new Error('receipts failed'); } });
  assert.equal(button('选择目录并记住授权').props.disabled, true);
  lateMetadata.resolve([dir()]); await pump();
  assert.match(text(tree), /receipts failed/); assert.equal(button('选择目录并记住授权').props.disabled, false);
  f.api.list_imported_data_files = async () => [copy()];
  await click('刷新记录 / 重试'); assert.match(text(tree), /副本路径/);
  console.log('PASS directory/metadata error recovery and no mutation while initial receipts pending');

  // Reset invokes the real effect cleanup. Remount has fresh hook cells; old
  // promises must update neither old instance nor the newly mounted instance.
  const unmountedBrowse = deferred();
  await fixture(DataAccessPanel, { authorized_directory_entries: () => unmountedBrowse.promise });
  button('Browse / 浏览').props.onClick(); await pump();
  await fixture();
  unmountedBrowse.resolve(page('', [entry('unmounted.txt')])); await pump();
  assert.doesNotMatch(text(tree), /unmounted.txt/);
  const unmountedChoose = deferred();
  await fixture(DataAccessPanel, { choose_authorized_directory: () => unmountedChoose.promise });
  button('选择目录并记住授权').props.onClick(); await pump();
  await fixture();
  unmountedChoose.resolve(dir('unmounted')); await pump();
  assert.doesNotMatch(text(tree), /Directory unmounted/);
  f.listeners.get('keydown')({ key: 'Escape', preventDefault() {} });
  assert.equal(f.closed, 1);
  console.log('PASS unmounted async browse/chooser cannot repopulate remount, Escape dismissal');

  // Integration exercises the real App, sequencer, api wrappers and actual
  // selected-session drafts; children are inert elements inspected by props.
  // Sidebar is mocked here: exercise its callback contract, not an App button.
  const assertSidebarLauncher = () => {
    assert.equal(typeof props('Sidebar').onOpenDataAccess, 'function');
    assert.equal(typeof props('Sidebar').showDataAccess, 'boolean');
    assert.equal(buttons('Data access / 数据目录').length, 0, 'no App text launcher');
    assert.equal(nodes(tree).some(n => /(?:^|\s)(?:main-toolbar|data-access-open)(?:\s|$)/.test(n.props.className ?? '')), false, 'no main-toolbar/data-access-open');
  };
  async function openDataAccess() {
    assertSidebarLauncher();
    const open = props('Sidebar').onOpenDataAccess;
    open(); await pump();
    assert.equal(props('Sidebar').onOpenDataAccess, open, 'stable Sidebar open callback');
    assert.equal(props('Sidebar').showDataAccess, true);
    assert.ok(props('DataAccessPanel'));
  }
  await fixture(App);
  assertSidebarLauncher();
  assert.equal(props('Sidebar').showDataAccess, false);
  assert.equal(props('DataAccessPanel'), undefined);
  assert.equal(props('Composer').value, '');
  const initialOpen = props('Sidebar').onOpenDataAccess;
  assert.equal(calls('list_authorized_directories').length, 0);
  props('Sidebar').onOpenConfig(); await pump();
  assert.equal(props('Sidebar').onOpenDataAccess, initialOpen, 'Config does not replace the open callback');
  assert.equal(props('Sidebar').showConfig, true);
  await openDataAccess();
  assert.equal(props('Sidebar').showConfig, true, 'opening data access does not toggle Config');
  assert.equal(typeof props('DataAccessPanel').onInsertDraft, 'function', 'available independently of Config');
  await openDataAccess(); // Open is idempotent, not a toggle.
  props('DataAccessPanel').onClose(); await pump();
  assert.equal(props('Sidebar').showDataAccess, false);
  assert.equal(props('DataAccessPanel'), undefined);
  assert.equal(props('Sidebar').showConfig, true, 'closing data access leaves Config alone');
  props('Sidebar').onOpenConfig(); await pump();
  assert.equal(props('Composer').value, '', 'opening/closing never inserts a draft');
  assert.equal(calls('send_message').length, 0, 'opening/closing never sends a message');
  props('Sidebar').onOpenConfig(); await pump();
  await openDataAccess();
  const originalInsert = props('DataAccessPanel').onInsertDraft;
  const instruction = copiedFileInstruction(copy());
  assert.equal(originalInsert(instruction), true); await pump();
  props('Sidebar').onOpenConfig(); await pump();
  assert.match(props('Composer').value, /\/private\/app\/imports\/copy1.txt/);
  props('Composer').onChange('Existing unsent draft'); await pump();
  assert.equal(props('DataAccessPanel').onInsertDraft(instruction), true); await pump();
  assert.equal(props('Composer').value, 'Existing unsent draft\n\n'+instruction);
  assert.equal(calls('send_message').length, 0);
  const sessionAInsert = props('DataAccessPanel').onInsertDraft;
  props('Sidebar').onSelectSession(session('/sessionB')); await pump();
  assert.equal(sessionAInsert(instruction), false);
  assert.equal(props('Composer').value, '');
  props('Sidebar').onSelectSession(session('/sessionA')); await pump();
  assert.equal(sessionAInsert(instruction), false, 'A->B->A rejects first A');
  assert.equal(props('Composer').value, 'Existing unsent draft\n\n'+instruction);
  console.log('PASS stable Sidebar launcher/flag, no main toolbar, Config-independent panel lifecycle, no automatic draft/send, explicit insertion and session epoch isolation A-B-A');

  const localInsert = props('DataAccessPanel').onInsertDraft;
  const switching = props('Sidebar').onSwitchSource('remote');
  assert.equal(localInsert(instruction), false, 'source ref invalidates synchronously before render');
  await switching; await pump();
  assert.equal(props('DataAccessPanel').onInsertDraft, undefined);
  assert.equal(props('Sidebar').onOpenDataAccess, initialOpen, 'launcher remains stable remotely');
  props('DataAccessPanel').onClose(); await pump();
  assert.equal(props('Sidebar').showDataAccess, false);
  await openDataAccess();
  assert.equal(props('DataAccessPanel').onInsertDraft, undefined, 'remote panel remains available without draft insertion');
  assert.equal(calls('send_message').length, 0, 'remote opening never sends');
  assert.equal(props('Thread').detail.path, '/sessionA', 'remote fixture intentionally has same path');
  assert.equal(props('Composer').value, '', 'remote same-path draft must not inherit local copy path');
  props('Composer').onChange('Separate remote draft'); await pump();
  await props('Sidebar').onSwitchSource(null); await pump();
  assert.equal(localInsert(instruction), false, 'local->remote->local rejects first local');
  assert.equal(typeof props('DataAccessPanel').onInsertDraft, 'function');
  assert.equal(props('Composer').value, 'Existing unsent draft\n\n'+instruction, 'local draft survives switching, without remote draft replacing it');
  console.log('PASS remote and pending sources disallow draft insertion; source epochs and same-path draft privacy isolation');

  const idleInsert = props('DataAccessPanel').onInsertDraft;
  const send = props('Composer').onSend('Explicit normal message');
  assert.equal(idleInsert(instruction), false, 'sending sets running guard synchronously');
  await send; await pump();
  assert.equal(props('DataAccessPanel').onInsertDraft, undefined);
  let externalRunning = true;
  await fixture(App, { list_sessions: async () => [session('/sessionA', externalRunning)] });
  await openDataAccess();
  assert.equal(props('DataAccessPanel').onInsertDraft, undefined);
  externalRunning = false;
  props('Sidebar').onRefresh(); await pump();
  assert.equal(typeof props('DataAccessPanel').onInsertDraft, 'function', 'external completion releases idle gating');
  assert.equal(props('Composer').running, false);
  await fixture(App, { list_projects: async () => [] });
  await openDataAccess();
  assert.equal(props('DataAccessPanel').onInsertDraft, undefined);
  console.log('PASS local and externally running sessions, no selected detail, and synchronous send all deny insertion');

  console.log('All data-access frontend tests passed (offline synthetic data; native chooser/OS grants not exercised).');
} finally {
  await rm(scratch, { recursive: true, force: true });
}
