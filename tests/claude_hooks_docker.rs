//! Docker integration tests for `meldr claude-hook` subcommands.
//!
//! These require:
//!  - A compiled `meldr` binary
//!  - `tmux` on PATH (present in Dockerfile.test)
//!
//! Each test creates an isolated detached tmux session and a per-test HOME
//! directory so state files never collide between parallel test runs.
//!
//! Gated behind the `docker-tests` Cargo feature — not run by bare `cargo test`.

#![cfg(feature = "docker-tests")]

use assert_cmd::Command;
use std::fs;
use std::process;
use tempfile::TempDir;

// ─── tmux helpers ─────────────────────────────────────────────────────────────

fn tmux_ok(args: &[&str]) -> String {
    let out = process::Command::new("tmux")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("tmux {args:?} failed to spawn: {e}"));
    assert!(
        out.status.success(),
        "tmux {args:?} exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn tmux_silent(args: &[&str]) {
    let _ = process::Command::new("tmux").args(args).output();
}

/// Create a detached session. Returns `(pane_id, window_id)`.
fn new_session(name: &str) -> (String, String) {
    // Ensure a tmux server is running — no-op if already up.
    let _ = process::Command::new("tmux").arg("start-server").output();
    tmux_ok(&["new-session", "-d", "-s", name, "-x", "200", "-y", "50"]);
    let pane_id = tmux_ok(&["display-message", "-t", name, "-p", "#{pane_id}"]);
    let window_id = tmux_ok(&["display-message", "-t", name, "-p", "#{window_id}"]);
    (pane_id, window_id)
}

fn kill_session(name: &str) {
    tmux_silent(&["kill-session", "-t", name]);
}

