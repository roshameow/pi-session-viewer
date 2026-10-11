//! Click-only local document opening. No directory enumeration, transcript parsing,
//! shell command, model send, or automatic opener launch. Source is captured once
//! for the entire operation, just like the other source-aware blocking commands.

use crate::{remote, sessions};
use std::path::{Path, PathBuf};
use std::process::Command;

#[tauri::command]
pub async fn open_local_file(href: String, session_path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        remote::with_current_source(|| {
            // Reject before consulting local roots or reading any header.
            reject_remote(remote::current_host().is_some())?;
            let roots = [
                sessions::sessions_dir(),
                sessions::pi_agent_dir().join("agent-logs"),
            ];
            open_with(
                &href,
                &session_path,
                &roots,
                std::env::var_os("HOME").map(PathBuf::from).as_deref(),
                run_opener,
            )
        })
    })
    .await
    .map_err(|e| format!("Local file opener task failed: {e}"))?
}

fn reject_remote(is_remote: bool) -> Result<(), String> {
    if is_remote {
        Err("Local file links are unavailable for remote sessions".into())
    } else {
        Ok(())
    }
}

fn clean_text(value: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("Empty paths and control characters are not allowed".into());
    }
    Ok(())
}

fn session_cwd(session_path: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    clean_text(session_path)?;
    let input = Path::new(session_path);
    if !input.is_absolute() || input.extension().is_none_or(|e| e != "jsonl") {
        return Err("Expected an absolute local session JSONL path".into());
    }
    // Gate lexically before ANY filesystem operation on user-supplied input.
    // Path::components normalizes interior '.', so inspect raw components too.
    if session_path
        .split('/')
        .any(|part| part == "." || part == "..")
        || !roots
            .iter()
            .any(|root| root.is_absolute() && input.starts_with(root))
    {
        return Err("Session path is outside the local session roots".into());
    }
    let canonical = input
        .canonicalize()
        .map_err(|_| "Local session file not found")?;
    // Retain canonical containment as well: symlinks may not escape either root.
    // No root or project scans.
    if !roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .any(|r| canonical.starts_with(r))
    {
        return Err("Session path is outside the local session roots".into());
    }
    if !canonical.is_file() {
        return Err("Expected a regular session file".into());
    }
    let header = sessions::session_header(canonical.to_str().ok_or("Session path is not UTF-8")?)?;
    clean_text(&header.cwd)?;
    let cwd = PathBuf::from(header.cwd);
    if !cwd.is_absolute() {
        return Err("Session header has no absolute working directory".into());
    }
    Ok(cwd)
}

/// Decode normalized Markdown URI destinations exactly once. Malformed escapes
/// fail closed, including when an ambiguously named raw-percent file exists.
fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = |c: u8| (c as char).to_digit(16).map(|n| n as u8);
            let a = bytes.get(i + 1).and_then(|c| hex(*c));
            let b = bytes.get(i + 2).and_then(|c| hex(*c));
            let (Some(a), Some(b)) = (a, b) else {
                return Err("Invalid percent encoding in file link".into());
            };
            decoded.push(a * 16 + b);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| "File link is not valid UTF-8")?;
    clean_text(&decoded)?;
    Ok(decoded)
}

