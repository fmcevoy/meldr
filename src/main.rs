mod cli;
mod core;
mod error;
mod git;
mod tmux;
mod trace;

use std::collections::HashMap;
use std::path::Path;

use clap::Parser;

use cli::{Cli, Commands, ConfigAction, PackageAction, WorktreeAction};
use core::config::{self, CliOverrides, GlobalConfig};
use core::workspace::{self, Manifest};
use git::{GitOps, RealGit};
use tmux::RealTmux;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> error::Result<()> {
    let git = RealGit::new();
    let tmux = RealTmux::new();

    let cli_overrides = CliOverrides {
        no_agent: cli.no_agent,
        no_tabs: cli.no_tabs,
    };

    match cli.command {
        Commands::Init { name } => {
            let cwd = std::env::current_dir()?;
            cli::init::run(&cwd, name.as_deref())
        }

        Commands::Create {
            name,
            repos,
            branch,
            agent,
        } => {
            let cwd = std::env::current_dir()?;
            let global = config::load_global_config()?;
            let mut config = config::EffectiveConfig::default();
            config.no_agent = cli_overrides.no_agent;
            config.no_tabs = cli_overrides.no_tabs;
            if let Some(ref a) = agent {
                config.agent = a.clone();
                config.agent_command = a.clone();
            }
            cli::create::run(
                &git,
                &tmux,
                &cwd,
                &name,
                &repos,
                branch.as_deref(),
                agent.as_deref(),
                &config,
                Some(&global),
            )
        }

        Commands::Package { action } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            let (config, _) = build_effective_config(&root, &cli_overrides)?;
            warn_if_out_of_sync(&git, &root, &config);
            match action {
                PackageAction::Add { urls } => cli::package::add(&git, &root, &urls),
                PackageAction::Remove { names } => cli::package::remove(&root, &names),
                PackageAction::List => cli::package::list(&root),
            }
        }

        Commands::Worktree { action } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            let (wt_config, _) = build_effective_config(&root, &cli_overrides)?;
            warn_if_out_of_sync(&git, &root, &wt_config);
            match action {
                WorktreeAction::Add { branch } => {
                    let (config, global) = build_effective_config(&root, &cli_overrides)?;
                    cli::worktree::add(&git, &tmux, &root, &branch, &config, Some(&global))
                }
                WorktreeAction::Remove { branch, force } => {
                    let target = match branch {
                        Some(b) => b,
                        None => {
                            let state = core::state::WorkspaceState::load(&root)?;
                            let dir_name = workspace::detect_current_worktree_dir(&root, &cwd);
                            dir_name
                                .and_then(|d| {
                                    workspace::resolve_branch_from_dir(
                                        &d,
                                        state.worktrees.keys().map(|s| s.as_str()),
                                    )
                                })
                                .ok_or_else(|| {
                                    error::MeldrError::Config(
                                        "Could not detect current worktree. Specify a branch name.".to_string(),
                                    )
                                })?
                        }
                    };
                    cli::worktree::remove(&git, &tmux, &root, &target, force)
                }
                WorktreeAction::Open { branch } => {
                    let (config, global) = build_effective_config(&root, &cli_overrides)?;
                    cli::worktree::open(&tmux, &root, &branch, &config, Some(&global))
                }
                WorktreeAction::List => cli::worktree::list(&root),
            }
        }

        Commands::Status => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            let (config, _) = build_effective_config(&root, &cli_overrides)?;
            warn_if_out_of_sync(&git, &root, &config);
            cli::status::run(&git, &root)
        }

        Commands::Exec {
            interactive,
            command,
        } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            let (config, _) = build_effective_config(&root, &cli_overrides)?;
            warn_if_out_of_sync(&git, &root, &config);
            cli::exec::run(&root, &cwd, &command, &config, interactive)
        }

        Commands::Sync {
            branch,
            all,
            strategy,
            merge,
            dry_run,
            only,
            exclude,
            undo,
        } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            let manifest = Manifest::load(&root)?;
            let (config, _) = build_effective_config(&root, &cli_overrides)?;

            // Handle --undo
            if undo {
                let target_branch = resolve_sync_branch(&root, branch.as_deref(), &cwd)?;
                let snapshot =
                    core::sync_history::load_latest_snapshot(&root, &target_branch)?
                        .ok_or_else(|| {
                            error::MeldrError::NoSyncSnapshot(target_branch.clone())
                        })?;

                println!(
                    "Undoing sync for '{}' (restoring to snapshot from {})",
                    target_branch, snapshot.timestamp
                );
                let results =
                    core::worktree::undo_sync(&git, &root, &target_branch, &snapshot)?;
                for (pkg, result) in &results {
                    match result {
                        Ok(()) => println!("  {} reset to {}", pkg, &snapshot.packages[pkg][..8.min(snapshot.packages[pkg].len())]),
                        Err(e) => eprintln!("  {} failed: {}", pkg, e),
                    }
                }
                return Ok(());
            }

            let sync_options = core::worktree::SyncOptions {
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
                let state = core::state::WorkspaceState::load(&root)?;
                state.worktrees.keys().cloned().collect()
            } else {
                let target = resolve_sync_branch(&root, branch.as_deref(), &cwd)?;
                vec![target]
            };

            // When no worktrees exist, still fetch all packages
            if branches_to_sync.is_empty() {
                println!("No active worktrees. Fetching all packages...\n");
                for pkg in &manifest.packages {
                    let repo_path = workspace::package_path(&root, &pkg.name);
                    let remote = pkg.remote.as_deref().unwrap_or(&config.remote);
                    eprint!("  Fetching {} ... ", pkg.name);
                    match git.fetch(&repo_path, remote) {
                        Ok(()) => eprintln!("done"),
                        Err(e) => eprintln!("failed: {}", e),
                    }
                }
                return Ok(());
            }

            for branch_name in &branches_to_sync {
                if branches_to_sync.len() > 1 {
                    println!("--- Worktree '{}' ---", branch_name);
                }

                // Save pre-sync snapshot (unless dry run)
                if !dry_run {
                    let mut pkg_heads = std::collections::HashMap::new();
                    for pkg in &manifest.packages {
                        let wt_path =
                            workspace::worktree_path(&root, branch_name, &pkg.name);
                        if wt_path.exists() {
                            if let Ok(sha) = git.current_head(&wt_path) {
                                pkg_heads.insert(pkg.name.clone(), sha);
                            }
                        }
                    }
                    if !pkg_heads.is_empty() {
                        let snapshot = core::sync_history::SyncSnapshot {
                            timestamp: core::sync_history::unix_timestamp(),
                            branch: branch_name.clone(),
                            packages: pkg_heads,
                        };
                        let _ = core::sync_history::save_snapshot(&root, &snapshot);
                        let _ = core::sync_history::prune_snapshots(&root, 10);
                    }
                }

                let outcomes = core::worktree::sync_worktree(
                    &git,
                    &manifest,
                    &root,
                    branch_name,
                    &config,
                    &sync_options,
                )?;

                // Log the sync
                if !dry_run {
                    let log_entry = core::sync_history::SyncLogEntry {
                        timestamp: core::sync_history::unix_timestamp(),
                        branch: branch_name.clone(),
                        outcomes: outcomes
                            .iter()
                            .map(|o| core::sync_history::PackageSyncLogEntry {
                                package: o.package.clone(),
                                status: o.status.to_string(),
                                method: o.method.clone(),
                                ahead: o.ahead,
                                behind: o.behind,
                            })
                            .collect(),
                    };
                    let _ = core::sync_history::append_log(&root, &log_entry);
                }

                // Print summary table
                print_sync_summary(&outcomes, dry_run);
            }

            Ok(())
        }

        Commands::Config { action } => {
            let cwd = std::env::current_dir()?;
            let workspace_root = workspace::find_workspace_root(&cwd).ok();
            match action {
                ConfigAction::Set { key, value, global } => {
                    cli::config_cmd::set(workspace_root.as_deref(), &key, &value, global)
                }
                ConfigAction::Get { key, global } => {
                    cli::config_cmd::get(workspace_root.as_deref(), &key, global)
                }
                ConfigAction::Unset { key, global } => {
                    cli::config_cmd::unset(workspace_root.as_deref(), &key, global)
                }
                ConfigAction::List { global } => {
                    cli::config_cmd::list(workspace_root.as_deref(), global)
                }
                ConfigAction::Show => {
                    cli::config_cmd::show(workspace_root.as_deref())
                }
            }
        }
    }
}

