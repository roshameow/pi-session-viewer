import React, { useEffect, useMemo, useRef, useState } from "react";
import type { ContentBlock, Entry, SessionDetail, DetailPageRequest, EntryBodyChunk } from "../types";
import { MemoMarkdown } from "./Markdown";
import { buildThreadItems } from "../threadItems";
import { api } from "../api";

// ---------- Live conversation blocks (assembled from pi json events) ----------

export type LiveBlock =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; thinking: string; done: boolean }
  | { kind: "tool"; toolCallId?: string; parentToolCallId?: string; name: string; args: string; result: string; isError: boolean; done: boolean };

export function appendLiveEvents(previous: LiveBlock[], events: any[]): LiveBlock[] {
  // Process only the newly arrived events. Rebuilding from the complete event
  // history on every token made a long response O(n²) and retained every raw
  // event for the duration of the turn.
  const blocks = previous.slice();
  const finishLast = () => {
    const last = blocks[blocks.length - 1];
    if (last?.kind === "assistant" || last?.kind === "tool") {
      blocks[blocks.length - 1] = { ...last, done: true };
    }
  };

  const toolIndex = (ev: any) => ev.toolCallId
    ? blocks.findIndex((b) => b.kind === "tool" && b.toolCallId === ev.toolCallId)
    : blocks.length - 1; // compatibility with old streams without ids

  for (const ev of events) {
    switch (ev.type) {
      case "message_start": {
        const m = ev.message;
        finishLast();
        if (m?.role === "user") {
          const text = contentText(m.content);
          if (text) blocks.push({ kind: "user", text });
        } else if (m?.role === "assistant") {
          blocks.push({ kind: "assistant", text: "", thinking: "", done: false });
        }
        break;
      }
      case "message_update": {
        const ae = ev.assistantMessageEvent;
        const last = blocks[blocks.length - 1];
        if (!ae || last?.kind !== "assistant") break;
        if (ae.type === "text_delta" && typeof ae.delta === "string") {
          blocks[blocks.length - 1] = { ...last, text: last.text + ae.delta };
        } else if (ae.type === "thinking_delta" && typeof ae.delta === "string") {
          blocks[blocks.length - 1] = { ...last, thinking: last.thinking + ae.delta };
        }
        break;
      }
      case "message_end": {
        const m = ev.message;
        const last = blocks[blocks.length - 1];
        if (m?.role === "assistant" && last?.kind === "assistant") {
          const text = contentText(m.content);
          blocks[blocks.length - 1] = { ...last, text: text || last.text, done: true };
        }
        break;
      }
      case "tool_execution_start": {
        blocks.push({
          kind: "tool",
          toolCallId: ev.toolCallId,
          parentToolCallId: ev.parentToolCallId,
          name: ev.toolName ?? "tool",
          args: stringify(ev.args),
          result: "",
          isError: false,
          done: false,
        });
        break;
      }
      case "tool_execution_update": {
        const index = toolIndex(ev);
        const last = blocks[index];
        if (last?.kind === "tool") {
          const result = stringify(ev.partialResult);
          if (result) blocks[index] = { ...last, result };
        }
        break;
      }
      case "tool_execution_end": {
        const index = toolIndex(ev);
        const last = blocks[index];
        if (last?.kind === "tool") {
          blocks[index] = {
            ...last,
            result: stringify(ev.result),
            isError: !!ev.isError,
            done: true,
          };
        }
        break;
      }
      default:
        break;
    }
  }
  return blocks;
}

export function buildLiveBlocks(events: any[]): LiveBlock[] {
  return appendLiveEvents([], events);
}

function stringify(v: unknown): string {
  if (v === undefined || v === null) return "";
  if (typeof v === "string") return v;
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

export function contentText(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((c) =>
        typeof c === "string" ? c : c?.type === "text" ? c.text : c?.type === "thinking" ? c.thinking : ""
      )
      .join(" ")
      .trim();
  }
  return "";
}

// ---------- compact tool summary (minimal-mode style) ----------

