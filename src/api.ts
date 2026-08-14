import { invoke } from "@tauri-apps/api/core";
import type { Channel } from "@tauri-apps/api/core";
import type { PiEvent, Project, RunningSession, SessionDetail, SessionMeta, ConfigView } from "./types";

export const api = {
  listProjects: () => invoke<Project[]>("list_projects"),
  listSessions: (projectKey: string) => invoke<SessionMeta[]>("list_sessions", { projectKey }),
  sessionDetail: (path: string) => invoke<SessionDetail>("session_detail", { path }),
  piBinPath: () => invoke<string | null>("pi_bin_path"),
  piVersion: () => invoke<string>("pi_version"),
  exportSession: (path: string) => invoke<string>("export_session_html", { sessionPath: path }),
  deleteSession: (path: string) => invoke<void>("delete_session", { path }),
  openInTerminal: (path: string) => invoke<string>("open_in_terminal", { sessionPath: path }),
  attachSession: (path: string) => invoke<string>("attach_session", { sessionPath: path }),
  detachFromRmux: (path: string) => invoke<void>("detach_from_rmux", { sessionPath: path }),
  killRmuxSession: (path: string) => invoke<void>("kill_rmux_session", { sessionPath: path }),
  sessionStatus: (path: string) => invoke<string>("session_status", { path }),
  listRunning: () => invoke<RunningSession[]>("list_running"),
  listConfig: () => invoke<ConfigView>("list_config"),
  sendMessage: (sessionPath: string, message: string, channel: Channel<PiEvent>) =>
    invoke<void>("send_message", { sessionPath, message, onEvent: channel }),
  abortMessage: (sessionPath: string) => invoke<void>("abort_message", { sessionPath }),
};
