//! Read pi configuration resources: MCP servers (mcp.json), agents
//! (global ~/.pi/agent/agents + per-project .pi/agents) and skills
//! (global + per-project SKILL.md).

use serde::Serialize;
use serde_json::Value;
use std::path::Path;

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
    let servers = v
        .get("mcpServers")
        .and_then(|x| x.as_object())
        .or_else(|| v.as_object());
    let Some(servers) = servers else { return };
    for (name, conf) in servers {
        if !conf.is_object() {
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
        });
    }
}

fn read_mcp() -> Vec<McpServer> {
    let mut out = Vec::new();
    // global config
    read_mcp_file(&pi_agent_dir().join("mcp.json"), "global", &mut out);
    // per-project .mcp.json (decode the encoded dir name to the real path)
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
            read_mcp_file(&Path::new(&real).join(".mcp.json"), &real, &mut out);
        }
    }
    out.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.name.cmp(&b.name))
    });
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
