use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[allow(deprecated)]
#[test]
fn test_status_no_worktrees() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("meldr.toml"),
        r#"
[workspace]
name = "test-ws"

[[package]]
name = "api"
url = "https://example.com/api.git"
groups = ["backend"]
"#,
    )
    .unwrap();
    fs::create_dir_all(dir.path().join(".meldr")).unwrap();
    fs::write(dir.path().join(".meldr/state.json"), r#"{"worktrees":{}}"#).unwrap();

    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["status"])
        .assert()
        .success()
        .stdout(predicates::str::contains("test-ws"))
        .stdout(predicates::str::contains("api"))
        .stdout(predicates::str::contains("No active worktrees"));
}

#[allow(deprecated)]
#[test]
fn test_status_with_group_filter() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("meldr.toml"),
        r#"
[workspace]
name = "test-ws"

[[package]]
name = "api"
url = "https://example.com/api.git"
groups = ["backend"]

[[package]]
name = "web"
url = "https://example.com/web.git"
groups = ["frontend"]
"#,
    )
    .unwrap();
    fs::create_dir_all(dir.path().join(".meldr")).unwrap();
    fs::write(dir.path().join(".meldr/state.json"), r#"{"worktrees":{}}"#).unwrap();

    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["status", "--group", "backend"])
        .assert()
        .success()
        .stdout(predicates::str::contains("api"))
        .stdout(predicates::str::is_match("web").unwrap().not());
}
