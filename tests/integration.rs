use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::process;
use tempfile::TempDir;

#[allow(deprecated)]
fn meldr() -> Command {
    Command::cargo_bin("meldr").unwrap()
}

fn init_workspace(dir: &std::path::Path) {
    meldr()
        .args(["init", "--name", "test-ws"])
        .current_dir(dir)
        .assert()
        .success();
}

fn create_bare_repo(dir: &std::path::Path, name: &str) -> String {
    let repo_path = dir.join(name);
    fs::create_dir_all(&repo_path).unwrap();

    process::Command::new("git")
        .args(["init", "--bare"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create a temporary clone to add an initial commit
    let tmp_clone = dir.join(format!("{name}-tmp"));
    process::Command::new("git")
        .args([
            "clone",
            repo_path.to_str().unwrap(),
            tmp_clone.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Configure git user for the clone
    process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();

    fs::write(tmp_clone.join("README.md"), "# test").unwrap();
    process::Command::new("git")
        .args(["add", "."])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["push"])
        .current_dir(&tmp_clone)
        .output()
        .unwrap();

    fs::remove_dir_all(&tmp_clone).unwrap();

    repo_path.to_str().unwrap().to_string()
}

#[test]
fn test_init() {
    let tmp = TempDir::new().unwrap();
    meldr()
        .args(["init", "--name", "my-workspace"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Initialized meldr workspace 'my-workspace'",
        ));

    assert!(tmp.path().join("meldr.toml").exists());
    assert!(tmp.path().join("packages").exists());
    assert!(tmp.path().join("worktrees").exists());
    assert!(tmp.path().join(".meldr").exists());
}

#[test]
fn test_init_default_name() {
    let tmp = TempDir::new().unwrap();
    meldr()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized meldr workspace"));

    assert!(tmp.path().join("meldr.toml").exists());
}

#[test]
fn test_init_already_initialized() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .arg("init")
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already initialized"));
}

#[test]
fn test_package_add_and_list() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Added package 'frontend'"));

    meldr()
        .args(["package", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("frontend"));
}

#[test]
fn test_package_add_multiple() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo1 = create_bare_repo(repos_dir.path(), "frontend");
    let repo2 = create_bare_repo(repos_dir.path(), "backend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Added package 'frontend'")
                .and(predicate::str::contains("Added package 'backend'")),
        );
}

#[test]
fn test_package_remove() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["package", "remove", "frontend"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed package 'frontend'"));
}

#[test]
fn test_package_list_empty() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["package", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No packages in workspace"));
}

#[test]
fn test_worktree_add_no_tabs() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created worktree 'feature-test'"));

    let wt_dir = tmp.path().join("worktrees/feature-test/frontend");
    assert!(wt_dir.exists(), "Worktree dir should exist on filesystem");
    assert!(
        wt_dir.join(".git").exists(),
        "Worktree should have a .git file (git worktree link)"
    );

    // Verify checked-out files exist
    let entries: Vec<_> = fs::read_dir(&wt_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "Worktree should contain checked-out files"
    );
}

#[test]
fn test_worktree_remove_no_tabs() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-rm"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/feature-rm");
    let wt_pkg_dir = wt_dir.join("frontend");
    assert!(wt_dir.exists(), "Worktree dir should exist before remove");
    assert!(
        wt_pkg_dir.exists(),
        "Worktree package dir should exist before remove"
    );

    meldr()
        .args(["--no-tabs", "worktree", "remove", "feature-rm"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed worktree 'feature-rm'"));

    // Verify the directory is actually gone after remove
    assert!(
        !wt_dir.exists(),
        "Worktree directory should be gone after remove"
    );
    assert!(
        !wt_pkg_dir.exists(),
        "Worktree package dir should be gone after remove"
    );
}

#[test]
fn test_worktree_list() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-list"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["worktree", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("feature-list"));
}

#[test]
fn test_worktree_add_duplicate() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-dup"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-dup"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_status() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .arg("status")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Workspace:")
                .and(predicate::str::contains("Packages:"))
                .and(predicate::str::contains("frontend")),
        );
}

#[test]
fn test_no_workspace_error() {
    let tmp = TempDir::new().unwrap();

    meldr()
        .args(["package", "list"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not a meldr workspace"));
}

#[test]
fn test_exec_from_worktree() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-exec"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/feature-exec/frontend");

    meldr()
        .args(["exec", "echo", "hello"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("[frontend] hello"));
}

#[test]
fn test_exec_fails_outside_worktree() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Running exec from workspace root should fail
    meldr()
        .args(["exec", "echo", "hello"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "meldr exec must be run from within a worktree directory",
        ));
}

#[test]
fn test_exec_runs_in_worktree_dir_not_packages() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-pwd"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/feature-pwd/frontend");

    // Create a marker file in the worktree dir to verify exec runs there
    fs::write(wt_dir.join("worktree-marker.txt"), "in-worktree").unwrap();

    meldr()
        .args(["exec", "cat", "worktree-marker.txt"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("in-worktree"));
}

#[test]
fn test_exec_from_worktree_root() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-root"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Running from the worktree branch root dir (not inside a package subdir)
    let wt_root = tmp.path().join("worktrees/feature-root");

    meldr()
        .args(["exec", "echo", "from-root"])
        .current_dir(&wt_root)
        .assert()
        .success()
        .stdout(predicate::str::contains("[frontend] from-root"));
}

#[test]
fn test_exec_with_slash_branch() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "fm/exec-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/fm-exec-test/frontend");

    meldr()
        .args(["exec", "echo", "slash-branch"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("slash-branch"));
}

