//! Work out which tmux pane a Claude hook event belongs to.
//!
//! # Why this looks the way it does
//!
//! The previous design asked five sources in turn — two environment variables, two
//! sidecar files, and a registry of past launches matched by working directory —
//! and took the first that looked plausible. Each source could go stale on its own,
//! nothing cross-checked them, and a wrong answer was indistinguishable from a
//! right one, so notifications routinely landed on unrelated windows.
//!
//! This version never accepts a hint on its own authority. Every answer is a row
//! from one `list-panes` snapshot, and every hint is only a *lookup key* into that
//! snapshot — so a resolved pane always carries the window it is genuinely in, and
//! a hint that no longer corresponds to anything is simply discarded.
//!
//! # Tiers
//!
//! - **P — process tree.** Walk `ppid` from this process until a pid matches a
//!   pane's `#{pane_pid}`. Hooks run as children of Claude Code, and an interactive
//!   `claude` is a child of its pane's shell, so this is the kernel's own answer to
//!   "where am I running". Needs no configuration and cannot go stale.
//! - **E — validated environment.** `TMUX_PANE`, but only if it names a pane that
//!   is in the snapshot *and* whose directory is consistent with the event's cwd.
//!   Notably absent: `MELDR_TMUX_PANE`. Claude hosts every background job in one
//!   long-lived daemon that inherited that variable from whichever pane happened to
//!   start it, so it is actively misleading and is no longer read anywhere.
//! - **S — stamped sidecar.** What SessionStart recorded, valid only while the tmux
//!   server that issued the pane id is still the one running.
//! - **C — working directory.** Panes whose cwd matches the event's. Usually the
//!   only thing available for a background job, since its daemon has no pane
//!   ancestry at all. When several panes share the directory the pane is genuinely
//!   unknown, so this yields the *window* rather than picking one.
//!
//! When nothing resolves, that is reported as `None` with a reason. Refusing to
//! flash is always better than flashing the wrong thing.

use std::path::{Path, PathBuf};

use crate::core::proc_table::{ProcTable, walk_to_pane};
use crate::tmux::TmuxOps;
use crate::tmux::snapshot::{PaneRow, TmuxSnapshot, is_ancestor_or_equal, is_pane_id, normalize};

use super::sidecar;

/// A resolved tmux pane, always copied out of a snapshot row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRef {
    pub pane_id: String,
    pub window_id: String,
    pub window_name: String,
}

impl From<&PaneRow> for PaneRef {
    fn from(row: &PaneRow) -> Self {
        Self {
            pane_id: row.pane_id.clone(),
            window_id: row.window_id.clone(),
            window_name: row.window_name.clone(),
        }
    }
}

/// How much we were able to determine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The exact pane.
    Pane(PaneRef),
    /// The right window, but the pane within it is genuinely ambiguous.
    Window {
        window_id: String,
        window_name: String,
    },
    /// Nothing trustworthy. Carries the reason so `doctor` can explain it.
    None(NoMatch),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoMatch {
    /// tmux is not running, or could not be queried.
    NoTmux(String),
    /// No hint of any kind was available.
    NoHints,
    /// A cwd was known but matched no live pane.
    NoPaneForCwd(PathBuf),
    /// The cwd matched panes spread across several windows.
    AmbiguousWindows(Vec<String>),
}

/// Whether the event came from a session attached to a terminal, or from Claude's
/// detached background-job host. Drives a visually distinct indicator so a
/// background job finishing is not mistaken for the agent in front of you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Interactive,
    Background,
}

/// Which tier produced the answer — reported by `doctor` and the self-test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    ProcessTree,
    Env,
    Sidecar,
    Cwd,
    NoneOfThem,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::ProcessTree => "process-tree",
            Tier::Env => "env",
            Tier::Sidecar => "sidecar",
            Tier::Cwd => "cwd",
            Tier::NoneOfThem => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub resolution: Resolution,
    pub source: Source,
    pub tier: Tier,
}

impl Resolved {
    pub fn pane(&self) -> Option<&PaneRef> {
        match &self.resolution {
            Resolution::Pane(p) => Some(p),
            _ => None,
        }
    }

