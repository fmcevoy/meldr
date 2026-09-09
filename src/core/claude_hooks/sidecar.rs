//! Per-session files under `~/.cache/claude-agents/`.
//!
//! Two kinds live here:
//!
//! - `<session_id>.pane.json` — where a session's pane was when it started, so
//!   Stop/Notification events that cannot walk the process tree (a resumed session
//!   whose ancestry has changed) still have somewhere to look.
//! - `<session_id>.json` — session status for dashboards. This is a published
//!   shape; other tools read it.
//!
//! A pane reference is only meaningful for the lifetime of one tmux server: pane
//! ids are handed out from `%0` again each time a server starts, so `%3` recorded
//! last month can name a live, unrelated pane today. Every pane sidecar therefore
//! carries the server's identity and is rejected the moment it stops matching.
//!
//! The predecessor format (`<session_id>.parent_pane`, a bare `%N` with no stamp
//! and no expiry) had neither property and was never collected: 346 files had
//! accumulated, 235 of them naming a pane that existed but belonged to something
//! else entirely. `gc_sidecars` removes them on sight.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::core::fs_util::write_bytes_atomic;
use crate::error::Result;
use crate::tmux::snapshot::ServerStamp;

/// Pane sidecars older than this are dropped even if their server still matches.
pub const PANE_SIDECAR_MAX_AGE_SECS: u64 = 7 * 86_400;

/// Session-state files older than this are dropped. Longer than the pane sidecars
/// because these are a dashboard history, not a routing hint.
pub const SESSION_STATE_MAX_AGE_SECS: u64 = 30 * 86_400;

const PANE_SUFFIX: &str = ".pane.json";
const LEGACY_SUFFIX: &str = ".parent_pane";

/// Where a session's pane was, stamped with the server that named it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSidecar {
    pub pane: String,
    pub window: String,
    /// `#{pid}` of the tmux server that issued `pane`.
    pub server_pid: u32,
    /// `#{start_time}` of that server, to catch a pid reused by a new server.
    pub server_start: u64,
    /// Session cwd at the time of writing, used as a sanity check on reuse.
    pub cwd: String,
    pub ts: u64,
}

impl PaneSidecar {
    /// True when this sidecar was written by the server we are talking to now.
    pub fn matches(&self, stamp: &ServerStamp) -> bool {
        self.server_pid == stamp.pid && self.server_start == stamp.start_time
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn pane_path(state_dir: &Path, session_id: &str) -> PathBuf {
    state_dir.join(format!("{session_id}{PANE_SUFFIX}"))
}

pub fn write_pane_sidecar(state_dir: &Path, session_id: &str, entry: &PaneSidecar) -> Result<()> {
    let json = serde_json::to_string(entry)?;
    write_bytes_atomic(&pane_path(state_dir, session_id), json.as_bytes())
}

/// Read a session's pane sidecar. Returns `None` when absent or unparseable.
pub fn read_pane_sidecar(state_dir: &Path, session_id: &str) -> Option<PaneSidecar> {
    let text = std::fs::read_to_string(pane_path(state_dir, session_id)).ok()?;
    serde_json::from_str(&text).ok()
}

/// What a GC pass removed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GcReport {
    /// Pre-stamp `*.parent_pane` files, which can never be validated.
    pub legacy: usize,
    /// Pane sidecars from a different tmux server.
    pub stale_server: usize,
    /// Pane sidecars past `PANE_SIDECAR_MAX_AGE_SECS`.
    pub aged: usize,
    /// Session-state files past `SESSION_STATE_MAX_AGE_SECS`.
    pub old_state: usize,
    /// Files that could not be parsed at all.
    pub unreadable: usize,
}

impl GcReport {
    pub fn total(&self) -> usize {
        self.legacy + self.stale_server + self.aged + self.old_state + self.unreadable
    }
}

/// Remove sidecars that can no longer name anything real.
///
/// `stamp` is the live tmux server; pass `None` when tmux is unreachable, in which
/// case pane sidecars are left alone (we cannot tell stale from current) but legacy
/// files and ancient state files still go.
pub fn gc_sidecars(state_dir: &Path, stamp: Option<&ServerStamp>, now: u64) -> GcReport {
    let mut report = GcReport::default();
    let Ok(entries) = std::fs::read_dir(state_dir) else {
        return report;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if name.ends_with(LEGACY_SUFFIX) {
            // Unstamped and unverifiable: 68% of these were actively pointing at
            // the wrong live pane. There is nothing to salvage.
            if std::fs::remove_file(&path).is_ok() {
                report.legacy += 1;
            }
            continue;
        }

        if name.ends_with(PANE_SUFFIX) {
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<PaneSidecar>(&t).ok())
            {
                None => {
                    if std::fs::remove_file(&path).is_ok() {
                        report.unreadable += 1;
                    }
                }
                Some(sc) => {
                    if now.saturating_sub(sc.ts) > PANE_SIDECAR_MAX_AGE_SECS {
                        if std::fs::remove_file(&path).is_ok() {
                            report.aged += 1;
                        }
                    } else if let Some(stamp) = stamp
                        && !sc.matches(stamp)
                        && std::fs::remove_file(&path).is_ok()
                    {
                        report.stale_server += 1;
                    }
                }
            }
            continue;
        }

        if name.ends_with(".json") {
            // Dashboard history — age out only.
            let old = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<SessionState>(&t).ok())
                .map(|s| now.saturating_sub(s.ts) > SESSION_STATE_MAX_AGE_SECS)
                .unwrap_or(false);
            if old && std::fs::remove_file(&path).is_ok() {
                report.old_state += 1;
            }
        }
    }

    report
}

