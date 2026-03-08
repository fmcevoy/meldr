use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;

use crate::core::config::{EffectiveConfig, GlobalConfig};
use crate::core::state::{WorkspaceState, WorktreeState};
use crate::core::workspace::{self, Manifest};
use crate::error::{MeldrError, Result};
use crate::git::GitOps;
use crate::tmux::TmuxOps;

/// Expand template variables in a string.
///
/// Replaces `{ws}`, `{branch}`, and `{pkg}` with their respective values.
pub fn expand_template(template: &str, ws: &str, branch: &str, pkg: &str) -> String {
    template
        .replace("{ws}", ws)
        .replace("{branch}", branch)
        .replace("{pkg}", pkg)
}

/// Result of setting up tmux windows for a worktree.
struct TmuxSetupResult {
    tmux_window: Option<String>,
    pane_mappings: HashMap<String, String>,
}

/// Create tmux windows and panes for a set of packages in a worktree branch.
///
/// Handles both manifest layout overrides and per-package dev window layouts.
/// Skips packages whose worktree path does not exist on disk.
fn setup_tmux_windows(
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    workspace_root: &Path,
    branch: &str,
    pkg_names: &[String],
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
) -> Result<TmuxSetupResult> {
    let ws_name = &manifest.workspace.name;
    let mut tmux_windows = Vec::new();
    let mut pane_mappings = HashMap::new();

    if let Some(ref lo) = manifest.layout {
        let window_name = expand_template(&config.window_name_template, ws_name, branch, "");
        let window_id = tmux.create_window(&window_name)?;

        for _ in 1..lo.panes.len() {
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
        let custom_layout = global_config.and_then(|gc| gc.layouts.get(&config.layout));

        for pkg_name in pkg_names {
            let wt_path = workspace::worktree_path(workspace_root, branch, pkg_name);
            if !wt_path.exists() {
                eprintln!("Warning: worktree path for '{}' does not exist, skipping", pkg_name);
                continue;
            }
            let wt_path_str = wt_path.to_string_lossy().to_string();

            let window_name =
                expand_template(&config.window_name_template, ws_name, branch, pkg_name);

            let dev =
                tmux.create_dev_window(&window_name, &wt_path_str, config, custom_layout)?;

            if let Some(ref editor_pane) = dev.editor {
                tmux.send_keys(editor_pane, &config.editor)?;
                pane_mappings.insert(format!("{}:editor", pkg_name), editor_pane.clone());
            }

            if config.should_launch_agent() {
                if let Some(ref agent_pane) = dev.agent {
                    tmux.send_keys(agent_pane, &config.agent_command)?;
                }
            }
            if let Some(ref agent_pane) = dev.agent {
                pane_mappings.insert(format!("{}:agent", pkg_name), agent_pane.clone());
            }

            tmux_windows.push(dev.window_id);
        }
    }

    let tmux_window = match tmux_windows.len() {
        0 => None,
        1 => Some(tmux_windows.into_iter().next().unwrap()),
        _ => Some(tmux_windows.join(",")),
    };

    Ok(TmuxSetupResult {
        tmux_window,
        pane_mappings,
    })
}

pub fn add_worktree(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
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

    let setup = if needs_tmux {
        setup_tmux_windows(tmux, manifest, workspace_root, branch, &created, config, global_config)?
    } else {
        TmuxSetupResult {
            tmux_window: None,
            pane_mappings: HashMap::new(),
        }
    };

    state.add_worktree(
        branch,
        WorktreeState {
            branch: branch.to_string(),
            tmux_window: setup.tmux_window,
            pane_mappings: setup.pane_mappings,
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

pub fn open_worktree(
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
) -> Result<()> {
    let wt_state = state
        .get_worktree(branch)
        .ok_or_else(|| MeldrError::WorktreeNotFound(branch.to_string()))?;

    // If tmux windows are still alive, just select the first one
    if let Some(ref window_ids) = wt_state.tmux_window {
        let first = window_ids.split(',').next().unwrap_or("");
        if !first.is_empty() && tmux.has_window(first) {
            tmux.select_window(first)?;
            return Ok(());
        }
    }

    // Windows are gone — recreate them
    if !config.should_use_tmux() {
        return Err(MeldrError::Tmux(
            "Cannot open worktree windows: tmux is disabled via --no-tabs or config".to_string(),
        ));
    }

    if !tmux.is_inside_tmux() {
        return Err(MeldrError::NotInTmux);
    }

    let packages: Vec<String> = manifest.packages.iter().map(|p| p.name.clone()).collect();
    let setup = setup_tmux_windows(
        tmux, manifest, workspace_root, branch, &packages, config, global_config,
    )?;

    state.add_worktree(
        branch,
        WorktreeState {
            branch: branch.to_string(),
            tmux_window: setup.tmux_window,
            pane_mappings: setup.pane_mappings,
        },
    );
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
    config: &EffectiveConfig,
    method_override: Option<&str>,
    strategy_override: Option<&str>,
) -> Result<()> {
    let method = method_override.unwrap_or(&config.sync_method);
    let strategy = strategy_override.unwrap_or(&config.sync_strategy);

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

        let remote = pkg.remote.as_deref().unwrap_or(&config.remote);

        println!("Syncing {}...", pkg.name);
        git.fetch(&repo_path, remote)?;

        // Resolve default branch: explicit > auto-detect > config fallback
        let detected;
        let default_branch = if let Some(ref b) = pkg.branch {
            b.as_str()
        } else {
            detected = git.detect_default_branch(&repo_path, remote);
            detected.as_deref().unwrap_or(&config.default_branch)
        };

        // Rebase/merge against the remote-tracking branch (bare repos have no local checkout)
        let upstream = format!("{}/{}", remote, default_branch);
        if method == "merge" {
            git.merge(&wt_path, &upstream, strategy)?;
        } else {
            git.rebase(&wt_path, &upstream, strategy, true)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_template_all_vars() {
        let result = expand_template("{ws}/{branch}:{pkg}", "myws", "feature-x", "frontend");
        assert_eq!(result, "myws/feature-x:frontend");
    }

    #[test]
    fn test_expand_template_no_pkg() {
        let result = expand_template("{ws}/{branch}", "myws", "main", "");
        assert_eq!(result, "myws/main");
    }

    #[test]
    fn test_expand_template_custom() {
        let result = expand_template("[{branch}] {pkg}", "ws", "dev", "api");
        assert_eq!(result, "[dev] api");
    }
}