function argLine(name: string, args: string): string {
  // A collapsed giant call must not JSON.parse / stringify megabytes. Its
  // complete arguments remain available when the user expands the row.
  if (args.length > 4096) return args.slice(0, 80) + "… (expand for full arguments)";
  // args is a JSON string from Rust; try to render compactly
  try {
    const o = JSON.parse(args);
    if (typeof o === "string") return o;
    if (o && typeof o === "object") {
      const parts: string[] = [];
      for (const [k, v] of Object.entries(o)) {
        const s = typeof v === "string" ? v : JSON.stringify(v);
        parts.push(`${k}=${s.length > 60 ? s.slice(0, 60) + "…" : s}`);
      }
      return parts.join(" ");
    }
    return String(o);
  } catch {
    return args.length > 80 ? args.slice(0, 80) + "…" : args;
  }
}

function lineCount(s: string): number {
  let n = 1;
  for (let i = 0; i < s.length; i++) if (s.charCodeAt(i) === 10) n++;
  return s.endsWith("\n") ? Math.max(0, n - 1) : n;
}

function toolSummary(name: string, args: string, output: string, isError: boolean): string {
  const arg = argLine(name, args);
  const lines = lineCount(output);
  switch (name) {
    case "read":
      return `Read(${arg}) → ${lines} lines`;
    case "write":
      return `Write(${arg}) → ${lines} lines`;
    case "edit":
      return `Edit(${arg}) → ${lines} lines`;
    case "bash":
      return `$ ${arg}` + (isError ? " → failed" : lines > 1 ? ` → ${lines} lines` : "");
    case "ls":
      return `Ls(${arg}) → ${lines} entries`;
    case "find":
      return `Find(${arg}) → ${lines} files`;
    case "grep":
      return `Grep(${arg}) → ${lines} matches`;
    default:
      return `${name}(${arg})` + (isError ? " → error" : lines > 1 ? ` → ${lines} lines` : "");
  }
}

// ---------- single-line tool row (click to expand output) ----------

export type FilterMode = "default" | "no-tools" | "user-only" | "labeled-only" | "all";

export const FILTER_CYCLE: FilterMode[] = ["default", "no-tools", "user-only", "labeled-only", "all"];

export const FILTER_LABELS: Record<FilterMode, string> = {
  default: "Default",
  "no-tools": "No tools",
  "user-only": "User only",
  "labeled-only": "Labeled",
  all: "All",
};

function ToolRow({
  name,
  arg,
  output,
  isError,
  running,
  defaultOpen,
  hideResult,
}: {
  name: string;
  arg: string;
  output: string;
  isError: boolean;
  running?: boolean;
  defaultOpen?: boolean;
  hideResult?: boolean;
}) {
  const [open, setOpen] = useState(!!defaultOpen);
  const summary = useMemo(() => toolSummary(name, arg, output, isError), [name, arg, output, isError]);
  const outputLen = output.length;
  return (
    <div className={`tool-row ${isError ? "err" : ""}`}>
      <button
        className="tool-line"
        onClick={() => setOpen(!open)}
        title={output && !hideResult ? "Click to expand/collapse output" : undefined}
      >
        <span className={`tdot ${isError ? "red" : running ? "pulse" : "green"}`}>●</span>
        <span className="tool-summary">{hideResult ? callOnly(name, arg) : summary}</span>
        {running && <span className="tool-running">running…</span>}
        {!running && !hideResult && outputLen > 0 && (
          <span className="tool-expand">{open ? "▾" : "▸"} {open ? "Collapse" : "Expand"}</span>
        )}
        {hideResult && <span className="tool-expand muted">(output hidden)</span>}
      </button>
      {open && arg && <pre className="tool-output">{name}
{arg}</pre>}
      {open && output && !hideResult && (
        <pre className="tool-output" onClick={(e) => e.stopPropagation()}>
          {output}
        </pre>
      )}
    </div>
  );
}

function callOnly(name: string, args: string): string {
  const arg = argLine(name, args);
  if (name === "bash") return `$ ${arg}`;
  return `${name}(${arg})`;
}

// ---------- thread ----------

