use std::path::Path;

use console::style;

use crate::core::filter::PackageFilter;
use crate::core::state::WorkspaceState;
use crate::core::workspace::{self, Manifest};
use crate::error::Result;
use crate::git::GitOps;

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub enum SyncState {
    Synced,
    Stale,
    Conflict,
}

impl SyncState {
    pub fn from_divergence(_ahead: u32, behind: u32) -> Self {
        if behind > 0 {
            SyncState::Stale
        } else {
            SyncState::Synced
        }
    }
    #[allow(dead_code)]
    pub fn label(&self) -> &str {
        match self {
            SyncState::Synced => "synced",
            SyncState::Stale => "stale",
            SyncState::Conflict => "conflict",
        }
    }
}

pub fn run(git: &dyn GitOps, workspace_root: &Path, filter: &PackageFilter) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let state = WorkspaceState::load(workspace_root)?;

    println!("{} {}", style("Workspace:").bold(), manifest.workspace.name);
    println!();

    // Packages overview
    println!("{}", style("Packages:").bold().underlined());
    let filtered_pkgs: Vec<_> = filter
        .apply(&manifest.packages)
        .into_iter()
        .cloned()
        .collect();
    for pkg in &filtered_pkgs {
        let pkg_path = workspace::package_path(workspace_root, &pkg.name);
        let marker = if pkg_path.exists() {
            style("✓").green()
        } else {
            style("✗").red()
        };
        let groups_info = if pkg.groups.is_empty() {
            String::new()
        } else {
            format!(" [{}]", pkg.groups.join(", "))
        };
        println!("  {} {}{}", marker, pkg.name, style(groups_info).dim());
    }
    println!();

    // Worktrees with per-package detail table
    if state.worktrees.is_empty() {
        println!("{}", style("No active worktrees.").dim());
        return Ok(());
    }

    for (branch, wt) in &state.worktrees {
        let tmux_info = wt
            .tmux_window
            .as_deref()
            .map(|w| format!(" [tmux: {w}]"))
            .unwrap_or_default();
        println!(
            "{} {}{}",
            style("Worktree:").bold(),
            branch,
            style(tmux_info).dim()
        );

        // Table header
        println!(
            "  {:<16} {:<8} {:<12} {:<30} {:<8}",
            style("Package").underlined(),
            style("Status").underlined(),
            style("Ahead/Behind").underlined(),
            style("Last Commit").underlined(),
            style("Sync").underlined(),
        );

        for pkg in &filtered_pkgs {
            let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
            if !wt_path.exists() {
                continue;
            }

            let dirty = git.is_dirty(&wt_path).unwrap_or(false);
            let (ahead, behind) = git
                .divergence(&wt_path, &format!("origin/{branch}"))
                .unwrap_or((0, 0));
            let last_commit = git
                .log_oneline(&wt_path, 1)
                .unwrap_or_default()
                .into_iter()
                .next()
                .unwrap_or_else(|| "\u{2014}".to_string());
            let last_commit = if last_commit.len() > 30 {
                format!("{}…", &last_commit[..29])
            } else {
                last_commit
            };
            let sync_state = SyncState::from_divergence(ahead, behind);

            let status_str = if dirty {
                style("dirty").yellow().to_string()
            } else {
                style("clean").green().to_string()
            };
            let ab_str = format!("\u{2191}{} \u{2193}{}", ahead, behind);
            let ab_styled = if behind > 0 {
                style(ab_str).red().to_string()
            } else if ahead > 0 {
                style(ab_str).yellow().to_string()
            } else {
                style(ab_str).green().to_string()
            };
            let sync_str = match sync_state {
                SyncState::Synced => style("synced").green().to_string(),
                SyncState::Stale => style("stale").yellow().to_string(),
                SyncState::Conflict => style("conflict").red().to_string(),
            };

            println!(
                "  {:<16} {:<8} {:<12} {:<30} {:<8}",
                pkg.name, status_str, ab_styled, last_commit, sync_str,
            );
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_state_from_divergence() {
        assert_eq!(SyncState::from_divergence(0, 0), SyncState::Synced);
        assert_eq!(SyncState::from_divergence(0, 3), SyncState::Stale);
        assert_eq!(SyncState::from_divergence(2, 0), SyncState::Synced);
        assert_eq!(SyncState::from_divergence(2, 3), SyncState::Stale);
    }

    #[test]
    fn test_sync_state_labels() {
        assert_eq!(SyncState::Synced.label(), "synced");
        assert_eq!(SyncState::Stale.label(), "stale");
        assert_eq!(SyncState::Conflict.label(), "conflict");
    }
}
