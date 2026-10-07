//! Pure synthetic regressions; no session discovery or process probes.
use super::*;
use std::cell::Cell;
use std::io::{Cursor, Read};

// Golden reference: 08bbc1d scan_head, only File::open replaced by a reader.
fn legacy_scan_head(reader: impl std::io::Read) -> HeadMeta {
    let mut id = String::new();
    let mut cwd = String::new();
    let mut created = String::new();
    let mut name = None;
    let mut first_msg = None;
    let mut model = None;

    // bounded read: session headers live in the first few lines; reading the
    // whole file here made list_sessions O(file-size) for every session
    let mut reader = reader;
    let mut buf = vec![0u8; 256 * 1024];
    let data = match reader.read(&mut buf) {
        Ok(n) => buf[..n].to_vec(),
        Err(_) => return (id, cwd, created, name, first_msg, model),
    };
    let text = String::from_utf8_lossy(&data);
    for (i, line) in text.lines().enumerate() {
        // stop early once we have the essentials: headers are the first lines
        if i > 40
            || (!id.is_empty() && !cwd.is_empty() && !created.is_empty() && first_msg.is_some())
        {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "session" => {
                id = v
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                cwd = v
                    .get("cwd")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                created = v
                    .get("timestamp")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
            }
            "session_info" => {
                name = v
                    .get("name")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
            }
            "message" => {
                if let Some(m) = v.get("message") {
                    let role = m.get("role").and_then(|x| x.as_str()).unwrap_or("");
                    if role == "user" {
                        if first_msg.is_none() {
                            first_msg = text_of_message(m, 140);
                        }
                    } else if role == "assistant" && model.is_none() {
                        model = m
                            .get("model")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
            "model_change" if model.is_none() => {
                model = v
                    .get("modelId")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string());
            }
            _ => {}
        }
    }
    (id, cwd, created, name, first_msg, model)
}

struct Counted<'a> {
    input: Cursor<&'a [u8]>,
    bytes: &'a Cell<usize>,
}
impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.input.read(buf)?;
        self.bytes.set(self.bytes.get() + n);
        Ok(n)
    }
}

const HEADER: &str =
    r#"{"type":"session","id":"fixture","cwd":"/synthetic","timestamp":"2026-01-01T00:00:00Z"}"#;
const USER: &str =
    r#"{"type":"message","message":{"role":"user","content":"first synthetic prompt"}}"#;
const NAME: &str = r#"{"type":"session_info","name":"Synthetic name"}"#;
const MODEL: &str = r#"{"type":"model_change","modelId":"synthetic-model"}"#;
const ASSISTANT: &str = r#"{"type":"message","message":{"role":"assistant","model":"assistant-model","content":"synthetic answer"}}"#;

fn same(data: &[u8]) {
    assert_eq!(
        scan_head_reader(Cursor::new(data)),
        legacy_scan_head(Cursor::new(data))
    );
}

#[test]
fn streaming_head_golden_preserves_fields_and_early_stop() {
    let data = format!("{HEADER}\n{NAME}\n{MODEL}\n{ASSISTANT}\n{USER}\n");
    same(data.as_bytes());
    let head = scan_head_reader(Cursor::new(data.as_bytes()));
    assert_eq!(head.0, "fixture");
    assert_eq!(head.3.as_deref(), Some("Synthetic name"));
    assert_eq!(head.4.as_deref(), Some("first synthetic prompt"));
    assert_eq!(head.5.as_deref(), Some("synthetic-model"));
    // The old parser deliberately ignores name/model rows after essentials.
    let after = format!("{HEADER}\n{USER}\n{NAME}\n{MODEL}\n");
    same(after.as_bytes());
    let head = scan_head_reader(Cursor::new(after.as_bytes()));
    assert!(head.3.is_none() && head.5.is_none());
    same(format!("{ASSISTANT}\n{HEADER}\n{USER}\n").as_bytes());
    same(format!("{MODEL}\n{MODEL}\n{HEADER}\n{USER}\n").as_bytes());
}

