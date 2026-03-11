use std::path::Path;

use console::style;

use crate::core::state::WorkspaceState;
use crate::core::workspace::{self, Manifest};
use crate::error::Result;
use crate::git::GitOps;

pub fn run(git: &dyn GitOps, workspace_root: &Path) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let state = WorkspaceState::load(workspace_root)?;

    println!("{} {}", style("Workspace:").bold(), manifest.workspace.name);
    println!();

    println!("{}", style("Packages:").bold().underlined());
    for pkg in &manifest.packages {
        let pkg_path = workspace::package_path(workspace_root, &pkg.name);
        let status_marker = if pkg_path.exists() {
            style("✓").green().to_string()
        } else {
            style("✗").red().to_string()
        };
        let branch_info = pkg.branch.as_deref().unwrap_or("(default)");
        println!("  {} {} ({})", status_marker, pkg.name, branch_info);
    }
    if manifest.packages.is_empty() {
        println!("  (none)");
    }

    println!();
    println!("{}", style("Worktrees:").bold().underlined());
    if state.worktrees.is_empty() {
        println!("  (none)");
    } else {
        for (branch, wt) in &state.worktrees {
            let tmux_info = wt
                .tmux_window
                .as_deref()
                .map(|w| format!(" [tmux: {}]", w))
                .unwrap_or_default();

            let mut dirty_pkgs = Vec::new();
            for pkg in &manifest.packages {
                let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
                if wt_path.exists()
                    && let Ok(true) = git.is_dirty(&wt_path)
                {
                    dirty_pkgs.push(pkg.name.clone());
                }
            }

            let dirty_info = if dirty_pkgs.is_empty() {
                String::new()
            } else {
                format!(
                    " {}",
                    style(format!("(dirty: {})", dirty_pkgs.join(", "))).red()
                )
            };

            println!("  {}{}{}", branch, tmux_info, dirty_info);
        }
    }

    Ok(())
}
