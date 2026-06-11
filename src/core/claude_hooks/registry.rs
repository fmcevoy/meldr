/// Launcher-registry helpers: one JSON entry per `claude agents` launch.
///
/// Each entry records the tmux pane and window where `claude agents` was started,
/// together with the working directory and a millisecond timestamp. The SessionStart
/// hook resolves the right pane by matching the new session's cwd against these
/// entries using component-aware path prefix matching (`Path::starts_with`), which
/// naturally avoids the sibling-prefix bug (e.g. `~/fmcevoy` must NOT match
/// `~/fmcevoy_tools/…`).
///
/// Entry files are written atomically under `<state_dir>/launchers/` and are
/// backward-compatible with the JSON format previously produced by the shell wrapper
/// in `~/fmcevoy_tools`.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::core::fs_util::write_bytes_atomic;
use crate::tmux::TmuxOps;

/// One entry written to `<state_dir>/launchers/<ts>-<pid>-<seq>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherEntry {
    pub pane: String,
    pub window: String,
    pub cwd: String,
    pub ts: u64,
}

impl LauncherEntry {
    pub fn cwd_path(&self) -> PathBuf {
        PathBuf::from(&self.cwd)
    }
}

/// Atomically write a launcher entry for the current invocation.
pub fn write_entry(
    launcher_dir: &Path,
    pane: &str,
    window: &str,
    cwd: &Path,
) -> crate::error::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    std::fs::create_dir_all(launcher_dir)?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let pid = std::process::id();
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);

    let entry = LauncherEntry {
        pane: pane.to_string(),
        window: window.to_string(),
        cwd: cwd.to_string_lossy().into_owned(),
        ts,
    };
    let content = serde_json::to_string(&entry)?;
    let filename = format!("{ts}-{pid}-{seq}.json");
    let path = launcher_dir.join(filename);
    write_bytes_atomic(&path, content.as_bytes())
}

/// Read all valid launcher entries from `launcher_dir`. Silently skips malformed files.
pub fn list_entries(launcher_dir: &Path) -> Vec<LauncherEntry> {
    let Ok(rd) = std::fs::read_dir(launcher_dir) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(e) = serde_json::from_str::<LauncherEntry>(&text)
        {
            entries.push(e);
        }
    }
    entries
}

/// Find the best launcher entry for a session whose working directory is `session_cwd`.
///
/// Selection criteria (in order of priority):
/// 1. The launcher's cwd must be a path-component–bounded prefix of `session_cwd`
///    (`Path::starts_with`, which is component-aware, not byte-prefix matching).
/// 2. The pane must still be alive in tmux.
/// 3. Among remaining candidates: longest cwd (most specific) wins; ties broken by
///    most-recent timestamp.
pub fn find_best_match(
    launcher_dir: &Path,
    session_cwd: &Path,
    tmux: &dyn TmuxOps,
) -> Option<LauncherEntry> {
    let candidates: Vec<LauncherEntry> = list_entries(launcher_dir)
        .into_iter()
        .filter(|e| {
            // Component-aware prefix match — fixes the ~/fmcevoy vs ~/fmcevoy_tools bug.
            session_cwd.starts_with(e.cwd_path()) && tmux.pane_exists(&e.pane)
        })
        .collect();

    candidates.into_iter().max_by(|a, b| {
        // Prefer deeper (more specific) cwd first; break ties by newer timestamp.
        let depth_a = a.cwd_path().components().count();
        let depth_b = b.cwd_path().components().count();
        depth_a.cmp(&depth_b).then(a.ts.cmp(&b.ts))
    })
}

