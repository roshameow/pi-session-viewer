//! HTTP links stay HTTP. Finder reveal is a separate, fail-closed operation:
//! one session-header cwd, one contained candidate, and a bounded loopback GET.
use crate::{local_links, remote, sessions};
use std::io::Read;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAX_BODY: usize = 1_048_576;
const MAX_HEADERS: usize = 65_536;
const VITE_CLIENT: &str = "<script type=\"module\" src=\"/@vite/client\"></script>";

#[tauri::command]
pub async fn open_web_url(url: String) -> Result<(), String> {
    // Deliberately independent of session/source/local-file readiness.
    tauri::async_runtime::spawn_blocking(move || open_with(&url, local_links::run_opener))
        .await
        .map_err(|e| format!("Web opener task failed: {e}"))?
}

#[tauri::command]
pub async fn reveal_web_url(url: String, session_path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        remote::with_current_source(|| {
            // Reject before even constructing local roots or reading a session.
            let parsed = reveal_url(&url, remote::current_host().is_some())?;
            let roots = [
                sessions::sessions_dir(),
                sessions::pi_agent_dir().join("agent-logs"),
            ];
            reveal_with(
                &url,
                &parsed,
                &session_path,
                &roots,
                fetch_loopback,
                local_links::run_opener,
            )
        })
    })
    .await
    .map_err(|e| format!("Web reveal task failed: {e}"))?
}

fn validate_url(value: &str) -> Result<tauri::Url, String> {
    if value.is_empty()
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains('\\')
    {
        return Err("Web URL contains whitespace, controls or backslashes".into());
    }
    let (scheme, rest) = value.split_once("://").ok_or("Expected an HTTP(S) URL")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err("Only HTTP and HTTPS URLs are supported".into());
    }
    // Do not let WHATWG URL repair (e.g. http:///host, backslashes, empty
    // ports, percent-encoded authorities) turn malformed input into a URL.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains(['@', '%']) || authority.ends_with(':') {
        return Err("Web URL has an invalid host or credentials".into());
    }
    decode(value)?; // Validate escapes and reject encoded controls as well.
    let parsed = tauri::Url::parse(value).map_err(|_| "Malformed web URL")?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL credentials are not allowed".into());
    }
    let host = parsed.host_str().ok_or("Web URL has no host")?;
    let raw_host = if authority.starts_with('[') {
        let end = authority.find(']').ok_or("Invalid IPv6 host")?;
        &authority[1..end]
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok() {
        // Reject ambiguous IPv4 shorthand, integer/hex/octal IP spellings.
        raw_host
            .parse::<IpAddr>()
            .map_err(|_| "Use an explicit IP address")?;
    } else if host.trim_end_matches('.').split('.').any(|label| {
        label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) || host.len() > 253
    {
        return Err("Malformed web URL host".into());
    }
    Ok(parsed)
}

fn decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = |b: &u8| (*b as char).to_digit(16);
            let a = bytes.get(i + 1).and_then(hex).ok_or("Invalid URL escape")?;
            let b = bytes.get(i + 2).and_then(hex).ok_or("Invalid URL escape")?;
            out.push((a * 16 + b) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let result = String::from_utf8(out).map_err(|_| "URL is not valid UTF-8")?;
    if result.chars().any(char::is_control) {
        return Err("URL controls are not allowed".into());
    }
    Ok(result)
}

fn open_with(
    value: &str,
    opener: impl FnOnce(Command) -> Result<(), String>,
) -> Result<(), String> {
    validate_url(value)?;
    opener(browser_command(value)?)
}

fn browser_command(value: &str) -> Result<Command, String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/open");
        command.arg("--").arg(value); // Original URL, including a real trailing slash.
        Ok(command)
    }
    #[cfg(target_os = "linux")]
    {
        let mut command = Command::new("xdg-open");
        // Validated http(s) scheme cannot be an option. xdg-open has no --.
        command.arg(value);
        Ok(command)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = value;
        Err("Web opening is unsupported on this platform".into())
    }
}

