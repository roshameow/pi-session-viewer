// Offline mock-only AGY adaptation regression runner.
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdtemp, rm } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import path from 'node:path';

const root = path.resolve(new URL('..', import.meta.url).pathname);
const scratch = await mkdtemp(path.join(root, 'node_modules/.agy-adaptation-'));

try {
  const outfile = path.join(scratch, 'components.mjs');
  await build({
    stdin: {
      contents: `export * from './src/components/Thread'; export * from './src/components/Sidebar';`,
      resolveDir: root,
      loader: 'tsx',
    },
    bundle: true,
    platform: 'node',
    format: 'esm',
    outfile,
    external: ['react', 'react-dom', 'react-dom/server'],
    logLevel: 'silent',
  });

  const { Thread, Sidebar } = await import(pathToFileURL(outfile));
  globalThis.localStorage = { getItem: () => null, setItem: () => {} };
  const React = await import('react');
  const { renderToStaticMarkup } = await import('react-dom/server');
  const render = (component, props) => renderToStaticMarkup(React.createElement(component, props));

  // 1. Sidebar SessionItem with AGY engine chip
  const agyMain = {
    id: 'agy-main-uuid',
    path: '/path/brain/agy-main-uuid/.system_generated/logs/transcript.jsonl',
    cwd: '/Users/wenliu/Code/python/quantnight',
    name: 'QuantNight AGY Session',
    firstMessage: 'Fix architecture boundaries',
    lastMessage: null,
    createdIso: '2026-11-06T18:00:00Z',
    createdAt: 1793988000,
    updatedAt: 1793988000,
    model: 'gemini',
    isSubagent: false,
    taskId: null,
    parentSessionId: null,
    parentSessionPath: null,
    messageCount: 12,
    running: true,
    sleeping: false,
    interrupted: false,
    inRmux: false,
    rmuxTarget: null,
    rmuxAttached: false,
    rmuxDead: false,
    termAlive: false,
    size: 2048,
    engine: 'agy',
  };

  const agySub = {
    id: 'agy-sub-uuid',
    path: '/path/brain/agy-sub-uuid/.system_generated/logs/transcript.jsonl',
    cwd: '/Users/wenliu/Code/python/quantnight',
    name: 'Research Subagent',
    firstMessage: 'Analyze dataset',
    lastMessage: null,
    createdIso: '2026-11-06T18:05:00Z',
    createdAt: 1793988300,
    updatedAt: 1793988300,
    model: 'gemini',
    isSubagent: true,
    taskId: 'research',
    parentSessionId: 'agy-main-uuid',
    parentSessionPath: '/path/brain/agy-main-uuid/.system_generated/logs/transcript.jsonl',
    messageCount: 6,
    running: false,
    sleeping: false,
    interrupted: false,
    inRmux: false,
    rmuxTarget: null,
    rmuxAttached: false,
    rmuxDead: false,
    termAlive: false,
    size: 1024,
    engine: 'agy',
  };

  const sidebarHtml = render(Sidebar, {
    projects: [{
      key: '--Users-wenliu-Code-python-quantnight--',
      cwd: '/Users/wenliu/Code/python/quantnight',
      sessionCount: 2,
      subagentCount: 1,
      runningCount: 1,
      rmuxCount: 0,
      termCount: 0,
      updatedAt: 1793988300,
    }],
    selectedProject: '--Users-wenliu-Code-python-quantnight--',
    sessions: [agyMain, agySub],
    selectedSessionPath: agyMain.path,
    finishedAt: {},
    remoteHosts: [],
    onSelectProject: () => {},
    onSelectSession: () => {},
  });

  assert.match(sidebarHtml, /engine-chip agy/);
  assert.match(sidebarHtml, /QuantNight AGY Session/);
  console.log('PASS AGY engine chip and session list rendering in Sidebar');

  // 2. Thread rendering of AGY entries (thinking, toolCall + result pairing, user text)
  const detail = {
    id: 'agy-main-uuid',
    cwd: '/Users/wenliu/Code/python/quantnight',
    path: agyMain.path,
    createdIso: '2026-11-06T18:00:00Z',
    taskId: null,
    stats: {
      tokenCount: 45000,
      messageCount: 3,
      model: 'gemini',
      provider: 'google',
      thinkingLevel: null,
      contextTokens: 12000,
      contextLimit: null,
      costTotal: 0,
    },
    entries: [
      {
        kind: 'message',
        id: '0',
        parentId: null,
        ts: '2026-11-06T18:00:00Z',
        role: 'user',
        content: [{ kind: 'text', text: 'Analyze the architecture document.' }],
        model: null,
        toolName: null,
        toolCallId: null,
        isError: null,
        summary: null,
        name: null,
        label: null,
      },
      {
        kind: 'message',
        id: '1',
        parentId: '0',
        ts: '2026-11-06T18:00:05Z',
        role: 'assistant',
        content: [
          { kind: 'thinking', thinking: 'Analyzing the request and reading files...' },
          { kind: 'toolCall', id: 'call_1_0', name: 'read_file', arguments: '{"Path":"ARCHITECTURE.md"}' },
          { kind: 'text', text: 'I have read the architecture file.' }
        ],
        model: 'gemini',
        toolName: null,
        toolCallId: null,
        isError: null,
        summary: null,
        name: null,
        label: null,
      },
      {
        kind: 'message',
        id: '2',
        parentId: '1',
        ts: '2026-11-06T18:00:08Z',
        role: 'toolResult',
        content: [{ kind: 'text', text: '# Architecture Overview\nAll components adhere to boundaries.' }],
        model: null,
        toolName: 'read_file',
        toolCallId: 'call_1_0',
        isError: false,
        summary: null,
        name: null,
        label: null,
      }
    ],
    active: [0, 1, 2],
    size: 2048,
    updatedAt: 1793988008,
  };

  const threadHtml = render(Thread, {
    detail,
    liveBlocks: [],
    running: false,
    preview: true,
  });

  assert.match(threadHtml, /Analyze the architecture document\./);
  assert.match(threadHtml, /Thinking…/);
  assert.match(threadHtml, /42 chars/);
  assert.match(threadHtml, /read_file\(Path=ARCHITECTURE\.md\)/);
  assert.match(threadHtml, /I have read the architecture file\./);
  console.log('PASS AGY Thread thinking, paired tool row, and user message rendering');

  const agyFullHtml = render(Thread, {
    detail: { ...detail, engine: 'agy' },
    liveBlocks: [],
    running: false,
    preview: false,
  });
  assert.match(agyFullHtml, /Open agy TUI/);
  assert.doesNotMatch(agyFullHtml, /Export HTML/);

  const piFullHtml = render(Thread, {
    detail: { ...detail, engine: null },
    liveBlocks: [],
    running: false,
    preview: false,
  });
  assert.match(piFullHtml, /Open TUI/);
  assert.match(piFullHtml, /Export HTML/);
  console.log('PASS AGY vs Pi TUI and Export actions in Thread header');

  console.log('ALL AGY adaptation tests passed!');
} finally {
  await rm(scratch, { recursive: true, force: true }).catch(() => {});
}
