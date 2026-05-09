//! Docker-based integration tests that exercise meldr against real GitHub repos.
//!
//! These tests run inside a Docker container (see `Dockerfile.test`) that
//! pre-clones well-known repos to `/test-repos/`. Each test copies repos to
//! its own temp dir so mutations are fully isolated and tests run in parallel.
//!
//! Repos used (all official GitHub octocat repos — guaranteed public & stable):
//!   - octocat/Hello-World       (GitHub's official test repo)
//!   - octocat/Spoon-Knife       (GitHub's fork demo repo)
//!   - octocat/git-consortium    (small test repo)
//!   - octocat/boysenberry-repo-1 (small test repo)
//!
//! Run via: ./run-docker-tests.sh
//!
//! Gated behind the `docker-tests` Cargo feature so a bare `cargo test` on the
//! host doesn't try to run them (MELDR_TEST_REPOS only exists inside the image).

#![cfg(feature = "docker-tests")]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use tempfile::TempDir;

// ─── Test infrastructure ──────────────────────────────────────────────────────

/// Directory containing pre-cloned bare repos (set in Dockerfile).
fn test_repos_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("MELDR_TEST_REPOS")
            .unwrap_or_else(|_| panic!("MELDR_TEST_REPOS env var not set — run via Docker")),
    )
}

/// Copy a pre-cloned bare repo to a temp dir, returning the path to the copy.
/// This gives each test an isolated "upstream" it can push to.
fn copy_repo(target_dir: &Path, repo_name: &str) -> String {
    let src = test_repos_dir().join(format!("{repo_name}.git"));
    assert!(src.exists(), "Pre-cloned repo not found: {src:?}");

    let dest = target_dir.join(format!("{repo_name}.git"));
    copy_dir_recursive(&src, &dest);

    dest.to_str().unwrap().to_string()
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dest_path);
        } else {
            fs::copy(&src_path, &dest_path).unwrap();
        }
    }
}

#[allow(deprecated)]
fn meldr() -> Command {
    Command::cargo_bin("meldr").unwrap()
}

fn meldr_with_home(home: &Path) -> Command {
    let mut cmd = meldr();
    cmd.env("HOME", home);
    cmd
}

fn init_workspace(dir: &Path) {
    meldr()
        .args(["init", "--name", "test-ws"])
        .current_dir(dir)
        .assert()
        .success();
}

/// Push a commit to a bare repo on its default branch.
fn push_commit_to_bare(bare_repo: &str, filename: &str, content: &str) {
    let bare_path = Path::new(bare_repo);
    let tmp_clone = bare_path.parent().unwrap().join(format!(
        "{}-push-tmp",
        bare_path.file_name().unwrap().to_str().unwrap()
    ));

    let _ = fs::remove_dir_all(&tmp_clone);

    process::Command::new("git")
        .args(["clone", bare_repo, tmp_clone.to_str().unwrap()])
        .output()
        .unwrap();

    process::Command::new("git")
        .args(["config", "user.email", "test@meldr.dev"])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["config", "user.name", "Meldr Test"])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();

    fs::write(tmp_clone.join(filename), content).unwrap();

    process::Command::new("git")
        .args(["add", "."])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["commit", "-m", &format!("add {filename}")])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["push"])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();

    fs::remove_dir_all(&tmp_clone).unwrap();
}

/// Get HEAD SHA of a git repo.
fn git_head(path: &Path) -> String {
    let out = process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

// ─── 1. PACKAGE MANAGEMENT WITH REAL REPOS ────────────────────────────────────

#[test]
fn test_add_single_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Added package"));

    // Verify the bare clone exists with proper structure
    let pkg_path = tmp.path().join("packages/spoon-knife");
    assert!(pkg_path.exists(), "Package dir should exist");

    // Verify it's a bare git repo
    assert!(
        pkg_path.join("HEAD").exists() || pkg_path.join(".git").exists(),
        "Should be a git repo"
    );
}

#[test]
fn test_add_multiple_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");
    let repo3 = copy_repo(repos.path(), "boysenberry-repo-1");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2, &repo3])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("spoon-knife")
                .and(predicate::str::contains("git-consortium"))
                .and(predicate::str::contains("boysenberry-repo-1")),
        );

    // Verify each package dir exists on disk
    for pkg in &["spoon-knife", "git-consortium", "boysenberry-repo-1"] {
        let pkg_path = tmp.path().join(format!("packages/{pkg}"));
        assert!(
            pkg_path.exists(),
            "Package directory should exist on disk: {pkg_path:?}"
        );
    }

    // All packages should be listed
    meldr()
        .args(["package", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("spoon-knife")
                .and(predicate::str::contains("git-consortium"))
                .and(predicate::str::contains("boysenberry-repo-1")),
        );
}

#[test]
fn test_add_then_remove_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert!(tmp.path().join("packages/spoon-knife").exists());

    meldr()
        .args(["package", "remove", "spoon-knife"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed package 'spoon-knife'"));

    assert!(!tmp.path().join("packages/spoon-knife").exists());
}

#[test]
fn test_add_same_repo_twice_fails() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("already exists, skipping"))
        .stdout(predicate::str::contains("No packages were added"));
}

#[test]
fn test_add_and_remove_multiple_then_readd() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["package", "remove", "spoon-knife"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Should be able to re-add the removed package
    meldr()
        .args(["package", "add", &repo1])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Added package"));

    // Both should be listed
    meldr()
        .args(["package", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("spoon-knife").and(predicate::str::contains("git-consortium")),
        );
}

// ─── 2. WORKTREE OPERATIONS WITH REAL REPOS ──────────────────────────────────

#[test]
fn test_worktree_add_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-xyz"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created worktree 'feature-xyz'"));

    // Both packages should have worktrees
    assert!(
        tmp.path()
            .join("worktrees/feature-xyz/spoon-knife")
            .exists()
    );
    assert!(
        tmp.path()
            .join("worktrees/feature-xyz/git-consortium")
            .exists()
    );

    // Worktrees should contain actual checked-out files
    let wt_pos = tmp.path().join("worktrees/feature-xyz/spoon-knife");
    let entries: Vec<_> = fs::read_dir(&wt_pos)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "Worktree should contain checked-out files"
    );

    // Each worktree directory should have a .git file (worktree link)
    assert!(
        wt_pos.join(".git").exists(),
        "Worktree should have a .git file/directory"
    );
    let wt_gc = tmp.path().join("worktrees/feature-xyz/git-consortium");
    assert!(
        wt_gc.join(".git").exists(),
        "Worktree should have a .git file/directory"
    );
}

#[test]
fn test_worktree_add_multiple_branches() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create multiple worktrees
    for branch in &["feature-a", "feature-b", "feature-c"] {
        meldr()
            .args(["--no-tabs", "worktree", "add", branch])
            .current_dir(tmp.path())
            .assert()
            .success();
    }

    // All should be listed
    let output = meldr()
        .args(["worktree", "list"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("feature-a"));
    assert!(stdout.contains("feature-b"));
    assert!(stdout.contains("feature-c"));
}

