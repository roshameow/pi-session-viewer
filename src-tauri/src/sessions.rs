//! Session discovery + JSONL parsing for pi coding agent sessions.
//!
//! Session files live in `~/.pi/agent/sessions/--<cwd-encoded>--/<ts>_<uuid>.jsonl`
//! (format documented in pi's docs/session-format.md). Subagent mirror files from
//! the pi-subagent-durable extension live in the same dir, named
//! `<ts>_subagent-task-<taskId>.jsonl`. Their header `id` IS the parent session's
//! uuid, which gives us exact parent->subagent linkage for free.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Global running-session registry (written by agent.rs, read by sessions.rs)
// ---------------------------------------------------------------------------

static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub fn running_set() -> &'static Mutex<HashSet<String>> {
    RUNNING.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn mark_running(path: &str) {
    if let Ok(mut s) = running_set().lock() {
        s.insert(path.to_string());
    }
}

pub fn unmark_running(path: &str) {
    if let Ok(mut s) = running_set().lock() {
        s.remove(path);
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn pi_agent_dir() -> PathBuf {
    if let Ok(d) = std::env::var("PI_CODING_AGENT_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".pi").join("agent")
}

pub fn sessions_dir() -> PathBuf {
    if let Ok(d) = std::env::var("PI_CODING_AGENT_SESSION_DIR") {
        return PathBuf::from(d);
    }
    pi_agent_dir().join("sessions")
}

pub fn resolve_pi_bin() -> Option<String> {
    if let Ok(b) = std::env::var("PI_CODING_AGENT_BIN") {
        if !b.is_empty() {
            return Some(b);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    for cand in [
        "/opt/homebrew/bin/pi",
        "/usr/local/bin/pi",
        &format!("{home}/.local/bin/pi"),
        "/usr/bin/pi",
    ] {
        if Path::new(cand).is_file() {
            return Some(cand.to_string());
        }
    }
    // last resort: PATH lookup
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = Path::new(dir).join("pi");
            if p.is_file() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Data model (serialized to the frontend)
// ---------------------------------------------------------------------------

const RUNNING_FRESH_SECS: u64 = 60;

/// A main pi session is running if it was written recently (pi appends to the
/// session JSONL while a turn executes in the terminal).
fn session_file_running(path: &Path) -> bool {
    let Ok(md) = fs::metadata(path) else { return false };
    let Ok(mt) = md.modified() else { return false };
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    let Ok(mtime) = mt.duration_since(std::time::UNIX_EPOCH) else { return false };
    now.saturating_sub(mtime).as_secs() < RUNNING_FRESH_SECS
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub key: String,          // encoded dir name
    pub cwd: String,          // real cwd from a session header (best effort)
    pub session_count: usize,
    pub subagent_count: usize,
    pub updated_at: i64,      // latest file mtime (secs)
    pub running_count: usize, // running main sessions + running subagents
    pub rmux_count: usize,    // alive sessions in rmux windows (attached or detached)
    pub term_count: usize,    // main sessions actively running in a terminal window
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub path: String,
    pub id: String,
    pub cwd: String,
    pub name: Option<String>,
    pub first_message: Option<String>,
    pub last_message: Option<String>,
    pub created_iso: String,
    pub created_at: i64,      // epoch secs from header timestamp
    pub updated_at: i64,      // file mtime (secs)
    pub model: Option<String>,
    pub is_subagent: bool,
    pub task_id: Option<String>,
    pub parent_session_id: Option<String>, // set when is_subagent (== header id)
    pub parent_session_path: Option<String>, // linked parent session file (text/timing match)
    pub message_count: usize,
    pub running: bool,
    pub sleeping: bool,   // process alive, waiting on a bash sleep
    pub interrupted: bool, // process dead + no terminal event, resumable
    pub in_rmux: bool,
    pub rmux_target: Option<String>, // e.g. "pi-Users-...:s<id8>"
    pub rmux_attached: bool, // a terminal client is attached to the rmux session
    pub rmux_dead: bool,     // rmux window kept by remain-on-exit, pane process exited
    pub term_alive: bool,    // an alive pi process runs this session in a terminal window
    pub size: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub token_count: u64,
    pub message_count: usize,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub thinking_level: Option<String>,
    pub context_tokens: Option<u64>,
    pub context_limit: Option<u64>,
    pub cost_total: f64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ContentBlock {
    Text { text: String },
    Thinking { thinking: String },
    ToolCall { id: String, name: String, arguments: String },
    Bash { command: String, output: String, exit_code: Option<i64>, truncated: bool },
    Image { mime_type: String },
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub kind: String,           // message | model_change | thinking_level_change | compaction | branch_summary | custom_message | label | session_info
    pub id: String,
    pub parent_id: Option<String>,
    pub ts: Option<String>,
    pub role: Option<String>,   // user | assistant | toolResult | bashExecution | ...
    pub content: Vec<ContentBlock>,
    pub model: Option<String>,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub is_error: Option<bool>,
    pub summary: Option<String>,
    pub name: Option<String>,
    pub label: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub id: String,
    pub cwd: String,
    pub created_iso: String,
    pub path: String,
    pub task_id: Option<String>,
    pub stats: Stats,
    pub entries: Vec<Entry>,
    pub active: Vec<usize>, // indices of the active branch, root -> leaf
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

pub fn decode_dir_name(dir: &str) -> String {
    // "--Users-<user>--" -> "/Users/<user>" (best effort; header cwd is authoritative)
    let inner = dir.trim_start_matches('-').trim_end_matches('-');
    if inner.is_empty() {
        return String::new();
    }
    format!("/{}", inner.replace('-', "/"))
}

/// "/Users/a/b" -> "--Users-a-b--" (mirror of decode; pi's session dir naming).
pub fn encode_dir_name(cwd: &str) -> String {
    format!("--{}--", cwd.trim_start_matches('/').replace('/', "-"))
}

/// Parse `YYYY-MM-DDTHH:MM:SS(.mmm)Z` -> epoch secs (std only).
fn parse_iso_ts(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    if s.is_empty() {
        return None;
    }
    // numeric epoch fallback
    if let Ok(n) = s.parse::<i64>() {
        if n > 10_000_000_000 {
            return Some(n / 1000);
        }
        return Some(n);
    }
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }
    let date: Vec<&str> = parts[0].split('-').collect();
    if date.len() != 3 {
        return None;
    }
    let year: i64 = date[0].parse().ok()?;
    let month: i64 = date[1].parse().ok()?;
    let day: i64 = date[2].parse().ok()?;
    let time: Vec<&str> = parts[1].split(':').collect();
    if time.len() < 2 {
        return None;
    }
    let hour: i64 = time[0].parse().ok()?;
    let min: i64 = time[1].parse().ok()?;
    let sec: f64 = time[2].parse().ok()?;
    // days from civil (Howard Hinnant's algorithm)
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hour * 3600 + min * 60 + sec as i64)
}

fn task_id_from_filename(name: &str) -> Option<String> {
    // 2026-08-04T03-37-30-125Z_subagent-task-msmmchn4-kml1.jsonl
    let base = name.strip_suffix(".jsonl").unwrap_or(name);
    base.split("subagent-task-").nth(1).map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

pub fn list_projects() -> Vec<Project> {
    let root = sessions_dir();
    let mut out = Vec::new();
    let (_, task_by_uuid, _) = subagent_index();
    let alive = alive_task_ids();
    let rmux_map = rmux_runtime_map();
    // alive_terminal_pis() spawns ps+lsof; compute once, not per project
    let term_alive = alive_terminal_pis();
    if let Ok(rd) = fs::read_dir(&root) {
        for e in rd.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("--") {
                continue;
            }
            let mut count = 0usize;
            let mut sub_count = 0usize;
            let mut updated = 0i64;
            let mut running_count = 0usize;
            let mut rmux_count = 0usize;
            let mut seen_uuids: HashSet<String> = HashSet::new();
            let mut best_cwd: Option<(i64, String)> = None;
            let cwd = decode_dir_name(&name);
            if let Ok(fd) = fs::read_dir(&path) {
                for f in fd.flatten() {
                    if !f.path().is_file() {
                        continue;
                    }
                    let fname = f.file_name().to_string_lossy().to_string();
                    if !fname.ends_with(".jsonl") {
                        continue;
                    }
                    count += 1;
                    if fname.contains("subagent-task-") {
                        sub_count += 1;
                    }
                    // running detection (dedupe mirror/real by uuid)
                    let header = first_line(&f.path()).and_then(|l| {
                        serde_json::from_str::<Value>(&l).ok()
                    });
                    if let Some(h) = &header {
                        if let Some(u) = h.get("id").and_then(|x| x.as_str()) {
                            if seen_uuids.insert(u.to_string()) {
                                if let Some(tid) = task_by_uuid.get(u) {
                                    // subagent: running tasks live in pi-agents (rmux)
                                    let is_running = alive.contains(tid);
                                    if is_running {
                                        running_count += 1;
                                        rmux_count += 1;
                                    }
                                } else {
                                    // main session
                                    if session_file_running(&f.path()) {
                                        running_count += 1;
                                    }
                                    let spath = f.path().to_string_lossy().into_owned();
                                    if let Some(rt) = rmux_map.get(&spath) {
                                        if !rt.dead {
                                            rmux_count += 1;
                                        }
                                    }
                                }
                            }
                        }
                        // best-effort real cwd from newest session header
                        if let Some(c) = h.get("cwd").and_then(|x| x.as_str()) {
                            let mt = f
                                .metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs() as i64)
                                .unwrap_or(0);
                            if best_cwd.as_ref().map(|(bm, _)| mt > *bm).unwrap_or(true) {
                                best_cwd = Some((mt, c.to_string()));
                            }
                        }
                    }
                    if let Ok(md) = f.metadata() {
                        if let Ok(mt) = md.modified() {
                            if let Ok(secs) = mt.duration_since(std::time::UNIX_EPOCH) {
                                let s = secs.as_secs() as i64;
                                if s > updated {
                                    updated = s;
                                }
                            }
                        }
                    }
                }
            }
            // term sessions = alive terminal pi processes running in this project
            let term_count = term_alive
                .iter()
                .filter(|(_, c)| *c == cwd)
                .count();

            if count == 0 {
                continue;
            }
            let cwd = best_cwd.map(|(_, c)| c).unwrap_or(cwd);
            out.push(Project {
                key: name,
                cwd,
                session_count: count,
                subagent_count: sub_count,
                updated_at: updated,
                running_count,
                rmux_count,
                term_count,
            });
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
    out
}

fn first_line(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = fs::File::open(p).ok()?;
    let mut buf = vec![0u8; 16 * 1024];
    let n = f.read(&mut buf).ok()?;
    let s = String::from_utf8_lossy(&buf[..n]);
    s.lines().next().map(|l| l.to_string())
}

/// Look up a model's context window from `~/.pi/agent/models.json`
/// (provider catalog). Result is cached per process.
fn model_context_window(model_id: &str) -> Option<u64> {
    static CACHE: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if let Some(v) = cache.get(model_id) {
        return Some(*v);
    }
    let found = (|| {
        let Ok(data) = fs::read_to_string(pi_agent_dir().join("models.json")) else {
            return None;
        };
        let Ok(root) = serde_json::from_str::<Value>(&data) else {
            return None;
        };
        fn find(v: &Value, id: &str) -> Option<u64> {
            match v {
                Value::Object(map) => {
                    if map.get("id").and_then(|x| x.as_str()) == Some(id) {
                        if let Some(w) = map
                            .get("contextWindow")
                            .and_then(|x| x.as_u64())
                        {
                            return Some(w);
                        }
                    }
                    for val in map.values() {
                        if let Some(w) = find(val, id) {
                            return Some(w);
                        }
                    }
                    None
                }
                Value::Array(arr) => arr.iter().find_map(|x| find(x, id)),
                _ => None,
            }
        }
        find(&root, model_id)
    })();
    if let Some(w) = found {
        cache.insert(model_id.to_string(), w);
        Some(w)
    } else {
        None
    }
}

/// A PATH that includes common Homebrew/local bins. GUI-launched apps have a
/// minimal PATH; child pi/rmux processes need /opt/homebrew/bin etc. so the
/// pi-subagent-durable extension can find rmux (execSync("rmux -V")).
pub fn full_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let existing = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    for p in [
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
        format!("{home}/.local/bin"),
        format!("{home}/.pi/agent/bin"),
    ] {
        if !parts.iter().any(|x| x == &p) {
            parts.push(p);
        }
    }
    for p in existing.split(':') {
        if !p.is_empty() && !parts.iter().any(|x| x == p) {
            parts.push(p.to_string());
        }
    }
    parts.join(":")
}

/// Is this session uuid one of the subagent uuids (from mirrors / agent-logs)?
pub fn is_subagent_uuid(id: &str) -> bool {
    let (uuids, _, _) = subagent_index();
    uuids.contains(id)
}

/// Read the session uuid from a session file's header (first line only).
pub fn session_id(path: &str) -> Option<String> {
    first_line(Path::new(path)).and_then(|l| {
        serde_json::from_str::<Value>(&l)
            .ok()
            .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()))
    })
}

// ---------------------------------------------------------------------------
// Subagent detection: a session is a subagent if its uuid appears in any
// agent-logs/task-*.jsonl first line, or in a *subagent-task-* mirror header,
// or its filename contains "subagent-task-". The pi-subagent-durable extension
// spawns real pi sessions (normal filenames) that ALSO have a slim mirror file
// and a task log — all sharing the same session uuid.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Cached indexes (rebuilt only when files change, so the 10s UI poll is cheap)
// ---------------------------------------------------------------------------

fn newest_mtime_secs(dir: &Path) -> u64 {
    let mut newest = 0u64;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Ok(sub) = fs::read_dir(&p) {
                    for f in sub.flatten() {
                        if let Ok(md) = f.metadata() {
                            if let Ok(mt) = md.modified() {
                                if let Ok(d) = mt.duration_since(std::time::UNIX_EPOCH) {
                                    newest = newest.max(d.as_secs());
                                }
                            }
                        }
                    }
                }
            } else if let Ok(md) = e.metadata() {
                if let Ok(mt) = md.modified() {
                    if let Ok(d) = mt.duration_since(std::time::UNIX_EPOCH) {
                        newest = newest.max(d.as_secs());
                    }
                }
            }
        }
    }
    newest
}

type SubIdx = (
    HashSet<String>,
    HashMap<String, String>,
    HashMap<String, String>,
);
static SUB_IDX: OnceLock<Mutex<Option<(u64, SubIdx)>>> = OnceLock::new();

pub fn subagent_index() -> SubIdx {
    let mut cache = SUB_IDX.get_or_init(|| Mutex::new(None)).lock().unwrap();
    let key = newest_mtime_secs(&sessions_dir()) ^ newest_mtime_secs(&pi_agent_dir().join("agent-logs"));
    if let Some((k, idx)) = cache.as_ref() {
        if *k == key {
            return idx.clone();
        }
    }
    let idx = build_subagent_index();
    *cache = Some((key, idx.clone()));
    idx
}

fn build_subagent_index() -> SubIdx {
    // uuid -> taskId
    let mut by_uuid: HashMap<String, String> = HashMap::new();
    // uuid -> first user message (from mirror files; matches the parent's
    // original subagent call text better than the resumed real session)
    let mut match_text_by_uuid: HashMap<String, String> = HashMap::new();
    // 1) agent-logs/task-<taskId>.jsonl first line -> session uuid
    let logs = pi_agent_dir().join("agent-logs");
    if let Ok(fd) = fs::read_dir(&logs) {
        for f in fd.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if !name.starts_with("task-") || !name.ends_with(".jsonl") {
                continue;
            }
            let task_id = name
                .strip_prefix("task-")
                .and_then(|s| s.strip_suffix(".jsonl"))
                .map(|s| s.to_string());
            if let Some(line) = first_line(&f.path()) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                        if let Some(t) = &task_id {
                            by_uuid.insert(id.to_string(), t.clone());
                        } else {
                            by_uuid.entry(id.to_string()).or_default();
                        }
                    }
                }
            }
        }
    }
    // 2) mirror files: header id + taskId + first user message
    let root = sessions_dir();
    if let Ok(rd) = fs::read_dir(&root) {
        for e in rd.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            if let Ok(fd) = fs::read_dir(&path) {
                for f in fd.flatten() {
                    let name = f.file_name().to_string_lossy().to_string();
                    if !name.contains("subagent-task-") || !name.ends_with(".jsonl") {
                        continue;
                    }
                    let task_id = task_id_from_filename(&name);
                    let Ok(data) = fs::read(f.path()) else { continue };
                    let text = String::from_utf8_lossy(&data);
                    let mut first_msg = None;
                    for (i, line) in text.lines().enumerate() {
                        if i > 20 {
                            break;
                        }
                        let Ok(v) = serde_json::from_str::<Value>(line) else {
                            continue;
                        };
                        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
                            "session" => {
                                if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
                                    if let Some(t) = &task_id {
                                        by_uuid.insert(id.to_string(), t.clone());
                                    } else {
                                        by_uuid.entry(id.to_string()).or_default();
                                    }
                                }
                            }
                            "message" => {
                                if let Some(m) = v.get("message") {
                                    if m.get("role").and_then(|x| x.as_str()) == Some("user")
                                        && first_msg.is_none()
                                    {
                                        first_msg = text_of_message(m, 400);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(fm)) = (
                        text.lines()
                            .next()
                            .and_then(|l| serde_json::from_str::<Value>(l).ok())
                            .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())),
                        first_msg,
                    ) {
                        match_text_by_uuid.insert(id, fm);
                    }
                }
            }
        }
    }
    let uuids: HashSet<String> = by_uuid.keys().cloned().collect();
    (uuids, by_uuid, match_text_by_uuid)
}