#[test]
fn streaming_head_golden_truncation_crlf_and_lossy_utf8() {
    let data = format!("\nmalformed\n{HEADER}\n{NAME}\n{MODEL}\n{USER}\n");
    for n in 0..=data.len() {
        same(&data.as_bytes()[..n]);
    }
    same(data.replace('\n', "\r\n").as_bytes());
    same(format!("{HEADER}\n{USER}").as_bytes());
    same(format!("{HEADER}\n{USER}\r").as_bytes());
    let mut invalid =
        format!("{HEADER}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"")
            .into_bytes();
    invalid.extend_from_slice(b"bad-\xff-utf8\"}}\n");
    same(&invalid);
    let unicode = format!("{HEADER}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"漢字😀\"}}}}\n");
    for n in 0..=unicode.len() {
        same(&unicode.as_bytes()[..n]);
    }
}

#[test]
fn streaming_head_golden_enforces_row_and_byte_caps() {
    for blanks in [39, 40, 41, 100] {
        same(format!("{}{HEADER}\n{USER}\n", "\n".repeat(blanks)).as_bytes());
    }
    // Missing essentials must not cause reads beyond the original caps.
    let long = format!("{HEADER}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"{}\"}}}}\n{MODEL}\n", "x".repeat(300 * 1024));
    let bytes = Cell::new(0);
    let head = scan_head_reader(Counted {
        input: Cursor::new(long.as_bytes()),
        bytes: &bytes,
    });
    assert_eq!(head, legacy_scan_head(Cursor::new(long.as_bytes())));
    assert_eq!(bytes.get(), 256 * 1024);
    assert!(head.4.is_none());
    // A complete JSON row ending exactly at the byte cap remains parseable.
    let mut exact = vec![b' '; 256 * 1024 - HEADER.len()];
    exact.extend_from_slice(HEADER.as_bytes());
    same(&exact);
    assert_eq!(scan_head_reader(Cursor::new(&exact)).0, "fixture");
    exact.extend_from_slice(format!("\n{USER}\n").as_bytes());
    same(&exact);
    // Multi-buffer lines below the cap preserve previews/model/name.
    let within = format!("{HEADER}\n{NAME}\n{MODEL}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"{}\"}}}}\n", "x".repeat(150 * 1024));
    same(within.as_bytes());
}

#[test]
fn streaming_head_small_header_large_body_reads_one_small_buffer() {
    let mut data = format!("{HEADER}\n{NAME}\n{MODEL}\n{USER}\n").into_bytes();
    data.extend(vec![b'x'; 2 * 1024 * 1024]);
    let old_bytes = Cell::new(0);
    let new_bytes = Cell::new(0);
    let old = legacy_scan_head(Counted {
        input: Cursor::new(&data),
        bytes: &old_bytes,
    });
    let new = scan_head_reader(Counted {
        input: Cursor::new(&data),
        bytes: &new_bytes,
    });
    assert_eq!(old, new);
    assert_eq!(old_bytes.get(), 256 * 1024);
    assert_eq!(new_bytes.get(), 8 * 1024);
}

fn slot(pid: u32, pane_pid: Option<u32>, path: &str) -> RuntimeEntry {
    RuntimeEntry {
        pid,
        pane_pid,
        session_path: path.into(),
        cwd: "/synthetic".into(),
        tty: "ttys-source".into(),
    }
}

#[test]
fn runtime_batch_requires_each_pid_and_preserves_private_sdk_owners() {
    let parsed = vec![
        (1, 0, slot(1, None, "legacy-owner")),
        (2, 0, slot(2, None, "dead-missing")),
        (3, 1000, slot(3, None, "reused")),
        (4, 1000, slot(4, Some(400), "sdk-owner")),
        (5, 1000, slot(5, None, "live-pi")),
        (6, 1000, slot(6, None, "missing-with-age")),
        (7, 1000, slot(7, None, "unknown-command")),
    ];
    let calls = Cell::new(0);
    let out = validate_runtime_batch(parsed, Some(2000), |pids| {
        calls.set(calls.get() + 1);
        assert_eq!(pids, &[1, 2, 3, 4, 5, 6, 7]);
        HashMap::from([
            (1, (1, "pi".into())),
            (3, (5, "unrelated".into())),
            (4, (1100, "node sdk-runner.js".into())),
            (5, (1100, "pi".into())),
            (7, (1100, String::new())),
        ])
    });
    assert_eq!(calls.get(), 1);
    assert_eq!(out.len(), 3);
    assert_eq!(out[&4].session_path, "sdk-owner");
    assert_eq!(out[&5].session_path, "live-pi");
    assert!(out.contains_key(&1));
    assert!(
        validate_runtime_batch(vec![(1, 0, slot(1, None, "unknown"))], None, |_| panic!(
            "missing observation cannot prove ownership"
        ))
        .is_empty()
    );
}

#[test]
fn source_snapshots_reject_corruption_and_freeze_age_clock() {
    let ps = parse_remote_ps_snapshot(
        "---TIME---\n2000\n---PS---\n4 ttys001 18:20 node sdk.js\n",
        9000,
    )
    .unwrap();
    assert_eq!(ps.observed_at, 2000);
    assert_eq!(ps.processes[&4].etime, 1100);
    let out = validate_runtime_batch(
        vec![(4, 1000, slot(4, None, "sdk"))],
        Some(ps.observed_at),
        |p| ps.batch(p),
    );
    assert!(out.contains_key(&4));
    assert_eq!(
        parse_remote_ps_snapshot("4 ttys001 18:20 pi\n", 9000)
            .unwrap()
            .observed_at,
        9000
    );
    for bad in [
        "garbage",
        "4 ttys001 bad pi",
        "4 ttys001 01:00",
        "0 ttys001 01:00 pi",
        "---TIME---\nwrong\n---PS---\n",
        "---TIME---\n2000\nmissing-marker\n",
        "4 ttys001 01:00 pi\n4 ttys002 01:00 pi\n",
    ] {
        assert!(parse_remote_ps_snapshot(bad, 9000).is_none());
    }
    assert!(parse_remote_panes("host:main.0 4 0 /source/session.jsonl\n").is_some());
    for bad in [
        "garbage",
        "host:main.0 4 x",
        "host:main.0 nope 0",
        "host 4 0",
    ] {
        assert!(parse_remote_panes(bad).is_none());
    }
    let root = Path::new("/synthetic/cache/agent");
    assert_eq!(
        remote_session_path("/host/.pi/agent/sessions/test.jsonl", root).as_deref(),
        Some("/synthetic/cache/agent/sessions/test.jsonl")
    );
    assert!(remote_session_path("/host/.pi/agent/../private", root).is_none());
    assert!(remote_session_path("/desktop/unrelated/session.jsonl", root).is_none());
}

// Run command-spy integration cases in separate test processes. No global HOME,
// CURRENT_HOST, PATH or cache mutation leaks into concurrent pure tests.
#[cfg(unix)]
#[test]
fn runtime_inventory_fake_cli_isolated() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!(
        "viewer-inventory-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("commands")).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    for case in ["remote", "local"] {
        let mut rows = String::new();
        if case == "local" {
            for pid in 1..=512 {
                rows.push_str(&format!("{pid} 20:00 pi\n"));
            }
            rows.push_str("1002 00:05 unrelated\n1003 20:00 node sdk-runner.js\n1004 invalid pi\n");
        } else {
            rows.push_str("11 20:00 pi\n");
        }
        for name in ["ps", "rmux", "lsof", "kill"] {
            let output=match name {
                "ps" => format!("case \"$*\" in *-eo*) printf '11 ttys-local 20:00 pi\\n';; *) printf '%s' '{rows}';; esac"),
                "lsof" => "printf 'p11\\nn/local-cwd\\n'".into(),
                _ => String::new(),
            };
            let script=format!("#!/bin/sh\nprintf '%s\\n' '{name}' >> \"$PI_VIEWER_TEST_COMMAND_TRACE\"\n{output}\n");
            let path = root.join("commands").join(name);
            fs::write(&path, script).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let trace = root.join(format!("{case}.trace"));
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "sessions::performance_regression_tests::runtime_inventory_isolated_child",
                "--nocapture",
            ])
            .env("HOME", &root)
            .env("PI_CODING_AGENT_DIR", root.join("local-agent"))
            .env(
                "PI_CODING_AGENT_SESSION_DIR",
                root.join("local-agent/sessions"),
            )
            .env("PI_VIEWER_TEST_COMMAND_DIR", root.join("commands"))
            .env("PI_VIEWER_TEST_COMMAND_TRACE", &trace)
            .env("PI_VIEWER_INVENTORY_CHILD_CASE", case)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            trace.is_file(),
            "isolated command-spy child did not execute"
        );
    }
}

