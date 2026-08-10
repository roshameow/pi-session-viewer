// Shared types mirroring the Rust backend (serde camelCase).

export interface Project {
  key: string;
  cwd: string;
  sessionCount: number;
  subagentCount: number;
  updatedAt: number;
  runningCount: number;
}

export interface SessionMeta {
  path: string;
  id: string;
  cwd: string;
  name: string | null;
  firstMessage: string | null;
  lastMessage: string | null;
  createdIso: string;
  createdAt: number;
  updatedAt: number;
  model: string | null;
  isSubagent: boolean;
  taskId: string | null;
  parentSessionId: string | null;
  parentSessionPath: string | null;
  messageCount: number;
  running: boolean;
  size: number;
}

export type ContentBlock =
  | { kind: "text"; text: string }
  | { kind: "thinking"; thinking: string }
  | { kind: "toolCall"; id: string; name: string; arguments: string }
  | { kind: "bash"; command: string; output: string; exitCode: number | null; truncated: boolean }
  | { kind: "image"; mimeType: string };

export interface Entry {
  kind: string;
  id: string;
  parentId: string | null;
  ts: string | null;
  role: string | null;
  content: ContentBlock[];
  model: string | null;
  toolName: string | null;
  toolCallId: string | null;
  isError: boolean | null;
  summary: string | null;
  name: string | null;
  label: string | null;
}

export interface Stats {
  tokenCount: number;
  messageCount: number;
  model: string | null;
  costTotal: number;
}

export interface SessionDetail {
  id: string;
  cwd: string;
  createdIso: string;
  path: string;
  stats: Stats;
  entries: Entry[];
  active: number[];
}

export interface RunningSession {
  path: string;
  title: string;
  isSubagent: boolean;
  taskId: string | null;
  projectKey: string;
}

export interface McpServer {
  name: string;
  command: string;
  args: string[];
  env: string[];
  enabled: boolean | null;
}

export interface AgentInfo {
  name: string;
  description: string;
  tools: string | null;
  file: string;
}

export interface SkillInfo {
  name: string;
  description: string;
  source: string;
}

export interface ConfigView {
  mcp: McpServer[];
  agents: AgentInfo[];
  skills: SkillInfo[];
}

// --- pi `--mode json` wire events (subset we render) ---

export interface PiEvent {
  type: string;
  [key: string]: unknown;
}
