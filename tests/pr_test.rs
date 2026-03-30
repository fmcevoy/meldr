use assert_cmd::Command;

#[allow(deprecated)]
#[test]
fn test_pr_create_help() {
    Command::cargo_bin("meldr")
        .unwrap()
        .args(["pr", "create", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--title"))
        .stdout(predicates::str::contains("--draft"))
        .stdout(predicates::str::contains("--group"));
}

#[allow(deprecated)]
#[test]
fn test_pr_status_help() {
    Command::cargo_bin("meldr")
        .unwrap()
        .args(["pr", "status", "--help"])
        .assert()
        .success();
}
