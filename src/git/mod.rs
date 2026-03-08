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

        let output = Command::new("git")
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
}
