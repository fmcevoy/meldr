//! End-to-end tests for the Claude notification pipeline against a real tmux
//! server, driving the real `meldr` binary.
//!
//! Every test here is a regression for a specific way notifications used to land
//! on the wrong pane or window, named after the failure it pins down. They are
//! deliberately e2e: each of these bugs was invisible to unit tests because the
//! test double could not reproduce what tmux actually does — accepting an empty
//! `-t` target, accepting a positional `@N.0` target, or handing out `%0` again
//! after a server restart.
//!
//! Gated behind `docker-tests` so a bare `cargo test` on a machine without tmux
//! still passes; run with `./run-docker-tests.sh`.

#![cfg(feature = "docker-tests")]

mod support;

use std::time::Duration;

use support::tmux_server::{HookRun, TmuxServer, wait_until};

/// A worktree-ish directory, since the resolver compares the event's cwd against
/// each pane's own directory.
fn workdir() -> tempfile::TempDir {
    tempfile::TempDir::new().unwrap()
}

fn home() -> tempfile::TempDir {
    tempfile::TempDir::new().unwrap()
}

fn stop_payload(cwd: &str, msg: &str) -> String {
    serde_json::json!({
        "hook_event_name": "Stop",
        "session_id": "sess-test",
        "cwd": cwd,
        "last_assistant_message": msg,
    })
    .to_string()
}

// ── the pane must be the one the hook ran in ──────────────────────────────────

#[test]
fn stop_lights_the_pane_it_ran_in_even_from_a_background_window() {
    // The headline failure. The old resolver read the window id from an
    // environment variable computed by an untargeted `tmux display-message`,
    // which returns the *active* window — so a hook in a background window lit
    // whichever tab happened to be in front of you.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (target_window, target_pane) = srv.new_session(&cwd);
    let (active_window, _) = srv.new_window(&cwd);
    assert_eq!(srv.active_window(), active_window);
    assert_ne!(target_window, active_window);

    let log = HookRun::new(&srv, home.path()).in_pane(
        &target_pane,
        2,
        "stop",
        &stop_payload(&cwd, "All done."),
    );

    assert_eq!(
        srv.cc_pane_status(&target_pane),
        "done",
        "the pane that ran the hook must be the one lit; hook log:\n{log}"
    );
    assert_eq!(srv.cc_status(&target_window), "done");
    assert_eq!(
        srv.cc_status(&active_window),
        "",
        "the focused window must be untouched"
    );
}

#[test]
fn process_tree_resolves_through_one_two_and_three_nested_shells() {
    // Claude wraps hooks in `sh -c`, and the depth is not contractual, so the
    // walk has to work for any of them.
    for depth in 1..=3 {
        let srv = TmuxServer::new();
        let home = home();
        let work = workdir();
        let cwd = work.path().to_string_lossy().to_string();

        let (window, pane) = srv.new_session(&cwd);
        let log = HookRun::new(&srv, home.path()).in_pane(
            &pane,
            depth,
            "stop",
            &stop_payload(&cwd, "Finished."),
        );

        assert_eq!(
            srv.cc_pane_status(&pane),
            "done",
            "depth {depth} failed to resolve; log:\n{log}"
        );
        assert_eq!(srv.cc_status(&window), "done", "depth {depth}");
    }
}

#[test]
fn the_exact_pane_wins_over_siblings_sharing_a_directory() {
    // Three agent panes in one worktree is the standard meldr layout. Matching on
    // directory alone cannot tell them apart, and the old code broke the tie by
    // "most recently launched" — a coin flip.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (window, p1) = srv.new_session(&cwd);
    let p2 = srv.split(&p1, &cwd);
    let p3 = srv.split(&p1, &cwd);

    let log = HookRun::new(&srv, home.path()).in_pane(&p2, 2, "stop", &stop_payload(&cwd, "Done."));

    assert_eq!(srv.cc_pane_status(&p2), "done", "log:\n{log}");
    assert_eq!(srv.cc_pane_status(&p1), "", "sibling must stay dark");
    assert_eq!(srv.cc_pane_status(&p3), "", "sibling must stay dark");
    assert_eq!(srv.cc_status(&window), "done");
}

