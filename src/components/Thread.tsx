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
  let curTool: LiveBlock & { kind: "tool" } | null = null;

  const flushAssistant = (done: boolean) => {
    if (curAssistant) {
      blocks.push({
        kind: "assistant",
        text: curAssistant.text,
        thinking: curAssistant.thinking,
        done,
      });
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
        if (ae.type === "thinking_delta" && typeof ae.delta === "string")
          curAssistant.thinking += ae.delta;
        break;
      }
      case "message_end": {
        const m = ev.message;
        if (m?.role === "assistant" && curAssistant) {
          // final authoritative text
          const t = contentText(m.content);
          if (t) curAssistant.text = t;
          flushAssistant(true);
        }
        break;
      }
      case "tool_execution_start": {
        flushAssistant(true);
        curTool = {
          kind: "tool",
          name: ev.toolName ?? "tool",
          args: stringify(ev.args),
          result: "",
          isError: false,
          done: false,
        };
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
        typeof c === "string"
          ? c
          : c?.type === "text"
            ? c.text
            : c?.type === "thinking"
              ? c.thinking
              : ""
      )
      .join(" ")
      .trim();
  }
  return "";
}

// ---------- Single block renderers ----------

function ToolResult({ entry }: { entry: Entry }) {
  const [open, setOpen] = useState(false);
  const text = entry.content
    .map((c) => (c.kind === "Text" ? c.text : c.kind === "Bash" ? `${c.command}\n${c.output}` : ""))
    .join("\n");
  const preview = text.length > 240 ? text.slice(0, 240) + "…" : text;
  return (
    <div className={`tool-result ${entry.isError ? "err" : ""}`} onClick={() => setOpen(!open)}>
      <div className="tool-result-head">
        <span className="dot" />
        <span className="tool-name">{entry.toolName ?? "toolResult"}</span>
        {entry.isError && <span className="badge err">ERROR</span>}
        <span className="tool-toggle">{open ? "▾ 收起" : "▸ 展开"}</span>
      </div>
      {open && (
        <pre className="tool-output">
          {text || "(空输出)"}
        </pre>
      )}
      {!open && preview && <pre className="tool-preview">{preview}</pre>}
    </div>
  );
}

function BashBlock({ block }: { block: Extract<ContentBlock, { kind: "Bash" }> }) {
  const [open, setOpen] = useState(false);
  const output = block.output || "(无输出)";
  const long = output.length > 400;
  return (
    <div className="bash-block">
      <div className="bash-command">
        <span className="prompt-sign">$</span> {block.command}
        {block.exitCode !== null && block.exitCode !== 0 && (
          <span className="badge err">exit {block.exitCode}</span>
        )}
        {block.truncated && <span className="badge">截断</span>}
        {long && (
          <button className="link-btn" onClick={() => setOpen(!open)}>
            {open ? "收起" : "展开输出"}
          </button>
        )}
      </div>
      {(open || !long) && <pre className="bash-output">{output}</pre>}
    </div>
  );
}

