//! Click-only local document opening and Finder reveal. No directory enumeration,
//! transcript parsing, shell command, model send, or automatic opener launch.
//! Source is captured once for the entire operation, like other source-aware commands.

use crate::{remote, sessions};
use std::path::{Path, PathBuf};
use std::process::Command;

#[tauri::command]
pub async fn open_local_file(href: String, session_path: String) -> Result<String, String> {
    local_file_action(href, session_path, LocalAction::Open).await
}

#[tauri::command]
pub async fn reveal_local_file(href: String, session_path: String) -> Result<String, String> {
    local_file_action(href, session_path, LocalAction::Reveal).await
}

#[derive(Clone, Copy)]
enum LocalAction {
    Open,
    Reveal,
}

async fn local_file_action(
    href: String,
    session_path: String,
    action: LocalAction,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        remote::with_current_source(|| {
            with_local_source(remote::current_host().is_some(), || {
                // Only a remote source is rejected here. Browser-preview/readiness
                // gates belong to the frontend; neither API accepts a source flag.
                let roots = [
                    sessions::sessions_dir(),
                    sessions::pi_agent_dir().join("agent-logs"),
                ];
                let home = std::env::var_os("HOME").map(PathBuf::from);
                match action {
                    LocalAction::Open => open_with(
                        &href,
                        &session_path,
                        &roots,
                        home.as_deref(),
                        default_browser,
                        run_opener,
                    ),
                    LocalAction::Reveal => {
                        reveal_with(&href, &session_path, &roots, home.as_deref(), run_opener)
                    }
                }
            })
        })
    })
    .await
    .map_err(|e| format!("Local file action task failed: {e}"))?
}

fn with_local_source<T>(
    is_remote: bool,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    // Reject before constructing local roots or performing ANY local reads.
    reject_remote(is_remote)?;
    action()
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
    Html,
    Markdown,
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
    if matches!(ext.as_str(), "html" | "htm") {
        return Some(FileKind::Html);
    }
    if matches!(ext.as_str(), "md" | "markdown" | "mdx") {
        // An exact filename like notes.md:12 is not a Markdown association.
        // Preserve its existing forced-text treatment rather than guessing a handler.
        return Some(
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(&ext))
            {
                FileKind::Markdown
            } else {
                FileKind::Text
            },
        );
    }
    if matches!(
        ext.as_str(),
        "txt"
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
    html: bool,
    opener_path: PathBuf,
}

/// Shared existence/type/path checks, deliberately independent of launch policy.
/// Reveal may select a program or unknown extension, but never launches it.
fn canonical_target(path: &Path) -> Result<(PathBuf, std::fs::Metadata), String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Local target is unavailable: {e}"))?;
    clean_text(canonical.to_str().ok_or("Local target path is not UTF-8")?)?;
    let metadata = canonical
        .metadata()
        .map_err(|e| format!("Cannot inspect local target: {e}"))?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("Only regular files and directories are supported".into());
    }
    Ok((canonical, metadata))
}

fn validate_target(path: &Path) -> Result<OpenTarget, String> {
    let (canonical, metadata) = canonical_target(path)?;
    if dangerous_extension(path) || dangerous_extension(&canonical) {
        return Err("Opening programs, installers, or application bundles is not allowed".into());
    }
    let mut force_text = false;
    let mut html = false;
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
        html = canonical_kind == FileKind::Html;
        // Ordinary Markdown respects its own system association (VS Code,
        // Insiders, or any user preference), not the default plain-text editor.
        // A script-looking alias must not bypass the forced-text guard either.
        force_text = cfg!(target_os = "macos")
            && (canonical_kind == FileKind::Text
                || (canonical_kind == FileKind::Markdown && script_extension(path)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 != 0 {
                if cfg!(target_os = "macos") && canonical_kind == FileKind::Markdown {
                    force_text = true;
                }
                if !force_text {
                    return Err("Opening executable files is not allowed".into());
                }
            }
        }
    }
    // Keep an ordinary HTML link at its original absolute location so relative
    // images/styles resolve beside that file, not beside an import/temp copy.
    // For a file symlink alias, open the real HTML and its real asset parent:
    // aliases may live elsewhere or even have a non-HTML extension. Directory
    // symlinks are retained for ordinary files (their sibling assets still work).
    let opener_path = if html
        && !std::fs::symlink_metadata(path)
            .map_err(|e| format!("Cannot inspect local target: {e}"))?
            .file_type()
            .is_symlink()
    {
        let text = path.to_str().ok_or("Local target path is not UTF-8")?;
        clean_text(text)?;
        if !path.is_absolute() {
            return Err("HTML target path must be absolute".into());
        }
        path.to_path_buf()
    } else {
        canonical.clone()
    };
    Ok(OpenTarget {
        path: canonical,
        force_text,
        html,
        opener_path,
    })
}

