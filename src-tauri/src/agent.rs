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

    let mut child = Command::new(&pi_bin)
        .arg("--session")
        .arg(&session_path)
        .arg("--mode")
        .arg("json")
        .arg(&message)
        .env("PATH", crate::sessions::full_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to launch pi: {e}"))?;

    mark_running(&session_path);
    let stdout = child.stdout.take().expect("stdout piped");
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
        unmark_running(&session_path);
        let _ = on_event.send(serde_json::json!({"type":"process_exit"}));
    });

    Ok(())
}

/// Abort a running conversation for a session.
#[tauri::command]
pub fn abort_message(
    state: tauri::State<'_, Arc<AgentState>>,
    session_path: String,
) -> Result<(), String> {
    if let Some(mut c) = state.children.lock().unwrap().remove(&session_path) {
        let _ = c.kill();
        let _ = c.wait();
    }
    unmark_running(&session_path);
    Ok(())
}