export function Thread({
  detail,
  liveBlocks,
  running,
  preview = false,
  pageBusy = false,
  pageNotice,
  pageRequest,
  onQuery,
  onLoadEarlier,
  onLoadEntry,
  onBodyChunk,
}: {
  detail: SessionDetail;
  liveBlocks: LiveBlock[];
  running: boolean;
  preview?: boolean;
  pageBusy?: boolean;
  pageNotice?: string | null;
  pageRequest?: DetailPageRequest;
  onQuery?: (request: DetailPageRequest) => Promise<void>;
  onLoadEarlier?: () => Promise<void>;
  onLoadEntry?: (entryId: string) => Promise<void>;
  onBodyChunk?: (generation: string, entryId: string, offset: number, recordOffset?: number) => Promise<EntryBodyChunk>;
}) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [filter, setFilter] = useState<FilterMode>("default");
  // The native path loads bounded server pages. A local/demo full detail also
  // keeps a DOM window; additional server pages are fetched only on demand.
  const [windowSize, setWindowSize] = useState(150);
  const scrollRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef(0);
  const prevScrollHeight = useRef(0);
  const [expanding, setExpanding] = useState(false);
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [branchLeaf, setBranchLeaf] = useState("");
  const lastPageRequest = useRef(pageRequest);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedSearch(search), 300);
    return () => window.clearTimeout(timer);
  }, [search]);
  useEffect(() => {
    if (!pageRequest) return;
    setSearch(pageRequest.query ?? "");
    setDebouncedSearch(pageRequest.query ?? "");
    setFilter(pageRequest.filter ?? "default");
    setBranchLeaf(pageRequest.branchLeafId ?? "");
  }, [pageRequest?.query, pageRequest?.filter, pageRequest?.branchLeafId, pageRequest?.entryId]);
  useEffect(() => {
    // External entry/branch navigation synchronizes the input above. Do not
    // launch a request with this render's old input before those setters land.
    if (lastPageRequest.current !== pageRequest) {
      lastPageRequest.current = pageRequest;
      return;
    }
    if (!detail.page || !onQuery) return;
    if (debouncedSearch === (pageRequest?.query ?? "") && filter === (pageRequest?.filter ?? "default")) return;
    void onQuery({ query: debouncedSearch, filter, branchLeafId: pageRequest?.branchLeafId });
  }, [debouncedSearch, filter, detail.page != null, onQuery, pageRequest]);
  const [exporting, setExporting] = useState(false);
  const [exportMsg, setExportMsg] = useState<string | null>(null);

  const doExport = async () => {
    setExporting(true);
    setExportMsg(null);
    try {
      const out = await api.exportSession(detail.path);
      setExportMsg("✓ Exported → " + out);
      setTimeout(() => setExportMsg(null), 6000);
    } catch (e) {
      setExportMsg("✗ Export failed: " + String(e));
      setTimeout(() => setExportMsg(null), 6000);
    } finally {
      setExporting(false);
    }
  };

  const doOpenTerminal = async () => {
    try {
      const msg = await api.openInTerminal(detail.path);
      setExportMsg("✓ " + msg);
      setTimeout(() => setExportMsg(null), 8000);
    } catch (e) {
      setExportMsg("✗ Failed to open terminal: " + String(e));
      setTimeout(() => setExportMsg(null), 6000);
    }
  };

  const doAttach = async () => {
    try {
      const msg = await api.attachSession(detail.path);
      setExportMsg("✓ " + msg);
      setTimeout(() => setExportMsg(null), 6000);
    } catch (e) {
      setExportMsg("✗ Failed to attach: " + String(e));
      setTimeout(() => setExportMsg(null), 6000);
    }
  };

  const [copied, setCopied] = useState(false);
  const copySessionId = async () => {
    const text = detail.id;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // fallback for non-secure contexts
      const ta = document.createElement("textarea");
      ta.value = text;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  // Ctrl+O cycles filter modes; Ctrl+F focuses the search box (like pi /tree)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        setFilter((f) => FILTER_CYCLE[(FILTER_CYCLE.indexOf(f) + 1) % FILTER_CYCLE.length]);
      } else if (e.ctrlKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const renderItems = useMemo(() => buildThreadItems(detail, filter, debouncedSearch), [detail, filter, debouncedSearch]);

  const offPagePeers = useMemo(() => {
    const ids = new Set(detail.entries.map(e => e.id));
    const peers = new Map<string, { id: string; role: string }[]>();
    for (const p of detail.page?.toolPairs ?? []) {
      for (const [id, peer, role] of [[p.callEntryId, p.resultEntryId, "result"], [p.resultEntryId, p.callEntryId, "call"]]) {
        if (ids.has(id) && !ids.has(peer)) {
          const list = peers.get(id) ?? [];
          list.push({ id: peer, role }); peers.set(id, list);
        }
      }
    }
    return peers;
  }, [detail]);

  // only the tail of the rendered items is mounted; expand on demand
  const visibleItems = useMemo(() => {
    const vis = renderItems.items;
    if (windowSize >= vis.length) return vis;
    return vis.slice(vis.length - windowSize);
  }, [renderItems, windowSize]);

  const loadEarlier = async () => {
    const el = scrollRef.current;
    if (el) prevScrollHeight.current = el.scrollHeight;
    setAutoScroll(false);
    if (visibleItems.length < renderItems.items.length) setWindowSize(w => w + 300);
    else if (detail.page?.hasMore && onLoadEarlier) {
      await onLoadEarlier();
      setWindowSize(w => w + 200);
    }
    setExpanding(true);
  };
  // keep the viewport anchored when older messages are prepended above
  useEffect(() => {
    if (expanding && scrollRef.current) {
      const el = scrollRef.current;
      el.scrollTop += el.scrollHeight - prevScrollHeight.current;
      setExpanding(false);
    }
  }, [windowSize, expanding, detail]);

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      // Re-starting smooth scrolling for every streamed token keeps WebKit's
      // compositor busy continuously. A direct tail lock is both cheaper and
      // more predictable while output is streaming.
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [renderItems.items.length, liveBlocks, autoScroll]);

  useEffect(() => () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
  }, []);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    // 每帧 setState 会触发 Thread 重渲染;rAF 节流到渲染帧
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = 0;
      setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 200);
    });
  };

  return (
    <div className="thread">
      <div className="thread-top">
        <div className="thread-top-inner">
          <div className="session-head">
            <div className="session-head-row">
              <div className="session-head-main">
                <div className="session-head-title">
                  <span className="session-provider">{detail.stats.provider ? `(${detail.stats.provider}) ` : ""}</span>
                  {detail.stats.model ?? "pi session"}
                </div>
                <div className="session-head-meta">
                  {detail.stats.messageCount} messages · {fmtTokens(detail.stats.tokenCount)} · $
                  {detail.stats.costTotal.toFixed(4)} · {detail.cwd}
                </div>
                <div className="session-head-meta">
                  {detail.stats.contextTokens != null && detail.stats.contextLimit != null && (
                    <span
                      className={`ctx-badge ${
                        (detail.stats.contextTokens / detail.stats.contextLimit) * 100 > 70 ? "warn" : ""
                      }`}
                      title="context usage"
                    >
                      ctx {((detail.stats.contextTokens / detail.stats.contextLimit) * 100).toFixed(1)}%
                      / {fmtContext(detail.stats.contextLimit)} (auto)
                    </span>
                  )}
                  {detail.stats.thinkingLevel && (
                    <span className="thinking-badge">thinking: {detail.stats.thinkingLevel}</span>
                  )}
                </div>
                <div className="session-head-id" title="Click to copy session id">
                  <span className="session-id-label">id:</span>
                  <span className="session-id-value">{detail.id}</span>
                  <button className="session-id-copy" onClick={copySessionId}>
                    {copied ? "✓ copied" : "copy"}
                  </button>
                  {detail.taskId && (
                    <span className="session-task-inline" title="subagent task id">
                      task: {detail.taskId}
                    </span>
                  )}
                </div>
              </div>
              {!preview && <div className="session-head-actions">
                <button
                  className="export-btn"
                  onClick={doAttach}
                  title="Attach to this session's rmux session (detach: Ctrl+G)"
                >
                  ⇄ Attach
                </button>
                <button
                  className="export-btn"
                  onClick={doOpenTerminal}
                  title="Open this session in pi TUI (rmux-backed, closable tab)"
                >
                  ⛭ Open TUI
                </button>
                <button
                  className={`export-btn ${exporting ? "busy" : ""}`}
                  onClick={doExport}
                  disabled={exporting}
                  title="Export this session to HTML (pi --export) and open it"
                >
                  {exporting ? "Exporting…" : "⬇ Export HTML"}
                </button>
              </div>}
            </div>
            {exportMsg && <div className="export-msg">{exportMsg}</div>}
          </div>
          <div className="thread-search-row">
            <input
              ref={searchRef}
              className="thread-search"
              placeholder="Search messages… (Ctrl+F)"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setSearch("");
                  (e.target as HTMLInputElement).blur();
                }
              }}
            />
            {search.trim() ? (
              <>
                <span className="thread-search-count">{search !== debouncedSearch ? "Waiting for search…" : pageBusy ? "Updating full-history results…" : `${renderItems.matchCount} matches (full branch)`}</span>
                <button className="clear-btn" onClick={() => setSearch("")} title="Clear search">
                  ×
                </button>
              </>
            ) : null}
          </div>
          {detail.page && <div className="session-head-meta" role="status">
            {detail.active.length} loaded / {detail.page.matchedEntries} matching entries · {detail.page.branchEntries} branch entries / {detail.page.totalEntries} file entries
            <span> · {detail.page.counters.user} user · {detail.page.counters.assistant} assistant · {detail.page.counters.toolResult} tool results · {detail.page.counters.labeled} labeled</span>
            {detail.page.incompleteTail && <span> · Incomplete trailing record — awaiting file update</span>}
            {detail.page.malformedLines > 0 && <span> · {detail.page.malformedLines} malformed lines skipped</span>}
            {pageNotice && <div>{pageNotice}</div>}
            <button disabled={pageBusy} onClick={() => void onQuery?.({ query: search, filter, branchLeafId: pageRequest?.branchLeafId })}>Latest</button>
            <input aria-label="Branch leaf entry ID" placeholder="Branch leaf entry ID (optional)" value={branchLeaf} onChange={e => setBranchLeaf(e.target.value)} />
            <button disabled={pageBusy} onClick={() => void onQuery?.({ query: search, filter, branchLeafId: branchLeaf || undefined })}>View branch</button>
          </div>}
          <div className="filter-bar" title="Ctrl+O cycles modes">
            {FILTER_CYCLE.map((m) => (
              <button
                key={m}
                className={`filter-btn ${filter === m ? "active" : ""}`}
                onClick={() => setFilter(m)}
              >
                {FILTER_LABELS[m]}
              </button>
            ))}
          </div>
        </div>
      </div>
      <div className="thread-scroll" onScroll={onScroll} ref={scrollRef}>
        <div className="thread-inner">
          {(renderItems.items.length > visibleItems.length || detail.page?.hasMore) && (
            <button className="load-earlier" disabled={pageBusy} onClick={() => void loadEarlier()}>
              {pageBusy ? "Loading…" : "↑ Load earlier messages"}
            </button>
          )}
          {visibleItems.map(({ entry, inlineResults }) => (
            <div key={entry.id} data-entry-id={entry.id}>
              <MemoEntryView entry={entry} inlineResults={inlineResults} hideToolOutput={renderItems.hideToolOutput} />
              {entry.bodyRef?.preview && <FullBodyView key={entry.bodyRef.generation + ":" + entry.id} entry={entry} onBodyChunk={onBodyChunk} />}
              {offPagePeers.get(entry.id)?.map(peer => <button key={peer.id} className="load-earlier" disabled={pageBusy}
                onClick={() => void onLoadEntry?.(peer.id)}>Load related tool {peer.role} · {peer.id}</button>)}
            </div>
          ))}

        {/* live conversation */}
        {liveBlocks.map((b, i) => {
          if (b.kind === "user")
            return (
              <div key={"live" + i} className="msg user">
                <div className="msg-text">{b.text}</div>
              </div>
            );
          if (b.kind === "assistant")
            return (
              <div key={"live" + i} className="msg assistant">
                {b.thinking && <ThinkingLine text={b.thinking} />}
                {b.text ? (
                  <div className="msg-text">
                    <MemoMarkdown text={b.text} />
                    {!b.done && <span className="cursor" />}
                  </div>
                ) : (
                  !b.done && <div className="thinking-dots"><span/><span/><span/></div>
                )}
              </div>
            );
          return (
            <div key={"live" + i} className="msg tool">
              <ToolRow
                name={b.name}
                arg={b.args}
                output={b.result}
                isError={b.isError}
                running={!b.done}
                defaultOpen={b.isError}
              />
            </div>
          );
        })}

        {running && <div className="running-bar">⏳ pi is working…</div>}
        <div ref={bottomRef} />
        </div>
      </div>
    </div>
  );
}

