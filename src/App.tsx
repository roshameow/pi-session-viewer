import React, { useCallback, useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";
import type { PiEvent, Project, RunningSession, SessionDetail, SessionMeta } from "./types";
import { Sidebar } from "./components/Sidebar";
import { Thread, buildLiveBlocks, type LiveBlock } from "./components/Thread";
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

export default function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [sessions, setSessions] = useState<SessionMeta[]>([]);
  const [loadingSessions, setLoadingSessions] = useState(false);
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [running, setRunning] = useState(false);
  const [liveEvents, setLiveEvents] = useState<any[]>([]);
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

  const switchSource = async (host: string | null) => {
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
  };

  const refreshProjects = useCallback(async () => {
    try {
      const ps = await api.listProjects();
      setProjects(ps);
      setSelectedProject((cur) => cur ?? ps[0]?.key ?? null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const refreshSessions = useCallback(async (projectKey: string) => {
    setLoadingSessions(true);
    try {
      const ss = await api.listSessions(projectKey);
      setSessions(ss);
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
      setSessions(ss);
    } catch {
      // ignore transient errors during polling
    }
  }, []);

  const loadDetail = useCallback(async (s: SessionMeta) => {
    activePathRef.current = s.path;
    setRunning(s.running);
    setLiveEvents([]);
    try {
      const d = await api.sessionDetail(s.path);
      setDetail(d);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refreshProjects();
    getCurrentWindow().onFocusChanged(({ payload }) => {
      if (payload) refreshProjects();
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
  }, [refreshProjects]);

  useEffect(() => {
    if (selectedProject) refreshSessions(selectedProject);
  }, [selectedProject, refreshSessions]);

  // auto-refresh: keep running badges / subagent states live
  useEffect(() => {
    const t = setInterval(async () => {
      try {
        const ps = await api.listProjects();
        setProjects(ps);
        if (selectedProject) {
          const ss = await api.listSessions(selectedProject);
          setSessions(ss);
          // live-update the detail view when the selected session is running
          if (detail) {
            const cur = ss.find((s) => s.path === detail.path);
            if (cur?.running) {
              const d = await api.sessionDetail(detail.path);
              setDetail(d);
            }
          }
        }
        // notify when a previously-running session finished / was paused
        const running = await api.listRunning();
        const prev = prevRunningRef.current;
        if (prev) {
          for (const [path, info] of prev) {
            if (!running.some((r) => r.path === path)) {
              // classify: sleeping (paused) vs finished
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
    }, 10000);
    return () => clearInterval(t);
  }, [selectedProject, detail?.path, addToast]);

  // auto-open the most recent session on first load (like `pi -c`)
  const autoOpened = useRef(false);
  useEffect(() => {
    if (!autoOpened.current && sessions.length > 0 && !detail) {
      autoOpened.current = true;
      loadDetail(sessions[0]);
    }
  }, [sessions, detail, loadDetail]);

  const selectProject = (p: Project) => {
    setSelectedProject(p.key);
    setDetail(null);
    setShowConfig(false);
  };

  const selectSession = (s: SessionMeta) => {
    setShowConfig(false);
    setFinishedAt((m) => {
      const n = { ...m };
      delete n[s.path];
      return n;
    });
    loadDetail(s);
  };

  const openConfig = () => {
    setShowConfig((v) => !v);
  };

  const send = async (msg: string) => {
    if (!detail || running) return;
    const evs: any[] = [];
    const channel = new Channel<PiEvent>();
    channel.onmessage = (ev) => {
      evs.push(ev);
      setLiveEvents([...evs]);
      if (ev.type === "process_exit") {
        onTurnDone(activePathRef.current);
      }
    };
    activePathRef.current = detail.path;
    setRunning(true);
    setLiveEvents([]);
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
    setLiveEvents([]);
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
      if (p === detail?.path) {
        setDetail(await api.sessionDetail(p));
        setLiveEvents([]);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const liveBlocks: LiveBlock[] = buildLiveBlocks(liveEvents);

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
        onSwitchSource={switchSource}
        projects={projects}
        sessions={sessions}
        selectedProject={selectedProject}
        selectedSession={detail ? { ...({ path: detail.path } as SessionMeta) } : null}
        loadingSessions={loadingSessions}
        onSelectProject={selectProject}
        onSelectSession={selectSession}
        onOpenConfig={openConfig}
        showConfig={showConfig}
        finishedAt={finishedAt}
        onOpenTerminal={(path) => {
          api.openInTerminal(path).catch((e) => setError(String(e)));
        }}
        onDetachFromRmux={async (path) => {
          try {
            await api.detachFromRmux(path);
            if (selectedProject) refreshSessions(selectedProject);
          } catch (e) {
            setError(String(e));
          }
        }}
        onKillRmuxSession={async (path) => {
          try {
            await api.killRmuxSession(path);
            refreshProjects();
            if (selectedProject) refreshSessions(selectedProject);
          } catch (e) {
            setError(String(e));
          }
        }}
        onDeleteSession={async (path) => {
          try {
            await api.deleteSession(path);
            refreshProjects();
            if (selectedProject) refreshSessions(selectedProject);
            if (detail?.path === path) setDetail(null);
          } catch (e) {
            setError(String(e));
          }
        }}
        onRefresh={() => {
          if (remoteHost) {
            api
              .refreshRemote()
              .then(() => {
                refreshProjects();
                if (selectedProject) refreshSessions(selectedProject);
                if (detail) loadDetail({ path: detail.path } as SessionMeta);
              })
              .catch((e) => setError(String(e)));
          } else {
            refreshProjects();
            if (selectedProject) refreshSessions(selectedProject);
            if (detail) loadDetail({ path: detail.path } as SessionMeta);
          }
        }}
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