/// Task lifecycle, process-alive aware:
/// - Running:     process alive, last event is not a long sleep
/// - Sleeping:    process alive, currently executing a bash `sleep` (will auto-continue)
/// - Interrupted: process dead + no terminal event (killed, resumable via reload)
/// - Finished:    last line is a terminal event (agent_end/agent_settled)
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum TaskStatus {
    Running,
    Sleeping,
    Interrupted,
    Finished,
    Unknown,
}

/// Subagent pi processes are visible in `ps` with the agent-logs path
/// `task-<id>.jsonl` (or the `pi-task-<id>.md` system-prompt arg) in the
/// command line. One ps call covers every task.
fn alive_task_ids() -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(res) = std::process::Command::new("ps")
        .args(["-axo", "command"])
        .output()
    else {
        return out;
    };
    let text = String::from_utf8_lossy(&res.stdout);
    for line in text.lines() {
        // task ids appear as path suffixes, e.g. .../agent-logs/task-<id>.jsonl
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i + 5 <= bytes.len() {
            if &bytes[i..i + 5] == b"task-" {
                let rest = &line[i + 5..];
                if let Some(idx) = rest.find(".jsonl") {
                    let id = &rest[..idx];
                    if !id.is_empty()
                        && id.len() <= 48
                        && id
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-')
                    {
                        out.insert(id.to_string());
                        i += 5 + idx + 6;
                        continue;
                    }
                }
            }
            i += 1;
        }
    }
    out
}

