import React, { useEffect, useMemo, useRef, useState } from "react";
import type { Project, SessionMeta } from "../types";

const MIN_SIDEBAR = 200;
const MAX_SIDEBAR = 640;

function relTime(epochSec: number): string {
  if (!epochSec) return "";
  const diff = Date.now() / 1000 - epochSec;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)}d ago`;
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
  onContextMenu,
  runningSubs = 0,
  sleepingSubs = 0,
  interruptedSubs = 0,
  hasSubs = false,
  subsCollapsed = false,
  onToggleSubs,
}: {
  s: SessionMeta;
  depth: number;
  selected: boolean;
  onSelect: (s: SessionMeta) => void;
  onContextMenu?: (s: SessionMeta, x: number, y: number) => void;
  runningSubs?: number;
  sleepingSubs?: number;
  interruptedSubs?: number;
  hasSubs?: boolean;
  subsCollapsed?: boolean;
  onToggleSubs?: () => void;
}) {
  const title = s.name || s.firstMessage || "(empty)";
  const statusClass = s.running
    ? "status-running"
    : s.isSubagent
      ? s.sleeping
        ? "status-sleeping"
        : s.interrupted
          ? "status-interrupted"
          : ""
      : "";
  return (
    <button
      className={`session-item ${selected ? "selected" : ""} ${s.isSubagent ? "sub" : "main"} ${statusClass}`}
      style={{ paddingLeft: 8 + depth * 14 }}
      onClick={() => onSelect(s)}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu?.(s, e.clientX, e.clientY);
      }}
      title={`${s.cwd}\n${s.path}\n(right-click for options)`}
    >
      <div className="session-item-line1">
        {!s.isSubagent && hasSubs && (
          <span
            className={`group-toggle ${subsCollapsed ? "collapsed" : ""}`}
            title={subsCollapsed ? "Expand subagents" : "Collapse subagents"}
            onClick={(e) => {
              e.stopPropagation();
              onToggleSubs?.();
            }}
          >
            {subsCollapsed ? "▸" : "▾"}
          </span>
        )}
        {s.running && <span className="pulse-dot" />}
        <span className="session-title">{title}</span>
        {s.isSubagent && <span className="sub-chip">SUB</span>}
        {!s.isSubagent && runningSubs > 0 && (
          <span className="subs-running-badge" title={`${runningSubs} subagent(s) running`}>
            {runningSubs} running
          </span>
        )}
        {!s.isSubagent && sleepingSubs > 0 && (
          <span className="subs-sleeping-badge" title={`${sleepingSubs} subagent(s) sleeping`}>
            {sleepingSubs} sleeping
          </span>
        )}
        {!s.isSubagent && interruptedSubs > 0 && (
          <span className="subs-interrupted-badge" title={`${interruptedSubs} subagent(s) interrupted`}>
            {interruptedSubs} interrupted
          </span>
        )}
      </div>
      <div className="session-item-line2">
        <span className="session-time">{relTime(s.updatedAt)}</span>
        {!s.isSubagent && (
          <span className="session-id" title={`session id: ${s.id}`}>
            {s.id.slice(0, 13)}…
          </span>
        )}
        {s.isSubagent && s.running && <span className="sub-running-chip">● running</span>}
        {s.isSubagent && s.sleeping && (
          <span className="sub-sleeping-chip" title="Process alive, waiting on a sleep — will auto-continue">
            ◐ sleeping
          </span>
        )}
        {s.isSubagent && s.interrupted && (
          <span className="sub-interrupted-chip" title="Process dead, no terminal event — resumable via subagent_reload">
            ✕ interrupted
          </span>
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
  onOpenTerminal,
  onDeleteSession,
  onOpenConfig,
  showConfig,
}: {
  projects: Project[];
  sessions: SessionMeta[];
  selectedProject: string | null;
  selectedSession: SessionMeta | null;
  loadingSessions: boolean;
  onSelectProject: (p: Project) => void;
  onSelectSession: (s: SessionMeta) => void;
  onRefresh: () => void;
  onOpenTerminal: (path: string) => void;
  onDeleteSession: (path: string) => Promise<void> | void;
  onOpenConfig: () => void;
  showConfig: boolean;
}) {
  const [collapsedMain, setCollapsedMain] = useState(false);
  const [collapsedSub, setCollapsedSub] = useState(false);
  const [collapsedProjects, setCollapsedProjects] = useState(false);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [sessionQuery, setSessionQuery] = useState("");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; s: SessionMeta } | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  // close the context menu on outside left-click / Escape
  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setCtxMenu(null);
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [ctxMenu]);

  // resizable sidebar width (persisted)
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    const saved = Number(localStorage.getItem("pi-sidebar-width"));
    return Number.isFinite(saved) && saved > 0
      ? Math.min(Math.max(saved, MIN_SIDEBAR), MAX_SIDEBAR)
      : 300;
  });
  const widthRef = useRef(sidebarWidth);
  widthRef.current = sidebarWidth;
  const draggingRef = useRef(false);
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!draggingRef.current) return;
      setSidebarWidth(Math.min(Math.max(e.clientX, MIN_SIDEBAR), MAX_SIDEBAR));
    };
    const onUp = () => {
      if (!draggingRef.current) return;
      draggingRef.current = false;
      setDragging(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      localStorage.setItem("pi-sidebar-width", String(widthRef.current));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, []);

  const startDrag = (e: React.MouseEvent) => {
    e.preventDefault();
    draggingRef.current = true;
    setDragging(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };

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
    const q = sessionQuery.trim().toLowerCase();
    const match = (s: SessionMeta) =>
      !q ||
      (s.name ?? "").toLowerCase().includes(q) ||
      (s.firstMessage ?? "").toLowerCase().includes(q) ||
      (s.lastMessage ?? "").toLowerCase().includes(q) ||
      (s.taskId ?? "").toLowerCase().includes(q) ||
      s.id.toLowerCase().includes(q);
    for (const s of sessions) {
      if (!match(s)) continue;
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
  }, [sessions, sessionQuery]);

  const parentTitle = (path: string | null): string | null => {
    if (!path) return null;
    const m = sessions.find((s) => s.path === path);
    if (!m) return null;
    const t = m.name || m.firstMessage || "(parent)";
    return t.length > 26 ? t.slice(0, 26) + "…" : t;
  };

  return (
    <div className="sidebar" style={{ width: sidebarWidth }}>
      <div
        className={`sidebar-resizer ${dragging ? "dragging" : ""}`}
        onMouseDown={startDrag}
        title="Drag to resize"
      />
      <div className="sidebar-top">
        <span className="app-title">Pi Desktop</span>
        <button
          className={`icon-btn ${showConfig ? "active" : ""}`}
          title="MCP / Agents / Skills config"
          onClick={onOpenConfig}
        >
          ⚙️
        </button>
        <button className="icon-btn" title="Refresh" onClick={onRefresh}>
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
          <span>Projects</span>
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

                <span className="project-name">{projectName(p.cwd)}</span>
                <span
                  className="project-count"
                  title={`${p.sessionCount} sessions · ${p.subagentCount} subagents`}
                >
                  {p.sessionCount}
                </span>
                {p.runningCount > 0 && (
                  <span className="project-running" title={`${p.runningCount} session(s) running`}>
                    ● {p.runningCount}
                  </span>
                )}
              </button>
            ))}
            {projects.length === 0 && <div className="empty">No pi sessions found</div>}
          </>
        )}
      </div>

      {/* sessions of selected project */}
      {selectedProject && (
        <div className="session-list">
          <input
            className="session-search"
            placeholder="Search sessions… (title / content)"
            value={sessionQuery}
            onChange={(e) => setSessionQuery(e.target.value)}
          />
          {loadingSessions ? (
            <div className="empty">Loading…</div>
          ) : (
            <>
              {/* main sessions */}
              <div
                className={`section-head ${collapsedMain ? "collapsed" : ""}`}
                onClick={() => setCollapsedMain(!collapsedMain)}
              >
                <span className="section-arrow">{collapsedMain ? "▸" : "▾"}</span>
                <span>Main sessions</span>
                <span className="section-count">{mainSessions.length}</span>
              </div>
              {!collapsedMain &&
                mainSessions.map((s) => {
                  const subs = childrenMap.get(s.path) ?? [];
                  const runningSubs = subs.filter((x) => x.running).length;
                  const sleepingSubs = subs.filter((x) => x.sleeping).length;
                  const interruptedSubs = subs.filter((x) => x.interrupted).length;
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
                        sleepingSubs={sleepingSubs}
                        interruptedSubs={interruptedSubs}
                        onContextMenu={(s, x, y) => {
                          setConfirmingDelete(false);
                          setCtxMenu({ x, y, s });
                        }}
                      />
                      {subs.length > 0 && (
                        <div className={`subagent-group ${groupCollapsed ? "collapsed" : ""}`}>
                          <div
                            className="subagent-group-head"
                            onClick={() => toggleGroup(s.path)}
                            title={groupCollapsed ? "Expand" : "Collapse"}
                          >
                            <span className="section-arrow">{groupCollapsed ? "▸" : "▾"}</span>
                            <span>subagents</span>
                            <span className="subagent-count">{subs.length}</span>
                            {subs.some((x) => x.running) && (
                              <span className="subagent-running">● running</span>
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
                                onContextMenu={(sub, x, y) => {
                                  setConfirmingDelete(false);
                                  setCtxMenu({ x, y, s: sub });
                                }}
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
                <span>Subagent sessions</span>
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
                        onContextMenu={(s, x, y) => {
                          setConfirmingDelete(false);
                          setCtxMenu({ x, y, s });
                        }}
                      />
                      <div className="subagent-parent">
                        {pt ? <>parent: {pt}</> : <span className="orphan">no parent</span>}
                      </div>
                    </React.Fragment>
                  );
                })}

              {mainSessions.length === 0 && subagents.length === 0 && (
                <div className="empty">
                  {sessionQuery.trim() ? "No matching sessions" : "No sessions"}
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* right-click context menu */}
      {ctxMenu && (
        <div
          className="ctx-menu"
          style={{
            left: Math.min(ctxMenu.x, window.innerWidth - 220),
            top: Math.min(ctxMenu.y, window.innerHeight - 160),
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="ctx-title" title={ctxMenu.s.path}>
            {(ctxMenu.s.isSubagent ? "[sub] " : "") +
              (ctxMenu.s.name || ctxMenu.s.firstMessage || "(empty)").slice(0, 42)}
          </div>
          <button
            className="ctx-item"
            onClick={() => {
              onOpenTerminal(ctxMenu.s.path);
              setCtxMenu(null);
            }}
          >
            <span>⛭</span> Open in pi TUI (terminal)
          </button>
          <button
            className={`ctx-item danger ${confirmingDelete ? "confirm" : ""}`}
            onClick={async () => {
              if (!confirmingDelete) {
                setConfirmingDelete(true);
                return;
              }
              const p = ctxMenu.s.path;
              setCtxMenu(null);
              try {
                await onDeleteSession(p);
              } catch {
                /* handled by App */
              }
            }}
          >
            {confirmingDelete ? (
              <span className="ctx-danger-text">⚠️ Confirm delete (to Trash)?</span>
            ) : (
              <span>🗑 Delete session</span>
            )}
          </button>
        </div>
      )}
    </div>
  );
}
