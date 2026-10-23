//! Deliberately narrow housekeeping exceptions. Unknown fields remain content.
use super::*;
use serde_json::Value;

fn json(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|_| refused("invalid local Code JSON"))
}

fn session(path: &Path) -> bool {
    path.components().count() == 1
        && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            n.strip_prefix("local_")
                .and_then(|n| n.strip_suffix(".json"))
                .is_some_and(uuid)
        })
}

fn focus(value: &mut Value) {
    if let Some(object) = value.as_object_mut()
        && object
            .get("lastFocusedAt")
            .is_some_and(|v| v.as_u64().is_some())
    {
        object.remove("lastFocusedAt");
    }
}

fn quit_marker(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == 5
            && matches!(
                object.get("exitKind").and_then(Value::as_str),
                Some("quit" | "update_restart")
            )
            && ["at", "cronCount", "durableCronCount", "backgroundTaskCount"]
                .iter()
                .all(|key| object.get(*key).is_some_and(|v| v.as_u64().is_some()))
    })
}

fn gc(value: &mut Value) {
    if let Some(object) = value
        .get_mut("untrackedDirGc")
        .and_then(Value::as_object_mut)
        && object
            .get("cwds")
            .and_then(Value::as_object)
            .is_some_and(|m| m.values().all(|v| v.as_u64().is_some()))
    {
        object.remove("cwds");
    }
}

fn max_field(target: &mut Value, other: &Value) -> bool {
    if let Some(n) = other.as_u64()
        && target.as_u64().is_some_and(|t| n > t)
    {
        *target = other.clone();
        return true;
    }
    false
}

impl Store {
    /// Claude consumes these markers when an updater opens a stale environment.
    /// Restore only missing, recognized markers in a comparison copy. The
    /// complete verified snapshot must still match; never merge restart state
    /// into the authority or ignore a marker that was added or changed.
    pub(super) fn matches_consumed_quit_markers(&self, baseline: &Store) -> Result<bool> {
        let mut restored = self.clone();
        let mut changed = false;
        for (path, data) in &mut restored.files {
            if !session(path) {
                continue;
            }
            let Some(prior) = baseline.files.get(path) else {
                continue;
            };
            let mut value = json(data)?;
            let prior = json(prior)?;
            if let Some(object) = value.as_object_mut()
                && !object.contains_key("armedWorkAtQuit")
                && let Some(marker) = prior.get("armedWorkAtQuit").filter(|v| quit_marker(v))
            {
                object.insert("armedWorkAtQuit".into(), marker.clone());
                *data = serde_json::to_vec(&value)?;
                changed = true;
            }
        }
        Ok(changed && restored.content_fingerprint()? == baseline.content_fingerprint()?)
    }

    pub(super) fn content_fingerprint(&self) -> Result<String> {
        let mut normalized = self.clone();
        for (path, data) in &mut normalized.files {
            if path.extension().is_some_and(|s| s == "json") {
                let mut value = json(data)?;
                if session(path) {
                    focus(&mut value);
                }
                *data = serde_json::to_vec(&value)?;
            }
        }
        if let Some(data) = &mut normalized.worktrees {
            let mut value = json(data)?;
            gc(&mut value);
            *data = serde_json::to_vec(&value)?;
        }
        if let Some(data) = &mut normalized.presets {
            *data = serde_json::to_vec(&json(data)?)?;
        }
        Ok(normalized.fingerprint())
    }

    /// Merge timestamps only into records that still exist in the authority.
    /// In particular, never resurrect a deleted thread or worktree GC entry.
    pub(super) fn merge_housekeeping(&mut self, others: &[Store]) -> Result<()> {
        for (path, data) in &mut self.files {
            if !session(path) {
                continue;
            }
            let mut value = json(data)?;
            let mut changed = false;
            for other in others {
                if let Some(bytes) = other.files.get(path) {
                    let peer = json(bytes)?;
                    if let (Some(target), Some(source)) =
                        (value.get_mut("lastFocusedAt"), peer.get("lastFocusedAt"))
                    {
                        changed |= max_field(target, source);
                    }
                }
            }
            if changed {
                *data = serde_json::to_vec(&value)?;
            }
        }
        if let Some(data) = &mut self.worktrees {
            let mut value = json(data)?;
            let mut changed = false;
            if let Some(cwds) = value
                .pointer_mut("/untrackedDirGc/cwds")
                .and_then(Value::as_object_mut)
            {
                for other in others {
                    if let Some(bytes) = &other.worktrees {
                        let peer = json(bytes)?;
                        if let Some(peer) = peer
                            .pointer("/untrackedDirGc/cwds")
                            .and_then(Value::as_object)
                        {
                            for (key, target) in cwds.iter_mut() {
                                if let Some(source) = peer.get(key) {
                                    changed |= max_field(target, source);
                                }
                            }
                        }
                    }
                }
            }
            if changed {
                *data = serde_json::to_vec(&value)?;
            }
        }
        Ok(())
    }

    pub(super) fn transaction_state(&self, prior: &Store, next: &Store) -> bool {
        (self.files == prior.files || self.files == next.files)
            && (self.worktrees == prior.worktrees || self.worktrees == next.worktrees)
            && (self.presets == prior.presets || self.presets == next.presets)
    }
}