/// Last agent-log event is a bash `sleep N` (the agent waiting, will continue).
fn last_is_sleep(data: &str) -> bool {
    let last = data.lines().rev().find(|l| !l.trim().is_empty());
    let Some(last) = last else { return false };
    let Ok(v) = serde_json::from_str::<Value>(last) else {
        return false;
    };
    let is_bash_tool = v.get("toolName").and_then(|x| x.as_str()) == Some("bash");
    if !is_bash_tool {
        return false;
    }
    let cmd = v
        .get("args")
        .and_then(|a| a.get("command"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    // "sleep 1500" / "sleep 2900; cd ..."
    let trimmed = cmd.trim_start();
    trimmed.starts_with("sleep ") || trimmed.starts_with("sleep\\t")
}

fn last_is_terminal(data: &str) -> bool {
    let last = data.lines().rev().find(|l| !l.trim().is_empty());
    let Some(last) = last else { return false };
    let Ok(v) = serde_json::from_str::<Value>(last) else {
        return false;
    };
    matches!(
        v.get("type").and_then(|x| x.as_str()),
        Some("agent_end") | Some("agent_settled")
    )
}

fn task_status(task_id: &str, alive: &HashSet<String>) -> TaskStatus {
    let p = pi_agent_dir()
        .join("agent-logs")
        .join(format!("task-{task_id}.jsonl"));
    let Ok(data) = fs::read(&p) else {
        return if alive.contains(task_id) {
            TaskStatus::Running
        } else {
            TaskStatus::Unknown
        };
    };
    let text = String::from_utf8_lossy(&data);
    if alive.contains(task_id) {
        // process alive: sleeping only while a bash sleep is running
        if last_is_sleep(&text) {
            TaskStatus::Sleeping
        } else {
            TaskStatus::Running
        }
    } else if last_is_terminal(&text) {
        TaskStatus::Finished
    } else {
        TaskStatus::Interrupted
    }
}

/// A lightweight snapshot of currently running sessions across ALL projects
/// (used by the frontend to notify when a running session finishes).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RunningSession {
    pub path: String,
    pub title: String,
    pub is_subagent: bool,
    pub task_id: Option<String>,
    pub project_key: String,
}

/// Status of a session file for the frontend: "running" | "sleeping" |
/// "finished" | "unknown". Used to classify a session that left the running set.
#[tauri::command]
pub fn session_status(path: String) -> String {
    let id = session_id(&path).unwrap_or_default();
    let (_, task_by_uuid, _) = subagent_index();
    if let Some(tid) = task_by_uuid.get(&id) {
        let alive = alive_task_ids();
        return match task_status(tid, &alive) {
            TaskStatus::Running => "running".into(),
            TaskStatus::Sleeping => "sleeping".into(),
            TaskStatus::Interrupted => "interrupted".into(),
            TaskStatus::Finished => "finished".into(),
            TaskStatus::Unknown => "unknown".into(),
        };
    }
    if session_file_running(Path::new(&path)) {
        "running".into()
    } else {
        "finished".into()
    }
}

#[tauri::command]
pub fn list_running() -> Vec<RunningSession> {
    let mut out = Vec::new();
    let root = sessions_dir();
    let running = running_set().lock().map(|s| s.clone()).unwrap_or_default();
    let (_, task_by_uuid, _) = subagent_index();
    let alive = alive_task_ids();
    let mut seen: HashSet<String> = HashSet::new();
    if let Ok(rd) = fs::read_dir(&root) {
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let project_key = e.file_name().to_string_lossy().to_string();
            if let Ok(fd) = fs::read_dir(&dir) {
                for f in fd.flatten() {
                    let path = f.path();
                    if !path.is_file() {
                        continue;
                    }
                    let fname = f.file_name().to_string_lossy().to_string();
                    if !fname.ends_with(".jsonl") {
                        continue;
                    }
                    let spath = path.to_string_lossy().to_string();
                    let (id, _, _, _, first_msg, _) = scan_head(&path);
                    if id.is_empty() || !seen.insert(id.clone()) {
                        continue;
                    }
                    let is_sub = fname.contains("subagent-task-") || task_by_uuid.contains_key(&id);
                    let is_running = if is_sub {
                        task_by_uuid.get(&id).map(|t| alive.contains(t)).unwrap_or(false)
                    } else {
                        session_file_running(&path)
                    };
                    if !is_running && !running.contains(&spath) {
                        continue;
                    }
                    out.push(RunningSession {
                        title: first_msg.unwrap_or_else(|| "(empty)".into()),
                        path: spath,
                        is_subagent: is_sub,
                        task_id: task_by_uuid.get(&id).cloned(),
                        project_key: project_key.clone(),
                    });
                }
            }
        }
    }
    out
}

/// A task id looks like `msmsnt8i-sebj` (8 lowercase alnum + "-" + 4 alnum).
fn is_task_id_token(s: &str, i: usize) -> bool {
    let b = s.as_bytes();
    if i + 13 > b.len() {
        return false;
    }
    if b[i + 8] != b'-' {
        return false;
    }
    let alnum_lc = |k: usize| b[k].is_ascii_digit() || b[k].is_ascii_lowercase();
    (0..8).all(|k| alnum_lc(i + k)) && (9..13).all(|k| alnum_lc(i + k))
}

fn extract_task_id(s: &str) -> Option<String> {
    let mut i = 0;
    while i + 13 <= s.len() {
        if is_task_id_token(s, i) {
            return Some(s[i..i + 13].to_string());
        }
        i += 1;
    }
    None
}

#[derive(Clone)]
pub struct RmuxRuntime {
    pub target: String, // e.g. "pi-Users-...:s<id8>"
    pub attached: bool, // a terminal client is currently attached (has UI)
    pub dead: bool,     // the pane process has exited (window kept by remain-on-exit)
}

