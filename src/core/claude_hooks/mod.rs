//! Claude Code hook event handlers.
//!
//! Wired into `~/.claude/settings.json` by `meldr install-hooks`:
//!
//! - `session-start` → records where the session's pane is (and collects sidecars
//!   that can no longer name anything).
//! - `stop`          → classifies the stop, plays a sound, flashes the pane/tab.
//! - `notify`        → flashes "waiting" for the notifications that mean the agent
//!   is actually blocked on you.
//! - `clear`         → expires a flash. Scheduled by `flash` itself, and safe to
//!   call at any time from a tmux hook.
//!
//! # How a flash is represented
//!
//! Truth lives on the **pane**: `@cc_pane_status` says what that agent is doing and
//! `@cc_pane_until` says when the claim expires. The window's `@cc_status` — what
//! the tab renders — is *derived* from its panes, recomputed after every change.
//!
//! That inversion is what makes overlapping events safe. The previous design wrote
//! the window directly and guarded the clear with a generation token, but the
//! token was cleared unconditionally, so a second flash arriving mid-timer erased
//! the token the first timer was about to check and the tab stayed lit forever.
//! Here a later flash simply pushes `@cc_pane_until` further out; every clear is
//! idempotent and only acts once its own deadline has passed.
//!
//! Background jobs get their own status values (`bg-done` / `bg-waiting`), because
//! Claude hosts them in a detached daemon where the exact pane is often unknowable
//! — the honest signal is "something finished in this worktree", rendered
//! differently from the agent sitting in front of you.

pub mod classify;
pub mod resolver;
pub mod sidecar;

use std::path::Path;

use serde::Deserialize;

use crate::error::Result;
use crate::tmux::{OptionScope, TmuxOps};

use classify::{StopStatus, classify_stop};
use resolver::{PaneResolver, RealEnv, Resolution, Resolved, Source};
use sidecar::{SessionState, expand_tilde, now_secs, write_session_state};

/// Window option the tab format reads. Published contract — users' `tmux.conf`
/// switches on these values.
pub const WIN_STATUS: &str = "@cc_status";
/// Window-level expiry, used only for window-scoped (background) flashes.
pub const WIN_UNTIL: &str = "@cc_until";
/// Pane option the pane-border format reads.
pub const PANE_STATUS: &str = "@cc_pane_status";
/// Pane-level expiry.
pub const PANE_UNTIL: &str = "@cc_pane_until";

/// How long a flash stays lit, in seconds, unless `MELDR_CC_TIMEOUT` says otherwise.
pub const DEFAULT_TIMEOUT_SECS: u64 = 5;

pub fn flash_timeout() -> u64 {
    std::env::var("MELDR_CC_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// What a tab or pane border is currently saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashStatus {
    Done,
    Waiting,
    BgDone,
    BgWaiting,
}

impl FlashStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FlashStatus::Done => "done",
            FlashStatus::Waiting => "waiting",
            FlashStatus::BgDone => "bg-done",
            FlashStatus::BgWaiting => "bg-waiting",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "done" => Some(FlashStatus::Done),
            "waiting" => Some(FlashStatus::Waiting),
            "bg-done" => Some(FlashStatus::BgDone),
            "bg-waiting" => Some(FlashStatus::BgWaiting),
            _ => None,
        }
    }

    /// Precedence when several panes in a window are lit at once. Anything
    /// *waiting* on you outranks anything merely finished, and a session you are
    /// sitting in outranks a background job.
    pub fn rank(self) -> u8 {
        match self {
            FlashStatus::Waiting => 4,
            FlashStatus::BgWaiting => 3,
            FlashStatus::Done => 2,
            FlashStatus::BgDone => 1,
        }
    }

    pub fn of(stop: StopStatus, source: Source) -> Self {
        match (stop, source) {
            (StopStatus::Waiting, Source::Interactive) => FlashStatus::Waiting,
            (StopStatus::Done, Source::Interactive) => FlashStatus::Done,
            (StopStatus::Waiting, Source::Background) => FlashStatus::BgWaiting,
            (StopStatus::Done, Source::Background) => FlashStatus::BgDone,
        }
    }

    /// macOS system sound, chosen so background jobs are audibly distinct.
    #[cfg(target_os = "macos")]
    fn sound(self) -> &'static str {
        match self {
            FlashStatus::Waiting => "/System/Library/Sounds/Funk.aiff",
            FlashStatus::Done => "/System/Library/Sounds/Glass.aiff",
            FlashStatus::BgWaiting => "/System/Library/Sounds/Submarine.aiff",
            FlashStatus::BgDone => "/System/Library/Sounds/Pop.aiff",
        }
    }
}

