//! Synthetic remote regressions. No actual SSH/rsync, real HOME or session data.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("viewer-remote-{label}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&root).unwrap(); Self(root)
    }
    fn session(&self) {
        let dir = self.0.join("sessions/--synthetic--");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("session.jsonl"), b"{\"type\":\"session\",\"id\":\"synthetic\",\"cwd\":\"/synthetic\"}\n").unwrap();
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn capture() -> String {
    "---TIME---\n1700000000\n---PS---\n4242 tty001 10:00 pi\n---RMUX---\npi-fixture:main.0 4242 0 /remote/.pi/agent/sessions/--synthetic--/session.jsonl\n".into()
}

#[test]
fn usable_cache_requires_valid_session_and_rejects_incomplete_initialization() {
    let root = Fixture::new("usable");
    assert!(!usable_cache(&root.0));
    root.session(); assert!(usable_cache(&root.0));
    std::fs::write(root.0.join(INITIALIZING), b"partial").unwrap();
    assert!(!usable_cache(&root.0));
    std::fs::remove_file(root.0.join(INITIALIZING)).unwrap();
    std::fs::write(root.0.join("sessions/--synthetic--/session.jsonl"), b"truncated JSON").unwrap();
    assert!(!usable_cache(&root.0));
}

#[test]
fn initial_failure_does_not_publish_ready_or_reveal_private_stderr() {
    let root = Fixture::new("initial-failure");
    let error = sync_remote_at("fixture", &root.0, |name, _| {
        assert_eq!(name, "rsync"); root.session();
        Err("PRIVATE argv token=secret/path session content".into())
    }).unwrap_err();
    assert!(!error.contains("PRIVATE")); assert!(!error.contains("secret"));
    assert!(!usable_cache(&root.0)); assert!(root.0.join(INITIALIZING).exists());
    let status = host_entry("fixture", &root.0).state.lock().unwrap().status.clone();
    assert_eq!(status.phase, "error"); assert!(!status.usable_cache);
    assert_eq!(status.last_success_at, None);
    let result = sync_remote_at("fixture", &root.0, |_, _| Ok(capture()));
    assert!(result.is_ok()); assert!(usable_cache(&root.0));
    assert!(last_success(&root.0).is_some());
}

#[test]
fn snapshot_failure_retains_cached_files_and_previous_success_time() {
    let root = Fixture::new("snapshot-failure"); root.session();
    std::fs::write(root.0.join("ps_snapshot.txt"), b"old ps").unwrap();
    std::fs::write(root.0.join("rmux_snapshot.txt"), b"old panes").unwrap();
    std::fs::write(root.0.join(MANIFEST), b"{\"lastSuccessAt\":123}").unwrap();
    let mut count = 0;
    assert!(sync_remote_at("fixture", &root.0, |name, _| {
        count += 1;
        if name == "rsync" { Ok(String::new()) } else { Err("private process arguments".into()) }
    }).is_err());
    assert_eq!(count, 2);
    assert_eq!(std::fs::read_to_string(root.0.join("ps_snapshot.txt")).unwrap(), "old ps");
    assert_eq!(std::fs::read_to_string(root.0.join("rmux_snapshot.txt")).unwrap(), "old panes");
    let status = host_entry("fixture", &root.0).state.lock().unwrap().status.clone();
    assert!(status.usable_cache); assert_eq!(status.last_success_at, Some(123));
    assert_eq!(status.phase, "error");
    assert!(!status.error.unwrap().contains("private"));
}

#[test]
fn concurrent_refresh_is_singleflight_per_host_root() {
    let root = Fixture::new("singleflight"); root.session();
    let path = root.0.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0)); let owner_calls = calls.clone();
    let owner = std::thread::spawn(move || sync_remote_at("fixture", &path, |name, _| {
        owner_calls.fetch_add(1, Ordering::SeqCst);
        if name == "rsync" { started_tx.send(()).unwrap(); release_rx.recv_timeout(Duration::from_secs(5)).unwrap(); }
        Ok(capture())
    }));
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let entry = host_entry("fixture", &root.0);
    assert_eq!(entry.state.lock().unwrap().status.phase, "syncing-history");
    let path = root.0.clone();
    let follower = std::thread::spawn(move || sync_remote_at("fixture", &path, |_, _| panic!("duplicate transport")));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if entry.state.lock().unwrap().active.as_ref().is_some_and(|a| Arc::strong_count(a) >= 3) { break; }
        assert!(Instant::now() < deadline, "follower did not join active sync");
        std::thread::yield_now();
    }
    release_tx.send(()).unwrap();
    assert!(owner.join().unwrap().is_ok()); assert!(follower.join().unwrap().is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(entry.state.lock().unwrap().status.phase, "ready");
}

