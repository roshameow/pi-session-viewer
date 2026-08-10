import React, { useCallback, useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";
import type { PiEvent, Project, RunningSession, SessionDetail, SessionMeta } from "./types";
import { Sidebar } from "./components/Sidebar";
import { Thread, buildLiveBlocks, type LiveBlock } from "./components/Thread";
import { Composer } from "./components/Composer";

interface Toast {
  id: number;
  path: string;
  projectKey: string;
  title: string;
  isSubagent: boolean;
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

  const channelRef = useRef<Channel<PiEvent> | null>(null);
  const activePathRef = useRef<string | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  // running snapshot from the previous poll (null = not seeded yet)
  const prevRunningRef = useRef<
    Map<string, { title: string; isSubagent: boolean; projectKey: string }> | null
  >(null);

  const addToast = useCallback(
    (path: string, projectKey: string, title: string, isSubagent: boolean) => {
      const id = ++toastIdRef.current;
      setToasts((ts) => [...ts, { id, path, projectKey, title, isSubagent }]);
      setTimeout(() => {
        setToasts((ts) => ts.filter((t) => t.id !== id));
      }, 8000);
    },
    []
  );

  const openToastSession = (t: Toast) => {
    setToasts((ts) => ts.filter((x) => x.id !== t.id));
    if (t.projectKey && t.projectKey !== selectedProject) {
      setSelectedProject(t.projectKey);
      refreshSessions(t.projectKey);
    }
    loadDetail({ path: t.path } as SessionMeta);
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
        // notify when a previously-running session finished
        const running = await api.listRunning();
        const prev = prevRunningRef.current;
        if (prev) {
          for (const [path, info] of prev) {
            if (!running.some((r) => r.path === path)) {
              addToast(path, info.projectKey, info.title, info.isSubagent);
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
  };

  const selectSession = (s: SessionMeta) => {
    loadDetail(s);
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
        {toasts.map((t) => (
          <button key={t.id} className="toast" onClick={() => openToastSession(t)}>
            <span className="toast-icon">{t.isSubagent ? "🕸️" : "💬"}</span>
            <span className="toast-body">
              <span className="toast-title">{t.isSubagent ? "Subagent finished" : "Session finished"}</span>
              <span className="toast-text">{t.title}</span>
            </span>
            <span className="toast-close" onClick={(e) => { e.stopPropagation(); setToasts((ts) => ts.filter((x) => x.id !== t.id)); }}>×</span>
          </button>
        ))}
      </div>
      <Sidebar
        projects={projects}
        sessions={sessions}
        selectedProject={selectedProject}
        selectedSession={detail ? { ...({ path: detail.path } as SessionMeta) } : null}
        loadingSessions={loadingSessions}
        onSelectProject={selectProject}
        onSelectSession={selectSession}
        onRefresh={() => {
          refreshProjects();
          if (selectedProject) refreshSessions(selectedProject);
          if (detail) loadDetail({ path: detail.path } as SessionMeta);
        }}
      />
      <div className="main">
        {error && (
          <div className="error-bar" onClick={() => setError(null)}>
            ⚠️ {error}
          </div>
        )}
        {detail ? (
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