#[test]
fn a_poisoned_meldr_tmux_pane_is_ignored() {
    // Claude hosts every background job in one long-lived daemon which keeps the
    // environment of whichever pane first started it. `MELDR_TMUX_PANE` therefore
    // named a live but unrelated pane for every later job, and the old Tier 1
    // trusted it above everything else.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (_w1, decoy) = srv.new_session("/tmp");
    let (real_window, real_pane) = srv.new_window(&cwd);

    let mut runner = HookRun::new(&srv, home.path());
    runner
        .env("MELDR_TMUX_PANE", &decoy)
        .env("MELDR_TMUX_WINDOW_ID", &srv.window_of(&decoy));
    let log = runner.in_pane(&real_pane, 2, "stop", &stop_payload(&cwd, "Done."));

    assert_eq!(
        srv.cc_pane_status(&decoy),
        "",
        "the pane named by the stale variable must not be touched; log:\n{log}"
    );
    assert_eq!(srv.cc_pane_status(&real_pane), "done");
    assert_eq!(srv.cc_status(&real_window), "done");
}

#[test]
fn a_sidecar_from_a_previous_tmux_server_is_ignored() {
    // tmux hands out pane ids from `%0` again for every new server, so a recorded
    // `%1` can name a completely unrelated live pane later. 235 of 346 sidecars on
    // the development machine were in exactly this state, and `pane_exists` said
    // all of them were fine.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (_window, decoy) = srv.new_session("/tmp");
    let (real_window, real_pane) = srv.new_window(&cwd);

    // A sidecar naming the decoy pane, stamped with a server that is not this one.
    let state_dir = home.path().join(".cache/claude-agents");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("sess-test.pane.json"),
        serde_json::json!({
            "pane": decoy,
            "window": srv.window_of(&decoy),
            "server_pid": 999_999,
            "server_start": 1,
            "cwd": cwd,
            "ts": support::tmux_server::now_secs_for_test(),
        })
        .to_string(),
    )
    .unwrap();

    let log =
        HookRun::new(&srv, home.path()).in_pane(&real_pane, 2, "stop", &stop_payload(&cwd, "Ok."));

    assert_eq!(
        srv.cc_pane_status(&decoy),
        "",
        "a sidecar from another server must be rejected; log:\n{log}"
    );
    assert_eq!(srv.cc_pane_status(&real_pane), "done");
    assert_eq!(srv.cc_status(&real_window), "done");
}

