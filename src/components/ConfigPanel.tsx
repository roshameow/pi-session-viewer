import React, { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { LocalFileLinkContext, MemoMarkdown, type LocalFileLinkScope } from "./Markdown";
import type { AgentInfo, ConfigView, McpServer, SkillDocument, SkillInfo } from "../types";

function CollapseGroup({
  title,
  count,
  accent,
  children,
}: {
  title: string;
  count: number;
  accent: string;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(true);
  return (
    <div className="cfg-group">
      <div className="cfg-group-head" onClick={() => setOpen(!open)}>
        <span className="section-arrow">{open ? "▾" : "▸"}</span>
        <span className="cfg-group-title" style={{ color: accent }}>
          {title}
        </span>
        <span className="section-count">{count}</span>
      </div>
      {open && <div className="cfg-group-body">{children}</div>}
    </div>
  );
}

export function McpCard({ s }: { s: McpServer }) {
  const source =
    s.source === "global" ? "global" : s.source.split("/").filter(Boolean).slice(-2).join("/");
  return (
    <div className="cfg-card">
      <div className="cfg-card-line1">
        <span className="cfg-icon">🔌</span>
        <span className="cfg-name">{s.name}</span>
        {s.enabled === false && <span className="badge err">native disabled</span>}
        {s.disabled === true && <span className="badge err">adapter disabled</span>}
        <span className="badge">{s.dialect === "adapter" ? "adapter" : "mcpServers"}</span>
        <span className="cfg-source">{source}</span>
      </div>
      <div className="cfg-card-meta dim">native exposure: {s.exposure ?? "codemode (default)"}</div>
      {s.toolExposure && Object.entries(s.toolExposure).map(([name, exposure]) => (
        <div className="cfg-card-meta mono dim" key={name}>toolExposure: {name} → {exposure}</div>
      ))}
      {s.directTools !== undefined && s.directTools !== null && (
        <div className="cfg-card-meta mono dim">adapter directTools: {JSON.stringify(s.directTools)}</div>
      )}
      {s.configPath && <div className="cfg-card-meta dim">{s.configPath}</div>}
      {s.command && <div className="cfg-card-meta mono">{s.command}</div>}
      {s.args.length > 0 && (
        <div className="cfg-card-meta mono dim">{s.args.join(" ")}</div>
      )}
      {s.socket && <div className="cfg-card-meta mono dim">socket: {s.socket}</div>}
      {s.url && <div className="cfg-card-meta mono dim">url: {s.url}</div>}
      {s.env.length > 0 && (
        <div className="cfg-card-meta mono dim">{s.env.join("  ")}</div>
      )}
    </div>
  );
}

function AgentCard({ a }: { a: AgentInfo }) {
  const source =
    a.source === "global" ? "global" : a.source.split("/").filter(Boolean).slice(-2).join("/");
  return (
    <div className="cfg-card">
      <div className="cfg-card-line1">
        <span className="cfg-icon">🤖</span>
        <span className="cfg-name">{a.name}</span>
        <span className="cfg-source">{source}</span>
        {a.tools && <span className="cfg-tools" title="allowed tools">{a.tools}</span>}
      </div>
      {a.description && <div className="cfg-desc">{a.description}</div>}
      <div className="cfg-card-meta dim" title={a.file}>
        {a.file}
      </div>
    </div>
  );
}

export function SkillCard({ s, onOpen, disabled = false, buttonRef }: {
  s: SkillInfo;
  onOpen: () => void;
  disabled?: boolean;
  buttonRef?: React.Ref<HTMLButtonElement>;
}) {
  const source =
    s.source === "global" ? "global" : s.source.split("/").filter(Boolean).slice(-2).join("/");
  return (
    <button type="button" className="cfg-card cfg-skill-card" ref={buttonRef}
      data-skill-file={s.file} disabled={disabled} onClick={onOpen}
      aria-label={`View skill ${s.name} (${s.source}): ${s.file}`}>
      <span className="cfg-card-line1">
        <span className="cfg-icon" aria-hidden="true">📖</span>
        <span className="cfg-name">{s.name}</span>
        <span className="cfg-source" title={s.source}>{source}</span>
      </span>
      {s.description && <span className="cfg-desc">{s.description}</span>}
      <span className="cfg-card-meta dim" title={s.file}>{s.file}</span>
    </button>
  );
}

// Keep the fetched document byte-for-byte in state/raw view. Only the preview
// omits a complete leading YAML frontmatter block (including CRLF/BOM files).
export function skillPreviewBody(content: string): string {
  const opening = /^(?:\uFEFF)?---[ \t]*\r?\n/.exec(content);
  if (!opening) return content;
  const rest = content.slice(opening[0].length);
  const closing = /^(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/m.exec(rest);
  return closing ? rest.slice(closing.index + closing[0].length) : content;
}

const skillInventoryNotice = "只读查看，不执行或启用技能。此配置清单不代表 Pi 运行时的完整技能资源。";

const skillLinkScope: LocalFileLinkScope = {
  sessionPath: "",
  allowed: false,
  disabledReason: "Local file links are disabled in skill previews.",
};

export function SkillDetail({ skill, document, loading, error, onBack, onRetry }: {
  skill: SkillInfo;
  document: SkillDocument | null;
  loading: boolean;
  error: string | null;
  onBack: () => void;
  onRetry: () => void;
}) {
  const [raw, setRaw] = useState(false);
  const title = useRef<HTMLHeadingElement | null>(null);
  useEffect(() => { title.current?.focus(); }, []);
  return (
    <section className="cfg-skill-detail" aria-labelledby="config-skill-title"
      data-skill-file={skill.file} data-skill-document-file={document?.file}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) {
          event.preventDefault(); event.stopPropagation(); onBack();
        }
      }}>
      <div className="cfg-skill-toolbar">
        <button type="button" className="cfg-small-btn" onClick={onBack}>← Back to Config</button>
        <span className="dim">Read-only skill</span>
      </div>
      <h2 className="config-title" id="config-skill-title" tabIndex={-1} ref={title}>{skill.name}</h2>
      <div className="cfg-card-meta dim">{skillInventoryNotice}</div>
      <div className="cfg-card-meta dim">Source: {skill.source}</div>
      <div className="cfg-card-meta dim">Path: {document?.file ?? skill.file}</div>
      {skill.description && <p className="cfg-desc">{skill.description}</p>}
      {loading && <div className="empty" role="status">Loading skill…</div>}
      {error && <div className="cfg-skill-error" role="alert">
        <span>Could not read skill: {error}</span>
        <button type="button" className="cfg-small-btn" disabled={loading} onClick={onRetry}>Retry</button>
      </div>}
      {document && <>
        <div className="cfg-skill-toolbar" role="group" aria-label="Skill display">
          <button type="button" className="cfg-small-btn" aria-pressed={!raw} onClick={() => setRaw(false)}>Preview</button>
          <button type="button" className="cfg-small-btn" aria-pressed={raw} onClick={() => setRaw(true)}>Raw source</button>
        </div>
        {raw ? <pre className="cfg-skill-raw" aria-label="Skill raw source"><code>{document.content}</code></pre> :
          <div className="cfg-skill-preview">
            <LocalFileLinkContext.Provider value={skillLinkScope}>
              <MemoMarkdown text={skillPreviewBody(document.content)} allowImages={false} />
            </LocalFileLinkContext.Provider>
          </div>}
      </>}
    </section>
  );
}