/// Parsed Claude Code hook JSON payload (sent on stdin).
#[derive(Debug, Default, Deserialize)]
pub struct HookPayload {
    pub hook_event_name: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    /// Present on `Stop`: the final assistant text of the turn. Preferred over
    /// re-reading the transcript, which may not be flushed yet.
    pub last_assistant_message: Option<String>,
    /// Present on `Notification`: which notification this is. Most of them are not
    /// the agent asking for anything.
    pub notification_type: Option<String>,
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

/// Notification types that mean the agent is actually blocked on the user.
///
/// The hook was previously registered with matcher `*`, so routine events —
/// `auth_success`, `agent_completed`, the `quota_auto_resume_*` family — lit the
/// tab as though input were needed.
pub const BLOCKING_NOTIFICATIONS: &[&str] = &[
    "permission_prompt",
    "idle_prompt",
    "elicitation_dialog",
    "elicitation_url_dialog",
    "agent_needs_input",
];

pub fn notification_is_blocking(kind: Option<&str>) -> bool {
    match kind {
        // Older Claude builds send no type; keep flashing rather than going silent.
        None => true,
        Some(k) => BLOCKING_NOTIFICATIONS.contains(&k),
    }
}

// ── dispatch ──────────────────────────────────────────────────────────────────

/// Handle the `SessionStart` hook: record the pane and sweep dead sidecars.
pub fn dispatch_session_start(
    payload: &HookPayload,
    state_dir: &Path,
    tmux: &dyn TmuxOps,
) -> Result<()> {
    let stamp = tmux.snapshot().ok().map(|s| s.stamp);
    let swept = sidecar::gc_sidecars(state_dir, stamp.as_ref(), now_secs());
    if swept.total() > 0 {
        eprintln!(
            "meldr claude-hook: swept {} unusable sidecar(s) ({} legacy, {} from an older tmux server)",
            swept.total(),
            swept.legacy,
            swept.stale_server
        );
    }

    let Some(sid) = payload.session_id.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let Some(stamp) = stamp else {
        return Ok(());
    };

    // Only an exact pane is worth recording; a window is not a routing hint.
    if let Some(pane) = resolve(payload, state_dir, tmux).pane() {
        let _ = sidecar::write_pane_sidecar(
            state_dir,
            sid,
            &sidecar::PaneSidecar {
                pane: pane.pane_id.clone(),
                window: pane.window_id.clone(),
                server_pid: stamp.pid,
                server_start: stamp.start_time,
                cwd: payload.cwd.clone().unwrap_or_default(),
                ts: now_secs(),
            },
        );
    }
    Ok(())
}

/// Handle the `Stop` hook.
pub fn dispatch_stop(payload: &HookPayload, state_dir: &Path, tmux: &dyn TmuxOps) -> Result<()> {
    if payload.is_subagent_event() {
        return Ok(());
    }

    let resolved = resolve(payload, state_dir, tmux);

    let transcript = payload
        .transcript_path
        .as_deref()
        .map(expand_tilde)
        .filter(|p| p.exists());
    let stop = classify_stop(
        payload.last_assistant_message.as_deref(),
        transcript.as_deref(),
    );
    let status = FlashStatus::of(stop, resolved.source);

    play_sound(status);
    write_state(payload, state_dir, &resolved, status.as_str());
    flash(tmux, &resolved, status, flash_timeout(), now_secs())
}

/// Handle the `Notification` hook — the agent wants something from you.
pub fn dispatch_notify(payload: &HookPayload, state_dir: &Path, tmux: &dyn TmuxOps) -> Result<()> {
    if payload.is_subagent_event() {
        return Ok(());
    }
    if !notification_is_blocking(payload.notification_type.as_deref()) {
        return Ok(());
    }

    let resolved = resolve(payload, state_dir, tmux);
    let status = FlashStatus::of(StopStatus::Waiting, resolved.source);

    play_sound(status);
    write_state(payload, state_dir, &resolved, status.as_str());
    flash(tmux, &resolved, status, flash_timeout(), now_secs())
}

// ── flashing ──────────────────────────────────────────────────────────────────

/// Light the pane (or, when the pane is unknowable, just the window) and schedule
/// its expiry.
pub fn flash(
    tmux: &dyn TmuxOps,
    resolved: &Resolved,
    status: FlashStatus,
    timeout: u64,
    now: u64,
) -> Result<()> {
    let until = (now + timeout).to_string();

    match &resolved.resolution {
        Resolution::Pane(pane) => {
            tmux.set_user_option(
                OptionScope::Pane,
                &pane.pane_id,
                PANE_STATUS,
                status.as_str(),
            )?;
            tmux.set_user_option(OptionScope::Pane, &pane.pane_id, PANE_UNTIL, &until)?;
            recompute_window(tmux, &pane.window_id, now)?;
            schedule_clear(
                tmux,
                timeout,
                &["--pane", &pane.pane_id, "--window", &pane.window_id],
            )
        }
        Resolution::Window { window_id, .. } => {
            // No pane to attribute this to; claim the tab directly.
            tmux.set_user_option(OptionScope::Window, window_id, WIN_UNTIL, &until)?;
            recompute_window(tmux, window_id, now)?;
            // The aggregate only raises the tab to the level its panes justify, so
            // set the window value after recomputing.
            raise_window_status(tmux, window_id, status)?;
            schedule_clear(tmux, timeout, &["--window", window_id])
        }
        Resolution::None(reason) => {
            // Deliberately visible: a hook that cannot place itself is a bug worth
            // seeing, not a silent no-op like it used to be.
            eprintln!("meldr claude-hook: no tmux target ({reason:?}); nothing flashed");
            Ok(())
        }
    }
}

/// Recompute a window's `@cc_status` from the panes inside it.
///
/// The tab shows the most urgent thing happening in the window. A window-scoped
/// claim (a background job with no known pane) is honoured until its own deadline
/// so that recomputing on behalf of some other pane does not wipe it.
pub fn recompute_window(tmux: &dyn TmuxOps, window_id: &str, now: u64) -> Result<()> {
    let worst = tmux
        .window_pane_options(window_id, PANE_STATUS)?
        .into_iter()
        .filter_map(|(_pane, value)| FlashStatus::parse(&value))
        .max_by_key(|s| s.rank());

    if let Some(worst) = worst {
        return tmux.set_user_option(OptionScope::Window, window_id, WIN_STATUS, worst.as_str());
    }

    // No pane is lit. Keep a live window-level claim, otherwise go dark.
    let window_claim_live = tmux
        .show_user_option(OptionScope::Window, window_id, WIN_UNTIL)?
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|until| now < until);