#[test]
fn test_exec_multiple_packages() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo1 = create_bare_repo(repos_dir.path(), "frontend");
    let repo2 = create_bare_repo(repos_dir.path(), "backend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-multi"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/feature-multi/frontend");

    meldr()
        .args(["exec", "echo", "multi"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(
            predicate::str::contains("[frontend] multi")
                .and(predicate::str::contains("[backend] multi")),
        );
}

#[test]
fn test_config_set_and_get() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["config", "set", "agent", "cursor"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Set agent = cursor"));

    meldr()
        .args(["config", "get", "agent"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("cursor"));

    // Verify the config file on disk contains the expected value
    let toml_content = fs::read_to_string(tmp.path().join("meldr.toml")).unwrap();
    assert!(
        toml_content.contains("cursor"),
        "meldr.toml should contain 'cursor' after config set, got: {toml_content}"
    );
}

#[test]
fn test_config_list() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["config", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("agent =")
                .and(predicate::str::contains("mode ="))
                .and(predicate::str::contains("sync_method =")),
        );
}

#[test]
fn test_pkg_alias() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["pkg", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No packages in workspace"));
}

#[test]
fn test_wt_alias() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["wt", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No active worktrees"));
}

#[test]
fn test_st_alias() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .arg("st")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Workspace:"));
}

#[test]
fn test_create_bare() {
    let tmp = TempDir::new().unwrap();

    meldr()
        .args(["create", "my-ws"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Created workspace 'my-ws'")
                .and(predicate::str::contains("Workspace ready at")),
        );

    assert!(tmp.path().join("my-ws/meldr.toml").exists());
    assert!(tmp.path().join("my-ws/packages").exists());
    assert!(tmp.path().join("my-ws/worktrees").exists());
}

#[test]
fn test_create_with_repos() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    meldr()
        .args(["create", "my-ws", "-r", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Created workspace 'my-ws'")
                .and(predicate::str::contains("Added package 'frontend'")),
        );

    assert!(tmp.path().join("my-ws/packages/frontend").exists());
}

#[test]
fn test_create_with_repos_and_branch() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    meldr()
        .args([
            "create",
            "my-ws",
            "-r",
            &repo_url,
            "-b",
            "feature-x",
            "--no-tabs",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Created workspace 'my-ws'")
                .and(predicate::str::contains("Added package 'frontend'"))
                .and(predicate::str::contains("Created worktree 'feature-x'")),
        );

    assert!(
        tmp.path()
            .join("my-ws/worktrees/feature-x/frontend")
            .exists()
    );
}

#[test]
fn test_create_with_agent() {
    let tmp = TempDir::new().unwrap();

    meldr()
        .args(["create", "my-ws", "-a", "cursor"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let content = fs::read_to_string(tmp.path().join("my-ws/meldr.toml")).unwrap();
    assert!(content.contains("cursor"));
}

#[test]
fn test_create_full_combo() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo1 = create_bare_repo(repos_dir.path(), "frontend");
    let repo2 = create_bare_repo(repos_dir.path(), "backend");

    meldr()
        .args([
            "create",
            "my-ws",
            "-r",
            &repo1,
            "-r",
            &repo2,
            "-b",
            "feature-y",
            "-a",
            "cursor",
            "--no-tabs",
        ])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Created workspace 'my-ws'")
                .and(predicate::str::contains("Added package 'frontend'"))
                .and(predicate::str::contains("Added package 'backend'"))
                .and(predicate::str::contains("Created worktree 'feature-y'")),
        );

    assert!(
        tmp.path()
            .join("my-ws/worktrees/feature-y/frontend")
            .exists()
    );
    assert!(
        tmp.path()
            .join("my-ws/worktrees/feature-y/backend")
            .exists()
    );
}

#[test]
fn test_create_already_exists() {
    let tmp = TempDir::new().unwrap();

    meldr()
        .args(["create", "my-ws"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["create", "my-ws"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already initialized"));
}

#[test]
fn test_create_branch_without_repos_warns() {
    let tmp = TempDir::new().unwrap();

    meldr()
        .args(["create", "my-ws", "-b", "feature-z", "--no-tabs"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "No packages to create worktrees for",
        ));
}

#[test]
fn test_config_set_invalid_key() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["config", "set", "bogus_key", "value"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown setting"));
}

#[test]
fn test_worktree_remove_nonexistent() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["worktree", "remove", "no-such-branch"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_version_flag() {
    meldr()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("meldr"));
}

#[test]
fn test_worktree_add_with_slash_branch() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "fm/whatever"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created worktree 'fm/whatever'"));

    // Directory should use sanitized name (/ replaced with -)
    assert!(tmp.path().join("worktrees/fm-whatever/frontend").exists());
    // Should NOT have a nested fm/whatever directory
    assert!(!tmp.path().join("worktrees/fm/whatever").exists());
}

#[test]
fn test_worktree_remove_with_slash_branch() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "fm/feature"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "remove", "fm/feature"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed worktree 'fm/feature'"));

    assert!(!tmp.path().join("worktrees/fm-feature").exists());
}

#[test]
fn test_worktree_remove_auto_detect_from_cwd() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "fm/auto-rm"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Run remove from within the worktree directory (no branch arg)
    let wt_dir = tmp.path().join("worktrees/fm-auto-rm/frontend");
    assert!(wt_dir.exists());

    meldr()
        .args(["--no-tabs", "worktree", "remove"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed worktree 'fm/auto-rm'"));
}

#[test]
fn test_worktree_remove_no_branch_outside_worktree_fails() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["worktree", "remove"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Could not detect current worktree",
        ));
}

#[test]
fn test_init_toml_has_commented_defaults() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    let content = fs::read_to_string(tmp.path().join("meldr.toml")).unwrap();
    assert!(content.contains("# agent = \"claude\""));
    assert!(content.contains("# mode = \"full\""));
    assert!(content.contains("# sync_method = \"rebase\""));
    assert!(content.contains("# sync_strategy = \"safe\""));
}

#[test]
fn test_create_with_agent_sets_setting() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    meldr()
        .args([
            "create",
            "my-ws",
            "-r",
            &repo_url,
            "-a",
            "cursor",
            "--no-tabs",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    // After packages are added, meldr.toml should have the agent setting and package entries
    let content = fs::read_to_string(tmp.path().join("my-ws/meldr.toml")).unwrap();
    assert!(
        content.contains("agent = \"cursor\""),
        "Agent setting should persist"
    );
    assert!(
        content.contains("[[package]]"),
        "Package entries should exist"
    );
    assert!(
        content.contains("frontend"),
        "Package name should be present"
    );
}

#[test]
fn test_package_add_creates_worktrees_for_existing_branches() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo1 = create_bare_repo(repos_dir.path(), "frontend");
    let repo2 = create_bare_repo(repos_dir.path(), "backend");

    init_workspace(tmp.path());

    // Add first package and create a worktree
    meldr()
        .args(["package", "add", &repo1])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-wt"])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert!(tmp.path().join("worktrees/feature-wt/frontend").exists());

    // Now add a second package — it should auto-create a worktree on "feature-wt"
    meldr()
        .args(["package", "add", &repo2])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Added package 'backend'").and(predicate::str::contains(
                "Created worktree for 'backend' on branch 'feature-wt'",
            )),
        );

    assert!(tmp.path().join("worktrees/feature-wt/backend").exists());
}

#[test]
fn test_config_set_new_keys() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    // Test setting all new config keys
    for (key, value) in &[
        ("editor", "code ."),
        ("default_branch", "develop"),
        ("remote", "upstream"),
        ("shell", "/bin/zsh"),
        ("layout", "minimal"),
        ("window_name", "[{branch}] {pkg}"),
    ] {
        meldr()
            .args(["config", "set", key, value])
            .current_dir(tmp.path())
            .assert()
            .success()
            .stdout(predicate::str::contains(format!("Set {key} = {value}")));

        meldr()
            .args(["config", "get", key])
            .current_dir(tmp.path())
            .assert()
            .success()
            .stdout(predicate::str::contains(*value));
    }
}

#[test]
fn test_config_list_shows_new_fields() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["config", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("editor =")
                .and(predicate::str::contains("default_branch ="))
                .and(predicate::str::contains("remote ="))
                .and(predicate::str::contains("shell ="))
                .and(predicate::str::contains("layout ="))
                .and(predicate::str::contains("window_name =")),
        );
}

