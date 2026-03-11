use std::path::Path;

use console::style;

use crate::core::config::EffectiveConfig;
use crate::core::state::WorkspaceState;
use crate::core::workspace::{self, Manifest};
use crate::core::worktree::{PackageSyncOutcome, SyncStatus};
use crate::core::{sync_history, worktree};
use crate::error;
use crate::git::GitOps;

#[allow(clippy::too_many_arguments)]
pub fn run(
    git: &dyn GitOps,
    root: &Path,
    cwd: &Path,
    config: &EffectiveConfig,
    branch: Option<String>,
    all: bool,
    strategy: Option<String>,
    merge: bool,
    dry_run: bool,
    only: Vec<String>,
    exclude: Vec<String>,
    undo: bool,
) -> error::Result<()> {
    let manifest = Manifest::load(root)?;

    // Handle --undo
    if undo {
        let target_branch = resolve_sync_branch(root, branch.as_deref(), cwd)?;
        let snapshot = sync_history::load_latest_snapshot(root, &target_branch)?
            .ok_or_else(|| error::MeldrError::NoSyncSnapshot(target_branch.clone()))?;

        println!(
            "Undoing sync for '{}' (restoring to snapshot from {})",
            target_branch, snapshot.timestamp
        );
        let results = worktree::undo_sync(git, root, &target_branch, &snapshot)?;
        for (pkg, result) in &results {
            match result {
                Ok(()) => println!(
                    "  {} reset to {}",
                    pkg,
                    &snapshot.packages[pkg][..8.min(snapshot.packages[pkg].len())]
                ),
                Err(e) => eprintln!("  {pkg} failed: {e}"),
            }
        }
        return Ok(());
    }

    let sync_options = worktree::SyncOptions {
        method_override: if merge {
            Some("merge".to_string())
        } else {
            None
        },
        strategy_override: strategy,
        dry_run,
        only,
        exclude,
    };

    if dry_run {
        println!("Dry run — no changes will be made.\n");
    }

    let branches_to_sync: Vec<String> = if all {
        let state = WorkspaceState::load(root)?;
        state.worktrees.keys().cloned().collect()
    } else {
        let target = resolve_sync_branch(root, branch.as_deref(), cwd)?;
        vec![target]
    };

    // When no worktrees exist, still fetch all packages
    if branches_to_sync.is_empty() {
        eprintln!("No active worktrees. Fetching all packages...\n");
        for pkg in &manifest.packages {
            let repo_path = workspace::package_path(root, &pkg.name);
            let remote = pkg.remote.as_deref().unwrap_or(&config.remote);
            eprint!("  Fetching {} ... ", pkg.name);
            match git.fetch(&repo_path, remote) {
                Ok(()) => eprintln!("done"),
                Err(e) => eprintln!("failed: {e}"),
            }
        }
        return Ok(());
    }

    for branch_name in &branches_to_sync {
        if branches_to_sync.len() > 1 {
            println!("--- Worktree '{branch_name}' ---");
        }

        // Save pre-sync snapshot (unless dry run)
        if !dry_run {
            let mut pkg_heads = std::collections::HashMap::new();
            for pkg in &manifest.packages {
                let wt_path = workspace::worktree_path(root, branch_name, &pkg.name);
                if wt_path.exists()
                    && let Ok(sha) = git.current_head(&wt_path)
                {
                    pkg_heads.insert(pkg.name.clone(), sha);
                }
            }
            if !pkg_heads.is_empty() {
                let snapshot = sync_history::SyncSnapshot {
                    timestamp: sync_history::unix_timestamp(),
                    branch: branch_name.clone(),
                    packages: pkg_heads,
                };
                let _ = sync_history::save_snapshot(root, &snapshot);
                let _ = sync_history::prune_snapshots(root, 10);
            }
        }

        let outcomes =
            worktree::sync_worktree(git, &manifest, root, branch_name, config, &sync_options)?;

        // Log the sync
        if !dry_run {
            let log_entry = sync_history::SyncLogEntry {
                timestamp: sync_history::unix_timestamp(),
                branch: branch_name.clone(),
                outcomes: outcomes
                    .iter()
                    .map(|o| sync_history::PackageSyncLogEntry {
                        package: o.package.clone(),
                        status: o.status.to_string(),
                        method: o.method.clone(),
                        ahead: o.ahead,
                        behind: o.behind,
                    })
                    .collect(),
            };
            let _ = sync_history::append_log(root, &log_entry);
        }

        // Print summary table
        print_sync_summary(&outcomes, dry_run);
    }

    Ok(())
}

