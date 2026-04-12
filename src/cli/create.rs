use std::path::Path;

use crate::core::config::{EffectiveConfig, GlobalConfig};
use crate::core::state::WorkspaceState;
use crate::core::workspace::Manifest;
use crate::error::{MeldrError, Result};
use crate::git::GitOps;
use crate::tmux::TmuxOps;

#[allow(clippy::too_many_arguments)]
pub fn run(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    parent_dir: &Path,
    name: &str,
    repos: &[String],
    branch: Option<&str>,
    agent: Option<&str>,
    leader: Option<&str>,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
) -> Result<()> {
    let workspace_root = parent_dir.join(name);
    if workspace_root.exists() {
        return Err(MeldrError::AlreadyInitialized(workspace_root));
    }

    std::fs::create_dir_all(&workspace_root)?;

    let mut manifest = Manifest::new(name);
    if let Some(agent_name) = agent {
        manifest.settings.agent = Some(agent_name.to_string());
    }
    manifest.save_initial(&workspace_root)?;

    std::fs::create_dir_all(workspace_root.join("packages"))?;
    std::fs::create_dir_all(workspace_root.join("worktrees"))?;
    std::fs::create_dir_all(workspace_root.join(".meldr"))?;

    println!("Created workspace '{name}'");

    if !repos.is_empty() {
        let added = crate::core::package::add_packages(git, &mut manifest, &workspace_root, repos)?;
        for pkg_name in &added {
            println!("Added package '{pkg_name}'");
        }
    }

    if let Some(branch_name) = branch {
        if manifest.packages.is_empty() {
            eprintln!("Warning: No packages to create worktrees for, skipping branch creation.");
        } else {
            let mut state = WorkspaceState::load(&workspace_root)?;
            crate::core::worktree::add_worktree(
                git,
                tmux,
                &manifest,
                &mut state,
                &workspace_root,
                branch_name,
                config,
                global_config,
                leader,
            )?;
            println!("Created worktree '{branch_name}'");
        }
    }

    println!("\nWorkspace ready at {}", workspace_root.display());
    Ok(())
}
