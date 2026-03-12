use std::path::Path;

use crate::core::workspace;

/// Run the prompt-check command.
///
/// Prints a warning when cwd is in a meldr worktree whose git branch
/// doesn't match the expected branch from the directory structure.
/// Always exits 0 — this is meant to be called from shell prompts.
pub fn run(workspace_root: &Path, cwd: &Path) {
    let Some(dir_name) = workspace::detect_current_worktree_dir(workspace_root, cwd) else {
        return;
    };

    let actual_branch = match read_current_branch(workspace_root, cwd) {
        Some(b) => b,
        None => {
            // Detached HEAD or unreadable — warn with "detached"
            eprintln!("\u{26a0} detached HEAD in worktree:{dir_name}");
            return;
        }
    };

    let expected_branch = workspace::sanitize_branch_for_dir(&actual_branch);
    if expected_branch != dir_name {
        eprintln!("\u{26a0} expected:{dir_name}");
    }
}

/// Read the current git branch by parsing `.git/HEAD` directly.
///
/// Returns `None` for detached HEAD or if the file can't be read.
fn read_current_branch(workspace_root: &Path, cwd: &Path) -> Option<String> {
    // Find the git dir for the package we're in. In a worktree the cwd is
    // something like `<root>/worktrees/<branch-dir>/<package>/...`.
    // The `.git` entry could be a file (pointing to the real git dir) or a
    // directory. We just need to read HEAD.
    let git_head = find_git_head(workspace_root, cwd)?;
    let content = std::fs::read_to_string(&git_head).ok()?;
    let content = content.trim();

    // Standard format: "ref: refs/heads/<branch>"
    content
        .strip_prefix("ref: refs/heads/")
        .map(|b| b.to_string())
}

/// Locate the HEAD file for the git repository containing `cwd`.
///
/// Walks up from `cwd` (stopping at `workspace_root`) looking for a `.git`
/// entry. If `.git` is a file (gitdir pointer), follows it to find HEAD.
fn find_git_head(workspace_root: &Path, cwd: &Path) -> Option<std::path::PathBuf> {
    let mut dir = cwd.to_path_buf();
    loop {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git.join("HEAD"));
        }
        if dot_git.is_file() {
            // Worktree `.git` file: "gitdir: <path>"
            let content = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = content.trim().strip_prefix("gitdir: ")?;
            let gitdir_path = if Path::new(gitdir).is_absolute() {
                std::path::PathBuf::from(gitdir)
            } else {
                dir.join(gitdir)
            };
            return Some(gitdir_path.join("HEAD"));
        }
        if dir == workspace_root || !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_current_branch_from_regular_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("worktrees").join("feature-auth").join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        // Create a .git directory with HEAD
        let git_dir = pkg_dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/feature-auth\n").unwrap();

        let branch = read_current_branch(tmp.path(), &pkg_dir);
        assert_eq!(branch, Some("feature-auth".to_string()));
    }

    #[test]
    fn test_read_current_branch_detached_head() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("worktrees").join("feature-x").join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let git_dir = pkg_dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "abc123def456\n").unwrap();

        let branch = read_current_branch(tmp.path(), &pkg_dir);
        assert_eq!(branch, None);
    }

    #[test]
    fn test_read_current_branch_from_gitdir_file() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("worktrees").join("feature-x").join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        // Create a real git dir somewhere else
        let real_git = tmp.path().join("real-git-dir");
        std::fs::create_dir_all(&real_git).unwrap();
        std::fs::write(real_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        // .git file pointing to it
        std::fs::write(
            pkg_dir.join(".git"),
            format!("gitdir: {}", real_git.display()),
        )
        .unwrap();

        let branch = read_current_branch(tmp.path(), &pkg_dir);
        assert_eq!(branch, Some("main".to_string()));
    }

    #[test]
    fn test_no_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("worktrees").join("feature-x").join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let branch = read_current_branch(tmp.path(), &pkg_dir);
        assert_eq!(branch, None);
    }

    #[test]
    fn test_run_matching_branch_no_output() {
        let tmp = tempfile::tempdir().unwrap();
        let branch_dir = tmp.path().join("worktrees").join("feature-auth");
        let pkg_dir = branch_dir.join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let git_dir = pkg_dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/feature-auth\n").unwrap();

        // This should produce no output (matching branch)
        run(tmp.path(), &pkg_dir);
    }

    #[test]
    fn test_not_in_worktree_silent() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp.path().join("packages").join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        // Should produce no output
        run(tmp.path(), &pkg_dir);
    }

    #[test]
    fn test_branch_with_slashes() {
        // Branch fm/whatever -> dir fm-whatever
        let tmp = tempfile::tempdir().unwrap();
        let pkg_dir = tmp
            .path()
            .join("worktrees")
            .join("fm-whatever")
            .join("pkg");
        std::fs::create_dir_all(&pkg_dir).unwrap();

        let git_dir = pkg_dir.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/fm/whatever\n").unwrap();

        // fm/whatever sanitizes to fm-whatever, which matches the dir — no warning
        run(tmp.path(), &pkg_dir);
    }
}
