//! Remote browsing uses a host-qualified local cache. Selection is cache-first;
//! historical sync is incremental rsync, but still enumerates all allowed history.
//! Captured runtime status is not live telemetry or an atomic history snapshot.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

pub const REMOTE_BASE: &str = ".pi/remote";

#[derive(Default)]
struct SourceSelection { host: Option<String>, epoch: u64 }
static CURRENT_HOST: OnceLock<Mutex<SourceSelection>> = OnceLock::new();
thread_local! {
    static READ_SOURCE: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}
fn source_state() -> &'static Mutex<SourceSelection> {
    CURRENT_HOST.get_or_init(|| Mutex::new(SourceSelection::default()))
}

pub fn current_host() -> Option<String> {
    READ_SOURCE.with(|source| source.borrow().clone())
        .unwrap_or_else(|| source_state().lock().unwrap().host.clone())
}

/// Capture one source for a complete blocking traversal, including helper reads.
/// Nested calls restore their previous context even during unwinding.
pub fn with_current_source<T>(read: impl FnOnce() -> T) -> T {
    struct Restore(Option<Option<String>>);
    impl Drop for Restore {
        fn drop(&mut self) { READ_SOURCE.with(|s| *s.borrow_mut() = self.0.take()); }
    }
    let captured = current_host();
    let old = READ_SOURCE.with(|s| s.replace(Some(captured)));
    let _restore = Restore(old);
    read()
}

pub fn set_current_host(host: Option<String>) {
    let mut source = source_state().lock().unwrap();
    source.epoch = source.epoch.wrapping_add(1);
    source.host = host;
}

pub fn begin_source_selection() -> u64 {
    let mut source = source_state().lock().unwrap();
    source.epoch = source.epoch.wrapping_add(1);
    source.epoch
}
fn commit_source(host: Option<String>, epoch: u64) -> Result<(), String> {
    let mut source = source_state().lock().unwrap();
    if source.epoch != epoch { return Err("Source selection superseded".into()); }
    source.host = host;
    Ok(())
}

fn select_source_with(
    host: Option<String>, epoch: u64,
    usable: impl FnOnce(&str) -> bool,
    sync: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(h) = &host {
        validate_host(h)?;
        if !usable(h) { sync(h)?; }
    }
    commit_source(host, epoch)
}
pub fn select_source(host: Option<String>, epoch: u64) -> Result<(), String> {
    select_source_with(host, epoch, |h| usable_cache(&remote_agent_dir(h)), sync_remote)
}

pub fn agent_root() -> PathBuf {
    match current_host() {
        Some(h) => remote_agent_dir(&h),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
            .join(".pi").join("agent"),
    }
}
pub fn remote_agent_dir(host: &str) -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
        .join(REMOTE_BASE).join(host).join("agent")
}
pub fn remote_hosts_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
        .join(".pi-session-viewer.json")
}
pub fn list_remote_hosts() -> Vec<String> {
    let Ok(data) = std::fs::read_to_string(remote_hosts_path()) else { return vec![] };
    serde_json::from_str::<serde_json::Value>(&data).ok()
        .and_then(|v| v.get("remoteHosts").and_then(|x| x.as_array()).cloned())
        .unwrap_or_default().iter().filter_map(|v| v.as_str())
        .filter(|h| validate_host(h).is_ok()).map(str::to_string).collect()
}

// SSH aliases must not escape the cache namespace or become command options.
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() || host.len() > 200 || !host.as_bytes()[0].is_ascii_alphanumeric()
        || !host.chars().all(|c| c.is_ascii_alphanumeric() || "._-@".contains(c))
        || host.matches('@').count() > 1 || host.ends_with('@') {
        return Err("Invalid remote host alias".into());
    }
    Ok(())
}
fn ssh_master_args() -> Vec<String> {
    ["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", "-o", "ControlMaster=auto",
     "-o", "ControlPersist=600", "-o", "ControlPath=/tmp/pi-remote-%r@%h:%p"]
        .into_iter().map(str::to_string).collect()
}