fn line_anchor(value: &str) -> bool {
    fn position(value: &str) -> bool {
        let Some(value) = value.strip_prefix('L') else {
            return false;
        };
        let (line, col) = value
            .split_once('C')
            .map_or((value, None), |(l, c)| (l, Some(c)));
        digits(line) && col.is_none_or(digits)
    }
    let (start, end) = value
        .split_once('-')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    position(start) && end.is_none_or(position)
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

fn without_location(value: &str) -> &str {
    let value = match value.rsplit_once('#') {
        Some((path, anchor)) if line_anchor(anchor) => path,
        _ => value,
    };
    // Strip at most :line[:column], only after trying the existing exact path.
    let Some((head, last)) = value.rsplit_once(':') else {
        return value;
    };
    if !digits(last) {
        return value;
    }
    match head.rsplit_once(':') {
        Some((path, line)) if digits(line) => path,
        _ => head,
    }
}

fn scheme(value: &str) -> Option<&str> {
    let (name, _) = value.split_once(':')?;
    let mut chars = name.chars();
    if !chars.next()?.is_ascii_alphabetic()
        || !chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    {
        return None;
    }
    Some(name)
}

fn reject_nonlocal(value: &str) -> Result<(), String> {
    if value.starts_with("//") || value.contains('\\') {
        return Err("Network paths and backslashes are not allowed".into());
    }
    if let Some(scheme) = scheme(value) {
        // A bare report.md:12[:3] is ambiguous with a URL scheme. Accept only
        // numeric location suffixes; explicit ./ handles other colon filenames.
        let name = scheme.to_ascii_lowercase();
        let rest = &value[scheme.len() + 1..];
        let rest = match rest.rsplit_once('#') {
            Some((path, anchor)) if line_anchor(anchor) => path,
            _ => rest,
        };
        let location = rest.split(':').count() <= 2 && rest.split(':').all(digits);
        if !location
            || [
                "file",
                "javascript",
                "data",
                "vscode",
                "http",
                "https",
                "mailto",
                "ftp",
                "ssh",
            ]
            .contains(&name.as_str())
        {
            return Err("Only local paths and file:// URLs are supported".into());
        }
    }
    Ok(())
}

fn file_url_path(href: &str) -> Result<(String, String), String> {
    // Strict local subset of file URLs, intentionally without URL normalization
    // or a second decoding pass. Reject authority credentials/ports/remote hosts.
    let rest = href.get(5..).ok_or("Invalid file URL")?;
    let rest = rest
        .strip_prefix("//")
        .ok_or("Expected file:/// or file://localhost/")?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or("File URL has no absolute path")?;
    if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
        return Err(
            "File URL must have an empty or localhost host, without credentials or port".into(),
        );
    }
    if path.contains('?') {
        return Err("File URL queries are not supported".into());
    }
    let path = match path.split_once('#') {
        Some((path, anchor)) if line_anchor(anchor) => path,
        Some(_) => return Err("Only #L line anchors are supported in file URLs".into()),
        None => path,
    };
    let raw = format!("/{path}");
    let path = percent_decode(&raw)?;
    if path.starts_with("//") || path.contains('\\') {
        return Err("Network file URLs are not supported".into());
    }
    // Remove only literal location syntax. An escaped %23L or %3A12 belongs
    // to the filename; it must not become an anchor/suffix after decoding.
    let without_location = percent_decode(without_location(&raw))?;
    Ok((path, without_location))
}

fn absolute_path(value: &str, cwd: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    clean_text(value)?;
    reject_nonlocal(value)?;
    let path = if let Some(rest) = value.strip_prefix("~/") {
        home.filter(|p| p.is_absolute())
            .ok_or("HOME is not available")?
            .join(rest)
    } else if value.starts_with('~') {
        return Err("Only ~/ home paths are supported".into());
    } else {
        let path = Path::new(value);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        }
    };
    Ok(path)
}

fn existing(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true), // Includes dangling symlinks: never fall back to another file.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("Cannot inspect local target: {e}")),
    }
}