fn write_runtime(root: &Path, pid: u32, pane: Option<u32>, session: &str, started: i64) {
    fs::create_dir_all(root.join("runtime")).unwrap();
    fs::write(
        root.join(format!("runtime/{pid}.jsonl")),
        serde_json::json!({
        "pid":pid,"panePid":pane,"sessionPath":session,"startedAt":started,
        "cwd":"/synthetic","tty":"ttys-record"})
        .to_string(),
    )
    .unwrap();
}
fn write_session(root: &Path, rel: &str) -> String {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("{HEADER}\n{USER}\n")).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn runtime_inventory_isolated_child() {
    let Ok(case) = std::env::var("PI_VIEWER_INVENTORY_CHILD_CASE") else {
        return;
    };
    let trace = PathBuf::from(std::env::var_os("PI_VIEWER_TEST_COMMAND_TRACE").unwrap());
    let local = pi_agent_dir();
    let local_path = write_session(&local, "sessions/--synthetic--/local.jsonl");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    if case == "local" {
        for pid in 1..=512 {
            write_runtime(
                &local,
                pid,
                None,
                &local_path,
                if pid % 2 == 0 { 0 } else { now - 1000 },
            );
        }
        for pid in [1001, 1002, 1003, 1004] {
            write_runtime(
                &local,
                pid,
                None,
                &local_path,
                if pid == 1001 { 0 } else { now - 1000 },
            );
        }
        let registry = runtime_registry();
        assert_eq!(registry.len(), 513);
        assert!(registry.contains_key(&1003));
        assert!(
            !registry.contains_key(&1001)
                && !registry.contains_key(&1002)
                && !registry.contains_key(&1004)
        );
        assert_eq!(fs::read_to_string(&trace).unwrap(), "ps\n");
        assert_eq!(runtime_registry().len(), 513);
        assert_eq!(fs::read_to_string(&trace).unwrap(), "ps\n");
        return;
    }
    // Warm LOCAL caches first, deliberately colliding with source PID 11.
    write_runtime(&local, 11, None, &local_path, 0);
    assert!(runtime_registry().contains_key(&11));
    assert_eq!(alive_terminal_pis()[0].1, "/local-cwd");
    let _ = rmux_runtime_map();
    let before = fs::read_to_string(&trace).unwrap();
    let rel = "sessions/--synthetic--/source.jsonl";
    crate::remote::set_current_host(Some("fixture-a".into()));
    let a = pi_agent_dir();
    let cached_a = write_session(&a, rel);
    for (pid, pane, started) in [
        (11, None, 9500),
        (22, Some(23), 9500),
        (33, None, 9500),
        (44, None, 0),
        (55, None, 1000),
    ] {
        write_runtime(&a, pid, pane, &format!("/host/.pi/agent/{rel}"), started);
    }
    fs::write(a.join("ps_snapshot.txt"),"---TIME---\n10000\n---PS---\n11 ttys-terminal 20:00 node sdk.js\n22 ttys-pane 20:00 pi\n33 ttys-pane 20:00 pi\n55 ttys-reused 00:05 pi\n").unwrap();
    fs::write(
        a.join("rmux_snapshot.txt"),
        format!("source:worker.0 23 0 /host/.pi/agent/{rel}\n"),
    )
    .unwrap();
    let registry = runtime_registry();
    assert_eq!(registry.len(), 3);
    assert_eq!(registry[&11].session_path, cached_a);
    let terminals = registry_terminal_pis();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].0, 11);
    assert_eq!(alive_terminal_pis().len(), 1);
    fs::write(a.join("rmux_snapshot.txt"), "unknown:main.0 777 0\n").unwrap();
    assert!(
        registry_terminal_pis().is_empty(),
        "missing pane TTY cannot prove non-pane ownership"
    );
    fs::write(
        a.join("rmux_snapshot.txt"),
        format!("source:worker.0 23 0 /host/.pi/agent/{rel}\n"),
    )
    .unwrap();
    assert!(session_has_live_terminal_pi(&cached_a));
    assert_eq!(rmux_runtime_map()[&cached_a].pi_alive, Some(true));
    assert!(pid_alive(11));
    assert!(!pid_alive(44));
    assert!(lsof_cwd(11).is_none());
    assert!(pane_cwd_session(&11, Some(1200), &HashMap::new()).is_none());
    assert_eq!(process_start_epoch(11), Some(8800));
    assert_eq!(list_projects()[0].term_count, 1);
    let _ = list_sessions("--synthetic--");
    let _ = list_running();
    assert_eq!(fs::read_to_string(&trace).unwrap(), before);
    // Switch remote sources within the TTL, reusing the exact same PID.
    crate::remote::set_current_host(Some("fixture-b".into()));
    let b = pi_agent_dir();
    let cached_b = write_session(&b, rel);
    write_runtime(&b, 11, Some(11), &format!("/other/.pi/agent/{rel}"), 9500);
    fs::write(
        b.join("ps_snapshot.txt"),
        "---TIME---\n10000\n---PS---\n11 ttys-b 20:00 pi\n",
    )
    .unwrap();
    fs::write(
        b.join("rmux_snapshot.txt"),
        format!("other:main.0 11 0 /other/.pi/agent/{rel}\n"),
    )
    .unwrap();
    assert_eq!(runtime_registry()[&11].session_path, cached_b);
    let map = rmux_runtime_map();
    assert!(map.contains_key(&cached_b) && !map.contains_key(&cached_a));
    assert!(registry_terminal_pis().is_empty());
    assert_eq!(
        list_projects()[0].term_count,
        0,
        "project cache leaked source A's terminal"
    );
    assert!(list_sessions("--synthetic--")
        .iter()
        .all(|s| Path::new(&s.path).starts_with(&b)));
    fs::write(b.join("ps_snapshot.txt"), "broken snapshot").unwrap();
    assert!(runtime_registry().is_empty());
    assert!(!pid_alive(11));
    assert!(alive_terminal_pis().is_empty());
    assert!(rmux_runtime_map().values().all(|r| r.pi_alive.is_none()));
    fs::write(b.join("rmux_snapshot.txt"), "corrupt panes").unwrap();
    assert!(rmux_runtime_map().is_empty());
    fs::remove_file(b.join("ps_snapshot.txt")).unwrap();
    assert!(runtime_registry().is_empty());
    assert!(registry_terminal_pis().is_empty());
    fs::write(b.join("ps_snapshot.txt"), "11 ttys-b 20:00 pi\n").unwrap();
    fs::write(b.join("rmux_snapshot.txt"), "bare:main.0 11 0\n").unwrap();
    fs::remove_file(b.join("runtime/11.jsonl")).unwrap();
    // Change the source stamp as a synchronized snapshot refresh would.
    fs::write(
        b.join("ps_snapshot.txt"),
        "---TIME---\n10000\n---PS---\n11 ttys-b 20:00 pi\n",
    )
    .unwrap();
    assert!(
        rmux_runtime_map().is_empty(),
        "bare host Pi has no source cwd/owner"
    );
    assert!(alive_terminal_pis().is_empty());
    assert_eq!(
        fs::read_to_string(&trace).unwrap(),
        before,
        "remote readers invoked a desktop probe"
    );
    crate::remote::set_current_host(None);
    assert_eq!(runtime_registry()[&11].session_path, local_path);
    assert_eq!(fs::read_to_string(&trace).unwrap(), format!("{before}ps\n"));
}