#[test]
fn test_worktree_remove_cleans_up_files() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "temp-branch"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/temp-branch");
    let wt_pkg_dir = wt_dir.join("spoon-knife");
    assert!(wt_dir.exists(), "Worktree dir should exist before remove");
    assert!(
        wt_pkg_dir.exists(),
        "Worktree package subdir should exist before remove"
    );
    assert!(
        wt_pkg_dir.join(".git").exists(),
        "Worktree should have .git before remove"
    );

    meldr()
        .args(["--no-tabs", "worktree", "remove", "temp-branch"])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert!(!wt_dir.exists(), "Worktree directory should be cleaned up");
    assert!(
        !wt_pkg_dir.exists(),
        "Worktree package subdir should also be gone"
    );

    // Verify worktree no longer appears in listing
    meldr()
        .args(["worktree", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("temp-branch").not());
}

#[test]
fn test_worktree_with_slash_branch_on_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "user/feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Slash should be sanitized in directory names
    assert!(
        tmp.path()
            .join("worktrees/user-feature-test/spoon-knife")
            .exists()
    );
    assert!(
        tmp.path()
            .join("worktrees/user-feature-test/git-consortium")
            .exists()
    );
    assert!(!tmp.path().join("worktrees/user/feature-test").exists());
}

#[test]
fn test_worktree_add_then_add_package_backfills() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    // Add first package and create worktree
    meldr()
        .args(["package", "add", &repo1])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-backfill"])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert!(
        tmp.path()
            .join("worktrees/feature-backfill/spoon-knife")
            .exists()
    );
    assert!(
        !tmp.path()
            .join("worktrees/feature-backfill/git-consortium")
            .exists()
    );

    // Adding second package should backfill existing worktrees
    meldr()
        .args(["package", "add", &repo2])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Created worktree for 'git-consortium' on branch 'feature-backfill'",
        ));

    assert!(
        tmp.path()
            .join("worktrees/feature-backfill/git-consortium")
            .exists()
    );
}

// ─── 3. FULL LIFECYCLE WITH REAL REPOS ────────────────────────────────────────

#[test]
fn test_create_workspace_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    meldr()
        .args([
            "create",
            "my-workspace",
            "-r",
            &repo1,
            "-r",
            &repo2,
            "--no-tabs",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Created workspace 'my-workspace'")
                .and(predicate::str::contains("spoon-knife"))
                .and(predicate::str::contains("git-consortium")),
        );

    let ws = tmp.path().join("my-workspace");
    assert!(ws.join("meldr.toml").exists());
    assert!(ws.join("packages/spoon-knife").exists());
    assert!(ws.join("packages/git-consortium").exists());
}

#[test]
fn test_create_with_repos_and_branch() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "boysenberry-repo-1");

    meldr()
        .args([
            "create",
            "ws-with-branch",
            "-r",
            &repo1,
            "-r",
            &repo2,
            "-b",
            "dev-branch",
            "--no-tabs",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Created workspace")
                .and(predicate::str::contains("Created worktree 'dev-branch'")),
        );

    assert!(
        tmp.path()
            .join("ws-with-branch/worktrees/dev-branch/spoon-knife")
            .exists()
    );
    assert!(
        tmp.path()
            .join("ws-with-branch/worktrees/dev-branch/boysenberry-repo-1")
            .exists()
    );
}

#[test]
fn test_full_lifecycle_create_add_sync_remove() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    // 1. Create workspace with one repo
    meldr()
        .args(["create", "lifecycle-ws", "-r", &repo1, "--no-tabs"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let ws = tmp.path().join("lifecycle-ws");

    // Verify workspace structure after create
    assert!(
        ws.join("meldr.toml").exists(),
        "meldr.toml should exist after create"
    );
    assert!(
        ws.join("packages").exists(),
        "packages dir should exist after create"
    );
    assert!(
        ws.join("packages/spoon-knife").exists(),
        "First package should exist after create"
    );

    // 2. Add another package
    meldr()
        .args(["package", "add", &repo2])
        .current_dir(&ws)
        .assert()
        .success();

    assert!(
        ws.join("packages/git-consortium").exists(),
        "Second package should exist after add"
    );

    // 3. Create worktree
    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-lifecycle"])
        .current_dir(&ws)
        .assert()
        .success();

    // 4. Verify both packages have worktrees with .git
    let wt_sk = ws.join("worktrees/feature-lifecycle/spoon-knife");
    let wt_gc = ws.join("worktrees/feature-lifecycle/git-consortium");
    assert!(wt_sk.exists());
    assert!(wt_gc.exists());
    assert!(
        wt_sk.join(".git").exists(),
        "spoon-knife worktree should have .git"
    );
    assert!(
        wt_gc.join(".git").exists(),
        "git-consortium worktree should have .git"
    );

    // 5. Push upstream change and sync
    push_commit_to_bare(&repo1, "lifecycle-change.txt", "lifecycle content");

    let head_before = git_head(&wt_sk);

    meldr()
        .args(["--no-tabs", "sync", "feature-lifecycle"])
        .current_dir(&ws)
        .assert()
        .success();

    // 6. Verify the file appeared and HEAD changed
    assert!(
        ws.join("worktrees/feature-lifecycle/spoon-knife/lifecycle-change.txt")
            .exists()
    );
    let head_after = git_head(&wt_sk);
    assert_ne!(head_before, head_after, "HEAD should change after sync");

    // 7. Remove a package
    meldr()
        .args(["package", "remove", "git-consortium"])
        .current_dir(&ws)
        .assert()
        .success();

    // 8. Worktree should still have the other package; removed package dir should be gone
    assert!(ws.join("worktrees/feature-lifecycle/spoon-knife").exists());
    assert!(
        !ws.join("packages/git-consortium").exists(),
        "Removed package should be gone from packages/"
    );

    // 9. Remove the worktree
    meldr()
        .args(["--no-tabs", "worktree", "remove", "feature-lifecycle"])
        .current_dir(&ws)
        .assert()
        .success();

    assert!(
        !ws.join("worktrees/feature-lifecycle").exists(),
        "Worktree dir should be gone after remove"
    );

    // Verify workspace is still functional (can list)
    meldr()
        .args(["worktree", "list"])
        .current_dir(&ws)
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-lifecycle").not());
}

// ─── 4. SYNC OPERATIONS WITH REAL REPOS ───────────────────────────────────────

#[test]
fn test_sync_real_repos_basic() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-basic"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/sync-basic/spoon-knife");
    let head_before = git_head(&wt);

    // Push upstream change
    push_commit_to_bare(&repo, "synced-file.txt", "synced content");

    // Sync should pull the change
    meldr()
        .args(["--no-tabs", "sync", "sync-basic"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    // HEAD SHA should have changed after sync
    let head_after = git_head(&wt);
    assert_ne!(
        head_before, head_after,
        "HEAD should have changed after syncing upstream changes"
    );

    // File should now exist in worktree
    assert!(
        tmp.path()
            .join("worktrees/sync-basic/spoon-knife/synced-file.txt")
            .exists()
    );
}

#[test]
fn test_sync_multiple_repos_simultaneously() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-multi"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Push changes to both repos
    push_commit_to_bare(&repo1, "multi-1.txt", "change 1");
    push_commit_to_bare(&repo2, "multi-2.txt", "change 2");

    meldr()
        .args(["--no-tabs", "sync", "sync-multi"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("spoon-knife").and(predicate::str::contains("git-consortium")),
        );

    // Both files should exist
    assert!(
        tmp.path()
            .join("worktrees/sync-multi/spoon-knife/multi-1.txt")
            .exists()
    );
    assert!(
        tmp.path()
            .join("worktrees/sync-multi/git-consortium/multi-2.txt")
            .exists()
    );
}

#[test]
fn test_sync_dry_run_doesnt_change_worktree() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-dry"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/sync-dry/spoon-knife");
    let head_before = git_head(&wt);

    push_commit_to_bare(&repo, "dry-run.txt", "should not appear");

    meldr()
        .args(["--no-tabs", "sync", "sync-dry", "--dry-run"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    let head_after = git_head(&wt);
    assert_eq!(head_before, head_after, "Dry run should not change HEAD");
    assert!(
        !wt.join("dry-run.txt").exists(),
        "Dry run should not create files"
    );
}

#[test]
fn test_sync_up_to_date_no_changes() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-uptodate"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Sync without pushing anything upstream
    meldr()
        .args(["--no-tabs", "sync", "sync-uptodate"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("up-to-date"));
}

#[test]
fn test_sync_all_multiple_worktrees_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create two worktrees
    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-x"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-y"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "all-sync.txt", "sync all content");

    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("branch-x").and(predicate::str::contains("branch-y")));
}

