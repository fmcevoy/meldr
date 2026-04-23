use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{MeldrError, Result};
use crate::trace;

/// A single entry from `git worktree list --porcelain`.
/// `branch` is `None` for detached-HEAD worktrees; bare entries are filtered out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub branch: Option<String>,
}

pub trait GitOps: Send + Sync {
    fn clone_repo(&self, url: &str, path: &Path) -> Result<()>;
    fn worktree_add(&self, repo: &Path, dest: &Path, branch: &str) -> Result<()>;
    fn worktree_remove(&self, repo: &Path, path: &Path, force: bool) -> Result<()>;
    fn is_dirty(&self, path: &Path) -> Result<bool>;
    fn fetch(&self, path: &Path, remote: &str) -> Result<()>;
    fn rebase(&self, path: &Path, onto: &str, strategy: &str, autostash: bool) -> Result<()>;
    fn merge(&self, path: &Path, branch: &str, strategy: &str) -> Result<()>;
    fn status_porcelain(&self, path: &Path) -> Result<String>;
    fn detect_default_branch(&self, path: &Path, remote: &str) -> Option<String>;
    fn ensure_remote_tracking(&self, path: &Path, remote: &str) -> Result<()>;
    /// Returns (ahead, behind) commit counts relative to upstream.
    fn divergence(&self, path: &Path, upstream: &str) -> Result<(u32, u32)>;
    /// Check for merge conflicts without modifying the working tree.
    /// Returns a list of conflicting file paths. Empty means clean merge.
    /// Uses `git merge-tree --write-tree` (Git 2.38+). Falls back gracefully on older Git.
    fn check_merge_conflicts(&self, path: &Path, upstream: &str) -> Result<Vec<String>>;
    /// Returns the last N commits as "short_sha message" lines.
    fn log_oneline(&self, path: &Path, count: u32) -> Result<Vec<String>>;
    /// Returns the current HEAD commit SHA.
    fn current_head(&self, path: &Path) -> Result<String>;
    /// Hard-reset to a specific commit.
    fn reset_hard(&self, path: &Path, commit: &str) -> Result<()>;
    /// Push a branch to a remote.
    fn push(&self, path: &Path, remote: &str, branch: &str) -> Result<()>;
    /// Fast-forward a local branch ref to match a remote tracking ref.
    /// Uses `git fetch . <src>:<dst>` which only succeeds for fast-forwards.
    fn fast_forward_branch(&self, repo: &Path, branch: &str, remote: &str) -> Result<()>;
    /// List the worktrees registered in `repo` (a bare or non-bare git dir).
    /// Bare entries are filtered out; detached-HEAD worktrees yield `branch: None`.
    fn worktree_list(&self, repo: &Path) -> Result<Vec<WorktreeEntry>>;
}

#[derive(Default)]
pub struct RealGit;

impl RealGit {
    pub fn new() -> Self {
        Self
    }

    fn run(args: &[&str], cwd: &Path) -> Result<String> {
        trace::trace_cmd("git", args, Some(&cwd.to_string_lossy()));

        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| MeldrError::Git(format!("Failed to run git: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(MeldrError::Git(stderr))
        }
    }
}

impl GitOps for RealGit {
    fn clone_repo(&self, url: &str, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        let args = ["clone", "--bare", url, &path_str];
        trace::trace_cmd("git", &args, None);

        let output =
            Command::new("git")
                .args(args)
                .output()
                .map_err(|e| MeldrError::CloneFailed {
                    url: url.to_string(),
                    reason: e.to_string(),
                })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(MeldrError::CloneFailed {
                url: url.to_string(),
                reason: stderr,
            });
        }

        // Bare clones don't set up remote tracking refs by default.
        // Configure the fetch refspec so `git fetch` populates refs/remotes/origin/*.
        self.ensure_remote_tracking(path, "origin")?;

        Ok(())
    }

    fn worktree_add(&self, repo: &Path, dest: &Path, branch: &str) -> Result<()> {
        let dest_str = dest.to_string_lossy().to_string();
        match Self::run(&["worktree", "add", &dest_str, branch], repo) {
            Ok(_) => Ok(()),
            Err(first_err) => {
                Self::run(&["worktree", "add", &dest_str, "-b", branch], repo).map_err(
                    |second_err| {
                        MeldrError::Git(format!(
                            "Could not attach to existing branch ({first_err}) or create new branch ({second_err})"
                        ))
                    },
                )?;
                Ok(())
            }
        }
    }

