import React, { useCallback, useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";
import type { PiEvent, Project, RunningSession, SessionDetail, SessionMeta } from "./types";
import { Sidebar } from "./components/Sidebar";
import { Thread, appendLiveEvents, type LiveBlock } from "./components/Thread";
import { Composer } from "./components/Composer";
import { ConfigPanel } from "./components/ConfigPanel";

interface Toast {
  id: number;
  path: string;
  projectKey: string;
  title: string;
  isSubagent: boolean;
  paused: boolean;
  interrupted: boolean;
}

// ---- identity helpers --------------------------------------------------------
// The 10s poll re-fetches projects/sessions every tick. Without a guard every
// poll replaced the arrays with fresh objects, re-rendering the whole sidebar
// (hundreds of SessionItems) even when nothing changed — and during streaming
// it re-rendered on every live event. Return the previous array when the
// fields the UI renders are all equal, so memoized views keep their identity.

function sameSessions(a: SessionMeta[], b: SessionMeta[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.path !== y.path ||
      x.updatedAt !== y.updatedAt ||
      x.running !== y.running ||
      x.inRmux !== y.inRmux ||
      x.termAlive !== y.termAlive ||
      x.rmuxAttached !== y.rmuxAttached ||
      x.rmuxDead !== y.rmuxDead ||
      x.sleeping !== y.sleeping ||
      x.interrupted !== y.interrupted ||
      x.isSubagent !== y.isSubagent ||
      x.taskId !== y.taskId ||
      x.parentSessionPath !== y.parentSessionPath ||
      x.name !== y.name ||
      x.lastMessage !== y.lastMessage
    )
      return false;
  }
  return true;
}

function sameProjects(a: Project[], b: Project[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    if (
      x.key !== y.key ||
      x.cwd !== y.cwd ||
      x.sessionCount !== y.sessionCount ||
      x.subagentCount !== y.subagentCount ||
      x.runningCount !== y.runningCount ||
      x.rmuxCount !== y.rmuxCount ||
      x.termCount !== y.termCount ||
      x.updatedAt !== y.updatedAt
    )
      return false;
  }
  return true;
}