/// Count pane sidecars that would be rejected, without removing anything.
/// Used by `meldr doctor hooks` to report drift.
pub fn survey(state_dir: &Path, stamp: Option<&ServerStamp>, now: u64) -> GcReport {
    let mut report = GcReport::default();
    let Ok(entries) = std::fs::read_dir(state_dir) else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(LEGACY_SUFFIX) {
            report.legacy += 1;
        } else if name.ends_with(PANE_SUFFIX) {
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<PaneSidecar>(&t).ok())
            {
                None => report.unreadable += 1,
                Some(sc) if now.saturating_sub(sc.ts) > PANE_SIDECAR_MAX_AGE_SECS => {
                    report.aged += 1
                }
                Some(sc) if matches!(stamp, Some(s) if !sc.matches(s)) => report.stale_server += 1,
                Some(_) => {}
            }
        }
    }
    report
}

/// Per-session status, written on every Stop / Notification.
///
/// Published shape — dashboards read these files, so fields are additive only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub status: String,
    pub ts: u64,
    pub cwd: String,
    /// Resolved pane, or empty when only the window could be determined.
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

    fn stamp(pid: u32, start: u64) -> ServerStamp {
        ServerStamp {
            pid,
            start_time: start,
        }
    }

    fn sidecar(pane: &str, pid: u32, start: u64, ts: u64) -> PaneSidecar {
        PaneSidecar {
            pane: pane.to_string(),
            window: "@1".to_string(),
            server_pid: pid,
            server_start: start,
            cwd: "/w".to_string(),
            ts,
        }
    }

    #[test]
    fn pane_sidecar_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let sc = sidecar("%5", 4242, 999, 1000);
        write_pane_sidecar(dir.path(), "sess", &sc).unwrap();
        assert_eq!(read_pane_sidecar(dir.path(), "sess"), Some(sc));
    }

    #[test]
    fn read_missing_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(read_pane_sidecar(dir.path(), "nope").is_none());
    }

    #[test]
    fn matches_only_the_issuing_server() {
        let sc = sidecar("%5", 100, 500, 0);
        assert!(sc.matches(&stamp(100, 500)));
        assert!(!sc.matches(&stamp(101, 500)), "different pid");
        // A recycled pid with a new start time must not validate — this is the
        // case that let month-old pane ids look alive.
        assert!(!sc.matches(&stamp(100, 501)), "pid reused by a new server");
    }

    // ── gc ────────────────────────────────────────────────────────────────────

    #[test]
    fn gc_removes_every_legacy_parent_pane_file() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.parent_pane"), "%1").unwrap();
        std::fs::write(dir.path().join("b.parent_pane"), "%2").unwrap();
        let r = gc_sidecars(dir.path(), Some(&stamp(1, 1)), 0);
        assert_eq!(r.legacy, 2);
        assert!(!dir.path().join("a.parent_pane").exists());
    }

    #[test]
    fn gc_removes_sidecars_from_another_server() {
        let dir = tempfile::TempDir::new().unwrap();
        write_pane_sidecar(dir.path(), "old", &sidecar("%3", 111, 10, 1_000)).unwrap();
        write_pane_sidecar(dir.path(), "cur", &sidecar("%3", 222, 20, 1_000)).unwrap();

        let r = gc_sidecars(dir.path(), Some(&stamp(222, 20)), 1_000);
        assert_eq!(r.stale_server, 1);
        assert!(read_pane_sidecar(dir.path(), "old").is_none());
        assert!(read_pane_sidecar(dir.path(), "cur").is_some());
    }

    #[test]
    fn gc_ages_out_sidecars_even_when_server_matches() {
        let dir = tempfile::TempDir::new().unwrap();
        write_pane_sidecar(dir.path(), "s", &sidecar("%1", 1, 1, 0)).unwrap();
        let r = gc_sidecars(
            dir.path(),
            Some(&stamp(1, 1)),
            PANE_SIDECAR_MAX_AGE_SECS + 1,
        );
        assert_eq!(r.aged, 1);
        assert!(read_pane_sidecar(dir.path(), "s").is_none());
    }

    #[test]
    fn gc_keeps_a_fresh_matching_sidecar() {
        let dir = tempfile::TempDir::new().unwrap();
        write_pane_sidecar(dir.path(), "s", &sidecar("%1", 7, 7, 1_000)).unwrap();
        let r = gc_sidecars(dir.path(), Some(&stamp(7, 7)), 1_050);
        assert_eq!(r.total(), 0);
        assert!(read_pane_sidecar(dir.path(), "s").is_some());
    }

    #[test]
    fn gc_removes_unparseable_pane_sidecars() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("junk.pane.json"), "{not json").unwrap();
        let r = gc_sidecars(dir.path(), Some(&stamp(1, 1)), 0);
        assert_eq!(r.unreadable, 1);
    }

    #[test]
    fn gc_without_a_stamp_leaves_pane_sidecars_alone() {
        // tmux unreachable: we cannot tell stale from current, so don't guess.
        let dir = tempfile::TempDir::new().unwrap();
        write_pane_sidecar(dir.path(), "s", &sidecar("%1", 1, 1, 1_000)).unwrap();
        std::fs::write(dir.path().join("legacy.parent_pane"), "%9").unwrap();

        let r = gc_sidecars(dir.path(), None, 1_000);
        assert_eq!(r.stale_server, 0);
        assert_eq!(r.legacy, 1, "legacy files are unverifiable regardless");
        assert!(read_pane_sidecar(dir.path(), "s").is_some());
    }

    #[test]
    fn gc_ages_out_session_state_but_keeps_recent() {
        let dir = tempfile::TempDir::new().unwrap();
        let mk = |sid: &str, ts: u64| {
            write_session_state(
                dir.path(),
                sid,
                &SessionState {
                    status: "done".into(),
                    ts,
                    cwd: "/w".into(),
                    pane: "%1".into(),
                    window: "@1".into(),
                    window_name: "w".into(),
                },
            )
            .unwrap();
        };
        mk("recent", 1_000);
        mk("ancient", 0);

        let r = gc_sidecars(
            dir.path(),
            Some(&stamp(1, 1)),
            1_000 + SESSION_STATE_MAX_AGE_SECS,
        );
        assert_eq!(r.old_state, 1);
        assert!(dir.path().join("recent.json").exists());
        assert!(!dir.path().join("ancient.json").exists());
    }

    #[test]
    fn gc_does_not_mistake_a_pane_sidecar_for_session_state() {
        // `*.pane.json` also ends in `.json`; the pane branch must claim it first,
        // otherwise a fresh pane sidecar would be judged by the 30-day rule.
        let dir = tempfile::TempDir::new().unwrap();
        write_pane_sidecar(dir.path(), "s", &sidecar("%1", 1, 1, 1_000)).unwrap();
        let r = gc_sidecars(dir.path(), Some(&stamp(1, 1)), 1_000);
        assert_eq!(r.old_state, 0);
        assert!(read_pane_sidecar(dir.path(), "s").is_some());
    }

    #[test]
    fn gc_on_a_missing_directory_is_a_noop() {
        let r = gc_sidecars(Path::new("/nonexistent-meldr-gc"), None, 0);
        assert_eq!(r.total(), 0);
    }

    // ── survey ────────────────────────────────────────────────────────────────

    #[test]
    fn survey_counts_without_deleting() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.parent_pane"), "%1").unwrap();
        write_pane_sidecar(dir.path(), "old", &sidecar("%3", 111, 10, 1_000)).unwrap();

        let r = survey(dir.path(), Some(&stamp(222, 20)), 1_000);
        assert_eq!((r.legacy, r.stale_server), (1, 1));
        assert!(
            dir.path().join("a.parent_pane").exists(),
            "survey is read-only"
        );
        assert!(read_pane_sidecar(dir.path(), "old").is_some());
    }

    // ── misc ──────────────────────────────────────────────────────────────────

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