    fn worktree_remove(&self, repo: &Path, path: &Path, force: bool) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let mut args = vec!["worktree", "remove", &path_str];
        if force {
            args.push("--force");
        }
        Self::run(&args, repo)?;
        Ok(())
    }

    fn is_dirty(&self, path: &Path) -> Result<bool> {
        let output = self.status_porcelain(path)?;
        Ok(!output.is_empty())
    }

    fn fetch(&self, path: &Path, remote: &str) -> Result<()> {
        // Ensure remote tracking is configured (handles repos cloned before the fix)
        self.ensure_remote_tracking(path, remote)?;
        Self::run(&["fetch", remote], path)?;
        Ok(())
    }

    fn rebase(&self, path: &Path, onto: &str, strategy: &str, autostash: bool) -> Result<()> {
        let strategy_flag = format!("-X{strategy}");
        let mut args = vec!["rebase", onto];
        if strategy != "manual" {
            args.push(&strategy_flag);
        }
        if autostash {
            args.push("--autostash");
        }
        Self::run(&args, path)?;
        Ok(())
    }

    fn merge(&self, path: &Path, branch: &str, strategy: &str) -> Result<()> {
        let strategy_flag = format!("-X{strategy}");
        let mut args = vec!["merge", branch];
        if strategy != "manual" {
            args.push(&strategy_flag);
        }
        Self::run(&args, path)?;
        Ok(())
    }

    fn status_porcelain(&self, path: &Path) -> Result<String> {
        Self::run(&["status", "--porcelain"], path)
    }

    fn detect_default_branch(&self, path: &Path, remote: &str) -> Option<String> {
        let ref_path = format!("refs/remotes/{remote}/HEAD");
        let output = Self::run(&["symbolic-ref", &ref_path], path).ok()?;
        // Output is like "refs/remotes/origin/main" — extract the branch name
        let prefix = format!("refs/remotes/{remote}/");
        output.strip_prefix(&prefix).map(|s| s.to_string())
    }

    fn ensure_remote_tracking(&self, path: &Path, remote: &str) -> Result<()> {
        let refspec_key = format!("remote.{remote}.fetch");
        let expected_refspec = format!("+refs/heads/*:refs/remotes/{remote}/*");

        // Check if the fetch refspec is already configured
        let current = Self::run(&["config", "--get-all", &refspec_key], path).unwrap_or_default();

        if !current.lines().any(|line| line.trim() == expected_refspec) {
            // Set the fetch refspec so `git fetch` populates refs/remotes/<remote>/*
            Self::run(&["config", &refspec_key, &expected_refspec], path)?;

            // Fetch to populate the remote tracking refs now
            Self::run(&["fetch", remote], path)?;
        }

        // Ensure refs/remotes/<remote>/HEAD is set
        let head_ref = format!("refs/remotes/{remote}/HEAD");
        if Self::run(&["symbolic-ref", &head_ref], path).is_err() {
            // Detect the default branch from the remote and set HEAD
            let _ = Self::run(&["remote", "set-head", remote, "--auto"], path);
        }

        Ok(())
    }

    fn divergence(&self, path: &Path, upstream: &str) -> Result<(u32, u32)> {
        let range = format!("HEAD...{upstream}");
        let output = Self::run(&["rev-list", "--left-right", "--count", &range], path)?;
        let parts: Vec<&str> = output.split_whitespace().collect();
        if parts.len() != 2 {
            return Ok((0, 0));
        }
        let ahead = parts[0].parse::<u32>().unwrap_or(0);
        let behind = parts[1].parse::<u32>().unwrap_or(0);
        Ok((ahead, behind))
    }

    fn check_merge_conflicts(&self, path: &Path, upstream: &str) -> Result<Vec<String>> {
        trace::trace_cmd(
            "git",
            &["merge-tree", "--write-tree", "HEAD", upstream],
            Some(&path.to_string_lossy()),
        );

        let output = Command::new("git")
            .args(["merge-tree", "--write-tree", "HEAD", upstream])
            .current_dir(path)
            .output()
            .map_err(|e| MeldrError::Git(format!("Failed to run git merge-tree: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Git < 2.38 doesn't support --write-tree; fall back gracefully
        if stderr.contains("unrecognized argument")
            || stderr.contains("unknown option")
            || stderr.contains("not a git command")
        {
            return Ok(vec![]);
        }

        if output.status.success() {
            return Ok(vec![]);
        }

        let conflicts: Vec<String> = stdout
            .lines()
            .filter(|line| line.starts_with("CONFLICT"))
            .filter_map(|line| line.rfind(" in ").map(|pos| line[pos + 4..].to_string()))
            .collect();

        Ok(conflicts)
    }

    fn log_oneline(&self, path: &Path, count: u32) -> Result<Vec<String>> {
        let output = Self::run(&["log", "--oneline", &format!("-{count}")], path)?;
        Ok(output.lines().map(String::from).collect())
    }

    fn current_head(&self, path: &Path) -> Result<String> {
        Self::run(&["rev-parse", "HEAD"], path)
    }

    fn reset_hard(&self, path: &Path, commit: &str) -> Result<()> {
        Self::run(&["reset", "--hard", commit], path)?;
        Ok(())
    }

    fn push(&self, path: &Path, remote: &str, branch: &str) -> Result<()> {
        Self::run(&["push", "-u", remote, branch], path)?;
        Ok(())
    }

    fn fast_forward_branch(&self, repo: &Path, branch: &str, remote: &str) -> Result<()> {
        // `git fetch . <src>:<dst>` fails when the target branch is checked
        // out in a non-bare repo.  Detect that case and use `git merge
        // --ff-only` instead.
        let is_bare = Self::run(&["rev-parse", "--is-bare-repository"], repo)
            .map(|s| s == "true")
            .unwrap_or(false);

        let checked_out = !is_bare
            && Self::run(&["symbolic-ref", "--quiet", "HEAD"], repo)
                .map(|head| head == format!("refs/heads/{branch}"))
                .unwrap_or(false);

        if checked_out {
            let upstream = format!("{remote}/{branch}");
            Self::run(&["merge", "--ff-only", &upstream], repo)?;
        } else {
            let src = format!("refs/remotes/{remote}/{branch}");
            let dst = format!("refs/heads/{branch}");
            let refspec = format!("{src}:{dst}");
            Self::run(&["fetch", ".", &refspec], repo)?;
        }
        Ok(())
    }

    fn worktree_list(&self, repo: &Path) -> Result<Vec<WorktreeEntry>> {
        let output = Self::run(&["worktree", "list", "--porcelain"], repo)?;
        Ok(parse_worktree_list_porcelain(&output))
    }
}

/// Parse the output of `git worktree list --porcelain`.
///
/// Entries are separated by blank lines. Each entry has a `worktree <path>` line
/// and optional `HEAD <sha>`, `branch refs/heads/<name>`, `bare`, `detached`,
/// `locked`, `prunable` lines. Bare entries are dropped. Detached entries yield
/// `branch: None`. Annotation lines (`locked`, `prunable`, ...) are ignored.
pub fn parse_worktree_list_porcelain(s: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_branch: Option<String> = None;
    let mut is_bare = false;

    let flush = |path: &mut Option<PathBuf>,
                 branch: &mut Option<String>,
                 bare: &mut bool,
                 out: &mut Vec<WorktreeEntry>| {
        if let Some(p) = path.take()
            && !*bare
        {
            out.push(WorktreeEntry {
                path: p,
                branch: branch.take(),
            });
        }
        branch.take();
        *bare = false;
    };

    for line in s.lines() {
        if line.is_empty() {
            flush(
                &mut current_path,
                &mut current_branch,
                &mut is_bare,
                &mut out,
            );
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            // Defensive: if we hit a new `worktree` without a blank line, flush first
            flush(
                &mut current_path,
                &mut current_branch,
                &mut is_bare,
                &mut out,
            );
            current_path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("branch ") {
            current_branch = Some(rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string());
        } else if line == "bare" {
            is_bare = true;
        }
        // Ignore HEAD, detached, locked, prunable, and anything else.
    }
    // Final entry (no trailing blank line)
    flush(
        &mut current_path,
        &mut current_branch,
        &mut is_bare,
        &mut out,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_worktree_list_porcelain_canonical() {
        let input = "\
worktree /repo/packages/pkg.git
HEAD 0000000000000000000000000000000000000000
bare

worktree /repo/worktrees/main-wt/pkg
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /repo/worktrees/feat-wt/pkg
HEAD 2222222222222222222222222222222222222222
detached
";
        let entries = parse_worktree_list_porcelain(input);
        assert_eq!(entries.len(), 2, "bare entry should be filtered out");
        assert_eq!(
            entries[0].path,
            PathBuf::from("/repo/worktrees/main-wt/pkg")
        );
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(
            entries[1].path,
            PathBuf::from("/repo/worktrees/feat-wt/pkg")
        );
        assert!(entries[1].branch.is_none(), "detached yields None branch");
    }

    #[test]
    fn test_parse_worktree_list_porcelain_empty() {
        assert!(parse_worktree_list_porcelain("").is_empty());
        assert!(parse_worktree_list_porcelain("\n\n").is_empty());
    }

    #[test]
    fn test_parse_worktree_list_porcelain_ignores_annotations() {
        let input = "\
worktree /repo/worktrees/a/pkg
HEAD 1111111111111111111111111111111111111111
branch refs/heads/feature/foo
locked

worktree /repo/worktrees/b/pkg
HEAD 2222222222222222222222222222222222222222
branch refs/heads/main
prunable gitdir file points to non-existent location
";
        let entries = parse_worktree_list_porcelain(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("feature/foo"));
        assert_eq!(entries[1].branch.as_deref(), Some("main"));
    }

    #[test]
    fn test_parse_worktree_list_porcelain_preserves_slash_branches() {
        let input = "worktree /w/a\nbranch refs/heads/team/epic/story\n";
        let entries = parse_worktree_list_porcelain(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch.as_deref(), Some("team/epic/story"));
    }
}