const PARENT_MARKER: &str = r#"{"type":"pi_subagent_parent","parentId":"parent-fixture"}"#;
fn legacy_first_line(mut reader: impl Read) -> Option<String> {
    let mut buf = vec![0; 16 * 1024];
    let n = reader.read(&mut buf).ok()?;
    String::from_utf8_lossy(&buf[..n])
        .lines()
        .next()
        .map(str::to_string)
}
fn legacy_preamble(mut reader: impl Read) -> LogPreamble {
    let mut buf = vec![0; 64 * 1024];
    let Ok(n) = reader.read(&mut buf) else {
        return (None, None);
    };
    let mut session = None;
    let mut parent = None;
    for line in String::from_utf8_lossy(&buf[..n]).lines().take(64) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("session") if session.is_none() => {
                session = v.get("id").and_then(Value::as_str).map(str::to_string)
            }
            Some("pi_subagent_parent") if parent.is_none() => {
                parent = v
                    .get("parentId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }
            _ => {}
        }
        if session.is_some() && parent.is_some() {
            break;
        }
    }
    (session, parent)
}
fn same_bounded_discovery(data: &[u8]) {
    assert_eq!(
        first_line_reader(Cursor::new(data)).unwrap(),
        legacy_first_line(Cursor::new(data))
    );
    assert_eq!(
        agent_log_preamble_reader(Cursor::new(data)).unwrap().parsed,
        legacy_preamble(Cursor::new(data))
    );
}