#[test]
fn test_sync_with_strategy_theirs_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-theirs"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/sync-theirs/spoon-knife");
    let head_before = git_head(&wt);

    push_commit_to_bare(&repo, "theirs.txt", "theirs strategy content");

    meldr()
        .args(["--no-tabs", "sync", "sync-theirs", "--strategy", "theirs"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    // HEAD SHA should have changed after sync
    let head_after = git_head(&wt);
    assert_ne!(
        head_before, head_after,
        "HEAD should have changed after syncing with theirs strategy"
    );

    // The synced file should exist in the worktree
    assert!(
        wt.join("theirs.txt").exists(),
        "Synced file should exist after theirs strategy sync"
    );
}

#[test]
fn test_sync_only_filter_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-only"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo1, "only-pos.txt", "only positive");
    push_commit_to_bare(&repo2, "only-neg.txt", "only negative");

    let wt_neg = tmp.path().join("worktrees/sync-only/git-consortium");
    let head_neg_before = git_head(&wt_neg);

    // Only sync spoon-knife
    meldr()
        .args(["--no-tabs", "sync", "sync-only", "--only", "spoon-knife"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // spoon-knife should have the file
    assert!(
        tmp.path()
            .join("worktrees/sync-only/spoon-knife/only-pos.txt")
            .exists()
    );

    // git-consortium should NOT have been synced
    let head_neg_after = git_head(&wt_neg);
    assert_eq!(
        head_neg_before, head_neg_after,
        "git-consortium should not have been synced"
    );
}

#[test]
fn test_sync_exclude_filter_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-excl"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo1, "excl-pos.txt", "exclude positive");
    push_commit_to_bare(&repo2, "excl-neg.txt", "exclude negative");

    let wt_pos = tmp.path().join("worktrees/sync-excl/spoon-knife");
    let head_pos_before = git_head(&wt_pos);

    // Exclude spoon-knife
    meldr()
        .args(["--no-tabs", "sync", "sync-excl", "--exclude", "spoon-knife"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // spoon-knife should NOT have been synced
    let head_pos_after = git_head(&wt_pos);
    assert_eq!(
        head_pos_before, head_pos_after,
        "spoon-knife should have been excluded"
    );

    // git-consortium SHOULD have the file
    assert!(
        tmp.path()
            .join("worktrees/sync-excl/git-consortium/excl-neg.txt")
            .exists()
    );
}

#[test]
fn test_sync_undo_with_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-undo"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/sync-undo/spoon-knife");
    let head_before_sync = git_head(&wt);

    push_commit_to_bare(&repo, "undo-file.txt", "will be undone");

    meldr()
        .args(["--no-tabs", "sync", "sync-undo"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // File should exist after sync
    assert!(wt.join("undo-file.txt").exists());

    // Undo
    meldr()
        .args(["--no-tabs", "sync", "sync-undo", "--undo"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Undoing"));

    // HEAD should be back to before sync
    let head_after_undo = git_head(&wt);
    assert_eq!(
        head_before_sync, head_after_undo,
        "Undo should restore HEAD"
    );
}

#[test]
fn test_sync_creates_snapshot_with_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "snap-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "snap.txt", "snapshot content");

    meldr()
        .args(["--no-tabs", "sync", "snap-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let snapshot_dir = tmp.path().join(".meldr/sync-snapshots");
    assert!(snapshot_dir.exists(), "Snapshots dir should exist");

    let entries: Vec<_> = fs::read_dir(&snapshot_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!entries.is_empty(), "Should have at least one snapshot");
}

#[test]
fn test_sync_multiple_commits_upstream() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "multi-commit"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Push multiple commits
    for i in 1..=5 {
        push_commit_to_bare(&repo, &format!("multi-{i}.txt"), &format!("content {i}"));
    }

    meldr()
        .args(["--no-tabs", "sync", "multi-commit"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    // All 5 files should exist
    let wt = tmp.path().join("worktrees/multi-commit/spoon-knife");
    for i in 1..=5 {
        assert!(
            wt.join(format!("multi-{i}.txt")).exists(),
            "File multi-{i}.txt should exist after sync"
        );
    }
}

// ─── 5. EXEC OPERATIONS WITH REAL REPOS ───────────────────────────────────────

#[test]
fn test_exec_across_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "exec-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/exec-test/spoon-knife");

    meldr()
        .args(["exec", "echo", "hello-from-exec"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[spoon-knife] hello-from-exec")
                .and(predicate::str::contains("[git-consortium] hello-from-exec")),
        );

    // Verify exec runs in the correct worktree directory by checking pwd
    let output = meldr()
        .args(["exec", "pwd"])
        .current_dir(&wt_dir)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("worktrees/exec-test/spoon-knife"),
        "pwd output should contain the worktree path for spoon-knife, got: {stdout}"
    );
    assert!(
        stdout.contains("worktrees/exec-test/git-consortium"),
        "pwd output should contain the worktree path for git-consortium, got: {stdout}"
    );
}

#[test]
fn test_exec_ls_shows_real_repo_files() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "exec-ls"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/exec-ls/spoon-knife");

    // ls should show real files from the repo
    let output = meldr()
        .args(["exec", "ls"])
        .current_dir(&wt_dir)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // spoon-knife is an npm package, should have package.json
    assert!(
        stdout.contains("package.json") || stdout.contains("index.js") || stdout.contains("README"),
        "Should list real repo files, got: {stdout}"
    );
}

#[test]
fn test_exec_git_status_in_worktree() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "exec-git"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/exec-git/spoon-knife");

    // git status should work inside the worktree
    meldr()
        .args(["exec", "git", "status"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("On branch"));
}

#[test]
fn test_exec_git_log_shows_real_history() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "exec-log"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/exec-log/spoon-knife");

    // git log should show real commit history
    let output = meldr()
        .args(["exec", "git", "log", "--oneline", "-5"])
        .current_dir(&wt_dir)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    // Should have at least one commit hash
    assert!(
        stdout.len() > 10,
        "Should have real commit history, got: {stdout}"
    );
}

#[test]
fn test_exec_from_worktree_root() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "exec-root"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_root = tmp.path().join("worktrees/exec-root");

    meldr()
        .args(["exec", "echo", "from-root"])
        .current_dir(&wt_root)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[spoon-knife] from-root")
                .and(predicate::str::contains("[git-consortium] from-root")),
        );
}

// ─── 6. STATUS & STALENESS WITH REAL REPOS ────────────────────────────────────

#[test]
fn test_status_shows_real_packages() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");
    let repo3 = copy_repo(repos.path(), "boysenberry-repo-1");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2, &repo3])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create a worktree so status shows branch info
    meldr()
        .args(["--no-tabs", "worktree", "add", "status-branch"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let output = meldr()
        .arg("status")
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);

    // Verify workspace and package info
    assert!(
        stdout.contains("Workspace:"),
        "Should show workspace header"
    );
    assert!(stdout.contains("Packages:"), "Should show packages header");
    assert!(stdout.contains("spoon-knife"), "Should list spoon-knife");
    assert!(
        stdout.contains("git-consortium"),
        "Should list git-consortium"
    );
    assert!(
        stdout.contains("boysenberry-repo-1"),
        "Should list boysenberry-repo-1"
    );

    // Verify worktree/branch info is shown
    assert!(
        stdout.contains("status-branch") || stdout.contains("Worktree"),
        "Status should show worktree or branch info, got: {stdout}"
    );
}