fn resolve_target(href: &str, cwd: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    clean_text(href)?;
    if scheme(href).is_some_and(|s| s.eq_ignore_ascii_case("file")) {
        let (path, without_location) = file_url_path(href)?;
        let exact = absolute_path(&path, cwd, home)?;
        if existing(&exact)? {
            return Ok(exact);
        }
        return absolute_path(&without_location, cwd, home);
    }
    reject_nonlocal(href)?;
    let decoded = percent_decode(href)?;
    let exact = absolute_path(&decoded, cwd, home)?;
    if existing(&exact)? {
        return Ok(exact);
    }
    // Strip syntax BEFORE decoding to preserve escaped #/colon provenance.
    let stripped = without_location(href);
    let decoded_location = absolute_path(&percent_decode(stripped)?, cwd, home)?;
    if existing(&decoded_location)? {
        return Ok(decoded_location);
    }
    // Compatibility fallback only after all decoded candidates are absent.
    // A decoded denied target/dangling link is selected above and must error
    // during validation; it can NEVER fall back to another raw-percent file.
    if href.contains('%') {
        let raw = absolute_path(href, cwd, home)?;
        if existing(&raw)? {
            return Ok(raw);
        }
        let raw_location = absolute_path(stripped, cwd, home)?;
        if existing(&raw_location)? {
            return Ok(raw_location);
        }
    }
    Ok(decoded_location)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Text,
    Document,
}

fn extension_name(path: &Path) -> Option<String> {
    // Exact existing colon/#L filenames are still files. Classify their base
    // suffix safely (.exe:12 remains forbidden; .md:12 is forced text).
    let name = path.file_name()?.to_str()?;
    Path::new(without_location(name))
        .extension()?
        .to_str()
        .map(str::to_ascii_lowercase)
}

fn file_kind(path: &Path) -> Option<FileKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "readme"
            | "license"
            | "changelog"
            | "makefile"
            | "dockerfile"
            | ".gitignore"
            | ".gitattributes"
            | ".editorconfig"
            | ".npmrc"
            | ".env"
    ) {
        return Some(FileKind::Text);
    }
    let ext = extension_name(path)?;
    if matches!(
        ext.as_str(),
        "md" | "markdown"
            | "mdx"
            | "txt"
            | "text"
            | "json"
            | "jsonl"
            | "yaml"
            | "yml"
            | "toml"
            | "csv"
            | "tsv"
            | "log"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "py"
            | "pyw"
            | "sh"
            | "zsh"
            | "bash"
            | "fish"
            | "ksh"
            | "csh"
            | "rs"
            | "go"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "cxx"
            | "hpp"
            | "swift"
            | "sql"
            | "java"
            | "kt"
            | "kts"
            | "scala"
            | "rb"
            | "pl"
            | "pm"
            | "php"
            | "lua"
            | "r"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "xml"
            | "svg"
            | "ini"
            | "cfg"
            | "conf"
            | "properties"
            | "lock"
            | "env"
            | "diff"
            | "patch"
            | "vue"
            | "svelte"
            | "ipynb"
            | "tex"
            | "rst"
            | "adoc"
            | "gradle"
            | "groovy"
            | "dockerfile"
    ) {
        Some(FileKind::Text)
    } else if matches!(
        ext.as_str(),
        "pdf"
            | "rtf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "ico"
            | "heic"
            | "avif"
    ) {
        Some(FileKind::Document)
    } else {
        None
    }
}

fn script_extension(path: &Path) -> bool {
    extension_name(path).is_some_and(|e| {
        matches!(
            e.as_str(),
            "js" | "jsx"
                | "mjs"
                | "cjs"
                | "ts"
                | "tsx"
                | "py"
                | "pyw"
                | "sh"
                | "zsh"
                | "bash"
                | "fish"
                | "ksh"
                | "csh"
                | "rb"
                | "pl"
                | "pm"
                | "php"
                | "lua"
                | "r"
                | "swift"
                | "kts"
                | "groovy"
        )
    })
}

