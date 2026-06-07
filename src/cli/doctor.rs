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

    if report.script_stale {
        any = true;
        if apply {
            println!(
                "  {} notify script stale — reinstalled ~/.local/share/meldr/meldr-agent-notify.sh",
                style("[apply]").green()
            );
        } else {
            println!(
                "  {} notify script is outdated — run {} to update",
                style("[warn]").yellow(),
                style("meldr install-hooks").bold()
            );
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
            match std::env::var("MELDR_TMUX_PANE") {
                Ok(pane) => println!("  {} MELDR_TMUX_PANE={}", style("[ok]").green(), pane),
                Err(_) => println!(
                    "  {} MELDR_TMUX_PANE not set — env injection (M2) may not be active",
                    style("[warn]").yellow()
                ),
            }
            match std::env::var("MELDR_AGENT_SESSION") {
                Ok(sess) => println!("  {} MELDR_AGENT_SESSION={}", style("[ok]").green(), sess),
                Err(_) => println!("  {} MELDR_AGENT_SESSION not set", style("[warn]").yellow()),
            }
        }
    }

    Ok(())
}

pub fn tmux_windows(workspace_root: &Path, apply: bool) -> Result<()> {
    println!("{}", style("== tmux ==").bold());
    let report = run_tmux(workspace_root, apply)?;

    if report.stale_windows.is_empty() && report.stale_status_windows.is_empty() {
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
        let total = report.stale_windows.len() + report.stale_status_windows.len();
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
