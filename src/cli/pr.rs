use std::path::Path;
use std::process::Command;

use console::style;

use crate::core::filter::PackageFilter;
use crate::core::hooks;
use crate::core::state::WorkspaceState;
use crate::core::workspace::{self, Manifest};
use crate::error::Result;
use crate::git::GitOps;

/// Extract "owner/repo" from a GitHub remote URL.
///
/// Supports both HTTPS and SSH formats:
/// - `https://github.com/owner/repo.git` -> `owner/repo`
/// - `git@github.com:owner/repo.git` -> `owner/repo`
pub fn extract_github_repo(url: &str) -> Option<String> {
    let url = url.trim();

    // SSH format: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let rest = rest.trim_end_matches(".git");
        if rest.contains('/') {
            return Some(rest.to_string());
        }
        return None;
    }

    // HTTPS format: https://github.com/owner/repo.git
    let stripped = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let stripped = stripped.trim_end_matches(".git").trim_end_matches('/');
    let parts: Vec<&str> = stripped.splitn(3, '/').collect();
    if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else {
        None
    }
}

/// Check whether a package has unpushed commits or uncommitted changes.
pub fn has_changes(git: &dyn GitOps, path: &Path, branch: &str) -> Result<bool> {
    if git.is_dirty(path)? {
        return Ok(true);
    }
    let upstream = format!("origin/{branch}");
    match git.divergence(path, &upstream) {
        Ok((ahead, _)) => Ok(ahead > 0),
        // If upstream doesn't exist yet, the branch is entirely new — treat as having changes
        Err(_) => Ok(true),
    }
}

/// Build cross-reference text to append to PR bodies.
pub fn build_cross_reference_body(workspace_name: &str, prs: &[(&str, String)]) -> String {
    let mut body = format!(
        "\n---\nPart of coordinated change across `{}`:\n",
        workspace_name
    );
    for (_pkg, pr_ref) in prs {
        body.push_str(&format!("- {pr_ref}\n"));
    }
    body
}

/// Parse a PR number from a GitHub PR URL like `https://github.com/owner/repo/pull/42`.
fn parse_pr_number(url: &str) -> Option<u32> {
    url.trim().rsplit('/').next()?.parse().ok()
}

/// Check that `gh` CLI is available.
fn check_gh_installed() -> Result<()> {
    let result = Command::new("gh").arg("--version").output();
    match result {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(crate::error::MeldrError::Config(
            "GitHub CLI (gh) is not installed or not in PATH. Install it from https://cli.github.com/"
                .to_string(),
        )),
    }
}

/// Detect the current worktree branch from cwd.
fn detect_branch(root: &Path, cwd: &Path) -> Result<String> {
    let state = WorkspaceState::load(root)?;
    let dir_name = workspace::detect_current_worktree_dir(root, cwd);
    dir_name
        .and_then(|d| {
            workspace::resolve_branch_from_dir(&d, state.worktrees.keys().map(|s| s.as_str()))
        })
        .ok_or_else(|| {
            crate::error::MeldrError::Config(
                "Could not detect current worktree branch. Run this command from within a worktree directory."
                    .to_string(),
            )
        })
}