function ThinkingBlock({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="thinking-block">
      <button className="thinking-head" onClick={() => setOpen(!open)}>
        <span>🧠 思考</span>
        <span>{open ? "▾" : "▸"}</span>
      </button>
      {open && <pre className="thinking-body">{text}</pre>}
    </div>
  );
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

// ---------- Thread ----------

export function Thread({
  detail,
  liveBlocks,
  running,
  onJumpToSubagent,
}: {
  detail: SessionDetail;
  liveBlocks: LiveBlock[];
  running: boolean;
  onJumpToSubagent?: (path: string) => void;
}) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  const { renderItems, skipToolResults } = useMemo(() => {
    const entries = detail.entries;
    const active = detail.active.map((i) => entries[i]);
    const skip = new Set<number>();
    const items: { entry: Entry; inlineResults: Entry[] }[] = [];

    active.forEach((entry, idx) => {
      const toolCalls =
        entry.role === "assistant"
          ? entry.content.filter((c) => c.kind === "ToolCall")
          : [];
      if (toolCalls.length === 0) {
        items.push({ entry, inlineResults: [] });
        return;
      }
      // collect following toolResult entries matching this assistant's calls
      const results: Entry[] = [];
      for (let j = idx + 1; j < active.length && results.length < toolCalls.length * 2; j++) {
        const e = active[j];
        if (e.role === "toolResult") {
          const callId = e.toolCallId;
          if (toolCalls.some((tc) => tc.kind === "ToolCall" && tc.id === callId)) {
            results.push(e);
          }
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
    return { renderItems: items, skipToolResults: skip };
  }, [detail]);

  useEffect(() => {
    if (autoScroll) {
      bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [renderItems.length, liveBlocks.length, liveBlocks, autoScroll]);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 200;
    setAutoScroll(nearBottom);
  };

  return (
    <div className="thread" onScroll={onScroll}>
      <div className="thread-inner">
        {/* session header */}
        <div className="session-head">
          <div className="session-head-title">{detail.stats.model ?? "pi session"}</div>
          <div className="session-head-meta">
            {detail.stats.messageCount} 条消息 · {formatTokens(detail.stats.tokenCount)} tokens ·
            花费 ${detail.stats.costTotal.toFixed(4)} · {detail.cwd}
          </div>
        </div>

        {renderItems.map(({ entry, inlineResults }, idx) => {
          if (skipToolResults.has(detail.active[idx])) return null;
          return (
            <EntryView
              key={entry.id + idx}
              entry={entry}
              inlineResults={inlineResults}
              onJumpToSubagent={onJumpToSubagent}
            />
          );
        })}

        {/* live conversation blocks */}
        {liveBlocks.map((b, i) => {
          if (b.kind === "user")
            return (
              <div key={"live" + i} className="msg user">
                <div className="msg-role">👤</div>
                <div className="msg-body">
                  <div className="bubble user">{b.text}</div>
                </div>
              </div>
            );
          if (b.kind === "assistant")
            return (
              <div key={"live" + i} className="msg assistant">
                <div className="msg-role">🤖</div>
                <div className="msg-body">
                  {b.thinking && <ThinkingBlock text={b.thinking} />}
                  {b.text ? (
                    <div className="bubble assistant">
                      {b.text}
                      {!b.done && <span className="cursor" />}
                    </div>
                  ) : (
                    !b.done && <div className="bubble assistant thinking-dots"><span/><span/><span/></div>
                  )}
                </div>
              </div>
            );
          return (
            <div key={"live" + i} className="msg tool">
              <div className="msg-role">🛠️</div>
              <div className="msg-body">
                <div className={`tool-result ${b.isError ? "err" : ""}`}>
                  <div className="tool-result-head">
                    <span className="dot pulse" />
                    <span className="tool-name">{b.name}</span>
                    {b.isError && <span className="badge err">ERROR</span>}
                    {!b.done && <span className="tool-toggle">运行中…</span>}
                  </div>
                  {b.done && b.result && <pre className="tool-output">{b.result}</pre>}
                </div>
              </div>
            </div>
          );
        })}

        {running && <div className="running-bar">⏳ pi 正在工作…</div>}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}

function EntryView({
  entry,
  inlineResults,
  onJumpToSubagent,
}: {
  entry: Entry;
  inlineResults: Entry[];
  onJumpToSubagent?: (path: string) => void;
}) {
  // meta entries
  if (entry.kind === "model_change")
    return (
      <div className="meta-line">
        ⚙️ 切换模型 → <b>{entry.model}</b>
        <TimeStamp ts={entry.ts} />
      </div>
    );
  if (entry.kind === "thinking_level_change")
    return (
      <div className="meta-line">
        🧠 思考级别 → <b>{entry.name}</b>
        <TimeStamp ts={entry.ts} />
      </div>
    );
  if (entry.kind === "compaction")
    return (
      <div className="meta-line compact">
        📦 上下文压缩
        {entry.name && <span className="badge">压缩前 {formatTokens(Number(entry.name))} tokens</span>}
        {entry.summary && <p>{entry.summary}</p>}
      </div>
    );
  if (entry.kind === "branch_summary")
    return (
      <div className="meta-line branch">
        🌿 分支摘要 {entry.summary && <p>{entry.summary}</p>}
      </div>
    );
  if (entry.kind === "session_info" && entry.name)
    return (
      <div className="meta-line name-line">📌 会话名称: <b>{entry.name}</b></div>
    );
  if (entry.kind === "label" && entry.label)
    return (
      <div className="meta-line">
        🏷️ <span className="badge">{entry.label}</span>
      </div>
    );
  if (entry.kind === "custom_message")
    return (
      <div className="msg custom">
        <div className="msg-role">🧩</div>
        <div className="msg-body">
          <div className="bubble custom">{entry.summary ?? ""}</div>
        </div>
      </div>
    );

  // message entries
  if (entry.role === "user")
    return (
      <div className="msg user">
        <div className="msg-role">👤</div>
        <div className="msg-body">
          <div className="bubble user">
            {entry.content.map((c, i) =>
              c.kind === "Text" ? (
                <pre key={i} className="text-pre">{c.text}</pre>
              ) : null
            )}
          </div>
        </div>
      </div>
    );

  if (entry.role === "assistant") {
    const text = entry.content.filter((c) => c.kind === "Text").map((c) => (c as any).text).join("\n");
    const thinking = entry.content.filter((c) => c.kind === "Thinking");
    const calls = entry.content.filter((c) => c.kind === "ToolCall");
    return (
      <div className="msg assistant">
        <div className="msg-role">🤖</div>
        <div className="msg-body">
          {thinking.map((c, i) => (
            <ThinkingBlock key={i} text={(c as any).thinking} />
          ))}
          {text && <div className="bubble assistant"><pre className="text-pre">{text}</pre></div>}
          {calls.map((c, i) => {
            const call = c as Extract<ContentBlock, { kind: "ToolCall" }>;
            const result = inlineResults.find((r) => r.toolCallId === call.id);
            return (
              <div key={i} className="tool-group">
                <div className="tool-call">
                  <span className="tool-name">🔧 {call.name}</span>
                  {call.arguments && <pre className="tool-args">{call.arguments}</pre>}
                </div>
                {result && <ToolResult entry={result} />}
              </div>
            );
          })}
          <TimeStamp ts={entry.ts} />
        </div>
      </div>
    );
  }

  if (entry.role === "toolResult") {
    // rendered inline; if it shows here (no matching call), render standalone
    return (
      <div className="msg tool">
        <div className="msg-role">🛠️</div>
        <div className="msg-body">
          <ToolResult entry={entry} />
        </div>
      </div>
    );
  }

  if (entry.role === "bashExecution") {
    const bash = entry.content.find((c) => c.kind === "Bash") as
      | Extract<ContentBlock, { kind: "Bash" }>
      | undefined;
    return (
      <div className="msg bash">
        <div className="msg-role">$</div>
        <div className="msg-body">{bash && <BashBlock block={bash} />}</div>
      </div>
    );
  }

  return null;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1000) return (n / 1000).toFixed(1) + "k";
  return String(n);
}