    if !window_claim_live {
        tmux.unset_user_option(OptionScope::Window, window_id, WIN_STATUS)?;
        tmux.unset_user_option(OptionScope::Window, window_id, WIN_UNTIL)?;
    }
    Ok(())
}

/// Set the window status if `status` is at least as urgent as what is already there.
fn raise_window_status(tmux: &dyn TmuxOps, window_id: &str, status: FlashStatus) -> Result<()> {
    let current = tmux
        .show_user_option(OptionScope::Window, window_id, WIN_STATUS)?
        .and_then(|v| FlashStatus::parse(&v));
    if current.is_none_or(|c| status.rank() >= c.rank()) {
        tmux.set_user_option(OptionScope::Window, window_id, WIN_STATUS, status.as_str())?;
    }
    Ok(())
}

/// What `clear` should act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearTarget {
    /// One pane. `window_id` is carried explicitly because a pane's options die
    /// with the pane, and the window still needs recomputing afterwards.
    Pane { pane_id: String, window_id: String },
    /// A window: its own claim, and optionally every pane inside it.
    Window { window_id: String, all_panes: bool },
}

/// Expire a flash.
///
/// Idempotent and safe to call early: unless `force` is set, a status is only
/// dropped once its recorded deadline has passed, so a later flash that pushed the
/// deadline out survives an earlier flash's timer.
pub fn clear(tmux: &dyn TmuxOps, target: &ClearTarget, now: u64, force: bool) -> Result<()> {
    match target {
        ClearTarget::Pane { pane_id, window_id } => {
            clear_pane(tmux, pane_id, now, force)?;
            recompute_window(tmux, window_id, now)
        }
        ClearTarget::Window {
            window_id,
            all_panes,
        } => {
            if *all_panes {
                let panes: Vec<String> = tmux
                    .window_pane_options(window_id, PANE_STATUS)?
                    .into_iter()
                    .map(|(pane, _)| pane)
                    .collect();
                for pane in panes {
                    clear_pane(tmux, &pane, now, force)?;
                }
            }
            if force || window_claim_expired(tmux, window_id, now)? {
                tmux.unset_user_option(OptionScope::Window, window_id, WIN_UNTIL)?;
            }
            recompute_window(tmux, window_id, now)
        }
    }
}

