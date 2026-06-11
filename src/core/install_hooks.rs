use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{MeldrError, Result};

const MELDR_MARKER: &str = "_meldr";

fn hook_command(event: &str) -> String {
    match event {
        "Stop" => "meldr claude-hook stop".to_string(),
        "Notification" => "meldr claude-hook notify".to_string(),
        "SessionStart" => "meldr claude-hook session-start".to_string(),
        other => format!("meldr claude-hook {}", other.to_lowercase()),
    }
}

/// Remove the stale `meldr-agent-notify.sh` script from previous versions.
/// Silent: errors are ignored since the file may not exist.
pub fn remove_legacy_notify_script(home: &Path) {
    let path = home.join(".local/share/meldr/meldr-agent-notify.sh");
    let _ = std::fs::remove_file(path);
}

/// Returns true if `~/.claude/claude-session-start.sh` exists and is a symlink
/// (likely pointing into fmcevoy_tools). Used to warn the user during install.
pub fn legacy_session_start_symlink_present(home: &Path) -> bool {
    let path = home.join(".claude/claude-session-start.sh");
    path.symlink_metadata()
        .map(|m| m.file_type().is_symlink() || m.file_type().is_file())
        .unwrap_or(false)
}

/// Install meldr-managed hook entries into `~/.claude/settings.json`.
/// Existing user entries are preserved; meldr-tagged entries are updated in-place.
pub fn install_claude_hooks(home: &Path, dry_run: bool) -> Result<PathBuf> {
    let settings_path = resolve_settings_path(home)?;
    let mut root = read_settings(&settings_path)?;

    for event in &["Stop", "Notification"] {
        upsert_hook(&mut root, event, &hook_command(event));
    }
    upsert_hook_with_matcher(
        &mut root,
        "SessionStart",
        "startup",
        &hook_command("SessionStart"),
    );

    if dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&root).unwrap_or_default()
        );
    } else {
        write_settings_atomic(&settings_path, &root)?;
    }

    Ok(settings_path)
}

/// Remove all hook entries tagged with `_meldr: true`.
pub fn uninstall_claude_hooks(home: &Path, dry_run: bool) -> Result<PathBuf> {
    let settings_path = resolve_settings_path(home)?;
    let mut root = read_settings(&settings_path)?;

    for event in &["Stop", "Notification", "SessionStart"] {
        remove_meldr_hooks(&mut root, event);
    }

    if dry_run {
        println!(
            "{}",
            serde_json::to_string_pretty(&root).unwrap_or_default()
        );
    } else {
        write_settings_atomic(&settings_path, &root)?;
    }

    Ok(settings_path)
}

/// Returns true if a meldr-tagged entry exists for `event`.
pub fn hooks_installed(home: &Path, event: &str) -> bool {
    let Ok(settings_path) = resolve_settings_path(home) else {
        return false;
    };
    let Ok(root) = read_settings(&settings_path) else {
        return false;
    };
    find_meldr_hook(&root, event).is_some()
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn resolve_settings_path(home: &Path) -> Result<PathBuf> {
    let candidate = home.join(".claude/settings.json");
    if candidate.exists() {
        std::fs::canonicalize(&candidate).map_err(MeldrError::Io)
    } else {
        Ok(candidate)
    }
}

fn read_settings(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(MeldrError::Json)
}

fn write_settings_atomic(path: &Path, value: &Value) -> Result<()> {
    crate::core::fs_util::write_json_atomic(path, value)
}

/// Add or update the meldr hook entry for `event`. Updates an existing meldr-tagged
/// entry in-place, falling back to appending to the first matcher or building the
/// structure from scratch if needed.
fn upsert_hook(root: &mut Value, event: &str, command: &str) {
    let entry = json!({ "type": "command", "command": command, "_meldr": true });

    // Try to update an existing entry.
    if let Some(event_arr) = root
        .pointer_mut(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array_mut())
    {
        for matcher_obj in event_arr.iter_mut() {
            if let Some(hooks_arr) = matcher_obj
                .pointer_mut("/hooks")
                .and_then(|v| v.as_array_mut())
            {
                for hook in hooks_arr.iter_mut() {
                    if hook
                        .get(MELDR_MARKER)
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        *hook = entry.clone();
                        return;
                    }
                }
                // Append to the first matcher that had no matching entry.
                hooks_arr.push(entry.clone());
                return;
            }
        }
    }

    // Build the structure from scratch.
    let hooks_obj = root
        .as_object_mut()
        .expect("settings root must be an object");
    let event_arr = hooks_obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .and_then(|o| {
            o.entry(event)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .map(|a| a as *mut Vec<Value>)
        });

    if let Some(arr) = event_arr {
        // SAFETY: we hold a unique borrow of root through the chain above.
        let arr = unsafe { &mut *arr };
        if arr.is_empty() {
            arr.push(json!({ "matcher": "*", "hooks": [] }));
        }
        if let Some(inner) = arr
            .first_mut()
            .and_then(|m| m.pointer_mut("/hooks"))
            .and_then(|v| v.as_array_mut())
        {
            inner.push(entry);
        }
    }
}

