mod cli;
mod core;
mod error;
mod git;
mod tmux;
mod trace;

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
        eprintln!("Error: {e}");
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
            let mut config = config::EffectiveConfig {
                no_agent: cli_overrides.no_agent,
                no_tabs: cli_overrides.no_tabs,
                ..Default::default()
            };
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
                                        "Could not detect current worktree. Specify a branch name."
                                            .to_string(),
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
            let (config, _) = build_effective_config(&root, &cli_overrides)?;
            cli::sync::run(
                &git, &root, &cwd, &config, branch, all, strategy, merge, dry_run, only, exclude,
                undo,
            )
        }

        Commands::PromptCheck => {
            let cwd = std::env::current_dir()?;
            if let Ok(root) = workspace::find_workspace_root(&cwd) {
                cli::prompt_check::run(&root, &cwd);
            }
            Ok(())
        }

        Commands::PromptCheck => {
            let cwd = std::env::current_dir()?;
            if let Ok(root) = workspace::find_workspace_root(&cwd) {
                cli::prompt_check::run(&root, &cwd);
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
                ConfigAction::Show => cli::config_cmd::show(workspace_root.as_deref()),
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
    eprintln!("{}", style("Run 'meldr sync --all' to update.\n").dim());
}

fn build_effective_config(
    workspace_root: &Path,
    cli_overrides: &CliOverrides,
) -> error::Result<(config::EffectiveConfig, GlobalConfig)> {
    let global = config::load_global_config()?;
    let manifest = Manifest::load(workspace_root)?;

    let env_overrides = config::collect_env_overrides();

    let effective =
        config::resolve_config(&global, &manifest.settings, cli_overrides, &env_overrides);
    Ok((effective, global))
}
