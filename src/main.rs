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
use git::RealGit;
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
            match action {
                PackageAction::Add { urls } => cli::package::add(&git, &root, &urls),
                PackageAction::Remove { names } => cli::package::remove(&root, &names),
                PackageAction::List => cli::package::list(&root),
            }
        }

        Commands::Worktree { action } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            match action {
                WorktreeAction::Add { branch } => {
                    let (config, global) = build_effective_config(&root, &cli_overrides)?;
                    cli::worktree::add(&git, &tmux, &root, &branch, &config, Some(&global))
                }
                WorktreeAction::Remove { branch, force } => {
                    cli::worktree::remove(&git, &tmux, &root, &branch, force)
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
            cli::status::run(&git, &root)
        }

        Commands::Exec { command } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            let (config, _) = build_effective_config(&root, &cli_overrides)?;
            cli::exec::run(&root, &command, &config)
        }

        Commands::Sync {
            branch,
            all,
            strategy,
            merge,
        } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            let manifest = Manifest::load(&root)?;
            let (config, _) = build_effective_config(&root, &cli_overrides)?;

            let method_override = if merge { Some("merge") } else { None };
            let strat_override = strategy.as_deref();

            if all {
                let state = core::state::WorkspaceState::load(&root)?;
                for branch_name in state.worktrees.keys() {
                    println!("Syncing worktree '{}'...", branch_name);
                    core::worktree::sync_worktree(
                        &git,
                        &manifest,
                        &root,
                        branch_name,
                        &config,
                        method_override,
                        strat_override,
                    )?;
                }
            } else {
                let target_branch =
                    branch.or_else(|| workspace::detect_current_worktree(&root, &cwd));
                match target_branch {
                    Some(b) => {
                        core::worktree::sync_worktree(
                            &git, &manifest, &root, &b, &config,
                            method_override, strat_override,
                        )?;
                        println!("Synced worktree '{}'", b);
                    }
                    None => {
                        eprintln!(
                            "Could not detect current worktree. Specify a branch or use --all."
                        );
                        std::process::exit(1);
                    }
                }
            }
            Ok(())
        }

        Commands::Config { action } => {
            let cwd = std::env::current_dir()?;
            let root = workspace::find_workspace_root(&cwd)?;
            match action {
                ConfigAction::Set { key, value } => cli::config_cmd::set(&root, &key, &value),
                ConfigAction::Get { key } => cli::config_cmd::get(&root, &key),
                ConfigAction::List => cli::config_cmd::list(&root),
            }
        }
    }
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