/// Like `upsert_hook` but creates the matcher object with a specific matcher string
/// rather than `"*"`. Used for events like `SessionStart` where only `"startup"` is wanted.
fn upsert_hook_with_matcher(root: &mut Value, event: &str, matcher: &str, command: &str) {
    let entry = json!({ "type": "command", "command": command, "_meldr": true });

    // Update existing meldr-tagged entry if one exists (regardless of matcher).
    if let Some(event_arr) = root
        .pointer_mut(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array_mut())
    {
        for matcher_obj in event_arr.iter_mut() {
            if let Some(hooks_arr) = matcher_obj
                .pointer_mut("/hooks")
                .and_then(|v| v.as_array_mut())
            {
                for hook in hooks_arr.iter_mut() {
                    if hook
                        .get(MELDR_MARKER)
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        *hook = entry.clone();
                        return;
                    }
                }
                hooks_arr.push(entry.clone());
                return;
            }
        }
    }

    // Build from scratch with the specified matcher.
    let hooks_obj = root
        .as_object_mut()
        .expect("settings root must be an object");
    let event_arr = hooks_obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .and_then(|o| {
            o.entry(event)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .map(|a| a as *mut Vec<Value>)
        });

    if let Some(arr) = event_arr {
        // SAFETY: unique borrow of root through the chain above.
        let arr = unsafe { &mut *arr };
        if arr.is_empty() {
            arr.push(json!({ "matcher": matcher, "hooks": [] }));
        }
        if let Some(inner) = arr
            .first_mut()
            .and_then(|m| m.pointer_mut("/hooks"))
            .and_then(|v| v.as_array_mut())
        {
            inner.push(entry);
        }
    }
}

fn remove_meldr_hooks(root: &mut Value, event: &str) {
    let Some(event_arr) = root
        .pointer_mut(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array_mut())
    else {
        return;
    };
    for matcher_obj in event_arr.iter_mut() {
        if let Some(hooks_arr) = matcher_obj
            .pointer_mut("/hooks")
            .and_then(|v| v.as_array_mut())
        {
            hooks_arr.retain(|hook| {
                !hook
                    .get(MELDR_MARKER)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            });
        }
    }
}

