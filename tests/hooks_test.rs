use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

#[allow(deprecated)]
#[test]
fn test_hooks_in_manifest_parses() {
    let dir = TempDir::new().unwrap();
    let manifest = r#"
[workspace]
name = "test-ws"

[hooks]
post_sync = ["echo synced"]

[[package]]
name = "pkg-a"
url = "https://example.com/a.git"

[package.hooks]
post_worktree_create = ["echo created"]
"#;
    fs::write(dir.path().join("meldr.toml"), manifest).unwrap();
    fs::create_dir_all(dir.path().join(".meldr")).unwrap();
    fs::write(dir.path().join(".meldr/state.json"), r#"{"worktrees":{}}"#).unwrap();

    // Verify meldr can still parse the manifest and run commands with hooks present
    Command::cargo_bin("meldr")
        .unwrap()
        .current_dir(dir.path())
        .args(["package", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("pkg-a"));
}