/// Create coordinated PRs across dirty packages in the current worktree.
#[allow(clippy::too_many_arguments)]
pub fn create(
    git: &dyn GitOps,
    root: &Path,
    cwd: &Path,
    filter: &PackageFilter,
    title: Option<String>,
    body: Option<String>,
    draft: bool,
) -> Result<()> {
    check_gh_installed()?;

    let branch = detect_branch(root, cwd)?;
    let manifest = Manifest::load(root)?;
    let filtered = filter.apply(&manifest.packages);

    if filtered.is_empty() {
        println!("{}", style("No packages match the filter.").yellow());
        return Ok(());
    }

    // Check which packages have changes
    let mut dirty_packages = Vec::new();
    for pkg in &filtered {
        let wt_path = workspace::worktree_path(root, &branch, &pkg.name);
        if !wt_path.exists() {
            continue;
        }
        match has_changes(git, &wt_path, &branch) {
            Ok(true) => dirty_packages.push(*pkg),
            Ok(false) => {}
            Err(e) => {
                eprintln!(
                    "  {} Could not check {} for changes: {}",
                    style("warning:").yellow(),
                    style(&pkg.name).bold(),
                    e
                );
            }
        }
    }

    if dirty_packages.is_empty() {
        println!(
            "{}",
            style("No packages have changes to create PRs for.").yellow()
        );
        return Ok(());
    }

    let pr_title = title.unwrap_or_else(|| branch.clone());
    let pr_body = body.unwrap_or_default();
    let ws_name = &manifest.workspace.name;

    // Phase 1: push + create PRs, collecting results
    let mut successes: Vec<(&str, String, String)> = Vec::new(); // (pkg_name, pr_url, owner/repo#num)
    let mut failures: Vec<(&str, String)> = Vec::new();

    for pkg in &dirty_packages {
        let wt_path = workspace::worktree_path(root, &branch, &pkg.name);
        let repo_slug = match extract_github_repo(&pkg.url) {
            Some(slug) => slug,
            None => {
                let msg = format!("Could not extract GitHub repo from URL: {}", pkg.url);
                eprintln!(
                    "  {} {}: {}",
                    style("skip").yellow(),
                    style(&pkg.name).bold(),
                    msg
                );
                failures.push((&pkg.name, msg));
                continue;
            }
        };

        // Push
        println!("  {} {}", style("pushing").cyan(), style(&pkg.name).bold());
        let push_result = Command::new("git")
            .args(["push", "-u", "origin", &branch])
            .current_dir(&wt_path)
            .output();

        match push_result {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                eprintln!(
                    "  {} {}: push failed: {}",
                    style("error").red(),
                    style(&pkg.name).bold(),
                    stderr.trim()
                );
                failures.push((&pkg.name, format!("push failed: {}", stderr.trim())));
                continue;
            }
            Err(e) => {
                eprintln!(
                    "  {} {}: push failed: {}",
                    style("error").red(),
                    style(&pkg.name).bold(),
                    e
                );
                failures.push((&pkg.name, format!("push failed: {e}")));
                continue;
            }
        }

        // Create PR
        println!(
            "  {} {}",
            style("creating PR").cyan(),
            style(&pkg.name).bold()
        );
        let mut gh_args = vec![
            "pr".to_string(),
            "create".to_string(),
            "--title".to_string(),
            pr_title.clone(),
            "--repo".to_string(),
            repo_slug.clone(),
            "--body".to_string(),
            pr_body.clone(),
            "--head".to_string(),
            branch.clone(),
        ];
        if draft {
            gh_args.push("--draft".to_string());
        }

        let pr_result = Command::new("gh")
            .args(&gh_args)
            .current_dir(&wt_path)
            .output();

        match pr_result {
            Ok(output) if output.status.success() => {
                let pr_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let pr_num = parse_pr_number(&pr_url).unwrap_or(0);
                let pr_ref = format!("{repo_slug}#{pr_num}");
                println!(
                    "  {} {}: {}",
                    style("created").green(),
                    style(&pkg.name).bold(),
                    style(&pr_url).underlined()
                );
                successes.push((&pkg.name, pr_url, pr_ref));
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                eprintln!(
                    "  {} {}: PR creation failed: {}",
                    style("error").red(),
                    style(&pkg.name).bold(),
                    stderr.trim()
                );
                failures.push((&pkg.name, format!("PR creation failed: {}", stderr.trim())));
            }
            Err(e) => {
                eprintln!(
                    "  {} {}: PR creation failed: {}",
                    style("error").red(),
                    style(&pkg.name).bold(),
                    e
                );
                failures.push((&pkg.name, format!("PR creation failed: {e}")));
            }
        }
    }

    // Phase 2: cross-reference if multiple PRs
    if successes.len() > 1 {
        let pr_refs: Vec<(&str, String)> = successes
            .iter()
            .map(|(name, _url, pr_ref)| (*name, pr_ref.clone()))
            .collect();
        let xref_body = build_cross_reference_body(ws_name, &pr_refs);

        for (pkg_name, pr_url, _) in &successes {
            let repo_slug = successes
                .iter()
                .find(|(n, _, _)| n == pkg_name)
                .and_then(|(_, _, r)| r.split('#').next())
                .unwrap_or("");

            let edit_result = Command::new("gh")
                .args([
                    "pr",
                    "edit",
                    pr_url,
                    "--repo",
                    repo_slug,
                    "--add-body",
                    &xref_body,
                ])
                .output();

            match edit_result {
                Ok(output) if output.status.success() => {
                    eprintln!(
                        "  {} cross-reference added to {}",
                        style("linked").blue(),
                        style(pkg_name).bold()
                    );
                }
                _ => {
                    eprintln!(
                        "  {} could not add cross-reference to {}",
                        style("warning:").yellow(),
                        style(pkg_name).bold()
                    );
                }
            }
        }
    }

    // Phase 3: run post_pr hooks
    let hook_packages: Vec<&crate::core::workspace::PackageEntry> = successes
        .iter()
        .filter_map(|(name, _, _)| manifest.packages.iter().find(|p| p.name == *name))
        .collect();

    if !hook_packages.is_empty() {
        hooks::run_hooks("post_pr", &manifest, &hook_packages, |pkg_name| {
            workspace::worktree_path(root, &branch, pkg_name)
        });
    }

    // Summary
    println!();
    if !successes.is_empty() {
        println!(
            "{} {} PR{} created",
            style("Summary:").bold(),
            successes.len(),
            if successes.len() == 1 { "" } else { "s" }
        );
        for (pkg, url, _) in &successes {
            println!("  {} {}", style(pkg).green().bold(), url);
        }
    }
    if !failures.is_empty() {
        println!(
            "{} {} failure{}:",
            style("Errors:").red().bold(),
            failures.len(),
            if failures.len() == 1 { "" } else { "s" }
        );
        for (pkg, reason) in &failures {
            println!("  {} {}", style(pkg).red().bold(), reason);
        }
    }

    Ok(())
}

