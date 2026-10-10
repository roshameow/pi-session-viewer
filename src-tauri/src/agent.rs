//! Live conversation: spawn `pi --session <file> --mode json <prompt>` and
//! stream every JSON event line back to the frontend over a Tauri Channel.
//! pi itself persists the new messages into the same JSONL file, so the on-disk
//! session stays the single source of truth.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::sessions::{mark_running, unmark_running};

#[derive(Default)]
pub struct AgentState {
    pub children: Mutex<HashMap<String, Child>>,
}

/// Stream one event line to the frontend. `delta` is the raw JSON from pi.
fn emit(channel: &tauri::ipc::Channel<Value>, value: Value) {
    let _ = channel.send(value);
}

fn pi_json_command(pi_bin: &str, cwd: &str, session_path: &str, message: &str) -> Command {
    let mut command = Command::new(pi_bin);
    command.current_dir(cwd)
        .arg("--session").arg(session_path)
        .arg("--mode").arg("json").arg(message)
        .env("PATH", crate::sessions::full_path())
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

// Keep draining even when the frontend drops its Channel; owned pipes must
// not backpressure the child. Return a join handle for deterministic closeout.
fn drain_diagnostics<R, F>(reader: R, on_line: F) -> thread::JoinHandle<()>
where R: BufRead + Send + 'static, F: Fn(String) + Send + 'static {
    thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) { on_line(line); }
    })
}

/// Send a message to a session. Spawns a fresh `pi` process (resumes the file),
/// streams events, and returns once the process exits.
#[tauri::command]
pub fn send_message(
    state: tauri::State<'_, Arc<AgentState>>,
    session_path: String,
    message: String,
    on_event: tauri::ipc::Channel<Value>,
) -> Result<(), String> {
    let pi_bin = crate::sessions::resolve_pi_bin().ok_or("pi executable not found")?;

    if state.children.lock().unwrap().contains_key(&session_path) {
        return Err("A task is already running for this session; wait or abort it first".into());
    }

    // Current Pi restores runtime/resource cwd from the --session header.
    // Also align process cwd (bootstrap and relative CLI paths) explicitly;
    // do not depend on a particular CLI version's cwd reconstruction.
    let cwd = crate::sessions::session_header(&session_path)?.cwd;
    let mut child = pi_json_command(&pi_bin, &cwd, &session_path, &message)
        .spawn()
        .map_err(|e| format!("Failed to launch pi: {e}"))?;

    mark_running(&session_path);
    let stdout = child.stdout.take().expect("stdout piped");
    // Native MCP startup/extension diagnostics use stderr. Drain it in parallel
    // so a full pipe cannot hang an otherwise completed JSON invocation.
    let stderr = child.stderr.take().expect("stderr piped");
    let diagnostic_channel = on_event.clone();
    let diagnostics = drain_diagnostics(BufReader::new(stderr), move |line| {
        emit(&diagnostic_channel, serde_json::json!({"type":"diagnostic", "line":line}));
    });
    state
        .children
        .lock()
        .unwrap()
        .insert(session_path.clone(), child);

    let state = state.inner().clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => emit(&on_event, v),
                Err(_) => {
                    emit(&on_event, serde_json::json!({"type":"raw","line":line}));
                }
            }
        }
        // process exit: re-take child from state, reap it
        if let Some(mut c) = state.children.lock().unwrap().remove(&session_path) {
            let _ = c.wait();
        }
        let _ = diagnostics.join();
        unmark_running(&session_path);
        let _ = on_event.send(serde_json::json!({"type":"process_exit"}));
    });

    Ok(())
}

/// Abort a running conversation for a session.
#[tauri::command]
pub async fn abort_message(
    state: tauri::State<'_, Arc<AgentState>>,
    session_path: String,
) -> Result<(), String> {
    let child = state.children.lock().unwrap().remove(&session_path);
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(mut c) = child {
            let _ = c.kill();
            let _ = c.wait();
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    unmark_running(&session_path);
    Ok(())
}

#[cfg(test)]
mod adaptation_tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn fake_cli_uses_project_cwd_and_drains_stderr_before_exit() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::time::Duration;
        let cwd = std::env::temp_dir().join(format!("viewer-fake-cli-{}", std::process::id()));
        std::fs::create_dir_all(cwd.join(".pi")).unwrap();
        std::fs::write(cwd.join(".pi/mcp.json"), "{\"fixtureMarker\":\"offline-project\"}").unwrap();
        let canonical_cwd = std::fs::canonicalize(&cwd).unwrap();
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/fixtures/fake-pi-cli.py");
        let mut child = pi_json_command(fixture, cwd.to_str().unwrap(), "/mock/session.jsonl", "mock prompt")
            .spawn().expect("launch only the offline fake CLI fixture");
        let bytes = Arc::new(AtomicUsize::new(0));
        let count = bytes.clone();
        let diagnostics = drain_diagnostics(BufReader::new(child.stderr.take().unwrap()), move |line| {
            count.fetch_add(line.len(), Ordering::Relaxed);
        });
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let records: Vec<Value> = BufReader::new(stdout).lines().map_while(Result::ok)
                .map(|line| serde_json::from_str(&line).unwrap()).collect();
            let _ = tx.send(records);
        });
        // Bounded local fixture test, not an external-state watcher/poll loop.
        let result = rx.recv_timeout(Duration::from_secs(5));
        if result.is_err() { let _ = child.kill(); }
        let status = child.wait().unwrap();
        reader.join().unwrap();
        diagnostics.join().unwrap();
        std::fs::remove_dir_all(&cwd).unwrap();
        let records = result.expect("stderr drain must prevent pipe-buffer deadlock");
        assert!(status.success());
        assert_eq!(records[0]["cwd"], canonical_cwd.to_string_lossy().as_ref());
        assert_eq!(records[0]["marker"], "offline-project");
        assert_eq!(records[0]["argv"], serde_json::json!([
            "--session", "/mock/session.jsonl", "--mode", "json", "mock prompt"]));
        assert_eq!(records[1]["type"], "agent_settled");
        assert!(bytes.load(Ordering::Relaxed) > 1024 * 1024);
    }

    #[test]
    fn cli_keeps_native_builtins_and_project_discovery_cwd() {
        let command = pi_json_command("mock-pi", "/mock/project", "/mock/session.jsonl", "hello");
        assert_eq!(command.get_current_dir(), Some(std::path::Path::new("/mock/project")));
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(args, ["--session", "/mock/session.jsonl", "--mode", "json", "hello"]);
        // No model overrides, --no-extensions, --mcp-config or restricted --tools.
        // Command inspection only: never execute mock-pi.
    }
}