fn dangerous_extension(path: &Path) -> bool {
    path.components().any(|component| {
        extension_name(Path::new(component.as_os_str())).is_some_and(|e| {
            matches!(
                e.as_str(),
                "app"
                    | "command"
                    | "bat"
                    | "cmd"
                    | "exe"
                    | "com"
                    | "ps1"
                    | "psm1"
                    | "psd1"
                    | "desktop"
                    | "lnk"
                    | "url"
                    | "webloc"
                    | "terminal"
                    | "iterm2"
                    | "vbs"
                    | "vbe"
                    | "jse"
                    | "wsf"
                    | "wsh"
                    | "scr"
                    | "pif"
                    | "hta"
                    | "msi"
                    | "msp"
                    | "pkg"
                    | "mpkg"
                    | "dmg"
                    | "iso"
                    | "jar"
                    | "application"
                    | "gadget"
                    | "scpt"
                    | "applescript"
                    | "workflow"
                    | "action"
                    | "service"
                    | "tool"
                    | "run"
                    | "bundle"
                    | "plugin"
                    | "dll"
                    | "so"
                    | "dylib"
                    | "cpl"
                    | "msc"
                    | "reg"
                    | "appref-ms"
                    | "osax"
                    | "saver"
                    | "prefpane"
                    | "kext"
            )
        })
    })
}

struct OpenTarget {
    path: PathBuf,
    force_text: bool,
}

fn validate_target(path: &Path) -> Result<OpenTarget, String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Local target is unavailable: {e}"))?;
    if dangerous_extension(path) || dangerous_extension(&canonical) {
        return Err("Opening programs, installers, or application bundles is not allowed".into());
    }
    let metadata = canonical
        .metadata()
        .map_err(|e| format!("Cannot inspect local target: {e}"))?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("Only regular documents and directories can be opened".into());
    }
    let mut force_text = false;
    if metadata.is_file() {
        // Both alias and actual target must be explicitly safe.
        file_kind(path).ok_or("Unknown or binary file type is not supported")?;
        let canonical_kind =
            file_kind(&canonical).ok_or("Unknown or binary target file type is not supported")?;
        // Linux has no supported forced-text launcher. Never rely on a script's
        // default handler, even when executable permissions are not set.
        if !cfg!(target_os = "macos") && (script_extension(path) || script_extension(&canonical)) {
            return Err("Opening script files safely is unsupported on this platform".into());
        }
        force_text = cfg!(target_os = "macos") && canonical_kind == FileKind::Text;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 != 0 && !force_text {
                return Err("Opening executable files is not allowed".into());
            }
        }
    }
    let text = canonical.to_str().ok_or("Local target path is not UTF-8")?;
    clean_text(text)?;
    Ok(OpenTarget {
        path: canonical,
        force_text,
    })
}

fn opener_command(target: &OpenTarget) -> Result<Command, String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/open");
        if target.force_text {
            command.arg("-t");
        }
        command.arg("--").arg(&target.path);
        Ok(command)
    }
    #[cfg(target_os = "linux")]
    {
        // Always an absolute canonical path, not a URL or an option.
        debug_assert!(!target.force_text);
        let mut command = Command::new("xdg-open");
        command.arg(&target.path);
        Ok(command)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = target;
        Err("Local file opening is unsupported on this platform".into())
    }
}