fn reveal_url(value: &str, is_remote: bool) -> Result<tauri::Url, String> {
    if is_remote {
        return Err("Web file reveal is unavailable for remote sessions".into());
    }
    let parsed = validate_url(value)?;
    let host = parsed.host_str().ok_or("Web URL has no host")?;
    // The HTTP tool receives the original URL. Do not allow IDNA repair to
    // validate one host while curl might resolve a different original spelling.
    let authority = value
        .split_once("://")
        .ok_or("Invalid URL")?
        .1
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if host == "localhost"
        && !authority
            .split(':')
            .next()
            .unwrap_or("")
            .eq_ignore_ascii_case("localhost")
    {
        return Err("Use the literal localhost hostname for file verification".into());
    }
    if host != "localhost"
        && !host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    {
        return Err("File reveal is available only for loopback URLs".into());
    }
    if parsed.scheme() != "http" {
        return Err("HTTPS loopback file verification is unavailable".into());
    }
    // Queries may select dynamic/credentialed content; do not guess a mapping
    // or transmit query secrets just to select a local file.
    if parsed.query().is_some() {
        return Err("File reveal is unavailable for URLs with query parameters".into());
    }
    relative_candidate(value)?; // Check raw path BEFORE URL dot normalization.
    Ok(parsed)
}

fn relative_candidate(value: &str) -> Result<PathBuf, String> {
    let rest = value.split_once("://").ok_or("Invalid URL")?.1;
    let tail = rest.find(['/', '?', '#']).map_or("", |i| &rest[i..]);
    let raw = tail.split(['?', '#']).next().unwrap_or("");
    let raw = if raw.is_empty() { "/" } else { raw };
    let lower = raw.to_ascii_lowercase();
    if lower.contains("%2f") || lower.contains("%5c") {
        return Err("Encoded path separators are not allowed".into());
    }
    let decoded = decode(raw)?;
    let path = decoded
        .strip_prefix('/')
        .ok_or("Expected absolute URL path")?;
    let parts: Vec<_> = path.trim_end_matches('/').split('/').collect();
    if decoded.contains('\\')
        || decoded.starts_with("//")
        || decoded.contains("//")
        || parts
            .iter()
            .any(|p| *p == "." || *p == ".." || p.contains(':'))
        || parts
            .first()
            .is_some_and(|p| p.starts_with('@') || *p == "node_modules")
    {
        return Err("Unsafe or server-internal URL path".into());
    }
    let mut relative = PathBuf::from(path);
    if decoded.ends_with('/') {
        relative.push("index.html");
    }
    Ok(relative)
}

fn reveal_with(
    value: &str,
    parsed: &tauri::Url,
    session_path: &str,
    roots: &[PathBuf],
    fetch: impl FnOnce(&str, &tauri::Url) -> Result<Vec<u8>, String>,
    opener: impl FnOnce(Command) -> Result<(), String>,
) -> Result<String, String> {
    let candidate = verified_candidate(value, parsed, session_path, roots, fetch)?;
    opener(local_links::reveal_command(&candidate)?)?;
    candidate
        .to_str()
        .map(str::to_owned)
        .ok_or("Project path is not UTF-8".into())
}

// Shared by reveal and the explicitly enabled read-only diagnostic. No native
// opener is constructed here, so verification alone cannot launch anything.
fn verified_candidate(
    value: &str,
    parsed: &tauri::Url,
    session_path: &str,
    roots: &[PathBuf],
    fetch: impl FnOnce(&str, &tauri::Url) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    let relative = relative_candidate(value)?;
    let cwd = local_links::session_cwd(session_path, roots)?
        .canonicalize()
        .map_err(|_| "Session project directory is unavailable")?;
    let candidate = cwd
        .join(relative)
        .canonicalize()
        .map_err(|_| "No corresponding project file is available")?;
    if !candidate.starts_with(&cwd) || !candidate.is_file() {
        return Err("Web file candidate is not a regular file within the session project".into());
    }
    let local = read_bounded(
        std::fs::File::open(&candidate).map_err(|_| "Cannot read corresponding project file")?,
        MAX_BODY,
    )?;
    let served = fetch(value, parsed)?;
    if served.len() > MAX_BODY || !content_matches(&candidate, &local, &served) {
        return Err(
            "File reveal unavailable: served content does not match the session project file"
                .into(),
        );
    }
    Ok(candidate)
}

