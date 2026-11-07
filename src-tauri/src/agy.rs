//! Antigravity (agy) session discovery + parsing adapter.
//! Sessions live in `~/.gemini/antigravity-cli/brain/<uuid>/.system_generated/logs/transcript.jsonl`.
//! Metadata is indexed in `~/.gemini/antigravity-cli/conversation_summaries.db`.

use crate::sessions::{ContentBlock, Entry, Project, SessionMeta};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn agy_dir() -> PathBuf {
    if let Ok(d) = std::env::var("AGY_CLI_HOME") {
        return PathBuf::from(d);
    }
    if let Ok(d) = std::env::var("ANTIGRAVITY_CLI_HOME") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".gemini").join("antigravity-cli")
}

pub fn db_path() -> PathBuf {
    agy_dir().join("conversation_summaries.db")
}

pub fn is_agy_available() -> bool {
    db_path().is_file()
}

pub fn is_agy_path(path: &str) -> bool {
    path.contains("antigravity-cli") || path.contains(".gemini")
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn resolve_agy_bin() -> Option<String> {
    if let Ok(b) = std::env::var("AGY_BIN") {
        if !b.is_empty() {
            return Some(b);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    for cand in [
        &format!("{home}/bin/agy"),
        &format!("{home}/.local/bin/agy"),
        "/opt/homebrew/bin/agy",
        "/usr/local/bin/agy",
        "/usr/bin/agy",
    ] {
        if Path::new(cand).is_file() {
            return Some(cand.to_string());
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = Path::new(dir).join("agy");
            if p.is_file() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

pub fn build_agy_tui_command(session_path: &str) -> Result<(String, String, String), String> {
    let bin = resolve_agy_bin().ok_or("agy executable not found")?;
    let id = conversation_id_from_path(session_path)
        .ok_or_else(|| format!("unable to extract agy conversation id from path: {session_path}"))?;
    let mut cwd = lookup_workspace_cwd(&id).unwrap_or_default();
    if cwd.is_empty() {
        cwd = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    }
    let cmd = format!(
        "cd {} && {} --dangerously-skip-permissions --conversation {}",
        shell_quote(&cwd),
        shell_quote(&bin),
        shell_quote(&id)
    );
    Ok((cwd, id, cmd))
}

pub fn conversation_id_from_path(path: &str) -> Option<String> {
    let p = Path::new(path);
    let comps: Vec<String> = p
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    for i in 0..comps.len() {
        if comps[i] == "brain" && i + 1 < comps.len() {
            let candidate = &comps[i + 1];
            if candidate.len() >= 32 {
                return Some(candidate.clone());
            }
        }
    }
    None
}

pub fn extract_workspace_path(uris_json: &str) -> Option<String> {
    let trimmed = uris_json.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return None;
    }
    let list: Vec<String> = serde_json::from_str(trimmed).ok()?;
    for uri in list {
        if let Some(stripped) = uri.strip_prefix("file://") {
            let clean = stripped.trim_end_matches('/');
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
        } else if uri.starts_with('/') {
            let clean = uri.trim_end_matches('/');
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
        }
    }
    None
}

pub fn lookup_workspace_cwd(conv_id: &str) -> Option<String> {
    let db = db_path();
    if !db.is_file() {
        return None;
    }
    let query = format!(
        "SELECT workspace_uris FROM conversation_summaries WHERE conversation_id = '{}' LIMIT 1;",
        conv_id.replace('\'', "''")
    );
    let out = Command::new("sqlite3")
        .arg("-json")
        .arg(&db)
        .arg(query)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let list: Vec<Value> = serde_json::from_slice(&out.stdout).ok()?;
    let uris = list.first()?.get("workspace_uris")?.as_str()?;
    extract_workspace_path(uris)
}

#[derive(serde::Deserialize, Default, Clone, Debug)]
pub struct AgySummary {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub step_count: usize,
    #[serde(default)]
    pub last_modified_time: String,
    #[serde(default)]
    pub workspace_uris: String,
    #[serde(default)]
    pub parent_conversation_id: String,
    #[serde(default)]
    pub nesting_depth: usize,
    #[serde(default, deserialize_with = "deserialize_idle_flag")]
    pub not_fully_idle: bool,
    #[serde(default)]
    pub agent_name: String,
}

fn deserialize_idle_flag<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;
    Ok(match v {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    })
}

pub fn query_all_summaries() -> Vec<AgySummary> {
    let db = db_path();
    if !db.is_file() {
        return Vec::new();
    }
    let out = Command::new("sqlite3")
        .arg("-json")
        .arg(&db)
        .arg("SELECT conversation_id, title, preview, step_count, last_modified_time, workspace_uris, parent_conversation_id, nesting_depth, not_fully_idle, agent_name FROM conversation_summaries ORDER BY last_modified_time DESC;")
        .output();
    match out {
        Ok(output) if output.status.success() => {
            serde_json::from_slice::<Vec<AgySummary>>(&output.stdout).unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

pub fn parse_agy_iso_epoch(s: &str) -> i64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let iso = s.replace(' ', "T");
    let iso_clean = if let Some(idx) = iso.find('+') {
        &iso[..idx]
    } else {
        &iso
    };
    let formatted = format!("{}Z", iso_clean.trim_end_matches('Z'));
    crate::sessions::parse_iso_ts(&formatted).unwrap_or(0)
}

pub fn list_agy_projects() -> Vec<Project> {
    let summaries = query_all_summaries();
    if summaries.is_empty() {
        return Vec::new();
    }

    let mut parent_workspaces: HashMap<String, String> = HashMap::new();
    for s in &summaries {
        if let Some(cwd) = extract_workspace_path(&s.workspace_uris) {
            parent_workspaces.insert(s.conversation_id.clone(), cwd);
        }
    }

    struct Group {
        cwd: String,
        session_count: usize,
        subagent_count: usize,
        running_count: usize,
        updated_at: i64,
    }

    let mut groups: HashMap<String, Group> = HashMap::new();

    for s in summaries {
        let cwd_opt = extract_workspace_path(&s.workspace_uris)
            .or_else(|| parent_workspaces.get(&s.parent_conversation_id).cloned());
        let cwd = match cwd_opt {
            Some(c) if !c.is_empty() => c,
            _ => continue,
        };

        let transcript_path = agy_dir()
            .join("brain")
            .join(&s.conversation_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");

        if !transcript_path.is_file() {
            continue;
        }

        let updated = parse_agy_iso_epoch(&s.last_modified_time);
        let g = groups.entry(cwd.clone()).or_insert_with(|| Group {
            cwd: cwd.clone(),
            session_count: 0,
            subagent_count: 0,
            running_count: 0,
            updated_at: 0,
        });

        if s.parent_conversation_id.is_empty() {
            g.session_count += 1;
        } else {
            g.subagent_count += 1;
        }
        if s.not_fully_idle {
            g.running_count += 1;
        }
        if updated > g.updated_at {
            g.updated_at = updated;
        }
    }

    groups
        .into_values()
        .map(|g| Project {
            key: crate::sessions::encode_dir_name(&g.cwd),
            cwd: g.cwd,
            session_count: g.session_count,
            subagent_count: g.subagent_count,
            updated_at: g.updated_at,
            running_count: g.running_count,
            rmux_count: 0,
            term_count: 0,
        })
        .collect()
}

pub fn list_agy_sessions(cwd: &str) -> Vec<SessionMeta> {
    let summaries = query_all_summaries();
    if summaries.is_empty() {
        return Vec::new();
    }

    let mut parent_workspaces: HashMap<String, String> = HashMap::new();
    for s in &summaries {
        if let Some(c) = extract_workspace_path(&s.workspace_uris) {
            parent_workspaces.insert(s.conversation_id.clone(), c);
        }
    }

    let norm_cwd = cwd.trim_end_matches('/');
    let mut out = Vec::new();

    for s in summaries {
        let scwd = extract_workspace_path(&s.workspace_uris)
            .or_else(|| parent_workspaces.get(&s.parent_conversation_id).cloned())
            .unwrap_or_default();

        if scwd.trim_end_matches('/') != norm_cwd {
            continue;
        }

        let transcript_path = agy_dir()
            .join("brain")
            .join(&s.conversation_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");

        let meta = match std::fs::metadata(&transcript_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_size = meta.len();
        let mtime_sec = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or_else(|| parse_agy_iso_epoch(&s.last_modified_time));

        let created_epoch = parse_agy_iso_epoch(&s.last_modified_time);
        let path_str = transcript_path.to_string_lossy().to_string();

        let is_subagent = !s.parent_conversation_id.is_empty();
        let parent_session_path = if is_subagent {
            Some(
                agy_dir()
                    .join("brain")
                    .join(&s.parent_conversation_id)
                    .join(".system_generated")
                    .join("logs")
                    .join("transcript.jsonl")
                    .to_string_lossy()
                    .to_string(),
            )
        } else {
            None
        };

        out.push(SessionMeta {
            path: path_str,
            id: s.conversation_id.clone(),
            cwd: norm_cwd.to_string(),
            name: if s.title.is_empty() {
                None
            } else {
                Some(s.title.clone())
            },
            first_message: if s.preview.is_empty() {
                None
            } else {
                Some(s.preview.clone())
            },
            last_message: None,
            created_iso: s.last_modified_time.clone(),
            created_at: created_epoch,
            updated_at: mtime_sec,
            model: Some("gemini".to_string()),
            is_subagent,
            task_id: if s.agent_name.is_empty() {
                None
            } else {
                Some(s.agent_name.clone())
            },
            parent_session_id: if is_subagent {
                Some(s.parent_conversation_id.clone())
            } else {
                None
            },
            parent_session_path,
            message_count: s.step_count,
            running: s.not_fully_idle,
            sleeping: false,
            interrupted: false,
            in_rmux: false,
            rmux_target: None,
            rmux_attached: false,
            rmux_pi_alive: None,
            rmux_dead: false,
            term_alive: false,
            size: file_size,
            engine: Some("agy".to_string()),
        });
    }

    out
}

pub fn clean_user_input(text: &str) -> String {
    if let Some(start_tag) = text.find("<USER_REQUEST>") {
        let after_start = &text[start_tag + "<USER_REQUEST>".len()..];
        if let Some(end_tag) = after_start.find("</USER_REQUEST>") {
            return after_start[..end_tag].trim().to_string();
        }
    }
    text.trim().to_string()
}

pub fn parse_agy_step(v: &Value) -> Option<Entry> {
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| v.get("step_index").map(|s| s.to_string()))?;

    let step_index = v.get("step_index").and_then(|x| x.as_u64());
    let parent_id = match step_index {
        Some(idx) if idx > 0 => Some((idx - 1).to_string()),
        _ => None,
    };

    let ts = v
        .get("created_at")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("timestamp").and_then(|x| x.as_str()))
        .map(str::to_string);

    let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("");

    match kind {
        "USER_INPUT" => {
            let content_text = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
            let cleaned = clean_user_input(content_text);
            let mut content = Vec::new();
            if !cleaned.is_empty() {
                content.push(ContentBlock::Text { text: cleaned });
            }
            Some(Entry {
                kind: "message".into(),
                id,
                parent_id,
                ts,
                role: Some("user".into()),
                content,
                model: None,
                tool_name: None,
                tool_call_id: None,
                is_error: None,
                summary: None,
                name: None,
                label: None,
                labeled: None,
                body_ref: None,
            })
        }
        "PLANNER_RESPONSE" => {
            let mut content = Vec::new();
            if let Some(thinking) = v.get("thinking").and_then(|x| x.as_str()) {
                if !thinking.trim().is_empty() {
                    content.push(ContentBlock::Thinking {
                        thinking: thinking.trim().to_string(),
                    });
                }
            }
            if let Some(tool_calls) = v.get("tool_calls").and_then(|x| x.as_array()) {
                for (i, tc) in tool_calls.iter().enumerate() {
                    let name = tc
                        .get("name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("tool")
                        .to_string();
                    let args = tc.get("args").map(|a| a.to_string()).unwrap_or_default();
                    let call_id = format!("call_{}_{}", id, i);
                    content.push(ContentBlock::ToolCall {
                        id: call_id,
                        name,
                        arguments: args,
                    });
                }
            }
            if let Some(resp) = v.get("content").and_then(|x| x.as_str()) {
                if !resp.trim().is_empty() {
                    content.push(ContentBlock::Text {
                        text: resp.to_string(),
                    });
                }
            }
            Some(Entry {
                kind: "message".into(),
                id,
                parent_id,
                ts,
                role: Some("assistant".into()),
                content,
                model: Some("gemini".into()),
                tool_name: None,
                tool_call_id: None,
                is_error: None,
                summary: None,
                name: None,
                label: None,
                labeled: None,
                body_ref: None,
            })
        }
        "GENERIC" => {
            let mut content = Vec::new();
            if let Some(text) = v.get("content").and_then(|x| x.as_str()) {
                if !text.is_empty() {
                    content.push(ContentBlock::Text {
                        text: text.to_string(),
                    });
                }
            }
            if let Some(media) = v.get("media").and_then(|x| x.as_array()) {
                for m in media {
                    let mime = m
                        .get("mime_type")
                        .and_then(|x| x.as_str())
                        .unwrap_or("image/png")
                        .to_string();
                    content.push(ContentBlock::Image { mime_type: mime });
                }
            }
            let is_error = v.get("status").and_then(|x| x.as_str()).map(|s| s == "ERROR");
            Some(Entry {
                kind: "message".into(),
                id,
                parent_id,
                ts,
                role: Some("toolResult".into()),
                content,
                model: None,
                tool_name: None,
                tool_call_id: None,
                is_error,
                summary: None,
                name: None,
                label: None,
                labeled: None,
                body_ref: None,
            })
        }
        "SYSTEM_MESSAGE" => {
            let content_text = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
            let mut content = Vec::new();
            if !content_text.is_empty() {
                content.push(ContentBlock::Text {
                    text: content_text.to_string(),
                });
            }
            Some(Entry {
                kind: "message".into(),
                id,
                parent_id,
                ts,
                role: Some("system".into()),
                content,
                model: None,
                tool_name: None,
                tool_call_id: None,
                is_error: None,
                summary: Some("System Notification".into()),
                name: None,
                label: None,
                labeled: None,
                body_ref: None,
            })
        }
        "ERROR_MESSAGE" => {
            let err = v
                .get("error")
                .and_then(|x| x.as_str())
                .or_else(|| v.get("content").and_then(|x| x.as_str()))
                .unwrap_or("Error");
            Some(Entry {
                kind: "message".into(),
                id,
                parent_id,
                ts,
                role: Some("system".into()),
                content: vec![ContentBlock::Text {
                    text: err.to_string(),
                }],
                model: None,
                tool_name: None,
                tool_call_id: None,
                is_error: Some(true),
                summary: Some("Error".into()),
                name: None,
                label: None,
                labeled: None,
                body_ref: None,
            })
        }
        _ => Some(Entry {
            kind: kind.to_string(),
            id,
            parent_id,
            ts,
            role: None,
            content: vec![],
            model: None,
            tool_name: None,
            tool_call_id: None,
            is_error: None,
            summary: None,
            name: None,
            label: None,
            labeled: None,
            body_ref: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_user_input() {
        let raw = "<USER_REQUEST>\n请仔细识别以下图片\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\ntime\n</ADDITIONAL_METADATA>";
        assert_eq!(clean_user_input(raw), "请仔细识别以下图片");

        let plain = "Hello world";
        assert_eq!(clean_user_input(plain), "Hello world");
    }

    #[test]
    fn test_extract_workspace_path() {
        let json = r#"["file:///Users/wenliu/Code/python/quantnight"]"#;
        assert_eq!(
            extract_workspace_path(json),
            Some("/Users/wenliu/Code/python/quantnight".to_string())
        );

        let multi = r#"["file:///tmp/other", "file:///Users/wenliu/Code"]"#;
        assert_eq!(
            extract_workspace_path(multi),
            Some("/tmp/other".to_string())
        );

        assert_eq!(extract_workspace_path("[]"), None);
        assert_eq!(extract_workspace_path(""), None);
    }

    #[test]
    fn test_conversation_id_from_path() {
        let path = "/Users/wenliu/.gemini/antigravity-cli/brain/23e37d0e-07f8-45ce-a40c-5b2f8c61aec6/.system_generated/logs/transcript.jsonl";
        assert_eq!(
            conversation_id_from_path(path),
            Some("23e37d0e-07f8-45ce-a40c-5b2f8c61aec6".to_string())
        );
    }

    #[test]
    fn test_parse_agy_steps() {
        let user_val = serde_json::json!({
            "step_index": 0,
            "source": "USER_EXPLICIT",
            "type": "USER_INPUT",
            "created_at": "2026-11-01T00:31:16Z",
            "content": "<USER_REQUEST>\nFix this bug\n</USER_REQUEST>"
        });
        let e0 = parse_agy_step(&user_val).unwrap();
        assert_eq!(e0.id, "0");
        assert_eq!(e0.parent_id, None);
        assert_eq!(e0.role.as_deref(), Some("user"));
        assert_eq!(e0.content.len(), 1);
        if let ContentBlock::Text { text } = &e0.content[0] {
            assert_eq!(text, "Fix this bug");
        } else {
            panic!("Expected text block");
        }

        let model_val = serde_json::json!({
            "step_index": 1,
            "source": "MODEL",
            "type": "PLANNER_RESPONSE",
            "created_at": "2026-11-01T00:31:17Z",
            "thinking": "Let's read files",
            "tool_calls": [
                { "name": "read_file", "args": { "Path": "test.txt" } }
            ],
            "content": "Looking at files."
        });
        let e1 = parse_agy_step(&model_val).unwrap();
        assert_eq!(e1.id, "1");
        assert_eq!(e1.parent_id.as_deref(), Some("0"));
        assert_eq!(e1.role.as_deref(), Some("assistant"));
        assert_eq!(e1.content.len(), 3); // Thinking + ToolCall + Text
        if let ContentBlock::ToolCall { id, name, .. } = &e1.content[1] {
            assert_eq!(id, "call_1_0");
            assert_eq!(name, "read_file");
        } else {
            panic!("Expected ToolCall block");
        }

        let generic_val = serde_json::json!({
            "step_index": 2,
            "source": "MODEL",
            "type": "GENERIC",
            "created_at": "2026-11-01T00:31:18Z",
            "content": "File contents here",
            "status": "DONE"
        });
        let e2 = parse_agy_step(&generic_val).unwrap();
        assert_eq!(e2.id, "2");
        assert_eq!(e2.parent_id.as_deref(), Some("1"));
        assert_eq!(e2.role.as_deref(), Some("toolResult"));
        assert_eq!(e2.is_error, Some(false));
    }

    #[test]
    fn test_real_or_synthetic_detail_page() {
        let sess_id = "23e37d0e-07f8-45ce-a40c-5b2f8c61aec6";
        let path = agy_dir()
            .join("brain")
            .join(sess_id)
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");

        if path.is_file() {
            let path_str = path.to_string_lossy().to_string();
            let detail = crate::sessions::detail::page(&path_str, Default::default())
                .expect("detail page should succeed");
            assert_eq!(detail.id, sess_id);
            assert!(!detail.entries.is_empty());
            assert!(detail.stats.token_count > 0);
            if let Some(page) = &detail.page {
                assert!(!page.tool_pairs.is_empty(), "Should have paired tool calls");
            }
        }
    }

    #[test]
    fn test_agy_tui_command_generation() {
        let fake_path = "/Users/test/.gemini/antigravity-cli/brain/c2ec9bfb-e49a-4d8e-842a-eb9bcbadba07/.system_generated/logs/transcript.jsonl";
        let res = build_agy_tui_command(fake_path);
        if resolve_agy_bin().is_some() {
            let (cwd, id, cmd) = res.expect("command should build when agy bin is present");
            assert_eq!(id, "c2ec9bfb-e49a-4d8e-842a-eb9bcbadba07");
            assert!(!cwd.is_empty());
            assert!(cmd.contains("--dangerously-skip-permissions"));
            assert!(cmd.contains("--conversation 'c2ec9bfb-e49a-4d8e-842a-eb9bcbadba07'"));
        }
    }
}
