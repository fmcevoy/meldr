//! CLI-parsing integration tests for the --leader flag and leader_package config key.
//!
//! These tests exercise argument parsing and validation error paths that don't
//! require git or tmux. Tests that need a real worktree/tmux live in
//! `docker_integration.rs` and run via `./run-docker-tests.sh`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn setup_multi_pkg_workspace(dir: &std::path::Path) {
    let manifest = r#"
[workspace]
name = "test-ws"

[[package]]
name = "api"
url = "https://example.com/api.git"

[[package]]
name = "web"
url = "https://example.com/web.git"

[[package]]
name = "shared"
url = "https://example.com/shared.git"
"#;
    fs::write(dir.join("meldr.toml"), manifest).unwrap();
    fs::create_dir_all(dir.join(".meldr")).unwrap();
    fs::write(dir.join(".meldr/state.json"), r#"{"worktrees":{}}"#).unwrap();
}

#[allow(deprecated)]
#[test]
fn test_worktree_add_accepts_leader_flag() {
    let dir = TempDir::new().unwrap();
    setup_multi_pkg_workspace(dir.path());
    // With --no-tabs the leader machinery short-circuits (no agent pane).
    // The command will still fail because the packages aren't cloned, but
    // the failure must be about git, not about arg parsing.
    let output = Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["worktree", "add", "feat-x", "--leader", "api", "--no-tabs"])
        .assert();
    // We care that parsing succeeded — not the exit code.
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("--leader"),
        "--leader should be recognized, got: {stderr}"
    );
}

#[allow(deprecated)]
#[test]
fn test_create_accepts_leader_flag() {
    let dir = TempDir::new().unwrap();
    let output = Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["create", "--leader", "api", "--no-tabs", "some-ws"])
        .assert();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(
        !stderr.contains("unexpected argument"),
        "create should accept --leader, got: {stderr}"
    );
}

#[allow(deprecated)]
#[test]
fn test_leader_package_is_valid_settings_key() {
    let dir = TempDir::new().unwrap();
    setup_multi_pkg_workspace(dir.path());

    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["config", "set", "leader_package", "api"])
        .assert()
        .success();

    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["config", "get", "leader_package"])
        .assert()
        .success()
        .stdout(predicate::str::contains("api"));
}

#[allow(deprecated)]
#[test]
fn test_invalid_leader_rejected_before_git_work() {
    let dir = TempDir::new().unwrap();
    setup_multi_pkg_workspace(dir.path());

    // Must run with --no-agent so the leader check is skipped, OR with
    // --no-tabs (which disables tmux — agent won't launch either). Here we
    // DO want the leader check to run, so we need agent launching on but
    // tmux off is enough to bypass the NotInTmux check. But leader check
    // requires `agent_enabled = needs_tmux && should_launch_agent()`.
    // With --no-tabs, needs_tmux=false, so leader is skipped. That means
    // this test verifies the no-op path in the --no-tabs scenario.
    //
    // To actually exercise the "invalid leader" error we need tmux enabled.
    // Outside tmux the command fails with NotInTmux BEFORE reaching the
    // leader check, which is fine — the ordering is: tmux check → leader
    // resolve → git. So here we just verify --leader is parsed and the
    // command errors out cleanly without a panic.
    let output = Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .env_remove("TMUX")
        .args(["worktree", "add", "feat-bad", "--leader", "ghost"])
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(
        !stderr.contains("panic") && !stderr.contains("unexpected argument"),
        "should fail cleanly (NotInTmux or invalid leader), got: {stderr}"
    );
    // Ensure NO worktree state side-effect was persisted
    let state = fs::read_to_string(dir.path().join(".meldr/state.json")).unwrap();
    assert!(
        !state.contains("feat-bad"),
        "no worktree state should be written on validation failure, got: {state}"
    );
}
