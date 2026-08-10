import React, { useMemo } from "react";
import type { Project, SessionMeta } from "../types";

function relTime(epochSec: number): string {
  if (!epochSec) return "";
  const diff = Date.now() / 1000 - epochSec;
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)} 天前`;
  return new Date(epochSec * 1000).toLocaleDateString();
}

function projectName(cwd: string): string {
  const parts = cwd.split("/").filter(Boolean);
  return parts.length ? parts[parts.length - 1] : cwd;
}

function SessionItem({
  s,
  depth,
  selected,
  onSelect,
}: {
  s: SessionMeta;
  depth: number;
  selected: boolean;
  onSelect: (s: SessionMeta) => void;
}) {
  const title = s.name || s.firstMessage || "(空会话)";
  return (
    <button
      className={`session-item ${selected ? "selected" : ""} ${s.isSubagent ? "sub" : ""}`}
      style={{ paddingLeft: 10 + depth * 18 }}
      onClick={() => onSelect(s)}
      title={`${s.cwd}\n${s.path}`}
    >
      <div className="session-item-line1">
        {s.running && <span className="pulse-dot" />}
        <span className="session-icon">{s.isSubagent ? "🕸️" : "💬"}</span>
        <span className="session-title">{title}</span>
      </div>
      <div className="session-item-line2">
        <span className="session-time">{relTime(s.updatedAt)}</span>
        {s.model && <span className="session-model">{s.model}</span>}
        {s.isSubagent && s.taskId && (
          <span className="session-task">task:{s.taskId}</span>
        )}
      </div>
    </button>
  );
}

export function Sidebar({
  projects,
  sessions,
  selectedProject,
  selectedSession,
  loadingSessions,
  onSelectProject,
  onSelectSession,
  onRefresh,
}: {
  projects: Project[];
  sessions: SessionMeta[];
  selectedProject: string | null;
  selectedSession: SessionMeta | null;
  loadingSessions: boolean;
  onSelectProject: (p: Project) => void;
  onSelectSession: (s: SessionMeta) => void;
  onRefresh: () => void;
}) {
  const { mainSessions, childrenMap, orphans } = useMemo(() => {
    const main: SessionMeta[] = [];
    const children = new Map<string, SessionMeta[]>();
    const orphans: SessionMeta[] = [];
    for (const s of sessions) {
      if (!s.isSubagent) {
        main.push(s);
        continue;
      }
      if (s.parentSessionId && sessions.some((m) => !m.isSubagent && m.id === s.parentSessionId)) {
        const list = children.get(s.parentSessionId) ?? [];
        list.push(s);
        children.set(s.parentSessionId, list);
      } else {
        orphans.push(s);
      }
    }
    return { mainSessions: main, childrenMap: children, orphans };
  }, [sessions]);

  return (
    <div className="sidebar">
      <div className="sidebar-top">
        <span className="app-title">Pi Desktop</span>
        <button className="icon-btn" title="刷新" onClick={onRefresh}>
          ⟳
        </button>
      </div>

      {/* projects */}
      <div className="project-list">
        {projects.map((p) => (
          <button
            key={p.key}
            className={`project-item ${selectedProject === p.key ? "selected" : ""}`}
            onClick={() => onSelectProject(p)}
          >
            <span className="project-icon">📁</span>
            <span className="project-name">{projectName(p.cwd)}</span>
            <span className="project-count">{p.sessionCount}</span>
            {p.subagentCount > 0 && <span className="project-sub">🕸️{p.subagentCount}</span>}
          </button>
        ))}
        {projects.length === 0 && <div className="empty">未找到 pi 会话目录</div>}
      </div>

      {/* sessions of selected project */}
      {selectedProject && (
        <div className="session-list">
          <div className="session-list-head">
            {loadingSessions ? "加载中…" : `${mainSessions.length + orphans.length} 个会话`}
          </div>
          {mainSessions.map((s) => (
            <React.Fragment key={s.path}>
              <SessionItem s={s} depth={0} selected={selectedSession?.path === s.path} onSelect={onSelectSession} />
              {(childrenMap.get(s.id) ?? []).map((sub) => (
                <SessionItem
                  key={sub.path}
                  s={sub}
                  depth={1}
                  selected={selectedSession?.path === sub.path}
                  onSelect={onSelectSession}
                />
              ))}
            </React.Fragment>
          ))}
          {orphans.map((s) => (
            <SessionItem key={s.path} s={s} depth={0} selected={selectedSession?.path === s.path} onSelect={onSelectSession} />
          ))}
        </div>
      )}
    </div>
  );
}
