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

/// Open the session in a new terminal window running `pi --session <file>`.
/// macOS: AppleScript -> Terminal (or iTerm2 if installed).
#[tauri::command]
fn open_in_terminal(session_path: String) -> Result<(), String> {
    let bin = sessions::resolve_pi_bin().ok_or("pi executable not found")?;
    let cwd = sessions::session_detail(&session_path)
        .map(|d| d.cwd)
        .unwrap_or_default();
    let cmd = format!(
        "cd {} && {} --session {}",
        shell_quote(&cwd),
        shell_quote(&bin),
        shell_quote(&session_path)
    );
    #[cfg(target_os = "macos")]
    {
        // prefer iTerm2 if installed, otherwise Terminal — open a NEW TAB in
        // the frontmost window instead of spawning a whole new window.
        let iterm = std::path::Path::new(
            "/Applications/iTerm.app/Contents/MacOS/iTerm2",
        )
        .is_file();
        let script = if iterm {
            format!(
                r#"tell application "iTerm"
  tell current window
    create tab with default profile command "{cmd}"
  end tell
end tell"#,
                cmd = apple_escape(&cmd)
            )
        } else {
            format!(
                r#"tell application "Terminal"
  do script "{cmd}" in front window
end tell"#,
                cmd = apple_escape(&cmd)
            )
        };
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
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("x-terminal-emulator")
            .args(["-e", "sh", "-c", &cmd])
            .spawn()
            .map_err(|e| format!("Failed to open terminal: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "cmd", "/k"])
            .arg(&cmd)
            .spawn()
            .map_err(|e| format!("Failed to open terminal: {e}"))?;
    }
    Ok(())
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
