import { invoke } from "@tauri-apps/api/core";
import type { Channel } from "@tauri-apps/api/core";
import type { PiEvent, Project, SessionDetail, SessionMeta } from "./types";

export const api = {
  listProjects: () => invoke<Project[]>("list_projects"),
  listSessions: (projectKey: string) => invoke<SessionMeta[]>("list_sessions", { projectKey }),
  sessionDetail: (path: string) => invoke<SessionDetail>("session_detail", { path }),
  piBinPath: () => invoke<string | null>("pi_bin_path"),
  piSessionsDir: () => invoke<string>("pi_sessions_dir"),
  piVersion: () => invoke<string>("pi_version"),
  fileExists: (path: string) => invoke<boolean>("file_exists", { path }),
  sendMessage: (sessionPath: string, message: string, channel: Channel<PiEvent>) =>
    invoke<void>("send_message", { sessionPath, message, onEvent: channel }),
  abortMessage: (sessionPath: string) => invoke<void>("abort_message", { sessionPath }),
};