    /// The window this event concerns, whether or not the pane is known.
    pub fn window_id(&self) -> Option<&str> {
        match &self.resolution {
            Resolution::Pane(p) => Some(&p.window_id),
            Resolution::Window { window_id, .. } => Some(window_id),
            Resolution::None(_) => None,
        }
    }

    pub fn window_name(&self) -> &str {
        match &self.resolution {
            Resolution::Pane(p) => &p.window_name,
            Resolution::Window { window_name, .. } => window_name,
            Resolution::None(_) => "",
        }
    }
}

/// Reading environment variables, injectable for tests.
pub trait Env: Send + Sync {
    fn var(&self, key: &str) -> Option<String>;
}

/// `Env` backed by the real process environment.
pub struct RealEnv;

impl Env for RealEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// `Env` over a fixed map. Owned `String`s so tests can use runtime pane ids.
#[cfg(test)]
#[derive(Default)]
pub struct FakeEnv(pub std::collections::HashMap<String, String>);

#[cfg(test)]
impl FakeEnv {
    pub fn new(pairs: &[(&str, &str)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
}

#[cfg(test)]
impl Env for FakeEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

/// Commands that plausibly host a Claude session, used only to narrow an
/// otherwise-ambiguous cwd match. Never used to widen one.
fn looks_like_agent(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    c.starts_with("claude") || c == "node" || c.starts_with("bun")
}

pub struct PaneResolver<'a> {
    pub env: &'a dyn Env,
    pub tmux: &'a dyn TmuxOps,
    pub procs: &'a dyn ProcTable,
    /// Directory holding sidecar files (`~/.cache/claude-agents`).
    pub state_dir: &'a Path,
    pub payload_session_id: Option<&'a str>,
    pub payload_cwd: Option<&'a str>,
    /// PID to start the process-tree walk from — normally this process.
    pub own_pid: u32,
}

impl<'a> PaneResolver<'a> {
    pub fn resolve(&self) -> Resolved {
        let snapshot = match self.tmux.snapshot() {
            Ok(s) => s,
            Err(e) => {
                return Resolved {
                    resolution: Resolution::None(NoMatch::NoTmux(e.to_string())),
                    source: Source::Interactive,
                    tier: Tier::NoneOfThem,
                };
            }
        };
        self.resolve_with(&snapshot)
    }

    /// Resolve against an already-taken snapshot.
    pub fn resolve_with(&self, snapshot: &TmuxSnapshot) -> Resolved {
        // Claude sets this in sessions it hosts itself rather than in a terminal.
        // It is undocumented, so it only reorders the tiers and labels the source —
        // correctness never depends on it.
        let child_session = self
            .env
            .var("CLAUDE_CODE_CHILD_SESSION")
            .is_some_and(|v| !v.is_empty());

        let cwd = self
            .payload_cwd
            .filter(|s| !s.is_empty())
            .map(|s| normalize(Path::new(s)));

        // For a child session the process tree leads to whichever pane launched the
        // daemon — which may be a completely unrelated project — so ask the working
        // directory first. Otherwise the process tree is the most reliable thing we
        // have and goes first.
        let order: [Tier; 4] = if child_session {
            [Tier::Cwd, Tier::ProcessTree, Tier::Env, Tier::Sidecar]
        } else {
            [Tier::ProcessTree, Tier::Env, Tier::Sidecar, Tier::Cwd]
        };

        let mut last_failure: Option<NoMatch> = None;

        for tier in order {
            let outcome = match tier {
                Tier::ProcessTree => self.tier_process_tree(snapshot).map(Resolution::Pane),
                Tier::Env => self
                    .tier_env(snapshot, cwd.as_deref())
                    .map(Resolution::Pane),
                Tier::Sidecar => self
                    .tier_sidecar(snapshot, cwd.as_deref())
                    .map(Resolution::Pane),
                Tier::Cwd => match cwd.as_deref() {
                    None => None,
                    Some(cwd) => match self.tier_cwd(snapshot, cwd) {
                        Ok(res) => Some(res),
                        Err(reason) => {
                            last_failure = Some(reason);
                            None
                        }
                    },
                },
                Tier::NoneOfThem => None,
            };

            if let Some(resolution) = outcome {
                // A cwd-only answer means we never proved which process this was;
                // treat it as a background job so the indicator says so.
                let source = if child_session || tier == Tier::Cwd {
                    Source::Background
                } else {
                    Source::Interactive
                };
                return Resolved {
                    resolution,
                    source,
                    tier,
                };
            }
        }

        let reason = last_failure.unwrap_or(match cwd {
            Some(c) => NoMatch::NoPaneForCwd(c),
            None => NoMatch::NoHints,
        });
        Resolved {
            resolution: Resolution::None(reason),
            source: if child_session {
                Source::Background
            } else {
                Source::Interactive
            },
            tier: Tier::NoneOfThem,
        }
    }