export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [loadingSessions, setLoadingSessions] = useState(false);
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [running, setRunning] = useState(false);
  const [liveBlocks, setLiveBlocks] = useState<LiveBlock[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [piInfo, setPiInfo] = useState<string>("");
  // per-session draft: draft text bound to each session path
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [showConfig, setShowConfig] = useState(false);
  const [remoteHosts, setRemoteHosts] = useState<string[]>([]);
  const [remoteHost, setRemoteHost] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);

  const channelRef = useRef<Channel<PiEvent> | null>(null);
  const activePathRef = useRef<string | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  // path -> finish time (epoch ms) for the '✓ finished' sidebar chip
  const [finishedAt, setFinishedAt] = useState<Record<string, number>>({});
  // running snapshot from the previous poll (null = not seeded yet)
  const prevRunningRef = useRef<
    Map<string, { title: string; isSubagent: boolean; projectKey: string }> | null
  >(null);

  const addToast = useCallback(
    (path: string, projectKey: string, title: string, isSubagent: boolean, paused = false, interrupted = false) => {
      const id = ++toastIdRef.current;
      // persistent until dismissed/clicked; cap the stack at 8 (drop oldest)
      setToasts((ts) => [...ts, { id, path, projectKey, title, isSubagent, paused, interrupted }].slice(-8));
    },
    []
  );

  const openToastSession = (t: Toast) => {
    setToasts((ts) => ts.filter((x) => x.id !== t.id));
    if (t.projectKey && t.projectKey !== selectedProject) {
      setSelectedProject(t.projectKey);
      refreshSessions(t.projectKey);
    }
    setFinishedAt((m) => {
      const n = { ...m };
      delete n[t.path];
      return n;
    });
    loadDetail({ path: t.path } as SessionMeta);
  };

  // 比较两次 session_detail 内容是否实质相同(消息数 + 最后条目签名)。
  // 相同则保持旧对象引用,让 Thread 的 memo 命中,避免全量重渲染。
  const sameSessionContent = (a: SessionDetail, b: SessionDetail): boolean => {
    if (a.entries.length !== b.entries.length) return false;
    if (a.active.length !== b.active.length) return false;
    const la = a.entries[a.entries.length - 1];
    const lb = b.entries[b.entries.length - 1];
    if (!la || !lb) return a.entries.length === b.entries.length && a.active.length === b.active.length;
    return la.id === lb.id && la.ts === lb.ts;
  };

  const refreshProjects = useCallback(async () => {
    try {
      const ps = await api.listProjects();
      setProjects((prev) => (sameProjects(prev, ps) ? prev : ps));
      setSelectedProject((cur) => cur ?? ps[0]?.key ?? null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const refreshSessions = useCallback(async (projectKey: string) => {
    setLoadingSessions(true);
    try {
      const ss = await api.listSessions(projectKey);
      setSessions((prev) => (sameSessions(prev, ss) ? prev : ss));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingSessions(false);
    }
  }, []);

  // silent variant for the polling loop (no loading flicker)
  const refreshSessionsSilent = useCallback(async (projectKey: string) => {
    try {
      const ss = await api.listSessions(projectKey);
      setSessions((prev) => (sameSessions(prev, ss) ? prev : ss));
    } catch {
      // ignore transient errors during polling
    }
  }, []);

  const loadDetail = useCallback(async (s: SessionMeta) => {
    activePathRef.current = s.path;
    setRunning(s.running);
    setLiveBlocks([]);
    try {
      const d = await api.sessionDetail(s.path);
      setDetail(d);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    refreshProjects();
    getCurrentWindow()
      .onFocusChanged(({ payload }) => {
        if (payload) refreshProjects();
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    api
      .piBinPath()
      .then((p) => api.piVersion().then((v) => setPiInfo(`${p} (v${v})`)).catch(() => setPiInfo(p ?? "pi not found")))
      .catch(() => setPiInfo("pi not found"));
    api
      .getRemoteHost()
      .then((h) => setRemoteHost(h))
      .catch(() => {});
    api
      .listRemoteHosts()
      .then((hs) => setRemoteHosts(hs))
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshProjects]);

  useEffect(() => {
    if (selectedProject) refreshSessions(selectedProject);
  }, [selectedProject, refreshSessions]);

  // auto-refresh: keep running badges / subagent states live
  // (detailRef keeps the latest detail visible to the interval closure even
  // when the effect doesn't re-run — detail content updates without a path change)
  const detailRef = useRef<SessionDetail | null>(null);
  detailRef.current = detail;
  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    const poll = async () => {
      // Recursive setTimeout guarantees that a slow filesystem scan cannot
      // overlap the next scan and multiply CPU usage. Background/minimized
      // windows do not need live badges; focus triggers a refresh separately.
      if (!document.hidden) {
        try {
          const ps = await api.listProjects();
          setProjects((prev) => (sameProjects(prev, ps) ? prev : ps));
          if (selectedProject) {
            const ss = await api.listSessions(selectedProject);
            setSessions((prev) => (sameSessions(prev, ss) ? prev : ss));
            const curDetail = detailRef.current;
            if (curDetail) {
              const cur = ss.find((s) => s.path === curDetail.path);
              if (cur?.running && (cur.size !== curDetail.size || cur.updatedAt !== curDetail.updatedAt)) {
                const d = await api.sessionDetail(curDetail.path);
                setDetail((prev) => {
                  if (prev && sameSessionContent(prev, d)) return prev;
                  return d;
                });
              }
            }
          }
          const running = await api.listRunning();
          const runningPaths = new Set(running.map((r) => r.path));
          const prev = prevRunningRef.current;
          if (prev) {
            for (const [path, info] of prev) {
              if (!runningPaths.has(path)) {
                let status = "finished";
                try {
                  status = await api.sessionStatus(path);
                } catch {
                  /* fall back to finished */
                }
                if (status === "finished") {
                  setFinishedAt((m) => ({ ...m, [path]: Date.now() }));
                }
                addToast(
                  path,
                  info.projectKey,
                  info.title,
                  info.isSubagent,
                  status === "sleeping",
                  status === "interrupted"
                );
              }
            }
          }
          prevRunningRef.current = new Map(
            running.map((r) => [
              r.path,
              { title: r.title, isSubagent: r.isSubagent, projectKey: r.projectKey },
            ])
          );
        } catch {
          // ignore transient polling errors
        }
      }
      if (!cancelled) timer = window.setTimeout(poll, 10000);
    };
    timer = window.setTimeout(poll, 10000);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [selectedProject, detail?.path, addToast]);

  // auto-open the most recent session on first load (like `pi -c`)
  const autoOpened = useRef(false);
  useEffect(() => {
    if (!autoOpened.current && sessions.length > 0 && !detail) {
      autoOpened.current = true;
      loadDetail(sessions[0]);
    }
  }, [sessions, detail, loadDetail]);

  const selectProject = useCallback((p: Project) => {
    setSelectedProject(p.key);
    setDetail(null);
    setShowConfig(false);
  }, []);

  const selectSession = useCallback(
    (s: SessionMeta) => {
      setShowConfig(false);
      setFinishedAt((m) => {
        const n = { ...m };
        delete n[s.path];
        return n;
      });
      loadDetail(s);
    },
    [loadDetail]
  );

  const openConfig = useCallback(() => {
    setShowConfig((v) => !v);
  }, []);

  // ---- live event batching ------------------------------------------------
  // Keep only events that have not yet been reduced into blocks. The old path
  // retained and reprocessed the entire raw stream on every animation frame,
  // turning long responses into O(n²) work and up to 60 markdown renders/s.
  const pendingEventsRef = useRef<any[]>([]);
  const liveTimerRef = useRef(0);
  const flushLive = useCallback(() => {
    liveTimerRef.current = 0;
    const batch = pendingEventsRef.current.splice(0);
    if (batch.length) setLiveBlocks((prev) => appendLiveEvents(prev, batch));
  }, []);

  const send = async (msg: string) => {
    if (!detail || running) return;
    pendingEventsRef.current = [];
    if (liveTimerRef.current) window.clearTimeout(liveTimerRef.current);
    liveTimerRef.current = 0;
    const channel = new Channel<PiEvent>();
    channel.onmessage = (ev) => {
      pendingEventsRef.current.push(ev);
      if (ev.type === "process_exit") {
        onTurnDone(activePathRef.current);
      } else if (!liveTimerRef.current) {
        // 12.5 updates/s is visually smooth for text while avoiding a full
        // markdown/layout pass for every token.
        liveTimerRef.current = window.setTimeout(flushLive, 80);
      }
    };
    activePathRef.current = detail.path;
    setRunning(true);
    setLiveBlocks([]);
    try {
      await api.sendMessage(detail.path, msg, channel);
    } catch (e) {
      setRunning(false);
      setError(String(e));
    }
  };

  const onTurnDone = async (path: string | null) => {
    if (!path) return;
    setRunning(false);
    setLiveBlocks([]);
    pendingEventsRef.current = [];
    if (liveTimerRef.current) window.clearTimeout(liveTimerRef.current);
    liveTimerRef.current = 0;
    try {
      if (activePathRef.current === path) {
        setDetail(await api.sessionDetail(path));
      }
    } catch (e) {
      setError(String(e));
    }
    refreshProjects();
    if (selectedProject) refreshSessions(selectedProject);
  };

  const sessionTitle = (path: string): string => {
    const s = sessions.find((x) => x.path === path);
    if (!s) return path.split("/").pop() ?? path;
    const t = s.name || s.firstMessage || "(empty)";
    return t.length > 40 ? t.slice(0, 40) + "…" : t;
  };

  const abort = async () => {
    const p = activePathRef.current;
    if (!p) return;
    try {
      await api.abortMessage(p);
      setRunning(false);
      pendingEventsRef.current = [];
      if (liveTimerRef.current) window.clearTimeout(liveTimerRef.current);
      liveTimerRef.current = 0;
      if (p === detail?.path) {
        setDetail(await api.sessionDetail(p));
        setLiveBlocks([]);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  // stable sidebar handlers (memoized Sidebar compares props by identity)
  const onOpenTerminal = useCallback((path: string) => {
    api.openInTerminal(path).catch((e) => setError(String(e)));
  }, []);
  const onDetachFromRmux = useCallback(
    async (path: string) => {
      try {
        await api.detachFromRmux(path);
        if (selectedProject) refreshSessions(selectedProject);
      } catch (e) {
        setError(String(e));
      }
    },
    [selectedProject, refreshSessions]
  );
  const onKillRmuxSession = useCallback(
    async (path: string) => {
      try {
        await api.killRmuxSession(path);
        refreshProjects();
        if (selectedProject) refreshSessions(selectedProject);
      } catch (e) {
        setError(String(e));
      }
    },
    [selectedProject, refreshProjects, refreshSessions]
  );
  const onDeleteSession = useCallback(
    async (path: string) => {
      try {
        await api.deleteSession(path);
        refreshProjects();
        if (selectedProject) refreshSessions(selectedProject);
        if (detail?.path === path) setDetail(null);
      } catch (e) {
        setError(String(e));
      }
    },
    [detail?.path, selectedProject, refreshProjects, refreshSessions]
  );
  const onTransferToRemote = useCallback(
    async (path: string) => {
      const hosts = remoteHosts.length ? remoteHosts : await api.listRemoteHosts();
      if (!hosts.length) {
        setError("No remote hosts configured (~/.pi-session-viewer.json)");
        return;
      }
      const host = hosts.length === 1 ? hosts[0] : (prompt(`Transfer to which host?\n${hosts.join("\n")}`, hosts[0]) ?? "");
      if (!host) return;
      // default guess: mac-mini-style layout ~/Project/<basename>
      const detail = await api.sessionDetail(path);
      const base = (detail.cwd || "").split("/").filter(Boolean).pop() || "project";
      const remoteCwd = prompt(
        `Remote working directory on ${host}:`,
        `~/Project/${base}`
      );
      if (!remoteCwd) return;
      const msg = prompt("First message to send after transfer (empty = just resume interactively):", "") ?? "";
      try {
        setSyncing(true);
        const sess = await api.transferSessionToRemote(path, host, remoteCwd, msg);
        alert(
          `Transferred to ${host}.\nrmux session: ${sess}\nAttach locally: ssh -t ${host} 'rmux attach -t ${sess}'`
        );
      } catch (e) {
        setError(String(e));
      } finally {
        setSyncing(false);
      }
    },
    [remoteHosts]
  );
  const onRefresh = useCallback(() => {
    const reloadAll = () => {
      refreshProjects();
      if (selectedProject) refreshSessions(selectedProject);
      if (detail) loadDetail({ path: detail.path } as SessionMeta);
    };
    if (remoteHost) {
      api
        .refreshRemote()
        .then(reloadAll)
        .catch((e) => setError(String(e)));
    } else {
      reloadAll();
    }
  }, [remoteHost, selectedProject, refreshProjects, refreshSessions, loadDetail, detail]);
  const onSwitchSource = useCallback(async (host: string | null) => {
    setSyncing(true);
    try {
      await api.setRemoteHost(host);
      setRemoteHost(host);
      setSelectedProject(null);
      setDetail(null);
      await refreshProjects();
    } catch (e) {
      setError(String(e));
    } finally {
      setSyncing(false);
    }
  }, [refreshProjects]);

  return (
    <div className="app">
      {/* finished-session toasts */}
      <div className="toast-stack">
        {toasts.length > 1 && (
          <button className="toast-clear-all" onClick={() => setToasts([])}>
            clear all
          </button>
        )}
        {toasts.map((t) => (
          <button key={t.id} className={`toast ${t.paused ? "paused" : ""} ${t.interrupted ? "interrupted" : ""}`} onClick={() => openToastSession(t)}>
            <span className="toast-body">
              <span className="toast-title">
                {t.interrupted
                  ? t.isSubagent
                    ? "Subagent interrupted"
                    : "Session interrupted"
                  : t.paused
                    ? t.isSubagent
                      ? "Subagent paused"
                      : "Session paused"
                    : t.isSubagent
                      ? "Subagent finished"
                      : "Session finished"}
              </span>
              <span className="toast-text">{t.title}</span>
            </span>
            <span className="toast-close" onClick={(e) => { e.stopPropagation(); setToasts((ts) => ts.filter((x) => x.id !== t.id)); }}>×</span>
          </button>
        ))}
      </div>
      <Sidebar
        remoteHosts={remoteHosts}
        remoteHost={remoteHost}
        syncing={syncing}
        onSwitchSource={onSwitchSource}
        projects={projects}
        sessions={sessions}
        selectedProject={selectedProject}
        selectedSessionPath={detail?.path ?? null}
        loadingSessions={loadingSessions}
        onSelectProject={selectProject}
        onSelectSession={selectSession}
        onOpenConfig={openConfig}
        showConfig={showConfig}
        finishedAt={finishedAt}
        onOpenTerminal={onOpenTerminal}
        onDetachFromRmux={onDetachFromRmux}
        onKillRmuxSession={onKillRmuxSession}
        onDeleteSession={onDeleteSession}
        onTransferToRemote={onTransferToRemote}
        onRefresh={onRefresh}
      />
      <div className="main">
        {error && (
          <div className="error-bar" onClick={() => setError(null)}>
            ⚠️ {error}
          </div>
        )}
        {showConfig ? (
          <ConfigPanel />
        ) : detail ? (
          <>
            <Thread detail={detail} liveBlocks={liveBlocks} running={running} />
            <Composer
              value={drafts[detail.path] ?? ""}
              onChange={(v) =>
                setDrafts((d) => ({ ...d, [detail.path]: v }))
              }
              running={running}
              targetName={sessionTitle(detail.path)}
              onSend={async (msg) => {
                try {
                  await send(msg);
                  // clear draft only after the send was accepted
                  setDrafts((d) => ({ ...d, [detail.path]: "" }));
                } catch {
                  /* keep draft on failure */
                }
              }}
              onAbort={abort}
            />
          </>
        ) : (
          <div className="empty-main">
            <h2>Pi Desktop</h2>
            <p>Select a session to view or continue</p>
            {piInfo && <p className="pi-info">{piInfo}</p>}
          </div>
        )}
      </div>
    </div>
  );
}
