use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;

use crate::core::config::EffectiveConfig;
use crate::core::state::{WorkspaceState, WorktreeState};
use crate::core::workspace::{self, Manifest};
use crate::error::{MeldrError, Result};
use crate::git::GitOps;
use crate::tmux::TmuxOps;

pub fn add_worktree(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
) -> Result<()> {
    if state.get_worktree(branch).is_some() {
        return Err(MeldrError::WorktreeAlreadyExists(branch.to_string()));
    }

    let needs_tmux = config.should_use_tmux();

    if needs_tmux && !tmux.is_inside_tmux() {
        return Err(MeldrError::NotInTmux);
    }

    let branch_dir = workspace::worktrees_dir(workspace_root).join(branch);
    std::fs::create_dir_all(&branch_dir)?;

    let results: Vec<_> = manifest
        .packages
        .par_iter()
        .map(|pkg| {
            let repo_path = workspace::package_path(workspace_root, &pkg.name);
            let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
            match git.worktree_add(&repo_path, &wt_path, branch) {
                Ok(()) => Ok(pkg.name.clone()),
                Err(e) => Err((pkg.name.clone(), e)),
            }
        })
        .collect();

    let mut created = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(name) => created.push(name),
            Err((name, e)) => errors.push(format!("{}: {}", name, e)),
        }
    }

    if !errors.is_empty() && created.is_empty() {
        return Err(MeldrError::Git(format!(
            "Failed to create any worktrees: {}",
            errors.join(", ")
        )));
    }

    for error in &errors {
        eprintln!("Warning: {}", error);
    }

    let ws_name = &manifest.workspace.name;
    let mut tmux_windows = Vec::new();
    let mut pane_mappings = HashMap::new();

    if needs_tmux {
        // Check for layout override in manifest
        if let Some(ref lo) = manifest.layout {
            let window_name = format!("{}/{}", ws_name, branch);
            let window_id = tmux.create_window(&window_name)?;

            let pane_count = lo.panes.len();
            for _ in 1..pane_count {
                tmux.split_window(&window_id)?;
            }

            let layout = crate::tmux::TmuxLayout {
                definition: lo.definition.clone(),
                pane_names: lo.panes.clone(),
            };
            tmux.apply_layout(&window_id, &layout)?;

            for (i, pkg_name) in lo.panes.iter().enumerate() {
                if pkg_name.is_empty() {
                    continue;
                }
                let wt_path = workspace::worktree_path(workspace_root, branch, pkg_name);
                let target = format!("{}.{}", window_id, i);
                tmux.send_keys(&target, &format!("cd {}", wt_path.display()))?;
                if config.should_launch_agent() {
                    tmux.send_keys(&target, &config.agent_command)?;
                }
                pane_mappings.insert(i.to_string(), pkg_name.clone());
            }

            tmux_windows.push(window_id);
        } else {
            // Default: one dev window per package with nvim + agent + 4 terminals
            for pkg_name in &created {
                let wt_path = workspace::worktree_path(workspace_root, branch, pkg_name);
                let wt_path_str = wt_path.to_string_lossy().to_string();

                let window_name = if created.len() == 1 {
                    format!("{}/{}", ws_name, branch)
                } else {
                    format!("{}/{}:{}", ws_name, branch, pkg_name)
                };

                let dev = tmux.create_dev_window(&window_name, &wt_path_str)?;

                // Launch nvim in pane 0
                tmux.send_keys(&dev.nvim, "nvim .")?;

                // Launch agent in the agent pane (top-right)
                if config.should_launch_agent() {
                    tmux.send_keys(&dev.agent, &config.agent_command)?;
                }

                pane_mappings.insert(
                    format!("{}:nvim", pkg_name),
                    dev.nvim.clone(),
                );
                pane_mappings.insert(
                    format!("{}:agent", pkg_name),
                    dev.agent.clone(),
                );

                tmux_windows.push(dev.window_id);
            }
        }
    }

    let tmux_window = if tmux_windows.len() == 1 {
        Some(tmux_windows.into_iter().next().unwrap())
    } else if tmux_windows.is_empty() {
        None
    } else {
        // Store comma-separated window IDs for multi-package worktrees
        Some(tmux_windows.join(","))
    };

    state.add_worktree(
        branch,
        WorktreeState {
            branch: branch.to_string(),
            tmux_window,
            pane_mappings,
        },
    );
    state.save(workspace_root)?;

    Ok(())
}

pub fn remove_worktree(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    force: bool,
) -> Result<()> {
    if state.get_worktree(branch).is_none() {
        return Err(MeldrError::WorktreeNotFound(branch.to_string()));
    }

    if !force {
        for pkg in &manifest.packages {
            let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
            if wt_path.exists() {
                if let Ok(true) = git.is_dirty(&wt_path) {
                    return Err(MeldrError::DirtyWorktree(
                        branch.to_string(),
                        pkg.name.clone(),
                    ));
                }
            }
        }
    }

    if let Some(wt_state) = state.get_worktree(branch) {
        if let Some(ref window_ids) = wt_state.tmux_window {
            // Handle comma-separated window IDs for multi-package worktrees
            for window_id in window_ids.split(',') {
                if let Err(e) = tmux.kill_window(window_id) {
                    eprintln!("Warning: Could not kill tmux window '{}': {}", window_id, e);
                }
            }
        }
    }

    for pkg in &manifest.packages {
        let repo_path = workspace::package_path(workspace_root, &pkg.name);
        let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
        if wt_path.exists() {
            if let Err(e) = git.worktree_remove(&repo_path, &wt_path, force) {
                eprintln!("Warning: Failed to remove worktree for '{}': {}", pkg.name, e);
            }
        }
    }

    let branch_dir = workspace::worktrees_dir(workspace_root).join(branch);
    if branch_dir.exists() {
        let _ = std::fs::remove_dir_all(&branch_dir);
    }

    state.remove_worktree(branch);
    state.save(workspace_root)?;

    Ok(())
}

pub fn list_worktrees(state: &WorkspaceState) -> Vec<&WorktreeState> {
    state.worktrees.values().collect()
}

pub fn sync_worktree(
    git: &dyn GitOps,
    manifest: &Manifest,
    workspace_root: &Path,
    branch: &str,
    method: &str,
    strategy: &str,
) -> Result<()> {
    for pkg in &manifest.packages {
        let repo_path = workspace::package_path(workspace_root, &pkg.name);
        let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);

        if !wt_path.exists() {
            eprintln!(
                "Warning: Worktree for '{}' on branch '{}' does not exist, skipping",
                pkg.name, branch
            );
            continue;
        }

        println!("Syncing {}...", pkg.name);
        git.fetch(&repo_path)?;
        if let Err(e) = git.pull_ff_only(&repo_path) {
            eprintln!("Warning: Could not fast-forward '{}' main: {}", pkg.name, e);
        }

        let default_branch = pkg.branch.as_deref().unwrap_or("main");
        if method == "merge" {
            git.merge(&wt_path, default_branch, strategy)?;
        } else {
            git.rebase(&wt_path, default_branch, strategy, true)?;
        }
    }
    Ok(())
}
