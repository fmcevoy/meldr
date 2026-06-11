/// Multi-tier pane resolver.
///
/// Resolves which tmux pane a Claude hook event should flash. Tiers are tried in
/// order; the first that returns a live pane wins. Env access and tmux queries go
/// through traits so unit tests can drive the resolver without a real tmux server.
///
/// Tier order:
/// 1. `MELDR_TMUX_PANE` env var  (injected by meldr at worktree-spawn time)
/// 2. `TMUX_PANE` env var        (inherited when claude runs directly in a pane)
/// 3. `MELDR_AGENT_SESSION` env → sidecar file `<state_dir>/<sid>.parent_pane`
/// 4. Hook payload `session_id`  → sidecar file `<state_dir>/<sid>.parent_pane`
/// 5. Launcher registry match by cwd (component-aware prefix, longest+newest first)
use std::path::Path;

use crate::tmux::TmuxOps;

use super::registry;
use super::sidecar;

/// A resolved tmux pane location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRef {
    pub pane_id: String,
    pub window_id: String,
    pub window_name: String,
}

/// Trait for reading environment variables, injectable for testing.
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

/// `Env` backed by a static `HashMap` for tests.
#[cfg(test)]
pub struct FakeEnv(pub std::collections::HashMap<&'static str, &'static str>);

#[cfg(test)]
impl Env for FakeEnv {
    fn var(&self, key: &str) -> Option<String> {
        self.0.get(key).map(|s| s.to_string())
    }
}

/// Resolves which pane a Claude hook event should target.
pub struct PaneResolver<'a> {
    pub env: &'a dyn Env,
    pub tmux: &'a dyn TmuxOps,
    /// Directory containing sidecar files (`~/.cache/claude-agents`).
    pub state_dir: &'a Path,
    /// `session_id` from the hook JSON payload, used for Tier 4.
    pub payload_session_id: Option<&'a str>,
    /// `cwd` from the hook JSON payload, used for Tier 5 registry matching.
    pub payload_cwd: Option<&'a str>,
}

impl<'a> PaneResolver<'a> {
    /// Attempt each tier in order and return the first live `PaneRef`.
    pub fn resolve(&self) -> Option<PaneRef> {
        // Tier 1 — MELDR_TMUX_PANE (injected at spawn by meldr worktree creation)
        if let Some(pane_id) = self.env.var("MELDR_TMUX_PANE").filter(|s| !s.is_empty())
            && self.tmux.pane_exists(&pane_id)
        {
            let window_id = self.env.var("MELDR_TMUX_WINDOW_ID").unwrap_or_default();
            let window_name = self
                .tmux
                .display_message(&pane_id, "#{window_name}")
                .unwrap_or_default();
            return Some(PaneRef {
                pane_id,
                window_id,
                window_name,
            });
        }

        // Tier 2 — TMUX_PANE (inherited when claude runs as a direct child of the pane)
        if let Some(tmux_pane) = self.env.var("TMUX_PANE").filter(|s| !s.is_empty()) {
            // TMUX_PANE may be `%N` already or a relative index; resolve via display-message.
            if let Ok(pane_id) = self.tmux.display_message(&tmux_pane, "#{pane_id}")
                && self.tmux.pane_exists(&pane_id)
            {
                let window_id = self
                    .tmux
                    .display_message(&tmux_pane, "#{window_id}")
                    .unwrap_or_default();
                let window_name = self
                    .tmux
                    .display_message(&tmux_pane, "#{window_name}")
                    .unwrap_or_default();
                return Some(PaneRef {
                    pane_id,
                    window_id,
                    window_name,
                });
            }
        }

        // Tier 3 — MELDR_AGENT_SESSION → sidecar
        if let Some(sess) = self
            .env
            .var("MELDR_AGENT_SESSION")
            .filter(|s| !s.is_empty())
            && let Some(pr) = self.resolve_from_sidecar(&sess)
        {
            return Some(pr);
        }

        // Tier 4 — hook payload session_id → sidecar
        if let Some(sid) = self.payload_session_id.filter(|s| !s.is_empty())
            && let Some(pr) = self.resolve_from_sidecar(sid)
        {
            return Some(pr);
        }

        // Tier 5 — launcher registry match by cwd
        if let Some(cwd_str) = self.payload_cwd.filter(|s| !s.is_empty()) {
            let cwd = std::path::PathBuf::from(cwd_str);
            let launcher_dir = self.state_dir.join("launchers");
            if let Some(entry) = registry::find_best_match(&launcher_dir, &cwd, self.tmux) {
                let window_name = self
                    .tmux
                    .display_message(&entry.pane, "#{window_name}")
                    .unwrap_or_default();
                return Some(PaneRef {
                    pane_id: entry.pane,
                    window_id: entry.window,
                    window_name,
                });
            }
        }

        None
    }

