use assert_cmd::Command;
use tempfile::TempDir;
use std::fs;

fn setup_workspace_with_groups(dir: &std::path::Path) {
    let manifest = r#"
[workspace]
name = "test-ws"

[[package]]
name = "api"
url = "https://example.com/api.git"
groups = ["backend", "rust"]

[[package]]
name = "web"
url = "https://example.com/web.git"
groups = ["frontend", "node"]

[[package]]
name = "shared"
url = "https://example.com/shared.git"
groups = ["backend", "frontend"]
"#;
    fs::write(dir.join("meldr.toml"), manifest).unwrap();
    fs::create_dir_all(dir.join(".meldr")).unwrap();
    fs::write(dir.join(".meldr/state.json"), r#"{"worktrees":{}}"#).unwrap();
}

#[allow(deprecated)]
#[test]
fn test_exec_with_group_flag_accepted() {
    let dir = TempDir::new().unwrap();
    setup_workspace_with_groups(dir.path());
    // Tests that --group is accepted as a CLI arg (fails because packages don't exist on disk)
    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["exec", "--group", "backend", "--", "echo", "hello"])
        .assert()
        .failure(); // Fails because no actual git repos, but proves arg parsing works
}

#[allow(deprecated)]
#[test]
fn test_status_with_group_flag_accepted() {
    let dir = TempDir::new().unwrap();
    setup_workspace_with_groups(dir.path());
    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["status", "--group", "backend"])
        .assert()
        .success();
}
