//! Shared-runtime regressions using only synthetic metadata and in-memory maps.
//! No session discovery, filesystem access, processes, or CLI calls.
use super::runtime::*;
use super::SessionMeta;
use std::collections::HashMap;

const ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_ID: &str = "22222222-2222-4222-8222-222222222222";
const REAL: &str = "/synthetic/sessions/real.jsonl";
const MIRROR: &str = "/synthetic/sessions/subagent-task-worker.jsonl";

fn session(engine: &str) -> SessionMeta {
    SessionMeta {
        path: if engine == "agy" {
            format!("/synthetic/brain/{ID}/.system_generated/logs/transcript.jsonl")
        } else {
            REAL.into()
        },
        id: ID.into(),
        cwd: "/synthetic/project".into(),
        name: Some("runtime fixture".into()),
        first_message: Some("first".into()),
        last_message: Some("last".into()),
        created_iso: "2026-01-01T00:00:00Z".into(),
        created_at: 1,
        updated_at: 2,
        model: Some("synthetic-model".into()),
        is_subagent: false,
        task_id: None,
        parent_session_id: None,
        parent_session_path: None,
        message_count: 3,
        running: false,
        sleeping: false,
        interrupted: false,
        // Deliberately stale: every hydration must replace all location fields.
        in_rmux: true,
        rmux_target: Some("stale:window.0".into()),
        rmux_attached: true,
        rmux_pi_alive: Some(true),
        rmux_dead: true,
        term_alive: false,
        size: 123,
        engine: Some(engine.into()),
    }
}

fn runtime(attached: bool, dead: bool, pi_alive: Option<bool>) -> RmuxRuntime {
    RmuxRuntime {
        target: "synthetic:worker.0".into(),
        attached,
        dead,
        pi_alive,
    }
}

fn location(meta: &SessionMeta) -> (bool, Option<&str>, bool, bool, Option<bool>) {
    (
        meta.in_rmux,
        meta.rmux_target.as_deref(),
        meta.rmux_attached,
        meta.rmux_dead,
        meta.rmux_pi_alive,
    )
}

fn assert_preserved(after: &SessionMeta, before: &SessionMeta) {
    assert_eq!(after.running, before.running);
    assert_eq!(after.sleeping, before.sleeping);
    assert_eq!(after.interrupted, before.interrupted);
    assert_eq!(after.term_alive, before.term_alive);
    assert_eq!(after.engine, before.engine);
    assert_eq!(after.path, before.path);
}

#[test]
fn pi_and_agy_share_the_entire_hydration_matrix_without_changing_activity() {
    // Attached/detached, live/dead (including contradictory live evidence),
    // ended, and unknown are location evidence, not busy/terminal state.
    for attached in [false, true] {
        for dead in [false, true] {
            for pi_alive in [Some(true), Some(false), None] {
                let rt = runtime(attached, dead, pi_alive);
                for flags in 0..16 {
                    let mut pi = session("pi");
                    let mut agy = session("agy");
                    for meta in [&mut pi, &mut agy] {
                        meta.running = flags & 1 != 0;
                        meta.sleeping = flags & 2 != 0;
                        meta.interrupted = flags & 4 != 0;
                        meta.term_alive = flags & 8 != 0;
                        let before = meta.clone();
                        apply_rmux_runtime(meta, Some(&rt));
                        assert_eq!(
                            location(meta),
                            (true, Some(rt.target.as_str()), attached, dead, pi_alive),
                            "engine={:?}, flags={flags}",
                            meta.engine,
                        );
                        assert_preserved(meta, &before);
                    }
                    assert_eq!(location(&pi), location(&agy));
                }
            }
        }
    }
}

#[test]
fn absent_runtime_clears_every_stale_location_field_for_both_engines() {
    for flags in 0..16 {
        let mut pi = session("pi");
        let mut agy = session("agy");
        for meta in [&mut pi, &mut agy] {
            meta.running = flags & 1 != 0;
            meta.sleeping = flags & 2 != 0;
            meta.interrupted = flags & 4 != 0;
            meta.term_alive = flags & 8 != 0;
            let before = meta.clone();
            apply_rmux_runtime(meta, None);
            assert_eq!(location(meta), (false, None, false, false, None));
            assert_preserved(meta, &before);
            // Repeated absence must not restore any stale state.
            apply_rmux_runtime(meta, None);
            assert_eq!(location(meta), (false, None, false, false, None));
            assert_preserved(meta, &before);
        }
        assert_eq!(location(&pi), location(&agy));
    }
}