    /// Tier P — ask the kernel.
    fn tier_process_tree(&self, snapshot: &TmuxSnapshot) -> Option<PaneRef> {
        walk_to_pane(self.procs, snapshot, self.own_pid).map(PaneRef::from)
    }

    /// Tier E — `TMUX_PANE`, validated against the snapshot.
    fn tier_env(&self, snapshot: &TmuxSnapshot, cwd: Option<&Path>) -> Option<PaneRef> {
        if snapshot.env_stale {
            // `$TMUX` names a server that is no longer running; anything else the
            // environment says about panes is from that same dead server.
            return None;
        }
        let pane_id = self.env.var("TMUX_PANE").filter(|s| is_pane_id(s))?;
        let row = snapshot.pane(&pane_id)?;
        self.cwd_consistent(row, cwd).then(|| PaneRef::from(row))
    }

    /// Tier S — what SessionStart recorded, if it is still about this server.
    fn tier_sidecar(&self, snapshot: &TmuxSnapshot, cwd: Option<&Path>) -> Option<PaneRef> {
        let sid = self.payload_session_id.filter(|s| !s.is_empty())?;
        let sc = sidecar::read_pane_sidecar(self.state_dir, sid)?;
        if !sc.matches(&snapshot.stamp) {
            return None;
        }
        let row = snapshot.pane(&sc.pane)?;
        self.cwd_consistent(row, cwd).then(|| PaneRef::from(row))
    }