/// Remove launcher entries that are older than `max_age_secs` seconds or whose
/// pane no longer exists in tmux. Silently ignores removal errors.
pub fn gc(launcher_dir: &Path, max_age_secs: u64, tmux: &dyn TmuxOps) {
    let Ok(rd) = std::fs::read_dir(launcher_dir) else {
        return;
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let max_age_ms = max_age_secs * 1000;

    for dir_entry in rd.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let should_remove = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<LauncherEntry>(&text).ok())
            .map(|e| {
                let age_ms = now_ms.saturating_sub(e.ts);
                age_ms > max_age_ms || !tmux.pane_exists(&e.pane)
            })
            .unwrap_or(true); // Remove unreadable / malformed entries.
        if should_remove {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::RecordingTmux;

    fn make_entry(dir: &Path, pane: &str, cwd: &str, ts: u64) {
        let e = LauncherEntry {
            pane: pane.to_string(),
            window: "@1".to_string(),
            cwd: cwd.to_string(),
            ts,
        };
        let content = serde_json::to_string(&e).unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{ts}-1-0.json")), content).unwrap();
    }

    // ── path matching ─────────────────────────────────────────────────────────

    #[test]
    fn sibling_prefix_not_matched() {
        // ~/fmcevoy should NOT match a session cwd under ~/fmcevoy_tools
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        make_entry(&launcher_dir, "%1", "/home/user/fmcevoy", 1000);
        make_entry(&launcher_dir, "%2", "/home/user/fmcevoy_tools", 2000);

        let tmux = RecordingTmux::new(vec!["%1".to_string(), "%2".to_string()]);
        let result = find_best_match(
            &launcher_dir,
            &PathBuf::from("/home/user/fmcevoy_tools/some/project"),
            &tmux,
        );
        assert_eq!(
            result.unwrap().pane,
            "%2",
            "must match fmcevoy_tools, not fmcevoy"
        );
    }

    #[test]
    fn exact_cwd_match() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        make_entry(&launcher_dir, "%3", "/home/user/project", 1000);

        let tmux = RecordingTmux::new(vec!["%3".to_string()]);
        let result = find_best_match(&launcher_dir, &PathBuf::from("/home/user/project"), &tmux);
        assert_eq!(result.unwrap().pane, "%3");
    }

    #[test]
    fn longer_match_wins_over_shorter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        // Two launchers: one shallower, one deeper — deeper should win.
        make_entry(&launcher_dir, "%1", "/home/user", 1000);
        make_entry(&launcher_dir, "%2", "/home/user/project", 2000);

        let tmux = RecordingTmux::new(vec!["%1".to_string(), "%2".to_string()]);
        let result = find_best_match(
            &launcher_dir,
            &PathBuf::from("/home/user/project/subdir"),
            &tmux,
        );
        assert_eq!(result.unwrap().pane, "%2", "deeper cwd should win");
    }

    #[test]
    fn most_recent_wins_when_depth_equal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        make_entry(&launcher_dir, "%1", "/home/user/project", 1000);
        make_entry(&launcher_dir, "%2", "/home/user/project", 2000);

        let tmux = RecordingTmux::new(vec!["%1".to_string(), "%2".to_string()]);
        let result = find_best_match(&launcher_dir, &PathBuf::from("/home/user/project"), &tmux);
        assert_eq!(
            result.unwrap().pane,
            "%2",
            "most recent should win when depth equal"
        );
    }

    #[test]
    fn dead_pane_filtered_out() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        make_entry(&launcher_dir, "%1", "/home/user/project", 1000);

        // %1 is NOT in live_panes
        let tmux = RecordingTmux::new(vec![]);
        let result = find_best_match(&launcher_dir, &PathBuf::from("/home/user/project"), &tmux);
        assert!(result.is_none(), "dead pane must be filtered");
    }

    #[test]
    fn no_match_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        make_entry(&launcher_dir, "%1", "/home/user/other", 1000);

        let tmux = RecordingTmux::new(vec!["%1".to_string()]);
        let result = find_best_match(&launcher_dir, &PathBuf::from("/home/user/project"), &tmux);
        assert!(result.is_none());
    }

    // ── gc ────────────────────────────────────────────────────────────────────

    #[test]
    fn gc_removes_old_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        // ts=1 ms → way older than 7 days
        make_entry(&launcher_dir, "%1", "/proj", 1);

        let tmux = RecordingTmux::new(vec!["%1".to_string()]);
        gc(&launcher_dir, 7 * 86400, &tmux);

        let remaining = list_entries(&launcher_dir);
        assert!(remaining.is_empty(), "old entry should be removed");
    }

    #[test]
    fn gc_removes_dead_pane_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        make_entry(&launcher_dir, "%99", "/proj", now_ms);

        // %99 is dead
        let tmux = RecordingTmux::new(vec![]);
        gc(&launcher_dir, 7 * 86400, &tmux);

        let remaining = list_entries(&launcher_dir);
        assert!(remaining.is_empty(), "dead pane entry should be removed");
    }

    #[test]
    fn gc_keeps_fresh_live_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        make_entry(&launcher_dir, "%5", "/proj", now_ms);

        let tmux = RecordingTmux::new(vec!["%5".to_string()]);
        gc(&launcher_dir, 7 * 86400, &tmux);

        let remaining = list_entries(&launcher_dir);
        assert_eq!(remaining.len(), 1, "fresh live entry should be kept");
    }
}
