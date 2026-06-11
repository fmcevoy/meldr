/// Claude Code hook event handlers.
///
/// Dispatches the four events wired by `meldr install-hooks` into
/// `~/.claude/settings.json`:
///
/// - `session-start` → called on every new Claude session; resolves the
///   originating tmux pane via the launcher registry and writes a sidecar so
///   subsequent Stop / Notification events can locate the pane quickly.
/// - `stop`          → classifies the stop status (done / waiting), plays a
///   sound, and flashes the tmux tab.
/// - `notify`        → always flashes with "waiting" status (mid-session notification).
/// - `register-launcher` → called from the `claude()` shell wrapper just before
///   `claude agents` is exec'd; writes a launcher-registry entry for the current pane.
pub mod classify;
pub mod registry;
pub mod resolver;
pub mod sidecar;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::error::Result;
use crate::tmux::{OptionScope, TmuxOps};

use classify::{StopStatus, classify_stop};
use resolver::{Env, PaneRef, PaneResolver, RealEnv};
use sidecar::{SessionState, expand_tilde, write_parent_pane, write_session_state};

/// Parsed Claude Code hook JSON payload (sent on stdin).
#[derive(Debug, Default, Deserialize)]
pub struct HookPayload {
    pub hook_event_name: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
}

impl HookPayload {
    /// Parse from a JSON string. Returns a default payload on parse failure.
    pub fn from_json(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }

    /// Returns true when this is a sub-agent event that should be suppressed.
    /// We skip only when `agent_id` is non-empty AND the event name contains
    /// "Subagent" — main-agent Stop events still flash even if agent_id is set.
    pub fn is_subagent_event(&self) -> bool {
        let has_agent_id = self
            .agent_id
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        let is_subagent_event = self
            .hook_event_name
            .as_deref()
            .map(|n| n.contains("Subagent"))
            .unwrap_or(false);
        has_agent_id && is_subagent_event
    }
}

/// Handle the `SessionStart` hook.
///
/// Reads the hook payload from `payload`, resolves the originating tmux pane via
/// the launcher registry (cwd-based matching), writes a sidecar so the Stop /
/// Notification handler can locate the pane quickly. Also GC-s stale launcher
/// entries.
pub fn dispatch_session_start(
    payload: &HookPayload,
    state_dir: &Path,
    tmux: &dyn TmuxOps,
) -> Result<()> {
    let launcher_dir = state_dir.join("launchers");

    // GC stale registry entries before doing anything else.
    registry::gc(&launcher_dir, 7 * 86400, tmux);

    let Some(sid) = payload.session_id.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(());
    };

    let env = RealEnv;
    let resolver = PaneResolver {
        env: &env,
        tmux,
        state_dir,
        payload_session_id: Some(sid),
        payload_cwd: payload.cwd.as_deref(),
    };

    if let Some(pr) = resolver.resolve() {
        let _ = write_parent_pane(state_dir, sid, &pr.pane_id);
    }

    Ok(())
}

/// Handle the `Stop` hook.
///
/// Classifies the stop status, plays a sound, writes session state, and flashes
/// the tmux tab and pane border.
pub fn dispatch_stop(payload: &HookPayload, state_dir: &Path, tmux: &dyn TmuxOps) -> Result<()> {
    if payload.is_subagent_event() {
        return Ok(());
    }

    let pane_ref = resolve(payload, state_dir, tmux);

    let transcript = payload
        .transcript_path
        .as_deref()
        .map(expand_tilde)
        .filter(|p| p.exists());

    let status = transcript
        .as_deref()
        .map(classify_stop)
        .unwrap_or(StopStatus::Done);

    play_sound(status);
    write_state(payload, state_dir, &pane_ref, status.as_str());
    if let Some(pr) = &pane_ref {
        flash(tmux, pr, status.as_str());
    }

    Ok(())
}

/// Handle the `Notification` hook.
///
/// Always flashes with "waiting" status — the agent is asking for input.
pub fn dispatch_notify(payload: &HookPayload, state_dir: &Path, tmux: &dyn TmuxOps) -> Result<()> {
    if payload.is_subagent_event() {
        return Ok(());
    }

    let pane_ref = resolve(payload, state_dir, tmux);

    play_sound(StopStatus::Waiting);
    write_state(payload, state_dir, &pane_ref, "waiting");
    if let Some(pr) = &pane_ref {
        flash(tmux, pr, "waiting");
    }

    Ok(())
}

