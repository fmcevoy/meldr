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
fn test_exec() {
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
        .args(["exec", "echo", "hello"])
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--- frontend ---").and(predicate::str::contains("hello")),
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
        .stdout(predicate::str::contains("agent = cursor"));
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
    assert!(content.contains("# sync_strategy = \"theirs\""));
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
            .stdout(predicate::str::contains(format!("{} = {}", key, value)));
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

    // Set shell to bash and verify exec still works
    meldr()
        .args(["config", "set", "shell", "bash"])
        .current_dir(tmp.path())
        .assert()
        .success();

    meldr()
        .args(["exec", "echo", "works"])
        .current_dir(tmp.path())
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