#[test]
fn discovery_readers_golden_utf8_truncation_and_marker_order() {
    for data in [
        format!("{HEADER}\n{PARENT_MARKER}\n"),
        format!("{PARENT_MARKER}\n{HEADER}\n"),
        format!("\nmalformed\n{HEADER}\n{PARENT_MARKER}\n"),
        format!("{HEADER}\n{HEADER}\n{PARENT_MARKER}"),
        format!("{{\"type\":\"session\",\"id\":\"漢字😀\"}}\n{PARENT_MARKER}\n"),
    ] {
        for n in 0..=data.len() {
            same_bounded_discovery(&data.as_bytes()[..n]);
        }
        same_bounded_discovery(data.replace('\n', "\r\n").as_bytes());
    }
    let mut bad = b"{\"type\":\"session\",\"id\":\"".to_vec();
    bad.extend_from_slice(b"\xff\"}\n");
    bad.extend_from_slice(PARENT_MARKER.as_bytes());
    same_bounded_discovery(&bad);
    assert!(
        !agent_log_preamble_reader(Cursor::new(b"malformed\n"))
            .unwrap()
            .cacheable
    );
}

#[test]
fn discovery_readers_golden_hard_byte_and_row_limits() {
    for blanks in [62, 63, 64, 100] {
        same_bounded_discovery(
            format!("{HEADER}\n{}{PARENT_MARKER}\n", "{}\n".repeat(blanks)).as_bytes(),
        );
    }
    for limit in [16 * 1024, 64 * 1024] {
        let mut exact = vec![b' '; limit - HEADER.len()];
        exact.extend_from_slice(HEADER.as_bytes());
        same_bounded_discovery(&exact);
        exact.extend_from_slice(format!("\n{PARENT_MARKER}\n").as_bytes());
        same_bounded_discovery(&exact);
        let mut utf8 = vec![b'x'; limit - 1];
        utf8.extend_from_slice("😀\n".as_bytes());
        same_bounded_discovery(&utf8);
    }
    let oversized = format!(
        "{{\"type\":\"session\",\"id\":\"{}\"}}\n{PARENT_MARKER}\n",
        "x".repeat(70 * 1024)
    );
    same_bounded_discovery(oversized.as_bytes());
    let count = Cell::new(0);
    let _ = agent_log_preamble_reader(Counted {
        input: Cursor::new(oversized.as_bytes()),
        bytes: &count,
    })
    .unwrap();
    assert_eq!(count.get(), 64 * 1024);
    let count = Cell::new(0);
    let _ = first_line_reader(Counted {
        input: Cursor::new(oversized.as_bytes()),
        bytes: &count,
    })
    .unwrap();
    assert_eq!(count.get(), 16 * 1024);
}

