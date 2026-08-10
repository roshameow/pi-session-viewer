import React, { useEffect, useMemo, useRef, useState } from "react";
import type { ContentBlock, Entry, SessionDetail } from "../types";

// ---------- Live conversation blocks (assembled from pi json events) ----------

export type LiveBlock =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; thinking: string; done: boolean }
  | { kind: "tool"; name: string; args: string; result: string; isError: boolean; done: boolean };

export function buildLiveBlocks(events: any[]): LiveBlock[] {
  const blocks: LiveBlock[] = [];
  let curAssistant: { text: string; thinking: string } | null = null;
  let curTool: (LiveBlock & { kind: "tool" }) | null = null;

  const flushAssistant = (done: boolean) => {
    if (curAssistant) {
      blocks.push({ kind: "assistant", text: curAssistant.text, thinking: curAssistant.thinking, done });
      curAssistant = null;
    }
  };
  const flushTool = (done: boolean) => {
    if (curTool) {
      curTool.done = done;
      blocks.push(curTool);
      curTool = null;
    }
  };

  for (const ev of events) {
    switch (ev.type) {
      case "message_start": {
        const m = ev.message;
        if (m?.role === "user") {
          flushAssistant(true);
          flushTool(true);
          const text = contentText(m.content);
          if (text) blocks.push({ kind: "user", text });
        } else if (m?.role === "assistant") {
          flushTool(true);
          curAssistant = { text: "", thinking: "" };
        }
        break;
      }
      case "message_update": {
        const ae = ev.assistantMessageEvent;
        if (!ae || !curAssistant) break;
        if (ae.type === "text_delta" && typeof ae.delta === "string") curAssistant.text += ae.delta;
        if (ae.type === "thinking_delta" && typeof ae.delta === "string") curAssistant.thinking += ae.delta;
        break;
      }
      case "message_end": {
        const m = ev.message;
        if (m?.role === "assistant" && curAssistant) {
          const t = contentText(m.content);
          if (t) curAssistant.text = t;
          flushAssistant(true);
        }
        break;
      }
      case "tool_execution_start": {
        flushAssistant(true);
        curTool = { kind: "tool", name: ev.toolName ?? "tool", args: stringify(ev.args), result: "", isError: false, done: false };
        break;
      }
      case "tool_execution_update": {
        if (curTool) {
          const r = stringify(ev.partialResult);
          if (r) curTool.result = r;
        }
        break;
      }
      case "tool_execution_end": {
        if (curTool) {
          curTool.result = stringify(ev.result);
          curTool.isError = !!ev.isError;
          flushTool(true);
        }
        break;
      }
      default:
        break;
    }
  }
  flushAssistant(true);
  flushTool(true);
  return blocks;
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
  const n = s.split("\n").length;
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
      return `$ ${arg}` + (isError ? " → 失败" : lines > 1 ? ` → ${lines} lines` : "");
    case "ls":
      return `Ls(${arg}) → ${lines} entries`;
    case "find":
      return `Find(${arg}) → ${lines} files`;
    case "grep":
      return `Grep(${arg}) → ${lines} matches`;
    default:
      return `${name}(${arg})` + (isError ? " → 错误" : lines > 1 ? ` → ${lines} lines` : "");
  }
}

// ---------- single-line tool row (click to expand output) ----------

