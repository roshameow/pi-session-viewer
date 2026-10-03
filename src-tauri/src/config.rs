//! Read pi configuration resources: MCP servers (mcp.json), agents
//! (global ~/.pi/agent/agents + per-project .pi/agents) and skills
//! (global + per-project SKILL.md).

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;

use crate::sessions::{pi_agent_dir, sessions_dir};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub enabled: Option<bool>,
    pub socket: Option<String>,
    pub url: Option<String>,
    pub source: String, // "global" or a project path
    pub config_path: String,
    pub dialect: String, // shared mcpServers, or adapter-only fields / legacy file
    pub disabled: Option<bool>, // adapter flag, NOT native enabled
    pub exposure: Option<String>,
    pub tool_exposure: BTreeMap<String, String>,
    pub direct_tools: Option<Value>, // adapter supports bool / tool lists
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub description: String,
    pub tools: Option<String>,
    pub file: String,
    pub source: String, // "global" or a project path
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub source: String, // "global" or a project path
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub mcp: Vec<McpServer>,
    pub agents: Vec<AgentInfo>,
    pub skills: Vec<SkillInfo>,
}

fn parse_frontmatter(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let lines: Vec<&str> = text.lines().collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return out;
    }
    for line in lines.iter().skip(1) {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_string();
            let v = v.trim().trim_matches('"').to_string();
            if !k.is_empty() {
                out.insert(k, v);
            }
        }
    }
    out
}

fn read_mcp_file(path: &Path, source: &str, out: &mut Vec<McpServer>) {
    let Ok(data) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(&data) else {
        return;
    };
    parse_mcp(&v, path, source, out);
}

fn parse_mcp(v: &Value, path: &Path, source: &str, out: &mut Vec<McpServer>) {
    let servers = v
        .get("mcpServers")
        .and_then(|x| x.as_object())
        .or_else(|| v.as_object());
    let Some(servers) = servers else { return };
    for (name, conf) in servers {
        if !conf.is_object() || !(conf.get("command").is_some() || conf.get("url").is_some() || conf.get("socket").is_some()) {
            continue;
        }
        let command = conf
            .get("command")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let args = conf
            .get("args")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let env = conf
            .get("env")
            .and_then(|x| x.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                    .collect()
            })
            .unwrap_or_default();
        let enabled = conf.get("enabled").and_then(|x| x.as_bool());
        let socket = conf.get("socket").and_then(|x| x.as_str()).map(|s| s.to_string());
        let url = conf.get("url").and_then(|x| x.as_str()).map(|s| s.to_string());
        out.push(McpServer {
            name: name.clone(),
            command,
            args,
            env,
            enabled,
            socket,
            url,
            source: source.to_string(),
            config_path: path.to_string_lossy().into_owned(),
            dialect: if path.file_name().is_some_and(|n| n == ".mcp.json")
                || ["disabled", "socket", "directTools"].iter().any(|k| conf.get(k).is_some()) {
                "adapter".into()
            } else { "shared".into() },
            disabled: conf.get("disabled").and_then(Value::as_bool),
            exposure: conf.get("exposure").and_then(Value::as_str).map(str::to_owned),
            tool_exposure: conf.get("toolExposure").and_then(Value::as_object)
                .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.into()))).collect())
                .unwrap_or_default(),
            direct_tools: conf.get("directTools").cloned(),
        });
    }
}

fn read_mcp() -> Vec<McpServer> {
    let mut out = Vec::new();
    // global config
    read_mcp_file(&pi_agent_dir().join("mcp.json"), "global", &mut out);
    // Read actual cwd headers: encoded directory names cannot distinguish
    // hyphens from separators. Do not probe local projects for a remote host.
    if crate::remote::current_host().is_none() {
        for cwd in mcp_project_cwds(&sessions_dir()) {
            read_mcp_file(&cwd.join(".pi/mcp.json"), &cwd.to_string_lossy(), &mut out);
            read_mcp_file(&cwd.join(".mcp.json"), &cwd.to_string_lossy(), &mut out);
        }
    }
    out.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

// Inventory only: showing a project file does NOT grant project trust or
// imply it is enabled in a session. Global and project rows are kept separate.
fn mcp_project_cwds(root: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    if let Ok(dirs) = std::fs::read_dir(root) {
        for dir in dirs.flatten().filter(|e| e.path().is_dir()) {
            if let Ok(files) = std::fs::read_dir(dir.path()) {
                for file in files.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "jsonl")) {
                    let Ok(f) = std::fs::File::open(file.path()) else { continue };
                    let Some(Ok(line)) = std::io::BufReader::new(f).lines().next() else { continue };
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if v.get("type").and_then(Value::as_str) == Some("session") {
                        if let Some(cwd) = v.get("cwd").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                            out.insert(PathBuf::from(cwd));
                        }
                    }
                }
            }
        }
    }
    out
}

