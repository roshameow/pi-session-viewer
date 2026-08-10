mod agent;
mod config;
mod sessions;

use std::path::Path;
use std::sync::Arc;

#[tauri::command]
fn list_projects() -> Vec<sessions::Project> {
    sessions::list_projects()
}

#[tauri::command]
fn list_sessions(project_key: String) -> Vec<sessions::SessionMeta> {
    sessions::list_sessions(&project_key)
}

#[tauri::command]
fn session_detail(path: String) -> Result<sessions::SessionDetail, String> {
    sessions::session_detail(&path)
}

#[tauri::command]
fn pi_bin_path() -> Option<String> {
    sessions::resolve_pi_bin()
}

#[tauri::command]
fn pi_sessions_dir() -> String {
    sessions::sessions_dir().to_string_lossy().to_string()
}

#[tauri::command]
fn pi_version() -> Result<String, String> {
    let bin = sessions::resolve_pi_bin().ok_or("pi executable not found")?;
    let out = std::process::Command::new(bin)
        .arg("--version")
        .env("PATH", sessions::full_path())
        .output()
        .map_err(|e| format!("{e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[tauri::command]
fn export_session_html(session_path: String) -> Result<String, String> {
    let bin = sessions::resolve_pi_bin().ok_or("pi executable not found")?;
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let downloads = std::path::Path::new(&home).join("Downloads");
    let out_dir = if downloads.is_dir() {
        downloads
    } else {
        std::path::PathBuf::from(&home)
    };
    let id = sessions::session_id(&session_path).unwrap_or_else(|| "session".into());
    let short: String = id.chars().take(8).collect();
    let out = out_dir.join(format!("pi-session-{short}.html"));

    let result = std::process::Command::new(&bin)
        .arg("--export")
        .arg(&session_path)
        .arg(&out)
        .env("PATH", sessions::full_path())
        .output()
        .map_err(|e| format!("Failed to run pi --export: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "pi --export failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    // open the exported HTML in the default browser
    let _ = open_file(&out);
    Ok(out.to_string_lossy().to_string())
}

fn open_file(p: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(p).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(p).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(["/c", "start", ""]).arg(p).spawn()?;
    }
    Ok(())
}

/// Delete a session file (plus its mirror/task-log when it is a subagent).
/// Uses the `trash` CLI when available (like pi /resume Ctrl+D), otherwise
/// falls back to a permanent delete.
#[tauri::command]
fn delete_session(path: String) -> Result<(), String> {
    let id = sessions::session_id(&path).unwrap_or_default();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if std::path::Path::new(&path).is_file() {
        files.push(std::path::PathBuf::from(&path));
    }
    // related files sharing the same session uuid (mirror) + agent-log task file
    let (_, task_by_uuid, _) = sessions::subagent_index();
    if !id.is_empty() {
        if let Ok(rd) = std::fs::read_dir(sessions::sessions_dir()) {
            for e in rd.flatten() {
                let dir = e.path();
                if !dir.is_dir() {
                    continue;
                }
                if let Ok(fd) = std::fs::read_dir(&dir) {
                    for f in fd.flatten() {
                        let fp = f.path();
                        if !fp.is_file() {
                            continue;
                        }
                        if sessions::session_id(&fp.to_string_lossy()).as_deref() == Some(id.as_str())
                            && fp.to_string_lossy() != path
                        {
                            files.push(fp);
                        }
                    }
                }
            }
        }
        if let Some(task_id) = task_by_uuid.get(&id) {
            let log = sessions::pi_agent_dir()
                .join("agent-logs")
                .join(format!("task-{task_id}.jsonl"));
            if log.is_file() {
                files.push(log);
            }
        }
    }
    for f in files {
        let status = std::process::Command::new("trash")
            .arg(&f)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // trash CLI unavailable/failed -> permanent delete
                std::fs::remove_file(&f)
                    .map_err(|e| format!("Failed to delete {}: {e}", f.display()))?;
            }
        }
    }
    Ok(())
}

/// Unique rmux session name derived from the FULL project path (avoids
/// collisions between same-named projects in different directories):
/// /Users/a/Code/python/quantnight -> pi-Users-a-Code-python-quantnight
fn rmux_session_name(cwd: &str) -> String {
    let encoded = cwd.trim_start_matches('/').replace('/', "-");
    let clean: String = encoded
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    format!("pi-{}", if clean.is_empty() { "default" } else { &clean })
}

fn rmux_bin() -> Option<String> {
    if let Ok(b) = std::env::var("RMUX_BIN") {
        if !b.is_empty() {
            return Some(b);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    for cand in [
        "/opt/homebrew/bin/rmux",
        "/usr/local/bin/rmux",
        &format!("{home}/.local/bin/rmux"),
    ] {
        if std::path::Path::new(cand).is_file() {
            return Some(cand.to_string());
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = std::path::Path::new(dir).join("rmux");
            if p.is_file() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn rmux_available() -> bool {
    rmux_bin().is_some()
}

/// Run a Terminal (or iTerm) window executing `cmd` via AppleScript (new window).
fn open_terminal_window(cmd: &str) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = std::process::Command::new("sh").args(["-c", cmd]).spawn()
            .map_err(|e| format!("Failed to open terminal: {e}"))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let app = if std::path::Path::new("/Applications/iTerm.app/Contents/MacOS/iTerm2").is_file() {
            "iTerm"
        } else {
            "Terminal"
        };
    let script = format!(
        r#"tell application "{app}" to do script "{cmd}""#,
        app = app,
        cmd = apple_escape(cmd)
    );
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("Failed to open terminal: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "osascript failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
        Ok(())
    }
}

/// Ensure a pi session runs in an rmux window named `s<id8>` of the project
/// session. Creates the session/window when missing; reuses when present.
/// Returns the rmux session name. None if rmux is unavailable.
fn ensure_rmux_window(
    cwd: &str,
    id: &str,
    cmd: &str,
) -> Result<Option<String>, String> {
    if !rmux_available() || cwd.is_empty() {
        return Ok(None);
    }
    let rmux = rmux_bin().unwrap();
    let sess = rmux_session_name(cwd);
    let win = format!("s{}", &id.chars().take(8).collect::<String>());
    let session_exists = std::process::Command::new(&rmux)
        .args(["has-session", "-t", &sess]).env("PATH", sessions::full_path())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    // does the window already exist AND is its pane process actually alive?
    // (reuse only live windows; a stale window from an old pi gets recreated)
    let pane_pid = |r: &str, s: &str, w: &str| -> Option<u32> {
        std::process::Command::new(r)
            .args(["list-panes", "-t", &format!("{s}:{w}"), "-F", "#{pane_pid}"])
            .env("PATH", sessions::full_path())
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok())
    };
    let pid_alive = |pid: u32| -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let win_alive = if session_exists {
        pane_pid(&rmux, &sess, &win).map(|p| pid_alive(p)).unwrap_or(false)
    } else {
        false
    };
    if !win_alive {
        // stale window (old pi) -> remove it so the next block starts fresh
        if session_exists {
            let _ = std::process::Command::new(&rmux)
                .args(["kill-window", "-t", &format!("{sess}:{win}")])
                .env("PATH", sessions::full_path())
                .output();
        }
        let session_exists = std::process::Command::new(&rmux)
            .args(["has-session", "-t", &sess])
            .env("PATH", sessions::full_path())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let args: Vec<String> = if session_exists {
            vec![
                "new-window".into(),
                "-d".into(),
                "-t".into(),
                sess.clone(),
                "-n".into(),
                win,
                cmd.to_string(),
            ]
        } else {
            vec![
                "new-session".into(),
                "-d".into(),
                "-s".into(),
                sess.clone(),
                "-n".into(),
                win,
                cmd.to_string(),
            ]
        };
        let out = std::process::Command::new(&rmux)
            .args(&args)
            .env("PATH", sessions::full_path())
            .output()
            .map_err(|e| format!("Failed to start rmux session: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "rmux failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(Some(sess))
}

/// Open the session in a terminal. With rmux installed, the pi process runs in
/// a persistent rmux session (`pi-<encoded-cwd>`), so the tab can be closed
/// anytime and the agent keeps running; reattach via `pim` or the Attach button.
/// Without rmux, falls back to a plain `pi --session` window.
#[tauri::command]
fn open_in_terminal(session_path: String) -> Result<(), String> {
    let bin = sessions::resolve_pi_bin().ok_or("pi executable not found")?;
    let cwd = sessions::session_detail(&session_path)
        .map(|d| d.cwd)
        .unwrap_or_default();
    let id = sessions::session_id(&session_path).unwrap_or_default();
    let cmd = format!(
        "cd {} && {} --session {}",
        shell_quote(&cwd),
        shell_quote(&bin),
        shell_quote(&session_path)
    );
    if let Some(sess) = ensure_rmux_window(&cwd, &id, &cmd)? {
        return open_terminal_window(&format!("rmux attach -t {}", shell_quote(&sess)));
    }
    open_terminal_window(&cmd)
}

/// Attach to the rmux session a session belongs to (pi-agents for subagents,
/// pi-<project> for main sessions).
#[tauri::command]
fn attach_session(session_path: String) -> Result<String, String> {
    let rmux = rmux_bin().ok_or("rmux is not installed")?;
    let cwd = sessions::session_detail(&session_path)
        .map(|d| d.cwd)
        .unwrap_or_default();
    let id = sessions::session_id(&session_path).unwrap_or_default();
    let is_sub = sessions::is_subagent_uuid(&id);
    let sess = if is_sub {
        // subagents live in pi-agents; attach directly (may not exist)
        "pi-agents".to_string()
    } else {
        // main sessions: ensure the rmux window (create if not running), then attach
        let cmd = format!(
            "cd {} && {} --session {}",
            shell_quote(&cwd),
            shell_quote(&sessions::resolve_pi_bin().unwrap_or_else(|| "pi".into())),
            shell_quote(&session_path)
        );
        match ensure_rmux_window(&cwd, &id, &cmd)? {
            Some(s) => s,
            None => "pi-agents".to_string(),
        }
    };
    // don't spawn another terminal if someone is already attached
    let clients = std::process::Command::new(&rmux)
        .args(["list-clients", "-t", &sess]).env("PATH", sessions::full_path())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if !clients.is_empty() {
        return Ok(format!("already attached (session {sess}) — no new window"));
    }
    open_terminal_window(&format!("rmux attach -t {}", shell_quote(&sess)))?;
    Ok(format!("attached to {sess}"))
}

/// shell single-quote a string (paths with spaces/special chars)
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// escape a string for embedding inside a double-quoted AppleScript string
fn apple_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[tauri::command]
fn file_exists(path: String) -> bool {
    Path::new(&path).is_file()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(agent::AgentState::default()))
        .invoke_handler(tauri::generate_handler![
            list_projects,
            list_sessions,
            session_detail,
            pi_bin_path,
            pi_sessions_dir,
            pi_version,
            file_exists,
            export_session_html,
            open_in_terminal,
            attach_session,
            delete_session,
            sessions::list_running,
            sessions::session_status,
            config::list_config,
            agent::send_message,
            agent::abort_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running pi-session-viewer");
}