#[test]
fn test_status_staleness_with_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "stale-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // No upstream changes: status should not warn
    meldr()
        .arg("status")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("behind").not());

    // Push upstream change and sync first to get refs updated
    push_commit_to_bare(&repo, "stale-1.txt", "first");
    meldr()
        .args(["sync", "stale-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Now push another commit and fetch (but don't sync) to create staleness
    push_commit_to_bare(&repo, "stale-2.txt", "second");

    let pkg_path = tmp.path().join("packages/spoon-knife");
    process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();

    // Status should now warn about being behind
    meldr()
        .arg("status")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("behind"));
}

// ─── 7. CONFIG WITH REAL REPO WORKSPACE ───────────────────────────────────────

#[test]
fn test_config_in_workspace_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    meldr_with_home(home.path())
        .args(["create", "config-ws", "-r", &repo, "--no-tabs"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let ws = tmp.path().join("config-ws");

    // Set and verify various config keys
    for (key, value) in &[
        ("agent", "cursor"),
        ("editor", "code ."),
        ("shell", "bash"),
        ("default_branch", "develop"),
    ] {
        meldr_with_home(home.path())
            .args(["config", "set", key, value])
            .current_dir(&ws)
            .assert()
            .success();

        meldr_with_home(home.path())
            .args(["config", "get", key])
            .current_dir(&ws)
            .assert()
            .success()
            .stdout(predicate::str::contains(*value));
    }

    // Config show should work
    meldr_with_home(home.path())
        .args(["config", "show"])
        .current_dir(&ws)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("agent = cursor")
                .and(predicate::str::contains("editor = code ."))
                .and(predicate::str::contains("shell = bash")),
        );

    // Verify the config file on disk contains expected values
    let toml_content = fs::read_to_string(ws.join("meldr.toml")).unwrap();
    assert!(
        toml_content.contains("cursor"),
        "meldr.toml should contain 'cursor' after config set, got: {toml_content}"
    );
    assert!(
        toml_content.contains("bash"),
        "meldr.toml should contain 'bash' after config set, got: {toml_content}"
    );
    assert!(
        toml_content.contains("develop"),
        "meldr.toml should contain 'develop' after config set, got: {toml_content}"
    );
}

#[test]
fn test_config_global_and_workspace_precedence_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    meldr_with_home(home.path())
        .args(["create", "prec-ws", "-r", &repo, "--no-tabs"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let ws = tmp.path().join("prec-ws");

    // Set global
    meldr_with_home(home.path())
        .args(["config", "set", "--global", "editor", "vim"])
        .assert()
        .success();

    // Set workspace (should override)
    meldr_with_home(home.path())
        .args(["config", "set", "editor", "nvim ."])
        .current_dir(&ws)
        .assert()
        .success();

    // Show should report workspace source
    meldr_with_home(home.path())
        .args(["config", "show"])
        .current_dir(&ws)
        .assert()
        .success()
        .stdout(predicate::str::contains("editor = nvim . (workspace)"));

    // Unset workspace, global should show through
    meldr_with_home(home.path())
        .args(["config", "unset", "editor"])
        .current_dir(&ws)
        .assert()
        .success();

    meldr_with_home(home.path())
        .args(["config", "show"])
        .current_dir(&ws)
        .assert()
        .success()
        .stdout(predicate::str::contains("editor = vim (global)"));
}

// ─── 8. STRESS TESTS ─────────────────────────────────────────────────────────

#[test]
fn test_many_worktrees_on_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create 10 worktrees
    for i in 0..10 {
        meldr()
            .args(["--no-tabs", "worktree", "add", &format!("stress-{i}")])
            .current_dir(tmp.path())
            .assert()
            .success();
    }

    // List should show all 10
    let output = meldr()
        .args(["worktree", "list"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    for i in 0..10 {
        assert!(
            stdout.contains(&format!("stress-{i}")),
            "Should list stress-{i}"
        );
    }

    // Remove them all
    for i in 0..10 {
        meldr()
            .args(["--no-tabs", "worktree", "remove", &format!("stress-{i}")])
            .current_dir(tmp.path())
            .assert()
            .success();
    }

    // Should be empty now
    meldr()
        .args(["worktree", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No active worktrees"));
}

#[test]
fn test_all_four_repos_workspace() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "hello-world");
    let repo2 = copy_repo(repos.path(), "spoon-knife");
    let repo3 = copy_repo(repos.path(), "git-consortium");
    let repo4 = copy_repo(repos.path(), "boysenberry-repo-1");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2, &repo3, &repo4])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "four-repos"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // All 4 repos should have worktrees
    for name in &[
        "hello-world",
        "spoon-knife",
        "git-consortium",
        "boysenberry-repo-1",
    ] {
        assert!(
            tmp.path()
                .join(format!("worktrees/four-repos/{name}"))
                .exists(),
            "Worktree for {name} should exist"
        );
    }

    // Status should list all 4
    meldr()
        .arg("status")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("hello-world")
                .and(predicate::str::contains("spoon-knife"))
                .and(predicate::str::contains("git-consortium"))
                .and(predicate::str::contains("boysenberry-repo-1")),
        );

    // Exec should run across all 4
    let output = meldr()
        .args(["exec", "echo", "all-four"])
        .current_dir(tmp.path().join("worktrees/four-repos/spoon-knife"))
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    for name in &[
        "hello-world",
        "spoon-knife",
        "git-consortium",
        "boysenberry-repo-1",
    ] {
        assert!(
            stdout.contains(&format!("[{name}] all-four")),
            "Exec output should contain [{name}] all-four"
        );
    }
}

#[test]
fn test_sync_all_four_repos_with_upstream_changes() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");
    let repo3 = copy_repo(repos.path(), "boysenberry-repo-1");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2, &repo3])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-three"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Push changes to all 3
    push_commit_to_bare(&repo1, "three-1.txt", "change 1");
    push_commit_to_bare(&repo2, "three-2.txt", "change 2");
    push_commit_to_bare(&repo3, "three-3.txt", "change 3");

    meldr()
        .args(["--no-tabs", "sync", "sync-three"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("spoon-knife")
                .and(predicate::str::contains("git-consortium"))
                .and(predicate::str::contains("boysenberry-repo-1")),
        );

    // All files should exist
    let base = tmp.path().join("worktrees/sync-three");
    assert!(base.join("spoon-knife/three-1.txt").exists());
    assert!(base.join("git-consortium/three-2.txt").exists());
    assert!(base.join("boysenberry-repo-1/three-3.txt").exists());
}