#[test]
fn host_and_snapshot_validation_never_echo_private_strings() {
    for bad in ["../host", "host/path", "-oProxyCommand=bad", "x\nsecret", "a:bad", "", ".."] {
        assert!(validate_host(bad).is_err());
    }
    assert!(validate_host("mac-mini.alias_2").is_ok());
    assert!(validate_host("pi-user@mac-mini").is_ok());
    for snap in ["PRIVATE argv/token=bad", "---TIME---\n0\n---PS---\n---RMUX---\n", "---TIME---\n123\n---PS---\nPRIVATE secret\n---RMUX---\n", "---TIME---\n123\n---PS---\n---RMUX---\nPRIVATE secret"] {
        let error = split_snapshot(snap).unwrap_err();
        assert!(!error.contains("PRIVATE")); assert!(!error.contains("secret"));
    }
    let snap = capture(); assert!(split_snapshot(&snap).is_ok());
    assert!(split_snapshot("---TIME---\n123\n---PS---\n---RMUX---\n").is_ok());
}

// Small component-aware fixture matcher for the explicit rsync filter vocabulary.
// Command arguments themselves are asserted separately; no real rsync is run.
fn star(pattern: &str, value: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == value,
        Some((prefix, rest)) => value.strip_prefix(prefix).is_some_and(|tail| (0..=tail.len()).any(|i| tail.is_char_boundary(i) && star(rest, &tail[i..]))),
    }
}
fn included(args: &[String], path: &str) -> bool {
    for arg in args {
        let (allow, pattern) = if let Some(p) = arg.strip_prefix("--include=") { (true, p) }
            else if let Some(p) = arg.strip_prefix("--exclude=") { (false, p) } else { continue };
        if pattern == "*" { return allow; }
        let parts: Vec<_> = path.trim_matches('/').split('/').collect();
        let matched = if pattern.starts_with('/') {
            let pattern: Vec<_> = pattern.trim_matches('/').split('/').collect();
            pattern.len() == parts.len() && pattern.iter().zip(parts.iter()).all(|(p, v)| star(p, v))
        } else { parts.iter().any(|p| star(pattern.trim_end_matches('/'), p)) };
        if matched { return allow; }
    }
    false
}
#[test]
fn rsync_allowlist_and_delete_protection_cover_only_required_leaves() {
    let args = rsync_args("fixture", Path::new("/synthetic/cache"));
    assert!(args.contains(&"--delay-updates".into()));
    assert!(args.contains(&"--no-links".into()));
    assert!(!args.iter().any(|a| ["--delete-excluded", "--inplace", "--append", "--copy-links"].contains(&a.as_str())));
    for name in ["ps_snapshot.txt", "rmux_snapshot.txt", MANIFEST, INITIALIZING] {
        assert!(args.contains(&format!("--filter=P /{name}")));
    }
    for path in ["sessions/--fixture--/one.jsonl", "agent-logs/task-synthetic.jsonl", "runtime/1234.jsonl", "agents/worker.md", "skills/example/SKILL.md"] {
        assert!(included(&args, path), "{path}");
    }
    for path in ["auth.json", "models.json", "mcp.json", "settings.json", "ps_snapshot.txt", ".viewer-last-sync.json", "sessions/--fixture--/.git/history.jsonl", "sessions/.git/leak.jsonl", "sessions/unrecognized/file.jsonl", "agent-logs/readme.jsonl", "packages/history.jsonl", "artifacts/history.jsonl", "skills/example/packages/SKILL.md", "skills/example/secrets.json", "agents/not-markdown.jsonl"] {
        assert!(!included(&args, path), "{path}");
    }
    assert_eq!(&args[args.len()-2..], &["fixture:.pi/agent/", "/synthetic/cache"]);
    assert_eq!(&ssh_master_args()[..4], &["-o", "ConnectTimeout=10", "-o", "BatchMode=yes"]);
}

