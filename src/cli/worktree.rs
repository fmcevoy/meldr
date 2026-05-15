use std::path::Path;

use crate::core::config::{EffectiveConfig, GlobalConfig};
use crate::core::filter::PackageFilter;
use crate::core::state::WorkspaceState;
use crate::core::workspace::Manifest;
use crate::error::Result;
use crate::git::GitOps;
use crate::tmux::TmuxOps;

#[allow(clippy::too_many_arguments)]
pub fn add(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
    filter: &PackageFilter,
    leader: Option<&str>,
) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let filtered_manifest = if filter.is_empty() {
        manifest.clone()
    } else {
        let mut m = manifest.clone();
        m.packages = filter
            .apply(&manifest.packages)
            .into_iter()
            .cloned()
            .collect();
        m
    };
    let mut state = WorkspaceState::load(workspace_root)?;
    crate::core::worktree::add_worktree(
        git,
        tmux,
        &filtered_manifest,
        &mut state,
        workspace_root,
        branch,
        config,
        global_config,
        leader,
    )?;
    println!("Created worktree '{branch}'");
    Ok(())
}

pub fn remove(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    workspace_root: &Path,
    branch: &str,
    force: bool,
    filter: &PackageFilter,
    config: &EffectiveConfig,
) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let partial = !filter.is_empty();
    let filtered_manifest = if partial {
        let mut m = manifest.clone();
        m.packages = filter
            .apply(&manifest.packages)
            .into_iter()
            .cloned()
            .collect();
        m
    } else {
        manifest.clone()
    };
    let mut state = WorkspaceState::load(workspace_root)?;
    crate::core::worktree::remove_worktree(
        git,
        tmux,
        &filtered_manifest,
        &mut state,
        workspace_root,
        branch,
        force,
        partial,
        config,
    )?;
    if partial {
        let names: Vec<_> = filtered_manifest.packages.iter().map(|p| &p.name).collect();
        println!(
            "Removed {} package(s) from worktree '{branch}'",
            names.len()
        );
    } else {
        println!("Removed worktree '{branch}'");
    }
    Ok(())
}

pub fn open(
    tmux: &dyn TmuxOps,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
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
        global_config,
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

pub fn scan(git: &dyn GitOps, workspace_root: &Path, prune: bool) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let mut state = WorkspaceState::load(workspace_root)?;
    let report = if prune {
        crate::core::worktree::scan_and_import_with_prune(
            git,
            &manifest,
            &mut state,
            workspace_root,
        )?
    } else {
        crate::core::worktree::scan_and_import(git, &manifest, &mut state, workspace_root)?
    };
    println!(
        "Scan: imported {}, already tracked {}, pruned {}",
        report.imported.len(),
        report.already_tracked.len(),
        report.pruned.len()
    );
    for branch in &report.imported {
        println!("  + {branch}");
    }
    for branch in &report.pruned {
        println!("  - {branch}");
    }
    Ok(())
}
