use assert_cmd::Command;
use tempfile::TempDir;
use std::fs;

#[allow(deprecated)]
#[test]
fn test_package_list_with_groups() {
    let dir = TempDir::new().unwrap();
    let manifest = r#"
[workspace]
name = "test-ws"

[[package]]
name = "pkg-a"
url = "https://example.com/a.git"
groups = ["backend"]
"#;
    fs::write(dir.path().join("meldr.toml"), manifest).unwrap();
    fs::create_dir_all(dir.path().join(".meldr")).unwrap();
    fs::write(dir.path().join(".meldr/state.json"), r#"{"worktrees":{}}"#).unwrap();

    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["package", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("pkg-a"));
}
