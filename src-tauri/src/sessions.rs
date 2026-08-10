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

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub key: String,          // encoded dir name
    pub cwd: String,          // real cwd from a session header (best effort)
    pub session_count: usize,
    pub subagent_count: usize,
    pub updated_at: i64,      // latest file mtime (secs)
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
    pub message_count: usize,
    pub running: bool,
    pub size: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub token_count: u64,
    pub message_count: usize,
    pub model: Option<String>,
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
    pub stats: Stats,
    pub entries: Vec<Entry>,
    pub active: Vec<usize>, // indices of the active branch, root -> leaf
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

fn decode_dir_name(dir: &str) -> String {
    // "--Users-wenliu--" -> "/Users/wenliu" (best effort; header cwd is authoritative)
    let inner = dir.trim_start_matches('-').trim_end_matches('-');
    if inner.is_empty() {
        return String::new();
    }
    format!("/{}", inner.replace('-', "/"))
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
            let mut cwd = decode_dir_name(&name);
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
            if count == 0 {
                continue;
            }
            // best-effort real cwd from newest session header
            if let Ok(fd) = fs::read_dir(&path) {
                let mut best: Option<(i64, String)> = None;
                for f in fd.flatten() {
                    let p = f.path();
                    let mt = f
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    if let Some((bm, _)) = &best {
                        if mt <= *bm {
                            continue;
                        }
                    }
                    if let Some(line) = first_line(&p) {
                        if let Ok(v) = serde_json::from_str::<Value>(&line) {
                            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                                best = Some((mt, c.to_string()));
                            }
                        }
                    }
                }
                if let Some((_, c)) = best {
                    cwd = c;
                }
            }
            out.push(Project {
                key: name,
                cwd,
                session_count: count,
                subagent_count: sub_count,
                updated_at: updated,
            });
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

fn first_line(p: &Path) -> Option<String> {
    fs::read(p)
        .ok()
        .and_then(|bytes| {
            let s = String::from_utf8_lossy(&bytes);
            s.lines().next().map(|l| l.to_string())
        })
}

pub fn list_sessions(project_key: &str) -> Vec<SessionMeta> {
    let root = sessions_dir();
    let dir = root.join(project_key);
    let mut out = Vec::new();
    let running = running_set().lock().map(|s| s.clone()).unwrap_or_default();

    if let Ok(fd) = fs::read_dir(&dir) {
        for f in fd.flatten() {
            let path = f.path();
            let fname = f.file_name().to_string_lossy().to_string();
            if !fname.ends_with(".jsonl") {
                continue;
            }
            if let Some(m) = parse_meta(&path, &fname, &running) {
                out.push(m);
            }
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

fn parse_meta(path: &Path, fname: &str, running: &HashSet<String>) -> Option<SessionMeta> {
    let is_sub = fname.contains("subagent-task-");
    let (id, cwd, created_iso, name, first_msg, model) = scan_head(path);
    if id.is_empty() {
        // not a valid pi session file; skip
        return None;
    }
    let md = fmetadata(path);
    let mtime = md.as_ref().map(|m| m.mtime).unwrap_or(0);
    let size = md.as_ref().map(|m| m.size).unwrap_or(0);
    let last_msg = tail_preview(path, 160);
    let spath = path.to_string_lossy().to_string();
    let parent = if is_sub { Some(id.clone()) } else { None };
    let task_id = if is_sub { task_id_from_filename(fname) } else { None };
    Some(SessionMeta {
        path: spath.clone(),
        id,
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
        parent_session_id: parent,
        message_count: 0,
        running: running.contains(&spath),
        size,
    })
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

    let data = match fs::read(path) {
        Ok(d) => d,
        Err(_) => return (id, cwd, created, name, first_msg, model),
    };
    let text = String::from_utf8_lossy(&data);
    for (i, line) in text.lines().enumerate() {
        if i > 150 {
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
            "model_change" => {
                if model.is_none() {
                    model = v.get("modelId").and_then(|x| x.as_str()).map(|s| s.to_string());
                }
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
    let data = fs::read(path).map_err(|e| format!("读取会话文件失败: {e}"))?;
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
        // accumulate stats
        if let Some(m) = &entry.model {
            if model.is_none() {
                model = Some(m.clone());
            }
        }
        if t == "message" {
            msg_count += 1;
            if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                if let Some(tot) = u.get("totalTokens").and_then(|x| x.as_u64()) {
                    tokens += tot;
                }
                if let Some(c) = u.get("cost").and_then(|c| c.get("total")).and_then(|x| x.as_f64()) {
                    cost_total += c;
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

    let stats = Stats {
        token_count: tokens,
        message_count: msg_count,
        model,
        cost_total,
    };
    Ok(SessionDetail {
        id: header_id,
        cwd,
        created_iso: created,
        path: path.to_string(),
        stats,
        entries,
        active,
    })
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
        for p in projects {
            let sessions = list_sessions(&p.key);
            assert!(!sessions.is_empty());
            for s in &sessions {
                if s.is_subagent {
                    saw_sub = true;
                    // mirror header id == parent session uuid
                    assert!(s.parent_session_id.is_some());
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
            break; // one project is enough for a smoke test
        }
        assert!(saw_main && saw_sub, "expected both main and subagent sessions");
    }

    #[test]
    fn test_subagent_parent_link() {
        // Every subagent whose header id matches a main session must link.
        let projects = list_projects();
        let mut subs = 0usize;
        let mut linked = 0usize;
        for p in projects {
            let sessions = list_sessions(&p.key);
            let mains: Vec<&SessionMeta> = sessions.iter().filter(|s| !s.is_subagent).collect();
            for s in sessions.iter().filter(|s| s.is_subagent) {
                subs += 1;
                if mains.iter().any(|m| m.id == s.parent_session_id.as_deref().unwrap_or("")) {
                    linked += 1;
                }
            }
        }
        assert!(subs > 0);
        // we observed 26/27 on the real machine; require most to link
        assert!(linked * 100 / subs >= 90, "linkage rate too low: {linked}/{subs}");
    }
}