#[test]
fn legacy_parent_pane_sidecars_are_swept_on_session_start() {
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();
    let (_w, pane) = srv.new_session(&cwd);

    let state_dir = home.path().join(".cache/claude-agents");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("ancient.parent_pane"), "%7").unwrap();

    HookRun::new(&srv, home.path()).in_pane(
        &pane,
        1,
        "session-start",
        &serde_json::json!({
            "hook_event_name": "SessionStart",
            "session_id": "sess-test",
            "cwd": cwd,
        })
        .to_string(),
    );

    assert!(
        !state_dir.join("ancient.parent_pane").exists(),
        "unstamped legacy sidecars must be removed"
    );
    // And the session's own location is recorded in the new, verifiable format.
    let recorded = std::fs::read_to_string(state_dir.join("sess-test.pane.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&recorded).unwrap();
    assert_eq!(v["pane"], pane);
    assert_eq!(v["server_pid"], srv.server_pid().parse::<u64>().unwrap());
}

// ── the window must follow the pane ───────────────────────────────────────────

#[test]
fn moving_a_pane_to_another_window_moves_its_notifications() {
    // The window used to be cached alongside the pane and never re-derived, so
    // after `break-pane` the flash kept going to the window the pane had left.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (old_window, p1) = srv.new_session(&cwd);
    let p2 = srv.split(&p1, &cwd);
    let runner = HookRun::new(&srv, home.path());

    runner.in_pane(&p2, 2, "stop", &stop_payload(&cwd, "First."));
    assert_eq!(srv.cc_status(&old_window), "done");

    let new_window = srv.break_pane(&p2);
    assert_ne!(new_window, old_window);

    runner.in_pane(&p2, 2, "stop", &stop_payload(&cwd, "Second."));
    assert_eq!(
        srv.cc_status(&new_window),
        "done",
        "the window the pane now lives in must light up"
    );
}

// ── expiry ────────────────────────────────────────────────────────────────────

#[test]
fn overlapping_flashes_both_clear_and_never_stick() {
    // The old clear timer wiped its generation token unconditionally, so a second
    // flash arriving mid-timer destroyed the token the first timer was about to
    // check — and the tab stayed lit indefinitely.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (window, p1) = srv.new_session(&cwd);
    let p2 = srv.split(&p1, &cwd);
    let mut runner = HookRun::new(&srv, home.path());

    // Short, staggered lifetimes so both timers are in flight at once.
    runner.env("MELDR_CC_TIMEOUT", "2");
    runner.in_pane(&p1, 2, "stop", &stop_payload(&cwd, "First."));
    runner.env("MELDR_CC_TIMEOUT", "4");
    runner.in_pane(&p2, 2, "stop", &stop_payload(&cwd, "Second."));

    assert_eq!(srv.cc_status(&window), "done", "both panes are lit");

    let cleared = wait_until(Duration::from_secs(20), || {
        srv.cc_pane_status(&p1).is_empty()
            && srv.cc_pane_status(&p2).is_empty()
            && srv.cc_status(&window).is_empty()
    });
    assert!(
        cleared,
        "everything must expire; pane1={:?} pane2={:?} window={:?}",
        srv.cc_pane_status(&p1),
        srv.cc_pane_status(&p2),
        srv.cc_status(&window)
    );
}

#[test]
fn a_waiting_sibling_keeps_the_tab_lit_after_another_pane_clears() {
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (window, p1) = srv.new_session(&cwd);
    let p2 = srv.split(&p1, &cwd);
    let mut runner = HookRun::new(&srv, home.path());

    runner.env("MELDR_CC_TIMEOUT", "2");
    runner.in_pane(&p1, 2, "stop", &stop_payload(&cwd, "Done here."));
    runner.env("MELDR_CC_TIMEOUT", "600");
    runner.in_pane(&p2, 2, "stop", &stop_payload(&cwd, "Shall I continue?"));

    assert_eq!(
        srv.cc_status(&window),
        "waiting",
        "waiting outranks done in the same window"
    );

    let p1_cleared = wait_until(Duration::from_secs(20), || {
        srv.cc_pane_status(&p1).is_empty()
    });
    assert!(p1_cleared, "the short-lived pane should expire");
    assert_eq!(
        srv.cc_status(&window),
        "waiting",
        "the tab must stay lit while a sibling is still waiting"
    );
    assert_eq!(srv.cc_pane_status(&p2), "waiting");
}

#[test]
fn clear_now_expires_a_flash_immediately() {
    // What a tmux `after-select-pane` hook needs: you have looked at it.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (window, pane) = srv.new_session(&cwd);
    HookRun::new(&srv, home.path()).in_pane(&pane, 2, "stop", &stop_payload(&cwd, "Done."));
    assert_eq!(srv.cc_status(&window), "done");

    let out = std::process::Command::new(support::tmux_server::meldr_bin())
        .args(["claude-hook", "clear", "--pane", &pane, "--now"])
        .env("HOME", home.path())
        .env("TMUX_TMPDIR", srv.tmpdir())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clear failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert_eq!(srv.cc_pane_status(&pane), "");
    assert_eq!(srv.cc_status(&window), "");
}

// ── background jobs ───────────────────────────────────────────────────────────

#[test]
fn a_detached_job_lights_the_worktree_window_with_a_background_status() {
    // A Claude background job runs under a daemon with no pane ancestry and no
    // `$TMUX`; only its working directory ties it to anything. Several panes share
    // that directory, so the pane is genuinely unknown — the window is the honest
    // answer, marked so it reads differently from the agent in front of you.
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (window, p1) = srv.new_session(&cwd);
    let p2 = srv.split(&p1, &cwd);
    let _p3 = srv.split(&p1, &cwd);
    // A second window elsewhere, which must stay dark.
    let (other_window, _) = srv.new_window("/tmp");

    let out = HookRun::new(&srv, home.path()).detached("stop", &stop_payload(&cwd, "Done."), &cwd);
    assert!(
        out.status.success(),
        "detached hook failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert_eq!(
        srv.cc_status(&window),
        "bg-done",
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(srv.cc_status(&other_window), "");
    for pane in [&p1, &p2] {
        assert_eq!(
            srv.cc_pane_status(pane),
            "",
            "no pane may be blamed when the pane is unknown"
        );
    }
}

#[test]
fn an_unresolvable_event_flashes_nothing_and_says_why() {
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let unrelated = workdir();
    let cwd = unrelated.path().to_string_lossy().to_string();

    let (window, pane) = srv.new_session(&work.path().to_string_lossy());

    let out = HookRun::new(&srv, home.path()).detached("stop", &stop_payload(&cwd, "Done."), &cwd);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(srv.cc_status(&window), "");
    assert_eq!(srv.cc_pane_status(&pane), "");
    assert!(
        stderr.contains("no tmux target"),
        "an unplaceable hook must say so rather than flashing something: {stderr}"
    );
}

// ── classification and filtering ──────────────────────────────────────────────

#[test]
fn a_question_is_classified_as_waiting() {
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (window, pane) = srv.new_session(&cwd);
    HookRun::new(&srv, home.path()).in_pane(
        &pane,
        2,
        "stop",
        &stop_payload(&cwd, "Which branch should I use?"),
    );

    assert_eq!(srv.cc_pane_status(&pane), "waiting");
    assert_eq!(srv.cc_status(&window), "waiting");
}

#[test]
fn only_blocking_notifications_flash() {
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();
    let (window, pane) = srv.new_session(&cwd);
    let runner = HookRun::new(&srv, home.path());

    let notify = |kind: &str| {
        serde_json::json!({
            "hook_event_name": "Notification",
            "session_id": "sess-test",
            "cwd": cwd,
            "notification_type": kind,
        })
        .to_string()
    };

    // Routine chatter: under the old `*` matcher each of these lit a "waiting" tab.
    for quiet in ["auth_success", "agent_completed", "quota_auto_resume_fired"] {
        runner.in_pane(&pane, 1, "notify", &notify(quiet));
        assert_eq!(
            srv.cc_status(&window),
            "",
            "{quiet} must not raise a notification"
        );
    }

    runner.in_pane(&pane, 1, "notify", &notify("permission_prompt"));
    assert_eq!(srv.cc_status(&window), "waiting");
    assert_eq!(srv.cc_pane_status(&pane), "waiting");
}

#[test]
fn subagent_stops_are_ignored() {
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();
    let (window, pane) = srv.new_session(&cwd);

    HookRun::new(&srv, home.path()).in_pane(
        &pane,
        1,
        "stop",
        &serde_json::json!({
            "hook_event_name": "SubagentStop",
            "agent_id": "agent-1",
            "session_id": "sess-test",
            "cwd": cwd,
            "last_assistant_message": "Subagent done.",
        })
        .to_string(),
    );

    assert_eq!(srv.cc_status(&window), "");
    assert_eq!(srv.cc_pane_status(&pane), "");
}

// ── self-test ─────────────────────────────────────────────────────────────────

#[test]
fn selftest_reports_the_pane_and_window_it_is_running_in() {
    let srv = TmuxServer::new();
    let home = home();
    let work = workdir();
    let cwd = work.path().to_string_lossy().to_string();

    let (window, pane) = srv.new_session(&cwd);
    // Background window active, to catch anything that reads "current" instead of
    // "mine".
    srv.new_window("/tmp");

    let out = HookRun::new(&srv, home.path()).in_pane(&pane, 2, "selftest", "");
    let json: serde_json::Value = out
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str(l).ok())
        .unwrap_or_else(|| panic!("selftest printed no JSON:\n{out}"));

    assert_eq!(json["pane"], pane, "selftest output: {out}");
    assert_eq!(json["window"], window);
    assert_eq!(json["tier"], "process-tree");
}

#[test]
fn selftest_fails_when_there_is_no_pane_to_find() {
    // An existing but empty TMUX_TMPDIR: tmux answers, but has no panes. Pointing
    // at a *non-existent* directory would not test this — tmux quietly falls back
    // to /tmp and would find the developer's own server.
    let home = home();
    let empty = support::tmux_server::empty_tmux_tmpdir();
    let out = std::process::Command::new(support::tmux_server::meldr_bin())
        .args(["claude-hook", "selftest"])
        .env("HOME", home.path())
        .env("TMUX_TMPDIR", empty.path())
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "selftest must fail when it cannot place itself; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