/// Deliberately never return remote stderr/argv (which may contain credentials).
fn run_command(name: &str, args: &[String]) -> Result<String, String> {
    let seconds = if name == "rsync" { 30 * 60 } else { 30 };
    run_command_bounded(name, args, std::time::Duration::from_secs(seconds))
}
fn run_command_bounded(name: &str, args: &[String], timeout: std::time::Duration) -> Result<String, String> {
    use std::process::Stdio;
    let mut child = std::process::Command::new(name).args(args)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|_| "Remote command could not be started".to_string())?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut data = Vec::new(); let mut buf = [0u8; 8192]; let mut overflow = false;
        let result = loop {
            match stdout.read(&mut buf) {
                Ok(0) => break if overflow { Err("Remote command output exceeded limit".to_string()) } else { Ok(data) },
                Ok(n) => {
                    if data.len() + n <= 8 * 1024 * 1024 { data.extend_from_slice(&buf[..n]); }
                    else { overflow = true; }
                }
                Err(_) => break Err("Remote command output unavailable".into()),
            }
        };
        let _ = send.send(result);
    });
    // Drain, but never retain/export stderr or command arguments.
    std::thread::spawn(move || { let _ = std::io::copy(&mut stderr, &mut std::io::sink()); });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill(); let _ = child.wait();
                return Err("Remote command timed out or became unavailable".into());
            }
        }
    };
    if !status.success() { return Err("Remote command failed".into()); }
    let output = receive.recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| "Remote command output did not close".to_string())??;
    String::from_utf8(output).map_err(|_| "Remote command returned invalid UTF-8".into())
}
pub fn ssh_run(host: &str, cmd: &str) -> Result<String, String> {
    validate_host(host)?;
    let mut args = ssh_master_args();
    args.push(host.into()); args.push(cmd.into());
    run_command("ssh", &args)
}

const MANIFEST: &str = ".viewer-last-sync.json";
const INITIALIZING: &str = ".viewer-initializing";

/// Legacy populated caches remain browsable, but have unknown sync timestamp.
/// A failed new initial sync is never adopted merely because rsync left files.
fn usable_cache(root: &Path) -> bool {
    if root.join(INITIALIZING).exists() { return false; }
    let sessions = root.join("sessions");
    if !std::fs::symlink_metadata(&sessions).is_ok_and(|m| m.file_type().is_dir()) { return false; }
    let Ok(projects) = std::fs::read_dir(sessions) else { return false };
    for project in projects.flatten() {
        let name = project.file_name();
        if !name.to_string_lossy().starts_with("--")
            || !project.file_type().is_ok_and(|t| t.is_dir()) { continue; }
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "jsonl")
                || std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) { continue; }
            let Ok(file) = std::fs::File::open(path) else { continue };
            let mut line = String::new();
            if std::io::BufReader::new(file.take(16 * 1024)).read_line(&mut line).is_err() { continue; }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if v["type"] == "session" && v["id"].as_str().is_some_and(|id| !id.is_empty()) {
                return true;
            }
        }
    }
    false
}
fn last_success(root: &Path) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(root.join(MANIFEST)).ok()?)
        .ok()?.get("lastSuccessAt")?.as_u64()
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSyncStatus {
    pub host: String,
    pub phase: String,
    pub last_success_at: Option<u64>,
    pub error: Option<String>,
    pub usable_cache: bool,
}
struct Completion { result: Mutex<Option<Result<(), String>>>, changed: Condvar }
struct HostState { active: Option<Arc<Completion>>, status: RemoteSyncStatus }
struct HostEntry { state: Mutex<HostState> }
static SYNC_HOSTS: OnceLock<Mutex<HashMap<PathBuf, Arc<HostEntry>>>> = OnceLock::new();
fn host_entry(host: &str, root: &Path) -> Arc<HostEntry> {
    SYNC_HOSTS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap()
        .entry(root.to_path_buf()).or_insert_with(|| Arc::new(HostEntry {
            state: Mutex::new(HostState { active: None, status: RemoteSyncStatus {
                host: host.into(), phase: "idle".into(), last_success_at: last_success(root),
                error: None, usable_cache: usable_cache(root),
            }}),
        })).clone()
}
pub fn sync_status(host: &str) -> Result<RemoteSyncStatus, String> {
    validate_host(host)?;
    let root = remote_agent_dir(host);
    let entry = host_entry(host, &root);
    let status = entry.state.lock().unwrap().status.clone();
    Ok(status)
}
fn phase(entry: &HostEntry, value: &str) {
    entry.state.lock().unwrap().status.phase = value.into();
}