fn content_matches(path: &Path, local: &[u8], served: &[u8]) -> bool {
    if local.len() > MAX_BODY || served.len() > MAX_BODY {
        return false;
    }
    if local == served {
        return true;
    }
    if !path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
    {
        return false;
    }
    let (Ok(local), Ok(served)) = (std::str::from_utf8(local), std::str::from_utf8(served)) else {
        return false;
    };
    // No broad HTML parser, attribute normalization or arbitrary script
    // stripping. Keep all nonblank line bytes (including spaces). Any HMR
    // timestamp tolerance additionally requires the exact served Vite marker.
    fn normalize(value: &str) -> String {
        value
            .replace(VITE_CLIENT, "")
            .split('\n')
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
    let normalized_local = normalize(local);
    let normalized_served = normalize(served);
    if normalized_local == normalized_served {
        return true;
    }
    served.contains(VITE_CLIENT)
        && normalize_vite_timestamps(&normalized_local, &normalized_served)
            .is_some_and(|value| value == normalized_local)
}

/// Comparison only: accept Vite's sole ?t=<1..20 ASCII digits> insertion on
/// exact empty module tags whose unchanged relative src exists in local HTML.
/// Never rewrite the page, remove scripts, parse arbitrary attributes or relax
/// inline bodies. Inputs are <=1 MiB and each side is limited to 64 module tags.
fn normalize_vite_timestamps(local: &str, served: &str) -> Option<String> {
    const PREFIX: &str = "<script type=\"module\" src=\"";
    const SUFFIX: &str = "\"></script>";
    const MAX_MODULES: usize = 64;

    fn modules(html: &str) -> Option<Vec<(usize, &str)>> {
        let mut result = Vec::new();
        for (count, (start, _)) in html.match_indices(PREFIX).enumerate() {
            if count == MAX_MODULES {
                return None;
            }
            let tail = &html[start + PREFIX.len()..];
            let Some(end) = tail.find('"') else { continue };
            if tail[end..].starts_with(SUFFIX) {
                result.push((start, &tail[..end]));
            }
        }
        Some(result)
    }
    let local_modules = modules(local)?;
    let relative_sources: Vec<_> = local_modules
        .iter()
        .map(|(_, src)| *src)
        .filter(|src| {
            !src.is_empty()
                && !src.starts_with('/')
                && !src.contains(['?', '#', ':', '\\', '<', '>', '\''])
                && !src.chars().any(|c| c.is_control() || c.is_whitespace())
        })
        .collect();
    let mut output = String::with_capacity(served.len());
    let mut cursor = 0;
    for (start, src) in modules(served)? {
        let Some((base, timestamp)) = src.split_once("?t=") else {
            continue;
        };
        if !(1..=20).contains(&timestamp.len())
            || !timestamp.bytes().all(|b| b.is_ascii_digit())
            || !relative_sources.contains(&base)
        {
            continue;
        }
        output.push_str(&served[cursor..start]);
        output.push_str(PREFIX);
        output.push_str(base);
        output.push_str(SUFFIX);
        cursor = start + PREFIX.len() + src.len() + SUFFIX.len();
    }
    output.push_str(&served[cursor..]);
    Some(output)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read verification content: {e}"))?;
    if bytes.len() > limit {
        return Err("File verification exceeds the 1 MiB limit".into());
    }
    Ok(bytes)
}

fn curl_command(value: &str, parsed: &tauri::Url) -> Command {
    let mut command = Command::new("/usr/bin/curl");
    // --disable MUST be first: a user curlrc could otherwise add redirects,
    // credentials, proxies, output files or unrelated requests.
    command.args([
        "--disable",
        "--noproxy",
        "*",
        "--proxy",
        "",
        "--connect-timeout",
        "1",
        "--max-time",
        "3",
        "--max-filesize",
        "1048576",
        "--globoff",
        "--fail",
        "--silent",
        "--show-error",
        "--proto",
        "=http",
        "--http1.1",
        "--include",
    ]);
    if parsed.host_str() == Some("localhost") {
        // Pin localhost instead of trusting DNS/hosts resolution or rebinding.
        command.arg("--resolve").arg(format!(
            "localhost:{}:127.0.0.1",
            parsed.port_or_known_default().unwrap_or(80)
        ));
    }
    command.arg("--").arg(value);
    command
}

fn fetch_loopback(value: &str, parsed: &tauri::Url) -> Result<Vec<u8>, String> {
    let mut child = curl_command(value, parsed)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Discard directly rather than piping/buffering stderr: no drain thread
        // or unbounded diagnostic buffer can block this bounded GET.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Loopback verification unavailable: {e}"))?;
    let result = read_http_output(child.stdout.take().ok_or("Missing verification output")?);
    if result.is_err() {
        // Bounds cover unknown Content-Length too; never use Command::output
        // (which buffers unbounded data) for an untrusted local server.
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|e| format!("Verification failed: {e}"))?;
    if !status.success() {
        return Err("Loopback HTTP verification failed or exceeded size/time limits".into());
    }
    result
}

fn read_http_output(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut body_start = None;
    loop {
        let limit = body_start.map_or(MAX_HEADERS, |start| start + MAX_BODY);
        let mut buf = [0; 8192];
        let amount = buf.len().min(limit + 1 - output.len());
        let n = reader
            .read(&mut buf[..amount])
            .map_err(|e| format!("HTTP verification read failed: {e}"))?;
        if n == 0 {
            break;
        }
        output.extend_from_slice(&buf[..n]);
        if body_start.is_none() {
            body_start = output
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|i| i + 4);
            if body_start.is_some_and(|start| start > MAX_HEADERS) {
                return Err("Verification response headers are too large".into());
            }
            if let Some(start) = body_start {
                let status = output[..start]
                    .split(|b| *b == b'\n')
                    .next()
                    .unwrap_or_default();
                if !status.starts_with(b"HTTP/1.1 200 ") && !status.starts_with(b"HTTP/1.0 200 ") {
                    return Err("Verification requires HTTP 200; redirects are not followed".into());
                }
            }
        }
        let limit = body_start.map_or(MAX_HEADERS, |start| start + MAX_BODY);
        if output.len() > limit {
            return Err("HTTP verification response exceeds size limits".into());
        }
    }
    let start = body_start.ok_or("Incomplete verification response")?;
    Ok(output.split_off(start))
}