#[test]
fn is_live_requires_positive_identity_and_a_non_dead_pane() {
    for attached in [false, true] {
        for dead in [false, true] {
            for pi_alive in [Some(true), Some(false), None] {
                assert_eq!(
                    runtime(attached, dead, pi_alive).is_live(),
                    !dead && pi_alive == Some(true),
                    "attached={attached}, dead={dead}, pi_alive={pi_alive:?}",
                );
            }
        }
    }
}

#[test]
fn runtime_ranking_prefers_live_then_unknown_then_ended_then_dead() {
    let live = runtime_rank(false, Some(true));
    let unknown = runtime_rank(false, None);
    let ended = runtime_rank(false, Some(false));
    assert!(live > unknown);
    assert!(unknown > ended);
    for pi_alive in [Some(true), Some(false), None] {
        assert!(ended > runtime_rank(true, pi_alive));
        assert_eq!(runtime_rank(true, pi_alive), runtime_rank(true, None));
    }
}

#[test]
fn counter_deduplicates_live_real_and_mirror_paths_in_either_order() {
    let map = HashMap::from([
        (REAL.into(), runtime(false, false, Some(true))),
        (MIRROR.into(), runtime(true, false, Some(true))),
    ]);
    for paths in [[REAL, MIRROR], [MIRROR, REAL]] {
        let mut count = RmuxSessionCount::default();
        assert_eq!(count.len(), 0);
        for path in paths {
            count.observe(ID, path, &map);
            assert_eq!(count.len(), 1);
        }
        for path in paths {
            count.observe(ID, path, &map);
        }
        assert_eq!(count.len(), 1);
    }
}

#[test]
fn non_live_alias_cannot_hide_a_live_uuid_regardless_of_observation_order() {
    // The first observation must not reserve a UUID unless it is live.
    for live_path in [REAL, MIRROR] {
        let other_path = if live_path == REAL { MIRROR } else { REAL };
        for other_runtime in [
            None,
            Some(runtime(false, false, None)),
            Some(runtime(true, false, Some(false))),
            Some(runtime(true, true, Some(true))),
        ] {
            let mut map = HashMap::from([(
                live_path.into(),
                runtime(false, false, Some(true)),
            )]);
            if let Some(rt) = other_runtime {
                map.insert(other_path.into(), rt);
            }
            for paths in [[REAL, MIRROR], [MIRROR, REAL]] {
                let mut count = RmuxSessionCount::default();
                let mut saw_live = false;
                for path in paths {
                    count.observe(ID, path, &map);
                    saw_live |= path == live_path;
                    assert_eq!(count.len(), if saw_live { 1 } else { 0 });
                }
                count.observe(ID, live_path, &map);
                assert_eq!(count.len(), 1);
                // This is UUID deduplication, not target or path deduplication.
                count.observe(OTHER_ID, live_path, &map);
                assert_eq!(count.len(), 2);
            }
        }
    }
}

#[test]
fn counter_ignores_unknown_ended_dead_and_unmapped_sessions() {
    let mut count = RmuxSessionCount::default();
    let missing = HashMap::new();
    count.observe(ID, REAL, &missing);
    assert_eq!(count.len(), 0);

    for attached in [false, true] {
        for dead in [false, true] {
            for pi_alive in [Some(true), Some(false), None] {
                if !dead && pi_alive == Some(true) {
                    continue;
                }
                let map = HashMap::from([(REAL.into(), runtime(attached, dead, pi_alive))]);
                count.observe(ID, REAL, &map);
                count.observe(OTHER_ID, MIRROR, &map);
                assert_eq!(
                    count.len(),
                    0,
                    "attached={attached}, dead={dead}, pi_alive={pi_alive:?}",
                );
            }
        }
    }
}