fn find_meldr_hook<'a>(root: &'a Value, event: &str) -> Option<&'a Value> {
    root.pointer(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|matcher_obj| {
                matcher_obj
                    .pointer("/hooks")
                    .and_then(|v| v.as_array())
                    .and_then(|hooks_arr| {
                        hooks_arr.iter().find(|hook| {
                            hook.get(MELDR_MARKER)
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                        })
                    })
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_meldr_hooks() -> Value {
        json!({
            "hooks": {
                "Stop": [{"matcher": "*", "hooks": [{"type": "command", "command": "meldr claude-hook stop", "_meldr": true}]}],
                "Notification": [{"matcher": "*", "hooks": [{"type": "command", "command": "meldr claude-hook notify", "_meldr": true}]}]
            },
            "model": "opus"
        })
    }

    fn write_settings(dir: &Path, v: &Value) {
        let p = dir.join(".claude");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("settings.json"),
            serde_json::to_string_pretty(v).unwrap(),
        )
        .unwrap();
    }

    fn read_back(dir: &Path) -> Value {
        let text = std::fs::read_to_string(dir.join(".claude/settings.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn test_idempotent_double_install() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let hooks = root
            .pointer("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(hooks.len(), 1, "no duplicates after double install");
    }

    #[test]
    fn test_idempotent_from_existing_meldr_settings() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_settings(tmp.path(), &settings_with_meldr_hooks());

        install_claude_hooks(tmp.path(), false).unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let hooks = root
            .pointer("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            hooks.len(),
            1,
            "no duplicates after double install from existing"
        );
    }

    #[test]
    fn test_uninstall_removes_only_meldr_entries() {
        let mut root = settings_with_meldr_hooks();
        root.pointer_mut("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "command", "command": "bash ~/my-custom-hook.sh"}));

        remove_meldr_hooks(&mut root, "Stop");
        let hooks = root
            .pointer("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(hooks.len(), 1, "only user entry should remain");
        assert_eq!(hooks[0]["command"], "bash ~/my-custom-hook.sh");
    }

    #[test]
    fn test_install_into_missing_file_creates_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();
        let root = read_back(tmp.path());
        assert!(root.pointer("/hooks/Stop").is_some());
    }

    #[test]
    fn test_install_into_malformed_json_errors_safely() {
        let tmp = tempfile::TempDir::new().unwrap();
        let settings_dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(settings_dir.join("settings.json"), b"not json {{{").unwrap();
        let result = install_claude_hooks(tmp.path(), false);
        assert!(result.is_err());
        let on_disk = std::fs::read_to_string(settings_dir.join("settings.json")).unwrap();
        assert_eq!(
            on_disk, "not json {{{",
            "malformed file must not be overwritten"
        );
    }

    #[test]
    fn test_dry_run_writes_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), true).unwrap();
        assert!(!tmp.path().join(".claude/settings.json").exists());
    }

    #[test]
    fn test_uninstall_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_settings(tmp.path(), &settings_with_meldr_hooks());

        install_claude_hooks(tmp.path(), false).unwrap();
        uninstall_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let stop_hooks = root
            .pointer("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(
            stop_hooks.is_empty(),
            "meldr entry removed, nothing else left"
        );
    }

    #[test]
    fn test_install_adds_session_start_hook() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let hooks = root
            .pointer("/hooks/SessionStart/0/hooks")
            .expect("SessionStart entry must be present")
            .as_array()
            .unwrap();
        assert_eq!(hooks.len(), 1);
        assert!(
            hooks[0]["command"]
                .as_str()
                .unwrap()
                .contains("meldr claude-hook session-start"),
            "command must use meldr claude-hook session-start"
        );
        assert_eq!(hooks[0][MELDR_MARKER], true);
    }

    #[test]
    fn test_session_start_matcher_is_startup() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let matcher = root
            .pointer("/hooks/SessionStart/0/matcher")
            .and_then(|v| v.as_str())
            .expect("SessionStart matcher must be set");
        assert_eq!(matcher, "startup");
    }

    #[test]
    fn test_idempotent_session_start_hook() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let hooks = root
            .pointer("/hooks/SessionStart/0/hooks")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(hooks.len(), 1, "no duplicates on double install");
    }

    #[test]
    fn test_uninstall_removes_session_start_hook() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();
        uninstall_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let session_hooks = root
            .pointer("/hooks/SessionStart/0/hooks")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        assert_eq!(session_hooks, 0, "SessionStart hook must be removed");
    }

    #[test]
    fn test_hooks_installed_detects_session_start() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!hooks_installed(tmp.path(), "SessionStart"));
        install_claude_hooks(tmp.path(), false).unwrap();
        assert!(hooks_installed(tmp.path(), "SessionStart"));
    }

    #[test]
    fn test_migration_from_old_bash_script_commands() {
        // Settings with the legacy bash-script commands from a previous meldr version.
        let tmp = tempfile::TempDir::new().unwrap();
        let legacy = json!({
            "hooks": {
                "Stop": [{"matcher": "*", "hooks": [{"type": "command", "command": "bash ~/.local/share/meldr/meldr-agent-notify.sh stop", "_meldr": true}]}],
                "Notification": [{"matcher": "*", "hooks": [{"type": "command", "command": "bash ~/.local/share/meldr/meldr-agent-notify.sh notify", "_meldr": true}]}],
                "SessionStart": [{"matcher": "startup", "hooks": [{"type": "command", "command": "bash ~/.claude/claude-session-start.sh", "_meldr": true}]}]
            }
        });
        write_settings(tmp.path(), &legacy);

        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let stop_cmd = root
            .pointer("/hooks/Stop/0/hooks/0/command")
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(stop_cmd, "meldr claude-hook stop", "Stop must be migrated");

        let ss_cmd = root
            .pointer("/hooks/SessionStart/0/hooks/0/command")
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(
            ss_cmd.contains("meldr claude-hook session-start"),
            "SessionStart must be migrated: got {ss_cmd}"
        );

        // Ensure no duplicates were introduced.
        let stop_hooks = root
            .pointer("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(stop_hooks.len(), 1, "no duplicates after migration");
    }

    #[test]
    fn test_legacy_session_start_symlink_detection() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!legacy_session_start_symlink_present(tmp.path()));
        // Create the file (simulates the fmcevoy_tools-managed copy).
        let claude_dir = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("claude-session-start.sh"), "#!/bin/bash").unwrap();
        assert!(legacy_session_start_symlink_present(tmp.path()));
    }
}