/// Include managed leaf files, never arbitrary resource support trees or secrets.
/// Excluded receiver files are protected; DO NOT add --delete-excluded.
fn rsync_args(host: &str, dst: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-az", "--no-links", "--delete", "--delay-updates", "--timeout=60",
        "--filter=P /ps_snapshot.txt", "--filter=P /rmux_snapshot.txt",
        "--filter=P /.viewer-last-sync.json", "--filter=P /.viewer-initializing",
        "--exclude=.git/", "--exclude=packages/", "--exclude=artifacts/", "--exclude=npm/",
        "--include=/sessions/", "--include=/sessions/--*/", "--include=/sessions/--*/*.jsonl",
        "--include=/agent-logs/", "--include=/agent-logs/task-*.jsonl",
        "--include=/runtime/", "--include=/runtime/*.jsonl",
        "--include=/agents/", "--include=/agents/*.md",
        "--include=/skills/", "--include=/skills/*/", "--include=/skills/*/SKILL.md",
        "--exclude=*"] .into_iter().map(str::to_string).collect();
    args.push("-e".into()); args.push(format!("ssh {}", ssh_master_args().join(" ")));
    args.push(format!("{host}:.pi/agent/")); args.push(dst.to_string_lossy().into_owned());
    args
}
const SNAPSHOT_COMMAND: &str = "echo '---TIME---'; date +%s; echo '---PS---'; ps -eo pid=,tty=,etime=,command= | grep -E '[p]i([ -]|$)' || true; echo '---RMUX---'; rmux list-panes -a -F '#{session_name}:#{window_name}.#{pane_index} #{pane_pid} #{pane_dead} #{@pi_session}' 2>/dev/null";

fn split_snapshot(snap: &str) -> Result<(&str, &str), String> {
    let (ps, rmux) = snap.split_once("---RMUX---\n").ok_or("Invalid runtime snapshot")?;
    let mut lines = ps.lines();
    if lines.next() != Some("---TIME---")
        || lines.next().and_then(|s| s.parse::<u64>().ok()).is_none_or(|n| n == 0)
        || lines.next() != Some("---PS---") {
        return Err("Invalid runtime snapshot".into());
    }
    // Do not echo invalid captured process strings into UI errors.
    let mut pids = std::collections::HashSet::new();
    for line in lines.filter(|s| !s.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 4 || !fields[0].parse::<u32>().is_ok_and(|p| p > 0 && pids.insert(p))
            || !valid_etime(fields[2]) {
            return Err("Invalid process snapshot".into());
        }
    }
    for line in rmux.lines().filter(|s| !s.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 3 || !fields[0].rsplit_once('.').is_some_and(|(w, i)| w.contains(':') && i.parse::<u32>().is_ok())
            || !fields[1].parse::<u32>().is_ok_and(|p| p > 0) || !["0", "1"].contains(&fields[2]) {
            return Err("Invalid pane snapshot".into());
        }
    }
    Ok((ps, rmux))
}
fn valid_etime(value: &str) -> bool {
    let time = if let Some((days, rest)) = value.split_once('-') {
        if days.parse::<u64>().is_err() { return false; }
        rest
    } else { value };
    let parts: Vec<_> = time.split(':').collect();
    (1..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.parse::<u64>().is_ok())
}
fn write_atomic(root: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    // One writer per root. Each file is atomically replaced, not the pair/tree.
    let temp = root.join(format!("{name}.viewer-next"));
    std::fs::write(&temp, data).map_err(|_| "Cache write failed".to_string())?;
    std::fs::rename(&temp, root.join(name)).map_err(|_| "Cache publish failed".to_string())
}
fn perform_sync(
    host: &str, root: &Path, entry: &HostEntry,
    run: &mut impl FnMut(&str, &[String]) -> Result<String, String>,
) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|_| "Cache directory unavailable".to_string())?;
    if !entry.state.lock().unwrap().status.usable_cache {
        std::fs::write(root.join(INITIALIZING), b"initial sync incomplete\n")
            .map_err(|_| "Cache initialization failed".to_string())?;
    }
    phase(entry, "syncing-history");
    run("rsync", &rsync_args(host, root)).map_err(|_| "History sync failed; cache may be partially updated".to_string())?;
    phase(entry, "capturing-snapshots");
    let mut args = ssh_master_args(); args.push(host.into()); args.push(SNAPSHOT_COMMAND.into());
    let snap = run("ssh", &args).map_err(|_| "Runtime capture failed; prior snapshots retained".to_string())?;
    let (ps, rmux) = split_snapshot(&snap)?;
    write_atomic(root, "ps_snapshot.txt", ps.as_bytes())?;
    write_atomic(root, "rmux_snapshot.txt", rmux.as_bytes())?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Local clock unavailable".to_string())?.as_secs();
    write_atomic(root, MANIFEST, serde_json::json!({"host":host,"lastSuccessAt":now}).to_string().as_bytes())?;
    if root.join(INITIALIZING).exists() {
        std::fs::remove_file(root.join(INITIALIZING)).map_err(|_| "Cache readiness publish failed".to_string())?;
    }
    Ok(())
}
fn sync_remote_at(
    host: &str, root: &Path,
    mut run: impl FnMut(&str, &[String]) -> Result<String, String>,
) -> Result<(), String> {
    validate_host(host)?;
    let entry = host_entry(host, root);
    let (completion, owner) = {
        let mut state = entry.state.lock().unwrap();
        if let Some(active) = &state.active { (active.clone(), false) }
        else {
            let active = Arc::new(Completion { result: Mutex::new(None), changed: Condvar::new() });
            state.active = Some(active.clone());
            state.status.phase = "syncing-history".into(); state.status.error = None;
            (active, true)
        }
    };
    if !owner {
        let mut result = completion.result.lock().unwrap();
        while result.is_none() { result = completion.changed.wait(result).unwrap(); }
        return result.as_ref().unwrap().clone();
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| perform_sync(host, root, &entry, &mut run)))
        .unwrap_or_else(|_| Err("Remote sync interrupted".into()));
    {
        let mut state = entry.state.lock().unwrap();
        state.status.phase = if result.is_ok() { "ready" } else { "error" }.into();
        state.status.error = result.clone().err();
        if result.is_ok() { state.status.last_success_at = last_success(root); }
        state.status.usable_cache = usable_cache(root);
        *completion.result.lock().unwrap() = Some(result.clone());
        state.active = None;
    }
    completion.changed.notify_all();
    result
}

