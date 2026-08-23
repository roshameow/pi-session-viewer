//! Remote pi agents over SSH (e.g. a mac-mini running daily SPC jobs).
//!
//! The desktop keeps a LOCAL cache of a remote host's `~/.pi/agent` tree:
//!   ~/.pi/remote/<host>/agent/...
//! mirrored from the host via `rsync` (sessions, agent-logs, runtime
//! registry) plus live snapshots (ps output, rmux pane list) captured at
//! sync time. All existing sessions.rs readers keep working unchanged —
//! `pi_agent_dir()` returns the cache dir while a remote host is selected.
//! Attach / open-in-terminal run `ssh -t <host> ...` in a local terminal.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const REMOTE_BASE: &str = ".pi/remote";

static CURRENT_HOST: OnceLock<Mutex<Option<String>>> = OnceLock::new();

pub fn current_host() -> Option<String> {
    CURRENT_HOST
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .clone()
}

/// Switch the desktop's agent source. None = local machine.
pub fn set_current_host(host: Option<String>) {
    *CURRENT_HOST.get_or_init(|| Mutex::new(None)).lock().unwrap() = host;
}

/// Home dir of the current host's agent cache (remote) or the real ~/.pi.
pub fn agent_root() -> PathBuf {
    match current_host() {
        Some(h) => remote_agent_dir(&h),
        None => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".pi").join("agent")
        }
    }
}

/// Cache dir for a remote host's agent tree: ~/.pi/remote/<host>/agent
pub fn remote_agent_dir(host: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(REMOTE_BASE)
        .join(host)
        .join("agent")
}

/// Config file listing remote hosts (plain JSON array of ssh aliases).
pub fn remote_hosts_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".pi-session-viewer.json")
}

/// Read the configured remote hosts (ssh aliases from ~/.ssh/config).
pub fn list_remote_hosts() -> Vec<String> {
    let path = remote_hosts_path();
    let Ok(data) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    match serde_json::from_str::<serde_json::Value>(&data) {
        Ok(v) => match v.get("remoteHosts") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            _ => vec![],
        },
        Err(_) => vec![],
    }
}

/// ssh 连接复用参数:ControlMaster 持久连接,后续 ssh/rsync 秒级复用
/// (Tailscale relay 每次握手 1-2s,复用后 ~50ms)。
fn ssh_master_args() -> Vec<String> {
    vec![
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        "ControlPersist=600".into(),
        "-o".into(),
        "ControlPath=/tmp/pi-remote-%r@%h:%p".into(),
    ]
}

/// Run a command on the remote host over ssh. Returns stdout.
pub fn ssh_run(host: &str, cmd: &str) -> Result<String, String> {
    let mut args = ssh_master_args();
    args.push(host.to_string());
    args.push(cmd.to_string());
    let out = std::process::Command::new("ssh")
        .args(&args)
        .output()
        .map_err(|e| format!("ssh failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ssh {}: {}",
            host,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Sync the remote host's agent tree (sessions, agent-logs, runtime
/// registry) plus live snapshots into the local cache. Must run before
/// switching the desktop's source to this host.
pub fn sync_remote(host: &str) -> Result<(), String> {
    let dst = remote_agent_dir(host);
    let dst_str = dst.to_string_lossy().into_owned();
    if !dst.exists() {
        std::fs::create_dir_all(&dst).map_err(|e| format!("cache mkdir: {e}"))?;
    }
    // 1) agent tree via rsync over ssh (exclude the cache itself, models,
    //    auth: desktop only needs sessions/agent-logs/runtime for browsing).
    let mut rargs: Vec<String> = vec!["-az".into(), "--delete".into()];
    // rsync 走同样的 ControlMaster 连接
    let master = ssh_master_args();
    rargs.push("-e".into());
    rargs.push(format!("ssh {}", master.join(" ")));
    for x in [
        "--exclude",
        "models.json",
        "--exclude",
        "auth.json",
        "--exclude",
        "mcp.json",
        "--exclude",
        "settings.json",
        "--exclude",
        "npm/",
    ] {
        rargs.push(x.into());
    }
    rargs.push(format!("{host}:.pi/agent/"));
    rargs.push(dst_str.clone());
    let out = std::process::Command::new("rsync")
        .args(&rargs)
        .output()
        .map_err(|e| format!("rsync failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "rsync {}: {}",
            host,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // 2) live snapshots for runtime chips (ps + rmux panes), single ssh round
    //    ps format: `pid tty etime command` (matches sessions.rs ps_lines)
    let snap = ssh_run(
        host,
        "echo '---PS---'; ps -eo pid=,tty=,etime=,command= | grep -E '[p]i([ ]|$)' || true; echo '---RMUX---'; rmux list-panes -a -F '#{session_name}:#{window_name}.#{pane_index} #{pane_pid} #{pane_dead} #{@pi_session}' 2>/dev/null || true",
    )?;
    let (ps_part, rmux_part) = snap
        .split_once("---RMUX---")
        .map(|(p, r)| (p.trim_start_matches("---PS---\n").to_string(), r.to_string()))
        .unwrap_or((String::new(), String::new()));
    std::fs::write(dst.join("ps_snapshot.txt"), ps_part).map_err(|e| format!("ps snapshot: {e}"))?;
    std::fs::write(dst.join("rmux_snapshot.txt"), rmux_part).map_err(|e| format!("rmux snapshot: {e}"))?;
    Ok(())
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