export function FullBodyView({ entry, onBodyChunk }: {
  entry: Entry;
  onBodyChunk?: (generation: string, entryId: string, offset: number, recordOffset?: number) => Promise<EntryBodyChunk>;
}) {
  const [raw, setRaw] = useState<string | null>(null);
  const [progress, setProgress] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [download, setDownload] = useState<string | null>(null);
  const alive = useRef(true);
  const run = useRef(0);
  useEffect(() => { alive.current = true; return () => { alive.current = false; run.current++; }; }, []);
  useEffect(() => {
    if (raw === null) { setDownload(null); return; }
    const url = URL.createObjectURL(new Blob([raw], { type: "application/json" }));
    setDownload(url);
    return () => URL.revokeObjectURL(url);
  }, [raw]);
  const load = async () => {
    const ref = entry.bodyRef;
    if (!ref || !onBodyChunk || busy) return;
    const currentRun = ++run.current;
    const current = () => alive.current && currentRun === run.current;
    setBusy(true); setError(null); setProgress(0);
    const chunks: string[] = [];
    let offset = 0;
    try {
      while (current()) {
        const chunk = await onBodyChunk(ref.generation, ref.entryId, offset, ref.recordOffset);
        if (!current()) return;
        if (chunk.generation !== ref.generation || chunk.entryId !== ref.entryId || chunk.recordOffset !== ref.recordOffset || chunk.offset !== offset || chunk.encoding !== "utf8-jsonl") throw new Error("Body chunk identity mismatch");
        const received = offset + new TextEncoder().encode(chunk.data).byteLength;
        if (chunk.totalBytes !== ref.byteLength || received > chunk.totalBytes) throw new Error("Body byte length mismatch");
        chunks.push(chunk.data);
        setProgress(received);
        if (chunk.complete) {
          if (chunk.nextOffset !== null || received !== chunk.totalBytes) throw new Error("Invalid final body chunk");
          const original = chunks.join("");
          JSON.parse(original); // validate completeness, never substitute a preview
          setRaw(original);
          break;
        }
        if (chunk.nextOffset === null || chunk.nextOffset <= offset) throw new Error("Incomplete body response made no progress");
        if (chunk.nextOffset !== received) throw new Error("Body byte offset mismatch");
        offset = chunk.nextOffset;
      }
    } catch (e) { if (current()) setError(String(e) + " — reload the page if history changed."); }
    finally { if (current()) setBusy(false); }
  };
  return <div className="meta-line" role="status">
    <strong>{raw === null ? "Incomplete preview — not the full code/output/evidence." : "Full original record (lossless JSONL)"}</strong>
    <span> · {entry.bodyRef?.byteLength.toLocaleString()} bytes</span>
    {raw === null && <button disabled={busy || !onBodyChunk} onClick={() => void load()}>
      {busy ? `Loading full body… ${progress.toLocaleString()} bytes` : "Load full body"}</button>}
    {busy && <button onClick={() => { run.current++; setBusy(false); setProgress(0); }}>Cancel body download</button>}
    {error && <div>{error}</div>}
    {download && <a href={download} download={`${entry.id}.jsonl`}>Download full original record</a>}
    {raw !== null && <>
      <button onClick={() => { setRaw(null); setProgress(0); }}>Release full body</button>
      <pre className="tool-output" style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{raw}</pre>
    </>}
  </div>;
}