fn read_agents() -> Vec<AgentInfo> {
    use std::collections::HashMap;
    // Mirrors pi-subagent-durable discovery: user dir first, then per-project
    // .pi/agents; on a name conflict the project agent wins (same as the
    // extension's scope "both" merge).
    let mut by_name: HashMap<String, AgentInfo> = HashMap::new();
    scan_agent_dir(&pi_agent_dir().join("agents"), "global", &mut by_name);
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("--") {
                continue;
            }
            let real = crate::sessions::decode_dir_name(&name);
            if real.is_empty() {
                continue;
            }
            scan_agent_dir(&Path::new(&real).join(".pi").join("agents"), &real, &mut by_name);
        }
    }
    let mut out: Vec<AgentInfo> = by_name.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn scan_agent_dir(dir: &Path, source: &str, out: &mut std::collections::HashMap<String, AgentInfo>) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(fd) = std::fs::read_dir(dir) {
        for f in fd.flatten() {
            let p = f.path();
            if !p.is_file() || !p.extension().map(|e| e == "md").unwrap_or(false) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let fm = parse_frontmatter(&text);
            let name = fm
                .get("name")
                .cloned()
                .unwrap_or_else(|| f.file_name().to_string_lossy().replace(".md", ""));
            out.insert(
                name.clone(),
                AgentInfo {
                    name,
                    description: fm.get("description").cloned().unwrap_or_default(),
                    tools: fm.get("tools").cloned(),
                    file: p.to_string_lossy().to_string(),
                    source: source.to_string(),
                },
            );
        }
    }
}

fn scan_skill_dir(dir: &Path, source: &str, out: &mut Vec<SkillInfo>) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(fd) = std::fs::read_dir(dir) {
        for f in fd.flatten() {
            let p = f.path();
            let skill = p.join("SKILL.md");
            if skill.is_file() {
                if let Ok(text) = std::fs::read_to_string(&skill) {
                    let fm = parse_frontmatter(&text);
                    let name = fm
                        .get("name")
                        .cloned()
                        .unwrap_or_else(|| f.file_name().to_string_lossy().to_string());
                    out.push(SkillInfo {
                        name,
                        description: fm.get("description").cloned().unwrap_or_default(),
                        source: source.to_string(),
                    });
                }
            }
        }
    }
}

fn read_skills() -> Vec<SkillInfo> {
    let mut out = Vec::new();
    // global
    scan_skill_dir(&pi_agent_dir().join("skills"), "global", &mut out);
    // per-project .agents/skills and .pi/skills (decode the encoded dir name
    // back to the real project path)
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("--") {
                continue;
            }
            let real = crate::sessions::decode_dir_name(&name);
            if real.is_empty() {
                continue;
            }
            scan_skill_dir(&Path::new(&real).join(".agents").join("skills"), &real, &mut out);
            scan_skill_dir(&Path::new(&real).join(".pi").join("skills"), &real, &mut out);
        }
    }
    out.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

#[tauri::command]
pub fn list_config() -> ConfigView {
    ConfigView {
        mcp: read_mcp(),
        agents: read_agents(),
        skills: read_skills(),
    }
}



#[cfg(test)]
mod adaptation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_exposure_and_adapter_flags_stay_distinct() {
        let v = json!({"autoEnableCodemode": false, "mcpServers": {
            "native": {"url":"https://example.invalid/mcp", "enabled":false,
                "exposure":"codemode-deferred", "toolExposure":{"get_*":"direct", "delete":"hidden"}},
            "adapter": {"socket":"/mock.sock", "disabled":true, "directTools":["read"]},
            "default": {"command":"mock"}
        }});
        let mut rows = vec![];
        parse_mcp(&v, Path::new("/project/.pi/mcp.json"), "/project", &mut rows);
        assert_eq!(rows.len(), 3);
        let native = rows.iter().find(|r| r.name == "native").unwrap();
        assert_eq!(native.enabled, Some(false));
        assert_eq!(native.disabled, None);
        assert_eq!(native.exposure.as_deref(), Some("codemode-deferred"));
        assert_eq!(native.tool_exposure["delete"], "hidden");
        assert_eq!(native.config_path, "/project/.pi/mcp.json");
        let adapter = rows.iter().find(|r| r.name == "adapter").unwrap();
        assert_eq!(adapter.enabled, None);
        assert_eq!(adapter.disabled, Some(true));
        assert_eq!(adapter.direct_tools, Some(json!(["read"])));
        assert_eq!(adapter.dialect, "adapter");
        let default = rows.iter().find(|r| r.name == "default").unwrap();
        assert_eq!(default.exposure, None); // native default is displayed, not fabricated config
    }

    #[test]
    fn legacy_flat_map_ignores_metadata_and_does_not_merge_projects() {
        let mut rows = vec![];
        parse_mcp(&json!({"autoEnableCodemode": false, "metadata": {}, "same":{"command":"global"}}),
            Path::new("/global/mcp.json"), "global", &mut rows);
        parse_mcp(&json!({"mcpServers":{"same":{"command":"project"}}}),
            Path::new("/project/.mcp.json"), "/project", &mut rows);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source, "global");
        assert_eq!(rows[1].source, "/project");
        assert_eq!(rows[1].dialect, "adapter");
    }

    #[test]
    fn discovery_uses_cwd_header_not_lossy_directory_encoding() {
        let root = std::env::temp_dir().join(format!("viewer-mcp-adaptation-{}", std::process::id()));
        let dir = root.join("--Users-test-project-with-hyphens--");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.jsonl"),
            "{\"type\":\"session\",\"cwd\":\"/Users/test/project-with-hyphens\"}\n").unwrap();
        let cwds = mcp_project_cwds(&root);
        assert_eq!(cwds, BTreeSet::from([PathBuf::from("/Users/test/project-with-hyphens")]));
        std::fs::remove_dir_all(root).unwrap();
    }
}