fn resolve_sync_branch(root: &Path, branch: Option<&str>, cwd: &Path) -> error::Result<String> {
    if let Some(b) = branch {
        return Ok(b.to_string());
    }
    let dir_name = workspace::detect_current_worktree_dir(root, cwd);
    let state = WorkspaceState::load(root)?;
    dir_name
        .and_then(|d| {
            workspace::resolve_branch_from_dir(&d, state.worktrees.keys().map(|s| s.as_str()))
        })
        .ok_or_else(|| {
            error::MeldrError::Config(
                "Could not detect current worktree. Specify a branch or use --all.".to_string(),
            )
        })
}

fn print_sync_summary(outcomes: &[PackageSyncOutcome], dry_run: bool) {
    let label = if dry_run { "Would" } else { "Sync" };

    println!();
    println!(
        "  {:<20} {:<16} {:>6} {:>7}  {}",
        style("Package").bold(),
        style("Status").bold(),
        style("Ahead").bold(),
        style("Behind").bold(),
        style("Method").bold(),
    );
    println!("  {}", "-".repeat(66));

    for o in outcomes {
        let status_str = match &o.status {
            SyncStatus::Synced => style("synced".to_string()).green().to_string(),
            SyncStatus::UpToDate => style("up-to-date".to_string()).green().to_string(),
            SyncStatus::Skipped(r) => style(r.to_string()).yellow().to_string(),
            SyncStatus::Conflict(files) => style(format!("conflict ({})", files.len()))
                .red()
                .to_string(),
            SyncStatus::Failed(msg) => {
                let short = if msg.len() > 30 {
                    format!("{}...", &msg[..27])
                } else {
                    msg.clone()
                };
                style(short).red().to_string()
            }
        };

        let ahead = o.ahead.map_or("-".to_string(), |a| a.to_string());
        let behind = o.behind.map_or("-".to_string(), |b| b.to_string());

        println!(
            "  {:<20} {:<16} {:>6} {:>7}  {}",
            o.package, status_str, ahead, behind, o.method,
        );
    }

    // Print conflict details
    let conflicts: Vec<_> = outcomes
        .iter()
        .filter(|o| matches!(&o.status, SyncStatus::Conflict(_)))
        .collect();
    if !conflicts.is_empty() {
        println!();
        for o in conflicts {
            if let SyncStatus::Conflict(files) = &o.status {
                eprintln!(
                    "  {} has conflicts in: {}",
                    style(&o.package).red().bold(),
                    files.join(", ")
                );
            }
        }
        eprintln!();
        eprintln!(
            "  {}",
            style("Use --strategy theirs to auto-resolve in favor of upstream,").dim()
        );
        eprintln!(
            "  {}",
            style("or --strategy manual to attempt merge and resolve manually.").dim()
        );
    }

    println!();
    println!(
        "  {} summary: {} synced, {} up-to-date, {} conflicts, {} skipped, {} failed",
        label,
        outcomes
            .iter()
            .filter(|o| o.status == SyncStatus::Synced)
            .count(),
        outcomes
            .iter()
            .filter(|o| o.status == SyncStatus::UpToDate)
            .count(),
        outcomes
            .iter()
            .filter(|o| matches!(o.status, SyncStatus::Conflict(_)))
            .count(),
        outcomes
            .iter()
            .filter(|o| matches!(o.status, SyncStatus::Skipped(_)))
            .count(),
        outcomes
            .iter()
            .filter(|o| matches!(o.status, SyncStatus::Failed(_)))
            .count(),
    );
}
