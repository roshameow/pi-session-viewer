//! Backend-only synthetic switching/concurrency cases. Real inventory commands
//! and private HOME are never used: integration runs in an isolated test child.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static FINGERPRINTS: AtomicUsize = AtomicUsize::new(0);
static REGISTRY_READS: AtomicUsize = AtomicUsize::new(0);
static INDEX_READS: AtomicUsize = AtomicUsize::new(0);
static SESSION_MISSES: AtomicUsize = AtomicUsize::new(0);
pub(super) fn record(what: &str) {
    if std::env::var_os("PI_VIEWER_SWITCH_CHILD").is_none() { return; }
    match what {
        "fingerprint" => &FINGERPRINTS,
        "registry" => &REGISTRY_READS,
        "index" => &INDEX_READS,
        "session_miss" => &SESSION_MISSES,
        _ => unreachable!(),
    }.fetch_add(1, Ordering::SeqCst);
}

fn source(root: &str) -> InventorySource {
    InventorySource { root: root.into(), sessions: PathBuf::from(root).join("sessions"),
        remote: false, ps: None, rmux: None }
}
fn meta(path: &str, running: bool) -> SessionMeta {
    SessionMeta { path: path.into(), id: path.into(), cwd: "/synthetic".into(),
        name: None, first_message: None, last_message: None, created_iso: "".into(),
        created_at: 0, updated_at: 0, model: None, is_subagent: false, task_id: None,
        parent_session_id: None, parent_session_path: None, message_count: 0,
        running, sleeping: false, interrupted: false, in_rmux: false,
        rmux_target: None, rmux_attached: false, rmux_dead: false,
        rmux_pi_alive: None, term_alive: false, size: 1 }
}

#[test]
fn session_result_lru_preserves_projects_sources_ttl_and_invalidations() {
    let mut cache = SessionListsCache::default();
    let a = source("/source-a"); let b = source("/source-b");
    let fp = vec![("same.jsonl".into(), 1, 10)];
    let running = HashSet::new();
    cache.put(a.clone(), "A", fp.clone(), running.clone(), vec![meta("a/A", true)]);
    cache.put(a.clone(), "B", fp.clone(), running.clone(), vec![meta("a/B", false)]);
    assert_eq!(cache.get(&a, "A", Some(&fp), &running).unwrap()[0].path, "a/A");
    cache.put(b.clone(), "A", fp.clone(), running.clone(), vec![meta("b/A", false)]);
    assert!(!cache.get(&b, "A", Some(&fp), &running).unwrap()[0].running);
    assert!(cache.get(&a, "A", Some(&fp), &running).unwrap()[0].running);
    // Same-second append, missing fingerprint, and desktop RUNNING changes invalidate.
    assert!(cache.get(&a, "A", Some(&vec![("same.jsonl".into(), 1, 11)]), &running).is_none());
    assert!(cache.get(&a, "B", None, &running).is_none());
    let active = HashSet::from(["desktop-owned".into()]);
    assert!(cache.get(&b, "A", Some(&fp), &active).is_none());
    cache.put(a.clone(), "A", fp.clone(), running.clone(), vec![meta("a/A", true)]);
    let original = Instant::now() - Duration::from_millis(1900);
    cache.0.back_mut().unwrap().at = original;
    assert!(cache.get(&a, "A", Some(&fp), &running).is_some());
    assert_eq!(cache.0.back().unwrap().at, original, "MRU touch must not keep busy alive indefinitely");
    cache.0.back_mut().unwrap().at = Instant::now() - Duration::from_secs(3);
    assert!(cache.get(&a, "A", Some(&fp), &running).is_none());
    // Captured source snapshots also invalidate even within the freshness window.
    let mut remote = b.clone(); remote.remote = true;
    remote.ps = Some((std::time::UNIX_EPOCH, 10));
    cache.put(remote.clone(), "A", fp.clone(), running.clone(), vec![meta("host/A", true)]);
    remote.ps = Some((std::time::UNIX_EPOCH, 11));
    assert!(cache.get(&remote, "A", Some(&fp), &running).is_none());
}

#[test]
fn session_result_lru_bound_touches_mru() {
    let mut cache = SessionListsCache::default(); let src = source("/synthetic");
    let fp = vec![]; let running = HashSet::new();
    for i in 0..8 { cache.put(src.clone(), &i.to_string(), fp.clone(), running.clone(), vec![]); }
    assert!(cache.get(&src, "0", Some(&fp), &running).is_some());
    cache.put(src.clone(), "8", fp.clone(), running.clone(), vec![]);
    assert_eq!(cache.0.len(), 8);
    assert!(cache.get(&src, "1", Some(&fp), &running).is_none());
    assert!(cache.get(&src, "0", Some(&fp), &running).is_some());
}