/// Historical sync remains incremental but enumerates all allowed histories.
/// Selection of usable cached sessions never needs to call this first.
pub fn sync_remote(host: &str) -> Result<(), String> {
    sync_remote_at(host, &remote_agent_dir(host), run_command)
}

/// Build the local-terminal command that attaches to a session on the host:
///   ssh -t <host> 'cd <cwd> && rmux attach -t <session>'
pub fn remote_attach_cmd(host: &str, cwd: &str, target: &str) -> String {
    let inner = format!(
        "cd {} && rmux attach -t {}",
        shell_quote(cwd),
        shell_quote(target)
    );
    format!("ssh -t {} {}", shell_quote(host), shell_quote(&inner))
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

// ---------------------------------------------------------------------------
// Session transfer (local -> remote)
// ---------------------------------------------------------------------------

/// Read the first line of a session jsonl as JSON (header carries id + cwd).
fn read_header(p: &std::path::Path) -> Result<serde_json::Value, String> {
    use std::io::BufRead;
    let f = std::fs::File::open(p).map_err(|e| format!("open {}: {e}", p.display()))?;
    let first = std::io::BufReader::new(f)
        .lines()
        .next()
        .ok_or("empty session file")?
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&first).map_err(|e| format!("parse header {}: {e}", p.display()))
}