// ─── 9. EDGE CASES ───────────────────────────────────────────────────────────

#[test]
fn test_add_remove_add_sync_cycle() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    // Add, create worktree, sync
    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "cycle-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "cycle-1.txt", "first cycle");

    meldr()
        .args(["--no-tabs", "sync", "cycle-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Remove worktree
    meldr()
        .args(["--no-tabs", "worktree", "remove", "cycle-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Remove package
    meldr()
        .args(["package", "remove", "spoon-knife"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Re-add package
    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create new worktree
    meldr()
        .args(["--no-tabs", "worktree", "add", "cycle-test-2"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Sync should work (repo should have the commit from before)
    meldr()
        .args(["--no-tabs", "sync", "cycle-test-2"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // The file from the first cycle should exist (it's in the repo history)
    assert!(
        tmp.path()
            .join("worktrees/cycle-test-2/spoon-knife/cycle-1.txt")
            .exists()
    );
}

#[test]
fn test_worktree_isolation_between_branches() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-iso-a"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-iso-b"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Write a file in branch-a's worktree
    let wt_a = tmp.path().join("worktrees/branch-iso-a/spoon-knife");
    let wt_b = tmp.path().join("worktrees/branch-iso-b/spoon-knife");

    fs::write(wt_a.join("branch-a-only.txt"), "only in a").unwrap();

    // Branch B should NOT have this file
    assert!(
        !wt_b.join("branch-a-only.txt").exists(),
        "Branch B should not see branch A's uncommitted files"
    );
}

#[test]
fn test_exec_fails_outside_worktree_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Running exec from workspace root should fail
    meldr()
        .args(["exec", "echo", "fail"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "meldr exec must be run from within a worktree",
        ));
}

#[test]
fn test_worktree_auto_detect_remove_from_cwd() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "auto-rm-real"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/auto-rm-real/spoon-knife");
    assert!(wt_dir.exists());

    // Remove from inside worktree (no branch arg)
    meldr()
        .args(["--no-tabs", "worktree", "remove"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed worktree 'auto-rm-real'"));
}

#[test]
fn test_sync_from_within_worktree_dir() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "sync-from-wt"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "from-wt.txt", "synced from worktree");

    let wt_dir = tmp.path().join("worktrees/sync-from-wt/spoon-knife");

    // Sync from within the worktree directory (auto-detect branch)
    meldr()
        .args(["--no-tabs", "sync"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    assert!(wt_dir.join("from-wt.txt").exists());
}

// ─── 10. REPO METADATA VERIFICATION ──────────────────────────────────────────

#[test]
fn test_bare_clone_has_remote_tracking_refs_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    let pkg_path = tmp.path().join("packages/spoon-knife");

    // Verify fetch refspec
    let refspec = process::Command::new("git")
        .args(["config", "--get-all", "remote.origin.fetch"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();
    let refspec_str = String::from_utf8_lossy(&refspec.stdout);
    assert!(
        refspec_str.contains("+refs/heads/*:refs/remotes/origin/*"),
        "Should have fetch refspec, got: {refspec_str}"
    );

    // Verify remote HEAD is set
    let head_ref = process::Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();
    assert!(
        head_ref.status.success(),
        "refs/remotes/origin/HEAD should be set"
    );
}

#[test]
fn test_hello_world_repo_has_known_structure() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "hello-world");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "hw-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/hw-test/hello-world");
    assert!(wt.exists(), "Hello-World worktree should exist");

    // Hello-World repo should have a README
    assert!(
        wt.join("README").exists() || wt.join("README.md").exists(),
        "Hello-World should have a README"
    );
}

// ─── 11. SYNC WITH MERGE MODE ────────────────────────────────────────────────

#[test]
fn test_sync_merge_mode_real_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "merge-mode"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/merge-mode/spoon-knife");
    let head_before = git_head(&wt);

    push_commit_to_bare(&repo, "merge-mode.txt", "merge mode content");

    meldr()
        .args(["--no-tabs", "sync", "merge-mode", "--merge"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // HEAD SHA should have changed after sync
    let head_after = git_head(&wt);
    assert_ne!(
        head_before, head_after,
        "HEAD should have changed after merge-mode sync"
    );

    // File should exist after merge
    assert!(
        tmp.path()
            .join("worktrees/merge-mode/spoon-knife/merge-mode.txt")
            .exists()
    );

    // Verify file content is correct
    let content = fs::read_to_string(wt.join("merge-mode.txt")).unwrap();
    assert_eq!(
        content, "merge mode content",
        "Synced file should have expected content"
    );
}

// ─── 12. CONCURRENT SYNC SAFETY ──────────────────────────────────────────────

#[test]
fn test_sync_all_no_worktrees_fetches_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    // --all with no worktrees should succeed (fetch-only mode)
    meldr()
        .args(["sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "No active worktrees. All packages fetched and main updated.",
        ));
}

// ─── 13. MELDR.TOML VERIFICATION WITH REAL REPOS ─────────────────────────────

#[test]
fn test_meldr_toml_reflects_real_repo_urls() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    let content = fs::read_to_string(tmp.path().join("meldr.toml")).unwrap();

    // meldr.toml should have package entries for both repos
    assert!(
        content.contains("[[package]]"),
        "Should have package entries"
    );
    assert!(
        content.contains("spoon-knife"),
        "Should contain spoon-knife"
    );
    assert!(
        content.contains("git-consortium"),
        "Should contain git-consortium"
    );
}

#[test]
fn test_meldr_toml_updated_after_remove() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["package", "remove", "spoon-knife"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let content = fs::read_to_string(tmp.path().join("meldr.toml")).unwrap();
    assert!(
        !content.contains("spoon-knife"),
        "Removed package should not be in meldr.toml"
    );
    assert!(
        content.contains("git-consortium"),
        "Remaining package should still be in meldr.toml"
    );
}

// ─── 14. WORKTREE FILE CONTENT VERIFICATION ──────────────────────────────────

#[test]
fn test_worktree_files_match_repo_content() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "content-check"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/content-check/spoon-knife");

    // Verify the worktree is on the correct branch
    let out = process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&wt)
        .output()
        .unwrap();
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(branch, "content-check", "Should be on the created branch");

    // Verify git status is clean
    let out = process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&wt)
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(status.is_empty(), "Worktree should have clean status");
}

#[test]
fn test_synced_files_have_correct_content() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "content-sync"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let expected_content = "hello from meldr test!\nline two\n";
    push_commit_to_bare(&repo, "content-verify.txt", expected_content);

    meldr()
        .args(["--no-tabs", "sync", "content-sync"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/content-sync/spoon-knife");
    let actual = fs::read_to_string(wt.join("content-verify.txt")).unwrap();
    assert_eq!(
        actual, expected_content,
        "Synced file should have correct content"
    );
}

// ─── 15. SYNC LOG VERIFICATION ───────────────────────────────────────────────

#[test]
fn test_sync_log_written_after_sync() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "log-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "log-file.txt", "log test");

    meldr()
        .args(["--no-tabs", "sync", "log-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let log_file = tmp.path().join(".meldr/sync-log.jsonl");
    assert!(log_file.exists(), "Sync log should exist");

    let content = fs::read_to_string(&log_file).unwrap();
    assert!(!content.is_empty(), "Sync log should not be empty");
    assert!(
        content.contains("log-test"),
        "Sync log should reference the branch"
    );
}

// ─── 16. ALIAS COMMANDS WITH REAL REPOS ───────────────────────────────────────

#[test]
fn test_pkg_alias_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["pkg", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Added package"));

    meldr()
        .args(["pkg", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("spoon-knife"));

    meldr()
        .args(["pkg", "remove", "spoon-knife"])
        .current_dir(tmp.path())
        .assert()
        .success();
}

#[test]
fn test_wt_alias_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["pkg", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "wt", "add", "alias-wt"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["wt", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("alias-wt"));

    meldr()
        .args(["--no-tabs", "wt", "remove", "alias-wt"])
        .current_dir(tmp.path())
        .assert()
        .success();
}

#[test]
fn test_st_alias_with_real_repos() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["pkg", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["st"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Workspace:").and(predicate::str::contains("spoon-knife")),
        );
}

// ─── 17. SYNC EDGE CASES ─────────────────────────────────────────────────────

#[test]
fn test_sync_undo_without_snapshot_fails() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "no-snap"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Undo without ever syncing should fail with "no snapshot"
    meldr()
        .args(["--no-tabs", "sync", "no-snap", "--undo"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("No sync snapshot"));
}

#[test]
fn test_sync_mixed_outcomes_multi_repo() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "mixed-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Only push to one repo - the other stays up-to-date
    push_commit_to_bare(&repo1, "mixed.txt", "only spoon-knife changed");

    let output = meldr()
        .args(["--no-tabs", "sync", "mixed-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);

    // spoon-knife should be synced, git-consortium should be up-to-date
    assert!(stdout.contains("spoon-knife"), "Should mention spoon-knife");
    assert!(
        stdout.contains("git-consortium"),
        "Should mention git-consortium"
    );
    assert!(stdout.contains("synced"), "Should show synced status");
    assert!(
        stdout.contains("up-to-date"),
        "Should show up-to-date status"
    );
}

#[test]
fn test_sync_multiple_sequential_accumulates_snapshots() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "multi-sync"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Perform 3 sequential syncs with upstream changes each time.
    // Sleep 1s between syncs because snapshot filenames use unix seconds.
    for i in 1..=3 {
        push_commit_to_bare(
            &repo,
            &format!("seq-{i}.txt"),
            &format!("sequential sync {i}"),
        );

        meldr()
            .args(["--no-tabs", "sync", "multi-sync"])
            .current_dir(tmp.path())
            .assert()
            .success();

        if i < 3 {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    // Each sync creates a snapshot (filename is unix timestamp).
    let snapshot_dir = tmp.path().join(".meldr/sync-snapshots");
    let entries: Vec<_> = fs::read_dir(&snapshot_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        entries.len() >= 2,
        "Should have multiple snapshots after sequential syncs, got {}",
        entries.len()
    );

    // All 3 files should exist
    let wt = tmp.path().join("worktrees/multi-sync/spoon-knife");
    for i in 1..=3 {
        assert!(wt.join(format!("seq-{i}.txt")).exists());
    }
}

#[test]
fn test_sync_with_local_commits_and_upstream_changes() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "diverge-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/diverge-test/spoon-knife");

    // Make a local commit in the worktree (ahead of upstream)
    fs::write(wt.join("local-change.txt"), "local only").unwrap();
    process::Command::new("git")
        .args(["add", "."])
        .current_dir(&wt)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["commit", "-m", "local commit"])
        .current_dir(&wt)
        .output()
        .unwrap();

    // Push a non-conflicting upstream change
    push_commit_to_bare(&repo, "upstream-change.txt", "upstream only");

    // Sync should succeed (no conflicts with different files)
    meldr()
        .args(["--no-tabs", "sync", "diverge-test"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    // Both local and upstream files should exist
    assert!(wt.join("local-change.txt").exists());
    assert!(wt.join("upstream-change.txt").exists());
}

#[test]
fn test_sync_conflict_detection_safe_strategy() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "conflict-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/conflict-test/spoon-knife");

    // Record HEAD before any local changes
    let head_before = git_head(&wt);

    // Make a local commit that modifies an existing file
    fs::write(wt.join("README.md"), "local conflicting change").unwrap();
    process::Command::new("git")
        .args(["add", "."])
        .current_dir(&wt)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["commit", "-m", "local conflicting commit"])
        .current_dir(&wt)
        .output()
        .unwrap();

    let head_after_local = git_head(&wt);
    assert_ne!(
        head_before, head_after_local,
        "Local commit should change HEAD"
    );

    // Push upstream change to the SAME file
    push_commit_to_bare(&repo, "README.md", "upstream conflicting change");

    // Sync with "safe" strategy should detect the conflict and NOT force-sync
    let output = meldr()
        .args(["--no-tabs", "sync", "conflict-test", "--strategy", "safe"])
        .current_dir(tmp.path())
        .assert()
        .success(); // Command succeeds but reports conflicts in output

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    let combined = format!("{stdout}{stderr}");

    // Safe strategy with diverged branches should detect conflict and skip sync.
    // Verify HEAD has NOT been force-reset (local commit is preserved).
    let head_after_sync = git_head(&wt);
    assert_eq!(
        head_after_local, head_after_sync,
        "Safe strategy should NOT force-sync when branches have diverged; HEAD should be unchanged"
    );

    // The local file content should still be our local change, not overwritten
    let readme_content = fs::read_to_string(wt.join("README.md")).unwrap();
    assert_eq!(
        readme_content, "local conflicting change",
        "Safe strategy should preserve local changes when conflict is detected"
    );

    // Output should mention conflict or skipped status
    assert!(
        combined.contains("conflict")
            || combined.contains("skip")
            || combined.contains("diverged")
            || combined.contains("spoon-knife"),
        "Should mention conflict/skip/diverged or the package in output, got stdout={stdout}, stderr={stderr}"
    );
}

#[test]
fn test_sync_theirs_strategy_resolves_conflicts() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "theirs-resolve"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt = tmp.path().join("worktrees/theirs-resolve/spoon-knife");

    // Make a local commit that modifies README.md
    fs::write(wt.join("README.md"), "local version").unwrap();
    process::Command::new("git")
        .args(["add", "."])
        .current_dir(&wt)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["commit", "-m", "local change"])
        .current_dir(&wt)
        .output()
        .unwrap();

    // Push upstream change to the same file
    push_commit_to_bare(&repo, "README.md", "upstream version wins");

    // With "theirs" strategy, upstream should win
    meldr()
        .args([
            "--no-tabs",
            "sync",
            "theirs-resolve",
            "--strategy",
            "theirs",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));
}

#[test]
fn test_sync_log_contains_expected_fields() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "log-fields"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "log-fields.txt", "log field test");

    meldr()
        .args(["--no-tabs", "sync", "log-fields"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let log_file = tmp.path().join(".meldr/sync-log.jsonl");
    let content = fs::read_to_string(&log_file).unwrap();

    // Verify JSONL format with expected fields
    assert!(
        content.contains("\"branch\":\"log-fields\""),
        "Should contain branch name"
    );
    assert!(
        content.contains("\"spoon-knife\""),
        "Should contain package name"
    );
    assert!(
        content.contains("\"status\""),
        "Should contain status field"
    );
    assert!(
        content.contains("\"method\""),
        "Should contain method field"
    );
}

#[test]
fn test_sync_dry_run_skips_snapshot_and_log() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "dry-nolog"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "dry-nolog.txt", "dry run no log");

    meldr()
        .args(["--no-tabs", "sync", "dry-nolog", "--dry-run"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Dry run should NOT create snapshot or log
    let snapshot_dir = tmp.path().join(".meldr/sync-snapshots");
    let log_file = tmp.path().join(".meldr/sync-log.jsonl");

    let has_snapshots = snapshot_dir.exists()
        && fs::read_dir(&snapshot_dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
    assert!(!has_snapshots, "Dry run should not create snapshots");
    assert!(!log_file.exists(), "Dry run should not create sync log");
}

// ─── 18. TMUX INTEGRATION TESTS ──────────────────────────────────────────────
//
// These tests verify tmux-related behavior. Tests that need an actual tmux
// session start one via `tmux new-session -d`. Tests for error paths (NotInTmux)
// work without tmux.

#[test]
fn test_worktree_add_without_no_tabs_fails_outside_tmux() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Without --no-tabs and outside tmux, should fail with NotInTmux
    meldr()
        .args(["worktree", "add", "tmux-fail"])
        .env_remove("TMUX")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not inside a tmux session"));
}

#[test]
fn test_worktree_open_fails_outside_tmux() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create with --no-tabs first
    meldr()
        .args(["--no-tabs", "worktree", "add", "open-fail"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Try to open without tmux — should fail
    meldr()
        .args(["worktree", "open", "open-fail"])
        .env_remove("TMUX")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("tmux"));
}

#[test]
fn test_mode_no_tabs_config_disables_tmux() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Set mode to no-tabs via config
    meldr()
        .args(["config", "set", "mode", "no-tabs"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Should succeed without --no-tabs flag because config disables tmux
    meldr()
        .args(["worktree", "add", "notabs-config"])
        .env_remove("TMUX")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created worktree 'notabs-config'"));

    assert!(
        tmp.path()
            .join("worktrees/notabs-config/spoon-knife")
            .exists()
    );
}

#[test]
fn test_no_tabs_worktree_has_no_tmux_state() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "no-tmux-state"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // State file should have the worktree but no tmux window
    let state_content = fs::read_to_string(tmp.path().join(".meldr/state.json")).unwrap();
    assert!(
        state_content.contains("no-tmux-state"),
        "State should contain worktree"
    );
    // tmux_window should be null
    assert!(
        state_content.contains("\"tmux_window\":null")
            || state_content.contains("\"tmux_window\": null"),
        "tmux_window should be null for --no-tabs worktree, got: {state_content}"
    );
}

#[test]
fn test_worktree_list_shows_no_tmux_window() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "list-tmux"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // List should show "no tmux window" for --no-tabs worktrees
    meldr()
        .args(["worktree", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("list-tmux").and(predicate::str::contains("no tmux window")),
        );
}

/// Helper to start a tmux server and return a cleanup guard.
/// The TMUX env var value needed to simulate being "inside" tmux.
fn start_tmux_server() -> Option<(String, String)> {
    // Start a detached tmux session
    let session_name = format!("meldr-test-{}", std::process::id());
    let result = process::Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            &session_name,
            "-x",
            "200",
            "-y",
            "50",
        ])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            // Get the TMUX env var value from the server
            let info = process::Command::new("tmux")
                .args([
                    "display-message",
                    "-t",
                    &session_name,
                    "-p",
                    "#{socket_path},#{pid},0",
                ])
                .output()
                .ok()?;
            let tmux_var = String::from_utf8_lossy(&info.stdout).trim().to_string();
            if tmux_var.is_empty() {
                // Fallback: construct a plausible TMUX value
                Some((
                    format!("/tmp/tmux-0/default,{},0", std::process::id()),
                    session_name,
                ))
            } else {
                Some((tmux_var, session_name))
            }
        }
        _ => None,
    }
}