fn run_opener(mut command: Command) -> Result<(), String> {
    let output = command
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not start local file opener: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Local file opener failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn open_with(
    href: &str,
    session_path: &str,
    roots: &[PathBuf],
    home: Option<&Path>,
    opener: impl FnOnce(Command) -> Result<(), String>,
) -> Result<String, String> {
    let cwd = session_cwd(session_path, roots)?;
    let target = validate_target(&resolve_target(href, &cwd, home)?)?;
    opener(opener_command(&target)?)?;
    Ok(target
        .path
        .to_str()
        .ok_or("Local target path is not UTF-8")?
        .to_owned())
}

#[cfg(test)]
mod local_links_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture {
        root: PathBuf,
        roots: [PathBuf; 2],
        cwd: PathBuf,
        session: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "viewer-local-links-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            let roots = [root.join("sessions"), root.join("agent-logs")];
            let cwd = root.join("true cwd-中文");
            for dir in [&roots[0], &roots[1], &cwd] {
                std::fs::create_dir(dir).unwrap();
            }
            let session = roots[0].join("synthetic.jsonl");
            // Poison transcript: command must read only the header.
            std::fs::write(
                &session,
                format!(
                    "{}\nnot valid JSON or transcript data\n",
                    serde_json::json!({"type":"session", "id":"fixture", "cwd":cwd})
                ),
            )
            .unwrap();
            Self {
                root,
                roots,
                cwd,
                session,
            }
        }
        fn file(&self, name: &str) -> PathBuf {
            let path = self.cwd.join(name);
            std::fs::write(&path, "synthetic document\n").unwrap();
            path
        }
        fn open(&self, href: &str) -> Result<String, String> {
            open_with(
                href,
                self.session.to_str().unwrap(),
                &self.roots,
                Some(&self.cwd),
                |command| {
                    #[cfg(target_os = "macos")]
                    assert_eq!(command.get_program(), "/usr/bin/open");
                    #[cfg(target_os = "linux")]
                    assert_eq!(command.get_program(), "xdg-open");
                    let args: Vec<_> = command.get_args().collect();
                    #[cfg(target_os = "macos")]
                    {
                        let target = Path::new(args.last().unwrap());
                        if target.is_file() && file_kind(target) == Some(FileKind::Text) {
                            assert_eq!(
                                args,
                                vec![
                                    std::ffi::OsStr::new("-t"),
                                    std::ffi::OsStr::new("--"),
                                    target.as_os_str()
                                ]
                            );
                        } else {
                            assert_eq!(args, vec![std::ffi::OsStr::new("--"), target.as_os_str()]);
                        }
                    }
                    assert!(Path::new(args.last().unwrap()).is_absolute());
                    Ok(()) // Inspect, never launch.
                },
            )
        }
        fn expect(&self, href: &str, target: &Path) {
            assert_eq!(
                Path::new(&self.open(href).unwrap()),
                target.canonicalize().unwrap()
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn relative_true_header_cwd_absolute_home_spaces_and_unicode() {
        let f = Fixture::new();
        let file = f.file("notes 中文.md");
        for href in [
            "notes 中文.md",
            "./notes 中文.md",
            "~/notes 中文.md",
            file.to_str().unwrap(),
        ] {
            f.expect(href, &file);
        }
        f.expect(".", &f.cwd);
        let shell_name = f.file("-$(touch SHOULD_NOT_EXIST); 'quote'.md");
        f.expect(shell_name.to_str().unwrap(), &shell_name);
        assert!(!f.cwd.join("SHOULD_NOT_EXIST").exists());
    }

    #[test]
    fn file_urls_decode_once_and_allow_only_local_authorities() {
        let f = Fixture::new();
        let file = f.file("notes 中文.md");
        let encoded = file
            .to_str()
            .unwrap()
            .replace(' ', "%20")
            .replace("中文", "%E4%B8%AD%E6%96%87");
        f.expect(&format!("file://{encoded}#L12-L20"), &file);
        f.expect(&format!("file://localhost{encoded}"), &file);
        f.expect(&format!("FILE://LOCALHOST{encoded}"), &file);
        for url in [
            "file://server/a",
            "file://user@localhost/a",
            "file://localhost:3/a",
            "file:///a?q=1",
            "file:///a#other",
            "file:///a%00b",
            "file:///a%0Ab",
            "file:///a%FF",
            "file:///a%ZZ",
            "file:relative",
            "file:////server/a",
            "file:///%5Cserver/a",
        ] {
            assert!(f.open(url).is_err(), "accepted {url}");
        }
        let literal = f.file("literal%20.md");
        let url = format!(
            "file://{}",
            literal
                .to_str()
                .unwrap()
                .replace(' ', "%20")
                .replace("literal%20", "literal%2520")
        );
        f.expect(&url, &literal);
        let hash = f.file("doc#L12.md");
        f.expect(
            &format!(
                "file://{}",
                hash.to_str()
                    .unwrap()
                    .replace(' ', "%20")
                    .replace('#', "%23")
            ),
            &hash,
        );
        // Encoded anchor/suffix syntax must never select a different real file.
        let base = f.file("base.md");
        let base_url = format!("file://{}", base.to_str().unwrap().replace(' ', "%20"));
        assert!(f.open(&format!("{base_url}%23L12")).is_err());
        assert!(f.open(&format!("{base_url}%3A12")).is_err());
        f.expect(&format!("{base_url}:12:3#L12"), &base);
    }

    #[test]
    fn plain_percent_decoded_precedence_and_no_double_decode() {
        let f = Fixture::new();
        let decoded = f.file("space name.md");
        f.expect("space%20name.md", &decoded);
        let literal = f.file("space%20name.md");
        f.expect("space%20name.md", &decoded); // Normalized Markdown URI wins.
        f.expect("space%2520name.md", &literal); // Literal percent needs %25.
        let invalid_literal = f.file("raw%.md");
        assert!(f.open("raw%.md").is_err());
        f.expect("raw%25.md", &invalid_literal);
        let unicode = f.file("中文.md");
        f.expect("%E4%B8%AD%E6%96%87.md", &unicode);
        assert!(f.open("space%252520name.md").is_err());
        assert!(f.open("%2F%2Fserver/a").is_err());
        assert!(f.open("%6Aavascript%3Aalert(1)").is_err());
    }

    #[test]
    fn location_suffixes_prefer_exact_existing_colon_and_hash_filenames() {
        let f = Fixture::new();
        let doc = f.file("report.md");
        for href in [
            "report.md:12",
            "report.md:12:3",
            "report.md#L12",
            "report.md#L12-L15",
            "report.md#L12C3-L15C4",
            "report.md:12:3#L12",
        ] {
            f.expect(href, &doc);
        }
        let colon = f.file("report.md:12:3");
        f.expect("report.md:12:3", &colon);
        let hash = f.file("report.md#L12");
        f.expect("report.md#L12", &hash);
        let both = f.file("file name.md");
        f.expect("file%20name.md#L12", &both);
        f.expect("file%20name.md:12:3", &both);
        assert!(f.open("report.md:bogus").is_err());
    }

    #[test]
    fn missing_targets_unsafe_schemes_and_controls_never_reach_opener() {
        let f = Fixture::new();
        for href in [
            "absent.md",
            "javascript:alert(1)",
            "javascript:1",
            "data:text/html,x",
            "vscode://file/a",
            "https://example.org",
            "http:2",
            "custom:thing",
            "//server/a",
            "\\\\server\\a",
            "",
            "bad\0name",
            "bad\nname",
            "%00.md",
            "%0A.md",
        ] {
            assert!(open_with(
                href,
                f.session.to_str().unwrap(),
                &f.roots,
                Some(&f.cwd),
                |_| panic!("opener reached for {href}")
            )
            .is_err());
        }
    }

    #[test]
    fn session_root_validation_and_agent_log_headers() {
        let f = Fixture::new();
        let file = f.file("notes.md");
        let outside = f.root.join("outside.jsonl");
        std::fs::copy(&f.session, &outside).unwrap();
        assert_eq!(
            session_cwd(outside.to_str().unwrap(), &f.roots).unwrap_err(),
            "Session path is outside the local session roots"
        );
        // This nonexistent outside path must get the lexical rejection, not
        // 'file not found' from canonicalize/metadata on arbitrary input.
        let nonexistent = f.root.join("never-created.jsonl");
        assert_eq!(
            session_cwd(nonexistent.to_str().unwrap(), &f.roots).unwrap_err(),
            "Session path is outside the local session roots"
        );
        for traversal in [
            format!("{}/../outside.jsonl", f.roots[0].display()),
            format!("{}/./synthetic.jsonl", f.roots[0].display()),
        ] {
            assert_eq!(
                session_cwd(&traversal, &f.roots).unwrap_err(),
                "Session path is outside the local session roots"
            );
        }
        assert!(session_cwd("relative.jsonl", &f.roots).is_err());
        let log = f.roots[1].join("task-fixture.jsonl");
        std::fs::copy(&f.session, &log).unwrap();
        assert!(open_with(
            "notes.md",
            log.to_str().unwrap(),
            &f.roots,
            None,
            |_| Ok(())
        )
        .is_ok());
        std::fs::write(&log, "{\"type\":\"session\",\"cwd\":\"relative\"}\n").unwrap();
        assert!(session_cwd(log.to_str().unwrap(), &f.roots).is_err());
        std::fs::write(&log, "{\"type\":\"message\",\"cwd\":\"/\"}\n").unwrap();
        assert!(session_cwd(log.to_str().unwrap(), &f.roots).is_err());
        #[cfg(unix)]
        {
            let link = f.roots[0].join("escape.jsonl");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(session_cwd(link.to_str().unwrap(), &f.roots).is_err());
        }
        f.expect("notes.md", &file);
    }

    #[test]
    fn remote_source_rejected_before_any_local_reads_or_launch() {
        let result = reject_remote(true).and_then(|_| -> Result<(), String> {
            panic!("must not inspect roots or read session headers")
        });
        assert!(result.unwrap_err().contains("remote"));
        assert!(reject_remote(false).is_ok());
        // No global current-host mutation: tests cannot switch another test's source.
    }

    #[test]
    fn executable_extensions_bundles_and_unix_modes_are_denied() {
        let f = Fixture::new();
        for name in [
            "run.command",
            "run.BAT",
            "run.cmd",
            "run.exe",
            "run.ps1",
            "run.desktop",
            "run.scpt",
            "run.jar",
            "run.pkg",
            "run.lnk",
            "run.terminal",
            "unknown.bin",
        ] {
            f.file(name);
            assert!(f.open(name).is_err(), "accepted {name}");
        }
        std::fs::create_dir(f.cwd.join("Danger.app")).unwrap();
        assert!(f.open("Danger.app").is_err());
        std::fs::write(f.cwd.join("Danger.app/doc.md"), "synthetic").unwrap();
        assert!(f.open("Danger.app/doc.md").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executable = f.file("executable.md");
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
            #[cfg(target_os = "macos")]
            f.expect("executable.md", &executable); // Only the forced -t route.
            #[cfg(not(target_os = "macos"))]
            assert!(f.open("executable.md").is_err());
            let executable_pdf = f.file("executable.pdf");
            std::fs::set_permissions(executable_pdf, std::fs::Permissions::from_mode(0o755))
                .unwrap();
            assert!(f.open("executable.pdf").is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn canonical_symlink_target_is_checked_and_returned() {
        let f = Fixture::new();
        let doc = f.file("document.md");
        std::os::unix::fs::symlink(&doc, f.cwd.join("alias.md")).unwrap();
        f.expect("alias.md", &doc);
        let program = f.file("program.command");
        std::os::unix::fs::symlink(program, f.cwd.join("looks-safe.md")).unwrap();
        assert!(f.open("looks-safe.md").is_err());
        let executable = f.file("no-extension");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(executable, f.cwd.join("other-safe.md")).unwrap();
        assert!(f.open("other-safe.md").is_err());
        std::fs::create_dir(f.cwd.join("Hidden.app")).unwrap();
        std::os::unix::fs::symlink(f.cwd.join("Hidden.app"), f.cwd.join("safe-directory")).unwrap();
        assert!(f.open("safe-directory").is_err());
        let control_target = f.file("control\nname.md");
        std::os::unix::fs::symlink(control_target, f.cwd.join("control-alias.md")).unwrap();
        assert!(f.open("control-alias.md").is_err());
        std::os::unix::fs::symlink(f.cwd.join("absent"), f.cwd.join("dangling link.md")).unwrap();
        f.file("dangling%20link.md");
        assert!(f.open("dangling%20link.md").is_err()); // Decoded dangling target wins.
        let program = f.file("denied.command");
        std::os::unix::fs::symlink(program, f.cwd.join("unsafe name.md")).unwrap();
        f.file("unsafe%20name.md");
        assert!(f.open("unsafe%20name.md").is_err()); // No denied-target fallback.
        let unknown = f.file("binary.bin");
        std::os::unix::fs::symlink(unknown, f.cwd.join("binary-alias.md")).unwrap();
        assert!(f.open("binary-alias.md").is_err());
    }

    #[test]
    fn known_text_source_routes_are_safe_and_default_documents_are_distinct() {
        let f = Fixture::new();
        for ext in [
            "md", "txt", "json", "yaml", "toml", "csv", "log", "ts", "tsx", "js", "jsx", "py",
            "sh", "zsh", "bash", "rs", "go", "c", "cpp", "swift", "sql", "java",
        ] {
            let name = format!("source.{ext}");
            let path = f.file(&name);
            #[cfg(target_os = "macos")]
            f.expect(&name, &path); // Fixture checks exact [-t, --, canonical] args.
            #[cfg(not(target_os = "macos"))]
            {
                if ["ts", "tsx", "js", "jsx", "py", "sh", "zsh", "bash", "swift"].contains(&ext) {
                    assert!(f.open(&name).is_err());
                } else {
                    f.expect(&name, &path);
                }
            }
        }
        let readme = f.file("README");
        f.expect("README", &readme);
        let pdf = f.file("normal.pdf");
        f.expect("normal.pdf", &pdf); // Fixture asserts no -t for default documents.
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn executable_source_is_forced_text_with_one_literal_canonical_argument() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let source = f.file("-source 'quoted'; $(touch SHOULD_NOT_EXIST).sh");
        std::fs::write(&source, "#!/bin/sh\ntouch SHOULD_NOT_EXIST\n").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        let canonical = source.canonicalize().unwrap();
        let result = open_with(
            source.to_str().unwrap(),
            f.session.to_str().unwrap(),
            &f.roots,
            None,
            |command| {
                assert_eq!(command.get_program(), "/usr/bin/open");
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    vec![
                        std::ffi::OsStr::new("-t"),
                        std::ffi::OsStr::new("--"),
                        canonical.as_os_str()
                    ]
                );
                Ok(()) // Never execute either open or the file.
            },
        )
        .unwrap();
        assert_eq!(Path::new(&result), canonical);
        assert!(!f.cwd.join("SHOULD_NOT_EXIST").exists());
        // The real target determines route, not the document-looking alias.
        std::os::unix::fs::symlink(&source, f.cwd.join("script-alias.md")).unwrap();
        f.expect("script-alias.md", &source);
        let pdf = f.file("canonical.pdf");
        std::os::unix::fs::symlink(&pdf, f.cwd.join("pdf-alias.md")).unwrap();
        f.expect("pdf-alias.md", &pdf); // No -t, based on canonical PDF target.
        let unknown_alias = f.cwd.join("unknown-alias.bin");
        std::os::unix::fs::symlink(source, &unknown_alias).unwrap();
        assert!(f.open("unknown-alias.bin").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn special_files_are_not_documents() {
        let f = Fixture::new();
        let socket_path = f.cwd.join("socket.md");
        let _socket = std::os::unix::net::UnixDatagram::bind(&socket_path).unwrap();
        assert!(f.open("socket.md").is_err());
    }

    #[test]
    fn fake_opener_failure_is_reported_not_successfully_spawned() {
        let f = Fixture::new();
        f.file("notes.md");
        let error = open_with(
            "notes.md",
            f.session.to_str().unwrap(),
            &f.roots,
            None,
            |_| Err("fake opener exit failure".into()),
        )
        .unwrap_err();
        assert_eq!(error, "fake opener exit failure");
    }
}
