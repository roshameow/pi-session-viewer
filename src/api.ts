import { invoke } from "@tauri-apps/api/core";
import type { Channel } from "@tauri-apps/api/core";
import type { PiEvent, Project, RunningSession, SessionDetail, SessionMeta, ConfigView, RemoteSyncStatus, DetailPageRequest, EntryBodyChunk, AuthorizedDirectory, AuthorizedDirectoryPage, ImportedDataFile } from "./types";

export const api = {
  listAuthorizedDirectories: () => invoke<AuthorizedDirectory[]>("list_authorized_directories"),
  chooseAuthorizedDirectory: () => invoke<AuthorizedDirectory | null>("choose_authorized_directory"),
  removeAuthorizedDirectory: (id: string) => invoke<void>("remove_authorized_directory", { id }),
  authorizedDirectoryEntries: (id: string, relativePath: string, offset = 0) =>
    invoke<AuthorizedDirectoryPage>("authorized_directory_entries", { id, relativePath, offset }),
  importAuthorizedFile: (id: string, relativePath: string) =>
    invoke<ImportedDataFile>("import_authorized_file", { id, relativePath }),
  listImportedDataFiles: () => invoke<ImportedDataFile[]>("list_imported_data_files"),
  removeImportedDataFile: (id: string) => invoke<void>("remove_imported_data_file", { id }),
  listProjects: () => invoke<Project[]>("list_projects"),
  listSessions: (projectKey: string) => invoke<SessionMeta[]>("list_sessions", { projectKey }),
  sessionDetailPage: (path: string, request: DetailPageRequest = {}) => invoke<SessionDetail>("session_detail_page", { path, request }),
  sessionEntryBody: (path: string, generation: string, entryId: string, offset = 0, recordOffset?: number) =>
    invoke<EntryBodyChunk>("session_entry_body", { path, generation, entryId, recordOffset, offset, maxBytes: 65536 }),
  sessionDetail: (path: string) => invoke<SessionDetail>("session_detail", { path }),
  piBinPath: () => invoke<string | null>("pi_bin_path"),
  piVersion: () => invoke<string>("pi_version"),
  exportSession: (path: string) => invoke<string>("export_session_html", { sessionPath: path }),
  deleteSession: (path: string) => invoke<void>("delete_session", { path }),
  openLocalFile: (href: string, sessionPath: string) => invoke<string>("open_local_file", { href, sessionPath }),
  openInTerminal: (path: string) => invoke<string>("open_in_terminal", { sessionPath: path }),
  attachSession: (path: string) => invoke<string>("attach_session", { sessionPath: path }),
  detachFromRmux: (path: string) => invoke<void>("detach_from_rmux", { sessionPath: path }),
  killRmuxSession: (path: string) => invoke<void>("kill_rmux_session", { sessionPath: path }),
  sessionStatus: (path: string) => invoke<string>("session_status", { path }),
  listRunning: () => invoke<RunningSession[]>("list_running"),
  listConfig: () => invoke<ConfigView>("list_config"),
  setRemoteHost: (host: string | null) => invoke<void>("set_remote_host", { host }),
  transferSessionToRemote: (sessionPath: string, host: string, remoteCwd: string, prompt: string) =>
    invoke<string>("transfer_session_to_remote", { sessionPath, host, remoteCwd, prompt }),
  listRemoteHosts: () => invoke<string[]>("list_remote_hosts"),
  getRemoteHost: () => invoke<string | null>("get_remote_host"),
  refreshRemote: (host?: string | null) => invoke<void>("refresh_remote", { host: host ?? null }),
  remoteSyncStatus: (host: string) => invoke<RemoteSyncStatus>("remote_sync_status", { host }),
  sendMessage: (sessionPath: string, message: string, channel: Channel<PiEvent>) =>
    invoke<void>("send_message", { sessionPath, message, onEvent: channel }),
  abortMessage: (sessionPath: string) => invoke<void>("abort_message", { sessionPath }),
};
