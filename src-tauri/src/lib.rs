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
            sessions::list_running,
            agent::send_message,
            agent::abort_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running pi-session-viewer");
}