/// Map of session file path -> rmux runtime info for sessions running inside
/// rmux panes.
///
/// rmux pane processes are pi itself, whose argv is scrubbed down to just
/// `pi` (the --session args are gone), so `ps -o command=` can never reveal
/// the session path. Instead we map by **window name**:
///
/// - Open TUI mains: window `s<id8>` (first 8 hex chars of the session id)
/// - subagents: window `<agent>-<taskId>` in the `pi-agents` session
///
/// Attached state comes from `rmux list-clients -t <session>`.
/// Alive pi processes running in terminal windows (not inside any rmux pane).
/// pi scrubs its argv to just "pi", so we identify them by process name and
/// exclude anything attached to an rmux pane pty. Returns (pid, cwd).
pub fn alive_terminal_pis() -> Vec<(u32, String)> {
    // lsof/ps are ~150ms; a 2s TTL dedupes the list_projects + list_sessions
    // calls inside one refresh cycle. Attach flows stay fresh (they take >2s).
    type TermCache = Option<(std::time::Instant, Vec<(u32, String)>)>;
    static CACHE: OnceLock<Mutex<TermCache>> = OnceLock::new();
    {
        let cache = CACHE.get_or_init(|| Mutex::new(None)).lock().unwrap();
        if let Some((at, res)) = cache.as_ref() {
            if at.elapsed() < std::time::Duration::from_secs(2) {
                return res.clone();
            }
        }
    }
    let mut out = Vec::new();
    let mut pending: Vec<u32> = Vec::new();
    // tty devices owned by rmux panes (normalized to "ttysNNN")
    let mut pane_ttys: HashSet<String> = HashSet::new();
    if let Ok(res) = std::process::Command::new("rmux")
        .args(["list-panes", "-a", "-F", "#{pane_tty}"])
        .env("PATH", full_path())
        .output()
    {
        for line in String::from_utf8_lossy(&res.stdout).lines() {
            let t = line.trim().trim_start_matches("/dev/");
            if !t.is_empty() {
                pane_ttys.insert(t.to_string());
            }
        }
    }
    let Ok(ps_out) = std::process::Command::new("ps")
        .args(["-axo", "pid=,tty=,comm="])
        .env("PATH", full_path())
        .output()
    else {
        return out;
    };
    for line in String::from_utf8_lossy(&ps_out.stdout).lines() {
        let mut it = line.split_whitespace();
        let (Some(pid_s), Some(tty), Some(comm)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if comm != "pi" || tty == "??" || pane_ttys.contains(tty) {
            continue;
        }
        let Ok(pid) = pid_s.parse::<u32>() else { continue };
        // cwd of the pi process = the project it runs in; one batched lsof
        // call for all candidate pids (lsof startup dominates the cost)
        pending.push(pid);
    }
    if !pending.is_empty() {
        let pid_list = pending
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        if let Ok(res) = std::process::Command::new("lsof")
            .args(["-a", "-p", &pid_list, "-d", "cwd", "-Fn"])
            .env("PATH", full_path())
            .output()
        {
            let mut cur: Option<u32> = None;
            for line in String::from_utf8_lossy(&res.stdout).lines() {
                if let Some(rest) = line.strip_prefix('p') {
                    cur = rest.parse::<u32>().ok();
                } else if let Some(rest) = line.strip_prefix('n') {
                    if let Some(pid) = cur {
                        out.push((pid, rest.to_string()));
                    }
                }
            }
        }
    }
    *CACHE.get_or_init(|| Mutex::new(None)).lock().unwrap() =
        Some((std::time::Instant::now(), out.clone()));
    out
}

/// cwd of a live process via lsof (single targeted call).
fn lsof_cwd(pid: u32) -> Option<String> {
    std::process::Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .env("PATH", full_path())
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find(|l| l.starts_with('n'))
                .map(|l| l[1..].to_string())
        })
        .filter(|c| !c.is_empty())
}

/// True when a pi process is currently running this session in a terminal
/// window — actively writing (fresh mtime) or alive-but-idle. Mirrors the
/// term_alive rule in list_sessions: the session must be among the N freshest
/// non-rmux main sessions of its project, where N = live terminal pis there.
/// Used by attach to avoid spawning a duplicate pi for an already-running
/// session (two pis appending the same jsonl corrupts it).
pub fn session_has_live_terminal_pi(session_path: &str) -> bool {
    let path = Path::new(session_path);
    if session_file_running(path) {
        return true;
    }
    let cwd = session_detail(session_path)
        .map(|d| d.cwd)
        .unwrap_or_default();
    let term_n = alive_terminal_pis()
        .iter()
        .filter(|(_, c)| *c == cwd)
        .count();
    if term_n == 0 {
        return false;
    }
    let Some(dir) = path.parent() else {
        return false;
    };
    let rmux_map = rmux_runtime_map();
    let mut mains: Vec<(i64, String)> = Vec::new();
    if let Ok(fd) = fs::read_dir(dir) {
        for f in fd.flatten() {
            let p = f.path();
            let name = f.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jsonl") || name.contains("subagent-task-") {
                continue;
            }
            let spath = p.to_string_lossy().into_owned();
            if rmux_map.get(&spath).map(|r| !r.dead).unwrap_or(false) {
                continue;
            }
            let mt = fmetadata(&p).map(|m| m.mtime).unwrap_or(0);
            mains.push((mt, spath));
        }
    }
    mains.sort_by_key(|a| std::cmp::Reverse(a.0));
    mains.iter().take(term_n).any(|(_, p)| p == session_path)
}

