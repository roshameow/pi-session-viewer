import React, { useCallback, useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";
import { afterViewPaint, RequestSequencer, type RequestToken } from "./requestSequencing";
import type { PiEvent, Project, RemoteSyncStatus, SessionDetail, SessionMeta } from "./types";
import { Sidebar } from "./components/Sidebar";
import { Thread, appendLiveEvents, type LiveBlock } from "./components/Thread";
import { Composer } from "./components/Composer";
import { ConfigPanel } from "./components/ConfigPanel";

interface Toast {
  sourceToken: RequestToken;
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
      x.rmuxPiAlive !== y.rmuxPiAlive ||
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
  const [piInfo, setPiInfo] = useState("");
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [showConfig, setShowConfig] = useState(false);
  const [remoteHosts, setRemoteHosts] = useState<string[]>([]);
  const [remoteHost, setRemoteHost] = useState<string | null>(null);
  const [remoteStatus, setRemoteStatus] = useState<RemoteSyncStatus | null>(null);
  const [sourcePending, setSourcePending] = useState(false);
  const [sourceNotice, setSourceNotice] = useState<{ host: string | null; error?: string } | null>(null);
  const [refreshingRemote, setRefreshingRemote] = useState(false);
  const sequenceRef = useRef(new RequestSequencer());
  const sourceRef = useRef<string | null>(null);
  const projectRef = useRef<string | null>(null);
  const activePathRef = useRef<string | null>(null);
  const sessionPathsRef = useRef<Set<string>>(new Set());
  const detailRef = useRef<SessionDetail | null>(null);
  detailRef.current = detail;
  const aliveRef = useRef(true);
  const paintCancelRef = useRef<(() => void) | null>(null);
  const pendingEventsRef = useRef<any[]>([]);
  const liveTimerRef = useRef(0);
  const autoOpened = useRef(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  const [finishedAt, setFinishedAt] = useState<Record<string, number>>({});
  const prevRunningRef = useRef<Map<string, { title: string; isSubagent: boolean; projectKey: string }> | null>(null);
  const valid = useCallback((token: RequestToken, pending = false) =>
    aliveRef.current && sequenceRef.current.current(token, pending), []);

  const clearLive = useCallback(() => {
    pendingEventsRef.current = [];
    if (liveTimerRef.current) window.clearTimeout(liveTimerRef.current);
    liveTimerRef.current = 0;
    setLiveBlocks([]);
    setRunning(false);
  }, []);

  // Refs/epochs change synchronously, before any render/effect can launch a scan.
  const changeProject = useCallback((key: string | null) => {
    if (projectRef.current === key) return;
    sequenceRef.current.selectProject(key);
    projectRef.current = key;
    activePathRef.current = null;
    detailRef.current = null;
    setSelectedProject(key);
    sessionPathsRef.current.clear();
    setSessions([]);
    setDetail(null);
    setLoadingSessions(false);
    clearLive();
  }, [clearLive]);

  const refreshProjects = useCallback(async (cancelled: () => boolean = () => false) => {
    const q = sequenceRef.current;
    const token = q.capture();
    if (cancelled() || !valid(token)) return;
    try {
      const ps = await q.request(token, "projects", api.listProjects);
      if (cancelled() || !valid(token)) return;
      setProjects(prev => sameProjects(prev, ps) ? prev : ps);
      // Do not resurrect a project removed by a refresh.
      if (!projectRef.current || !ps.some(p => p.key === projectRef.current)) {
        changeProject(ps[0]?.key ?? null);
      }
    } catch (e) {
      if (!cancelled() && valid(token)) setError(String(e));
    }
  }, [valid, changeProject]);

  const refreshSessions = useCallback(async (projectKey: string, silent = false, cancelled: () => boolean = () => false) => {
    const q = sequenceRef.current;
    const token = q.capture("project");
    if (cancelled() || !valid(token) || token.project !== projectKey) return;
    if (!silent) setLoadingSessions(true);
    try {
      const ss = await q.request(token, `sessions:${projectKey}`, () => api.listSessions(projectKey));
      if (cancelled() || !valid(token)) return;
      sessionPathsRef.current = new Set(ss.map(s => s.path));
      setSessions(prev => sameSessions(prev, ss) ? prev : ss);
      return ss;
    } catch (e) {
      if (!silent && !cancelled() && valid(token)) setError(String(e));
    } finally {
      if (!silent && !cancelled() && valid(token)) setLoadingSessions(false);
    }
  }, [valid]);

  const readDetail = useCallback(async (token: RequestToken, silent = false, cancelled: () => boolean = () => false) => {
    const q = sequenceRef.current;
    if (cancelled() || !token.path || !valid(token)) return;
    try {
      const d = await q.request(token, `detail:${token.path}`, () => api.sessionDetail(token.path!));
      if (cancelled() || !valid(token)) return;
      setDetail(prev => {
        // Keep Thread's memo identity on unchanged polls, but retain metadata updates.
        if (prev?.path === d.path && prev.size === d.size && prev.updatedAt === d.updatedAt &&
            prev.entries.length === d.entries.length && prev.active.length === d.active.length &&
            prev.entries[prev.entries.length - 1]?.id === d.entries[d.entries.length - 1]?.id && prev.entries[prev.entries.length - 1]?.ts === d.entries[d.entries.length - 1]?.ts) return prev;
        return d;
      });
      return d;
    } catch (e) {
      if (!silent && !cancelled() && valid(token)) setError(String(e));
    }
  }, [valid]);

  const loadDetail = useCallback(async (s: SessionMeta) => {
    const q = sequenceRef.current;
    if (!q.ready || !aliveRef.current) return;
    const changed = q.path !== s.path;
    q.selectDetail(s.path);
    activePathRef.current = s.path;
    if (changed) {
      detailRef.current = null;
      setDetail(null);
      clearLive();
      setRunning(!!s.running);
    }
    return readDetail(q.capture("detail"));
  }, [readDetail, clearLive]);

  const readRemoteStatus = useCallback(async (token: RequestToken, host: string, cancelled: () => boolean = () => false) => {
    if (cancelled() || !valid(token, true)) return;
    try {
      const status = await sequenceRef.current.request(token, `status:${host}`, () => api.remoteSyncStatus(host));
      if (cancelled() || !valid(token, true)) return;
      setRemoteStatus(status);
      return status;
    } catch {
      // An unavailable status endpoint must not block source selection.
    }
  }, [valid]);

  const runRemoteRefresh = useCallback(async (token: RequestToken) => {
    const host = token.source;
    const projectToken = sequenceRef.current.capture("project");
    const detailToken = sequenceRef.current.capture("detail");
    if (!host || !valid(token)) return;
    setRefreshingRemote(true);
    try {
      const sync = sequenceRef.current.request(token, `refresh:${host}`, () => api.refreshRemote(host));
      // Status remains visible alongside the cached view; no loading overlay.
      void readRemoteStatus(token, host);
      await sync;
      if (!valid(token)) return;
      await readRemoteStatus(token, host);
      if (!valid(token)) return;
      await refreshProjects();
      if (!valid(token)) return;
      if (valid(projectToken) && projectToken.project) await refreshSessions(projectToken.project, true);
      if (!valid(token)) return;
      if (valid(detailToken)) await readDetail(detailToken, true);
      if (!valid(token)) return;
    } catch (e) {
      if (!valid(token)) return;
      setRemoteStatus(prev => ({ host, phase: "error", lastSuccessAt: prev?.lastSuccessAt ?? null,
        usableCache: prev?.usableCache ?? true, error: String(e) }));
    } finally {
      if (valid(token)) setRefreshingRemote(false);
    }
  }, [valid, readRemoteStatus, refreshProjects, refreshSessions, readDetail]);

  const resetSourceView = useCallback(() => {
    paintCancelRef.current?.();
    paintCancelRef.current = null;
    projectRef.current = activePathRef.current = null;
    detailRef.current = null;
    setProjects([]);
    setSelectedProject(null);
    sessionPathsRef.current.clear();
    setSessions([]);
    setLoadingSessions(false);
    setDetail(null);
    setShowConfig(false);
    setError(null);
    setToasts([]);
    setFinishedAt({});
    setRefreshingRemote(false);
    prevRunningRef.current = null;
    autoOpened.current = false;
    clearLive();
  }, [clearLive]);

  const onSwitchSource = useCallback(async (host: string | null) => {
    const q = sequenceRef.current;
    const token = q.beginSource(host);
    sourceRef.current = host;
    resetSourceView();
    setRemoteStatus(null);
    setSourcePending(true);
    setSourceNotice({ host });
    try {
      // Cheap local status distinguishes usable cache from blocking initial sync.
      const before = host ? await readRemoteStatus(token, host) : undefined;
      if (!valid(token, true)) return;
      const selected = await q.commitSource(token, () => api.setRemoteHost(host));
      if (!selected || !valid(token)) return;
      setRemoteHost(host);
      setSourcePending(false);
      setSourceNotice(null);
      await refreshProjects();
      if (!valid(token)) return;
      const key = projectRef.current;
      if (key) await refreshSessions(key);
      if (!valid(token)) return;
      if (host) {
        void readRemoteStatus(token, host);
        if (before?.usableCache) {
          paintCancelRef.current = afterViewPaint(() => {
            paintCancelRef.current = null;
            if (valid(token)) void runRemoteRefresh(token);
          });
        }
      }
    } catch (e) {
      if (!valid(token, true)) return;
      // Failed initial sync does not select the new host. Restore the backend's
      // last committed source, including an earlier superseded successful commit.
      let fallback = q.committedSource;
      try { fallback = await api.getRemoteHost(); } catch { /* retain last known source */ }
      if (!valid(token, true)) return;
      const restored = q.beginSource(fallback);
      q.adoptSource(restored, fallback);
      sourceRef.current = fallback;
      setRemoteHost(fallback);
      setSourcePending(false);
      setRemoteStatus(null);
      setSourceNotice({ host, error: String(e) });
      await refreshProjects();
      if (!valid(restored)) return;
    }
  }, [resetSourceView, valid, readRemoteStatus, refreshProjects, refreshSessions, runRemoteRefresh]);

  useEffect(() => {
    aliveRef.current = true;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const q = sequenceRef.current;
    const bootstrap = q.capture();
    void (async () => {
      try {
        const host = await api.getRemoteHost();
        if (disposed || !valid(bootstrap, true)) return;
        q.adoptSource(bootstrap, host);
        sourceRef.current = host;
        setRemoteHost(host);
        await refreshProjects();
        if (disposed || !valid(bootstrap)) return;
        if (host) void readRemoteStatus(q.capture(), host);
      } catch (e) {
        if (!disposed && valid(bootstrap, true)) {
          q.adoptSource(bootstrap, null);
          setError(String(e));
          void refreshProjects();
        }
      }
    })();
    getCurrentWindow().onFocusChanged(({ payload }) => {
      if (payload && !disposed) void refreshProjects();
    }).then(fn => { if (disposed) fn(); else unlisten = fn; }).catch(() => {});
    void (async () => {
      try {
        const p = await api.piBinPath();
        if (disposed) return;
        try {
          const v = await api.piVersion();
          if (!disposed) setPiInfo(`${p} (v${v})`);
        } catch { if (!disposed) setPiInfo(p ?? "pi not found"); }
      } catch { if (!disposed) setPiInfo("pi not found"); }
    })();
    api.listRemoteHosts().then(hs => { if (!disposed) setRemoteHosts(hs); }).catch(() => {});
    return () => {
      disposed = true;
      aliveRef.current = false;
      unlisten?.();
      paintCancelRef.current?.();
      if (liveTimerRef.current) window.clearTimeout(liveTimerRef.current);
    };
  }, [valid, refreshProjects, readRemoteStatus]);

  useEffect(() => {
    if (selectedProject) void refreshSessions(selectedProject);
  }, [selectedProject, remoteHost, refreshSessions]);

  // Poll pending initial-sync hosts too. Cancellation is checked after every await.
  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    const pollStatus = async () => {
      const token = sequenceRef.current.capture();
      if (token.source && !document.hidden) {
        await readRemoteStatus(token, token.source, () => cancelled);
        if (cancelled || !valid(token, true)) return;
      }
      if (!cancelled) timer = window.setTimeout(pollStatus, 1000);
    };
    timer = window.setTimeout(pollStatus, 1000);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [remoteHost, sourcePending, readRemoteStatus, valid]);

  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    const poll = async () => {
      const q = sequenceRef.current;
      const sourceToken = q.capture();
      const projectToken = q.capture("project");
      const current = () => !cancelled && valid(sourceToken) && valid(projectToken);
      if (!document.hidden && current()) {
        try {
          await refreshProjects(() => cancelled);
          if (!current()) return;
          let ss: SessionMeta[] | undefined;
          if (projectToken.project) {
            ss = await refreshSessions(projectToken.project, true, () => !current());
            if (!current()) return;
          }
          const d = detailRef.current;
          const row = ss?.find(s => s.path === d?.path);
          if (d && row?.running && (row.size !== d.size || row.updatedAt !== d.updatedAt)) {
            const detailToken = q.capture("detail");
            await readDetail(detailToken, true, () => !current());
            if (!current() || !valid(detailToken)) return;
          }
          const runningNow = await q.request(sourceToken, "running", api.listRunning);
          if (!current()) return;
          const paths = new Set(runningNow.map(r => r.path));
          const prev = prevRunningRef.current;
          if (prev) {
            for (const [path, info] of prev) {
              if (!paths.has(path)) {
                let status = "finished";
                try { status = await q.request(sourceToken, `session-status:${path}`, () => api.sessionStatus(path)); }
                catch { /* preserve finished fallback */ }
                if (!current()) return;
                if (status === "finished") setFinishedAt(m => ({ ...m, [path]: Date.now() }));
                const id = ++toastIdRef.current;
                setToasts(ts => [...ts, { id, path, ...info, sourceToken,
                  paused: status === "sleeping", interrupted: status === "interrupted" }].slice(-8));
              }
            }
          }
          if (!current()) return;
          prevRunningRef.current = new Map(runningNow.map(r => [r.path,
            { title: r.title, isSubagent: r.isSubagent, projectKey: r.projectKey }]));
        } catch { /* ignore transient poll errors */ }
      }
      if (!cancelled) timer = window.setTimeout(poll, 10000);
    };
    timer = window.setTimeout(poll, 10000);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [selectedProject, remoteHost, sourcePending, detail?.path, valid, refreshProjects, refreshSessions, readDetail]);

  useEffect(() => {
    if (!autoOpened.current && sessions.length && !detail && sequenceRef.current.ready) {
      autoOpened.current = true;
      void loadDetail(sessions[0]);
    }
  }, [sessions, detail, loadDetail]);

  const selectProject = useCallback((p: Project) => {
    if (!sequenceRef.current.ready) return;
    changeProject(p.key);
    setShowConfig(false);
  }, [changeProject]);
  const selectSession = useCallback((s: SessionMeta) => {
    setShowConfig(false);
    setFinishedAt(m => { const n = { ...m }; delete n[s.path]; return n; });
    void loadDetail(s);
  }, [loadDetail]);
  const openToastSession = (t: Toast) => {
    if (!valid(t.sourceToken)) return;
    setToasts(ts => ts.filter(x => x.id !== t.id));
    changeProject(t.projectKey || projectRef.current);
    selectSession({ path: t.path } as SessionMeta);
  };
  const openConfig = useCallback(() => setShowConfig(v => !v), []);

  const flushLive = useCallback((token: RequestToken) => {
    liveTimerRef.current = 0;
    if (!valid(token)) return;
    const batch = pendingEventsRef.current.splice(0);
    if (batch.length) setLiveBlocks(prev => appendLiveEvents(prev, batch));
  }, [valid]);

  const onTurnDone = async (token: RequestToken) => {
    if (!valid(token)) return;
    clearLive();
    await readDetail(token);
    if (!valid(token)) return;
    await refreshProjects();
    if (!valid(token)) return;
    if (token.project) await refreshSessions(token.project, true);
    if (!valid(token)) return;
  };

  const send = async (msg: string): Promise<boolean> => {
    const token = sequenceRef.current.capture("detail");
    if (!detail || running || !token.path || !valid(token)) return false;
    clearLive();
    const channel = new Channel<PiEvent>();
    channel.onmessage = ev => {
      if (!valid(token)) return;
      if (ev.type === "diagnostic") { setError(`Pi: ${String(ev.line ?? "").slice(-2000)}`); return; }
      pendingEventsRef.current.push(ev);
      if (ev.type === "process_exit") void onTurnDone(token);
      else if (!liveTimerRef.current) liveTimerRef.current = window.setTimeout(() => flushLive(token), 80);
    };
    setRunning(true);
    try {
      await api.sendMessage(token.path, msg, channel);
      if (!valid(token)) return false;
      return true;
    } catch (e) {
      if (valid(token)) { setRunning(false); setError(String(e)); }
      return false;
    }
  };

  const abort = async () => {
    const token = sequenceRef.current.capture("detail");
    if (!token.path || !valid(token)) return;
    try {
      await api.abortMessage(token.path);
      if (!valid(token)) return;
      clearLive();
      await readDetail(token);
      if (!valid(token)) return;
    } catch (e) { if (valid(token)) setError(String(e)); }
  };

  // Action responses are scoped too: a slow A action cannot clear B's detail,
  // show A's error, refresh B, or launch subsequent IPC against B's backend.
  const performAction = useCallback(async (path: string, action: (path: string) => Promise<unknown>, remove = false) => {
    const token = sequenceRef.current.capture("project");
    const detailToken = sequenceRef.current.capture("detail");
    if (!valid(token) || !sessionPathsRef.current.has(path)) return;
    try {
      await action(path);
      if (!valid(token)) return;
      if (remove && valid(detailToken) && activePathRef.current === path) {
        sequenceRef.current.selectDetail(null);
        activePathRef.current = null;
        detailRef.current = null;
        setDetail(null);
        clearLive();
      }
      await refreshProjects();
      if (!valid(token)) return;
      if (token.project) await refreshSessions(token.project, true);
      if (!valid(token)) return;
    } catch (e) { if (valid(token)) setError(String(e)); }
  }, [valid, clearLive, refreshProjects, refreshSessions]);
  const onOpenTerminal = useCallback((path: string) => {
    const token = sequenceRef.current.capture("project");
    if (valid(token) && sessionPathsRef.current.has(path)) api.openInTerminal(path).catch(e => { if (valid(token)) setError(String(e)); });
  }, [valid]);
  const onDetachFromRmux = useCallback((path: string) => performAction(path, api.detachFromRmux), [performAction]);
  const onKillRmuxSession = useCallback((path: string) => performAction(path, api.killRmuxSession), [performAction]);
  const onDeleteSession = useCallback((path: string) => performAction(path, api.deleteSession, true), [performAction]);

  const onTransferToRemote = useCallback(async (path: string) => {
    const q = sequenceRef.current;
    const token = q.capture("project");
    if (!valid(token) || !sessionPathsRef.current.has(path)) return;
    try {
      const hosts = remoteHosts.length ? remoteHosts : await api.listRemoteHosts();
      if (!valid(token)) return;
      if (!hosts.length) { setError("No remote hosts configured (~/.pi-session-viewer.json)"); return; }
      const host = hosts.length === 1 ? hosts[0] : prompt(`Transfer to which host?\n${hosts.join("\n")}`, hosts[0]);
      if (!host || !valid(token)) return;
      const d = await q.request(token, `transfer-detail:${path}`, () => api.sessionDetail(path));
      if (!valid(token)) return;
      const base = (d.cwd || "").split("/").filter(Boolean).pop() || "project";
      const remoteCwd = prompt(`Remote working directory on ${host}:`, `~/Project/${base}`);
      if (!remoteCwd || !valid(token)) return;
      const msg = prompt("First message to send after transfer (empty = just resume interactively):", "") ?? "";
      if (!valid(token)) return;
      const sess = await api.transferSessionToRemote(path, host, remoteCwd, msg);
      if (!valid(token)) return;
      alert(`Transferred to ${host}.\nrmux session: ${sess}\nAttach locally: ssh -t ${host} 'rmux attach -t ${sess}'`);
    } catch (e) { if (valid(token)) setError(String(e)); }
  }, [remoteHosts, valid]);

  const onRefresh = useCallback(() => {
    const token = sequenceRef.current.capture();
    if (!valid(token)) return;
    if (sourceRef.current) void runRemoteRefresh(token);
    else {
      void refreshProjects();
      if (projectRef.current) void refreshSessions(projectRef.current, true);
      void readDetail(sequenceRef.current.capture("detail"));
    }
  }, [valid, runRemoteRefresh, refreshProjects, refreshSessions, readDetail]);

  const sessionTitle = (path: string) => {
    const s = sessions.find(x => x.path === path);
    const title = s?.name || s?.firstMessage || path.split("/").pop() || "(empty)";
    return title.length > 40 ? title.slice(0, 40) + "…" : title;
  };
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
        syncing={false}
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
        {(remoteHost || sourceNotice) && (
          <div className="source-status" role="status" style={{ padding: "8px 16px", borderBottom: "1px solid var(--border)", fontSize: 12 }}>
            <strong>Source: {sourceNotice ? sourceNotice.host ?? "Local" : remoteHost ?? "Local"}</strong>
            {sourceNotice?.error ? (
              <span> · Initial selection/sync failed: {sourceNotice.error}. Not selected; browsing {remoteHost ?? "Local"}.
                <button onClick={() => void onSwitchSource(sourceNotice.host)}>Retry</button>
              </span>
            ) : sourcePending ? (
              <span> · {sourceNotice?.host === null ? "Switching to Local…" : remoteStatus?.usableCache ? "Opening cached source…" : "Initial sync — source not ready (no usable cache yet)"}
                {remoteStatus && ` · ${remoteStatus.phase}`}
                {remoteStatus?.error && ` · ${remoteStatus.error}`}
                <button type="button" onClick={() => void onSwitchSource(null)}>Use Local</button>
              </span>
            ) : (
              <span> · {remoteStatus?.usableCache ? "Cached view" : "Cache availability unknown"}
                {remoteStatus?.lastSuccessAt != null
                  ? ` · Last full sync: ${new Date(remoteStatus.lastSuccessAt * 1000).toLocaleString()} · ${Math.max(0, Math.floor(Date.now() / 1000 - remoteStatus.lastSuccessAt))}s old`
                  : " · No successful full-sync timestamp"}
                {` · ${remoteStatus?.phase ?? "status unavailable"}${refreshingRemote ? " · refreshing in background" : ""}`}
                {remoteStatus?.error && ` · ${remoteStatus.error}`}
                <span> · Cache/snapshot state, not live telemetry</span>
              </span>
            )}
          </div>
        )}
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
                  const token = sequenceRef.current.capture("detail");
                  const accepted = await send(msg);
                  if (accepted && valid(token)) setDrafts(d => ({ ...d, [detail.path]: "" }));
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