#[test]
fn test_init_toml_has_new_commented_defaults() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    let content = fs::read_to_string(tmp.path().join("meldr.toml")).unwrap();
    assert!(content.contains("# editor = \"nvim .\""));
    assert!(content.contains("# default_branch = \"main\""));
    assert!(content.contains("# remote = \"origin\""));
    assert!(content.contains("# shell = \"sh\""));
    assert!(content.contains("# layout = \"default\""));
}

#[test]
fn test_exec_respects_shell_config() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());
    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-shell"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Set shell to bash and verify exec still works
    meldr()
        .args(["config", "set", "shell", "bash"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_dir = tmp.path().join("worktrees/feature-shell/frontend");

    meldr()
        .args(["exec", "echo", "works"])
        .current_dir(&wt_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("works"));
}

#[test]
fn test_bare_clone_has_remote_tracking_refs() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "myrepo");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    let pkg_path = tmp.path().join("packages/myrepo");

    // Verify fetch refspec is configured for remote tracking
    let refspec = process::Command::new("git")
        .args(["config", "--get-all", "remote.origin.fetch"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();
    let refspec_str = String::from_utf8_lossy(&refspec.stdout);
    assert!(
        refspec_str.contains("+refs/heads/*:refs/remotes/origin/*"),
        "Bare clone should have fetch refspec for remote tracking, got: {refspec_str}"
    );

    // Verify refs/remotes/origin/HEAD is set
    let head_ref = process::Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();
    assert!(
        head_ref.status.success(),
        "refs/remotes/origin/HEAD should be set after clone"
    );
    let head_str = String::from_utf8_lossy(&head_ref.stdout);
    assert!(
        head_str.contains("refs/remotes/origin/"),
        "HEAD should point to a remote tracking ref, got: {head_str}"
    );

    // Verify refs/remotes/origin/main exists
    let remote_main = process::Command::new("git")
        .args(["rev-parse", "refs/remotes/origin/main"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();
    assert!(
        remote_main.status.success(),
        "refs/remotes/origin/main should exist after clone"
    );
}

#[test]
fn test_sync_works_after_package_add() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "test-sync"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Sync should succeed — this would fail without remote tracking refs
    meldr()
        .args(["sync", "test-sync"])
        .current_dir(tmp.path())
        .assert()
        .success();
}

#[test]
fn test_sync_no_worktrees_at_root() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    // Sync at workspace root with no worktrees should fail (can't detect branch)
    meldr()
        .args(["sync"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Could not detect current worktree",
        ));
}

#[test]
fn test_sync_all_no_worktrees_succeeds() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    // --all with no worktrees should succeed and fetch packages
    meldr()
        .args(["sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "No active worktrees. All packages fetched and main updated.",
        ));
}

#[test]
fn test_sync_at_workspace_root_with_worktrees() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-sync"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Sync at workspace root (not inside a worktree) without explicit branch should fail
    meldr()
        .args(["sync"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Could not detect current worktree",
        ));
}