#[test]
fn discovery_readers_small_preambles_reduce_actual_read_bytes() {
    let mut small = format!("{HEADER}\n{PARENT_MARKER}\n").into_bytes();
    small.extend(vec![b'x'; 2 * 1024 * 1024]);
    let old = Cell::new(0);
    let new = Cell::new(0);
    let expected = legacy_first_line(Counted {
        input: Cursor::new(&small),
        bytes: &old,
    });
    let actual = first_line_reader(Counted {
        input: Cursor::new(&small),
        bytes: &new,
    })
    .unwrap();
    assert_eq!(expected, actual);
    assert_eq!(old.get(), 16 * 1024);
    assert_eq!(new.get(), 1024);
    let old = Cell::new(0);
    let new = Cell::new(0);
    let expected = legacy_preamble(Counted {
        input: Cursor::new(&small),
        bytes: &old,
    });
    let actual = agent_log_preamble_reader(Counted {
        input: Cursor::new(&small),
        bytes: &new,
    })
    .unwrap()
    .parsed;
    assert_eq!(expected, actual);
    assert_eq!(old.get(), 64 * 1024);
    assert_eq!(new.get(), 4 * 1024);
    // A two-buffer preamble still reads eight times less than the old reader.
    let mut two = format!(
        "{{\"type\":\"pi_subagent_task\",\"padding\":\"{}\"}}\n{HEADER}\n{PARENT_MARKER}\n",
        "x".repeat(5000)
    )
    .into_bytes();
    two.extend(vec![b'x'; 2 * 1024 * 1024]);
    let new = Cell::new(0);
    let actual = agent_log_preamble_reader(Counted {
        input: Cursor::new(&two),
        bytes: &new,
    })
    .unwrap()
    .parsed;
    assert_eq!(actual, legacy_preamble(Cursor::new(&two)));
    assert_eq!(new.get(), 8 * 1024);
}

