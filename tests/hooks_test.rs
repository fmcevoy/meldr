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

/// Run the bundled meldr-agent-notify.sh with DRY_RUN=1 and assert the printed
/// tmux commands contain the expected @cc_status value for each scenario.
/// Requires bash and jq on PATH (present on any dev/CI machine).
#[cfg(unix)]
mod notify_script_tests {
    use std::fs;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn script_path() -> std::path::PathBuf {
        let manifest = env!("CARGO_MANIFEST_DIR");
        std::path::Path::new(manifest).join("src/assets/meldr-agent-notify.sh")
    }

    fn run_notify(event: &str, hook_json: &str) -> String {
        let mut child = Command::new("bash")
            .arg(script_path())
            .arg(event)
            .env("MELDR_AGENT_NOTIFY_DRY_RUN", "1")
            .env("MELDR_TMUX_PANE", "%1")
            .env("MELDR_TMUX_WINDOW_ID", "@1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("bash must be available");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(hook_json.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn make_transcript(dir: &tempfile::TempDir, last_text: &str) -> String {
        let path = dir.path().join("transcript.jsonl");
        let line = format!(
            "{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}\n",
            last_text
        );
        fs::write(&path, line).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn stop_with_question_transcript_yields_waiting() {
        let dir = tempfile::TempDir::new().unwrap();
        let tf = make_transcript(&dir, "Which option do you prefer?");
        let hook = format!(
            r#"{{"session_id":"s1","cwd":"/tmp","hook_event_name":"Stop","transcript_path":"{}"}}"#,
            tf
        );
        let out = run_notify("stop", &hook);
        assert!(
            out.contains("@cc_status waiting"),
            "expected 'waiting' in: {out}"
        );
    }

    #[test]
    fn stop_with_needs_input_transcript_yields_waiting() {
        let dir = tempfile::TempDir::new().unwrap();
        let tf = make_transcript(&dir, "needs input: should I add tests here");
        let hook = format!(
            r#"{{"session_id":"s2","cwd":"/tmp","hook_event_name":"Stop","transcript_path":"{}"}}"#,
            tf
        );
        let out = run_notify("stop", &hook);
        assert!(
            out.contains("@cc_status waiting"),
            "expected 'waiting' in: {out}"
        );
    }

    #[test]
    fn stop_with_statement_transcript_yields_done() {
        let dir = tempfile::TempDir::new().unwrap();
        let tf = make_transcript(&dir, "All checks pass.");
        let hook = format!(
            r#"{{"session_id":"s3","cwd":"/tmp","hook_event_name":"Stop","transcript_path":"{}"}}"#,
            tf
        );
        let out = run_notify("stop", &hook);
        assert!(out.contains("@cc_status done"), "expected 'done' in: {out}");
    }

    #[test]
    fn stop_without_transcript_path_yields_done() {
        let hook = r#"{"session_id":"s4","cwd":"/tmp","hook_event_name":"Stop"}"#;
        let out = run_notify("stop", hook);
        assert!(
            out.contains("@cc_status done"),
            "expected 'done' fallback in: {out}"
        );
    }

    #[test]
    fn stop_with_ask_user_question_tool_yields_waiting() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("transcript.jsonl");
        let line = r#"{"role":"assistant","content":[{"type":"tool_use","name":"AskUserQuestion","id":"t1","input":{"questions":[]}}]}"#;
        fs::write(&path, format!("{line}\n")).unwrap();
        let hook = format!(
            r#"{{"session_id":"s5","cwd":"/tmp","hook_event_name":"Stop","transcript_path":"{}"}}"#,
            path.display()
        );
        let out = run_notify("stop", &hook);
        assert!(
            out.contains("@cc_status waiting"),
            "expected 'waiting' for AskUserQuestion in: {out}"
        );
    }

    #[test]
    fn notify_event_always_yields_waiting() {
        let hook = r#"{"session_id":"s6","cwd":"/tmp","hook_event_name":"Notification"}"#;
        let out = run_notify("notify", hook);
        assert!(
            out.contains("@cc_status waiting"),
            "expected 'waiting' for notify in: {out}"
        );
    }

    #[test]
    fn default_timeout_is_five_seconds() {
        let hook = r#"{"session_id":"s7","cwd":"/tmp","hook_event_name":"Stop"}"#;
        let out = run_notify("stop", hook);
        assert!(
            out.contains("clear timer after 5s"),
            "expected default timeout of 5s in: {out}"
        );
    }
}
