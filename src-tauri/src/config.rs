//! Read pi configuration resources: MCP servers (mcp.json), agents
//! (global ~/.pi/agent/agents + per-project .pi/agents) and skills
//! (global + per-project SKILL.md).

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read};

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
    pub file: String, // discovered display path; also the exact document lookup key
    pub name: String,
    pub description: String,
    pub source: String, // "global" or a project path
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillDocument {
    pub file: String,
    pub content: String,
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
    for cwd in local_project_cwds(crate::remote::current_host().is_some(), || {
        mcp_project_cwds(&sessions_dir())
    }) {
        read_mcp_file(&cwd.join(".pi/mcp.json"), &cwd.to_string_lossy(), &mut out);
        read_mcp_file(&cwd.join(".mcp.json"), &cwd.to_string_lossy(), &mut out);
    }
    out.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.name.cmp(&b.name)));
    out
}

// Inventory only: showing a project file does NOT grant project trust or
// imply it is enabled in a session. Global and project rows are kept separate.
fn mcp_project_cwds(root: &Path) -> BTreeSet<PathBuf> {
    let mut out = BTreeSet::new();
    if let Ok(dirs) = std::fs::read_dir(root) {
        for dir in dirs.flatten().filter(|e| e.path().is_dir()) {
            if let Ok(files) = std::fs::read_dir(dir.path()) {
                for file in files
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
                {
                    let Ok(f) = open_regular_file(&file.path()) else {
                        continue;
                    };
                    // A header is one bounded line, never an entire JSONL body.
                    let mut line = Vec::new();
                    let mut reader = std::io::BufReader::new(f.take(MAX_METADATA_BYTES));
                    if reader.read_until(b'\n', &mut line).is_err()
                        || (line.len() == MAX_METADATA_BYTES as usize && !line.ends_with(b"\n"))
                    {
                        continue;
                    }
                    let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                        continue;
                    };
                    if v.get("type").and_then(Value::as_str) == Some("session") {
                        if let Some(cwd) = v
                            .get("cwd")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
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
    for cwd in local_project_cwds(crate::remote::current_host().is_some(), || {
        mcp_project_cwds(&sessions_dir())
    }) {
        scan_agent_dir(
            &cwd.join(".pi/agents"),
            &cwd.to_string_lossy(),
            &mut by_name,
        );
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

const MAX_METADATA_BYTES: u64 = 64 * 1024;
const MAX_SKILL_BYTES: u64 = 1024 * 1024;

// The closure must not run for a remote source: cached headers can contain
// paths that happen to exist on this machine, but do not authorize local I/O.
fn local_project_cwds(
    remote: bool,
    discover: impl FnOnce() -> BTreeSet<PathBuf>,
) -> BTreeSet<PathBuf> {
    if remote {
        BTreeSet::new()
    } else {
        discover()
    }
}

#[derive(Clone)]
struct SkillEntry {
    file: PathBuf,
    source: String,
    fallback_name: String,
    remote_boundary: Option<RemoteSkillBoundary>,
}

#[derive(Clone)]
struct RemoteSkillBoundary {
    root: PathBuf,       // canonical selected remote agent cache, captured at discovery
    skills_dir: PathBuf, // original scanned directory; revalidate before reads
}

fn confined_remote_path(path: &Path, root: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve cached skill path: {e}"))?;
    if !canonical.starts_with(root) {
        return Err("Skill path escapes the current remote agent cache".into());
    }
    Ok(canonical)
}

fn skill_read_path(entry: &SkillEntry) -> Result<PathBuf, String> {
    if let Some(boundary) = &entry.remote_boundary {
        // Both boundaries matter: an outside skills directory may itself hold
        // an alias back into the cache. Never traverse that outside directory.
        confined_remote_path(&boundary.skills_dir, &boundary.root)?;
        confined_remote_path(&entry.file, &boundary.root)
    } else {
        // Local configured symlinks intentionally remain supported.
        Ok(entry.file.clone())
    }
}

// Path discovery is separate from metadata/body reads. The explicit click
// rebuilds this allowlist without opening any other skill document.
fn scan_skill_paths(
    dir: &Path,
    source: &str,
    remote_root: Option<&Path>,
    out: &mut Vec<SkillEntry>,
) {
    // Refuse an escaping skills-directory symlink before enumerating it.
    if let Some(root) = remote_root {
        if confined_remote_path(dir, root).is_err() {
            return;
        }
    }
    if let Ok(fd) = std::fs::read_dir(dir) {
        for f in fd.flatten() {
            let file = f.path().join("SKILL.md");
            let entry = SkillEntry {
                file,
                source: source.to_string(),
                fallback_name: f.file_name().to_string_lossy().into_owned(),
                remote_boundary: remote_root.map(|root| RemoteSkillBoundary {
                    root: root.to_path_buf(),
                    skills_dir: dir.to_path_buf(),
                }),
            };
            // Resolve and confine the candidate before even checking its type;
            // an entry-directory or SKILL.md symlink must not authorize a local file.
            if skill_read_path(&entry).is_ok_and(|path| path.is_file()) {
                out.push(entry);
            }
        }
    }
}

fn discover_skill_paths(agent_root: &Path, sessions_root: &Path, remote: bool) -> Vec<SkillEntry> {
    let mut out = Vec::new();
    let remote_root = if remote {
        let Ok(root) = agent_root.canonicalize() else {
            return out;
        };
        Some(root)
    } else {
        None
    };
    scan_skill_paths(
        &agent_root.join("skills"),
        "global",
        remote_root.as_deref(),
        &mut out,
    );
    for cwd in local_project_cwds(remote, || mcp_project_cwds(sessions_root)) {
        let source = cwd.to_string_lossy();
        scan_skill_paths(&cwd.join(".agents/skills"), &source, None, &mut out);
        scan_skill_paths(&cwd.join(".pi/skills"), &source, None, &mut out);
    }
    out
}

fn current_skill_paths() -> Vec<SkillEntry> {
    // pi_agent_dir selects remote.agent_root()/skills when the source is remote.
    discover_skill_paths(
        &pi_agent_dir(),
        &sessions_dir(),
        crate::remote::current_host().is_some(),
    )
}

fn open_regular_file(path: &Path) -> Result<std::fs::File, String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("Cannot read skill/resource file: {e}"))?;
    if !metadata.is_file() {
        return Err("Skill/resource is not a regular file".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Avoid hanging if a regular file is replaced by a FIFO before open.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("Cannot open skill/resource file: {e}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Skill/resource is not a regular file".into());
    }
    Ok(file)
}

// Remote reads walk a resolved, contained path without following any new
// symlinks. This also closes the check/open gap if a cache alias is retargeted.
fn open_skill_file(entry: &SkillEntry) -> Result<std::fs::File, String> {
    let path = skill_read_path(entry)?;
    if entry.remote_boundary.is_none() {
        return open_regular_file(&path);
    }
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        let components: Vec<_> = path.components().collect();
        if !matches!(components.first(), Some(std::path::Component::RootDir)) {
            return Err("Remote cache skill path must be absolute".into());
        }
        let mut handle = std::fs::File::open("/").map_err(|e| e.to_string())?;
        for (index, component) in components.iter().enumerate().skip(1) {
            let std::path::Component::Normal(name) = component else {
                return Err("Invalid remote cache skill path".into());
            };
            let name = std::ffi::CString::new(name.as_bytes()).map_err(|e| e.to_string())?;
            let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
            if index + 1 < components.len() {
                flags |= libc::O_DIRECTORY;
            }
            // Each descriptor remains held until its child has been opened.
            let fd = unsafe { libc::openat(handle.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(format!(
                    "Cannot open confined cached skill: {}",
                    std::io::Error::last_os_error()
                ));
            }
            handle = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        if !handle.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Skill/resource is not a regular file".into());
        }
        Ok(handle)
    }
    #[cfg(not(unix))]
    {
        // Do not silently weaken the remote cache boundary on an unsupported OS.
        Err("Confined remote skill reads are unsupported on this platform".into())
    }
}

fn frontmatter_from_file(file: std::fs::File) -> Result<String, String> {
    let mut reader = std::io::BufReader::new(file.take(MAX_METADATA_BYTES));
    let mut text = String::new();
    // Stop at the closing frontmatter delimiter, not the document body.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        if text.is_empty() && line.trim() != "---" {
            return Ok(String::new());
        }
        let end = !text.is_empty() && line.trim() == "---";
        text.push_str(&line);
        if end {
            break;
        }
    }
    Ok(text)
}

#[cfg(test)]
fn skill_metadata(path: &Path) -> std::collections::HashMap<String, String> {
    parse_frontmatter(
        &open_regular_file(path)
            .and_then(frontmatter_from_file)
            .unwrap_or_default(),
    )
}

fn skill_inventory(entries: Vec<SkillEntry>) -> Vec<SkillInfo> {
    let mut out: Vec<_> = entries
        .into_iter()
        .map(|entry| {
            let fm = parse_frontmatter(
                &open_skill_file(&entry)
                    .and_then(frontmatter_from_file)
                    .unwrap_or_default(),
            );
            SkillInfo {
                file: entry.file.to_string_lossy().into_owned(),
                name: fm.get("name").cloned().unwrap_or(entry.fallback_name),
                description: fm.get("description").cloned().unwrap_or_default(),
                source: entry.source,
            }
        })
        .collect();
    out.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.name.cmp(&b.name)));
    out
}

fn read_skills() -> Vec<SkillInfo> {
    skill_inventory(current_skill_paths())
}

fn validate_skill_key(file: &str) -> Result<(), String> {
    // Check raw segments before Path normalization (which erases `.`).
    if file.is_empty() || file.split('/').any(|p| p == "." || p == "..") {
        return Err("Invalid skill file path: traversal is not allowed".into());
    }
    Ok(())
}

fn read_discovered_skill(file: &str, entries: &[SkillEntry]) -> Result<SkillDocument, String> {
    validate_skill_key(file)?;
    let entry = entries
        .iter()
        .find(|entry| entry.file.to_string_lossy() == file)
        .ok_or("Skill is unknown, missing, or no longer discovered for the current source")?;
    let handle = open_skill_file(entry)?;
    if handle.metadata().map_err(|e| e.to_string())?.len() > MAX_SKILL_BYTES {
        return Err("Skill document exceeds the 1 MiB limit".into());
    }
    let mut bytes = Vec::new();
    handle
        .take(MAX_SKILL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read skill document: {e}"))?;
    if bytes.len() > MAX_SKILL_BYTES as usize {
        return Err("Skill document exceeds the 1 MiB limit".into());
    }
    let content = String::from_utf8(bytes).map_err(|_| "Skill document is not valid UTF-8")?;
    // Do not canonicalize the result: keep the selected inventory path (including
    // a discovered symlink alias), even when two rows share a skill name.
    Ok(SkillDocument {
        file: file.to_string(),
        content,
    })
}

pub fn read_config_skill(file: &str) -> Result<SkillDocument, String> {
    validate_skill_key(file)?;
    read_discovered_skill(file, &current_skill_paths())
}

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

#[cfg(test)]
mod config_skill_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "viewer-config-skill-{}-{unique}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn skill(&self, relative: &str, content: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(relative).join("SKILL.md");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
            path
        }
        fn session(&self, cwd: &Path) {
            let dir = self.0.join("sessions/--deliberately-wrong-encoded-name--");
            std::fs::create_dir_all(&dir).unwrap();
            let header = serde_json::json!({"type":"session", "cwd":cwd});
            std::fs::write(
                dir.join("test.jsonl"),
                format!("{header}\nnot parsed: body\n"),
            )
            .unwrap();
        }
        fn discover(&self, remote: bool) -> Vec<SkillEntry> {
            discover_skill_paths(&self.0.join("agent"), &self.0.join("sessions"), remote)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn key(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn duplicate_names_keep_distinct_exact_inventory_file_identity() {
        let f = Fixture::new();
        let a = f.skill(
            "agent/skills/a",
            "---\nname: same\ndescription: first\n---\nA",
        );
        let b = f.skill(
            "agent/skills/b",
            "---\nname: same\ndescription: second\n---\nB",
        );
        let paths = f.discover(false);
        let rows = skill_inventory(paths.clone());
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|r| r.name == "same" && r.source == "global"));
        assert_eq!(
            rows.iter().map(|r| r.file.clone()).collect::<BTreeSet<_>>(),
            BTreeSet::from([key(&a), key(&b)])
        );
        for path in [a, b] {
            let doc = read_discovered_skill(&key(&path), &paths).unwrap();
            assert_eq!(doc.file, key(&path));
            assert_eq!(doc.content, std::fs::read_to_string(path).unwrap());
            let dto = serde_json::to_value(doc).unwrap();
            assert_eq!(dto.as_object().unwrap().len(), 2);
            assert!(dto["file"].is_string() && dto["content"].is_string());
        }
    }

    #[test]
    fn actual_hyphenated_cwd_discovers_both_existing_project_skill_roots_and_sorts() {
        let f = Fixture::new();
        let cwd = f.0.join("real-project-with-hyphens");
        f.session(&cwd);
        let a = f.skill(
            "real-project-with-hyphens/.agents/skills/a",
            "---\nname: z-last\n---\nA",
        );
        let b = f.skill(
            "real-project-with-hyphens/.pi/skills/b",
            "---\nname: a-first\n---\nB",
        );
        f.skill(
            "real-project-with-hyphens/.pi/skills/c",
            "---\nname: same\n---\nproject",
        );
        f.skill("agent/skills/global", "---\nname: same\n---\nglobal");
        let paths = f.discover(false);
        let rows = skill_inventory(paths.clone());
        assert_eq!(rows.len(), 4);
        assert!(rows
            .iter()
            .any(|r| r.file == key(&a) && r.source == key(&cwd)));
        assert!(rows
            .iter()
            .any(|r| r.file == key(&b) && r.source == key(&cwd)));
        assert_eq!(rows.iter().filter(|r| r.name == "same").count(), 2);
        assert!(rows
            .windows(2)
            .all(|w| (&w[0].source, &w[0].name) <= (&w[1].source, &w[1].name)));
        assert_eq!(
            read_discovered_skill(&key(&b), &paths).unwrap().file,
            key(&b)
        );
    }

    #[test]
    fn remote_uses_cached_global_only_and_never_calls_local_project_discovery() {
        let f = Fixture::new();
        let global = f.skill("agent/skills/cached", "cached remote skill");
        let local = f.skill("local-project/.pi/skills/local", "must not be read");
        f.session(&f.0.join("local-project"));
        assert!(local_project_cwds(true, || panic!(
            "remote must not inspect even cached CWD headers"
        ))
        .is_empty());
        let paths = f.discover(true);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].file, global);
        assert_eq!(
            read_discovered_skill(&key(&global), &paths)
                .unwrap()
                .content,
            "cached remote skill"
        );
        assert!(read_discovered_skill(&key(&local), &paths)
            .unwrap_err()
            .contains("current source"));
    }

    #[test]
    fn exact_utf8_text_read_is_readonly_and_never_executed() {
        let f = Fixture::new();
        let marker = f.0.join("MUST-NOT-EXIST");
        let text = format!(
            "#!/bin/sh\r\ntouch '{}'\r\n# 非执行 🐈\n\0end\n",
            marker.display()
        );
        let path = f.skill("agent/skills/text", &text);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        let before = std::fs::metadata(&path).unwrap();
        let doc = read_discovered_skill(&key(&path), &f.discover(false)).unwrap();
        assert_eq!(doc.content.as_bytes(), text.as_bytes());
        assert_eq!(doc.file, key(&path));
        assert!(!marker.exists());
        assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(before.permissions(), after.permissions());
    }

    #[test]
    fn exact_limit_succeeds_and_large_file_stays_in_inventory_but_read_errors() {
        let f = Fixture::new();
        let path = f.skill("agent/skills/large", "---\nname: large\n---\n");
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_SKILL_BYTES).unwrap();
        let paths = f.discover(false);
        assert_eq!(
            read_discovered_skill(&key(&path), &paths)
                .unwrap()
                .content
                .len(),
            MAX_SKILL_BYTES as usize
        );
        file.set_len(MAX_SKILL_BYTES + 1).unwrap();
        assert_eq!(skill_inventory(f.discover(false))[0].name, "large");
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("1 MiB"));
    }

    #[test]
    fn invalid_utf8_body_is_visible_as_inventory_then_explicit_error() {
        let f = Fixture::new();
        let path = f.skill(
            "agent/skills/invalid",
            b"---\nname: invalid-body\n---\n\xff",
        );
        let paths = f.discover(false);
        assert_eq!(skill_inventory(paths.clone())[0].name, "invalid-body");
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("UTF-8"));
        std::fs::write(&path, [0xff]).unwrap();
        let rows = skill_inventory(f.discover(false));
        assert_eq!(rows[0].name, "invalid");
        assert_eq!(rows[0].file, key(&path));
    }

    #[test]
    fn missing_deleted_unknown_and_traversal_paths_return_visible_errors() {
        let f = Fixture::new();
        let path = f.skill("agent/skills/known", "known");
        let unknown = f.skill("outside-discovery", "private unrelated file");
        let paths = f.discover(false);
        assert!(read_discovered_skill(&key(&unknown), &paths)
            .unwrap_err()
            .contains("unknown"));
        for input in [
            String::new(),
            format!("{}/./SKILL.md", path.parent().unwrap().display()),
            format!("{}/../known/SKILL.md", path.parent().unwrap().display()),
        ] {
            assert!(read_discovered_skill(&input, &paths)
                .unwrap_err()
                .contains("traversal"));
        }
        std::fs::remove_file(&path).unwrap();
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("Cannot read"));
        assert!(read_discovered_skill(&key(&path), &f.discover(false))
            .unwrap_err()
            .contains("missing"));
    }

    #[test]
    fn replaced_nonregular_document_is_rejected_without_opening() {
        let f = Fixture::new();
        let path = f.skill("agent/skills/replaced", "regular initially");
        let paths = f.discover(false);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("regular file"));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            std::fs::remove_dir(&path).unwrap();
            let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
            assert!(read_discovered_skill(&key(&path), &paths)
                .unwrap_err()
                .contains("regular file"));
        }
    }

    #[test]
    fn metadata_reads_are_bounded_and_click_does_not_need_other_document_bodies() {
        let f = Fixture::new();
        let selected = f.skill("agent/skills/selected", "selected body");
        let other = f.skill("agent/skills/other", "---\nname: bounded\n---\n");
        // A huge sparse body cannot be read into inventory memory; frontmatter
        // closes before it. Removing read permission is not relied on (root).
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&other)
            .unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap();
        let paths = f.discover(false);
        assert_eq!(skill_metadata(&other)["name"], "bounded");
        assert_eq!(
            read_discovered_skill(&key(&selected), &paths)
                .unwrap()
                .content,
            "selected body"
        );
        // No closing delimiter: keys beyond the metadata budget stay unseen.
        let mut prefix = b"---\n".to_vec();
        prefix.resize(MAX_METADATA_BYTES as usize, b' ');
        prefix.extend_from_slice(b"\nname: outside-budget\n---\n");
        std::fs::write(&other, prefix).unwrap();
        assert!(!skill_metadata(&other).contains_key("name"));
    }

    #[test]
    fn cwd_header_read_is_bounded_to_first_64_kib_line() {
        let f = Fixture::new();
        f.session(&f.0.join("project-with-hyphens"));
        let session =
            f.0.join("sessions/--deliberately-wrong-encoded-name--/test.jsonl");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&session)
            .unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap();
        assert_eq!(
            mcp_project_cwds(&f.0.join("sessions")),
            BTreeSet::from([f.0.join("project-with-hyphens")])
        );
        let huge = format!(
            "{{\"type\":\"session\",\"cwd\":\"/not-accepted\",\"padding\":\"{}\"}}\n",
            "x".repeat(MAX_METADATA_BYTES as usize)
        );
        std::fs::write(&session, huge).unwrap();
        assert!(mcp_project_cwds(&f.0.join("sessions")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn discovered_symlink_display_path_is_not_replaced_by_canonical_target() {
        let f = Fixture::new();
        let target = f.skill("agent/skills/original", "---\nname: same\n---\nexact");
        let alias = f.0.join("agent/skills/alias");
        std::os::unix::fs::symlink(target.parent().unwrap(), &alias).unwrap();
        let display = alias.join("SKILL.md");
        let paths = f.discover(false);
        assert_eq!(paths.len(), 2);
        let doc = read_discovered_skill(&key(&display), &paths).unwrap();
        assert_eq!(doc.file, key(&display));
        assert_eq!(doc.content, std::fs::read_to_string(target).unwrap());
    }
    #[cfg(unix)]
    #[test]
    fn remote_cache_rejects_escaping_entry_and_document_symlinks_but_local_allows_them() {
        let f = Fixture::new();
        let external = f.skill("outside/real", "---\nname: outside-secret\n---\nlocal-only");
        let inside = f.skill("agent/skills/inside", "---\nname: inside\n---\ncached");
        let skills = f.0.join("agent/skills");
        std::os::unix::fs::symlink(external.parent().unwrap(), skills.join("escaped-dir")).unwrap();
        std::fs::create_dir(skills.join("escaped-file")).unwrap();
        std::os::unix::fs::symlink(&external, skills.join("escaped-file/SKILL.md")).unwrap();
        std::os::unix::fs::symlink(inside.parent().unwrap(), skills.join("contained-alias"))
            .unwrap();
        let remote = f.discover(true);
        assert_eq!(remote.len(), 2);
        assert!(remote
            .iter()
            .all(|e| !e.file.to_string_lossy().contains("escaped-")));
        for e in &remote {
            let document = read_discovered_skill(&key(&e.file), &remote).unwrap();
            assert_eq!(document.file, key(&e.file));
            assert!(document.content.ends_with("cached"));
        }
        let local = f.discover(false);
        assert_eq!(local.len(), 4);
        for relative in ["escaped-dir/SKILL.md", "escaped-file/SKILL.md"] {
            let display = key(&skills.join(relative));
            assert!(read_discovered_skill(&display, &remote).is_err());
            let doc = read_discovered_skill(&display, &local).unwrap();
            assert_eq!(doc.file, display);
            assert!(doc.content.ends_with("local-only"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn remote_cache_rejects_escaping_scanned_skills_directory_even_with_contained_candidate() {
        let f = Fixture::new();
        let target = f.skill("agent/contained/real", "---\nname: cached\n---\ninside");
        let outside = f.0.join("outside-skills");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(target.parent().unwrap(), outside.join("alias-back-inside"))
            .unwrap();
        std::os::unix::fs::symlink(&outside, f.0.join("agent/skills")).unwrap();
        assert!(f.discover(true).is_empty());
        let local = f.discover(false);
        assert_eq!(local.len(), 1);
        assert!(read_discovered_skill(&key(&local[0].file), &local)
            .unwrap()
            .content
            .ends_with("inside"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_cache_rechecks_boundary_before_metadata_and_document_reads() {
        let f = Fixture::new();
        let path = f.skill(
            "agent/skills/retargeted",
            "---\nname: originally-cached\n---\nsafe",
        );
        let outside = f.skill("outside/secret", "---\nname: MUST-NOT-LOAD\n---\nprivate");
        let discovered = f.discover(true);
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(read_discovered_skill(&key(&path), &discovered)
            .unwrap_err()
            .contains("escapes"));
        let rows = skill_inventory(discovered);
        assert_eq!(rows[0].name, "retargeted"); // no outside frontmatter loaded
        assert_eq!(rows[0].file, key(&path));
        assert!(f.discover(true).is_empty());

        // Replacing the scanned directory also invalidates an earlier row,
        // even if its ultimate file target is still within the remote cache.
        let target = f.skill("agent/contained/real", "still cached");
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let discovered = f.discover(true);
        let outside_dir = f.0.join("outside-skills");
        std::fs::rename(f.0.join("agent/skills"), &outside_dir).unwrap();
        std::os::unix::fs::symlink(&outside_dir, f.0.join("agent/skills")).unwrap();
        assert!(read_discovered_skill(&key(&path), &discovered)
            .unwrap_err()
            .contains("escapes"));
        assert!(f.discover(true).is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn confined_remote_reads_keep_regular_file_utf8_and_size_checks() {
        let f = Fixture::new();
        let path = f.skill("agent/skills/checked", "valid");
        let paths = f.discover(true);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_SKILL_BYTES + 1)
            .unwrap();
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("1 MiB"));
        std::fs::write(&path, [0xff]).unwrap();
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("UTF-8"));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("regular file"));
        std::fs::remove_dir(&path).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        assert!(read_discovered_skill(&key(&path), &paths)
            .unwrap_err()
            .contains("regular file"));
    }
}