fn kill_tmux_session(session_name: &str) {
    let _ = process::Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .output();
}

#[test]
fn test_worktree_add_inside_tmux_creates_windows() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Add worktree WITH tmux (no --no-tabs)
    let result = meldr()
        .args(["worktree", "add", "tmux-add"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&result.get_output().stdout);
    assert!(
        stdout.contains("Created worktree 'tmux-add'"),
        "Should create worktree"
    );

    // State should have a tmux_window set
    let state_content = fs::read_to_string(tmp.path().join(".meldr/state.json")).unwrap();
    assert!(
        !state_content.contains("\"tmux_window\":null")
            && !state_content.contains("\"tmux_window\": null"),
        "tmux_window should be set when created inside tmux, got: {state_content}"
    );

    // Verify worktree dir was created
    assert!(tmp.path().join("worktrees/tmux-add/spoon-knife").exists());

    kill_tmux_session(&session);
}

/// Verifies the default layout produces 9 panes laid out as
/// 3 equal-width claude panes filling the top ~2/3 of the window
/// and 6 equal-size terminals filling the bottom ~1/3 in a 2×3 grid.
#[test]
fn test_worktree_layout_top_bottom_geometry() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["worktree", "add", "geometry"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    // Read the window id meldr recorded in state.json — meldr issues `new-window`
    // without an explicit session target, so the window may land in whichever
    // session tmux treats as "current" for the supplied TMUX env var (not
    // necessarily our test session). Querying by window_id avoids that ambiguity.
    let state_content = fs::read_to_string(tmp.path().join(".meldr/state.json")).unwrap();
    let window_id = state_content
        .lines()
        .find_map(|l| {
            l.trim().strip_prefix("\"tmux_window\":").map(|rest| {
                rest.trim()
                    .trim_matches(|c: char| c == '"' || c == ',')
                    .to_string()
            })
        })
        .unwrap_or_else(|| panic!("tmux_window not found in: {state_content}"));

    let panes_out = process::Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            &window_id,
            "-F",
            "#{pane_left} #{pane_top} #{pane_width} #{pane_height}",
        ])
        .output()
        .expect("tmux list-panes");
    let panes: Vec<(u32, u32, u32, u32)> = String::from_utf8_lossy(&panes_out.stdout)
        .lines()
        .map(|l| {
            let mut it = l.split_whitespace().map(|n| n.parse::<u32>().unwrap());
            (
                it.next().unwrap(),
                it.next().unwrap(),
                it.next().unwrap(),
                it.next().unwrap(),
            )
        })
        .collect();

    kill_tmux_session(&session);

    assert_eq!(
        panes.len(),
        9,
        "expected 9 panes (3 claude + 6 terminals), got {}: {panes:?}",
        panes.len()
    );

    // Group panes by pane_top (y-origin → row).
    let mut tops: Vec<u32> = panes.iter().map(|p| p.1).collect();
    tops.sort();
    tops.dedup();
    assert_eq!(tops.len(), 3, "expected 3 distinct rows, got {tops:?}");

    let row = |top: u32| -> Vec<(u32, u32, u32, u32)> {
        let mut r: Vec<_> = panes.iter().copied().filter(|p| p.1 == top).collect();
        r.sort_by_key(|p| p.0);
        r
    };
    let top_row = row(tops[0]);
    let mid_row = row(tops[1]);
    let bot_row = row(tops[2]);

    // Widths can spread up to 2 cells across 3 columns due to integer rounding
    // when tmux splits an even-width pane via two successive percentage splits
    // (e.g. 200 → 65/66/67). 2 cells out of ~200 is < 1% — visually equal.
    for (label, r) in [("top", &top_row), ("mid", &mid_row), ("bot", &bot_row)] {
        assert_eq!(r.len(), 3, "{label} row should have 3 panes, got {r:?}");
        let widths: Vec<u32> = r.iter().map(|p| p.2).collect();
        let max = *widths.iter().max().unwrap();
        let min = *widths.iter().min().unwrap();
        assert!(
            max - min <= 2,
            "{label} row widths should be equal (±2), got {widths:?}"
        );
    }

    // Top row height should be ~2/3 of total. Total here = sum of one column's heights + 2 borders.
    let total_h = top_row[0].3 + mid_row[0].3 + bot_row[0].3 + 2;
    let top_h = top_row[0].3;
    let expected = (total_h * 2) / 3;
    assert!(
        top_h.abs_diff(expected) <= 2,
        "top row height should be ~2/3 of total ({expected} of {total_h}), got {top_h}"
    );

    // Middle and bottom row heights should be equal (±1 row from integer rounding).
    let mid_h = mid_row[0].3;
    let bot_h = bot_row[0].3;
    assert!(
        mid_h.abs_diff(bot_h) <= 1,
        "middle ({mid_h}) and bottom ({bot_h}) row heights should match within 1"
    );
}

