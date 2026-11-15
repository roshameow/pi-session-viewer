//! Pure argv/liveness checks. No process or session discovery.
use super::*;

const ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

#[test]
fn exact_agy_conversation_argv_only() {
    for command in [
        format!("agy --conversation {ID}"),
        format!("/synthetic/bin/agy --dangerously-skip-permissions --conversation {ID}"),
        format!("/synthetic/bin/agy --conversation={ID}"),
    ] {
        assert_eq!(agy_command_conversation(&command), Some(ID));
    }
    for command in [
        format!("/bin/bash -c agy --conversation {ID}"),
        format!("echo agy --conversation {ID}"),
        format!("pi --conversation {ID}"),
        format!("agy-worker --conversation {ID}"),
        "agy --conversation garbage".into(),
        "agy --conversation".into(),
        "agy".into(),
        "agy --conversation 111111111111411181111111111111111111".into(),
    ] {
        assert_eq!(agy_command_conversation(&command), None, "{command}");
    }
}

#[test]
fn agy_pane_location_does_not_fabricate_liveness() {
    let path = format!("/synthetic/brain/{ID}/.system_generated/logs/transcript.jsonl");
    let cmd = format!("/bin/agy --conversation {ID}");
    let wrong = format!("/bin/agy --conversation {OTHER}");
    assert_eq!(agy_pane_liveness(false, false, &path, Some(&cmd)), Some(true));
    assert_eq!(agy_pane_liveness(true, false, &path, Some(&cmd)), Some(false));
    assert_eq!(agy_pane_liveness(false, false, &path, Some(&wrong)), Some(false));
    assert_eq!(agy_pane_liveness(false, false, &path, Some("/bin/zsh")), None);
    assert_eq!(agy_pane_liveness(false, false, &path, None), None);
    assert_eq!(agy_pane_liveness(false, false, &path, Some("agy")), None);
}

#[test]
fn engine_classification_requires_exact_agy_transcript_shape() {
    let path = format!("/synthetic/brain/{ID}/.system_generated/logs/transcript.jsonl");
    assert_eq!(agy_transcript_conversation(&path), Some(ID));
    for path in [
        format!("/synthetic/brain/{ID}/sessions/pi.jsonl"),
        format!("/synthetic/brain/{ID}/transcript.jsonl"),
        "/synthetic/brain/not-a-uuid/.system_generated/logs/transcript.jsonl".into(),
    ] {
        assert_eq!(agy_transcript_conversation(&path), None);
    }
    assert_eq!(agy_pane_liveness(false, true, &path, Some("/bin/zsh")), Some(true));
    assert_eq!(agy_pane_liveness(true, true, &path, Some("/bin/zsh")), Some(false));
}

#[test]
fn incomplete_timestamps_are_rejected_without_inventory_panics() {
    assert_eq!(parse_iso_ts("2026-01-01T12:34"), None);
    assert_eq!(parse_iso_ts("2026-01-01T12"), None);
    assert_eq!(parse_iso_ts("2026-01-01T12:34:56:78"), None);
    assert!(parse_iso_ts("2026-01-01T12:34:56Z").is_some());
}

#[test]
fn rmux_unicode_names_are_not_uuid_slices() {
    for name in [
        "测试兼容窗口.0", "s😀中文窗口.0", "中文会话", "ééééééé",
        "😀😀😀😀", "ascii", "s中文测试窗口.0",
    ] {
        assert_eq!(trailing_id12(name), None, "{name}");
        assert_eq!(window_id8(name), None, "{name}");
    }
    assert_eq!(trailing_id12("中文项目-12345678-abc").as_deref(), Some("12345678-abc"));
    assert_eq!(trailing_id12("😀12345678-abc").as_deref(), Some("12345678-abc"));
    assert_eq!(window_id8("s12345678.0"), Some("12345678"));
}