#[cfg(unix)]
#[test]
fn switching_fake_inventory_isolated() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("viewer-switch-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(root.join("commands")).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(root.clone());
    for name in ["ps", "rmux", "lsof", "kill"] {
        let body = match name {
            "ps" => "case \"$*\" in *-eo*) cat \"$PI_VIEWER_SWITCH_ROOT/full-ps\";; *) cat \"$PI_VIEWER_SWITCH_ROOT/batch-ps\";; esac",
            "rmux" => "case \"$*\" in *pane_tty*) :;; *list-clients*) :;; *) cat \"$PI_VIEWER_SWITCH_ROOT/panes\";; esac",
            _ => "echo 'unexpected inventory command' >&2; exit 99",
        };
        let script = format!("#!/bin/sh\nprintf '%s\\n' '{name} ' \"$*\" >> \"$PI_VIEWER_TEST_COMMAND_TRACE\"\n{body}\n");
        let p = root.join("commands").join(name);
        fs::write(&p, script).unwrap(); fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "sessions::switching_tests::switching_inventory_child", "--nocapture"])
        .env("HOME", &root).env("PI_CODING_AGENT_DIR", root.join("agent"))
        .env("PI_CODING_AGENT_SESSION_DIR", root.join("agent/sessions"))
        .env("PI_VIEWER_TEST_COMMAND_DIR", root.join("commands"))
        .env("PI_VIEWER_TEST_COMMAND_TRACE", root.join("trace"))
        .env("PI_VIEWER_SWITCH_ROOT", &root).env("PI_VIEWER_SWITCH_CHILD", "1")
        .output().unwrap();
    assert!(result.status.success(), "{}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    println!("{}", String::from_utf8_lossy(&result.stdout));
}

fn write_session(root: &Path, project: &str, i: usize, pending: bool) -> String {
    let id = format!("{:08x}-0000-0000-0000-{i:012}", i + 1);
    let p = root.join(format!("sessions/{project}/2026-01-01T00-00-00-000Z_{id}.jsonl"));
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let message = if pending { serde_json::json!({"role":"user","content":"synthetic pending"}) }
        else { serde_json::json!({"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"synthetic done"}]}) };
    fs::write(&p, format!("{}\n{}\n", serde_json::json!({"type":"session","id":id,"cwd":"/synthetic","timestamp":"2026-01-01T00:00:00.000Z"}),
        serde_json::json!({"type":"message","id":"message","message":message}))).unwrap();
    let f = fs::OpenOptions::new().write(true).open(&p).unwrap();
    f.set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1700000000))).unwrap();
    p.to_string_lossy().into_owned()
}
fn slot(root: &Path, pid: u32, path: &str, started: i64) {
    fs::create_dir_all(root.join("runtime")).unwrap();
    fs::write(root.join(format!("runtime/{pid}.jsonl")), serde_json::json!({"pid":pid,"sessionPath":path,
        "startedAt":started,"cwd":"/synthetic","tty":"ttys-fixture","panePid":pid}).to_string()).unwrap();
}
fn row<'a>(rows: &'a [SessionMeta], path: &str) -> &'a SessionMeta { rows.iter().find(|m| m.path == path).unwrap() }
fn commands(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("trace")).unwrap_or_default().lines()
        .filter(|l| ["ps ","rmux ","lsof ","kill "].contains(l)).map(str::to_owned).collect()
}