#[test]
fn test_worktree_remove_inside_tmux_kills_window() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Add with tmux
    meldr()
        .args(["worktree", "add", "tmux-rm"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    // Remove with tmux — should kill window
    meldr()
        .args(["worktree", "remove", "tmux-rm"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed worktree 'tmux-rm'"));

    // Worktree directory should be gone
    assert!(!tmp.path().join("worktrees/tmux-rm").exists());

    // State should no longer have this worktree
    let state_after = fs::read_to_string(tmp.path().join(".meldr/state.json")).unwrap();
    assert!(
        !state_after.contains("tmux-rm"),
        "State should not contain removed worktree"
    );

    kill_tmux_session(&session);
}

#[test]
fn test_worktree_add_multiple_inside_tmux() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo1 = copy_repo(repos.path(), "spoon-knife");
    let repo2 = copy_repo(repos.path(), "git-consortium");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create two worktrees with tmux
    meldr()
        .args(["worktree", "add", "tmux-multi-a"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["worktree", "add", "tmux-multi-b"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    // Both should be in state with tmux windows
    let state = fs::read_to_string(tmp.path().join(".meldr/state.json")).unwrap();
    assert!(state.contains("tmux-multi-a"), "Should have worktree a");
    assert!(state.contains("tmux-multi-b"), "Should have worktree b");

    // List should show both with tmux info
    meldr()
        .args(["worktree", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("tmux-multi-a").and(predicate::str::contains("tmux-multi-b")),
        );

    kill_tmux_session(&session);
}

#[test]
fn test_create_with_repos_inside_tmux() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    // Full create with repo + branch inside tmux
    meldr()
        .args(["create", "tmux-ws", "-r", &repo, "-b", "tmux-branch"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Created workspace 'tmux-ws'")
                .and(predicate::str::contains("Created worktree 'tmux-branch'")),
        );

    let ws = tmp.path().join("tmux-ws");
    assert!(ws.join("worktrees/tmux-branch/spoon-knife").exists());

    // State should have tmux window
    let state = fs::read_to_string(ws.join(".meldr/state.json")).unwrap();
    assert!(state.contains("tmux-branch"));

    kill_tmux_session(&session);
}

#[test]
fn test_sync_inside_tmux_works() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["worktree", "add", "tmux-sync"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    push_commit_to_bare(&repo, "tmux-synced.txt", "synced inside tmux");

    // Sync should work inside tmux
    meldr()
        .args(["sync", "tmux-sync"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    assert!(
        tmp.path()
            .join("worktrees/tmux-sync/spoon-knife/tmux-synced.txt")
            .exists()
    );

    kill_tmux_session(&session);
}

#[test]
fn test_exec_inside_tmux_works() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["worktree", "add", "tmux-exec"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/tmux-exec/spoon-knife");

    meldr()
        .args(["exec", "echo", "hello-tmux"])
        .env("TMUX", &tmux_var)
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("[spoon-knife] hello-tmux"));

    kill_tmux_session(&session);
}

#[test]
fn test_status_inside_tmux_shows_tmux_info() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["worktree", "add", "tmux-status"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["status"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Workspace:").and(predicate::str::contains("spoon-knife")),
        );

    kill_tmux_session(&session);
}

// ─── 19. NO-AGENT FLAG ────────────────────────────────────────────────────────

#[test]
fn test_no_agent_flag_with_no_tabs() {
    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    // --no-agent --no-tabs should both work together
    meldr()
        .args([
            "--no-agent",
            "--no-tabs",
            "worktree",
            "add",
            "no-agent-test",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created worktree 'no-agent-test'"));

    assert!(
        tmp.path()
            .join("worktrees/no-agent-test/spoon-knife")
            .exists()
    );
}

// ─── 20. NEW BUILT-IN AGENTS ──────────────────────────────────────────────────

#[test]
fn test_worktree_add_with_kiro_tui_agent_inside_tmux() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["config", "set", "agent", "kiro-tui"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["config", "list"])
        .env_remove("MELDR_AGENT")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("agent = kiro-tui")
                .and(predicate::str::contains("agent_command = kiro-cli --tui")),
        );

    meldr()
        .args(["worktree", "add", "kiro-tui-branch"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Created worktree 'kiro-tui-branch'",
        ));

    assert!(
        tmp.path()
            .join("worktrees/kiro-tui-branch/spoon-knife")
            .exists()
    );

    kill_tmux_session(&session);
}

#[test]
fn test_worktree_add_with_deepseek_tui_agent_inside_tmux() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["config", "set", "agent", "deepseek-tui"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["config", "list"])
        .env_remove("MELDR_AGENT")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("agent = deepseek-tui")
                .and(predicate::str::contains("agent_command = deepseek-tui")),
        );

    meldr()
        .args(["worktree", "add", "deepseek-tui-branch"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Created worktree 'deepseek-tui-branch'",
        ));

    assert!(
        tmp.path()
            .join("worktrees/deepseek-tui-branch/spoon-knife")
            .exists()
    );

    kill_tmux_session(&session);
}

#[test]
fn test_worktree_add_with_devin_agent_inside_tmux() {
    let Some((tmux_var, session)) = start_tmux_server() else {
        eprintln!("Skipping: tmux not available");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let repos = TempDir::new().unwrap();
    let repo = copy_repo(repos.path(), "spoon-knife");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["config", "set", "agent", "devin"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["config", "list"])
        .env_remove("MELDR_AGENT")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("agent = devin").and(predicate::str::contains(
                "agent_command = devin --permission-mode bypass",
            )),
        );

    meldr()
        .args(["worktree", "add", "devin-branch"])
        .env("TMUX", &tmux_var)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created worktree 'devin-branch'"));

    assert!(
        tmp.path()
            .join("worktrees/devin-branch/spoon-knife")
            .exists()
    );

    kill_tmux_session(&session);
}