fn warn_if_out_of_sync(git: &dyn GitOps, root: &Path, config: &config::EffectiveConfig) {
    let state = match core::state::WorkspaceState::load(root) {
        Ok(s) => s,
        Err(_) => return,
    };
    if state.worktrees.is_empty() {
        return;
    }
    let manifest = match Manifest::load(root) {
        Ok(m) => m,
        Err(_) => return,
    };
    let branches: Vec<String> = state.worktrees.keys().cloned().collect();
    let stale = core::worktree::check_worktree_staleness(git, &manifest, root, &branches, config);
    if stale.is_empty() {
        return;
    }
    use console::style;
    eprintln!(
        "{}",
        style("Warning: some worktrees are behind upstream:").yellow()
    );
    for (branch, pkg, behind) in &stale {
        eprintln!(
            "  {} ({}) is {} commit{} behind",
            style(branch).bold(),
            pkg,
            behind,
            if *behind == 1 { "" } else { "s" }
        );
    }
    eprintln!(
        "{}",
        style("Run 'meldr sync --all' to update.\n").dim()
    );
}

fn resolve_sync_branch(
    root: &Path,
    branch: Option<&str>,
    cwd: &Path,
) -> error::Result<String> {
    if let Some(b) = branch {
        return Ok(b.to_string());
    }
    let dir_name = workspace::detect_current_worktree_dir(root, cwd);
    let state = core::state::WorkspaceState::load(root)?;
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

fn print_sync_summary(outcomes: &[core::worktree::PackageSyncOutcome], dry_run: bool) {
    use console::style;
    use core::worktree::SyncStatus;

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
            SyncStatus::Skipped(r) => style(format!("{}", r)).yellow().to_string(),
            SyncStatus::Conflict(files) => {
                style(format!("conflict ({})", files.len())).red().to_string()
            }
            SyncStatus::Failed(msg) => {
                let short = if msg.len() > 30 {
                    format!("{}...", &msg[..27])
                } else {
                    msg.clone()
                };
                style(format!("{}", short)).red().to_string()
            }
        };

        let ahead = o
            .ahead
            .map(|a| a.to_string())
            .unwrap_or_else(|| "-".to_string());
        let behind = o
            .behind
            .map(|b| b.to_string())
            .unwrap_or_else(|| "-".to_string());

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

fn build_effective_config(
    workspace_root: &Path,
    cli_overrides: &CliOverrides,
) -> error::Result<(config::EffectiveConfig, GlobalConfig)> {
    let global = config::load_global_config()?;
    let manifest = Manifest::load(workspace_root)?;

    let mut env_overrides = HashMap::new();
    for key in &[
        "MELDR_AGENT",
        "MELDR_MODE",
        "MELDR_EDITOR",
        "MELDR_DEFAULT_BRANCH",
        "MELDR_REMOTE",
        "MELDR_SHELL",
        "MELDR_LAYOUT",
        "VISUAL",
        "EDITOR",
        "SHELL",
    ] {
        if let Ok(val) = std::env::var(key) {
            env_overrides.insert(key.to_string(), val);
        }
    }

    let effective = config::resolve_config(
        &global,
        &manifest.settings,
        cli_overrides,
        &env_overrides,
    );
    Ok((effective, global))
}
