use std::path::Path;
use std::process::Command;

use crate::error::{MeldrError, Result};
use crate::trace;

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
    /// Returns the current HEAD commit SHA.
    fn current_head(&self, path: &Path) -> Result<String>;
    /// Hard-reset to a specific commit.
    fn reset_hard(&self, path: &Path, commit: &str) -> Result<()>;
}

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
            .map_err(|e| MeldrError::Git(format!("Failed to run git: {}", e)))?;

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
                            "Could not attach to existing branch ({}) or create new branch ({})",
                            first_err, second_err
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
        let mut args = vec!["rebase".to_string(), onto.to_string()];
        if strategy != "manual" {
            args.push(format!("-X{}", strategy));
        }
        if autostash {
            args.push("--autostash".to_string());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        Self::run(&arg_refs, path)?;
        Ok(())
    }

    fn merge(&self, path: &Path, branch: &str, strategy: &str) -> Result<()> {
        let mut args = vec!["merge".to_string(), branch.to_string()];
        if strategy != "manual" {
            args.push(format!("-X{}", strategy));
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        Self::run(&arg_refs, path)?;
        Ok(())
    }

    fn status_porcelain(&self, path: &Path) -> Result<String> {
        Self::run(&["status", "--porcelain"], path)
    }

    fn detect_default_branch(&self, path: &Path, remote: &str) -> Option<String> {
        let ref_path = format!("refs/remotes/{}/HEAD", remote);
        let output = Self::run(&["symbolic-ref", &ref_path], path).ok()?;
        // Output is like "refs/remotes/origin/main" — extract the branch name
        let prefix = format!("refs/remotes/{}/", remote);
        output.strip_prefix(&prefix).map(|s| s.to_string())
    }

    fn ensure_remote_tracking(&self, path: &Path, remote: &str) -> Result<()> {
        let refspec_key = format!("remote.{}.fetch", remote);
        let expected_refspec = format!(
            "+refs/heads/*:refs/remotes/{}/*",
            remote
        );

        // Check if the fetch refspec is already configured
        let current = Self::run(&["config", "--get-all", &refspec_key], path)
            .unwrap_or_default();

        if !current.lines().any(|line| line.trim() == expected_refspec) {
            // Set the fetch refspec so `git fetch` populates refs/remotes/<remote>/*
            Self::run(
                &["config", &refspec_key, &expected_refspec],
                path,
            )?;

            // Fetch to populate the remote tracking refs now
            Self::run(&["fetch", remote], path)?;
        }

        // Ensure refs/remotes/<remote>/HEAD is set
        let head_ref = format!("refs/remotes/{}/HEAD", remote);
        if Self::run(&["symbolic-ref", &head_ref], path).is_err() {
            // Detect the default branch from the remote and set HEAD
            let _ = Self::run(
                &["remote", "set-head", remote, "--auto"],
                path,
            );
        }

        Ok(())
    }

    fn divergence(&self, path: &Path, upstream: &str) -> Result<(u32, u32)> {
        let range = format!("HEAD...{}", upstream);
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
            .map_err(|e| MeldrError::Git(format!("Failed to run git merge-tree: {}", e)))?;

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

    fn current_head(&self, path: &Path) -> Result<String> {
        Self::run(&["rev-parse", "HEAD"], path)
    }

    fn reset_hard(&self, path: &Path, commit: &str) -> Result<()> {
        Self::run(&["reset", "--hard", commit], path)?;
        Ok(())
    }
}