    /// Tier C — which live panes are sitting in this directory?
    fn tier_cwd(
        &self,
        snapshot: &TmuxSnapshot,
        cwd: &Path,
    ) -> std::result::Result<Resolution, NoMatch> {
        // Prefer panes whose directory is exactly the event's; fall back to the
        // deepest ancestor, which covers a session started above its own subdir.
        let mut candidates: Vec<&PaneRow> = snapshot
            .panes
            .iter()
            .filter(|p| normalize(&p.cwd) == cwd)
            .collect();

        if candidates.is_empty() {
            let mut ancestors: Vec<&PaneRow> = snapshot
                .panes
                .iter()
                .filter(|p| is_ancestor_or_equal(&p.cwd, cwd))
                .collect();
            let deepest = ancestors
                .iter()
                .map(|p| normalize(&p.cwd).components().count())
                .max();
            if let Some(depth) = deepest {
                ancestors.retain(|p| normalize(&p.cwd).components().count() == depth);
            }
            candidates = ancestors;
        }

        if candidates.is_empty() {
            return Err(NoMatch::NoPaneForCwd(cwd.to_path_buf()));
        }
        if candidates.len() == 1 {
            return Ok(Resolution::Pane(PaneRef::from(candidates[0])));
        }

        let mut windows: Vec<String> = candidates
            .iter()
            .map(|p| p.window_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        if windows.len() > 1 {
            // Several windows have a pane here — a stray shell `cd`'d into the same
            // tree, most likely. Keep only panes that could actually be running an
            // agent and see whether that settles it.
            let narrowed: Vec<&PaneRow> = candidates
                .iter()
                .copied()
                .filter(|p| looks_like_agent(&p.current_command))
                .collect();
            let narrowed_windows: Vec<String> = narrowed
                .iter()
                .map(|p| p.window_id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            if narrowed_windows.len() == 1 {
                if narrowed.len() == 1 {
                    return Ok(Resolution::Pane(PaneRef::from(narrowed[0])));
                }
                windows = narrowed_windows;
            } else {
                return Err(NoMatch::AmbiguousWindows(windows));
            }
        }

        let window_id = windows.remove(0);
        Ok(Resolution::Window {
            window_name: snapshot.window_name(&window_id),
            window_id,
        })
    }

    /// A pane may only answer for an event whose cwd is at or below the pane's own.
    ///
    /// This is what stops a hint inherited from an unrelated project — the whole
    /// mechanism behind background jobs flashing another repo's pane — from being
    /// accepted just because the pane still exists.
    fn cwd_consistent(&self, row: &PaneRow, cwd: Option<&Path>) -> bool {
        match cwd {
            None => true,
            Some(cwd) => is_ancestor_or_equal(&row.cwd, cwd),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::proc_table::FakeProcTable;
    use crate::tmux::RecordingTmux;
    use crate::tmux::snapshot::ServerStamp;

    const STAMP: ServerStamp = ServerStamp {
        pid: 900,
        start_time: 9_000,
    };

    fn row(pane: &str, win: &str, cwd: &str, pid: u32, cmd: &str) -> PaneRow {
        PaneRow {
            pane_pid: pid,
            pane_id: pane.to_string(),
            window_id: win.to_string(),
            session_id: "$0".to_string(),
            cwd: PathBuf::from(cwd),
            current_command: cmd.to_string(),
            window_name: format!("win{}", win.trim_start_matches('@')),
        }
    }

    struct Fixture {
        tmux: RecordingTmux,
        env: FakeEnv,
        procs: FakeProcTable,
        dir: tempfile::TempDir,
    }

    fn fixture(panes: Vec<PaneRow>) -> Fixture {
        Fixture {
            tmux: RecordingTmux::new(vec![])
                .with_panes(panes)
                .with_stamp(STAMP.pid, STAMP.start_time),
            env: FakeEnv::default(),
            procs: FakeProcTable::new(&[]),
            dir: tempfile::TempDir::new().unwrap(),
        }
    }

    impl Fixture {
        fn env(mut self, pairs: &[(&str, &str)]) -> Self {
            self.env = FakeEnv::new(pairs);
            self
        }
        fn procs(mut self, pairs: &[(u32, u32)]) -> Self {
            self.procs = FakeProcTable::new(pairs);
            self
        }
        fn stale_env(mut self) -> Self {
            self.tmux = self.tmux.with_env_stale(true);
            self
        }
        fn resolve(&self, pid: u32, sid: Option<&str>, cwd: Option<&str>) -> Resolved {
            PaneResolver {
                env: &self.env,
                tmux: &self.tmux,
                procs: &self.procs,
                state_dir: self.dir.path(),
                payload_session_id: sid,
                payload_cwd: cwd,
                own_pid: pid,
            }
            .resolve()
        }
    }

    // ── Tier P ────────────────────────────────────────────────────────────────

    #[test]
    fn process_tree_resolves_at_depth_one() {
        let f = fixture(vec![row("%9", "@1", "/w", 100, "claude")]).procs(&[(200, 100), (100, 1)]);
        let r = f.resolve(200, None, Some("/w"));
        assert_eq!(r.tier, Tier::ProcessTree);
        assert_eq!(r.pane().unwrap().pane_id, "%9");
        assert_eq!(r.source, Source::Interactive);
    }

    #[test]
    fn process_tree_resolves_at_depth_three() {
        // hook -> sh -c -> claude -> pane shell
        let f = fixture(vec![row("%9", "@1", "/w", 100, "claude")]).procs(&[
            (400, 300),
            (300, 200),
            (200, 100),
            (100, 1),
        ]);
        let r = f.resolve(400, None, Some("/w"));
        assert_eq!(r.pane().unwrap().pane_id, "%9");
    }

    #[test]
    fn process_tree_window_always_comes_from_the_snapshot() {
        // Regression for the wrong-window bug: the window is never taken from a
        // hint, so it cannot disagree with the pane.
        let f = fixture(vec![row("%9", "@7", "/w", 100, "claude")])
            .env(&[("TMUX_PANE", "%9"), ("MELDR_TMUX_WINDOW_ID", "@0")])
            .procs(&[(200, 100), (100, 1)]);
        let r = f.resolve(200, None, Some("/w"));
        assert_eq!(r.pane().unwrap().window_id, "@7");
    }

    // ── Tier E ────────────────────────────────────────────────────────────────

    #[test]
    fn env_tier_used_when_process_tree_dead_ends() {
        let f = fixture(vec![row("%3", "@2", "/w", 100, "claude")])
            .env(&[("TMUX_PANE", "%3")])
            .procs(&[(900, 1)]);
        let r = f.resolve(900, None, Some("/w"));
        assert_eq!(r.tier, Tier::Env);
        assert_eq!(r.pane().unwrap().pane_id, "%3");
    }

    #[test]
    fn env_tier_rejects_a_pane_not_in_the_snapshot() {
        let f = fixture(vec![row("%3", "@2", "/w", 100, "zsh")])
            .env(&[("TMUX_PANE", "%99")])
            .procs(&[(900, 1)]);
        assert!(matches!(
            f.resolve(900, None, None).resolution,
            Resolution::None(_)
        ));
    }

    #[test]
    fn env_tier_rejects_positional_targets() {
        let f = fixture(vec![row("%3", "@2", "/w", 100, "zsh")])
            .env(&[("TMUX_PANE", "@2.0")])
            .procs(&[(900, 1)]);
        assert!(matches!(
            f.resolve(900, None, None).resolution,
            Resolution::None(_)
        ));
    }

    #[test]
    fn env_tier_rejects_a_pane_from_an_unrelated_directory() {
        // The shape of the background-job bug: the variable is real and the pane is
        // alive, but it belongs to a different project than the event.
        let f = fixture(vec![row("%2", "@0", "/other/project", 100, "claude")])
            .env(&[("TMUX_PANE", "%2")])
            .procs(&[(900, 1)]);
        let r = f.resolve(900, None, Some("/ws/meldr"));
        assert!(matches!(r.resolution, Resolution::None(_)));
    }

    #[test]
    fn env_tier_rejected_when_tmux_env_is_from_a_dead_server() {
        let f = fixture(vec![row("%3", "@2", "/w", 100, "claude")])
            .env(&[("TMUX_PANE", "%3")])
            .procs(&[(900, 1)])
            .stale_env();
        let r = f.resolve(900, None, None);
        assert!(matches!(r.resolution, Resolution::None(_)));
    }

    #[test]
    fn meldr_tmux_pane_is_never_consulted() {
        // The variable that poisoned every background job. Even set to a live pane
        // in the right directory it must not resolve anything.
        let f = fixture(vec![row("%2", "@0", "/w", 100, "claude")])
            .env(&[("MELDR_TMUX_PANE", "%2"), ("MELDR_TMUX_WINDOW_ID", "@0")])
            .procs(&[(900, 1)]);
        let r = f.resolve(900, None, None);
        assert!(
            matches!(r.resolution, Resolution::None(_)),
            "MELDR_TMUX_PANE must not be a resolution source, got {:?}",
            r.resolution
        );
    }

    // ── Tier S ────────────────────────────────────────────────────────────────

    fn write_sidecar(f: &Fixture, sid: &str, pane: &str, pid: u32, start: u64) {
        sidecar::write_pane_sidecar(
            f.dir.path(),
            sid,
            &sidecar::PaneSidecar {
                pane: pane.to_string(),
                window: "@1".to_string(),
                server_pid: pid,
                server_start: start,
                cwd: "/w".to_string(),
                ts: sidecar::now_secs(),
            },
        )
        .unwrap();
    }

    #[test]
    fn sidecar_tier_used_when_env_absent() {
        let f = fixture(vec![row("%4", "@3", "/w", 100, "claude")]).procs(&[(900, 1)]);
        write_sidecar(&f, "s1", "%4", STAMP.pid, STAMP.start_time);
        let r = f.resolve(900, Some("s1"), Some("/w"));
        assert_eq!(r.tier, Tier::Sidecar);
        assert_eq!(r.pane().unwrap().pane_id, "%4");
    }

    #[test]
    fn sidecar_from_a_previous_server_is_ignored() {
        // The 235-stale-files case: pane ids restart per server, so an old id can
        // name a live pane that has nothing to do with this session.
        let f = fixture(vec![row("%4", "@3", "/w", 100, "claude")]).procs(&[(900, 1)]);
        write_sidecar(&f, "s1", "%4", 111, 222);
        let r = f.resolve(900, Some("s1"), None);
        assert!(matches!(r.resolution, Resolution::None(_)));
    }

    #[test]
    fn sidecar_naming_a_dead_pane_is_ignored() {
        let f = fixture(vec![row("%4", "@3", "/w", 100, "claude")]).procs(&[(900, 1)]);
        write_sidecar(&f, "s1", "%77", STAMP.pid, STAMP.start_time);
        assert!(matches!(
            f.resolve(900, Some("s1"), None).resolution,
            Resolution::None(_)
        ));
    }

    #[test]
    fn legacy_parent_pane_files_are_not_read() {
        let f = fixture(vec![row("%4", "@3", "/w", 100, "claude")]).procs(&[(900, 1)]);
        std::fs::write(f.dir.path().join("s1.parent_pane"), "%4").unwrap();
        assert!(matches!(
            f.resolve(900, Some("s1"), None).resolution,
            Resolution::None(_)
        ));
    }

    // ── Tier C ────────────────────────────────────────────────────────────────

    #[test]
    fn cwd_tier_resolves_a_single_matching_pane() {
        let f = fixture(vec![
            row("%1", "@0", "/other", 10, "zsh"),
            row("%2", "@1", "/ws/meldr", 20, "claude"),
        ])
        .procs(&[(900, 1)]);
        let r = f.resolve(900, None, Some("/ws/meldr"));
        assert_eq!(r.tier, Tier::Cwd);
        assert_eq!(r.pane().unwrap().pane_id, "%2");
        assert_eq!(
            r.source,
            Source::Background,
            "a cwd-only match never proved which process this was"
        );
    }

    #[test]
    fn cwd_tier_yields_the_window_when_several_panes_share_the_directory() {
        // The real 9-pane layout: three agent panes and terminals all in one
        // worktree. Picking one would be a coin flip, so name the window instead.
        let f = fixture(vec![
            row("%5", "@1", "/ws/meldr", 10, "zsh"),
            row("%8", "@1", "/ws/meldr", 20, "claude"),
            row("%9", "@1", "/ws/meldr", 30, "claude"),
        ])
        .procs(&[(900, 1)]);
        let r = f.resolve(900, None, Some("/ws/meldr"));
        assert_eq!(
            r.resolution,
            Resolution::Window {
                window_id: "@1".to_string(),
                window_name: "win1".to_string()
            }
        );
        assert_eq!(r.source, Source::Background);
    }

    #[test]
    fn cwd_tier_refuses_when_the_directory_spans_windows() {
        let f = fixture(vec![
            row("%1", "@0", "/ws/meldr", 10, "zsh"),
            row("%2", "@1", "/ws/meldr", 20, "zsh"),
        ])
        .procs(&[(900, 1)]);
        match f.resolve(900, None, Some("/ws/meldr")).resolution {
            Resolution::None(NoMatch::AmbiguousWindows(w)) => assert_eq!(w, vec!["@0", "@1"]),
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn cwd_tier_narrows_across_windows_by_command() {
        // A stray shell parked in the same tree must not create ambiguity when only
        // one window actually has an agent running there.
        let f = fixture(vec![
            row("%1", "@0", "/ws/meldr", 10, "zsh"),
            row("%2", "@1", "/ws/meldr", 20, "claude"),
        ])
        .procs(&[(900, 1)]);
        let r = f.resolve(900, None, Some("/ws/meldr"));
        assert_eq!(r.pane().unwrap().pane_id, "%2");
    }

    #[test]
    fn cwd_tier_falls_back_to_the_deepest_ancestor() {
        let f = fixture(vec![
            row("%1", "@0", "/ws", 10, "claude"),
            row("%2", "@1", "/ws/meldr", 20, "claude"),
        ])
        .procs(&[(900, 1)]);
        let r = f.resolve(900, None, Some("/ws/meldr/src/deep"));
        assert_eq!(r.pane().unwrap().pane_id, "%2");
    }

    #[test]
    fn cwd_tier_does_not_match_a_sibling_directory() {
        // Regression for ~/fmcevoy vs ~/fmcevoy_tools.
        let f = fixture(vec![row("%1", "@0", "/home/u/fmcevoy", 10, "claude")]).procs(&[(900, 1)]);
        match f
            .resolve(900, None, Some("/home/u/fmcevoy_tools/p"))
            .resolution
        {
            Resolution::None(NoMatch::NoPaneForCwd(p)) => {
                assert_eq!(p, PathBuf::from("/home/u/fmcevoy_tools/p"))
            }
            other => panic!("sibling prefix must not match, got {other:?}"),
        }
    }

    // ── ordering / source ─────────────────────────────────────────────────────

    #[test]
    fn child_session_prefers_cwd_over_the_process_tree() {
        // A background job's ancestry leads to whichever pane started the daemon —
        // here a different project. The directory is the trustworthy signal.
        let f = fixture(vec![
            row("%2", "@0", "/other/project", 100, "claude"),
            row("%8", "@1", "/ws/meldr", 200, "claude"),
        ])
        .env(&[("CLAUDE_CODE_CHILD_SESSION", "1")])
        .procs(&[(500, 100), (100, 1)]);

        let r = f.resolve(500, None, Some("/ws/meldr"));
        assert_eq!(r.tier, Tier::Cwd);
        assert_eq!(r.pane().unwrap().pane_id, "%8");
        assert_eq!(r.source, Source::Background);
    }

    #[test]
    fn child_session_still_resolves_via_process_tree_when_cwd_says_nothing() {
        let f = fixture(vec![row("%2", "@0", "/other", 100, "claude")])
            .env(&[("CLAUDE_CODE_CHILD_SESSION", "1")])
            .procs(&[(500, 100), (100, 1)]);
        let r = f.resolve(500, None, Some("/unmatched"));
        assert_eq!(r.tier, Tier::ProcessTree);
        assert_eq!(r.source, Source::Background, "still a child session");
    }

    #[test]
    fn interactive_session_prefers_process_tree_over_cwd() {
        let f = fixture(vec![
            row("%8", "@1", "/ws/meldr", 100, "claude"),
            row("%9", "@1", "/ws/meldr", 200, "claude"),
        ])
        .procs(&[(500, 200), (200, 1)]);
        let r = f.resolve(500, None, Some("/ws/meldr"));
        assert_eq!(r.tier, Tier::ProcessTree);
        assert_eq!(
            r.pane().unwrap().pane_id,
            "%9",
            "must pick the pane it actually runs in, not a sibling"
        );
    }

    // ── failure reporting ─────────────────────────────────────────────────────

    #[test]
    fn no_hints_at_all() {
        let f = fixture(vec![row("%1", "@0", "/w", 10, "zsh")]).procs(&[(900, 1)]);
        assert_eq!(
            f.resolve(900, None, None).resolution,
            Resolution::None(NoMatch::NoHints)
        );
    }

    #[test]
    fn tmux_unavailable_is_reported_not_swallowed() {
        let tmux = RecordingTmux::new(vec![]).failing("list-panes");
        let dir = tempfile::TempDir::new().unwrap();
        let env = FakeEnv::default();
        let procs = FakeProcTable::new(&[]);
        let r = PaneResolver {
            env: &env,
            tmux: &tmux,
            procs: &procs,
            state_dir: dir.path(),
            payload_session_id: None,
            payload_cwd: Some("/w"),
            own_pid: 1,
        }
        .resolve();
        assert!(matches!(r.resolution, Resolution::None(NoMatch::NoTmux(_))));
    }

    #[test]
    fn window_accessors() {
        let f = fixture(vec![row("%9", "@4", "/w", 100, "claude")]).procs(&[(200, 100), (100, 1)]);
        let r = f.resolve(200, None, Some("/w"));
        assert_eq!(r.window_id(), Some("@4"));
        assert_eq!(r.window_name(), "win4");
    }
}