/// Read the `@cc_status` window-scoped user option; returns empty string if unset.
fn cc_status(window_id: &str) -> String {
    let out = process::Command::new("tmux")
        .args(["show-options", "-wqv", "-t", window_id, "@cc_status"])
        .output()
        .expect("tmux show-options failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// ─── tests ────────────────────────────────────────────────────────────────────

/// `meldr claude-hook stop` with no transcript → @cc_status = "done".
#[test]
fn stop_no_transcript_flashes_done() {
    let sess = "meldr-hook-stop-done";
    let home = TempDir::new().unwrap();
    let (pane_id, window_id) = new_session(sess);

    Command::cargo_bin("meldr")
        .unwrap()
        .env("HOME", home.path())
        .env("MELDR_TMUX_PANE", &pane_id)
        .env("MELDR_TMUX_WINDOW_ID", &window_id)
        .env("MELDR_CC_TIMEOUT", "300") // prevent auto-clear during test
        .args(["claude-hook", "stop"])
        .write_stdin(r#"{"hook_event_name":"Stop","session_id":"test-stop-done"}"#)
        .assert()
        .success();

    assert_eq!(
        cc_status(&window_id),
        "done",
        "@cc_status should be 'done' after Stop with no transcript"
    );
    kill_session(sess);
}

/// `meldr claude-hook notify` always → @cc_status = "waiting".
#[test]
fn notify_always_flashes_waiting() {
    let sess = "meldr-hook-notify";
    let home = TempDir::new().unwrap();
    let (pane_id, window_id) = new_session(sess);

    Command::cargo_bin("meldr")
        .unwrap()
        .env("HOME", home.path())
        .env("MELDR_TMUX_PANE", &pane_id)
        .env("MELDR_TMUX_WINDOW_ID", &window_id)
        .env("MELDR_CC_TIMEOUT", "300")
        .args(["claude-hook", "notify"])
        .write_stdin(r#"{"hook_event_name":"Notification","session_id":"test-notify"}"#)
        .assert()
        .success();

    assert_eq!(
        cc_status(&window_id),
        "waiting",
        "@cc_status should be 'waiting' after Notify"
    );
    kill_session(sess);
}

/// `meldr claude-hook stop` with an AskUserQuestion transcript → @cc_status = "waiting".
#[test]
fn stop_with_ask_user_question_transcript_flashes_waiting() {
    let sess = "meldr-hook-stop-waiting";
    let home = TempDir::new().unwrap();
    let (pane_id, window_id) = new_session(sess);

    let transcript_dir = home.path().join("transcripts");
    fs::create_dir_all(&transcript_dir).unwrap();
    let transcript = transcript_dir.join("test.jsonl");
    // Transcript with an AskUserQuestion tool_use in the last assistant message.
    fs::write(
        &transcript,
        r#"{"role":"assistant","content":[{"type":"tool_use","name":"AskUserQuestion","id":"x","input":{}}]}"#,
    )
    .unwrap();

    let payload = format!(
        r#"{{"hook_event_name":"Stop","session_id":"test-stop-waiting","transcript_path":"{}"}}"#,
        transcript.display()
    );

    Command::cargo_bin("meldr")
        .unwrap()
        .env("HOME", home.path())
        .env("MELDR_TMUX_PANE", &pane_id)
        .env("MELDR_TMUX_WINDOW_ID", &window_id)
        .env("MELDR_CC_TIMEOUT", "300")
        .args(["claude-hook", "stop"])
        .write_stdin(payload)
        .assert()
        .success();

    assert_eq!(
        cc_status(&window_id),
        "waiting",
        "@cc_status should be 'waiting' when transcript contains AskUserQuestion"
    );
    kill_session(sess);
}

/// Regression test for the ~/fmcevoy vs ~/fmcevoy_tools sibling-prefix bug.
///
/// Two launcher entries exist: one for `/tmp/meldr-sibling-base` and one for
/// `/tmp/meldr-sibling-baseplus`. A session whose cwd is under `…/baseplus/…`
/// must resolve to the `baseplus` pane, not the `base` pane.
#[test]
fn session_start_sibling_prefix_regression() {
    let sess_base = "meldr-sibling-base";
    let sess_plus = "meldr-sibling-plus";
    let home = TempDir::new().unwrap();

    let (pane_base, _) = new_session(sess_base);
    let (pane_plus, _) = new_session(sess_plus);

    // Seed the launcher registry manually.
    let launcher_dir = home.path().join(".cache/claude-agents/launchers");
    fs::create_dir_all(&launcher_dir).unwrap();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let base_entry = serde_json::json!({
        "pane": pane_base,
        "window": "@1",
        "cwd": "/tmp/meldr-sibling-base",
        "ts": now_ms
    });
    fs::write(
        launcher_dir.join("100-1-0.json"),
        serde_json::to_string(&base_entry).unwrap(),
    )
    .unwrap();

    let plus_entry = serde_json::json!({
        "pane": pane_plus,
        "window": "@2",
        "cwd": "/tmp/meldr-sibling-baseplus",
        "ts": now_ms + 1
    });
    fs::write(
        launcher_dir.join("100-1-1.json"),
        serde_json::to_string(&plus_entry).unwrap(),
    )
    .unwrap();

    // Run session-start with cwd under /tmp/meldr-sibling-baseplus. The session
    // has no MELDR_TMUX_PANE or TMUX_PANE, so the resolver falls through to Tier 5.
    Command::cargo_bin("meldr")
        .unwrap()
        .env("HOME", home.path())
        .env_remove("MELDR_TMUX_PANE")
        .env_remove("MELDR_TMUX_WINDOW_ID")
        .env_remove("TMUX_PANE")
        .env_remove("MELDR_AGENT_SESSION")
        .args(["claude-hook", "session-start"])
        .write_stdin(
            serde_json::json!({
                "hook_event_name": "SessionStart",
                "session_id": "sibling-test-session",
                "cwd": "/tmp/meldr-sibling-baseplus/project"
            })
            .to_string(),
        )
        .assert()
        .success();

    // The sidecar must point to pane_plus, not pane_base.
    let state_dir = home.path().join(".cache/claude-agents");
    let sidecar = state_dir.join("sibling-test-session.parent_pane");
    assert!(sidecar.exists(), "sidecar file should have been written");
    let resolved = fs::read_to_string(&sidecar).unwrap();
    let resolved = resolved.trim();
    assert_eq!(
        resolved, pane_plus,
        "should resolve to pane_plus ({pane_plus}) not pane_base ({pane_base}) — sibling-prefix bug"
    );

    kill_session(sess_base);
    kill_session(sess_plus);
}
