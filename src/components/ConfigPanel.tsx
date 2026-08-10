import React, { useEffect, useState } from "react";
import { api } from "../api";
import type { AgentInfo, ConfigView, McpServer, SkillInfo } from "../types";

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

function McpCard({ s }: { s: McpServer }) {
  const source =
    s.source === "global" ? "global" : s.source.split("/").filter(Boolean).slice(-2).join("/");
  return (
    <div className="cfg-card">
      <div className="cfg-card-line1">
        <span className="cfg-icon">🔌</span>
        <span className="cfg-name">{s.name}</span>
        {s.enabled === false && <span className="badge err">disabled</span>}
        <span className="cfg-source">{source}</span>
      </div>
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
  return (
    <div className="cfg-card">
      <div className="cfg-card-line1">
        <span className="cfg-icon">🤖</span>
        <span className="cfg-name">{a.name}</span>
        {a.tools && <span className="cfg-tools" title="allowed tools">{a.tools}</span>}
      </div>
      {a.description && <div className="cfg-desc">{a.description}</div>}
      <div className="cfg-card-meta dim" title={a.file}>
        {a.file}
      </div>
    </div>
  );
}

function SkillCard({ s }: { s: SkillInfo }) {
  const source =
    s.source === "global" ? "global" : s.source.split("/").filter(Boolean).slice(-2).join("/");
  return (
    <div className="cfg-card">
      <div className="cfg-card-line1">
        <span className="cfg-icon">📖</span>
        <span className="cfg-name">{s.name}</span>
        <span className="cfg-source">{source}</span>
      </div>
      {s.description && <div className="cfg-desc">{s.description}</div>}
    </div>
  );
}

export function ConfigPanel() {
  const [cfg, setCfg] = useState<ConfigView | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .listConfig()
      .then(setCfg)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="config-panel">
      <div className="config-inner">
        <h2 className="config-title">⚙️ Config</h2>
        {error && <div className="error-bar" onClick={() => setError(null)}>⚠️ {error}</div>}
        {!cfg && !error && <div className="empty">Loading…</div>}
        {cfg && (
          <>
            <CollapseGroup title="MCP Servers" count={cfg.mcp.length} accent="var(--cyan)">
              {cfg.mcp.length === 0 && <div className="empty">No MCP servers configured</div>}
              {cfg.mcp.map((s) => (
                <McpCard key={s.name} s={s} />
              ))}
            </CollapseGroup>

            <CollapseGroup title="Agents" count={cfg.agents.length} accent="var(--purple)">
              {cfg.agents.length === 0 && <div className="empty">No agents configured</div>}
              {cfg.agents.map((a) => (
                <AgentCard key={a.name} a={a} />
              ))}
            </CollapseGroup>

            <CollapseGroup title="Skills" count={cfg.skills.length} accent="var(--yellow)">
              {cfg.skills.length === 0 && <div className="empty">No skills found</div>}
              {cfg.skills.map((s) => (
                <SkillCard key={s.source + s.name} s={s} />
              ))}
            </CollapseGroup>
          </>
        )}
      </div>
    </div>
  );
}
