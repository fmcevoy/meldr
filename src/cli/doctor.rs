use std::path::Path;

use console::style;

use crate::core::doctor::{ActionKind, run_claude, run_tmux, run_worktrees};
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

pub fn tmux_windows(workspace_root: &Path, apply: bool) -> Result<()> {
    println!("{}", style("== tmux ==").bold());
    let report = run_tmux(workspace_root, apply)?;

    if report.stale_windows.is_empty() {
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
        let total = report.stale_windows.len();
        let applied = report.applied;
        if apply {
            println!(
                "  {}",
                style(format!("killed {applied}/{total} stale windows")).green()
            );
        } else {
            println!(
                "  {}",
                style(format!(
                    "{total} stale window(s) — run with --apply to kill"
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