#[test]
fn preamble_cache_reads_once_invalidates_and_does_not_cross_roots() {
    let root = std::env::temp_dir().join(format!(
        "viewer-preamble-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    for dir in ["a", "b"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let a = root.join("a/task-same.jsonl");
    let b = root.join("b/task-same.jsonl");
    fs::write(&a, format!("{HEADER}\n{PARENT_MARKER}\n")).unwrap();
    fs::write(
        &b,
        format!("{{\"type\":\"session\",\"id\":\"other-root\"}}\n{PARENT_MARKER}\n"),
    )
    .unwrap();
    let mut cache = HashMap::new();
    let reads = Cell::new(0);
    let load = |path: &Path| {
        reads.set(reads.get() + 1);
        agent_log_preamble_reader(fs::File::open(path)?)
    };
    let expected = (Some("fixture".into()), Some("parent-fixture".into()));
    assert_eq!(
        cached_log_preamble(&a, &mut cache, || snapshot_stamp(&a), || load(&a)),
        expected
    );
    assert_eq!(
        cached_log_preamble(&a, &mut cache, || snapshot_stamp(&a), || load(&a)),
        expected
    );
    assert_eq!(
        reads.get(),
        1,
        "prescan/main passes must share one actual file read"
    );
    assert_eq!(
        cached_log_preamble(&b, &mut cache, || snapshot_stamp(&b), || load(&b))
            .0
            .as_deref(),
        Some("other-root")
    );
    assert_eq!(reads.get(), 2);
    fs::write(
        &a,
        format!("{{\"type\":\"session\",\"id\":\"changed-child\"}}\n{PARENT_MARKER}\n"),
    )
    .unwrap();
    assert_eq!(
        cached_log_preamble(&a, &mut cache, || snapshot_stamp(&a), || load(&a))
            .0
            .as_deref(),
        Some("changed-child")
    );
    assert_eq!(reads.get(), 3);
    let before = snapshot_stamp(&a).unwrap();
    let after = (before.0 + std::time::Duration::from_nanos(1), before.1);
    // Same-size, high-resolution mtime invalidation independent of clock granularity.
    assert_eq!(
        cached_log_preamble(&a, &mut cache, || Some(after), || load(&a))
            .0
            .as_deref(),
        Some("changed-child")
    );
    assert_eq!(reads.get(), 4);
    let stamps = Cell::new(0);
    assert_eq!(
        cached_log_preamble(
            &a,
            &mut cache,
            || {
                stamps.set(stamps.get() + 1);
                Some(if stamps.get() == 1 { after } else { before })
            },
            || panic!("cache hit must not read")
        ),
        (None, None)
    );
    assert!(!cache.contains_key(&a));
    let stamps = Cell::new(0);
    assert_eq!(
        cached_log_preamble(
            &a,
            &mut cache,
            || {
                stamps.set(stamps.get() + 1);
                Some(if stamps.get() == 1 { before } else { after })
            },
            || load(&a)
        ),
        (None, None)
    );
    assert!(!cache.contains_key(&a));
    assert_eq!(
        cached_log_preamble(&a, &mut cache, || None, || panic!("missing metadata")),
        (None, None)
    );
    assert_eq!(
        cached_log_preamble(
            &a,
            &mut cache,
            || Some(before),
            || Err(std::io::Error::other("synthetic read error"))
        ),
        (None, None)
    );
    let _ = cached_log_preamble(
        &a,
        &mut cache,
        || Some(before),
        || agent_log_preamble_reader(Cursor::new(b"malformed\n")),
    );
    assert!(!cache.contains_key(&a));
    let _ = cached_log_preamble(
        &a,
        &mut cache,
        || Some(before),
        || agent_log_preamble_reader(Cursor::new(b"{}\n")),
    );
    assert!(!cache.contains_key(&a));
    // Legacy, marker-less logs still cache their valid child without inventing a parent.
    let legacy = cached_log_preamble(
        &a,
        &mut cache,
        || Some(before),
        || agent_log_preamble_reader(Cursor::new(HEADER.as_bytes())),
    );
    assert_eq!(legacy, (Some("fixture".into()), None));
    assert!(cache.contains_key(&a));
    cache.clear();
    for i in 0..16_384 {
        cache.insert(
            PathBuf::from(format!("/synthetic/{i}")),
            (before, expected.clone()),
        );
    }
    let _ = cached_log_preamble(&a, &mut cache, || Some(before), || load(&a));
    assert_eq!(
        cache.len(),
        1,
        "bounded parsed-entry cache must clear at its limit"
    );
}

#[test]
fn legacy_parent_projects_cache_reuses_switches_and_changed_files_only() {
    let mut cache = ParentProjectsCache::new();
    let a = PathBuf::from("/source-a/sessions/project-a");
    let b = PathBuf::from("/source-a/sessions/project-b");
    let mut files_a = HashMap::from([
        ("a-one.jsonl".into(), (1, 10)),
        ("a-two.jsonl".into(), (1, 20)),
    ]);
    let files_b = HashMap::from([("b-one.jsonl".into(), (1, 10))]);
    let scans = std::cell::RefCell::new(Vec::<String>::new());
    let scan = |path: &Path, out: &mut Vec<ParentCall>| {
        let path = path.to_string_lossy().into_owned();
        scans.borrow_mut().push(path.clone());
        out.push(ParentCall {
            path,
            task: "synthetic legacy task".into(),
            alpha_id: None,
        });
    };
    assert_eq!(
        refresh_parent_project(&mut cache, a.clone(), files_a.clone(), scan).len(),
        2
    );
    let _ = refresh_parent_project(&mut cache, b, files_b, scan);
    assert_eq!(scans.borrow().len(), 3);
    let _ = refresh_parent_project(&mut cache, a.clone(), files_a.clone(), scan);
    assert_eq!(scans.borrow().len(), 3, "A-B-A unchanged must not rescan A");
    scans.borrow_mut().clear();
    files_a.insert("a-one.jsonl".into(), (1, 11)); // same-second append, size changes
    let calls = refresh_parent_project(&mut cache, a.clone(), files_a.clone(), scan);
    assert_eq!(&*scans.borrow(), &["a-one.jsonl"]);
    assert_eq!(calls.len(), 2);
    scans.borrow_mut().clear();
    files_a.remove("a-one.jsonl");
    let calls = refresh_parent_project(&mut cache, a, files_a, scan);
    assert!(scans.borrow().is_empty());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].path, "a-two.jsonl");
}

