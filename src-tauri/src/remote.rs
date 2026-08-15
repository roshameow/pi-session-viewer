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
