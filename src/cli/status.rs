use std::path::Path;

use console::style;

use crate::core::config::EffectiveConfig;
use crate::core::filter::PackageFilter;
use crate::core::state::WorkspaceState;
use crate::core::workspace::{self, Manifest};
use crate::error::Result;
use crate::git::GitOps;

#[derive(Debug, PartialEq)]
pub enum SyncState {
    Synced,
    Stale,
}

impl SyncState {
    pub fn from_divergence(_ahead: u32, behind: u32) -> Self {
        if behind > 0 {
            SyncState::Stale
        } else {
            SyncState::Synced
        }
    }

    pub fn label(&self) -> &str {
        match self {
            SyncState::Synced => "synced",
            SyncState::Stale => "stale",
        }
    }
}

pub fn run(
    git: &dyn GitOps,
    workspace_root: &Path,
    config: &EffectiveConfig,
    filter: &PackageFilter,
) -> Result<()> {
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

            let remote = pkg.remote.as_deref().unwrap_or(&config.remote);
            let dirty = match git.is_dirty(&wt_path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!(
                        "  {} {}: {}",
                        style("warning:").yellow(),
                        style(&pkg.name).bold(),
                        e
                    );
                    continue;
                }
            };
            let (ahead, behind) = match git.divergence(&wt_path, &format!("{remote}/{branch}")) {
                Ok(ab) => ab,
                Err(e) => {
                    eprintln!(
                        "  {} {}: divergence check failed: {}",
                        style("warning:").yellow(),
                        style(&pkg.name).bold(),
                        e
                    );
                    (0, 0)
                }
            };
            let last_commit = git
                .log_oneline(&wt_path, 1)
                .unwrap_or_default()
                .into_iter()
                .next()
                .unwrap_or_else(|| "\u{2014}".to_string());
            let last_commit = if last_commit.chars().count() > 30 {
                format!("{}…", last_commit.chars().take(29).collect::<String>())
            } else {
                last_commit
            };
            let sync_state = SyncState::from_divergence(ahead, behind);

            let status_padded = format!("{:<8}", if dirty { "dirty" } else { "clean" });
            let status_str = if dirty {
                style(status_padded).yellow().to_string()
            } else {
                style(status_padded).green().to_string()
            };
            let ab_raw = format!("\u{2191}{} \u{2193}{}", ahead, behind);
            let ab_padded = format!("{:<12}", ab_raw);
            let ab_styled = if behind > 0 {
                style(ab_padded).red().to_string()
            } else if ahead > 0 {
                style(ab_padded).yellow().to_string()
            } else {
                style(ab_padded).green().to_string()
            };
            let sync_padded = format!("{:<8}", sync_state.label());
            let sync_str = match sync_state {
                SyncState::Synced => style(sync_padded).green().to_string(),
                SyncState::Stale => style(sync_padded).yellow().to_string(),
            };

            println!(
                "  {:<16} {} {} {:<30} {}",
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
    }
}
