mod agent;
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
    let bin = sessions::resolve_pi_bin().ok_or("未找到 pi 可执行文件")?;
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .map_err(|e| format!("{e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
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
            agent::send_message,
            agent::abort_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running pi-session-viewer");
}