    fn resolve_from_sidecar(&self, session_id: &str) -> Option<PaneRef> {
        let pane_id = sidecar::read_parent_pane(self.state_dir, session_id)?;
        if !self.tmux.pane_exists(&pane_id) {
            return None;
        }
        let window_id = self
            .tmux
            .display_message(&pane_id, "#{window_id}")
            .unwrap_or_default();
        let window_name = self
            .tmux
            .display_message(&pane_id, "#{window_name}")
            .unwrap_or_default();
        Some(PaneRef {
            pane_id,
            window_id,
            window_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::RecordingTmux;

    fn fake_env(pairs: &[(&'static str, &'static str)]) -> FakeEnv {
        FakeEnv(pairs.iter().cloned().collect())
    }

    fn recording_tmux(live: Vec<&str>) -> RecordingTmux {
        let live: Vec<String> = live.into_iter().map(str::to_owned).collect();
        let mut tmux = RecordingTmux::new(live);
        // Stub a couple of common display_message calls.
        tmux = tmux
            .with_display("%1", "#{window_id}", "@1")
            .with_display("%1", "#{window_name}", "ws/feat")
            .with_display("%1", "#{pane_id}", "%1")
            .with_display("%2", "#{window_id}", "@2")
            .with_display("%2", "#{window_name}", "ws/other")
            .with_display("%2", "#{pane_id}", "%2");
        tmux
    }

    #[test]
    fn tier1_meldr_tmux_pane_wins() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tmux = recording_tmux(vec!["%1"]);
        let env = fake_env(&[("MELDR_TMUX_PANE", "%1"), ("MELDR_TMUX_WINDOW_ID", "@1")]);
        let resolver = PaneResolver {
            env: &env,
            tmux: &tmux,
            state_dir: tmp.path(),
            payload_session_id: None,
            payload_cwd: None,
        };
        let pr = resolver.resolve().unwrap();
        assert_eq!(pr.pane_id, "%1");
        assert_eq!(pr.window_id, "@1");
    }

    #[test]
    fn tier1_dead_pane_falls_through_to_tier2() {
        let tmp = tempfile::TempDir::new().unwrap();
        // %1 is dead, %2 is live
        let tmux = recording_tmux(vec!["%2"]);
        let env = fake_env(&[("MELDR_TMUX_PANE", "%1"), ("TMUX_PANE", "%2")]);
        let resolver = PaneResolver {
            env: &env,
            tmux: &tmux,
            state_dir: tmp.path(),
            payload_session_id: None,
            payload_cwd: None,
        };
        let pr = resolver.resolve().unwrap();
        assert_eq!(pr.pane_id, "%2", "should fall through to tier 2");
    }

    #[test]
    fn tier3_meldr_agent_session_sidecar() {
        let tmp = tempfile::TempDir::new().unwrap();
        sidecar::write_parent_pane(tmp.path(), "sess123", "%1").unwrap();
        let tmux = recording_tmux(vec!["%1"]);
        let env = fake_env(&[("MELDR_AGENT_SESSION", "sess123")]);
        let resolver = PaneResolver {
            env: &env,
            tmux: &tmux,
            state_dir: tmp.path(),
            payload_session_id: None,
            payload_cwd: None,
        };
        let pr = resolver.resolve().unwrap();
        assert_eq!(pr.pane_id, "%1");
    }

    #[test]
    fn tier4_payload_session_id_sidecar() {
        let tmp = tempfile::TempDir::new().unwrap();
        sidecar::write_parent_pane(tmp.path(), "sess456", "%1").unwrap();
        let tmux = recording_tmux(vec!["%1"]);
        let env = fake_env(&[]);
        let resolver = PaneResolver {
            env: &env,
            tmux: &tmux,
            state_dir: tmp.path(),
            payload_session_id: Some("sess456"),
            payload_cwd: None,
        };
        let pr = resolver.resolve().unwrap();
        assert_eq!(pr.pane_id, "%1");
    }

    #[test]
    fn tier5_registry_match_by_cwd() {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        registry::write_entry(
            &launcher_dir,
            "%1",
            "@1",
            std::path::Path::new("/home/user/project"),
        )
        .unwrap();

        let mut tmux = recording_tmux(vec!["%1"]);
        tmux = tmux.with_display("%1", "#{window_name}", "ws/feat");

        let env = fake_env(&[]);
        let resolver = PaneResolver {
            env: &env,
            tmux: &tmux,
            state_dir: tmp.path(),
            payload_session_id: None,
            payload_cwd: Some("/home/user/project/subdir"),
        };
        let pr = resolver.resolve().unwrap();
        assert_eq!(pr.pane_id, "%1");
    }

    #[test]
    fn tier5_sibling_prefix_not_matched() {
        // Regression: ~/fmcevoy launcher must NOT match ~/fmcevoy_tools session.
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher_dir = tmp.path().join("launchers");
        // Only register ~/fmcevoy, NOT ~/fmcevoy_tools
        registry::write_entry(
            &launcher_dir,
            "%1",
            "@1",
            std::path::Path::new("/home/user/fmcevoy"),
        )
        .unwrap();

        let tmux = recording_tmux(vec!["%1"]);
        let env = fake_env(&[]);
        let resolver = PaneResolver {
            env: &env,
            tmux: &tmux,
            state_dir: tmp.path(),
            payload_session_id: None,
            payload_cwd: Some("/home/user/fmcevoy_tools/project"),
        };
        assert!(
            resolver.resolve().is_none(),
            "sibling prefix must not match"
        );
    }

    #[test]
    fn no_tier_resolves_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tmux = recording_tmux(vec![]);
        let env = fake_env(&[]);
        let resolver = PaneResolver {
            env: &env,
            tmux: &tmux,
            state_dir: tmp.path(),
            payload_session_id: None,
            payload_cwd: None,
        };
        assert!(resolver.resolve().is_none());
    }
}