function ThinkingLine({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="thinking">
      <button className="thinking-head" onClick={() => setOpen(!open)}>
        <span>▸ {open ? "Thinking…" : "Thinking…"}</span>
        {text.length > 0 && <span className="thinking-len">{text.length} chars</span>}
      </button>
      {open && <pre className="thinking-body">{text}</pre>}
    </div>
  );
}

const MemoEntryView = React.memo(function EntryView({
  entry,
  inlineResults,
  hideToolOutput,
}: {
  entry: Entry;
  inlineResults: Entry[];
  hideToolOutput?: boolean;
}) {
  switch (entry.kind) {
    case "model_change":
      return (
        <div className="meta-line">
          ⚙️ Model → <b>{entry.model}</b>
          <TimeStamp ts={entry.ts} />
        </div>
      );
    case "thinking_level_change":
      return (
        <div className="meta-line">
          Thinking → <b>{entry.name}</b>
          <TimeStamp ts={entry.ts} />
        </div>
      );
    case "compaction":
      return (
        <div className="meta-line compact">
          📦 Context compacted{entry.name && <>（{fmtTokens(Number(entry.name))} tokens）</>}
          {entry.summary && <p>{entry.summary}</p>}
        </div>
      );
    case "branch_summary":
      return (
        <div className="meta-line branch">
          🌿 Branch summary{entry.summary && <p>{entry.summary}</p>}
        </div>
      );
    case "session_info":
      return entry.name ? (
        <div className="meta-line name-line">📌 <b>{entry.name}</b></div>
      ) : null;
    case "label":
      return entry.label ? (
        <div className="meta-line">
          🏷️ <span className="badge">{entry.label}</span>
        </div>
      ) : null;
    default:
      break;
  }

  if (entry.role === "user")
    return (
      <div className="msg user">
        <div className="msg-text">
          {entry.content.map((c, i) =>
            c.kind === "text" ? <MemoMarkdown key={i} text={c.text} /> : null
          )}
        </div>
      </div>
    );

  if (entry.role === "assistant") {
    const text = entry.content.filter((c) => c.kind === "text").map((c) => (c as any).text).join("\n");
    const thinking = entry.content.filter((c) => c.kind === "thinking");
    const calls = entry.content.filter((c) => c.kind === "toolCall");
    const resultsById = new Map(inlineResults.map(r => [r.toolCallId, r]));
    return (
      <div className="msg assistant">
        {thinking.map((c, i) => (
          <ThinkingLine key={i} text={(c as any).thinking} />
        ))}
        {text && (
          <div className="msg-text">
            <MemoMarkdown text={text} />
          </div>
        )}
        {calls.map((c, i) => {
          const call = c as Extract<ContentBlock, { kind: "toolCall" }>;
          const result = resultsById.get(call.id);
          const output = result?.content
            .map((blk) => (blk.kind === "text" ? blk.text : blk.kind === "bash" ? blk.output : ""))
            .join("\n")
            .trim();
          return (
            <ToolRow
              key={call.id + i}
              name={call.name}
              arg={call.arguments}
              output={output ?? ""}
              isError={result?.isError ?? false}
              defaultOpen={!!result?.isError}
              hideResult={hideToolOutput}
            />
          );
        })}
      </div>
    );
  }

  if (entry.role === "toolResult") {
    // standalone (no matching call found): show compact row
    const output = entry.content
      .map((blk) => (blk.kind === "text" ? blk.text : blk.kind === "bash" ? blk.output : ""))
      .join("\n")
      .trim();
    return (
      <div className="msg tool">
        <ToolRow name={entry.toolName ?? "tool"} arg="" output={output} isError={entry.isError ?? false} defaultOpen={entry.isError ?? false} />
      </div>
    );
  }

  if (entry.role === "bashExecution") {
    const bash = entry.content.find((c) => c.kind === "bash") as Extract<ContentBlock, { kind: "bash" }> | undefined;
    if (!bash) return null;
    return (
      <div className="msg tool">
        <ToolRow name="bash" arg={bash.command} output={bash.output} isError={(bash.exitCode ?? 0) !== 0} />
      </div>
    );
  }

  if (entry.role === "custom" || entry.kind === "custom_message")
    return (
      <div className="msg custom">
        <div className="msg-text custom-text">
          {entry.summary ?? entry.content.map((c) => (c.kind === "text" ? c.text : "")).join("\n")}
        </div>
      </div>
    );

  return null;
});

function TimeStamp({ ts }: { ts: string | null }) {
  if (!ts) return null;
  const n = Number(ts);
  const d = Number.isFinite(n) && n > 0 ? new Date(n) : new Date(ts);
  if (isNaN(d.getTime())) return null;
  return (
    <span className="ts" title={d.toLocaleString()}>
      {d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
    </span>
  );
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M tokens";
  if (n >= 1000) return (n / 1000).toFixed(1) + "k tokens";
  return n + " tokens";
}

/// pi-style context limit formatting: 1000000 -> 1000.0k
function fmtContext(n: number): string {
  if (n >= 1000) return (n / 1000).toFixed(1) + "k";
  return String(n);
}
