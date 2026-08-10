import React, { useCallback, useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";
import type { PiEvent, Project, SessionDetail, SessionMeta } from "./types";
import { Sidebar } from "./components/Sidebar";
import { Thread, buildLiveBlocks, type LiveBlock } from "./components/Thread";
import { Composer } from "./components/Composer";

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

  const channelRef = useRef<Channel<PiEvent> | null>(null);
  const activePathRef = useRef<string | null>(null);

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
      .then((p) => api.piVersion().then((v) => setPiInfo(`${p} (v${v})`)).catch(() => setPiInfo(p ?? "pi 未找到")))
      .catch(() => setPiInfo("pi 未找到"));
  }, [refreshProjects]);

  useEffect(() => {
    if (selectedProject) refreshSessions(selectedProject);
  }, [selectedProject, refreshSessions]);

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
            <Composer running={running} onSend={send} onAbort={abort} />
          </>
        ) : (
          <div className="empty-main">
            <h2>Pi Desktop</h2>
            <p>选择一个会话查看 / 继续对话</p>
            {piInfo && <p className="pi-info">{piInfo}</p>}
          </div>
        )}
      </div>
    </div>
  );
}