// Source selection tests run in their own child so global host changes cannot
// contaminate other concurrently running Rust tests.
#[test]
fn source_selection_isolated() {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "remote::remote_connect_tests::source_selection_isolated_child", "--nocapture"])
        .env("VIEWER_REMOTE_SELECTION_CHILD", "1").output().unwrap();
    assert!(out.status.success(), "isolated source test failed: {}", String::from_utf8_lossy(&out.stderr));
}
#[test]
fn source_selection_isolated_child() {
    if std::env::var("VIEWER_REMOTE_SELECTION_CHILD").as_deref() != Ok("1") { return; }
    let root = Fixture::new("select"); root.session();
    set_current_host(None);
    let epoch = begin_source_selection();
    select_source_with(Some("cached".into()), epoch, |_| usable_cache(&root.0), |_| panic!("cached selection must not wait for transport")).unwrap();
    assert_eq!(current_host().as_deref(), Some("cached"));
    // A refresh already blocked in transport must not become a selection gate.
    let path = root.0.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let refresh = std::thread::spawn(move || sync_remote_at("cached", &path, |name, _| {
        if name == "rsync" {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Ok(capture())
    }));
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let before = Instant::now();
    select_source_with(Some("cached".into()), begin_source_selection(), |_| usable_cache(&root.0), |_| panic!("must not join blocked refresh")).unwrap();
    assert!(before.elapsed() < Duration::from_secs(1));
    release_tx.send(()).unwrap(); assert!(refresh.join().unwrap().is_ok());
    let epoch = begin_source_selection();
    assert!(select_source_with(Some("uncached".into()), epoch, |_| false, |_| Err("Initial sync failed".into())).is_err());
    assert_eq!(current_host().as_deref(), Some("cached"));
    let old = begin_source_selection();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let slow = std::thread::spawn(move || select_source_with(Some("slow".into()), old, |_| false, |_| {
        started_tx.send(()).unwrap(); release_rx.recv_timeout(Duration::from_secs(5)).unwrap(); Ok(())
    }));
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let new = begin_source_selection();
    select_source_with(None, new, |_| false, |_| unreachable!()).unwrap();
    release_tx.send(()).unwrap(); assert!(slow.join().unwrap().is_err());
    assert_eq!(current_host(), None);
    set_current_host(Some("source-a".into()));
    with_current_source(|| {
        set_current_host(Some("source-b".into()));
        assert_eq!(current_host().as_deref(), Some("source-a"));
        with_current_source(|| assert_eq!(current_host().as_deref(), Some("source-a")));
    });
    assert_eq!(current_host().as_deref(), Some("source-b"));
    let _ = std::panic::catch_unwind(|| with_current_source(|| panic!("synthetic unwind")));
    assert_eq!(current_host().as_deref(), Some("source-b"));
}

#[test]
fn fake_local_command_runner_bounds_wait_and_sanitizes_stderr() {
    let error = run_command_bounded("/bin/sh", &["-c".into(), "echo 'PRIVATE token=secret path/argv' >&2; exit 1".into()], Duration::from_secs(2)).unwrap_err();
    assert!(!error.contains("PRIVATE")); assert!(!error.contains("secret"));
    let before = Instant::now();
    let error = run_command_bounded("python3", &["-c".into(), "import time; time.sleep(5)".into()], Duration::from_millis(40)).unwrap_err();
    assert!(error.contains("timed out")); assert!(before.elapsed() < Duration::from_secs(1));
    let error = run_command_bounded("python3", &["-c".into(), "import sys; sys.stdout.write('x' * (9 * 1024 * 1024))".into()], Duration::from_secs(3)).unwrap_err();
    assert!(error.contains("exceeded limit"));
}

#[test]
fn fake_sync_fixture_deletes_managed_leaves_but_preserves_excluded_cache() {
    let root = Fixture::new("filter-fixture"); root.session();
    let old = "sessions/--synthetic--/obsolete.jsonl";
    std::fs::write(root.0.join(old), b"old managed history").unwrap();
    for path in ["settings.json", "ps_snapshot.txt", "rmux_snapshot.txt", "skills/example/README.md", "sessions/--synthetic--/.git/config"] {
        let full = root.0.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, b"excluded local fixture").unwrap();
    }
    let source = [
        ("sessions/--synthetic--/current.jsonl", "{\"type\":\"session\",\"id\":\"current\"}\n"),
        ("agent-logs/task-fixture.jsonl", "{}\n"), ("runtime/4242.jsonl", "{}\n"),
        ("agents/synthetic.md", "synthetic agent"), ("skills/example/SKILL.md", "synthetic skill"),
        ("auth.json", "not approved"), ("packages/resource.jsonl", "not approved"),
        ("artifacts/resource.jsonl", "not approved"), ("skills/example/.git/config", "not approved"),
    ];
    fn files(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for file in std::fs::read_dir(dir).unwrap().flatten() {
            if file.file_type().unwrap().is_dir() { files(root, &file.path(), out); }
            else { out.push(file.path().strip_prefix(root).unwrap().to_string_lossy().into_owned()); }
        }
    }
    let mut phases = Vec::new();
    sync_remote_at("fixture", &root.0, |name, args| {
        phases.push(name.to_string());
        if name == "rsync" {
            let mut old_files = Vec::new(); files(&root.0, &root.0, &mut old_files);
            for old in old_files {
                if included(args, &old) && !source.iter().any(|(p, _)| *p == old) {
                    std::fs::remove_file(root.0.join(old)).unwrap();
                }
            }
            for (path, data) in source {
                if included(args, path) {
                    let full = root.0.join(path);
                    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                    std::fs::write(full, data).unwrap();
                }
            }
            // These must survive history deletion until a successful capture.
            assert_eq!(std::fs::read(root.0.join("ps_snapshot.txt")).unwrap(), b"excluded local fixture");
            assert_eq!(std::fs::read(root.0.join("rmux_snapshot.txt")).unwrap(), b"excluded local fixture");
            Ok(String::new())
        } else { Ok(capture()) }
    }).unwrap();
    assert_eq!(phases, ["rsync", "ssh"]);
    assert!(!root.0.join(old).exists());
    for (path, _) in source {
        assert_eq!(root.0.join(path).exists(), included(&rsync_args("fixture", &root.0), path), "{path}");
    }
    for path in ["settings.json", "skills/example/README.md", "sessions/--synthetic--/.git/config"] {
        assert_eq!(std::fs::read(root.0.join(path)).unwrap(), b"excluded local fixture");
    }
}