function ToolRow({
  name,
  arg,
  output,
  isError,
  running,
  defaultOpen,
}: {
  name: string;
  arg: string;
  output: string;
  isError: boolean;
  running?: boolean;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(!!defaultOpen);
  const summary = toolSummary(name, arg, output, isError);
  const outputLen = output.length;
  return (
    <div className={`tool-row ${isError ? "err" : ""}`}>
      <button
        className="tool-line"
        onClick={() => setOpen(!open)}
        title={output ? "点击展开/收起输出" : undefined}
      >
        <span className={`tdot ${isError ? "red" : running ? "pulse" : "green"}`}>●</span>
        <span className="tool-summary">{summary}</span>
        {running && <span className="tool-running">运行中…</span>}
        {!running && outputLen > 0 && (
          <span className="tool-expand">{open ? "▾" : "▸"} {open ? "收起" : "展开"}</span>
        )}
      </button>
      {open && output && (
        <pre className="tool-output" onClick={(e) => e.stopPropagation()}>
          {output}
        </pre>
      )}
    </div>
  );
}

// ---------- thread ----------

export function Thread({
  detail,
  liveBlocks,
  running,
}: {
  detail: SessionDetail;
  liveBlocks: LiveBlock[];
  running: boolean;
}) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  const renderItems = useMemo(() => {
    const entries = detail.entries;
    const active = detail.active.map((i) => entries[i]);
    const skip = new Set<number>();
    const items: { entry: Entry; inlineResults: Entry[] }[] = [];

    active.forEach((entry, idx) => {
      const toolCalls =
        entry.role === "assistant" ? entry.content.filter((c) => c.kind === "toolCall") : [];
      if (toolCalls.length === 0) {
        items.push({ entry, inlineResults: [] });
        return;
      }
      const results: Entry[] = [];
      for (let j = idx + 1; j < active.length && results.length < toolCalls.length * 2; j++) {
        const e = active[j];
        if (e.role === "toolResult") {
          const callId = e.toolCallId;
          if (toolCalls.some((tc) => tc.kind === "toolCall" && tc.id === callId)) results.push(e);
        } else if (e.role === "assistant" || e.role === "user") {
          break;
        }
      }
      items.push({ entry, inlineResults: results });
      results.forEach((r) => {
        const ri = active.indexOf(r);
        if (ri >= 0) skip.add(ri);
      });
    });
    return { items, skip };
  }, [detail]);

  useEffect(() => {
    if (autoScroll) bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [renderItems.items.length, liveBlocks, autoScroll]);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 200);
  };

  return (
    <div className="thread" onScroll={onScroll}>
      <div className="thread-inner">
        <div className="session-head">
          <div className="session-head-title">{detail.stats.model ?? "pi session"}</div>
          <div className="session-head-meta">
            {detail.stats.messageCount} 条消息 · {fmtTokens(detail.stats.tokenCount)} · ${detail.stats.costTotal.toFixed(4)} · {detail.cwd}
          </div>
        </div>

        {renderItems.items.map(({ entry, inlineResults }, idx) => {
          if (renderItems.skip.has(detail.active[idx])) return null;
          return <EntryView key={entry.id + idx} entry={entry} inlineResults={inlineResults} />;
        })}

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
                    {b.text}
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

        {running && <div className="running-bar">⏳ pi 正在工作…</div>}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}

function ThinkingLine({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="thinking">
      <button className="thinking-head" onClick={() => setOpen(!open)}>
        <span>▸ {open ? "思考中" : "Thinking…"}</span>
        {text.length > 0 && <span className="thinking-len">{text.length} chars</span>}
      </button>
      {open && <pre className="thinking-body">{text}</pre>}
    </div>
  );
}

function EntryView({
  entry,
  inlineResults,
}: {
  entry: Entry;
  inlineResults: Entry[];
}) {
  switch (entry.kind) {
    case "model_change":
      return (
        <div className="meta-line">
          ⚙️ 模型 → <b>{entry.model}</b>
          <TimeStamp ts={entry.ts} />
        </div>
      );
    case "thinking_level_change":
      return (
        <div className="meta-line">
          🧠 思考级别 → <b>{entry.name}</b>
          <TimeStamp ts={entry.ts} />
        </div>
      );
    case "compaction":
      return (
        <div className="meta-line compact">
          📦 上下文压缩{entry.name && <>（{fmtTokens(Number(entry.name))} tokens）</>}
          {entry.summary && <p>{entry.summary}</p>}
        </div>
      );
    case "branch_summary":
      return (
        <div className="meta-line branch">
          🌿 分支摘要{entry.summary && <p>{entry.summary}</p>}
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
            c.kind === "text" ? (
              <pre key={i} className="text-pre">{c.text}</pre>
            ) : null
          )}
        </div>
      </div>
    );

  if (entry.role === "assistant") {
    const text = entry.content.filter((c) => c.kind === "text").map((c) => (c as any).text).join("\n");
    const thinking = entry.content.filter((c) => c.kind === "thinking");
    const calls = entry.content.filter((c) => c.kind === "toolCall");
    return (
      <div className="msg assistant">
        {thinking.map((c, i) => (
          <ThinkingLine key={i} text={(c as any).thinking} />
        ))}
        {text && <div className="msg-text"><pre className="text-pre">{text}</pre></div>}
        {calls.map((c, i) => {
          const call = c as Extract<ContentBlock, { kind: "toolCall" }>;
          const result = inlineResults.find((r) => r.toolCallId === call.id);
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
}

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
