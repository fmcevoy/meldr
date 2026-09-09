//! Parent-process lookup, used to answer "which tmux pane is this hook running in?"
//! without asking the environment.
//!
//! Claude Code runs a hook as a child of itself, and an interactive `claude` is a
//! child of its pane's shell — which is exactly `#{pane_pid}`. So walking `ppid`
//! upwards from the hook lands on a pane row in a handful of hops, and the answer
//! comes from the kernel rather than from an exported variable that may have been
//! inherited by an unrelated process minutes or months ago.
//!
//! The walk deliberately dead-ends for Claude's detached background-job daemon
//! (reparented to pid 1): there genuinely is no pane above it, and reporting "no
//! pane" is the correct answer rather than picking one.

use std::collections::HashMap;

use crate::tmux::snapshot::{PaneRow, TmuxSnapshot};

/// Maximum ancestry hops before giving up. Real chains are 2–4 deep; the cap only
/// matters as protection against a pid cycle produced by pid reuse mid-walk.
pub const MAX_HOPS: usize = 64;

pub trait ProcTable: Send + Sync {
    /// Parent pid of `pid`, or `None` when the process is gone or has no parent.
    fn ppid(&self, pid: u32) -> Option<u32>;
}

/// `ProcTable` backed by the running system.
#[derive(Default)]
pub struct RealProcTable {
    /// macOS only: one `ps` call, cached. Linux reads `/proc` per pid instead, so
    /// nothing is cached there and no external binary is required — the test
    /// container has no `ps`.
    #[cfg(not(target_os = "linux"))]
    cache: std::sync::OnceLock<HashMap<u32, u32>>,
}

impl RealProcTable {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ProcTable for RealProcTable {
    #[cfg(target_os = "linux")]
    fn ppid(&self, pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        parse_proc_stat_ppid(&stat)
    }

    #[cfg(not(target_os = "linux"))]
    fn ppid(&self, pid: u32) -> Option<u32> {
        let table = self.cache.get_or_init(|| {
            std::process::Command::new("ps")
                .args(["-axo", "pid=,ppid="])
                .output()
                .ok()
                .map(|o| parse_ps_table(&String::from_utf8_lossy(&o.stdout)))
                .unwrap_or_default()
        });
        table.get(&pid).copied()
    }
}

/// Extract the ppid from a `/proc/<pid>/stat` line.
///
/// Field 2 is the executable name in parentheses and may itself contain spaces and
/// parentheses (`(my (odd) prog)`), so everything up to the *last* `)` is skipped
/// before counting fields: state is then first, ppid second.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_proc_stat_ppid(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Parse `ps -axo pid=,ppid=` output into a pid → ppid map.
pub fn parse_ps_table(out: &str) -> HashMap<u32, u32> {
    out.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            Some((pid, ppid))
        })
        .collect()
}

/// Walk up the process tree from `start` until a pid matches a pane's `pane_pid`.
///
/// Returns the pane the process is running inside, or `None` when the chain
/// reaches init, dies, loops, or exceeds `MAX_HOPS` without touching a pane.
pub fn walk_to_pane<'a>(
    procs: &dyn ProcTable,
    snapshot: &'a TmuxSnapshot,
    start: u32,
) -> Option<&'a PaneRow> {
    let mut pid = start;
    let mut seen = Vec::with_capacity(8);

    for _ in 0..MAX_HOPS {
        if let Some(row) = snapshot.by_pid(pid) {
            return Some(row);
        }
        if pid <= 1 || seen.contains(&pid) {
            return None;
        }
        seen.push(pid);
        pid = procs.ppid(pid)?;
    }
    None
}

/// `ProcTable` over a fixed pid → ppid map.
#[cfg(test)]
pub struct FakeProcTable(pub HashMap<u32, u32>);

#[cfg(test)]
impl FakeProcTable {
    pub fn new(pairs: &[(u32, u32)]) -> Self {
        Self(pairs.iter().copied().collect())
    }
}