/// Handle the `register-launcher` pseudo-event.
///
/// Called from the `claude()` shell wrapper immediately before `claude agents` is
/// exec'd. Records the current pane, window, and cwd so the SessionStart handler
/// can map new sessions back to this pane.
pub fn dispatch_register_launcher(state_dir: &Path, tmux: &dyn TmuxOps) -> Result<()> {
    let launcher_dir = state_dir.join("launchers");
    let env = RealEnv;

    // Prefer MELDR_TMUX_PANE (set by the wrapper), fall back to TMUX_PANE.
    let pane_id = env
        .var("MELDR_TMUX_PANE")
        .filter(|s| !s.is_empty())
        .or_else(|| {
            env.var("TMUX_PANE")
                .filter(|s| !s.is_empty())
                .and_then(|tp| tmux.display_message(&tp, "#{pane_id}").ok())
        });

    let Some(pane_id) = pane_id else {
        // Not in tmux — nothing to register.
        return Ok(());
    };

    let window_id = env
        .var("MELDR_TMUX_WINDOW_ID")
        .filter(|s| !s.is_empty())
        .or_else(|| tmux.display_message(&pane_id, "#{window_id}").ok())
        .unwrap_or_default();

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    registry::write_entry(&launcher_dir, &pane_id, &window_id, &cwd)?;

    // GC stale entries while we're here.
    registry::gc(&launcher_dir, 7 * 86400, tmux);

    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn resolve(payload: &HookPayload, state_dir: &Path, tmux: &dyn TmuxOps) -> Option<PaneRef> {
    let env = RealEnv;
    PaneResolver {
        env: &env,
        tmux,
        state_dir,
        payload_session_id: payload.session_id.as_deref(),
        payload_cwd: payload.cwd.as_deref(),
    }
    .resolve()
}

fn write_state(payload: &HookPayload, state_dir: &Path, pane_ref: &Option<PaneRef>, status: &str) {
    let Some(sid) = payload.session_id.as_deref().filter(|s| !s.is_empty()) else {
        return;
    };
    let pr = pane_ref.as_ref();
    let state = SessionState {
        status: status.to_string(),
        ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        cwd: payload.cwd.clone().unwrap_or_default(),
        pane: pr.map(|p| p.pane_id.clone()).unwrap_or_default(),
        window: pr.map(|p| p.window_id.clone()).unwrap_or_default(),
        window_name: pr.map(|p| p.window_name.clone()).unwrap_or_default(),
    };
    let _ = write_session_state(state_dir, sid, &state);
}

/// Set `@cc_status`, `@cc_pane_status`, and `@cc_status_gen` on the window/pane,
/// then schedule an async clear after `MELDR_CC_TIMEOUT` seconds (default 5).
fn flash(tmux: &dyn TmuxOps, pr: &PaneRef, status: &str) {
    let timeout: u64 = std::env::var("MELDR_CC_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    // Generation token: nano-ts + pid ensures uniqueness across concurrent flashes.
    let flash_token = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );

    let _ = tmux.set_user_option(OptionScope::Window, &pr.window_id, "@cc_status", status);
    let _ = tmux.set_user_option(
        OptionScope::Window,
        &pr.window_id,
        "@cc_status_gen",
        &flash_token,
    );
    let _ = tmux.set_user_option(OptionScope::Pane, &pr.pane_id, "@cc_pane_status", status);

    // Schedule an async clear: only clears if the generation token still matches,
    // so a later flash from a sibling pane is not wiped.
    let wid = &pr.window_id;
    let pane = &pr.pane_id;
    let clear_cmd = format!(
        "sleep {timeout}; \
         CUR=$(tmux show-options -wqv -t '{wid}' @cc_status_gen 2>/dev/null); \
         [ \"$CUR\" = '{flash_token}' ] && tmux set-option -wu -t '{wid}' @cc_status 2>/dev/null; \
         tmux set-option -wu -t '{wid}' @cc_status_gen 2>/dev/null; \
         tmux set-option -pu -t '{pane}' @cc_pane_status 2>/dev/null"
    );
    let _ = tmux.run_shell_bg(&clear_cmd);
}

/// Play a notification sound if `afplay` is available (macOS only).
/// Fire-and-forget: the process is spawned in the background and not waited on.
fn play_sound(status: StopStatus) {
    #[cfg(target_os = "macos")]
    {
        let sound = match status {
            StopStatus::Waiting => "/System/Library/Sounds/Funk.aiff",
            StopStatus::Done => "/System/Library/Sounds/Glass.aiff",
        };
        let _ = std::process::Command::new("afplay").arg(sound).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = status; // suppress unused warning
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::RecordingTmux;

    fn make_state_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    fn recording_tmux_with_pane(pane: &str) -> RecordingTmux {
        let mut t = RecordingTmux::new(vec![pane.to_string()]);
        t = t
            .with_display(pane, "#{window_id}", "@1")
            .with_display(pane, "#{window_name}", "ws/feat")
            .with_display(pane, "#{pane_id}", pane);
        t
    }

    // ── HookPayload ───────────────────────────────────────────────────────────

    #[test]
    fn is_subagent_event_true_when_both_set() {
        let p = HookPayload {
            hook_event_name: Some("SubagentStop".to_string()),
            agent_id: Some("agent-123".to_string()),
            ..Default::default()
        };
        assert!(p.is_subagent_event());
    }

    #[test]
    fn is_subagent_event_false_for_main_stop() {
        let p = HookPayload {
            hook_event_name: Some("Stop".to_string()),
            agent_id: Some("agent-123".to_string()),
            ..Default::default()
        };
        assert!(!p.is_subagent_event());
    }

    #[test]
    fn from_json_parses_minimal() {
        let p = HookPayload::from_json(r#"{"session_id":"s1","cwd":"/tmp"}"#);
        assert_eq!(p.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn from_json_defaults_on_invalid() {
        let p = HookPayload::from_json("not json {{{{");
        assert!(p.session_id.is_none());
    }

    // ── dispatch_stop sets @cc_status ─────────────────────────────────────────

    #[test]
    fn dispatch_stop_sets_cc_status_done() {
        let tmp = make_state_dir();
        let tmux = recording_tmux_with_pane("%1");

        // Write a sidecar so tier 4 resolves.
        sidecar::write_parent_pane(tmp.path(), "sess1", "%1").unwrap();

        let payload = HookPayload {
            hook_event_name: Some("Stop".to_string()),
            session_id: Some("sess1".to_string()),
            cwd: Some("/tmp".to_string()),
            ..Default::default()
        };

        dispatch_stop(&payload, tmp.path(), &tmux).unwrap();

        let calls = tmux.set_calls.lock().unwrap();
        let status_call = calls
            .iter()
            .find(|(scope, _tgt, key, _val)| *scope == OptionScope::Window && key == "@cc_status");
        assert!(status_call.is_some(), "should set @cc_status");
        assert_eq!(status_call.unwrap().3, "done");
    }

    #[test]
    fn dispatch_notify_sets_waiting() {
        let tmp = make_state_dir();
        let tmux = recording_tmux_with_pane("%1");

        sidecar::write_parent_pane(tmp.path(), "sess2", "%1").unwrap();

        let payload = HookPayload {
            hook_event_name: Some("Notification".to_string()),
            session_id: Some("sess2".to_string()),
            cwd: Some("/tmp".to_string()),
            ..Default::default()
        };

        dispatch_notify(&payload, tmp.path(), &tmux).unwrap();

        let calls = tmux.set_calls.lock().unwrap();
        let status_call = calls
            .iter()
            .find(|(scope, _tgt, key, _val)| *scope == OptionScope::Window && key == "@cc_status");
        assert_eq!(status_call.unwrap().3, "waiting");
    }

    #[test]
    fn dispatch_stop_subagent_skipped() {
        let tmp = make_state_dir();
        let tmux = recording_tmux_with_pane("%1");

        let payload = HookPayload {
            hook_event_name: Some("SubagentStop".to_string()),
            agent_id: Some("agent-x".to_string()),
            session_id: Some("sess3".to_string()),
            ..Default::default()
        };

        dispatch_stop(&payload, tmp.path(), &tmux).unwrap();

        assert!(
            tmux.set_calls.lock().unwrap().is_empty(),
            "subagent stop must not flash"
        );
    }

    #[test]
    fn dispatch_stop_no_pane_resolved_no_flash() {
        let tmp = make_state_dir();
        let tmux = RecordingTmux::new(vec![]);

        let payload = HookPayload {
            hook_event_name: Some("Stop".to_string()),
            session_id: Some("unknown".to_string()),
            ..Default::default()
        };

        dispatch_stop(&payload, tmp.path(), &tmux).unwrap();

        assert!(tmux.set_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn dispatch_session_start_writes_sidecar() {
        let tmp = make_state_dir();
        // Write a launcher entry so tier 5 resolves.
        let launcher_dir = tmp.path().join("launchers");
        registry::write_entry(
            &launcher_dir,
            "%1",
            "@1",
            std::path::Path::new("/home/user/proj"),
        )
        .unwrap();

        let tmux = recording_tmux_with_pane("%1");

        let payload = HookPayload {
            hook_event_name: Some("SessionStart".to_string()),
            session_id: Some("new-session".to_string()),
            cwd: Some("/home/user/proj/sub".to_string()),
            ..Default::default()
        };

        dispatch_session_start(&payload, tmp.path(), &tmux).unwrap();

        let written = sidecar::read_parent_pane(tmp.path(), "new-session");
        assert_eq!(written.as_deref(), Some("%1"));
    }
}
