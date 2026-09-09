use std::path::Path;

use console::style;

use crate::core::doctor::{ActionKind, run_claude, run_hooks, run_tmux, run_worktrees};
use crate::error::Result;
use crate::git::GitOps;

pub fn claude(git: &dyn GitOps, workspace_root: &Path, apply: bool) -> Result<()> {
    let _ = git; // not used in this section, but consistent signature
    println!("{}", style("== claude ==").bold());
    let report = run_claude(workspace_root, apply)?;

    if report.actions.is_empty() {
        println!("  {}", style("no issues found").dim());
    } else {
        let tag = if apply {
            style("[apply]").green().to_string()
        } else {
            style("[dry-run]").yellow().to_string()
        };
        for action in &report.actions {
            println!("  {tag} {}", action.description);
        }
        let applied = report.applied;
        let total = report.actions.len();
        if apply {
            println!(
                "  {}",
                style(format!("applied {applied}/{total} actions")).green()
            );
        } else {
            println!(
                "  {}",
                style(format!(
                    "{total} action(s) pending — run with --apply to fix"
                ))
                .dim()
            );
        }
    }
    for w in &report.warnings {
        eprintln!("  {}: {w}", style("warning").yellow());
    }
    Ok(())
}

pub fn worktrees(git: &dyn GitOps, workspace_root: &Path, apply: bool) -> Result<()> {
    println!("{}", style("== worktrees ==").bold());
    let report = run_worktrees(git, workspace_root, apply)?;

    let tag = if apply {
        style("[apply]").green().to_string()
    } else {
        style("[dry-run]").yellow().to_string()
    };

    let mut any = false;

    for action in &report.actions {
        any = true;
        match &action.kind {
            ActionKind::PruneState { branch } => {
                println!("  {tag} {}", style(format!("prune state: {branch}")).bold());
            }
            ActionKind::Archive { src: _, dest } if dest == &std::path::PathBuf::new() => {
                // Dirty orphan dir — cannot archive.
                println!("  {} {}", style("[skip]").red(), action.description);
            }
            ActionKind::Archive { .. } => {
                println!("  {tag} {}", action.description);
            }
            ActionKind::GitWorktreePrune { repo } => {
                println!(
                    "  {tag} git worktree prune in {}",
                    repo.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
    }

    if !report.name_mismatches.is_empty() {
        for (key, expected_dir) in &report.name_mismatches {
            any = true;
            println!(
                "  {} state key '{key}' → expected dir '{expected_dir}' is missing \
                 (may be renamed). Fix with: {}",
                style("[mismatch]").yellow(),
                style(format!("meldr worktree remove {key}")).bold()
            );
        }
    }

    if !any {
        println!("  {}", style("no issues found").dim());
    } else {
        let total = report.actions.len();
        let applied = report.applied;
        if apply {
            println!(
                "  {}",
                style(format!("applied {applied}/{total} actions")).green()
            );
        } else {
            println!(
                "  {}",
                style(format!(
                    "{total} action(s) pending — run with --apply to fix"
                ))
                .dim()
            );
        }
    }

    for w in &report.warnings {
        eprintln!("  {}: {w}", style("warning").yellow());
    }
    Ok(())
}

pub fn hooks(apply: bool, env_check: bool) -> Result<()> {
    let home = match std::env::var_os("HOME").map(std::path::PathBuf::from) {
        Some(h) => h,
        None => {
            eprintln!("  {}: HOME not set", style("error").red());
            return Ok(());
        }
    };

    println!("{}", style("== hooks ==").bold());
    let report = run_hooks(&home, apply)?;
    let mut any = false;

    if report.claude_detected {
        if report.claude_hook_missing {
            any = true;
            if apply {
                println!(
                    "  {} Claude hook missing — installed meldr entry in settings.json",
                    style("[apply]").green()
                );
            } else {
                println!(
                    "  {} Claude hook missing — run {} to fix",
                    style("[warn]").yellow(),
                    style("meldr install-hooks").bold()
                );
            }
        }
    } else {
        println!(
            "  {}",
            style("claude not found on PATH — skipping hook check").dim()
        );
    }

    if report.session_start_hook_missing && report.claude_detected {
        any = true;
        if apply {
            println!(
                "  {} SessionStart hook missing — installed entry in settings.json",
                style("[apply]").green()
            );
        } else {
            println!(
                "  {} SessionStart hook missing (tab-flash won't work for claude agents) — run {} to fix",
                style("[warn]").yellow(),
                style("meldr install-hooks").bold()
            );
        }
    }

    if report.legacy_notify_script_present {
        any = true;
        println!(
            "  {} ~/.local/share/meldr/meldr-agent-notify.sh still present from an old meldr version — safe to delete",
            style("[warn]").yellow()
        );
    }

    if report.legacy_session_start_symlink_present {
        any = true;
        println!(
            "  {} ~/.claude/claude-session-start.sh still present from a previous setup (fmcevoy_tools) — meldr now owns the SessionStart hook via 'meldr claude-hook session-start'; delete that file",
            style("[warn]").yellow()
        );
    }

    if let Some(st) = &report.resolver_selftest {
        if st.skipped {
            println!(
                "  {} resolver self-test skipped (not in tmux)",
                style("[info]").dim()
            );
        } else if let Some(err) = &st.error {
            any = true;
            println!(
                "  {} resolver self-test error: {err}",
                style("[warn]").yellow()
            );
        } else {
            if st.pane_match && st.window_match {
                println!(
                    "  {} resolver self-test passed — pane {}, window {} (via {})",
                    style("[ok]").green(),
                    st.reported_pane,
                    st.reported_window,
                    st.tier
                );
            } else {
                any = true;
                if !st.pane_match {
                    println!(
                        "  {} resolver self-test: pane MISMATCH — reported {:?}, expected {:?} (via {})",
                        style("[warn]").yellow(),
                        st.reported_pane,
                        st.expected_pane,
                        st.tier
                    );
                }
                if !st.window_match {
                    println!(
                        "  {} resolver self-test: window MISMATCH — reported {:?}, expected {:?}; notifications will light the wrong tab",
                        style("[warn]").yellow(),
                        st.reported_window,
                        st.expected_window
                    );
                }
            }

            if st.legacy_sidecars > 0 {
                any = true;
                println!(
                    "  {} {} legacy *.parent_pane sidecar(s) in ~/.cache/claude-agents — unstamped and unverifiable; run 'meldr doctor hooks --apply' or delete them",
                    style("[warn]").yellow(),
                    st.legacy_sidecars
                );
            }
            if st.stale_sidecars > 0 {
                println!(
                    "  {} {} pane sidecar(s) from an older tmux server — ignored, and swept on next SessionStart",
                    style("[info]").dim(),
                    st.stale_sidecars
                );
            }
        }
    }

    if report.tmux_conf_missing_cc_status {
        any = true;
        println!(
            "  {} ~/.tmux.conf does not reference @cc_status — tab-flash will not work",
            style("[warn]").yellow()
        );
        println!("  Add to ~/.tmux.conf:");
        println!(
            "    set -g window-status-format \" #I:#W#{{?#{{==:#{{@cc_status}},done}},#[bg=#f7768e fg=#1a1b26 bold]  ✓ ,#{{?#{{==:#{{@cc_status}},waiting}},#[bg=#e0af68 fg=#1a1b26 bold]  ⏳ ,}}}} \""
        );
        println!(
            "    set -g window-status-current-format \" #I:#W#{{?#{{==:#{{@cc_status}},done}},#[bg=#f7768e fg=#1a1b26 bold]  ✓ ,#{{?#{{==:#{{@cc_status}},waiting}},#[bg=#e0af68 fg=#1a1b26 bold]  ⏳ ,}}}} \""
        );
    }

    if report.tmux_conf_missing_pane_focus_clear {
        any = true;
        println!(
            "  {} ~/.tmux.conf does not clear @cc_pane_status on focus — pane border will stay coloured until timer expires",
            style("[warn]").yellow()
        );
        println!("  Add to ~/.tmux.conf (replace existing after-select-* hooks if present):");
        println!(
            "    set-hook -g after-select-pane   'run-shell -b \"meldr claude-hook clear --pane #{{pane_id}} --now\"'"
        );
        println!(
            "    set-hook -g after-select-window 'run-shell -b \"meldr claude-hook clear --window #{{window_id}} --all-panes --now\"'"
        );
        println!("  (the older 'set-option -wu @cc_status' form still works, but clears the whole");
        println!(
            "   window when you merely switch panes, hiding a sibling agent that is still waiting)"
        );
    }

    if !any && report.claude_detected {
        println!("  {}", style("no issues found").dim());
    }

    for w in &report.warnings {
        eprintln!("  {}: {w}", style("warning").yellow());
    }

    if env_check {
        println!("{}", style("-- env-check --").bold());
        if std::env::var("TMUX").is_err() {
            println!("  not in a tmux session, skipping env-check");
        } else {
            for key in ["TMUX", "TMUX_PANE", "CLAUDE_CODE_CHILD_SESSION"] {
                match std::env::var(key) {
                    Ok(v) => println!("  {} {key}={v}", style("[ok]").green()),
                    Err(_) => println!("  {} {key} not set", style("[info]").dim()),
                }
            }
            // These are no longer consulted. A leftover value is harmless now, but
            // it means a shell wrapper is still exporting it.
            for key in [
                "MELDR_TMUX_PANE",
                "MELDR_TMUX_WINDOW_ID",
                "MELDR_AGENT_SESSION",
            ] {
                if let Ok(v) = std::env::var(key) {
                    println!(
                        "  {} {key}={v} — obsolete and ignored; remove the claude() wrapper that sets it",
                        style("[warn]").yellow()
                    );
                }
            }
        }
    }

    Ok(())
}

pub fn tmux_windows(workspace_root: &Path, apply: bool) -> Result<()> {
    println!("{}", style("== tmux ==").bold());
    let report = run_tmux(workspace_root, apply)?;

    if report.stale_windows.is_empty()
        && report.stale_status_windows.is_empty()
        && report.healed_state.is_empty()
    {
        println!("  {}", style("no stale windows found").dim());
    } else {
        let tag = if apply {
            style("[apply]").green().to_string()
        } else {
            style("[dry-run]").yellow().to_string()
        };
        for window in &report.stale_windows {
            println!(
                "  {tag} kill stale window '{}' ({}:{})",
                style(&window.name).bold(),
                window.session,
                window.index
            );
        }
        for wid in &report.stale_status_windows {
            println!(
                "  {tag} clear stale @cc_status on window {}",
                style(wid).bold()
            );
        }
        for branch in &report.healed_state {
            println!(
                "  {tag} heal state tmux_window for '{}' → @id",
                style(branch).bold()
            );
        }
        let total = report.stale_windows.len()
            + report.stale_status_windows.len()
            + report.healed_state.len();
        let applied = report.applied;
        if apply {
            println!(
                "  {}",
                style(format!("applied {applied}/{total} actions")).green()
            );
        } else {
            println!(
                "  {}",
                style(format!(
                    "{total} action(s) pending — run with --apply to fix"
                ))
                .dim()
            );
        }
    }

    for w in &report.warnings {
        eprintln!("  {}: {w}", style("warning").yellow());
    }
    Ok(())
}