fn clear_pane(tmux: &dyn TmuxOps, pane_id: &str, now: u64, force: bool) -> Result<()> {
    // A pane that has gone away takes its options with it; nothing to do.
    if !tmux.pane_exists(pane_id) {
        return Ok(());
    }
    let expired = tmux
        .show_user_option(OptionScope::Pane, pane_id, PANE_UNTIL)?
        .and_then(|v| v.parse::<u64>().ok())
        .is_none_or(|until| now >= until);

    if force || expired {
        tmux.unset_user_option(OptionScope::Pane, pane_id, PANE_STATUS)?;
        tmux.unset_user_option(OptionScope::Pane, pane_id, PANE_UNTIL)?;
    }
    Ok(())
}

fn window_claim_expired(tmux: &dyn TmuxOps, window_id: &str, now: u64) -> Result<bool> {
    Ok(tmux
        .show_user_option(OptionScope::Window, window_id, WIN_UNTIL)?
        .and_then(|v| v.parse::<u64>().ok())
        .is_none_or(|until| now >= until))
}

/// Ask the tmux server to run `meldr claude-hook clear …` after `timeout` seconds.
///
/// Uses the absolute path to this executable: the tmux server's `PATH` is whatever
/// it inherited when it started and frequently lacks `~/.cargo/bin`, so a bare
/// `meldr` would fail silently and leave the flash stuck.
fn schedule_clear(tmux: &dyn TmuxOps, timeout: u64, args: &[&str]) -> Result<()> {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "meldr".to_string());

    let mut cmd = format!("sleep {timeout}; {}", shell_quote(&exe));
    cmd.push_str(" claude-hook clear");
    for arg in args {
        cmd.push(' ');
        cmd.push_str(&shell_quote(arg));
    }
    tmux.run_shell_bg(&cmd)
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@%_-./:+".contains(&b))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn resolve(payload: &HookPayload, state_dir: &Path, tmux: &dyn TmuxOps) -> Resolved {
    let env = RealEnv;
    PaneResolver {
        env: &env,
        tmux,
        procs: &crate::core::proc_table::RealProcTable::new(),
        state_dir,
        payload_session_id: payload.session_id.as_deref(),
        payload_cwd: payload.cwd.as_deref(),
        own_pid: std::process::id(),
    }
    .resolve()
}

/// Resolve for the `selftest` subcommand, which reports what it found.
pub fn resolve_for_selftest(cwd: Option<&str>, state_dir: &Path, tmux: &dyn TmuxOps) -> Resolved {
    let env = RealEnv;
    PaneResolver {
        env: &env,
        tmux,
        procs: &crate::core::proc_table::RealProcTable::new(),
        state_dir,
        payload_session_id: None,
        payload_cwd: cwd,
        own_pid: std::process::id(),
    }
    .resolve()
}

fn write_state(payload: &HookPayload, state_dir: &Path, resolved: &Resolved, status: &str) {
    let Some(sid) = payload.session_id.as_deref().filter(|s| !s.is_empty()) else {
        return;
    };
    let state = SessionState {
        status: status.to_string(),
        ts: now_secs(),
        cwd: payload.cwd.clone().unwrap_or_default(),
        pane: resolved
            .pane()
            .map(|p| p.pane_id.clone())
            .unwrap_or_default(),
        window: resolved.window_id().unwrap_or_default().to_string(),
        window_name: resolved.window_name().to_string(),
    };
    let _ = write_session_state(state_dir, sid, &state);
}