/// Resolve only the configured HTTPS handler. NSWorkspace performs a local
/// configuration lookup; it does not fetch this URL or change any defaults.
#[cfg(target_os = "macos")]
fn default_browser() -> Result<PathBuf, String> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    autoreleasepool(|_| {
        let probe = NSURL::URLWithString(&NSString::from_str("https://example.invalid"))
            .ok_or("Could not construct default-browser lookup URL")?;
        let app = NSWorkspace::sharedWorkspace()
            .URLForApplicationToOpenURL(&probe)
            .ok_or("macOS has no application configured to open HTTPS URLs")?;
        if !app.isFileURL() {
            return Err("macOS returned a non-file default-browser application URL".into());
        }
        let path = app
            .path()
            .ok_or("Default-browser application URL has no path")?;
        Ok(PathBuf::from(path.to_string()))
    })
}

#[cfg(not(target_os = "macos"))]
fn default_browser() -> Result<PathBuf, String> {
    Err("Explicit default-browser lookup is only available on macOS".into())
}

fn opener_command(
    target: &OpenTarget,
    browser: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<Command, String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/open");
        if target.html {
            let lookup_error = |reason| {
                format!("Cannot find the configured default browser: {reason}. Choose a default web browser in macOS System Settings, then try again")
            };
            let app = browser().map_err(lookup_error)?;
            if !app.is_absolute() || app.to_str().is_none_or(|s| clean_text(s).is_err()) {
                return Err(lookup_error("invalid application path".into()));
            }
            command.arg("-a").arg(app);
        } else if target.force_text {
            command.arg("-t");
        }
        command.arg("--").arg(&target.opener_path);
        Ok(command)
    }
    #[cfg(target_os = "linux")]
    {
        // Linux uses its existing default handler, not macOS browser lookup.
        // HTML keeps its asset base; scripts/executables remain denied above.
        let _ = browser;
        debug_assert!(!target.force_text);
        debug_assert!(!target.html || target.opener_path.is_absolute());
        let mut command = Command::new("xdg-open");
        command.arg(&target.opener_path);
        Ok(command)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (target, browser);
        Err("Local file opening is unsupported on this platform".into())
    }
}

fn run_opener(mut command: Command) -> Result<(), String> {
    let output = command
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not start local file opener: {e}"))?;
    check_opener_output(output)
}

fn check_opener_output(output: std::process::Output) -> Result<(), String> {
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
    browser: impl FnOnce() -> Result<PathBuf, String>,
    opener: impl FnOnce(Command) -> Result<(), String>,
) -> Result<String, String> {
    let target = validate_target(&session_target(href, session_path, roots, home)?)?;
    opener(opener_command(&target, browser)?)?;
    Ok(target
        .path
        .to_str()
        .ok_or("Local target path is not UTF-8")?
        .to_owned())
}

fn session_target(
    href: &str,
    session_path: &str,
    roots: &[PathBuf],
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let cwd = session_cwd(session_path, roots)?;
    resolve_target(href, &cwd, home)
}

fn reveal_command(path: &Path) -> Result<Command, String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/open");
        command.args(["-R", "--"]).arg(path);
        Ok(command)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("Finder reveal is unsupported on this platform".into())
    }
}