type SkillSelection = { skill: SkillInfo; id: number };
type SkillRead = {
  selection: SkillSelection;
  document: SkillDocument | null;
  loading: boolean;
  error: string | null;
};
const alwaysValid = () => true;

export function ConfigPanel({ ready = true, validate = alwaysValid }: {
  ready?: boolean;
  validate?: () => boolean;
}) {
  const [cfg, setCfg] = useState<ConfigView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [detail, setDetail] = useState<SkillRead | null>(null);
  const mounted = useRef(false);
  const current = useRef({ ready, validate });
  current.current = { ready, validate };
  const listRequest = useRef(0);
  const listPending = useRef(false);
  const bodyRequest = useRef(0);
  const bodyPending = useRef(false);
  const active = useRef<SkillSelection | null>(null);
  const selectionId = useRef(0);
  const restoreFile = useRef<string | null>(null);
  const cards = useRef(new Map<string, HTMLButtonElement>());
  const eligible = () => mounted.current && ready && current.current.ready &&
    current.current.validate === validate && validate();

  const loadConfig = useCallback(async () => {
    if (!eligible() || listPending.current) return;
    const request = ++listRequest.current;
    listPending.current = true;
    setError(null);
    try {
      const result = await api.listConfig();
      if (eligible() && listRequest.current === request) setCfg(result);
    } catch (e) {
      if (eligible() && listRequest.current === request) setError(String(e));
    } finally {
      if (listRequest.current === request) listPending.current = false;
    }
  }, [ready, validate]);

  useEffect(() => {
    // Reset on readiness/validator changes, and re-arm after StrictMode's
    // setup/cleanup/setup replay. Never reuse an invalidated body request.
    mounted.current = true;
    setCfg(null);
    setError(null);
    setDetail(null);
    void loadConfig();
    return () => {
      mounted.current = false;
      listRequest.current++;
      bodyRequest.current++;
      listPending.current = bodyPending.current = false;
      active.current = null;
      restoreFile.current = null;
    };
  }, [loadConfig]);

  const readSkill = async (selection: SkillSelection) => {
    // Both flags and source validator are checked synchronously BEFORE IPC,
    // including calls retained from a render before a source switch.
    if (!eligible() || active.current !== selection || bodyPending.current) return;
    const request = ++bodyRequest.current;
    bodyPending.current = true;
    setDetail({ selection, document: null, loading: true, error: null });
    const validRead = () => eligible() && bodyRequest.current === request && active.current === selection;
    try {
      const document = await api.readConfigSkill(selection.skill.file);
      if (!validRead()) return;
      if (document.file !== selection.skill.file || typeof document.content !== "string") {
        throw new Error("Skill document does not match the selected file.");
      }
      setDetail({ selection, document, loading: false, error: null });
    } catch (e) {
      if (validRead()) setDetail({ selection, document: null, loading: false, error: String(e) });
    } finally {
      if (bodyRequest.current === request) bodyPending.current = false;
    }
  };
  const openSkill = (skill: SkillInfo) => {
    if (!eligible() || active.current || !skill.file) return;
    const selection = { skill, id: ++selectionId.current };
    active.current = selection;
    void readSkill(selection);
  };
  const back = () => {
    if (!eligible() || !active.current) return;
    restoreFile.current = active.current.skill.file;
    active.current = null;
    bodyRequest.current++;
    bodyPending.current = false;
    setDetail(null);
  };
  useEffect(() => {
    if (!detail && restoreFile.current && eligible()) {
      cards.current.get(restoreFile.current)?.focus();
      restoreFile.current = null;
    }
  }, [detail]);

  return (
    <div className="config-panel">
      <div className="config-inner">
        {!ready || !validate() ? <div className="empty" role="status">Waiting for source…</div> : detail ? (
          <SkillDetail key={detail.selection.id} skill={detail.selection.skill} document={detail.document}
            loading={detail.loading} error={detail.error}
            onBack={() => { if (active.current === detail.selection) back(); }}
            onRetry={() => { if (detail.error) void readSkill(detail.selection); }} />
        ) : <>
          <h2 className="config-title">⚙️ Config</h2>
          {error && <div className="cfg-skill-error" role="alert">
            <span>Could not load Config: {error}</span>
            <button type="button" className="cfg-small-btn" onClick={() => void loadConfig()}>Retry</button>
          </div>}
          {!cfg && !error && <div className="empty" role="status">Loading…</div>}
          {cfg && (
            <>
              <CollapseGroup title="MCP Servers" count={cfg.mcp.length} accent="var(--cyan)">
                <div className="cfg-card-meta dim">Configuration inventory, not connection status. Trusted project .pi/mcp.json entries replace global names; legacy .mcp.json is adapter-only. Extensions may replace built-in MCP or add session-only servers.</div>
                {cfg.mcp.length === 0 && <div className="empty">No MCP servers configured</div>}
                {cfg.mcp.map((s) => (
                  <McpCard key={`${s.configPath ?? s.source}:${s.name}`} s={s} />
                ))}
              </CollapseGroup>

              <CollapseGroup title="Agents" count={cfg.agents.length} accent="var(--purple)">
                {cfg.agents.length === 0 && <div className="empty">No agents configured</div>}
                {cfg.agents.map((a) => (
                  <AgentCard key={a.name} a={a} />
                ))}
              </CollapseGroup>

              <CollapseGroup title="Skills" count={cfg.skills.length} accent="var(--yellow)">
                <div className="cfg-card-meta dim">{skillInventoryNotice}</div>
                {cfg.skills.length === 0 && <div className="empty">No skills found</div>}
                {cfg.skills.map((s) => (
                  <SkillCard key={s.file} s={s} onOpen={() => openSkill(s)} disabled={!s.file}
                    buttonRef={(node) => { if (node) cards.current.set(s.file, node); else cards.current.delete(s.file); }} />
                ))}
              </CollapseGroup>
            </>
          )}
        </>}
      </div>
    </div>
  );
}
