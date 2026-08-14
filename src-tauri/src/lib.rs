mod agent;
mod config;
mod sessions;

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
        if let Some(task_ids) = task_by_uuid.get(&id) {
            if let Some(task_id) = task_ids.first() {
                let log = sessions::pi_agent_dir()
                    .join("agent-logs")
                    .join(format!("task-{task_id}.jsonl"));
                if log.is_file() {
                    files.push(log);
                }
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
fn rmux_session_name(cwd: &str, id: &str) -> String {
    let encoded = cwd.trim_start_matches('/').replace('/', "-");
    let clean: String = encoded
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    // one rmux session PER pi session: attaching one window used to re-target
    // the whole project session (every client jumped to the last-attached
    // window). the session name now carries id12 (uuid chars 0..12, includes
    // the 8th-char dash) so each pi has its own session and attach is isolated.
    let id12: String = id.chars().take(12).collect();
    format!(
        "pi-{}-{}",
        if clean.is_empty() { "default" } else { &clean },
        if id12.is_empty() { "s" } else { &id12 }
    )
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
fn open_terminal_window(cmd: &str) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        std::process::Command::new("sh").args(["-c", cmd]).spawn()
            .map_err(|e| format!("Failed to open terminal: {e}"))?;
        return Ok("spawned".to_string());
    }
    #[cfg(target_os = "macos")]
    {
        let app = if std::path::Path::new("/Applications/iTerm.app/Contents/MacOS/iTerm2").is_file() {
            "iTerm"
        } else {
            "Terminal"
        };
        let run = |script: &str| -> Result<(), String> {
            let out = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output()
                .map_err(|e| format!("Failed to open terminal: {e}"))?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            Ok(())
        };
        let esc = apple_escape(cmd);
        if app == "iTerm" {
            let script = format!(
                r#"tell application "iTerm2"
  activate
  if (count of windows) = 0 then create window with default profile
  tell current window
    create tab with default profile
    tell current session to write text "{esc}"
  end tell
end tell"#
            );
            run(&script)?;
            return Ok("opened in a new tab".to_string());
        }
        // Terminal.app: Cmd+T opens a new tab (handled by the app itself, so it
        // is safe even when the front tab runs the pi TUI in raw mode); then the
        // command runs in that fresh idle tab. Needs Accessibility for the
        // keystroke; falls back to a plain new window when unavailable.
        //
        // IMPORTANT: activate + keystroke + do script must run as SEPARATE
        // osascript invocations — in one script the keystroke lands before
        // Terminal is frontmost and silently does nothing (observed: it fell
        // back to a new window every time).
        let run_term = |script: &str| -> Result<(), String> {
            let out = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output()
                .map_err(|e| format!("osascript failed: {e}"))?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            Ok(())
        };
        let count_of = |expr: &str| -> u32 {
            std::process::Command::new("osascript")
                .arg("-e")
                .arg(format!("tell application \"Terminal\" to return {expr}"))
                .output()
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok())
                .unwrap_or(0)
        };
        // if Terminal has no window at all, there is nothing to tab into
        let has_window = count_of("count of windows") > 0;
        if !has_window {
            // no existing window -> open one (a tab is impossible)
            run_term(&format!(r#"tell application "Terminal" to do script "{esc}""#))?;
            return Ok("opened a new window (Terminal had no window to tab into)".to_string());
        }
        let _ = run_term(r#"tell application "Terminal" to activate"#);
        std::thread::sleep(std::time::Duration::from_millis(600));
        // The stable tab-open-helper binary posts Cmd+T (Terminal creates a
        // fresh idle tab which becomes the front tab). It is never rebuilt, so
        // its Accessibility grant survives app rebuilds. Trust its success —
        // re-verifying tab counts here raced and double-opened a window.
        // runtime: <exe>/../Resources/resources/tab-open-helper; dev: source dir
        let helper = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|p| p.join("../Resources/resources/tab-open-helper"))
            })
            .filter(|p| p.is_file())
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("resources")
                    .join("tab-open-helper")
            });
        let run_after_tab = || -> Result<String, String> {
            // give Terminal a moment to finish creating the tab, then run the
            // command in the (new, idle) front tab
            std::thread::sleep(std::time::Duration::from_millis(900));
            run_term(&format!(r#"tell application "Terminal" to do script "{esc}" in front window"#))?;
            Ok("opened in a new tab".to_string())
        };
        if helper.is_file() {
            match std::process::Command::new(&helper).output() {
                Ok(o) if o.status.success() => return run_after_tab(),
                Ok(o) => {
                    let why = format!(
                        "{} {}",
                        String::from_utf8_lossy(&o.stdout).trim(),
                        String::from_utf8_lossy(&o.stderr).trim()
                    )
                    .trim()
                    .to_string();
                    // helper denied (e.g. 1002) -> plain new window, explain why
                    run_term(&format!(r#"tell application "Terminal" to do script "{esc}""#))?;
                    return Ok(format!(
                        "opened a new window (tab failed: {why}) — enable Accessibility for pi-session-viewer (System Settings → Privacy & Security → Accessibility)"
                    ));
                }
                Err(e) => {
                    run_term(&format!(r#"tell application "Terminal" to do script "{esc}""#))?;
                    return Ok(format!(
                        "opened a new window (tab failed: helper error {e}) — enable Accessibility for pi-session-viewer"
                    ));
                }
            }
        }
        // no helper bundled: fall back to in-process osascript keystroke
        let before = count_of("count of tabs of front window");
        for _ in 0..2 {
            let _ = run_term(r#"tell application "System Events" to keystroke "t" using command down"#);
            std::thread::sleep(std::time::Duration::from_millis(600));
            if count_of("count of tabs of front window") > before {
                return run_after_tab();
            }
        }
        run_term(&format!(r#"tell application "Terminal" to do script "{esc}""#))?;
        Ok("opened a new window (could not create a tab — enable Accessibility for pi-session-viewer)".to_string())
    }
}

/// Ensure a pi session runs in an rmux window named `s<id8>` of the project
/// session. Creates the session/window when missing; reuses when present.
/// Returns the rmux session name. None if rmux is unavailable.
fn ensure_rmux_window(
    cwd: &str,
    id: &str,
    session_path: &str,
    cmd: &str,
) -> Result<Option<String>, String> {
    if !rmux_available() || cwd.is_empty() {
        return Ok(None);
    }
    let rmux = rmux_bin().unwrap();
    // one session per pi session: sess = pi-<encoded-cwd>-<id12>, window = "main"
    let sess = rmux_session_name(cwd, id);
    let win = "main";
    let run = |args: &[&str]| -> std::process::Output {
        std::process::Command::new(&rmux)
            .args(args)
            .env("PATH", sessions::full_path())
            .output()
            .unwrap_or_else(|_| std::process::Output { status: std::process::ExitStatus::default(), stdout: Vec::new(), stderr: Vec::new() })
    };

    let session_exists = run(&["has-session", "-t", &sess]).status.success();
    let win_alive = if session_exists {
        let pid_out = run(&["list-panes", "-t", &format!("{sess}:{win}"), "-F", "#{pane_pid}"]);
        let pid = String::from_utf8_lossy(&pid_out.stdout).trim().parse::<u32>().ok();
        match pid {
            Some(p) => std::process::Command::new("kill")
                .args(["-0", &p.to_string()])
                .env("PATH", sessions::full_path())
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
            None => false,
        }
    } else {
        false
    };
    // reuse only when this session actually runs the requested pi session;
    // otherwise kill and recreate (id collisions or stale sessions)
    let session_runs_other = if session_exists && win_alive {
        let map = sessions::rmux_runtime_map();
        let target = format!("{sess}:{win}");
        match map
            .iter()
            .find(|(_, v)| v.target.starts_with(&target))
            .map(|(k, _)| k)
        {
            Some(p) => p != session_path,
            None => false, // unmapped: freshly created or unknown — reuse
        }
    } else {
        false
    };
    if !win_alive || session_runs_other {
        let _ = run(&["kill-session", "-t", &sess]);
        let out = run(&[
            "new-session",
            "-d",
            "-s",
            &sess,
            "-n",
            win,
            cmd,
        ]);
        if !out.status.success() {
            return Err(format!(
                "rmux failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        // record which pi session this rmux session runs (window-level user
        // option; -w is required — plain set-option scopes @ to the SESSION)
        let _ = run(&["set-option", "-w", "-t", &format!("{sess}:{win}"), "@pi_session", session_path]);
    }
    Ok(Some(format!("{sess}:{win}")))
}

/// Detach all terminals from an rmux session (from the app side). The session
/// keeps running; the attached terminal just drops back to its shell prompt.
#[tauri::command]
fn detach_from_rmux(session_path: String) -> Result<(), String> {
    let rmux = rmux_bin().ok_or("rmux is not installed")?;
    let map = sessions::rmux_runtime_map();
    let rt = map
        .get(&session_path)
        .ok_or("session is not running in an rmux window")?;
    if rt.dead {
        return Err("the rmux pane is dead (pi exited) — nothing to detach".into());
    }
    let sess = rt.target.split(':').next().unwrap_or("").to_string();
    if sess.is_empty() {
        return Err("invalid rmux target".into());
    }
    let out = std::process::Command::new(&rmux)
        .args(["detach-client", "-s", &sess])
        .env("PATH", sessions::full_path())
        .output()
        .map_err(|e| format!("failed to run rmux detach-client: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "rmux detach failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Kill (close) the rmux session running this pi session. The whole session
/// dies — any live pi process in it is terminated and the window is removed
/// (no remain-on-exit since the session itself is destroyed). Caller must
/// confirm: this is a hard stop, unlike Detach which keeps the pi running.
#[tauri::command]
fn kill_rmux_session(session_path: String) -> Result<(), String> {
    let rmux = rmux_bin().ok_or("rmux is not installed")?;
    let map = sessions::rmux_runtime_map();
    let rt = map
        .get(&session_path)
        .ok_or("session is not running in an rmux window")?;
    let sess = rt.target.split(':').next().unwrap_or("").to_string();
    if sess.is_empty() {
        return Err("invalid rmux target".into());
    }
    let out = std::process::Command::new(&rmux)
        .args(["kill-session", "-t", &sess])
        .env("PATH", sessions::full_path())
        .output()
        .map_err(|e| format!("failed to run rmux kill-session: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "rmux kill-session failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Open the session in a terminal. With rmux installed, the pi process runs in
/// a persistent rmux session (`pi-<encoded-cwd>`), so the tab can be closed
/// anytime and the agent keeps running; reattach via `pim` or the Attach button.
/// Without rmux, falls back to a plain `pi --session` window.
#[tauri::command]
fn open_in_terminal(session_path: String) -> Result<String, String> {
    // already alive in an rmux pane (incl. pim sessions with short names)?
    // attach to that pane's session — never spawn a second pi.
    if let Some(sess) = existing_rmux_session(&session_path) {
        return open_terminal_window(&format!("rmux attach -t {}", shell_quote(&sess)));
    }
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
    if let Some(sess) = ensure_rmux_window(&cwd, &id, &session_path, &cmd)? {
        return open_terminal_window(&format!("rmux attach -t {}", shell_quote(&sess)));
    }
    open_terminal_window(&cmd)
}


/// If a session already runs alive in an rmux pane, return the rmux SESSION
/// name from its mapped target (e.g. "pi-quantnight" — pim creates short
/// names the standard pi-<encoded-cwd> lookup can't find). Dead panes are
/// ignored so the caller can recreate.
fn existing_rmux_session(session_path: &str) -> Option<String> {
    let map = sessions::rmux_runtime_map();
    let rt = map.get(session_path)?;
    if rt.dead {
        return None;
    }
    // target = "session:window.pane" -> attach target "session:window" so the
    // user lands on THIS pane, not the session's active window (which may be
    // a different session's window)
    let t = rt
        .target
        .rsplit_once('.')
        .map(|(w, _)| w)
        .unwrap_or(&rt.target);
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
/// Attach to the rmux session a session belongs to (pi-agents for subagents,
/// pi-<project> for main sessions).
#[tauri::command]
fn attach_session(session_path: String) -> Result<String, String> {
    let cwd = sessions::session_detail(&session_path)
        .map(|d| d.cwd)
        .unwrap_or_default();
    let id = sessions::session_id(&session_path).unwrap_or_default();
    let is_sub = sessions::is_subagent_uuid(&id);
    let sess = if let Some(s) = existing_rmux_session(&session_path) {
        // already alive in an rmux pane (incl. pim short-name sessions)
        s
    } else if is_sub {
        // subagents live in pi-agents; attach directly (may not exist)
        "pi-agents".to_string()
    } else {
        // main sessions: refuse to spawn a duplicate pi when the session is
        // already alive in a terminal window (two pis appending the same
        // jsonl corrupts it). rmux-active sessions fall through to reuse.
        if sessions::session_has_live_terminal_pi(&session_path) {
            return Ok("already running in a terminal window — no duplicate created".to_string());
        }
        // main sessions: ensure the rmux window (create if not running), then attach
        let cmd = format!(
            "cd {} && {} --session {}",
            shell_quote(&cwd),
            shell_quote(&sessions::resolve_pi_bin().unwrap_or_else(|| "pi".into())),
            shell_quote(&session_path)
        );
        match ensure_rmux_window(&cwd, &id, &session_path, &cmd)? {
            Some(s) => s,
            None => "pi-agents".to_string(),
        }
    };
    // no per-session client guard: list-clients is session-scoped, so it would
    // block attaching a DIFFERENT window of the same rmux session (e.g.
    // 019fe979 and 019fee76-14ae live in one session with different windows).
    // tmux supports multiple clients; each attach targets its own window.
    let msg = open_terminal_window(&format!("rmux attach -t {}", shell_quote(&sess)))?;
    Ok(format!("attached to {sess} ({msg})"))
}

/// shell single-quote a string (paths with spaces/special chars)
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// escape a string for embedding inside a double-quoted AppleScript string
fn apple_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
            pi_version,
            export_session_html,
            open_in_terminal,
            attach_session,
            detach_from_rmux,
            kill_rmux_session,
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
