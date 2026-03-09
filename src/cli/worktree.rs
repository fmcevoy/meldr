use std::path::Path;

use crate::core::config::{EffectiveConfig, GlobalConfig};
use crate::core::state::WorkspaceState;
use crate::core::workspace::Manifest;
use crate::error::Result;
use crate::git::GitOps;
use crate::tmux::TmuxOps;

pub fn add(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let mut state = WorkspaceState::load(workspace_root)?;
    crate::core::worktree::add_worktree(
        git,
        tmux,
        &manifest,
        &mut state,
        workspace_root,
        branch,
        config,
    )?;
    println!("Created worktree '{}'", branch);
    Ok(())
}

pub fn remove(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    workspace_root: &Path,
    branch: &str,
    force: bool,
) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let mut state = WorkspaceState::load(workspace_root)?;
    crate::core::worktree::remove_worktree(
        git,
        tmux,
        &manifest,
        &mut state,
        workspace_root,
        branch,
        force,
    )?;
    println!("Removed worktree '{}'", branch);
    Ok(())
}

pub fn open(
    _git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let mut state = WorkspaceState::load(workspace_root)?;
    crate::core::worktree::open_worktree(
        tmux,
        &manifest,
        &mut state,
        workspace_root,
        branch,
        config,
    )?;
    Ok(())
}

pub fn list(workspace_root: &Path) -> Result<()> {
    let state = WorkspaceState::load(workspace_root)?;
    let worktrees = crate::core::worktree::list_worktrees(&state);
    if worktrees.is_empty() {
        println!("No active worktrees.");
    } else {
        for wt in worktrees {
            let tmux_info = wt.tmux_window.as_deref().unwrap_or("no tmux window");
            println!("  {} (tmux: {})", wt.branch, tmux_info);
        }
    }
    Ok(())
}