fn reveal_with(
    href: &str,
    session_path: &str,
    roots: &[PathBuf],
    home: Option<&Path>,
    revealer: impl FnOnce(Command) -> Result<(), String>,
) -> Result<String, String> {
    let (canonical, _) = canonical_target(&session_target(href, session_path, roots, home)?)?;
    // Select the validated real target, not a symlink alias elsewhere. Unlike
    // opening HTML, revealing has no relative-asset base to preserve. There is
    // intentionally no launch-type allowlist or browser lookup on this path.
    revealer(reveal_command(&canonical)?)?;
    Ok(canonical
        .to_str()
        .ok_or("Local target path is not UTF-8")?
        .to_owned())
}

#[cfg(test)]
mod local_links_tests {
    // Isolated regression method: cargo test --offline --manifest-path
    // src-tauri/Cargo.toml local_links_tests -- --test-threads=1
    // Use synthetic headers, explicit file modes, fake browser providers and
    // Command inspection; test exit statuses with synthetic Output, never open.
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fake_browser() -> Result<PathBuf, String> {
        Ok(PathBuf::from("/Applications/Fake Browser.app"))
    }

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
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            path
        }
        fn open(&self, href: &str) -> Result<String, String> {
            open_with(
                href,
                self.session.to_str().unwrap(),
                &self.roots,
                Some(&self.cwd),
                fake_browser,
                |command| {
                    #[cfg(target_os = "macos")]
                    assert_eq!(command.get_program(), "/usr/bin/open");
                    #[cfg(target_os = "linux")]
                    assert_eq!(command.get_program(), "xdg-open");
                    let args: Vec<_> = command.get_args().collect();
                    #[cfg(target_os = "macos")]
                    {
                        let target = Path::new(args.last().unwrap());
                        if target.is_file()
                            && (file_kind(target) == Some(FileKind::Text)
                                || (file_kind(target) == Some(FileKind::Markdown) && {
                                    use std::os::unix::fs::PermissionsExt;
                                    target.metadata().unwrap().permissions().mode() & 0o111 != 0
                                }))
                        {
                            assert_eq!(
                                args,
                                vec![
                                    std::ffi::OsStr::new("-t"),
                                    std::ffi::OsStr::new("--"),
                                    target.as_os_str()
                                ]
                            );
                        } else if file_kind(target) == Some(FileKind::Html) {
                            assert_eq!(
                                args,
                                vec![
                                    std::ffi::OsStr::new("-a"),
                                    std::ffi::OsStr::new("/Applications/Fake Browser.app"),
                                    std::ffi::OsStr::new("--"),
                                    target.as_os_str(),
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
        #[cfg(target_os = "macos")]
        fn expect_reveal(&self, href: &str, target: &Path) {
            let canonical = target.canonicalize().unwrap();
            let result = reveal_with(
                href,
                self.session.to_str().unwrap(),
                &self.roots,
                Some(&self.cwd),
                |command| {
                    assert_eq!(command.get_program(), "/usr/bin/open");
                    assert_eq!(
                        command.get_args().collect::<Vec<_>>(),
                        vec![
                            std::ffi::OsStr::new("-R"),
                            std::ffi::OsStr::new("--"),
                            canonical.as_os_str(),
                        ]
                    );
                    assert!(command.get_current_dir().is_none());
                    Ok(()) // Never execute open, a shell, or the target.
                },
            )
            .unwrap();
            assert_eq!(Path::new(&result), canonical);
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
                || panic!("browser lookup reached for {href}"),
                |_| panic!("opener reached for {href}")
            )
            .is_err());
            assert!(reveal_with(
                href,
                f.session.to_str().unwrap(),
                &f.roots,
                Some(&f.cwd),
                |_| panic!("revealer reached for {href}")
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
            fake_browser,
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
        let result = with_local_source(true, || -> Result<(), String> {
            panic!("must not inspect roots or read session headers")
        });
        assert!(result.unwrap_err().contains("remote"));
        // Local preview is a frontend concept, not an additional backend gate.
        assert_eq!(with_local_source(false, || Ok("local")).unwrap(), "local");
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
            f.expect(&name, &path); // Markdown association; other source text uses -t.
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
            || panic!("source must not consult default browser"),
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

    #[cfg(target_os = "macos")]
    #[test]
    fn html_uses_configured_safari_or_chrome_never_text_or_hardcoded_browser() {
        let f = Fixture::new();
        for name in ["page.html", "page.HTM"] {
            let html = f.file(name);
            for app in ["/Applications/Safari.app", "/Applications/Fake Chrome.app"] {
                let result = open_with(
                    name,
                    f.session.to_str().unwrap(),
                    &f.roots,
                    None,
                    || Ok(PathBuf::from(app)),
                    |command| {
                        assert_eq!(command.get_program(), "/usr/bin/open");
                        assert_eq!(
                            command.get_args().collect::<Vec<_>>(),
                            vec![
                                std::ffi::OsStr::new("-a"),
                                std::ffi::OsStr::new(app),
                                std::ffi::OsStr::new("--"),
                                html.as_os_str(),
                            ]
                        );
                        assert!(command.get_current_dir().is_none());
                        Ok(()) // Inspect only, regardless of installed browsers.
                    },
                )
                .unwrap();
                assert_eq!(Path::new(&result), html.canonicalize().unwrap());
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn browser_lookup_failure_is_actionable_and_never_launches_a_fallback() {
        let f = Fixture::new();
        f.file("page.html");
        for lookup in [
            Err("fake missing HTTPS handler".into()),
            Ok(PathBuf::from("relative.app")),
        ] {
            let error = open_with(
                "page.html",
                f.session.to_str().unwrap(),
                &f.roots,
                None,
                || lookup,
                |_| panic!("must not launch after failed browser lookup"),
            )
            .unwrap_err();
            assert!(error.contains("configured default browser"));
            assert!(error.contains("System Settings"));
            assert!(
                error.contains("fake missing HTTPS handler")
                    || error.contains("invalid application path")
            );
        }
    }

    #[test]
    fn html_and_relative_assets_stay_at_the_original_location() {
        let f = Fixture::new();
        let folder = f.cwd.join("original HTML 中文");
        std::fs::create_dir(&folder).unwrap();
        std::fs::create_dir(folder.join("images")).unwrap();
        let html = folder.join("-page 'quoted'.html");
        let body = "<html><img src=\"images/picture.png\"><link href=\"style.css\"></html>";
        std::fs::write(&html, body).unwrap();
        let image = folder.join("images/picture.png");
        std::fs::write(&image, b"synthetic image").unwrap();
        std::fs::write(folder.join("style.css"), "synthetic stylesheet").unwrap();
        open_with(
            html.to_str().unwrap(),
            f.session.to_str().unwrap(),
            &f.roots,
            None,
            fake_browser,
            |command| {
                let args = command.get_args().collect::<Vec<_>>();
                assert_eq!(*args.last().unwrap(), html.as_os_str());
                assert_eq!(
                    Path::new(args.last().unwrap()).parent(),
                    Some(folder.as_path())
                );
                assert!(command.get_current_dir().is_none());
                Ok(())
            },
        )
        .unwrap();
        // Only tests read these synthetic contents; runtime uses metadata only.
        assert_eq!(std::fs::read_to_string(&html).unwrap(), body);
        assert_eq!(std::fs::read(&image).unwrap(), b"synthetic image");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn javascript_python_and_shell_remain_forced_text_without_browser_lookup() {
        let f = Fixture::new();
        for name in ["source.js", "source.py", "source.sh"] {
            let source = f.file(name).canonicalize().unwrap();
            open_with(
                name,
                f.session.to_str().unwrap(),
                &f.roots,
                None,
                || panic!("text source must not query the browser"),
                |command| {
                    assert_eq!(
                        command.get_args().collect::<Vec<_>>(),
                        vec![
                            std::ffi::OsStr::new("-t"),
                            std::ffi::OsStr::new("--"),
                            source.as_os_str(),
                        ]
                    );
                    Ok(())
                },
            )
            .unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn html_alias_routes_follow_real_type_and_real_html_asset_parent() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let f = Fixture::new();
        let folder = f.cwd.join("real HTML");
        std::fs::create_dir(&folder).unwrap();
        let html = folder.join("page.html");
        std::fs::write(&html, "synthetic HTML").unwrap();
        let canonical = html.canonicalize().unwrap();
        for alias in ["html-alias.md", "html-alias.htm"] {
            symlink(&html, f.cwd.join(alias)).unwrap();
            f.expect(alias, &html); // Fixture verifies browser route on macOS.
            let target = validate_target(&f.cwd.join(alias)).unwrap();
            assert!(target.html);
            assert_eq!(target.opener_path, canonical);
        }
        let script = f.file("source.py");
        symlink(&script, f.cwd.join("script-alias.html")).unwrap();
        #[cfg(target_os = "macos")]
        f.expect("script-alias.html", &script); // Still -t, never browser.
        #[cfg(not(target_os = "macos"))]
        assert!(f.open("script-alias.html").is_err());
        let program = f.file("unsafe.command");
        symlink(program, f.cwd.join("unsafe-alias.html")).unwrap();
        assert!(f.open("unsafe-alias.html").is_err());
        std::fs::set_permissions(&html, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(f.open("html-alias.md").is_err());
        assert!(f.open(html.to_str().unwrap()).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn markdown_uses_its_system_association_without_text_editor_or_browser_override() {
        let f = Fixture::new();
        for name in ["README.md", "guide.markdown", "page.mdx", "UPPER.MD"] {
            let canonical = f.file(name).canonicalize().unwrap();
            let result = open_with(
                name,
                f.session.to_str().unwrap(),
                &f.roots,
                None,
                || panic!("Markdown must not consult the browser"),
                |command| {
                    assert_eq!(command.get_program(), "/usr/bin/open");
                    assert_eq!(
                        command.get_args().collect::<Vec<_>>(),
                        vec![std::ffi::OsStr::new("--"), canonical.as_os_str()]
                    );
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(Path::new(&result), canonical);
        }
        // A bare README is still plain text, not a promised Markdown association.
        let readme = f.file("README");
        f.expect("README", &readme);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn markdown_executable_and_script_alias_guards_stay_forced_text() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let f = Fixture::new();
        let markdown = f.file("executable.mdx");
        std::fs::set_permissions(&markdown, std::fs::Permissions::from_mode(0o755)).unwrap();
        f.expect("executable.mdx", &markdown); // Fixture checks -t for executable Markdown.
        let script = f.file("real.py"); // Even a non-executable Python target must use -t.
        symlink(&script, f.cwd.join("script-alias.md")).unwrap();
        f.expect("script-alias.md", &script);
        let markdown = f.file("ordinary.md").canonicalize().unwrap();
        symlink(&markdown, f.cwd.join("reverse-alias.py")).unwrap();
        open_with(
            "reverse-alias.py",
            f.session.to_str().unwrap(),
            &f.roots,
            None,
            || panic!("script alias must not consult the browser"),
            |command| {
                assert_eq!(command.get_program(), "/usr/bin/open");
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    vec![
                        std::ffi::OsStr::new("-t"),
                        std::ffi::OsStr::new("--"),
                        markdown.as_os_str(),
                    ]
                );
                Ok(())
            },
        )
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reveal_uses_literal_canonical_finder_selection_for_files_and_directories() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let f = Fixture::new();
        let name = "-中文 'quoted'; $(touch SHOULD_NOT_EXIST).md";
        let file = f.file(name);
        for href in [
            name.to_owned(),
            format!("./{name}"),
            format!("~/{name}"),
            file.to_str().unwrap().to_owned(),
        ] {
            f.expect_reveal(&href, &file);
        }
        let encoded = file
            .to_str()
            .unwrap()
            .replace(' ', "%20")
            .replace("中文", "%E4%B8%AD%E6%96%87");
        f.expect_reveal(&format!("file://localhost{encoded}#L12"), &file);
        let literal_percent = f.file("literal%20.md");
        f.expect_reveal("literal%2520.md", &literal_percent); // Exactly one decoding pass.
        f.expect_reveal(".", &f.cwd);
        let folder = f.cwd.join("Program.app");
        std::fs::create_dir(&folder).unwrap();
        f.expect_reveal("Program.app", &folder);
        for name in ["unknown.bin", "program.command", "script.py", "page.html"] {
            let file = f.file(name);
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            f.expect_reveal(name, &file); // No launch allowlist or browser lookup.
        }
        let program = f.cwd.join("program.command");
        symlink(&program, f.cwd.join("document-alias.md")).unwrap();
        f.expect_reveal("document-alias.md", &program); // Select real target, not alias.
        assert!(!f.cwd.join("SHOULD_NOT_EXIST").exists());
    }

    #[test]
    fn reveal_reuses_session_root_and_target_validation_before_any_command() {
        let f = Fixture::new();
        let outside = f.root.join("outside.jsonl");
        std::fs::write(&outside, "not a session header").unwrap();
        for session in [outside, f.root.join("nonexistent.jsonl")] {
            assert_eq!(
                reveal_with(
                    "anything",
                    session.to_str().unwrap(),
                    &f.roots,
                    None,
                    |_| panic!("outside session must not reach revealer"),
                )
                .unwrap_err(),
                "Session path is outside the local session roots"
            );
        }
        for href in [
            "absent.md",
            "file://server/a",
            "file://user@localhost/a",
            "file://localhost:3/a",
            "file:///a?q=1",
            "file:///a#other",
            "file:///a%00b",
            "file:///a%FF",
            "file:///a%ZZ",
            "file:relative",
            "file:////server/a",
            "file:///%5Cserver/a",
            "%2F%2Fserver/a",
            "%6Aavascript%3Aalert(1)",
        ] {
            assert!(reveal_with(
                href,
                f.session.to_str().unwrap(),
                &f.roots,
                None,
                |_| panic!("invalid target reached revealer: {href}"),
            )
            .is_err());
        }
        #[cfg(unix)]
        {
            let socket = f.cwd.join("socket.md");
            let _socket = std::os::unix::net::UnixDatagram::bind(&socket).unwrap();
            let control = f.file("control\nname.md");
            std::os::unix::fs::symlink(control, f.cwd.join("control-alias.md")).unwrap();
            std::os::unix::fs::symlink(f.cwd.join("missing"), f.cwd.join("dangling.md")).unwrap();
            for href in ["socket.md", "control-alias.md", "dangling.md"] {
                assert!(reveal_with(
                    href,
                    f.session.to_str().unwrap(),
                    &f.roots,
                    None,
                    |_| panic!("invalid target reached revealer: {href}"),
                )
                .is_err());
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reveal_fake_failure_and_native_exit_status_are_not_reported_as_success() {
        use std::os::unix::process::ExitStatusExt;
        let f = Fixture::new();
        f.file("notes.md");
        assert_eq!(
            reveal_with(
                "notes.md",
                f.session.to_str().unwrap(),
                &f.roots,
                None,
                |_| Err("fake Finder exit failure".into())
            )
            .unwrap_err(),
            "fake Finder exit failure"
        );
        // Exercise the shared wait-status check with synthetic output, no process.
        let output = |code, stderr: &[u8]| std::process::Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: Vec::new(),
            stderr: stderr.to_vec(),
        };
        assert!(check_opener_output(output(0, b"")).is_ok());
        let error = check_opener_output(output(7, b"synthetic Finder failure\n")).unwrap_err();
        assert!(error.contains("7"));
        assert!(error.contains("synthetic Finder failure"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn finder_reveal_is_explicitly_unsupported_elsewhere() {
        let f = Fixture::new();
        f.file("notes.md");
        let error = reveal_with(
            "notes.md",
            f.session.to_str().unwrap(),
            &f.roots,
            None,
            |_| panic!("must not launch on unsupported platforms"),
        )
        .unwrap_err();
        assert!(error.contains("unsupported"));
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
            fake_browser,
            |_| Err("fake opener exit failure".into()),
        )
        .unwrap_err();
        assert_eq!(error, "fake opener exit failure");
    }
}