#[cfg(test)]
mod web_links_tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    const CASE: &str = "http://127.0.0.1:5194/synthetic/model/viewer/";

    struct Fixture {
        base: PathBuf,
        cwd: PathBuf,
        roots: [PathBuf; 1],
        session: String,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let base = std::env::temp_dir().join(format!(
                "viewer-web-links-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let cwd = base.join("true-project 中文");
            let roots = [base.join("sessions-not-the-project")];
            std::fs::create_dir_all(&cwd).unwrap();
            std::fs::create_dir(&roots[0]).unwrap();
            let session = roots[0].join("fixture.jsonl");
            std::fs::write(
                &session,
                format!(
                    "{}\npoison transcript: not JSON\n",
                    serde_json::json!({"type":"session", "id":"web-fixture", "cwd":cwd})
                ),
            )
            .unwrap();
            Self {
                base,
                cwd,
                roots,
                session: session.to_str().unwrap().to_owned(),
            }
        }
        fn file(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.cwd.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            path.canonicalize().unwrap()
        }
        fn reveal(
            &self,
            value: &str,
            bytes: &[u8],
            opener: impl FnOnce(Command) -> Result<(), String>,
        ) -> Result<String, String> {
            let parsed = reveal_url(value, false)?;
            reveal_with(
                value,
                &parsed,
                &self.session,
                &self.roots,
                |_, _| Ok(bytes.to_vec()),
                opener,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn browser_preserves_http_url_and_all_original_components() {
        for value in [
            CASE,
            "https://example.com:8443/a%20b/?value=%2F&x=1#part",
            "http://localhost:5194/empty?x=1#f",
        ] {
            open_with(value, |command| {
                let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
                #[cfg(target_os = "macos")]
                {
                    assert_eq!(command.get_program(), "/usr/bin/open");
                    assert_eq!(args, ["--", value]);
                }
                #[cfg(target_os = "linux")]
                {
                    assert_eq!(command.get_program(), "xdg-open");
                    assert_eq!(args, [value]);
                }
                assert!(!args.iter().any(|a| a.starts_with("file:")));
                Ok(())
            })
            .unwrap();
        }
    }

    #[test]
    fn invalid_urls_never_invoke_opener() {
        for value in [
            "",
            "file:///tmp/a.html",
            "javascript:alert(1)",
            "data:text/html,x",
            "ftp://localhost/a",
            "--help",
            "//example.com/",
            "http:///example.com/",
            "http://",
            "http://bad host/",
            "http://bad_host/",
            "http://a..b/",
            "http://-host/",
            "http://host:/",
            "http://host:bad/",
            "http://[bad]/",
            "http://user:pass@host/",
            "http://@host/",
            "http://host/a\n",
            "http://host/\0",
            "http://host/%00",
            "http://host/%0D",
            "http://host/%C2%85",
            "http://host/%zz",
            "http://host/\\path",
            "http://127.1/",
            "http://2130706433/",
            "http://0x7f000001/",
        ] {
            assert!(
                open_with(value, |_| panic!("must not open {value:?}")).is_err(),
                "{value:?}"
            );
        }
        assert!(open_with(CASE, |_| Err("fake failed status".into()))
            .unwrap_err()
            .contains("fake failed status"));
    }

    #[test]
    fn reveal_only_accepts_plain_http_loopback_current_local_source() {
        for value in [
            CASE,
            "http://localhost:123/a",
            "http://[::1]:123/a",
            "http://127.0.0.2/a",
        ] {
            assert!(reveal_url(value, false).is_ok(), "{value}");
        }
        for value in [
            "http://example.com/a",
            "http://127.0.0.1.example.com/a",
            "http://localhost.example/a",
            "http://ｌｏｃａｌｈｏｓｔ/a",
            "http://192.168.1.1/a",
            "http://[::ffff:127.0.0.1]/a",
            "http://[::]/a",
            "https://localhost/a",
            "http://localhost/a?token=secret",
            "http://user@localhost/a",
        ] {
            assert!(reveal_url(value, false).is_err(), "{value}");
        }
        // Source rejection wins over parsing, before roots or session reads.
        assert!(reveal_url("invalid", true)
            .unwrap_err()
            .contains("remote sessions"));
        assert!(reveal_url(CASE, true)
            .unwrap_err()
            .contains("remote sessions"));
    }

    #[test]
    fn path_checks_precede_url_dot_normalization() {
        for path in [
            "/../index.html",
            "/x/../index.html",
            "/./index.html",
            "/%2e%2e/index.html",
            "/x/%2E./index.html",
            "/x%2fy/index.html",
            "/x%5cy/index.html",
            "//tmp/index.html",
            "/x//index.html",
            "/@fs/tmp/index.html",
            "/%40fs/tmp/index.html",
            "/@vite/client",
            "/node_modules/a",
            "/C:/index.html",
            "/%00.html",
        ] {
            assert!(
                reveal_url(&format!("http://127.0.0.1:123{path}"), false).is_err(),
                "{path}"
            );
        }
        assert_eq!(
            relative_candidate(CASE).unwrap(),
            PathBuf::from("synthetic/model/viewer/index.html")
        );
        assert_eq!(
            relative_candidate("http://localhost/a%2520b.html").unwrap(),
            PathBuf::from("a%20b.html")
        );
        assert_eq!(
            relative_candidate("http://localhost/a%20b.html#L3").unwrap(),
            PathBuf::from("a b.html")
        );
        assert_eq!(
            relative_candidate("http://localhost/").unwrap(),
            PathBuf::from("index.html")
        );
        assert_eq!(
            relative_candidate("http://localhost/a").unwrap(),
            PathBuf::from("a")
        ); // no invented slash/index
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn synthetic_case_maps_session_header_cwd_and_native_reveal_argv() {
        let fixture = Fixture::new();
        let local =
            b"<!doctype html>\n<head>\n<title>Viewer</title>\n</head>\n<body>model</body>\n";
        let target = fixture.file("synthetic/model/viewer/index.html", local);
        let served = String::from_utf8(local.to_vec())
            .unwrap()
            .replace("<head>\n", &format!("<head>\n  {VITE_CLIENT}\n\n"));
        let result = fixture
            .reveal(CASE, served.as_bytes(), |command| {
                assert_eq!(command.get_program(), "/usr/bin/open");
                assert_eq!(
                    command.get_args().collect::<Vec<_>>(),
                    [
                        std::ffi::OsStr::new("-R"),
                        std::ffi::OsStr::new("--"),
                        target.as_os_str()
                    ]
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(Path::new(&result), target);
        assert!(fixture
            .reveal(
                CASE,
                served.as_bytes(),
                |_| Err("fake reveal status".into())
            )
            .unwrap_err()
            .contains("fake reveal status"));
    }

    #[test]
    fn missing_mismatch_and_wrong_project_never_reveal() {
        let fixture = Fixture::new();
        let other = Fixture::new();
        other.file("report/index.html", b"served from other project");
        fixture.file("report/index.html", b"unrelated project content");
        assert!(fixture
            .reveal(
                "http://localhost/report/",
                b"served from other project",
                |_| panic!("no reveal")
            )
            .unwrap_err()
            .contains("does not match"));
        let value = "http://localhost/missing/";
        let parsed = reveal_url(value, false).unwrap();
        assert!(reveal_with(
            value,
            &parsed,
            &fixture.session,
            &fixture.roots,
            |_, _| panic!("no fetch for missing"),
            |_| panic!("no reveal")
        )
        .is_err());
        assert!(fixture
            .reveal("http://example.com/report/", b"", |_| panic!("remote URL"))
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn candidate_symlink_may_not_escape_cwd() {
        let fixture = Fixture::new();
        let other = Fixture::new();
        let outside = other.file("outside.html", b"identical");
        std::os::unix::fs::symlink(outside, fixture.cwd.join("alias.html")).unwrap();
        let value = "http://localhost/alias.html";
        let parsed = reveal_url(value, false).unwrap();
        assert!(reveal_with(
            value,
            &parsed,
            &fixture.session,
            &fixture.roots,
            |_, _| panic!("no outside fetch"),
            |_| panic!("no reveal")
        )
        .unwrap_err()
        .contains("within the session project"));
    }

    #[test]
    fn html_only_known_vite_injection_and_blank_lines_are_normalized() {
        let local = b"<head>\n<title>x</title>\n</head>";
        let served = format!("<head>\n  {VITE_CLIENT}\n\n<title>x</title>\n</head>\n");
        assert!(content_matches(
            Path::new("a.html"),
            local,
            served.as_bytes()
        ));
        assert!(!content_matches(
            Path::new("a.bin"),
            local,
            served.as_bytes()
        ));
        assert!(!content_matches(
            Path::new("a.html"),
            local,
            b"<head>\n<title>y</title>\n</head>"
        ));
        assert!(!content_matches(
            Path::new("a.html"),
            local,
            &format!(
                "{}\n<script src=\"other.js\"></script>",
                String::from_utf8_lossy(local)
            )
            .into_bytes()
        ));
        assert!(!content_matches(
            Path::new("a.html"),
            local,
            b"<head>\n <title>x</title>\n</head>"
        ));
        assert!(content_matches(
            Path::new("model.glb"),
            b"\0glb\xff",
            b"\0glb\xff"
        ));
        assert!(!content_matches(
            Path::new("model.glb"),
            b"\0glb\xff",
            b"\0glb\xfe"
        ));
    }

    fn module_tag(src: &str) -> String {
        format!("<script type=\"module\" src=\"{src}\"></script>")
    }

    #[test]
    fn vite_hmr_timestamps_allow_only_matching_empty_relative_module_tags() {
        let local = format!(
            "<head>\n</head>\n{}\n{}\n{}\n<script>unchanged()</script>",
            module_tag("./main.js"),
            module_tag("../other.js"),
            module_tag("assets/third.js")
        );
        let served = format!(
            "<head>\n  {VITE_CLIENT}\n\n</head>\n{}\n{}\n{}\n<script>unchanged()</script>",
            module_tag("./main.js?t=1700000000001"),
            module_tag("../other.js?t=0"),
            module_tag("assets/third.js?t=12345678901234567890")
        );
        assert!(content_matches(
            Path::new("index.html"),
            local.as_bytes(),
            served.as_bytes()
        ));
        assert!(!content_matches(
            Path::new("index.txt"),
            local.as_bytes(),
            served.as_bytes()
        ));
        assert!(!content_matches(
            Path::new("index.html"),
            local.as_bytes(),
            served.replace(VITE_CLIENT, "").as_bytes()
        ));
        assert!(!content_matches(
            Path::new("index.html"),
            local.as_bytes(),
            served.replace("unchanged()", "changed()").as_bytes()
        ));
    }

    #[test]
    fn vite_hmr_rejects_other_queries_sources_attributes_and_bodies() {
        let local = module_tag("./main.js");
        for src in [
            "./main.js?t=",
            "./main.js?t=1&evil=1",
            "./main.js?t=1#evil",
            "./main.js?t=1?t=2",
            "./main.js?t=-1",
            "./main.js?t=1.0",
            "./main.js?t=１",
            "./main.js?t=123456789012345678901",
            "./main.js?evil=1",
            "./main.js?v=1",
            "./other.js?t=1",
            "/main.js?t=1",
            "https://example.com/main.js?t=1",
        ] {
            let served = format!("{VITE_CLIENT}\n{}", module_tag(src));
            assert!(
                !content_matches(Path::new("index.html"), local.as_bytes(), served.as_bytes()),
                "{src}"
            );
        }
        for tag in [
            "<script type=\"module\" src=\"./main.js?t=1\" async></script>",
            "<script src=\"./main.js?t=1\" type=\"module\"></script>",
            "<script type=\"module\" src=\"./main.js?t=1\">evil()</script>",
            "<script type=\"module\" src=\"./main.js?t=1\"> </script>",
            "<script type='module' src='./main.js?t=1'></script>",
        ] {
            let served = format!("{VITE_CLIENT}\n{tag}");
            assert!(
                !content_matches(Path::new("index.html"), local.as_bytes(), served.as_bytes()),
                "{tag}"
            );
        }
        // Even otherwise identical unknown attributes must not gain tolerance.
        let local = "<script type=\"module\" src=\"./main.js\" async></script>";
        let served = format!("{VITE_CLIENT}\n{}", local.replace("main.js", "main.js?t=1"));
        assert!(!content_matches(
            Path::new("index.html"),
            local.as_bytes(),
            served.as_bytes()
        ));
        for src in [
            "/main.js",
            "//example.com/main.js",
            "https://example.com/main.js",
            "./main.js?x=1",
            "./main.js#part",
            "./main.js?t=1",
        ] {
            let local = module_tag(src);
            let served = format!("{VITE_CLIENT}\n{}", module_tag(&format!("{src}?t=2")));
            assert!(
                !content_matches(Path::new("index.html"), local.as_bytes(), served.as_bytes()),
                "{src}"
            );
        }
    }

    #[test]
    fn vite_hmr_module_and_input_caps_fail_closed() {
        let tags = |count, stamp: &str| {
            (0..count)
                .map(|i| module_tag(&format!("./module-{i}.js{stamp}")))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let local = tags(64, "");
        let served = format!("{VITE_CLIENT}\n{}", tags(64, "?t=1"));
        assert!(content_matches(
            Path::new("index.html"),
            local.as_bytes(),
            served.as_bytes()
        ));
        let local = tags(65, "");
        let served = format!("{VITE_CLIENT}\n{}", tags(65, "?t=1"));
        assert!(!content_matches(
            Path::new("index.html"),
            local.as_bytes(),
            served.as_bytes()
        ));
        // Too many served tags or unrecognized extra modules also fail closed.
        assert!(!content_matches(
            Path::new("index.html"),
            tags(64, "").as_bytes(),
            served.as_bytes()
        ));
        let oversized = vec![b'x'; MAX_BODY + 1];
        assert!(!content_matches(
            Path::new("index.html"),
            &oversized,
            &oversized
        ));
    }

    #[test]
    fn local_and_response_reads_are_bounded_even_without_content_length() {
        assert!(read_bounded(Cursor::new(vec![0; MAX_BODY + 1]), MAX_BODY).is_err());
        assert_eq!(
            read_bounded(Cursor::new(vec![0; MAX_BODY]), MAX_BODY)
                .unwrap()
                .len(),
            MAX_BODY
        );
        let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        response.extend(vec![0; MAX_BODY + 1]);
        assert!(read_http_output(Cursor::new(response)).is_err());
        assert!(read_http_output(Cursor::new(vec![b'a'; MAX_HEADERS + 1])).is_err());
        assert!(read_http_output(Cursor::new(
            b"HTTP/1.1 302 Found\r\nLocation: http://example.com/\r\n\r\n".to_vec()
        ))
        .is_err());
    }

    #[test]
    fn curl_argv_disables_configs_proxies_redirects_and_pins_localhost() {
        let value = "http://localhost:5194/a/";
        let parsed = reveal_url(value, false).unwrap();
        let command = curl_command(value, &parsed);
        assert_eq!(command.get_program(), "/usr/bin/curl");
        let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(args[0], "--disable");
        for pair in [
            ["--noproxy", "*"],
            ["--proxy", ""],
            ["--connect-timeout", "1"],
            ["--max-time", "3"],
            ["--max-filesize", "1048576"],
            ["--resolve", "localhost:5194:127.0.0.1"],
        ] {
            assert!(args.windows(2).any(|w| w == pair));
        }
        assert!(args.ends_with(&["--", value]));
        assert!(!args.contains(&"--location") && !args.contains(&"-L"));
    }

    // Exactly one disposable loopback connection; no development servers, real
    // sessions, user projects, browsers or Finder are touched by these tests.
    fn server(response: Vec<u8>, pause: Duration) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/report/", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("disposable listener: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            let n = stream.read(&mut request).unwrap();
            assert!(request[..n].starts_with(b"GET /report/ HTTP/1.1\r\n"));
            thread::sleep(pause);
            let _ = stream.write_all(&response); // oversized/timebound clients may close early
        });
        (url, handle)
    }

    /// Main may explicitly run this ignored diagnostic with both environment
    /// variables set to the authorized case. Never prints body/session contents,
    /// starts a server, constructs an opener, or launches Finder/browser.
    #[test]
    #[ignore = "manual read-only mapping diagnostic; requires explicitly supplied session and URL"]
    fn manual_web_link_mapping_diagnostic() -> Result<(), String> {
        let (Ok(session), Ok(value)) = (
            std::env::var("PI_VIEWER_WEB_LINK_DIAGNOSTIC_SESSION"),
            std::env::var("PI_VIEWER_WEB_LINK_DIAGNOSTIC_URL"),
        ) else {
            return Ok(()); // Unconfigured diagnostic is a no-op.
        };
        remote::with_current_source(|| {
            let parsed = reveal_url(&value, remote::current_host().is_some())?;
            let roots = [
                sessions::sessions_dir(),
                sessions::pi_agent_dir().join("agent-logs"),
            ];
            let candidate = verified_candidate(&value, &parsed, &session, &roots, fetch_loopback)?;
            println!("{}", candidate.display());
            Ok(())
        })
    }

    #[test]
    fn disposable_http_fetch_handles_chunked_vite_html() {
        let body = format!("<head>\n  {VITE_CLIENT}\n\n</head>");
        let response = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
        let (value, handle) = server(response.into_bytes(), Duration::ZERO);
        let parsed = reveal_url(&value, false).unwrap();
        assert_eq!(fetch_loopback(&value, &parsed).unwrap(), body.as_bytes());
        handle.join().unwrap();
    }

    #[test]
    fn diagnostic_verification_path_matches_fixture_over_real_loopback_get() {
        let fixture = Fixture::new();
        let target = fixture.file("report/index.html", b"<head>\n</head>");
        let body = format!("<head>\n  {VITE_CLIENT}\n\n</head>");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (value, handle) = server(response.into_bytes(), Duration::ZERO);
        let parsed = reveal_url(&value, false).unwrap();
        // The same function used by the manual diagnostic has no opener at all.
        let result = verified_candidate(
            &value,
            &parsed,
            &fixture.session,
            &fixture.roots,
            fetch_loopback,
        );
        handle.join().unwrap();
        assert_eq!(result.unwrap(), target);
    }

    #[test]
    fn disposable_http_redirect_is_refused_not_followed() {
        let forbidden = TcpListener::bind("127.0.0.1:0").unwrap();
        forbidden.set_nonblocking(true).unwrap();
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{}/outside\r\nContent-Length: 0\r\n\r\n",
            forbidden.local_addr().unwrap()
        );
        let (value, handle) = server(response.into_bytes(), Duration::ZERO);
        let parsed = reveal_url(&value, false).unwrap();
        assert!(fetch_loopback(&value, &parsed).is_err());
        handle.join().unwrap();
        assert!(matches!(forbidden.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    }

    #[test]
    fn disposable_http_unknown_length_overflow_is_refused() {
        let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        response.extend(vec![b'x'; MAX_BODY + 4096]);
        let (value, handle) = server(response, Duration::ZERO);
        let parsed = reveal_url(&value, false).unwrap();
        let result = fetch_loopback(&value, &parsed);
        assert!(
            result.is_err(),
            "unexpected body size: {:?}",
            result.as_ref().map(Vec::len)
        );
        handle.join().unwrap();
    }

    #[test]
    fn disposable_http_timeout_is_bounded() {
        let (value, handle) = server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            Duration::from_millis(3300),
        );
        let parsed = reveal_url(&value, false).unwrap();
        let start = Instant::now();
        assert!(fetch_loopback(&value, &parsed).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
        handle.join().unwrap();
    }
}