#[test]
fn test_sync_all_with_worktrees() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-all"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // --all should sync worktrees and show summary
    meldr()
        .args(["sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("frontend").and(predicate::str::contains("up-to-date")));
}

#[test]
fn test_out_of_sync_warning_on_status() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-warn"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Push an upstream commit to make the worktree behind
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "warn-file.txt",
        "trigger warning",
    );

    // Fetch so local refs know about the new commit
    meldr()
        .args(["sync", "feat-warn"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Push another commit — after sync the worktree is up to date,
    // so push again and fetch (without syncing) to make it behind
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "warn-file2.txt",
        "trigger warning 2",
    );

    // Fetch the bare repo directly so refs update without syncing the worktree
    let pkg_path = tmp.path().join("packages/frontend");
    std::process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();

    // Now `status` should show the warning with specific staleness info
    let output = meldr()
        .args(["status"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(
        stderr.contains("behind"),
        "Status should warn about being behind, got stderr: {stderr}"
    );
    // Staleness warning should mention the package name
    assert!(
        stderr.contains("frontend"),
        "Staleness warning should mention the package name 'frontend', got stderr: {stderr}"
    );
}

#[test]
fn test_no_warning_when_up_to_date() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-ok"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Status should NOT show a warning when worktree is up to date
    meldr()
        .args(["status"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("behind").not());
}

// ─── Global config tests ───────────────────────────────────────

/// Helper to run meldr with a custom HOME directory so global config
/// writes go to a temp dir instead of the real ~/.meldr.
fn meldr_with_home(home: &std::path::Path) -> Command {
    let mut cmd = meldr();
    cmd.env("HOME", home);
    cmd
}

#[test]
fn test_config_set_global() {
    let home = TempDir::new().unwrap();

    meldr_with_home(home.path())
        .args(["config", "set", "--global", "editor", "code ."])
        .assert()
        .success()
        .stdout(predicate::str::contains("Set editor = code . (global)"));

    // Verify the file was created
    assert!(home.path().join(".meldr/config.toml").exists());
}

#[test]
fn test_config_get_global() {
    let home = TempDir::new().unwrap();

    // Set first
    meldr_with_home(home.path())
        .args(["config", "set", "--global", "remote", "upstream"])
        .assert()
        .success();

    // Get
    meldr_with_home(home.path())
        .args(["config", "get", "--global", "remote"])
        .assert()
        .success()
        .stdout(predicate::str::contains("upstream"));
}

#[test]
fn test_config_unset_global() {
    let home = TempDir::new().unwrap();

    meldr_with_home(home.path())
        .args(["config", "set", "--global", "layout", "minimal"])
        .assert()
        .success();

    meldr_with_home(home.path())
        .args(["config", "unset", "--global", "layout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unset layout (global)"));

    meldr_with_home(home.path())
        .args(["config", "get", "--global", "layout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not set"));
}

#[test]
fn test_config_list_global() {
    let home = TempDir::new().unwrap();

    meldr_with_home(home.path())
        .args(["config", "set", "--global", "agent", "cursor"])
        .assert()
        .success();

    meldr_with_home(home.path())
        .args(["config", "list", "--global"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Global configuration")
                .and(predicate::str::contains("agent = cursor")),
        );
}

#[test]
fn test_config_unset_workspace() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["config", "set", "agent", "cursor"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["config", "unset", "agent"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Unset agent (workspace)"));

    meldr()
        .args(["config", "get", "agent"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("not set"));
}

#[test]
fn test_config_show_sources() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Init workspace with custom HOME
    meldr_with_home(home.path())
        .args(["init", "--name", "test-ws"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Set global editor
    meldr_with_home(home.path())
        .args(["config", "set", "--global", "editor", "code ."])
        .assert()
        .success();

    // Set workspace agent
    meldr_with_home(home.path())
        .args(["config", "set", "agent", "cursor"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Show should display sources
    meldr_with_home(home.path())
        .args(["config", "show"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("agent = cursor (workspace)")
                .and(predicate::str::contains("editor = code . (global)"))
                .and(predicate::str::contains("remote = origin (default)")),
        );
}

#[test]
fn test_config_precedence_ws_over_global() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Init workspace
    meldr_with_home(home.path())
        .args(["init", "--name", "test-ws"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Set global default_branch
    meldr_with_home(home.path())
        .args(["config", "set", "--global", "default_branch", "develop"])
        .assert()
        .success();

    // Set workspace default_branch (should override global)
    meldr_with_home(home.path())
        .args(["config", "set", "default_branch", "staging"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // List effective config — workspace should win
    meldr_with_home(home.path())
        .args(["config", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("default_branch = staging"));
}

#[test]
fn test_config_global_overrides_default() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Init workspace
    meldr_with_home(home.path())
        .args(["init", "--name", "test-ws"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Set global shell
    meldr_with_home(home.path())
        .args(["config", "set", "--global", "shell", "/bin/zsh"])
        .assert()
        .success();

    // Effective config should reflect global override
    meldr_with_home(home.path())
        .args(["config", "list"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("shell = /bin/zsh"));
}

#[test]
fn test_config_global_without_workspace() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    // Should work even without a workspace
    meldr_with_home(home.path())
        .args(["config", "set", "--global", "agent", "none"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Set agent = none (global)"));

    meldr_with_home(home.path())
        .args(["config", "get", "--global", "agent"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("none"));
}

#[test]
fn test_config_workspace_without_global_flag_needs_workspace() {
    let tmp = TempDir::new().unwrap();

    // Without --global and outside a workspace, should fail
    meldr()
        .args(["config", "set", "agent", "cursor"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Not in a meldr workspace"));
}

#[test]
fn test_config_set_global_invalid_key() {
    let home = TempDir::new().unwrap();

    meldr_with_home(home.path())
        .args(["config", "set", "--global", "bogus", "value"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown setting"));
}

#[test]
fn test_config_unset_global_invalid_key() {
    let home = TempDir::new().unwrap();

    meldr_with_home(home.path())
        .args(["config", "unset", "--global", "bogus"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown setting"));
}

#[test]
fn test_init_creates_global_config_dir() {
    let tmp = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();

    meldr_with_home(home.path())
        .args(["init", "--name", "test-ws"])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert!(home.path().join(".meldr").exists());
    assert!(home.path().join(".meldr/config.toml").exists());
}

#[test]
fn test_global_config_default_file_contents() {
    let home = TempDir::new().unwrap();

    // Trigger global config creation
    meldr_with_home(home.path())
        .args(["config", "list", "--global"])
        .assert()
        .success();

    let content = fs::read_to_string(home.path().join(".meldr/config.toml")).unwrap();
    assert!(content.contains("# agent = \"claude\""));
    assert!(content.contains("# editor = \"nvim .\""));
    assert!(content.contains("# default_branch = \"main\""));
}

#[test]
fn test_config_set_all_global_keys() {
    let home = TempDir::new().unwrap();

    for (key, value) in &[
        ("agent", "cursor"),
        ("mode", "no-tabs"),
        ("editor", "hx ."),
        ("default_branch", "develop"),
        ("remote", "upstream"),
        ("shell", "/bin/fish"),
        ("layout", "minimal"),
        ("window_name", "[{branch}]"),
    ] {
        meldr_with_home(home.path())
            .args(["config", "set", "--global", key, value])
            .assert()
            .success();

        meldr_with_home(home.path())
            .args(["config", "get", "--global", key])
            .assert()
            .success()
            .stdout(predicate::str::contains(*value));
    }
}

#[test]
fn test_config_sync_method_not_in_global() {
    let home = TempDir::new().unwrap();

    // sync_method and sync_strategy are workspace-only settings
    meldr_with_home(home.path())
        .args(["config", "set", "--global", "sync_method", "merge"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown setting"));
}

// ---------------------------------------------------------------------------
// Sync integration tests
// ---------------------------------------------------------------------------

/// Push an additional commit to a bare repo so worktrees have something to sync.
fn push_upstream_commit(bare_repo_path: &std::path::Path, filename: &str, content: &str) {
    let tmp_clone_dir = bare_repo_path.parent().unwrap().join(format!(
        "{}-push-tmp",
        bare_repo_path.file_name().unwrap().to_str().unwrap()
    ));

    process::Command::new("git")
        .args([
            "clone",
            bare_repo_path.to_str().unwrap(),
            tmp_clone_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&tmp_clone_dir)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&tmp_clone_dir)
        .output()
        .unwrap();

    fs::write(tmp_clone_dir.join(filename), content).unwrap();
    process::Command::new("git")
        .args(["add", "."])
        .current_dir(&tmp_clone_dir)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["commit", "-m", &format!("add {filename}")])
        .current_dir(&tmp_clone_dir)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["push"])
        .current_dir(&tmp_clone_dir)
        .output()
        .unwrap();

    fs::remove_dir_all(&tmp_clone_dir).unwrap();
}

/// Helper: get HEAD sha of a git repo at `path`.
fn git_head(path: &std::path::Path) -> String {
    let out = process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn test_sync_basic() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feature-test/frontend");
    let head_before = git_head(&wt_path);

    // Push an upstream commit so there is something to sync
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "new-file.txt",
        "upstream change",
    );

    meldr()
        .args(["--no-tabs", "sync", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("synced")
                .and(predicate::str::contains("Package"))
                .and(predicate::str::contains("Status")),
        );

    // HEAD SHA should have changed after sync
    let head_after = git_head(&wt_path);
    assert_ne!(
        head_before, head_after,
        "HEAD should have changed after syncing upstream changes"
    );

    // The new file should exist in the worktree
    assert!(
        wt_path.join("new-file.txt").exists(),
        "Synced file should exist in worktree"
    );
}

#[test]
fn test_sync_dry_run() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feature-test/frontend");
    let head_before = git_head(&wt_path);

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "dry-file.txt",
        "dry run content",
    );

    meldr()
        .args(["--no-tabs", "sync", "feature-test", "--dry-run"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    // HEAD should not have changed
    let head_after = git_head(&wt_path);
    assert_eq!(head_before, head_after, "Dry run should not change HEAD");
}

#[test]
fn test_sync_all() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-a"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-b"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_a = tmp.path().join("worktrees/branch-a/frontend");
    let wt_b = tmp.path().join("worktrees/branch-b/frontend");
    let head_a_before = git_head(&wt_a);
    let head_b_before = git_head(&wt_b);

    push_upstream_commit(std::path::Path::new(&repo_url), "all-file.txt", "sync all");

    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("branch-a").and(predicate::str::contains("branch-b")));

    // HEAD SHA should have changed for both worktrees
    let head_a_after = git_head(&wt_a);
    let head_b_after = git_head(&wt_b);
    assert_ne!(
        head_a_before, head_a_after,
        "branch-a HEAD should have changed after sync --all"
    );
    assert_ne!(
        head_b_before, head_b_after,
        "branch-b HEAD should have changed after sync --all"
    );
}

#[test]
fn test_sync_with_strategy_theirs() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feature-test/frontend");
    let head_before = git_head(&wt_path);

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "theirs-file.txt",
        "theirs strategy",
    );

    meldr()
        .args(["--no-tabs", "sync", "feature-test", "--strategy", "theirs"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    // HEAD SHA should have changed after sync
    let head_after = git_head(&wt_path);
    assert_ne!(
        head_before, head_after,
        "HEAD should have changed after syncing with theirs strategy"
    );

    // The synced file should exist
    assert!(
        wt_path.join("theirs-file.txt").exists(),
        "Synced file should exist after theirs strategy sync"
    );
}

#[test]
fn test_sync_only_filter() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo1 = create_bare_repo(repos_dir.path(), "frontend");
    let repo2 = create_bare_repo(repos_dir.path(), "backend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_upstream_commit(
        std::path::Path::new(&repo1),
        "only-file.txt",
        "only frontend",
    );
    push_upstream_commit(
        std::path::Path::new(&repo2),
        "only-file.txt",
        "only backend",
    );

    let wt_fe = tmp.path().join("worktrees/feature-test/frontend");
    let wt_be = tmp.path().join("worktrees/feature-test/backend");
    let head_fe_before = git_head(&wt_fe);
    let head_be_before = git_head(&wt_be);

    let output = meldr()
        .args(["--no-tabs", "sync", "feature-test", "--only", "frontend"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("frontend"),
        "Output should mention frontend"
    );

    // frontend HEAD should have changed, backend should not
    let head_fe_after = git_head(&wt_fe);
    let head_be_after = git_head(&wt_be);
    assert_ne!(
        head_fe_before, head_fe_after,
        "frontend HEAD should change when synced with --only frontend"
    );
    assert_eq!(
        head_be_before, head_be_after,
        "backend HEAD should NOT change when synced with --only frontend"
    );
}

#[test]
fn test_sync_exclude_filter() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo1 = create_bare_repo(repos_dir.path(), "frontend");
    let repo2 = create_bare_repo(repos_dir.path(), "backend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo1, &repo2])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_upstream_commit(
        std::path::Path::new(&repo1),
        "excl-file.txt",
        "exclude test fe",
    );
    push_upstream_commit(
        std::path::Path::new(&repo2),
        "excl-file.txt",
        "exclude test be",
    );

    let wt_fe = tmp.path().join("worktrees/feature-test/frontend");
    let wt_be = tmp.path().join("worktrees/feature-test/backend");
    let head_fe_before = git_head(&wt_fe);
    let head_be_before = git_head(&wt_be);

    let output = meldr()
        .args(["--no-tabs", "sync", "feature-test", "--exclude", "backend"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("frontend"),
        "Output should mention frontend"
    );

    // frontend HEAD should have changed, backend should not
    let head_fe_after = git_head(&wt_fe);
    let head_be_after = git_head(&wt_be);
    assert_ne!(
        head_fe_before, head_fe_after,
        "frontend HEAD should change when synced with --exclude backend"
    );
    assert_eq!(
        head_be_before, head_be_after,
        "backend HEAD should NOT change when excluded"
    );
}

#[test]
fn test_sync_undo() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "undo-file.txt",
        "undo test",
    );

    // Perform a sync first
    meldr()
        .args(["--no-tabs", "sync", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Now undo
    meldr()
        .args(["--no-tabs", "sync", "feature-test", "--undo"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Undoing").and(predicate::str::contains("reset to")));
}

#[test]
fn test_sync_up_to_date() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Sync without any new upstream commits — should be up-to-date
    meldr()
        .args(["--no-tabs", "sync", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("up-to-date"));
}

#[test]
fn test_sync_no_branch_outside_worktree() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());

    meldr()
        .args(["--no-tabs", "sync"])
        .current_dir(tmp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Could not detect current worktree",
        ));
}

#[test]
fn test_sync_creates_snapshot() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "snap-file.txt",
        "snapshot test",
    );

    meldr()
        .args(["--no-tabs", "sync", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let snapshot_dir = tmp.path().join(".meldr/sync-snapshots");
    assert!(
        snapshot_dir.exists(),
        "Snapshot directory should exist after sync"
    );

    let entries: Vec<_> = fs::read_dir(&snapshot_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "There should be at least one snapshot file"
    );
}

// ---------------------------------------------------------------------------
// Robust sync tests — multi-package, merge method, diverged state, edge cases
// ---------------------------------------------------------------------------

/// Helper: set up a workspace with two packages (frontend + backend) and a worktree.
/// Returns (tmp_dir, repos_dir, frontend_bare_path, backend_bare_path).
fn setup_two_package_workspace(
    branch: &str,
) -> (TempDir, TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let frontend_url = create_bare_repo(repos_dir.path(), "frontend");
    let backend_url = create_bare_repo(repos_dir.path(), "backend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &frontend_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["package", "add", &backend_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", branch])
        .current_dir(tmp.path())
        .assert()
        .success();

    let frontend_bare = std::path::PathBuf::from(&frontend_url);
    let backend_bare = std::path::PathBuf::from(&backend_url);
    (tmp, repos_dir, frontend_bare, backend_bare)
}

/// Helper: create a local commit in a worktree directory.
fn make_local_commit(wt_path: &std::path::Path, filename: &str, content: &str) {
    fs::write(wt_path.join(filename), content).unwrap();
    process::Command::new("git")
        .args(["add", "."])
        .current_dir(wt_path)
        .output()
        .unwrap();
    process::Command::new("git")
        .args(["commit", "-m", &format!("local: add {filename}")])
        .current_dir(wt_path)
        .output()
        .unwrap();
}

#[test]
fn test_sync_multiple_packages_with_upstream_changes() {
    let (tmp, _repos_dir, frontend_bare, backend_bare) = setup_two_package_workspace("feat-multi");

    let wt_fe = tmp.path().join("worktrees/feat-multi/frontend");
    let wt_be = tmp.path().join("worktrees/feat-multi/backend");
    let head_fe_before = git_head(&wt_fe);
    let head_be_before = git_head(&wt_be);

    // Push upstream changes to both repos
    push_upstream_commit(&frontend_bare, "fe-update.txt", "frontend upstream");
    push_upstream_commit(&backend_bare, "be-update.txt", "backend upstream");

    meldr()
        .args(["--no-tabs", "sync", "feat-multi"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("frontend")
                .and(predicate::str::contains("backend"))
                .and(predicate::str::contains("synced")),
        );

    // Both packages should have advanced
    assert_ne!(head_fe_before, git_head(&wt_fe), "frontend should sync");
    assert_ne!(head_be_before, git_head(&wt_be), "backend should sync");

    // Files should exist in worktrees
    assert!(wt_fe.join("fe-update.txt").exists());
    assert!(wt_be.join("be-update.txt").exists());
}

#[test]
fn test_sync_all_multiple_packages_with_upstream_changes() {
    let (tmp, _repos_dir, frontend_bare, backend_bare) = setup_two_package_workspace("branch-x");

    // Add a second worktree
    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-y"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_x_fe = tmp.path().join("worktrees/branch-x/frontend");
    let wt_x_be = tmp.path().join("worktrees/branch-x/backend");
    let wt_y_fe = tmp.path().join("worktrees/branch-y/frontend");
    let wt_y_be = tmp.path().join("worktrees/branch-y/backend");

    let heads_before = (
        git_head(&wt_x_fe),
        git_head(&wt_x_be),
        git_head(&wt_y_fe),
        git_head(&wt_y_be),
    );

    push_upstream_commit(&frontend_bare, "all-fe.txt", "all sync fe");
    push_upstream_commit(&backend_bare, "all-be.txt", "all sync be");

    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("branch-x")
                .and(predicate::str::contains("branch-y"))
                .and(predicate::str::contains("synced")),
        );

    // All four worktree/package combos should have advanced
    assert_ne!(heads_before.0, git_head(&wt_x_fe));
    assert_ne!(heads_before.1, git_head(&wt_x_be));
    assert_ne!(heads_before.2, git_head(&wt_y_fe));
    assert_ne!(heads_before.3, git_head(&wt_y_be));
}

#[test]
fn test_sync_with_merge_method() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-merge"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feat-merge/frontend");
    let head_before = git_head(&wt_path);

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "merge-file.txt",
        "merge content",
    );

    meldr()
        .args(["--no-tabs", "sync", "feat-merge", "--merge"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced").and(predicate::str::contains("merge")));

    assert_ne!(head_before, git_head(&wt_path));
    assert!(wt_path.join("merge-file.txt").exists());
}

#[test]
fn test_sync_with_local_commits_ahead() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-ahead"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feat-ahead/frontend");

    // Make a local commit (ahead of upstream)
    make_local_commit(&wt_path, "local-work.txt", "my local changes");

    // Push an upstream commit (behind upstream)
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "upstream-new.txt",
        "upstream work",
    );

    // Sync should rebase local commits on top of upstream
    meldr()
        .args(["--no-tabs", "sync", "feat-ahead"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    // Both files should exist after sync
    assert!(
        wt_path.join("local-work.txt").exists(),
        "local commit should survive rebase"
    );
    assert!(
        wt_path.join("upstream-new.txt").exists(),
        "upstream changes should be applied"
    );
}

#[test]
fn test_sync_detects_worktree_from_cwd() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-detect"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feat-detect/frontend");
    let head_before = git_head(&wt_path);

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "detect-file.txt",
        "detect test",
    );

    // Run sync from inside the worktree dir (no explicit branch)
    meldr()
        .args(["--no-tabs", "sync"])
        .current_dir(&wt_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    assert_ne!(head_before, git_head(&wt_path));
}

#[test]
fn test_sync_slash_branch_name() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "fm/sync-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Worktree dir should be sanitized
    let wt_path = tmp.path().join("worktrees/fm-sync-test/frontend");
    assert!(wt_path.exists(), "sanitized worktree dir should exist");

    let head_before = git_head(&wt_path);

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "slash-file.txt",
        "slash branch",
    );

    // Sync using the original branch name (with slash)
    meldr()
        .args(["--no-tabs", "sync", "fm/sync-test"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    assert_ne!(head_before, git_head(&wt_path));
}

#[test]
fn test_sync_all_from_worktree_cwd() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "wt-a"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "wt-b"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_a = tmp.path().join("worktrees/wt-a/frontend");
    let wt_b = tmp.path().join("worktrees/wt-b/frontend");

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "from-inside.txt",
        "from inside worktree",
    );

    // Run --all from INSIDE a worktree dir — should still sync both
    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(&wt_a)
        .assert()
        .success()
        .stdout(predicate::str::contains("wt-a").and(predicate::str::contains("wt-b")));

    assert!(wt_a.join("from-inside.txt").exists());
    assert!(wt_b.join("from-inside.txt").exists());
}

#[test]
fn test_sync_dry_run_does_not_change_head() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-dry"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feat-dry/frontend");
    let head_before = git_head(&wt_path);

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "dry-file.txt",
        "dry run content",
    );

    meldr()
        .args(["--no-tabs", "sync", "feat-dry", "--dry-run"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Dry run"));

    // HEAD should NOT change in dry run
    assert_eq!(
        head_before,
        git_head(&wt_path),
        "dry run should not modify HEAD"
    );
    assert!(
        !wt_path.join("dry-file.txt").exists(),
        "dry run should not create files"
    );
}

#[test]
fn test_sync_sequential_syncs() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-seq"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feat-seq/frontend");

    // First upstream change + sync
    push_upstream_commit(std::path::Path::new(&repo_url), "seq-1.txt", "first change");

    meldr()
        .args(["--no-tabs", "sync", "feat-seq"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    assert!(wt_path.join("seq-1.txt").exists());
    let head_after_first = git_head(&wt_path);

    // Second upstream change + sync
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "seq-2.txt",
        "second change",
    );

    meldr()
        .args(["--no-tabs", "sync", "feat-seq"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    assert!(wt_path.join("seq-2.txt").exists());
    assert_ne!(
        head_after_first,
        git_head(&wt_path),
        "HEAD should advance after second sync"
    );

    // Third sync with no changes — should be up-to-date
    meldr()
        .args(["--no-tabs", "sync", "feat-seq"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("up-to-date"));
}

#[test]
fn test_sync_log_created() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-log"])
        .current_dir(tmp.path())
        .assert()
        .success();

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "log-file.txt",
        "log content",
    );

    meldr()
        .args(["--no-tabs", "sync", "feat-log"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let log_path = tmp.path().join(".meldr/sync-log.jsonl");
    assert!(log_path.exists(), "sync log should be created after sync");

    let log_content = fs::read_to_string(&log_path).unwrap();
    assert!(
        log_content.contains("feat-log"),
        "sync log should contain the branch name"
    );
    assert!(
        log_content.contains("frontend"),
        "sync log should contain the package name"
    );
}

#[test]
fn test_sync_undo_restores_head() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-undo2"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feat-undo2/frontend");
    let head_before_sync = git_head(&wt_path);

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "undo-file.txt",
        "will be undone",
    );

    meldr()
        .args(["--no-tabs", "sync", "feat-undo2"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let head_after_sync = git_head(&wt_path);
    assert_ne!(
        head_before_sync, head_after_sync,
        "sync should advance HEAD"
    );

    // Undo should restore to pre-sync state
    meldr()
        .args(["--no-tabs", "sync", "feat-undo2", "--undo"])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert_eq!(
        head_before_sync,
        git_head(&wt_path),
        "undo should restore HEAD to pre-sync value"
    );
}

#[test]
fn test_sync_only_with_multiple_packages() {
    let (tmp, _repos_dir, frontend_bare, backend_bare) = setup_two_package_workspace("feat-only2");

    let wt_fe = tmp.path().join("worktrees/feat-only2/frontend");
    let wt_be = tmp.path().join("worktrees/feat-only2/backend");

    push_upstream_commit(&frontend_bare, "only-fe.txt", "only frontend");
    push_upstream_commit(&backend_bare, "only-be.txt", "only backend");

    let head_fe_before = git_head(&wt_fe);
    let head_be_before = git_head(&wt_be);

    // Only sync backend
    meldr()
        .args(["--no-tabs", "sync", "feat-only2", "--only", "backend"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("backend"));

    assert_eq!(
        head_fe_before,
        git_head(&wt_fe),
        "frontend should NOT be synced with --only backend"
    );
    assert_ne!(
        head_be_before,
        git_head(&wt_be),
        "backend should be synced with --only backend"
    );
}

#[test]
fn test_sync_exclude_with_multiple_packages() {
    let (tmp, _repos_dir, frontend_bare, backend_bare) = setup_two_package_workspace("feat-excl2");

    let wt_fe = tmp.path().join("worktrees/feat-excl2/frontend");
    let wt_be = tmp.path().join("worktrees/feat-excl2/backend");

    push_upstream_commit(&frontend_bare, "excl-fe.txt", "exclude fe");
    push_upstream_commit(&backend_bare, "excl-be.txt", "exclude be");

    let head_fe_before = git_head(&wt_fe);
    let head_be_before = git_head(&wt_be);

    // Exclude frontend
    meldr()
        .args(["--no-tabs", "sync", "feat-excl2", "--exclude", "frontend"])
        .current_dir(tmp.path())
        .assert()
        .success();

    assert_eq!(
        head_fe_before,
        git_head(&wt_fe),
        "frontend should NOT be synced with --exclude frontend"
    );
    assert_ne!(
        head_be_before,
        git_head(&wt_be),
        "backend should be synced with --exclude frontend"
    );
}

#[test]
fn test_sync_all_with_slash_branches() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "fm/branch-a"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "fm/branch-b"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_a = tmp.path().join("worktrees/fm-branch-a/frontend");
    let wt_b = tmp.path().join("worktrees/fm-branch-b/frontend");
    assert!(wt_a.exists(), "sanitized worktree dir a should exist");
    assert!(wt_b.exists(), "sanitized worktree dir b should exist");

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "slash-all.txt",
        "slash all",
    );

    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced"));

    assert!(wt_a.join("slash-all.txt").exists());
    assert!(wt_b.join("slash-all.txt").exists());
}

#[test]
fn test_sync_merge_with_local_commits() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-merge-local"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let wt_path = tmp.path().join("worktrees/feat-merge-local/frontend");

    // Make local commit
    make_local_commit(&wt_path, "local-merge.txt", "local for merge");

    // Push upstream change
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "upstream-merge.txt",
        "upstream for merge",
    );

    // Sync using merge method
    meldr()
        .args(["--no-tabs", "sync", "feat-merge-local", "--merge"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("synced").and(predicate::str::contains("merge")));

    // Both files should exist
    assert!(
        wt_path.join("local-merge.txt").exists(),
        "local commit should survive merge"
    );
    assert!(
        wt_path.join("upstream-merge.txt").exists(),
        "upstream changes should be merged"
    );
}

#[test]
fn test_sync_snapshot_contains_correct_data() {
    let (tmp, _repos_dir, frontend_bare, backend_bare) = setup_two_package_workspace("feat-snap");

    let wt_fe = tmp.path().join("worktrees/feat-snap/frontend");
    let wt_be = tmp.path().join("worktrees/feat-snap/backend");
    let head_fe_before = git_head(&wt_fe);
    let head_be_before = git_head(&wt_be);

    push_upstream_commit(&frontend_bare, "snap-fe.txt", "snapshot fe");
    push_upstream_commit(&backend_bare, "snap-be.txt", "snapshot be");

    meldr()
        .args(["--no-tabs", "sync", "feat-snap"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Verify snapshot file content
    let snapshot_dir = tmp.path().join(".meldr/sync-snapshots");
    assert!(snapshot_dir.exists());

    let mut snapshots: Vec<_> = fs::read_dir(&snapshot_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!snapshots.is_empty());

    // Read the latest snapshot
    snapshots.sort_by_key(|e| e.file_name());
    let latest = snapshots.last().unwrap();
    let content = fs::read_to_string(latest.path()).unwrap();

    // Snapshot should contain the pre-sync HEADs
    assert!(
        content.contains(&head_fe_before),
        "snapshot should contain frontend pre-sync HEAD"
    );
    assert!(
        content.contains(&head_be_before),
        "snapshot should contain backend pre-sync HEAD"
    );
    assert!(content.contains("feat-snap"));
}

// ── prompt-check ──────────────────────────────────────────────────────

#[test]
fn test_prompt_check_not_in_workspace() {
    let tmp = TempDir::new().unwrap();
    // No meldr.toml — should exit 0 silently
    meldr()
        .args(["prompt-check"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
}

#[test]
fn test_prompt_check_in_workspace_root() {
    let tmp = TempDir::new().unwrap();
    init_workspace(tmp.path());
    // In workspace root (not in a worktree) — silent exit
    meldr()
        .args(["prompt-check"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
}

#[test]
fn test_prompt_check_matching_branch() {
    let tmp = TempDir::new().unwrap();
    let repos = tmp.path().join("repos");
    fs::create_dir_all(&repos).unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();

    init_workspace(&ws);
    let repo_url = create_bare_repo(&repos, "pkg");
    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(&ws)
        .assert()
        .success();
    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-check"])
        .current_dir(&ws)
        .assert()
        .success();

    let wt_pkg = ws.join("worktrees").join("feat-check").join("pkg");
    // Branch should be feat-check, dir is feat-check — match, no output
    meldr()
        .args(["prompt-check"])
        .current_dir(&wt_pkg)
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
}

#[test]
fn test_prompt_check_mismatched_branch() {
    let tmp = TempDir::new().unwrap();
    let repos = tmp.path().join("repos");
    fs::create_dir_all(&repos).unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();

    init_workspace(&ws);
    let repo_url = create_bare_repo(&repos, "pkg");
    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(&ws)
        .assert()
        .success();
    meldr()
        .args(["--no-tabs", "worktree", "add", "feat-mismatch"])
        .current_dir(&ws)
        .assert()
        .success();

    let wt_pkg = ws.join("worktrees").join("feat-mismatch").join("pkg");

    // Switch to a different branch inside the worktree to create a mismatch
    let out = process::Command::new("git")
        .args(["checkout", "-b", "wrong-branch"])
        .current_dir(&wt_pkg)
        .output()
        .unwrap();
    assert!(out.status.success(), "git checkout failed: {out:?}");

    meldr()
        .args(["prompt-check"])
        .current_dir(&wt_pkg)
        .assert()
        .success()
        .stderr(predicate::str::contains("expected:feat-mismatch"));
}

/// Helper: get a specific ref's SHA from a repo.
fn git_rev_parse(path: &std::path::Path, rev: &str) -> String {
    let out = process::Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn test_sync_fast_forwards_bare_repo_main() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let bare_repo = tmp.path().join("packages/frontend");

    // Record bare repo's main before upstream changes
    let main_before = git_rev_parse(&bare_repo, "refs/heads/main");

    // Push an upstream commit
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "new-file.txt",
        "upstream change",
    );

    // Sync
    meldr()
        .args(["--no-tabs", "sync", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // After sync, bare repo's main should be fast-forwarded to match origin/main
    let main_after = git_rev_parse(&bare_repo, "refs/heads/main");
    let origin_main = git_rev_parse(&bare_repo, "refs/remotes/origin/main");

    assert_ne!(
        main_before, main_after,
        "Bare repo main should be updated after sync"
    );
    assert_eq!(
        main_after, origin_main,
        "Bare repo main should match origin/main after sync"
    );
}

#[test]
fn test_sync_all_fast_forwards_bare_repo_main() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-a"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let bare_repo = tmp.path().join("packages/frontend");

    // Push upstream commit
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "upstream.txt",
        "new content",
    );

    // Sync --all
    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Bare repo's main should now match origin/main
    let main_ref = git_rev_parse(&bare_repo, "refs/heads/main");
    let origin_main = git_rev_parse(&bare_repo, "refs/remotes/origin/main");
    assert_eq!(
        main_ref, origin_main,
        "Bare repo main should match origin/main after sync --all"
    );
}

#[test]
fn test_worktree_add_updates_main_first() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    let bare_repo = tmp.path().join("packages/frontend");
    let main_before = git_rev_parse(&bare_repo, "refs/heads/main");

    // Push a new commit to the upstream bare repo
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "new-feature.txt",
        "new feature content",
    );

    // Create a worktree — this should fetch + fast-forward main first
    meldr()
        .args(["--no-tabs", "worktree", "add", "fresh-branch"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Bare repo's main should now be at the latest commit
    let main_after = git_rev_parse(&bare_repo, "refs/heads/main");
    let origin_main = git_rev_parse(&bare_repo, "refs/remotes/origin/main");

    assert_ne!(
        main_before, main_after,
        "Bare repo main should be updated before worktree creation"
    );
    assert_eq!(
        main_after, origin_main,
        "Bare repo main should match origin/main after worktree add"
    );

    // The worktree should contain the new file (branched from latest main)
    let wt_path = tmp.path().join("worktrees/fresh-branch/frontend");
    assert!(
        wt_path.join("new-feature.txt").exists(),
        "Worktree should be based on latest main and contain the new file"
    );
}

#[test]
fn test_sync_updates_main_before_rebase() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["--no-tabs", "worktree", "add", "my-feature"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let bare_repo = tmp.path().join("packages/frontend");

    // Push upstream change
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "upstream-change.txt",
        "upstream",
    );

    // Sync — should update main first, then rebase
    meldr()
        .args(["--no-tabs", "sync", "my-feature"])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Bare repo main should match origin/main
    let main_ref = git_rev_parse(&bare_repo, "refs/heads/main");
    let origin_main = git_rev_parse(&bare_repo, "refs/remotes/origin/main");
    assert_eq!(
        main_ref, origin_main,
        "Bare repo main should be updated by sync"
    );

    // Worktree should have the upstream file after rebase
    let wt_path = tmp.path().join("worktrees/my-feature/frontend");
    assert!(
        wt_path.join("upstream-change.txt").exists(),
        "Worktree should contain upstream changes after sync"
    );
}

#[test]
fn test_sync_updates_main_even_with_skip_fetch_false() {
    // Verifies that sync --all fetches once at the top, not per-worktree
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Create two worktrees
    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-a"])
        .current_dir(tmp.path())
        .assert()
        .success();
    meldr()
        .args(["--no-tabs", "worktree", "add", "branch-b"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let bare_repo = tmp.path().join("packages/frontend");

    // Push upstream commit
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "shared-update.txt",
        "shared",
    );

    // Sync --all
    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Fetching packages and updating main branches",
        ));

    // Main should be updated
    let main_ref = git_rev_parse(&bare_repo, "refs/heads/main");
    let origin_main = git_rev_parse(&bare_repo, "refs/remotes/origin/main");
    assert_eq!(main_ref, origin_main);

    // Both worktrees should have the update
    let wt_a = tmp.path().join("worktrees/branch-a/frontend");
    let wt_b = tmp.path().join("worktrees/branch-b/frontend");
    assert!(wt_a.join("shared-update.txt").exists());
    assert!(wt_b.join("shared-update.txt").exists());
}

/// Helper: replace the bare-cloned package repo with a regular (non-bare) clone
/// that has `main` checked out, simulating workspaces set up before bare cloning.
fn convert_package_to_non_bare(workspace: &std::path::Path, pkg_name: &str, upstream_url: &str) {
    let pkg_path = workspace.join("packages").join(pkg_name);
    fs::remove_dir_all(&pkg_path).unwrap();
    process::Command::new("git")
        .args(["clone", upstream_url, pkg_path.to_str().unwrap()])
        .output()
        .unwrap();
    // Ensure remote tracking refs are populated
    process::Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(&pkg_path)
        .output()
        .unwrap();
}

#[test]
fn test_sync_fast_forwards_non_bare_checked_out_main() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Replace bare clone with a regular clone (main checked out)
    convert_package_to_non_bare(tmp.path(), "frontend", &repo_url);

    meldr()
        .args(["--no-tabs", "worktree", "add", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let pkg_repo = tmp.path().join("packages/frontend");
    let main_before = git_rev_parse(&pkg_repo, "refs/heads/main");

    // Push an upstream commit
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "new-file.txt",
        "upstream change",
    );

    // Sync should fast-forward main even though it's checked out
    meldr()
        .args(["--no-tabs", "sync", "feature-test"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let main_after = git_rev_parse(&pkg_repo, "refs/heads/main");
    let origin_main = git_rev_parse(&pkg_repo, "refs/remotes/origin/main");

    assert_ne!(
        main_before, main_after,
        "Non-bare repo main should be updated after sync"
    );
    assert_eq!(
        main_after, origin_main,
        "Non-bare repo main should match origin/main after sync"
    );
}

#[test]
fn test_worktree_add_works_with_non_bare_checked_out_main() {
    let tmp = TempDir::new().unwrap();
    let repos_dir = TempDir::new().unwrap();
    let repo_url = create_bare_repo(repos_dir.path(), "frontend");

    init_workspace(tmp.path());

    meldr()
        .args(["package", "add", &repo_url])
        .current_dir(tmp.path())
        .assert()
        .success();

    // Replace bare clone with a regular clone (main checked out)
    convert_package_to_non_bare(tmp.path(), "frontend", &repo_url);

    let pkg_repo = tmp.path().join("packages/frontend");
    let main_before = git_rev_parse(&pkg_repo, "refs/heads/main");

    // Push upstream commit
    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "new-feature.txt",
        "new feature content",
    );

    // Worktree add should fetch + fast-forward main, then create the worktree
    meldr()
        .args(["--no-tabs", "worktree", "add", "fresh-branch"])
        .current_dir(tmp.path())
        .assert()
        .success();

    let main_after = git_rev_parse(&pkg_repo, "refs/heads/main");
    let origin_main = git_rev_parse(&pkg_repo, "refs/remotes/origin/main");

    assert_ne!(
        main_before, main_after,
        "Non-bare repo main should be updated before worktree creation"
    );
    assert_eq!(
        main_after, origin_main,
        "Non-bare repo main should match origin/main after worktree add"
    );

    // The worktree should contain the new file
    let wt_path = tmp.path().join("worktrees/fresh-branch/frontend");
    assert!(
        wt_path.join("new-feature.txt").exists(),
        "Worktree should be based on latest main and contain the new file"
    );
}