#[cfg(test)]
impl ProcTable for FakeProcTable {
    fn ppid(&self, pid: u32) -> Option<u32> {
        self.0.get(&pid).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::snapshot::{FS, ServerStamp, parse_list_panes};

    fn snapshot(rows: &[(&str, &str, &str, u32)]) -> TmuxSnapshot {
        // (pane_id, window_id, cwd, pane_pid)
        let text = rows
            .iter()
            .map(|(pane, win, cwd, pid)| {
                [pid.to_string().as_str(), pane, win, "$0", cwd, "zsh", "w"].join(&FS.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n");
        TmuxSnapshot {
            stamp: ServerStamp {
                pid: 1,
                start_time: 1,
            },
            panes: parse_list_panes(&text),
            env_stale: false,
        }
    }

    // ── /proc parsing ─────────────────────────────────────────────────────────

    #[test]
    fn proc_stat_simple() {
        assert_eq!(parse_proc_stat_ppid("1234 (zsh) S 1200 1234 …"), Some(1200));
    }

    #[test]
    fn proc_stat_comm_with_spaces_and_parens() {
        // The reason we scan to the last ')' rather than splitting on whitespace.
        let stat = "42 (my (odd) prog) S 7 42 7 0 -1 4194304";
        assert_eq!(parse_proc_stat_ppid(stat), Some(7));
    }

    #[test]
    fn proc_stat_garbage() {
        assert_eq!(parse_proc_stat_ppid(""), None);
        assert_eq!(parse_proc_stat_ppid("no parens here"), None);
        assert_eq!(parse_proc_stat_ppid("1 (x) S notanumber"), None);
    }

    // ── ps parsing ────────────────────────────────────────────────────────────

    #[test]
    fn ps_table() {
        let t = parse_ps_table("    1     0\n  145     1\n88650 29951\n");
        assert_eq!(t.get(&1), Some(&0));
        assert_eq!(t.get(&88650), Some(&29951));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn ps_table_skips_junk_lines() {
        let t = parse_ps_table("PID PPID\n  10   5\n\nbroken\n");
        assert_eq!(t, [(10u32, 5u32)].into_iter().collect::<HashMap<_, _>>());
    }

    // ── walk ──────────────────────────────────────────────────────────────────

    #[test]
    fn walk_finds_pane_at_depth_one() {
        // claude launched directly by the pane shell.
        let snap = snapshot(&[("%9", "@1", "/w", 100)]);
        let procs = FakeProcTable::new(&[(200, 100), (100, 1)]);
        assert_eq!(walk_to_pane(&procs, &snap, 200).unwrap().pane_id, "%9");
    }

    #[test]
    fn walk_finds_pane_at_depth_three() {
        // hook -> sh -c -> claude -> pane shell: the real interactive shape.
        let snap = snapshot(&[("%9", "@1", "/w", 100)]);
        let procs = FakeProcTable::new(&[(400, 300), (300, 200), (200, 100), (100, 1)]);
        let row = walk_to_pane(&procs, &snap, 400).unwrap();
        assert_eq!((row.pane_id.as_str(), row.window_id.as_str()), ("%9", "@1"));
    }

    #[test]
    fn walk_returns_the_pane_itself_when_started_from_pane_pid() {
        let snap = snapshot(&[("%0", "@0", "/w", 55)]);
        let procs = FakeProcTable::new(&[(55, 1)]);
        assert_eq!(walk_to_pane(&procs, &snap, 55).unwrap().pane_id, "%0");
    }

    #[test]
    fn walk_dead_ends_for_detached_daemon() {
        // Claude's background-job host is reparented to pid 1; no pane owns it, so
        // the honest answer is None rather than some nearby pane.
        let snap = snapshot(&[("%9", "@1", "/w", 100)]);
        let procs = FakeProcTable::new(&[(900, 800), (800, 1)]);
        assert!(walk_to_pane(&procs, &snap, 900).is_none());
    }

    #[test]
    fn walk_stops_when_a_pid_has_no_parent() {
        let snap = snapshot(&[("%9", "@1", "/w", 100)]);
        let procs = FakeProcTable::new(&[(900, 800)]); // 800 unknown
        assert!(walk_to_pane(&procs, &snap, 900).is_none());
    }

    #[test]
    fn walk_terminates_on_a_cycle() {
        let snap = snapshot(&[("%9", "@1", "/w", 100)]);
        let procs = FakeProcTable::new(&[(10, 11), (11, 12), (12, 10)]);
        assert!(walk_to_pane(&procs, &snap, 10).is_none());
    }

    #[test]
    fn walk_picks_the_nearest_pane_ancestor() {
        // Two panes in the same chain (nested tmux): the closest one wins.
        let snap = snapshot(&[("%1", "@0", "/w", 10), ("%2", "@1", "/w", 30)]);
        let procs = FakeProcTable::new(&[(50, 40), (40, 30), (30, 20), (20, 10), (10, 1)]);
        assert_eq!(walk_to_pane(&procs, &snap, 50).unwrap().pane_id, "%2");
    }
}
