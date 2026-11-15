//! Engine-neutral location state. Collection happens once in the shared
//! inventory; adapters supply identity and busy state, not their own rmux UI rules.
use super::SessionMeta;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct RmuxRuntime {
    pub target: String,
    pub attached: bool,
    pub dead: bool, // pane_dead only; an idle shell is not a live agent
    // Legacy internal/IPC name; represents either Pi or AGY agent identity.
    pub pi_alive: Option<bool>,
}

impl RmuxRuntime {
    pub fn is_live(&self) -> bool {
        !self.dead && self.pi_alive == Some(true)
    }
}

/// Apply only location evidence. Never infer busy, sleeping, interrupted, or
/// non-rmux terminal liveness from a pane. Missing evidence clears stale state.
pub fn apply_rmux_runtime(session: &mut SessionMeta, runtime: Option<&RmuxRuntime>) {
    session.in_rmux = runtime.is_some();
    session.rmux_target = runtime.map(|rt| rt.target.clone());
    session.rmux_attached = runtime.is_some_and(|rt| rt.attached);
    session.rmux_dead = runtime.is_some_and(|rt| rt.dead);
    session.rmux_pi_alive = runtime.and_then(|rt| rt.pi_alive);
}

/// One confirmed-live session per engine UUID, even if both its canonical file
/// and a durable mirror are present. Observe all paths before deduplicating
/// metadata: the strongest runtime may be on the later mirror.
#[derive(Default)]
pub struct RmuxSessionCount(HashSet<String>);

impl RmuxSessionCount {
    pub fn observe(&mut self, id: &str, path: &str, map: &HashMap<String, RmuxRuntime>) {
        if map.get(path).is_some_and(RmuxRuntime::is_live) {
            self.0.insert(id.to_string());
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

pub(super) fn runtime_rank(dead: bool, agent_alive: Option<bool>) -> u8 {
    if dead { 0 } else {
        match agent_alive { Some(true) => 3, None => 2, Some(false) => 1 }
    }
}
