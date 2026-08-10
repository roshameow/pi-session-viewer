import React, { useMemo, useState } from "react";
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
  runningSubs = 0,
  hasSubs = false,
  subsCollapsed = false,
  onToggleSubs,
}: {
  s: SessionMeta;
  depth: number;
  selected: boolean;
  onSelect: (s: SessionMeta) => void;
  runningSubs?: number;
  hasSubs?: boolean;
  subsCollapsed?: boolean;
  onToggleSubs?: () => void;
}) {
  const title = s.name || s.firstMessage || "(空会话)";
  return (
    <button
      className={`session-item ${selected ? "selected" : ""} ${s.isSubagent ? "sub" : "main"} ${s.running ? "running" : ""}`}
      style={{ paddingLeft: 8 + depth * 14 }}
      onClick={() => onSelect(s)}
      title={`${s.cwd}\n${s.path}`}
    >
      <div className="session-item-line1">
        {!s.isSubagent && hasSubs && (
          <span
            className={`group-toggle ${subsCollapsed ? "collapsed" : ""}`}
            title={subsCollapsed ? "展开子代理" : "折叠子代理"}
            onClick={(e) => {
              e.stopPropagation();
              onToggleSubs?.();
            }}
          >
            {subsCollapsed ? "▸" : "▾"}
          </span>
        )}
        {s.running && <span className="pulse-dot" />}
        <span className="session-icon">{s.isSubagent ? "🕸️" : "💬"}</span>
        <span className="session-title">{title}</span>
        {s.isSubagent && <span className="sub-chip">SUB</span>}
        {!s.isSubagent && runningSubs > 0 && (
          <span className="subs-running-badge" title={`${runningSubs} 个子代理运行中`}>
            🕸️ {runningSubs} 运行中
          </span>
        )}
      </div>
      <div className="session-item-line2">
        <span className="session-time">{relTime(s.updatedAt)}</span>
        {s.model && <span className="session-model">{s.model}</span>}
        {s.isSubagent && s.taskId && <span className="session-task">task:{s.taskId}</span>}
        {s.isSubagent && s.running && <span className="sub-running-chip">● 运行中</span>}
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
  const [collapsedMain, setCollapsedMain] = useState(false);
  const [collapsedSub, setCollapsedSub] = useState(false);
  const [collapsedProjects, setCollapsedProjects] = useState(false);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());

  const toggleGroup = (path: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const { mainSessions, childrenMap, subagents } = useMemo(() => {
    const main: SessionMeta[] = [];
    const children = new Map<string, SessionMeta[]>();
    const subs: SessionMeta[] = [];
    for (const s of sessions) {
      if (!s.isSubagent) {
        main.push(s);
        continue;
      }
      subs.push(s);
      if (s.parentSessionPath && sessions.some((m) => !m.isSubagent && m.path === s.parentSessionPath)) {
        const list = children.get(s.parentSessionPath) ?? [];
        list.push(s);
        children.set(s.parentSessionPath, list);
      }
    }
    return { mainSessions: main, childrenMap: children, subagents: subs };
  }, [sessions]);

  const parentTitle = (path: string | null): string | null => {
    if (!path) return null;
    const m = sessions.find((s) => s.path === path);
    if (!m) return null;
    const t = m.name || m.firstMessage || "(父会话)";
    return t.length > 26 ? t.slice(0, 26) + "…" : t;
  };

  return (
    <div className="sidebar">
      <div className="sidebar-top">
        <span className="app-title">Pi Desktop</span>
        <button className="icon-btn" title="刷新" onClick={onRefresh}>
          ⟳
        </button>
      </div>

      {/* projects (collapsible) */}
      <div className="project-list">
        <div
          className={`section-head ${collapsedProjects ? "collapsed" : ""}`}
          onClick={() => setCollapsedProjects(!collapsedProjects)}
        >
          <span className="section-arrow">{collapsedProjects ? "▸" : "▾"}</span>
          <span>项目</span>
          <span className="section-count">{projects.length}</span>
        </div>
        {!collapsedProjects && (
          <>
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
          </>
        )}
      </div>

      {/* sessions of selected project */}
      {selectedProject && (
        <div className="session-list">
          {loadingSessions ? (
            <div className="empty">加载中…</div>
          ) : (
            <>
              {/* main sessions */}
              <div
                className={`section-head ${collapsedMain ? "collapsed" : ""}`}
                onClick={() => setCollapsedMain(!collapsedMain)}
              >
                <span className="section-arrow">{collapsedMain ? "▸" : "▾"}</span>
                <span>主会话</span>
                <span className="section-count">{mainSessions.length}</span>
              </div>
              {!collapsedMain &&
                mainSessions.map((s) => {
                  const subs = childrenMap.get(s.path) ?? [];
                  const runningSubs = subs.filter((x) => x.running).length;
                  const groupCollapsed = collapsedGroups.has(s.path);
                  return (
                    <React.Fragment key={s.path}>
                      <SessionItem
                        s={s}
                        depth={0}
                        selected={selectedSession?.path === s.path}
                        onSelect={onSelectSession}
                        runningSubs={runningSubs}
                        hasSubs={subs.length > 0}
                        subsCollapsed={groupCollapsed}
                        onToggleSubs={() => toggleGroup(s.path)}
                      />
                      {subs.length > 0 && (
                        <div className={`subagent-group ${groupCollapsed ? "collapsed" : ""}`}>
                          <div
                            className="subagent-group-head"
                            onClick={() => toggleGroup(s.path)}
                            title={groupCollapsed ? "展开" : "折叠"}
                          >
                            <span className="section-arrow">{groupCollapsed ? "▸" : "▾"}</span>
                            <span>🕸️ 子代理</span>
                            <span className="subagent-count">{subs.length}</span>
                            {subs.some((x) => x.running) && (
                              <span className="subagent-running">● 运行中</span>
                            )}
                          </div>
                          {!groupCollapsed &&
                            subs.map((sub) => (
                              <SessionItem
                                key={sub.path}
                                s={sub}
                                depth={0}
                                selected={selectedSession?.path === sub.path}
                                onSelect={onSelectSession}
                              />
                            ))}
                        </div>
                      )}
                    </React.Fragment>
                  );
                })}

              {/* subagent sessions (dedicated section) */}
              <div
                className={`section-head sub ${collapsedSub ? "collapsed" : ""}`}
                onClick={() => setCollapsedSub(!collapsedSub)}
              >
                <span className="section-arrow">{collapsedSub ? "▸" : "▾"}</span>
                <span>子代理会话</span>
                <span className="section-count">{subagents.length}</span>
              </div>
              {!collapsedSub &&
                subagents.map((s) => {
                  const pt = parentTitle(s.parentSessionPath);
                  return (
                    <React.Fragment key={s.path}>
                      <SessionItem
                        s={s}
                        depth={0}
                        selected={selectedSession?.path === s.path}
                        onSelect={onSelectSession}
                      />
                      <div className="subagent-parent">
                        {pt ? <>父: {pt}</> : <span className="orphan">未关联父会话</span>}
                      </div>
                    </React.Fragment>
                  );
                })}
            </>
          )}
        </div>
      )}
    </div>
  );
}
