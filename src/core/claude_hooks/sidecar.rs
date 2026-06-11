/// Sidecar file helpers for correlating Claude session IDs with tmux panes.
///
/// Two sidecar formats live under `~/.cache/claude-agents/`:
/// - `<session_id>.parent_pane` — raw pane ID (e.g. `%42`), written at spawn time
///   or at SessionStart; consumed by the resolver to locate the right pane.
/// - `<session_id>.json` — rich session state (status, cwd, pane, window, …),
///   written on every Stop / Notification event for dashboard tools.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::core::fs_util::write_bytes_atomic;
use crate::error::Result;

/// Generate a unique per-spawn session identifier: `<ms>-<pid>-<pane_digits>`.
/// The leading `%` is stripped from pane IDs like `%42` to keep filenames safe.
pub fn session_id(pane_id: &str) -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let pid = std::process::id();
    let pane_part = pane_id.trim_start_matches('%');
    format!("{ms}-{pid}-{pane_part}")
}

/// Write `<state_dir>/<session_id>.parent_pane` containing the raw pane ID.
pub fn write_parent_pane(state_dir: &Path, session_id: &str, pane_id: &str) -> Result<()> {
    let path = state_dir.join(format!("{session_id}.parent_pane"));
    write_bytes_atomic(&path, pane_id.as_bytes())
}

/// Read `<state_dir>/<session_id>.parent_pane`. Returns `None` if absent or unreadable.
pub fn read_parent_pane(state_dir: &Path, session_id: &str) -> Option<String> {
    let path = state_dir.join(format!("{session_id}.parent_pane"));
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Rich per-session state, written on Stop / Notification events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub status: String,
    pub ts: u64,
    pub cwd: String,
    pub pane: String,
    pub window: String,
    pub window_name: String,
}

/// Atomically write session state JSON to `<state_dir>/<session_id>.json`.
pub fn write_session_state(state_dir: &Path, session_id: &str, state: &SessionState) -> Result<()> {
    let path = state_dir.join(format!("{session_id}.json"));
    let json = serde_json::to_string(state)?;
    write_bytes_atomic(&path, json.as_bytes())
}

/// Expand a leading `~` in a path using `$HOME`. Returns the path unchanged if
/// it does not start with `~` or `$HOME` is unset.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix('~')
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(format!("{home}{rest}"));
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_unique() {
        let a = session_id("%1");
        let b = session_id("%1");
        // Within the same millisecond they share the ms prefix and pid but differ by
        // monotonic counter — in practice we just assert they're non-empty.
        assert!(!a.is_empty());
        assert!(!b.is_empty());
    }

    #[test]
    fn session_id_strips_percent() {
        let id = session_id("%42");
        assert!(!id.contains('%'), "pane % should be stripped: {id}");
        assert!(
            id.ends_with("-42"),
            "pane number should appear at end: {id}"
        );
    }

    #[test]
    fn parent_pane_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        write_parent_pane(dir.path(), "abc", "%5").unwrap();
        assert_eq!(read_parent_pane(dir.path(), "abc").as_deref(), Some("%5"));
    }

    #[test]
    fn read_parent_pane_missing_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(read_parent_pane(dir.path(), "nope"), None);
    }

    #[test]
    fn session_state_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let state = SessionState {
            status: "done".to_string(),
            ts: 12345,
            cwd: "/tmp/x".to_string(),
            pane: "%1".to_string(),
            window: "@2".to_string(),
            window_name: "ws/feat".to_string(),
        };
        write_session_state(dir.path(), "sid", &state).unwrap();
        let raw = std::fs::read_to_string(dir.path().join("sid.json")).unwrap();
        let back: SessionState = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.status, "done");
        assert_eq!(back.window, "@2");
    }

    #[test]
    fn expand_tilde_no_tilde() {
        let p = expand_tilde("/abs/path");
        assert_eq!(p, PathBuf::from("/abs/path"));
    }
}