#[test]
fn legacy_parent_projects_cache_is_source_scoped_and_evicts_lru() {
    let mut cache = ParentProjectsCache::new();
    let a = PathBuf::from("/source-a/sessions/same-project");
    let b = PathBuf::from("/source-b/sessions/same-project");
    // Deliberately equal relative paths/fingerprints: only source key can
    // distinguish these, as when a relative source root changes cwd.
    let files = HashMap::from([("same.jsonl".into(), (1, 10))]);
    let scans = Cell::new(0);
    let load = |cache: &mut ParentProjectsCache, dir: PathBuf, label: &str| {
        refresh_parent_project(cache, dir, files.clone(), |path, out| {
            scans.set(scans.get() + 1);
            out.push(ParentCall {
                path: path.to_string_lossy().into_owned(),
                task: label.into(),
                alpha_id: None,
            });
        })
    };
    assert_eq!(load(&mut cache, a.clone(), "source-a")[0].task, "source-a");
    assert_eq!(load(&mut cache, b, "source-b")[0].task, "source-b");
    assert_eq!(
        load(&mut cache, a, "wrong rescan label")[0].task,
        "source-a"
    );
    assert_eq!(scans.get(), 2);
    cache.clear();
    for i in 0..8 {
        let _ = load(
            &mut cache,
            PathBuf::from(format!("/source/project-{i}")),
            "cached",
        );
    }
    let _ = load(&mut cache, PathBuf::from("/source/project-0"), "hit");
    let _ = load(&mut cache, PathBuf::from("/source/project-8"), "new");
    assert_eq!(cache.len(), 8);
    assert!(cache
        .iter()
        .any(|p| p.dir == Path::new("/source/project-0")));
    assert!(!cache
        .iter()
        .any(|p| p.dir == Path::new("/source/project-1")));
}