pub fn rmux_runtime_map() -> HashMap<String, RmuxRuntime> {
    type RmuxCache = Option<(std::time::Instant, HashMap<String, RmuxRuntime>)>;
    static CACHE: OnceLock<Mutex<RmuxCache>> = OnceLock::new();
    {
        let cache = CACHE.get_or_init(|| Mutex::new(None)).lock().unwrap();
        if let Some((at, res)) = cache.as_ref() {
            if at.elapsed() < std::time::Duration::from_secs(2) {
                return res.clone();
            }
        }
    }
    let mut out = HashMap::new();
    let Ok(res) = std::process::Command::new("rmux")
        .args(["list-panes", "-a", "-F", "#{session_name}:#{window_name}.#{pane_index} #{pane_pid} #{pane_dead} #{@pi_session}"])
        .env("PATH", full_path())
        .output()
    else {
        return out;
    };
    let pid_alive = |pid: u32| -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };

    // Build id8 -> path and taskId -> path indexes from one scan of the session dirs.
    let mut id8_map: HashMap<String, String> = HashMap::new();
    let mut task_map: HashMap<String, String> = HashMap::new();
    if let Ok(root) = fs::read_dir(sessions_dir()) {
        for dir in root.flatten() {
            if let Ok(entries) = fs::read_dir(dir.path()) {
                for f in entries.flatten() {
                    let name = f.file_name().to_string_lossy().to_string();
                    if !name.ends_with(".jsonl") {
                        continue;
                    }
                    let p = f.path().to_string_lossy().into_owned();
                    if name.contains("subagent-") {
                        // <ts>_subagent-task-<taskId>.jsonl (or _subagent-<taskId>)
                        let tail = &name[name.rfind("subagent-").unwrap() + "subagent-".len()..];
                        if let Some(tid) = extract_task_id(tail) {
                            task_map.entry(tid).or_insert_with(|| p.clone());
                        }
                    } else if let Some(pos) = name.find('_') {
                        // main session: <ts>_<uuid>.jsonl -> id8 from the uuid part
                        let id_part = &name[pos + 1..];
                        if id_part.len() >= 9 && id_part.as_bytes()[8] == b'-' {
                            let id8 = &id_part[..8];
                            if id8.chars().all(|c| c.is_ascii_hexdigit()) {
                                // id8 prefix collision (two sessions sharing the
                                // first 8 chars): keep the freshest mtime — the
                                // window's pi was created for the most recent one
                                match id8_map.get(id8) {
                                    Some(existing) => {
                                        let new_mt = fmetadata(&f.path()).map(|m| m.mtime).unwrap_or(0);
                                        let old_mt = fmetadata(Path::new(existing)).map(|m| m.mtime).unwrap_or(0);
                                        if new_mt > old_mt {
                                            id8_map.insert(id8.to_string(), p);
                                        }
                                    }
                                    None => {
                                        id8_map.insert(id8.to_string(), p);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // session name -> has attached client (queried once per session)
    let mut attached_cache: HashMap<String, bool> = HashMap::new();
    let add = |path: String, target: String, sess: &str, dead: bool, out: &mut HashMap<String, RmuxRuntime>, attached_cache: &mut HashMap<String, bool>| {
        // a LIVE window wins over a DEAD one for the same session: after the
        // user kills a window and reopens the session, a stale dead pane (with
        // its leftover @pi_session option) must not override the new live pane
        if let Some(existing) = out.get(&path) {
            if !existing.dead {
                return; // already have a live window — ignore dead duplicates
            }
        }
        let attached = *attached_cache.entry(sess.to_string()).or_insert_with(|| {
            std::process::Command::new("rmux")
                .args(["list-clients", "-t", sess])
                .env("PATH", full_path())
                .output()
                .map(|o| !o.stdout.is_empty() && o.status.success())
                .unwrap_or(false)
        });
        out.insert(path, RmuxRuntime { target, attached, dead });
    };

    for line in String::from_utf8_lossy(&res.stdout).lines() {
        let mut parts = line.splitn(4, ' ');
        let (target, pid_s) = (parts.next().unwrap_or("").trim(), parts.next().unwrap_or("").trim());
        let dead = parts.next().unwrap_or("").trim() == "1";
        let opt = parts.next().unwrap_or("").trim().to_string();
        let Ok(pid) = pid_s.parse::<u32>() else { continue };
        if !dead && !pid_alive(pid) {
            continue;
        }
        let sess = target.split(':').next().unwrap_or("").to_string();
        // authoritative: the window records which session it was created for
        // (set by ensure_rmux_window). this beats the id8-prefix heuristic,
        // which misattributes windows when two sessions share the first 8
        // chars of their uuid.
        if !opt.is_empty() && Path::new(&opt).is_file() {
            add(opt.clone(), target.to_string(), &sess, dead, &mut out, &mut attached_cache);
            continue;
        }
        let win = target.split(':').nth(1).unwrap_or("").to_string();
        // Open TUI main: window s<id8>
        if let Some(id8) = win.strip_prefix('s') {
            if id8.len() >= 8 && id8[..8].chars().all(|c| c.is_ascii_hexdigit()) {
                if let Some(p) = id8_map.get(&id8[..8]) {
                    add(p.clone(), target.to_string(), &sess, dead, &mut out, &mut attached_cache);
                }
                continue;
            }
        }
        // subagent: window <agent>-<taskId>
        if let Some(tid) = extract_task_id(&win) {
            if let Some(p) = task_map.get(&tid) {
                add(p.clone(), target.to_string(), &sess, dead, &mut out, &mut attached_cache);
                continue;
            }
        }
        // fallback: pi scrubs its argv to just "pi", so pane command lines can
        // not reveal the session. For a bare `pi` pane (e.g. the `pim` helper
        // creates sessions named pi-<proj> with a generic window name), map by
        // the pane's cwd -> the freshest main session of that project.
        if let Ok(ps_out) = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .env("PATH", full_path())
            .output()
        {
            let cmd = String::from_utf8_lossy(&ps_out.stdout);
            if cmd.trim() == "pi" {
                if let Some(p) = pane_cwd_session(&pid, &out) {
                    add(p, target.to_string(), &sess, dead, &mut out, &mut attached_cache);
                    continue;
                }
            }
            for tok in cmd.split_whitespace() {
                if tok == "--session" {
                    continue;
                }
                let clean = tok.trim_matches('\'');
                if clean.ends_with(".jsonl") && clean.contains("sessions/") {
                    add(clean.to_string(), target.to_string(), &sess, dead, &mut out, &mut attached_cache);
                    break;
                }
            }
        }
    }
    *CACHE.get_or_init(|| Mutex::new(None)).lock().unwrap() =
        Some((std::time::Instant::now(), out.clone()));
    out
}

/// Map a bare `pi` pane (cwd known, session not discoverable from argv or the
/// window name) to a main session of its project that is not already claimed
/// by another rmux pane.
///
/// pi resumes the latest session as of launch, so we pick the session whose
/// mtime is closest to the pane pi's start time. A plain "current freshest"
/// rule breaks when a DIFFERENT pi (e.g. a terminal one) is actively writing
/// its own session right now: that session would win by mtime but is not the
/// one this pane is running.
fn pane_cwd_session(pid: &u32, already: &HashMap<String, RmuxRuntime>) -> Option<String> {
    let cwd = lsof_cwd(*pid)?;
    let dir = sessions_dir().join(encode_dir_name(&cwd));
    let start = process_start_epoch(*pid)?;
    let mut best: Option<(i64, String)> = None; // (|mtime - start|, path)
    if let Ok(fd) = fs::read_dir(&dir) {
        for f in fd.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jsonl") || name.contains("subagent-") {
                continue;
            }
            let p = f.path().to_string_lossy().into_owned();
            if already.contains_key(&p) {
                continue;
            }
            let mt = fmetadata(&f.path()).map(|m| m.mtime).unwrap_or(0);
            let dist = (mt - start).abs();
            if best.as_ref().map(|(b, _)| dist < *b).unwrap_or(true) {
                best = Some((dist, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Epoch seconds when the process started. macOS `ps` has no `etimes`, so we
/// parse `etime` (formats: MM:SS, HH:MM:SS, D-HH:MM:SS).
fn process_start_epoch(pid: u32) -> Option<i64> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "etime="])
        .env("PATH", full_path())
        .output()
        .ok()?;
    let etime = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut days = 0i64;
    let time_part = if let Some((d, t)) = etime.split_once('-') {
        days = d.parse::<i64>().ok()?;
        t
    } else {
        &etime
    };
    let parts: Vec<i64> = time_part
        .split(':')
        .map(|p| p.parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let secs = match parts.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    } + days * 86400;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some(now - secs)
}

pub fn list_sessions(project_key: &str) -> Vec<SessionMeta> {
    let root = sessions_dir();
    let dir = root.join(project_key);
    let mut out = Vec::new();
    let running = running_set().lock().map(|s| s.clone()).unwrap_or_default();
    let alive = alive_task_ids();
    let rmux_map = rmux_runtime_map();
    let (sub_uuids, task_by_uuid, match_text_by_uuid) = subagent_index();

    if let Ok(fd) = fs::read_dir(&dir) {
        for f in fd.flatten() {
            let path = f.path();
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.ends_with(".jsonl") {
                continue;
            }
            if let Some(mut m) = parse_meta(&path, &fname, &running, &sub_uuids, &task_by_uuid) {
                if m.is_subagent {
                    if let Some(tid) = &m.task_id {
                        match task_status(tid, &alive) {
                            TaskStatus::Running => m.running = true,
                            TaskStatus::Sleeping => m.sleeping = true,
                            TaskStatus::Interrupted => m.interrupted = true,
                            _ => {}
                        }
                    }
                    // subagents live in rmux pi-agents when running
                    if m.running {
                        m.in_rmux = true;
                        if let Some(rt) = rmux_map.get(&m.path) {
                            m.rmux_target = Some(rt.target.clone());
                            m.rmux_attached = rt.attached;
                        } else {
                            m.rmux_target = Some("pi-agents".to_string());
                        }
                    }
                } else if let Some(rt) = rmux_map.get(&m.path) {
                    // runtime location is independent of the task state: an idle
                    // pi parked in an rmux window is not "running" but is in rmux
                    m.in_rmux = true;
                    m.rmux_target = Some(rt.target.clone());
                    m.rmux_attached = rt.attached;
                    m.rmux_dead = rt.dead;
                    if !rt.dead {
                        // running = pi actively writing (mtime fresh)
                        m.running = session_file_running(&path);
                    }
                } else {
                    // not in rmux: running = pi actively writing in a terminal
                    m.running = session_file_running(&path);
                }
                out.push(m);
            }
        }
    }
    // alive terminal pis in this project -> mark the freshest main sessions
    // that aren't in rmux as term_alive (chip shows even when the pi is idle)
    let term_n = alive_terminal_pis()
        .iter()
        .filter(|(_, c)| *c == decode_dir_name(project_key))
        .count();
    if term_n > 0 {
        let mut cands: Vec<&mut SessionMeta> = out
            .iter_mut()
            .filter(|m| !m.is_subagent && !m.in_rmux)
            .collect();
        cands.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
        for m in cands.into_iter().take(term_n) {
            m.term_alive = true;
        }
    }

    // dedupe: same uuid can have a mirror (subagent-task-*) and a real session
    // file (normal name). Prefer the real one (fuller history).
    let mut by_id: HashMap<String, SessionMeta> = HashMap::new();
    for m in out {
        let cur = by_id.get(&m.id);
        let replace = match cur {
            None => true,
            Some(c) => {
                if !c.path.contains("subagent-task-") && m.path.contains("subagent-task-") {
                    false // current is real pi session, new is mirror -> keep current
                } else if c.path.contains("subagent-task-") && !m.path.contains("subagent-task-") {
                    true // current is mirror, new is real -> replace
                } else {
                    m.size > c.size
                }
            }
        };
        if replace {
            by_id.insert(m.id.clone(), m);
        }
    }
    let mut out: Vec<SessionMeta> = by_id.into_values().collect();

    // parent linkage: match subagent first message against the parent session's
    // subagent tool calls (task text), within this project.
    let parent_calls = collect_parent_calls(project_key);
    for m in out.iter_mut() {
        if m.is_subagent {
            let match_text = match_text_by_uuid
                .get(&m.id)
                .cloned()
                .or_else(|| m.first_message.clone());
            if let Some(fm) = match_text {
                if let Some(p) = match_parent(&fm, &parent_calls) {
                    m.parent_session_path = Some(p);
                }
            }
        }
    }

    out.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
    out
}

fn parse_meta(
    path: &Path,
    fname: &str,
    running: &HashSet<String>,
    sub_uuids: &HashSet<String>,
    task_by_uuid: &HashMap<String, String>,
) -> Option<SessionMeta> {
    // per-file cache keyed on (mtime, size): steady-state refreshes re-read
    // only the files that actually changed
    type MetaCache = HashMap<String, (i64, u64, SessionMeta)>;
    static META_CACHE: OnceLock<Mutex<MetaCache>> = OnceLock::new();
    let pstr = path.to_string_lossy().into_owned();
    {
        let cache = META_CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        if let Some((mt, sz, m)) = cache.get(&pstr) {
            if let Some(md) = fmetadata(path) {
                if md.mtime == *mt && md.size == *sz {
                    return Some(m.clone());
                }
            }
        }
    }
    let (id, cwd, created_iso, name, first_msg, model) = scan_head(path);
    if id.is_empty() {
        // not a valid pi session file; skip
        return None;
    }
    let is_sub = fname.contains("subagent-task-") || sub_uuids.contains(&id);
    let md = fmetadata(path);
    let mtime = md.as_ref().map(|m| m.mtime).unwrap_or(0);
    let size = md.as_ref().map(|m| m.size).unwrap_or(0);
    let last_msg = tail_preview(path, 160);
    let spath = path.to_string_lossy().to_string();
    let task_id = if is_sub {
        task_id_from_filename(fname).or_else(|| task_by_uuid.get(&id).cloned())
    } else {
        None
    };
    let meta = SessionMeta {
        path: spath.clone(),
        id: id.clone(),
        cwd,
        name,
        first_message: first_msg,
        last_message: last_msg,
        created_iso: created_iso.clone(),
        created_at: parse_iso_ts(&created_iso).unwrap_or(mtime),
        updated_at: mtime,
        model,
        is_subagent: is_sub,
        task_id,
        parent_session_id: None,
        parent_session_path: None,
        message_count: 0,
        running: running.contains(&spath),
        sleeping: false,
        interrupted: false,
        in_rmux: false,
        rmux_target: None,
        rmux_attached: false,
        rmux_dead: false,
        term_alive: false,
        size,};
    META_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(
            pstr,
            (
                md.as_ref().map(|m| m.mtime).unwrap_or(mtime),
                md.as_ref().map(|m| m.size).unwrap_or(size),
                meta.clone(),
            ),
        );
    Some(meta)
}

// ---------------------------------------------------------------------------
// Parent linkage: the pi-subagent-durable tool result does not record the
// taskId, so we match the subagent's first user message ("Task: ...") against
// the parent session's `subagent` tool call `task` text. Prefix/containment
// + length ratio gives exact matches (score 1.0) in the common case; orphans
// land in the dedicated subagent section.
// ---------------------------------------------------------------------------

struct ParentCall {
    path: String,
    task: String, // normalized
}

impl Clone for ParentCall {
    fn clone(&self) -> Self {
        ParentCall {
            path: self.path.clone(),
            task: self.task.clone(),
        }
    }
}

fn normalize_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_parent_calls(project_key: &str) -> Vec<ParentCall> {
    // Incremental per-file cache: keyed on the (mtime,size) map of every main
    // session. When nothing changed we reuse everything; when a session grows
    // (an active run), only that file is rescanned.
    type ParentCache = Option<(String, HashMap<String, (i64, u64)>, Vec<ParentCall>)>;
    static CACHE: OnceLock<Mutex<ParentCache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock().unwrap();
    let dir = sessions_dir().join(project_key);

    let mut cur: HashMap<String, (i64, u64)> = HashMap::new();
    if let Ok(fd) = fs::read_dir(&dir) {
        for f in fd.flatten() {
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.ends_with(".jsonl") || fname.contains("subagent-task-") {
                continue;
            }
            if let Some(md) = fmetadata(&f.path()) {
                cur.insert(f.path().to_string_lossy().into_owned(), (md.mtime, md.size));
            }
        }
    }
    if let Some((pk, prev, calls)) = cache.as_ref() {
        if pk == project_key && *prev == cur {
            return calls.clone();
        }
    }
    // keep cached calls for files whose (mtime,size) is unchanged, drop stale
    // ones (changed or deleted), rescan the rest
    let prev_map: HashMap<String, (i64, u64)> = cache
        .as_ref()
        .filter(|(pk, _, _)| pk == project_key)
        .map(|(_, m, _)| m.clone())
        .unwrap_or_default();
    let mut out: Vec<ParentCall> = cache
        .as_ref()
        .filter(|(pk, _, _)| pk == project_key)
        .map(|(_, _, calls)| {
            calls
                .iter()
                .filter(|pc| prev_map.get(&pc.path) == cur.get(&pc.path))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    for (path, md) in &cur {
        if prev_map.get(path) != Some(md) {
            scan_file_for_parent_calls(Path::new(path), &mut out);
        }
    }
    let cached = out.clone();
    *cache = Some((project_key.to_string(), cur, cached));
    out
}

fn scan_file_for_parent_calls(path: &Path, out: &mut Vec<ParentCall>) {
    let Ok(data) = fs::read(path) else { return };
    let text = String::from_utf8_lossy(&data);
    for line in text.lines() {
        if !line.contains("subagent") {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(m) = v.get("message") else { continue };
        if m.get("role").and_then(|x| x.as_str()) != Some("assistant") {
            continue;
        }
        let Some(content) = m.get("content").and_then(|x| x.as_array()) else {
            continue;
        };
        for c in content {
            if c.get("type").and_then(|x| x.as_str()) != Some("toolCall") {
                continue;
            }
            if c.get("name").and_then(|x| x.as_str()) != Some("subagent") {
                continue;
            }
            let args = c.get("arguments").cloned().unwrap_or(Value::Null);
            extract_tasks(&args, out, path);
        }
    }
}

fn extract_tasks(args: &Value, out: &mut Vec<ParentCall>, path: &Path) {
    match args {
        Value::String(s) => {
            if let Ok(v) = serde_json::from_str::<Value>(s) {
                extract_tasks(&v, out, path);
            }
        }
        Value::Object(map) => {
            if let Some(t) = map.get("task").and_then(|x| x.as_str()) {
                out.push(ParentCall {
                    path: path.to_string_lossy().to_string(),
                    task: normalize_text(t),
                });
            }
            if let Some(tasks) = map.get("tasks").and_then(|x| x.as_array()) {
                for t in tasks {
                    if let Some(s) = t.get("task").and_then(|x| x.as_str()) {
                        out.push(ParentCall {
                            path: path.to_string_lossy().to_string(),
                            task: normalize_text(s),
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

fn match_parent(first_message: &str, calls: &[ParentCall]) -> Option<String> {
    let body = first_message
        .split_once("Task:")
        .map(|(_, r)| r)
        .unwrap_or(first_message);
    let body = normalize_text(body);
    if body.is_empty() {
        return None;
    }
    let body_alpha = extract_alpha_id(&body);
    let mut best: Option<(f64, String)> = None;
    for c in calls {
        if c.task.is_empty() {
            continue;
        }
        let a = &c.task;
        let b = &body;
        let mut score = if a.contains(b) || b.contains(a) {
            let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
            short.len() as f64 / long.len() as f64
        } else {
            0.0
        };
        // alpha id (e.g. blQqQL86) present in both -> strong signal
        if let Some(pid) = extract_alpha_id(a) {
            if body_alpha.as_deref() == Some(pid.as_str()) {
                score = score.max(1.0);
            }
        }
        // long common prefix (re-submission with appended context)
        let lcp = common_prefix_len(a, b);
        score = score.max((lcp as f64 / 50.0).min(1.0));
        if score >= 0.55 && best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, c.path.clone()));
        }
    }
    best.map(|(_, p)| p)
}

/// Extract an 8-char alpha id right after "alpha"/"Alpha" (WorldQuant style).
fn extract_alpha_id(s: &str) -> Option<String> {
    let lower = s.to_lowercase();
    for (i, m) in lower.match_indices("alpha") {
        let rest = &s[i + m.len()..];
        let rest = rest.trim_start_matches([':', ' ', '　']);
        let rest = rest.trim_start();
        let token: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if token.len() >= 8 && token.len() <= 12 {
            return Some(token);
        }
    }
    None
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

struct Fmeta {
    mtime: i64,
    size: u64,
}

fn fmetadata(path: &Path) -> Option<Fmeta> {
    let md = fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(Fmeta {
        mtime,
        size: md.len(),
    })
}

/// Read header + first 120 lines for id/cwd/name/first message/model.
fn scan_head(path: &Path) -> (String, String, String, Option<String>, Option<String>, Option<String>) {
    let mut id = String::new();
    let mut cwd = String::new();
    let mut created = String::new();
    let mut name = None;
    let mut first_msg = None;
    let mut model = None;

    // bounded read: session headers live in the first few lines; reading the
    // whole file here made list_sessions O(file-size) for every session
    use std::io::Read;
    let data = match fs::File::open(path).and_then(|mut f| {
        let mut buf = vec![0u8; 256 * 1024];
        let n = f.read(&mut buf)?;
        Ok(buf[..n].to_vec())
    }) {
        Ok(d) => d,
        Err(_) => return (id, cwd, created, name, first_msg, model),
    };
    let text = String::from_utf8_lossy(&data);
    for (i, line) in text.lines().enumerate() {
        // stop early once we have the essentials: headers are the first lines
        if i > 40 || (!id.is_empty() && !cwd.is_empty() && !created.is_empty() && first_msg.is_some()) {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session" => {
                id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                cwd = v.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string();
                created = v
                    .get("timestamp")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
            }
            "session_info" => {
                name = v.get("name").and_then(|x| x.as_str()).map(|s| s.to_string());
            }
            "message" => {
                if let Some(m) = v.get("message") {
                    let role = m.get("role").and_then(|x| x.as_str()).unwrap_or("");
                    if role == "user" {
                        if first_msg.is_none() {
                            first_msg = text_of_message(m, 140);
                        }
                    } else if role == "assistant" && model.is_none() {
                        model = m.get("model").and_then(|x| x.as_str()).map(|s| s.to_string());
                    }
                }
            }
            "model_change" if model.is_none() => {
                model = v.get("modelId").and_then(|x| x.as_str()).map(|s| s.to_string());
            }
            _ => {}
        }
    }
    (id, cwd, created, name, first_msg, model)
}

/// Extract readable text preview from a message object.
pub fn text_of_message(m: &Value, max: usize) -> Option<String> {
    let content = m.get("content")?;
    let mut out = String::new();
    match content {
        Value::String(s) => out.push_str(s),
        Value::Array(arr) => {
            for c in arr {
                if let Some(t) = c.get("text").and_then(|x| x.as_str()) {
                    out.push_str(t);
                    out.push(' ');
                } else if let Some(th) = c.get("thinking").and_then(|x| x.as_str()) {
                    out.push_str(th);
                    out.push(' ');
                }
            }
        }
        _ => {}
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        return None;
    }
    let mut chars: Vec<char> = out.chars().collect();
    if chars.len() > max {
        chars.truncate(max);
        chars.push('…');
    }
    Some(chars.into_iter().collect())
}

/// Preview from the tail of the file (last user/assistant text).
fn tail_preview(path: &Path, max_chars: usize) -> Option<String> {
    let md = fs::metadata(path).ok()?;
    let size = md.len();
    let tail = 16 * 1024u64;
    let (buf, _off) = if size > tail {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = fs::File::open(path).ok()?;
        f.seek(SeekFrom::End(-(tail as i64))).ok()?;
        let mut b = vec![0u8; tail as usize];
        let n = f.read(&mut b).ok()?;
        (b[..n].to_vec(), true)
    } else {
        (fs::read(path).ok()?, false)
    };
    let text = String::from_utf8_lossy(&buf);
    let mut preview = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        if let Some(m) = v.get("message") {
            let role = m.get("role").and_then(|x| x.as_str()).unwrap_or("");
            if role == "user" || role == "assistant" {
                if let Some(t) = text_of_message(m, max_chars) {
                    preview = Some(format!("{}: {}", if role == "user" { "👤" } else { "🤖" }, t));
                }
            }
        }
    }
    preview
}

// ---------------------------------------------------------------------------
// Detail parsing
// ---------------------------------------------------------------------------

pub fn session_detail(path: &str) -> Result<SessionDetail, String> {
    // cache by (mtime, size): re-parsing a multi-MB session on every switch is
    // the dominant cost; idle files only change when a new message lands
    type DetailCache = HashMap<String, (i64, u64, SessionDetail)>;
    static DETAIL_CACHE: OnceLock<Mutex<DetailCache>> = OnceLock::new();
    let key = path.to_string();
    {
        let cache = DETAIL_CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        if let Some((mt, sz, d)) = cache.get(&key) {
            if let Some(md) = fmetadata(Path::new(path)) {
                if md.mtime == *mt && md.size == *sz {
                    return Ok(d.clone());
                }
            }
        }
    }
    let data = fs::read(path).map_err(|e| format!("Failed to read session file: {e}"))?;
    let text = String::from_utf8_lossy(&data);

    let mut header_id = String::new();
    let mut cwd = String::new();
    let mut created = String::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut index_by_id: HashMap<String, usize> = HashMap::new();
    let mut children: HashMap<Option<String>, Vec<usize>> = HashMap::new();

    let mut tokens: u64 = 0;
    let mut msg_count = 0usize;
    let mut model: Option<String> = None;
    let mut provider: Option<String> = None;
    let mut thinking_level: Option<String> = None;
    let mut context_tokens: Option<u64> = None;
    let mut cost_total = 0.0f64;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if t == "session" {
            header_id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
            cwd = v.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string();
            created = v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            continue;
        }
        let entry = parse_entry(&t, &v);
        if entry.is_none() {
            continue;
        }
        let mut entry = entry.unwrap();
        // accumulate stats (last occurrence wins for model/provider/thinking)
        if let Some(m) = &entry.model {
            model = Some(m.clone());
        }
        if t == "model_change" {
            if let Some(p) = v.get("provider").and_then(|x| x.as_str()) {
                provider = Some(p.to_string());
            }
        }
        if t == "thinking_level_change" {
            if let Some(lv) = v.get("thinkingLevel").and_then(|x| x.as_str()) {
                thinking_level = Some(lv.to_string());
            }
        }
        if t == "message" {
            msg_count += 1;
            if let Some(m) = v.get("message") {
                if let Some(p) = m.get("provider").and_then(|x| x.as_str()) {
                    provider = Some(p.to_string());
                }
                // context size = input + cacheRead of the latest assistant reply
                if m.get("role").and_then(|x| x.as_str()) == Some("assistant") {
                    if let Some(u) = m.get("usage") {
                        let input = u.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
                        let cache = u.get("cacheRead").and_then(|x| x.as_u64()).unwrap_or(0);
                        let total = input + cache;
                        if total > 0 {
                            context_tokens = Some(total);
                        }
                    }
                }
                if let Some(u) = m.get("usage") {
                    if let Some(tot) = u.get("totalTokens").and_then(|x| x.as_u64()) {
                        tokens += tot;
                    }
                    if let Some(c) = u.get("cost").and_then(|c| c.get("total")).and_then(|x| x.as_f64()) {
                        cost_total += c;
                    }
                }
            }
            if let Some(u) = v.get("usage").and_then(|u| u.get("totalTokens")).and_then(|x| x.as_u64()) {
                tokens += u;
            }
        }
        if t == "compaction" {
            if let Some(u) = v.get("usage").and_then(|u| u.get("totalTokens")).and_then(|x| x.as_u64()) {
                tokens += u;
            }
        }
        let pid = entry.parent_id.clone();
        let idx = entries.len();
        index_by_id.insert(entry.id.clone(), idx);
        children.entry(pid).or_default().push(idx);
        entries.push(std::mem::replace(&mut entry, dummy_entry()));
    }

    // active branch: walk parent chain from the last entry
    let mut active = Vec::new();
    if let Some(leaf) = index_by_id.get(&entries.last().map(|e| e.id.clone()).unwrap_or_default()) {
        let mut cur = Some(*leaf);
        let mut chain = Vec::new();
        while let Some(i) = cur {
            chain.push(i);
            cur = entries[i].parent_id.as_ref().and_then(|p| index_by_id.get(p)).copied();
        }
        chain.reverse();
        active = chain;
    }

    let (_, task_by_uuid, _) = subagent_index();
    let task_id = task_by_uuid.get(&header_id).cloned();
    let stats = Stats {
        token_count: tokens,
        message_count: msg_count,
        model: model.clone(),
        provider,
        thinking_level,
        context_tokens,
        context_limit: model.as_deref().and_then(model_context_window),
        cost_total,
    };
    let detail = SessionDetail {
        id: header_id,
        cwd,
        created_iso: created,
        path: path.to_string(),
        task_id,
        stats,
        entries,
        active,
    };
    let (mtime, size) = fmetadata(Path::new(path))
        .map(|m| (m.mtime, m.size))
        .unwrap_or((0, 0));
    DETAIL_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(key, (mtime, size, detail.clone()));
    Ok(detail)
}

fn dummy_entry() -> Entry {
    Entry {
        kind: String::new(),
        id: String::new(),
        parent_id: None,
        ts: None,
        role: None,
        content: vec![],
        model: None,
        tool_name: None,
        tool_call_id: None,
        is_error: None,
        summary: None,
        name: None,
        label: None,
    }
}

fn parse_entry(t: &str, v: &Value) -> Option<Entry> {
    let id = v.get("id").and_then(|x| x.as_str())?.to_string();
    let parent_id = v.get("parentId").and_then(|x| x.as_str()).map(|s| s.to_string());
    let ts = v
        .get("timestamp")
        .map(|x| match x {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty());

    let mut e = Entry {
        kind: t.to_string(),
        id,
        parent_id,
        ts,
        role: None,
        content: vec![],
        model: None,
        tool_name: None,
        tool_call_id: None,
        is_error: None,
        summary: None,
        name: None,
        label: None,
    };

    match t {
        "message" => {
            let m = v.get("message")?;
            e.role = m.get("role").and_then(|x| x.as_str()).map(|s| s.to_string());
            e.model = m.get("model").and_then(|x| x.as_str()).map(|s| s.to_string());
            match e.role.as_deref() {
                Some("user") | Some("custom") => {
                    e.content = parse_content(m.get("content"), None);
                }
                Some("assistant") => {
                    e.content = parse_content(m.get("content"), None);
                }
                Some("toolResult") => {
                    e.tool_name = m.get("toolName").and_then(|x| x.as_str()).map(|s| s.to_string());
                    e.tool_call_id = m
                        .get("toolCallId")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                    e.is_error = m.get("isError").and_then(|x| x.as_bool());
                    e.content = parse_content(m.get("content"), None);
                }
                Some("bashExecution") => {
                    let cmd = m.get("command").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let out = m
                        .get("output")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let code = m.get("exitCode").and_then(|x| x.as_i64());
                    let truncated = m.get("truncated").and_then(|x| x.as_bool()).unwrap_or(false);
                    e.content.push(ContentBlock::Bash {
                        command: cmd,
                        output: out,
                        exit_code: code,
                        truncated,
                    });
                }
                _ => {}
            }
        }
        "model_change" => {
            e.model = v.get("modelId").and_then(|x| x.as_str()).map(|s| s.to_string());
            e.name = v.get("provider").and_then(|x| x.as_str()).map(|s| s.to_string());
        }
        "thinking_level_change" => {
            e.name = v.get("thinkingLevel").and_then(|x| x.as_str()).map(|s| s.to_string());
        }
        "compaction" => {
            e.summary = v.get("summary").and_then(|x| x.as_str()).map(|s| s.to_string());
            e.name = v
                .get("tokensBefore")
                .and_then(|x| x.as_u64())
                .map(|n| n.to_string());
        }
        "branch_summary" => {
            e.summary = v.get("summary").and_then(|x| x.as_str()).map(|s| s.to_string());
        }
        "custom_message" => {
            e.name = v.get("customType").and_then(|x| x.as_str()).map(|s| s.to_string());
            e.summary = v
                .get("content")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            e.content = parse_content(v.get("content"), None);
        }
        "session_info" => {
            e.name = v.get("name").and_then(|x| x.as_str()).map(|s| s.to_string());
        }
        "label" => {
            e.label = v.get("label").and_then(|x| x.as_str()).map(|s| s.to_string());
            e.name = v.get("targetId").and_then(|x| x.as_str()).map(|s| s.to_string());
        }
        _ => {}
    }
    Some(e)
}

fn parse_content(content: Option<&Value>, _ctx: Option<&str>) -> Vec<ContentBlock> {    let mut out = Vec::new();
    let content = match content {
        Some(c) => c,
        None => return out,
    };
    match content {
        Value::String(s) => {
            if !s.is_empty() {
                out.push(ContentBlock::Text { text: s.clone() });
            }
        }
        Value::Array(arr) => {
            for c in arr {
                let t = c.get("type").and_then(|x| x.as_str()).unwrap_or("");
                match t {
                    "text" => {
                        if let Some(s) = c.get("text").and_then(|x| x.as_str()) {
                            out.push(ContentBlock::Text { text: s.to_string() });
                        }
                    }
                    "thinking" => {
                        if let Some(s) = c.get("thinking").and_then(|x| x.as_str()) {
                            out.push(ContentBlock::Thinking { thinking: s.to_string() });
                        }
                    }
                    "toolCall" => {
                        let id = c.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let name = c.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let args = c.get("arguments").map(|a| a.to_string()).unwrap_or_default();
                        out.push(ContentBlock::ToolCall {
                            id,
                            name,
                            arguments: args,
                        });
                    }
                    "image" => {
                        let mime = c
                            .get("mimeType")
                            .and_then(|x| x.as_str())
                            .unwrap_or("image/png")
                            .to_string();
                        out.push(ContentBlock::Image { mime_type: mime });
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso() {
        assert_eq!(parse_iso_ts("2026-08-10T02:43:05.649Z"), Some(1786329785));
        assert_eq!(parse_iso_ts("1785814651241"), Some(1785814651));
        assert_eq!(parse_iso_ts(""), None);
    }

    #[test]
    fn test_list_and_detail() {
        let projects = list_projects();
        assert!(!projects.is_empty(), "expected sessions on this machine");
        let mut saw_main = false;
        let mut saw_sub = false;
        // one project is enough for a smoke test
        if let Some(p) = projects.first() {
            let sessions = list_sessions(&p.key);
            assert!(!sessions.is_empty());
            for s in &sessions {
                if s.is_subagent {
                    saw_sub = true;
                    assert!(s.task_id.is_some(), "subagent must carry a task id");
                } else {
                    saw_main = true;
                }
            }
            // detail parse on the newest main session
            if let Some(main) = sessions.iter().find(|s| !s.is_subagent) {
                let d = session_detail(&main.path).expect("parse detail");
                assert!(!d.active.is_empty());
                assert_eq!(d.id, main.id);
                assert!(d.stats.message_count >= 1);
                let mut has_user = false;
                for &i in &d.active {
                    if d.entries[i].role.as_deref() == Some("user") {
                        has_user = true;
                    }
                }
                assert!(has_user, "active branch should contain a user message");
            }
        }
        assert!(saw_main && saw_sub, "expected both main and subagent sessions");
    }

    #[test]
    fn test_subagent_parent_link() {
        // Text-based linkage: most subagents resolve to a parent main session.
        let projects = list_projects();
        let mut subs = 0usize;
        let mut linked = 0usize;
        for p in projects {
            let sessions = list_sessions(&p.key);
            for s in sessions.iter().filter(|s| s.is_subagent) {
                subs += 1;
                if s.parent_session_path.is_some() {
                    linked += 1;
                }
            }
        }
        assert!(subs > 0);
        // observed ~74% direct match; require most to resolve
        assert!(linked * 100 / subs >= 50, "linkage rate too low: {linked}/{subs}");
    }

    #[test]
    fn test_real_subagent_file_detected() {
        // Real pi session files spawned by the subagent extension (normal
        // filenames) must be flagged is_subagent; mirror + real pair for the
        // same uuid must be deduped to a single entry.
        let projects = list_projects();
        let mut found_real_sub = false;
        for p in projects {
            let sessions = list_sessions(&p.key);
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for s in &sessions {
                *counts.entry(s.id.as_str()).or_insert(0) += 1;
                if s.is_subagent && !s.path.contains("subagent-task-") {
                    found_real_sub = true;
                }
            }
            for (id, n) in counts {
                assert!(n == 1, "uuid {id} appears {n} times (dedupe failed)");
            }
        }
        assert!(found_real_sub, "expected a real-file subagent to be detected");
    }

}
