//! AGY transcripts are ordered event logs, not Pi parent-id trees.
//! step_index can have gaps; it is an identity, not a predecessor pointer.
use crate::sessions::Entry;
use serde_json::Value;

pub(crate) fn is_step(kind: &str, value: &Value) -> bool {
    matches!(kind, "USER_INPUT" | "PLANNER_RESPONSE" | "GENERIC"
        | "SYSTEM_MESSAGE" | "ERROR_MESSAGE" | "CHECKPOINT")
        || value.get("step_index").is_some()
}

pub(crate) fn link_previous(entry: &mut Entry, previous: Option<&Entry>) {
    entry.parent_id = previous.map(|entry| entry.id.clone());
}
