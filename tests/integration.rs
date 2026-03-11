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
    let tmp_clone = dir.join(format!("{}-tmp", name));
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

    assert!(tmp.path().join("worktrees/feature-test/frontend").exists());
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

    meldr()
        .args(["--no-tabs", "worktree", "remove", "feature-rm"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed worktree 'feature-rm'"));
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
        .stdout(
            predicate::str::contains("[frontend] hello"),
        );
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
        .stdout(
            predicate::str::contains("[frontend] from-root"),
        );
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
    assert!(tmp
        .path()
        .join("worktrees/fm-whatever/frontend")
        .exists());
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
        .stderr(predicate::str::contains("Could not detect current worktree"));
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
            .stdout(predicate::str::contains(format!("Set {} = {}", key, value)));

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
        "Bare clone should have fetch refspec for remote tracking, got: {}",
        refspec_str
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
        "HEAD should point to a remote tracking ref, got: {}",
        head_str
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
        .stderr(predicate::str::contains("Could not detect current worktree"));
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
        .stderr(predicate::str::contains("No active worktrees. Fetching all packages"));
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
        .stderr(predicate::str::contains("Could not detect current worktree"));
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
        .stdout(
            predicate::str::contains("frontend")
                .and(predicate::str::contains("up-to-date")),
        );
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

    // Now `status` should show the warning
    meldr()
        .args(["status"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("behind"));
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
        .args(["commit", "-m", &format!("add {}", filename)])
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

    push_upstream_commit(
        std::path::Path::new(&repo_url),
        "all-file.txt",
        "sync all",
    );

    meldr()
        .args(["--no-tabs", "sync", "--all"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("branch-a")
                .and(predicate::str::contains("branch-b")),
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

    let output = meldr()
        .args([
            "--no-tabs",
            "sync",
            "feature-test",
            "--only",
            "frontend",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("frontend"),
        "Output should mention frontend"
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

    let output = meldr()
        .args([
            "--no-tabs",
            "sync",
            "feature-test",
            "--exclude",
            "backend",
        ])
        .current_dir(tmp.path())
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("frontend"),
        "Output should mention frontend"
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
        .stdout(
            predicate::str::contains("Undoing")
                .and(predicate::str::contains("reset to")),
        );
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