/// Transfer a local session (plus its subagent sessions) to a remote host so a
/// long-running task can keep running there. Steps:
/// 1. collect the main session file + subagent files whose header id matches
/// 2. rewrite the local project root -> remote cwd inside each jsonl
/// 3. upload into <host>:~/.pi/agent/sessions/<remote-slug>/
/// 4. start `pi --session <file> [prompt]` in a detached rmux session
/// Returns the rmux session name.
pub fn transfer_session_to_remote(
    host: &str,
    session_path: &str,
    remote_cwd: &str,
    prompt: &str,
) -> Result<String, String> {
    if remote_cwd.trim().is_empty() {
        return Err("remote cwd is required".into());
    }
    let p = PathBuf::from(session_path);
    if !p.exists() {
        return Err(format!("session file not found: {session_path}"));
    }
    let dir = p.parent().ok_or("no parent dir")?;

    let header = read_header(&p)?;
    let id = header
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("session header missing id")?
        .to_string();
    let local_root = header
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // subagent sessions of THIS parent live in the same dir; their header id
    // equals the parent's uuid (pi-subagent-durable linkage).
    let mut files: Vec<PathBuf> = vec![p.clone()];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".jsonl") || !name.contains("subagent-task-") {
                continue;
            }
            if let Ok(h) = read_header(&e.path()) {
                if h.get("id").and_then(|v| v.as_str()) == Some(id.as_str()) {
                    files.push(e.path());
                }
            }
        }
    }

    // rewrite local root -> remote cwd, stage to tmp, remember filenames
    let tmp = std::env::temp_dir().join(format!(
        "piv-transfer-{}",
        id.chars().take(12).collect::<String>()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("tmp mkdir: {e}"))?;
    for f in &files {
        let content =
            std::fs::read_to_string(f).map_err(|e| format!("read {}: {e}", f.display()))?;
        let out = if local_root.is_empty() || local_root == remote_cwd {
            content
        } else {
            content.replace(&local_root, remote_cwd)
        };
        let fname = f.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(tmp.join(&fname), out).map_err(|e| format!("write {fname}: {e}"))?;
    }

    // upload via scp reusing the ControlMaster connection
    let slug = crate::sessions::encode_dir_name(remote_cwd);
    let dest_dir = format!(".pi/agent/sessions/{slug}/");
    ssh_run(host, &format!("mkdir -p {}", shell_quote(&format!("~/{dest_dir}"))))?;
    let scp_args = [
        "-o", "ConnectTimeout=10", "-o", "BatchMode=yes", "-o", "ControlMaster=auto",
        "-o", "ControlPersist=600", "-o", "ControlPath=/tmp/pi-remote-%r@%h:%p",
        // NOTE: glob must be expanded locally by scp's shell — pass via sh -c
    ];
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "scp {} {} {}:{}",
            scp_args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" "),
            shell_quote(&tmp.join("*.jsonl").to_string_lossy()),
            shell_quote(host),
            shell_quote(&format!("~/{dest_dir}"))
        ))
        .output()
        .map_err(|e| format!("scp failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "scp {}: {}",
            host,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    // start pi inside a detached rmux session (same naming rule as local:
    // pi-<encoded-cwd>-<id12>, one rmux session per pi session)
    let encoded = remote_cwd.trim_start_matches('/').replace('/', "-");
    let clean: String = encoded
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let id12: String = id.chars().take(12).collect();
    let sess_name = format!("pi-{clean}-{id12}");
    let main_fname = p.file_name().unwrap().to_string_lossy().into_owned();
    let sess_file = format!("$HOME/{dest_dir}{main_fname}");
    // If the session has subagents, remind the agent to re-spawn them: the
    // durable extension persists each subagent's progress in the session, so
    // re-issuing the same tasks resumes from where they left off (no redo).
    let subagent_count = files.len().saturating_sub(1);
    let resume_note = if subagent_count > 0 {
        format!(
            "\n\n[系统提示] 本会话已从另一台机器转移到当前主机继续运行。你的 {} 个 subagent 进程没有随之迁移。请根据会话中记录的任务清单和持久化进度,逐个重新发起未完成的 subagent 任务(durable 扩展会自动从断点续跑,已完成的部分不要重复执行)。",
            subagent_count
        )
    } else {
        String::new()
    };
    let effective_prompt = match prompt.trim() {
        "" => resume_note.trim().to_string(),
        msg => format!("{}{}", msg, resume_note),
    };
    let inner = if effective_prompt.is_empty() {
        format!(
            "cd {} && pi --session {}",
            shell_quote(remote_cwd),
            shell_quote(&sess_file)
        )
    } else {
        format!(
            "cd {} && pi --session {} {}",
            shell_quote(remote_cwd),
            shell_quote(&sess_file),
            shell_quote(&effective_prompt)
        )
    };
    let start_cmd = format!(
        "rmux new-session -d -s {} -c {} {}",
        shell_quote(&sess_name),
        shell_quote(remote_cwd),
        shell_quote(&inner)
    );
    ssh_run(host, &start_cmd)?;

    let _ = std::fs::remove_dir_all(&tmp);
    Ok(sess_name)
}

#[cfg(test)]
#[path = "remote_connect_tests.rs"]
mod remote_connect_tests;