/// Show PR status (stub — to be implemented in Task 9).
pub fn status(_git: &dyn GitOps, _root: &Path, _cwd: &Path, _filter: &PackageFilter) -> Result<()> {
    println!("PR status not yet implemented.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_github_repo_https() {
        assert_eq!(
            extract_github_repo("https://github.com/fmcevoy/meldr.git"),
            Some("fmcevoy/meldr".to_string())
        );
    }

    #[test]
    fn test_extract_github_repo_ssh() {
        assert_eq!(
            extract_github_repo("git@github.com:fmcevoy/meldr.git"),
            Some("fmcevoy/meldr".to_string())
        );
    }

    #[test]
    fn test_extract_github_repo_no_git_suffix() {
        assert_eq!(
            extract_github_repo("https://github.com/fmcevoy/meldr"),
            Some("fmcevoy/meldr".to_string())
        );
    }

    #[test]
    fn test_extract_github_repo_non_github() {
        assert_eq!(extract_github_repo("https://gitlab.com/foo/bar.git"), None);
    }

    #[test]
    fn test_extract_github_repo_ssh_no_dot_git() {
        assert_eq!(
            extract_github_repo("git@github.com:org/repo"),
            Some("org/repo".to_string())
        );
    }

    #[test]
    fn test_cross_reference_body() {
        let prs = vec![
            ("api", "fmcevoy/api#42".to_string()),
            ("web", "fmcevoy/web#18".to_string()),
        ];
        let body = build_cross_reference_body("ws-meldr", &prs);
        assert!(body.contains("fmcevoy/api#42"));
        assert!(body.contains("fmcevoy/web#18"));
        assert!(body.contains("coordinated change"));
    }

    #[test]
    fn test_cross_reference_body_single_pr() {
        let prs = vec![("api", "fmcevoy/api#42".to_string())];
        let body = build_cross_reference_body("ws-meldr", &prs);
        assert!(body.contains("fmcevoy/api#42"));
    }

    #[test]
    fn test_parse_pr_number() {
        assert_eq!(
            parse_pr_number("https://github.com/fmcevoy/meldr/pull/42"),
            Some(42)
        );
        assert_eq!(parse_pr_number("not-a-url"), None);
    }
}