/// Play a notification sound if `afplay` is available (macOS only).
/// Fire-and-forget: the process is spawned in the background and not waited on.
fn play_sound(status: FlashStatus) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("afplay")
            .arg(status.sound())
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::RecordingTmux;
    use crate::tmux::snapshot::PaneRow;
    use resolver::PaneRef;

    fn row(pane: &str, win: &str) -> PaneRow {
        PaneRow {
            // Not an ancestor of the test process, so the process-tree tier cannot
            // accidentally claim these panes.
            pane_pid: 4_000_000_001,
            pane_id: pane.to_string(),
            window_id: win.to_string(),
            session_id: "$0".to_string(),
            cwd: std::path::PathBuf::from("/w"),
            current_command: "claude".to_string(),
            window_name: "w".to_string(),
        }
    }

    fn pane_res(pane: &str, win: &str) -> Resolved {
        Resolved {
            resolution: Resolution::Pane(PaneRef {
                pane_id: pane.to_string(),
                window_id: win.to_string(),
                window_name: "w".to_string(),
            }),
            source: Source::Interactive,
            tier: resolver::Tier::ProcessTree,
        }
    }

    fn window_res(win: &str) -> Resolved {
        Resolved {
            resolution: Resolution::Window {
                window_id: win.to_string(),
                window_name: "w".to_string(),
            },
            source: Source::Background,
            tier: resolver::Tier::Cwd,
        }
    }

    fn tmux_with(panes: &[(&str, &str)]) -> RecordingTmux {
        RecordingTmux::new(vec![]).with_panes(panes.iter().map(|(p, w)| row(p, w)).collect())
    }

    // ── status semantics ──────────────────────────────────────────────────────

    #[test]
    fn status_values_are_the_published_strings() {
        assert_eq!(FlashStatus::Done.as_str(), "done");
        assert_eq!(FlashStatus::Waiting.as_str(), "waiting");
        assert_eq!(FlashStatus::BgDone.as_str(), "bg-done");
        assert_eq!(FlashStatus::BgWaiting.as_str(), "bg-waiting");
        for s in [
            FlashStatus::Done,
            FlashStatus::Waiting,
            FlashStatus::BgDone,
            FlashStatus::BgWaiting,
        ] {
            assert_eq!(FlashStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(FlashStatus::parse(""), None);
        assert_eq!(FlashStatus::parse("nonsense"), None);
    }

    #[test]
    fn waiting_outranks_done_and_foreground_outranks_background() {
        assert!(FlashStatus::Waiting.rank() > FlashStatus::BgWaiting.rank());
        assert!(FlashStatus::BgWaiting.rank() > FlashStatus::Done.rank());
        assert!(FlashStatus::Done.rank() > FlashStatus::BgDone.rank());
    }

    #[test]
    fn background_source_selects_background_values() {
        assert_eq!(
            FlashStatus::of(StopStatus::Done, Source::Background),
            FlashStatus::BgDone
        );
        assert_eq!(
            FlashStatus::of(StopStatus::Waiting, Source::Background),
            FlashStatus::BgWaiting
        );
        assert_eq!(
            FlashStatus::of(StopStatus::Done, Source::Interactive),
            FlashStatus::Done
        );
    }

    // ── flash ─────────────────────────────────────────────────────────────────

    #[test]
    fn flash_sets_the_pane_and_derives_the_window() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 100).unwrap();

        assert_eq!(
            t.opt(OptionScope::Pane, "%1", PANE_STATUS).as_deref(),
            Some("done")
        );
        assert_eq!(
            t.opt(OptionScope::Pane, "%1", PANE_UNTIL).as_deref(),
            Some("105")
        );
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).as_deref(),
            Some("done"),
            "tab is derived from the pane"
        );
    }

    #[test]
    fn window_shows_the_most_urgent_pane() {
        let t = tmux_with(&[("%1", "@1"), ("%2", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 100).unwrap();
        flash(&t, &pane_res("%2", "@1"), FlashStatus::Waiting, 5, 100).unwrap();
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).as_deref(),
            Some("waiting")
        );
    }

    #[test]
    fn flash_refuses_an_empty_window_target() {
        // `set-option -w -t ''` would have landed on whatever window was focused.
        let t = tmux_with(&[]);
        let err = flash(&t, &window_res(""), FlashStatus::BgDone, 5, 0).unwrap_err();
        assert!(err.to_string().contains("refusing to target"));
        assert!(t.set_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn flash_refuses_a_positional_pane_target() {
        let t = tmux_with(&[]);
        assert!(flash(&t, &pane_res("@1.0", "@1"), FlashStatus::Done, 5, 0).is_err());
        assert!(t.set_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn flash_on_an_unresolved_event_touches_nothing() {
        let t = tmux_with(&[("%1", "@1")]);
        let unresolved = Resolved {
            resolution: Resolution::None(resolver::NoMatch::NoHints),
            source: Source::Interactive,
            tier: resolver::Tier::NoneOfThem,
        };
        flash(&t, &unresolved, FlashStatus::Done, 5, 0).unwrap();
        assert!(t.set_calls.lock().unwrap().is_empty());
        assert!(t.bg_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn tmux_failures_are_not_swallowed() {
        let t = tmux_with(&[("%1", "@1")]).failing("set-option");
        assert!(flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 0).is_err());
    }

    #[test]
    fn window_scoped_flash_sets_the_tab_without_a_pane() {
        let t = tmux_with(&[("%1", "@1"), ("%2", "@1")]);
        flash(&t, &window_res("@1"), FlashStatus::BgDone, 5, 100).unwrap();
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).as_deref(),
            Some("bg-done")
        );
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_UNTIL).as_deref(),
            Some("105")
        );
        assert!(
            t.opt(OptionScope::Pane, "%1", PANE_STATUS).is_none(),
            "no pane may be blamed for a background job"
        );
    }

    #[test]
    fn an_interactive_pane_outranks_a_background_window_claim() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &window_res("@1"), FlashStatus::BgDone, 60, 100).unwrap();
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Waiting, 5, 100).unwrap();
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).as_deref(),
            Some("waiting")
        );
    }

    #[test]
    fn schedule_uses_an_absolute_executable_and_names_the_window() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 7, 0).unwrap();
        let bg = t.bg_calls.lock().unwrap();
        let cmd = bg.first().expect("a clear must be scheduled");
        assert!(cmd.starts_with("sleep 7; "), "got {cmd}");
        assert!(cmd.contains("claude-hook clear"));
        assert!(cmd.contains("--pane %1"));
        // The window travels with it so the aggregate can be recomputed even if the
        // pane has died by the time the timer fires.
        assert!(cmd.contains("--window @1"));
        let exe = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(cmd.contains(&exe), "must use the absolute path; got {cmd}");
    }

    #[test]
    fn shell_quoting_survives_spaces_and_quotes() {
        assert_eq!(shell_quote("%1"), "%1");
        assert_eq!(shell_quote("/usr/local/bin/meldr"), "/usr/local/bin/meldr");
        assert_eq!(shell_quote("/a b/meldr"), "'/a b/meldr'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    // ── clear ─────────────────────────────────────────────────────────────────

    fn clear_pane_target(pane: &str, win: &str) -> ClearTarget {
        ClearTarget::Pane {
            pane_id: pane.to_string(),
            window_id: win.to_string(),
        }
    }

    #[test]
    fn clear_before_the_deadline_does_nothing() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 100).unwrap();
        clear(&t, &clear_pane_target("%1", "@1"), 104, false).unwrap();
        assert_eq!(
            t.opt(OptionScope::Pane, "%1", PANE_STATUS).as_deref(),
            Some("done")
        );
    }

    #[test]
    fn clear_at_the_deadline_unsets_pane_and_window() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 100).unwrap();
        clear(&t, &clear_pane_target("%1", "@1"), 105, false).unwrap();
        assert!(t.opt(OptionScope::Pane, "%1", PANE_STATUS).is_none());
        assert!(t.opt(OptionScope::Pane, "%1", PANE_UNTIL).is_none());
        assert!(t.opt(OptionScope::Window, "@1", WIN_STATUS).is_none());
    }

    #[test]
    fn overlapping_flashes_never_leave_the_tab_stuck() {
        // The exact failure the generation token used to produce: two flashes a
        // second apart, the first timer firing while the second is still live.
        let t = tmux_with(&[("%1", "@1"), ("%2", "@1")]);

        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 100).unwrap(); // until 105
        flash(&t, &pane_res("%2", "@1"), FlashStatus::Waiting, 5, 101).unwrap(); // until 106

        // First timer fires at 105: its own pane expires, the second must not.
        clear(&t, &clear_pane_target("%1", "@1"), 105, false).unwrap();
        assert!(t.opt(OptionScope::Pane, "%1", PANE_STATUS).is_none());
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).as_deref(),
            Some("waiting"),
            "the still-live pane keeps the tab lit"
        );

        // Second timer fires at 106: everything goes dark. Under the old scheme the
        // first timer had already destroyed the token this check depended on, and
        // the tab stayed lit indefinitely.
        clear(&t, &clear_pane_target("%2", "@1"), 106, false).unwrap();
        assert!(t.opt(OptionScope::Pane, "%2", PANE_STATUS).is_none());
        assert!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).is_none(),
            "tab must go dark once no pane is lit"
        );
    }

    #[test]
    fn re_flashing_the_same_pane_pushes_the_deadline_out() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 100).unwrap();
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 103).unwrap(); // until 108
        clear(&t, &clear_pane_target("%1", "@1"), 105, false).unwrap();
        assert_eq!(
            t.opt(OptionScope::Pane, "%1", PANE_STATUS).as_deref(),
            Some("done"),
            "the earlier timer must not expire a refreshed claim"
        );
        clear(&t, &clear_pane_target("%1", "@1"), 108, false).unwrap();
        assert!(t.opt(OptionScope::Pane, "%1", PANE_STATUS).is_none());
    }

    #[test]
    fn forced_clear_ignores_the_deadline() {
        // What a tmux `after-select-pane` hook wants: you looked at it, so drop it.
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Waiting, 600, 100).unwrap();
        clear(&t, &clear_pane_target("%1", "@1"), 101, true).unwrap();
        assert!(t.opt(OptionScope::Pane, "%1", PANE_STATUS).is_none());
        assert!(t.opt(OptionScope::Window, "@1", WIN_STATUS).is_none());
    }

    #[test]
    fn clear_window_all_panes_clears_the_whole_window() {
        let t = tmux_with(&[("%1", "@1"), ("%2", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 600, 100).unwrap();
        flash(&t, &pane_res("%2", "@1"), FlashStatus::Waiting, 600, 100).unwrap();

        clear(
            &t,
            &ClearTarget::Window {
                window_id: "@1".to_string(),
                all_panes: true,
            },
            101,
            true,
        )
        .unwrap();

        assert!(t.opt(OptionScope::Pane, "%1", PANE_STATUS).is_none());
        assert!(t.opt(OptionScope::Pane, "%2", PANE_STATUS).is_none());
        assert!(t.opt(OptionScope::Window, "@1", WIN_STATUS).is_none());
    }

    #[test]
    fn clear_window_leaves_other_windows_alone() {
        let t = tmux_with(&[("%1", "@1"), ("%2", "@2")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 600, 100).unwrap();
        flash(&t, &pane_res("%2", "@2"), FlashStatus::Waiting, 600, 100).unwrap();

        clear(
            &t,
            &ClearTarget::Window {
                window_id: "@1".to_string(),
                all_panes: true,
            },
            101,
            true,
        )
        .unwrap();

        assert!(t.opt(OptionScope::Window, "@1", WIN_STATUS).is_none());
        assert_eq!(
            t.opt(OptionScope::Window, "@2", WIN_STATUS).as_deref(),
            Some("waiting")
        );
    }

    #[test]
    fn clear_of_a_dead_pane_still_recomputes_the_window() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &pane_res("%1", "@1"), FlashStatus::Done, 5, 100).unwrap();
        // %9 never existed; the window must still be reconciled.
        clear(&t, &clear_pane_target("%9", "@1"), 999, false).unwrap();
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).as_deref(),
            Some("done"),
            "%1 is still lit, so the tab stays lit"
        );
    }

    #[test]
    fn background_window_claim_expires_on_its_own() {
        let t = tmux_with(&[("%1", "@1")]);
        flash(&t, &window_res("@1"), FlashStatus::BgWaiting, 5, 100).unwrap();
        clear(
            &t,
            &ClearTarget::Window {
                window_id: "@1".to_string(),
                all_panes: false,
            },
            104,
            false,
        )
        .unwrap();
        assert_eq!(
            t.opt(OptionScope::Window, "@1", WIN_STATUS).as_deref(),
            Some("bg-waiting"),
            "not yet due"
        );
        clear(
            &t,
            &ClearTarget::Window {
                window_id: "@1".to_string(),
                all_panes: false,
            },
            105,
            false,
        )
        .unwrap();
        assert!(t.opt(OptionScope::Window, "@1", WIN_STATUS).is_none());
        assert!(t.opt(OptionScope::Window, "@1", WIN_UNTIL).is_none());
    }

    // ── notification filtering ────────────────────────────────────────────────

    #[test]
    fn only_blocking_notifications_flash() {
        assert!(notification_is_blocking(Some("permission_prompt")));
        assert!(notification_is_blocking(Some("idle_prompt")));
        assert!(notification_is_blocking(Some("agent_needs_input")));
        // These fired a "waiting" tab for no reason under matcher `*`.
        assert!(!notification_is_blocking(Some("auth_success")));
        assert!(!notification_is_blocking(Some("agent_completed")));
        assert!(!notification_is_blocking(Some("quota_auto_resume_fired")));
        // Absent type: keep the old behaviour rather than going quiet.
        assert!(notification_is_blocking(None));
    }

    // ── payload ───────────────────────────────────────────────────────────────

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
    fn parses_the_fields_the_hooks_actually_send() {
        let p = HookPayload::from_json(
            r#"{"session_id":"s1","cwd":"/tmp","last_assistant_message":"Shall I?",
                "notification_type":"permission_prompt"}"#,
        );
        assert_eq!(p.session_id.as_deref(), Some("s1"));
        assert_eq!(p.last_assistant_message.as_deref(), Some("Shall I?"));
        assert_eq!(p.notification_type.as_deref(), Some("permission_prompt"));
    }

    #[test]
    fn from_json_defaults_on_invalid() {
        let p = HookPayload::from_json("not json {{{{");
        assert!(p.session_id.is_none());
    }

    #[test]
    fn unknown_payload_fields_are_ignored() {
        // Claude adds fields over time; a new one must not break parsing.
        let p = HookPayload::from_json(r#"{"session_id":"s","brand_new_field":42}"#);
        assert_eq!(p.session_id.as_deref(), Some("s"));
    }

    // ── dispatch ──────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_stop_subagent_is_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let t = tmux_with(&[("%1", "@1")]);
        let payload = HookPayload {
            hook_event_name: Some("SubagentStop".to_string()),
            agent_id: Some("agent-x".to_string()),
            session_id: Some("sess3".to_string()),
            ..Default::default()
        };
        dispatch_stop(&payload, tmp.path(), &t).unwrap();
        assert!(t.set_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn dispatch_notify_ignores_non_blocking_types() {
        let tmp = tempfile::TempDir::new().unwrap();
        let t = tmux_with(&[("%1", "@1")]);
        let payload = HookPayload {
            hook_event_name: Some("Notification".to_string()),
            session_id: Some("s".to_string()),
            notification_type: Some("auth_success".to_string()),
            cwd: Some("/w".to_string()),
            ..Default::default()
        };
        dispatch_notify(&payload, tmp.path(), &t).unwrap();
        assert!(t.set_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn dispatch_session_start_sweeps_legacy_sidecars() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("old.parent_pane"), "%3").unwrap();
        let t = tmux_with(&[("%1", "@1")]);
        let payload = HookPayload {
            hook_event_name: Some("SessionStart".to_string()),
            session_id: Some("new".to_string()),
            cwd: Some("/w".to_string()),
            ..Default::default()
        };
        dispatch_session_start(&payload, tmp.path(), &t).unwrap();
        assert!(
            !tmp.path().join("old.parent_pane").exists(),
            "unverifiable legacy sidecars must be collected"
        );
    }

    #[test]
    fn dispatch_session_start_records_only_an_exact_pane() {
        // Two panes share the directory, so the pane is ambiguous; recording a
        // guess here is what made a bad resolution stick for the whole session.
        let tmp = tempfile::TempDir::new().unwrap();
        let t = tmux_with(&[("%1", "@1"), ("%2", "@1")]);
        let payload = HookPayload {
            hook_event_name: Some("SessionStart".to_string()),
            session_id: Some("amb".to_string()),
            cwd: Some("/w".to_string()),
            ..Default::default()
        };
        dispatch_session_start(&payload, tmp.path(), &t).unwrap();
        assert!(sidecar::read_pane_sidecar(tmp.path(), "amb").is_none());
    }
}