#[test]
fn switching_inventory_child() {
    if std::env::var_os("PI_VIEWER_SWITCH_CHILD").is_none() { return; }
    let root = PathBuf::from(std::env::var_os("PI_VIEWER_SWITCH_ROOT").unwrap());
    let agent = pi_agent_dir();
    let paths: Vec<_> = (0..64).map(|i| write_session(&agent, "--A--", i, i < 6)).collect();
    write_session(&agent, "--B--", 80, false);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    slot(&agent, 11, &paths[0], 0); // validated private SDK owner, old pending transcript
    slot(&agent, 12, &paths[1], now - 1000);
    slot(&agent, 13, &paths[2], now - 1000); // younger reused PID must be rejected
    slot(&agent, 14, &paths[3], 0); // missing PID must be rejected even with startedAt=0
    // pane_tty stub excludes tty; comm=pi fallback would lsof. Put pi rows on ?? in full PS.
    fs::write(root.join("full-ps"), "11 ?? 20:00 node sdk-runner.js\n12 ?? 20:00 pi\n13 ?? 00:05 pi\n15 ?? 20:00 bash\n16 ?? 20:00 bash\n").unwrap();
    fs::write(root.join("batch-ps"), "11 20:00 node sdk-runner.js\n12 20:00 pi\n13 00:05 pi\n15 20:00 bash\n16 20:00 bash\n").unwrap();
    fs::write(root.join("panes"), format!("sdk:main.0 11 0 {}\npi:main.0 12 0 {}\nunknown:main.0 15 0 {}\ndead:main.0 16 1 {}\n", paths[0], paths[1], paths[4], paths[5])).unwrap();
    // Six equivalent session requests plus independent projects/running requests.
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut threads = vec![];
    for i in 0..8 {
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            match i { 6 => { list_projects(); }, 7 => { list_running(); }, _ => { list_sessions("--A--"); } }
        }));
    }
    for thread in threads { thread.join().unwrap(); }
    assert_eq!(SESSION_MISSES.load(Ordering::SeqCst), 1);
    assert_eq!(FINGERPRINTS.load(Ordering::SeqCst), 3, "one global walk pair per list miss, not four individual walks");
    assert_eq!(INDEX_READS.load(Ordering::SeqCst), 3);
    assert_eq!(REGISTRY_READS.load(Ordering::SeqCst), 3, "one registry copy per list, not per main");
    let cmds = commands(&root);
    assert_eq!(cmds.iter().filter(|c| *c == "ps ").count(), 3, "one registry batch + one pane batch + one full PS");
    assert_eq!(cmds.iter().filter(|c| *c == "rmux ").count(), 3, "one panes + one clients + one shared tty probe");
    assert!(!cmds.iter().any(|c| c == "kill " || c == "lsof "));
    let a = list_sessions("--A--");
    assert!(row(&a, &paths[0]).running && row(&a, &paths[1]).running);
    assert!(!row(&a, &paths[2]).running && !row(&a, &paths[3]).running);
    assert_eq!(row(&a, &paths[4]).rmux_pi_alive, None);
    assert!(!row(&a, &paths[4]).running);
    assert!(row(&a, &paths[5]).rmux_dead && !row(&a, &paths[5]).running);
    list_sessions("--B--");
    let before = SESSION_MISSES.load(Ordering::SeqCst);
    assert!(row(&list_sessions("--A--"), &paths[0]).running);
    assert_eq!(SESSION_MISSES.load(Ordering::SeqCst), before, "unchanged A survives A-B-A");
    // Selected data append invalidates the LRU and completed turn isn't falsely busy.
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(&paths[0]).unwrap().write_all(b"{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"stop\",\"content\":[]}}\n").unwrap();
    assert!(!row(&list_sessions("--A--"), &paths[0]).running);
    assert_eq!(SESSION_MISSES.load(Ordering::SeqCst), before + 1);
    let before = SESSION_MISSES.load(Ordering::SeqCst);
    mark_running(&paths[3]);
    assert!(row(&list_sessions("--A--"), &paths[3]).running);
    unmark_running(&paths[3]);
    assert!(!row(&list_sessions("--A--"), &paths[3]).running);
    assert_eq!(SESSION_MISSES.load(Ordering::SeqCst), before + 2);
    // Colliding PID + project key in remote A and B, no desktop command fallbacks.
    let commands_before = commands(&root).len();
    for (host, pending) in [("switch-a", true), ("switch-b", false), ("switch-a", true)] {
        crate::remote::set_current_host(Some(host.into()));
        let remote = pi_agent_dir();
        if !remote.exists() {
            let path = write_session(&remote, "--A--", 0, pending);
            slot(&remote, 11, &path, 0);
            fs::write(remote.join("ps_snapshot.txt"), format!("---TIME---\n{now}\n---PS---\n11 ttys-host 20:00 node sdk-runner.js\n")).unwrap();
            fs::write(remote.join("rmux_snapshot.txt"), format!("host:main.0 11 0 {path}\n")).unwrap();
        }
        let rows = list_sessions("--A--");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].path.starts_with(&remote.to_string_lossy().to_string()));
        assert_eq!(rows[0].running, pending);
        assert_eq!(rows[0].rmux_pi_alive, Some(true), "private SDK PID is valid ownership evidence");
    }
    assert_eq!(commands(&root).len(), commands_before, "remote snapshots never probe desktop PIDs");
    crate::remote::set_current_host(None);
    assert!(!row(&list_sessions("--A--"), &paths[0]).running);
    // A malformed snapshot invalidates previous evidence via its source stamp.
    crate::remote::set_current_host(Some("switch-a".into()));
    fs::write(pi_agent_dir().join("ps_snapshot.txt"), "broken\n").unwrap();
    let rows = list_sessions("--A--");
    assert!(!rows[0].running);
    assert_eq!(rows[0].rmux_pi_alive, None);
    // Corrupt rmux results stay unknown and equivalent concurrent failures share a fill.
    fs::write(pi_agent_dir().join("rmux_snapshot.txt"), "broken panes\n").unwrap();
    let before = REGISTRY_READS.load(Ordering::SeqCst);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
    let threads: Vec<_> = (0..6).map(|_| {
        let barrier = barrier.clone();
        std::thread::spawn(move || { barrier.wait(); assert!(rmux_runtime_map().is_empty()); })
    }).collect();
    for thread in threads { thread.join().unwrap(); }
    assert_eq!(REGISTRY_READS.load(Ordering::SeqCst), before + 1);
    let rows = list_sessions("--A--");
    assert!(!rows[0].running && !rows[0].in_rmux && !rows[0].rmux_dead);
    assert_eq!(rows[0].rmux_pi_alive, None);
    println!("synthetic: one fill for six concurrent session requests; 3 fingerprint pairs/3 registry copies for three list kinds; 6 fake CLI calls; A-B-A/append/RUNNING/source/unknown/age/failed-inventory coalescing guards passed");
}
