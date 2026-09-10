use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{MeldrError, Result};

const MELDR_MARKER: &str = "_meldr";

/// The events meldr owns, with the matcher and command each should carry.
///
/// Matchers matter as much as the commands. `Notification` used to be registered
/// with `*`, so routine events — `auth_success`, `agent_completed`, the
/// `quota_auto_resume_*` family — lit the tab as though the agent were blocked on
/// you. `SessionStart` used to be `startup` only, so resuming a session in a new
/// pane never re-recorded where that pane was.
pub const MELDR_HOOKS: &[(&str, &str, &str)] = &[
    ("Stop", "*", "meldr claude-hook stop"),
    (
        "Notification",
        "permission_prompt|idle_prompt|elicitation_dialog|elicitation_url_dialog|agent_needs_input",
        "meldr claude-hook notify",
    ),
    (
        "SessionStart",
        "startup|resume|clear|fork",
        "meldr claude-hook session-start",
    ),
];

fn canonical(event: &str) -> Option<(&'static str, &'static str)> {
    MELDR_HOOKS
        .iter()
        .find(|(e, _, _)| *e == event)
        .map(|(_, matcher, cmd)| (*matcher, *cmd))
}

/// Is this hook entry one of ours?
///
/// Recognises three shapes, because all three exist in the wild:
/// the `_meldr` marker; the marker placed on the *matcher* object instead of the
/// entry; and an untagged entry whose command is plainly a meldr hook. Without the
/// last two, a settings file that had lost its markers looked hook-free — so
/// `doctor` reported the hooks missing while they fired twice, and a reinstall
/// appended a third copy.
fn is_meldr_hook(hook: &Value) -> bool {
    if hook.get(MELDR_MARKER).and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    let Some(cmd) = hook.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    let cmd = cmd.trim();
    cmd.starts_with("meldr claude-hook")
        || cmd.contains("meldr-agent-notify.sh")
        || cmd.contains("claude-session-start.sh")
}

/// How an event's registration currently stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookState {
    /// No meldr entry at all.
    Missing,
    /// More than one meldr entry — every event fires that many times.
    Duplicated(usize),
    /// Exactly one entry, but under the wrong matcher.
    WrongMatcher {
        found: String,
        want: String,
    },
    Ok,
}

/// Inspect one event's registration without changing anything.
pub fn hook_state(home: &Path, event: &str) -> HookState {
    let Ok(path) = resolve_settings_path(home) else {
        return HookState::Missing;
    };
    let Ok(root) = read_settings(&path) else {
        return HookState::Missing;
    };
    hook_state_in(&root, event)
}

fn hook_state_in(root: &Value, event: &str) -> HookState {
    let groups = root
        .pointer(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array());
    let Some(groups) = groups else {
        return HookState::Missing;
    };

    let mut found: Vec<String> = Vec::new();
    for group in groups {
        let matcher = group
            .get("matcher")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let n = group
            .pointer("/hooks")
            .and_then(|v| v.as_array())
            .map(|hs| hs.iter().filter(|h| is_meldr_hook(h)).count())
            .unwrap_or(0);
        for _ in 0..n {
            found.push(matcher.clone());
        }
    }

    match found.len() {
        0 => HookState::Missing,
        1 => {
            let want = canonical(event).map(|(m, _)| m).unwrap_or("*");
            if found[0] == want {
                HookState::Ok
            } else {
                HookState::WrongMatcher {
                    found: found.remove(0),
                    want: want.to_string(),
                }
            }
        }
        n => HookState::Duplicated(n),
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

    for (event, matcher, command) in MELDR_HOOKS {
        set_sole_hook(&mut root, event, matcher, command);
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

/// Remove all hook entries tagged with `_meldr: true`.
pub fn uninstall_claude_hooks(home: &Path, dry_run: bool) -> Result<PathBuf> {
    let settings_path = resolve_settings_path(home)?;
    let mut root = read_settings(&settings_path)?;

    for (event, _, _) in MELDR_HOOKS {
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

// ── helpers ──────────────────────────────────────────────────────────────────

/// Where the hooks are written.
///
/// Deliberately *not* canonicalised. This path is very often a symlink into a
/// dotfiles repo, and resolving it made every install write through the link
/// into tracked source — which is how meldr's own hook block came to be
/// committed to fmcevoy_tools, serde key ordering and all. Writing to the link
/// path instead lets the atomic rename in `write_json_atomic` replace the
/// symlink with a real file: the repo is left alone, the live settings that
/// were reachable through the link are preserved (they are read first), and the
/// path self-heals on the first install.
fn resolve_settings_path(home: &Path) -> Result<PathBuf> {
    Ok(home.join(".claude/settings.json"))
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

/// Make `event` carry exactly one meldr hook, under `matcher`.
///
/// Every meldr-owned entry is stripped from every matcher group first, then a
/// single tagged entry is inserted. That is what makes the operation idempotent
/// against the states this file is actually found in: duplicated entries, entries
/// whose `_meldr` marker was dropped by another tool, and entries left under an
/// obsolete matcher. The previous implementation updated the first tagged entry it
/// found and appended when it found none, so an untagged duplicate survived every
/// reinstall and quietly doubled every sound and flash.
///
/// User-authored hooks are untouched.
fn set_sole_hook(root: &mut Value, event: &str, matcher: &str, command: &str) {
    remove_meldr_hooks(root, event);

    let entry = json!({ "type": "command", "command": command, MELDR_MARKER: true });
    let group = json!({ "matcher": matcher, "hooks": [entry] });

    let Some(obj) = root.as_object_mut() else {
        return;
    };
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    let Some(hooks) = hooks else {
        return;
    };
    match hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
    {
        Some(arr) => arr.insert(0, group),
        None => {
            hooks.insert(event.to_string(), json!([group]));
        }
    }
}

/// Remove every meldr-owned hook from `event`, dropping groups left empty.
fn remove_meldr_hooks(root: &mut Value, event: &str) {
    let Some(groups) = root
        .pointer_mut(&format!("/hooks/{event}"))
        .and_then(|v| v.as_array_mut())
    else {
        return;
    };

    for group in groups.iter_mut() {
        if let Some(hooks) = group.pointer_mut("/hooks").and_then(|v| v.as_array_mut()) {
            hooks.retain(|hook| !is_meldr_hook(hook));
        }
        // Some settings files carry the marker on the matcher object rather than
        // the entry; it is ours to remove either way.
        if let Some(obj) = group.as_object_mut() {
            obj.remove(MELDR_MARKER);
        }
    }

    // A group we emptied is meldr's own leftover, not a user's.
    groups.retain(|group| {
        group
            .pointer("/hooks")
            .and_then(|v| v.as_array())
            .map(|hs| !hs.is_empty())
            .unwrap_or(true)
    });

    if groups.is_empty()
        && let Some(hooks) = root.pointer_mut("/hooks").and_then(|v| v.as_object_mut())
    {
        hooks.remove(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A symlinked settings file must not be written through: the hooks belong
    /// in $HOME, not in whatever dotfiles repo the link points at.
    #[test]
    fn install_never_writes_through_a_symlinked_settings_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(&repo).unwrap();

        let tracked = repo.join("settings.json");
        std::fs::write(&tracked, "{\n  \"theme\": \"dark\"\n}\n").unwrap();
        let before = std::fs::read_to_string(&tracked).unwrap();

        let live = home.join(".claude/settings.json");
        std::os::unix::fs::symlink(&tracked, &live).unwrap();

        install_claude_hooks(&home, false).unwrap();

        // The tracked file is byte-for-byte untouched.
        assert_eq!(std::fs::read_to_string(&tracked).unwrap(), before);

        // The live path is a real file now, carrying the hooks.
        assert!(!live.symlink_metadata().unwrap().file_type().is_symlink());
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&live).unwrap()).unwrap();
        for (event, _, _) in MELDR_HOOKS {
            assert_eq!(hook_state_in(&root, event), HookState::Ok, "{event}");
        }

        // Settings that were only reachable through the link survive.
        assert_eq!(root["theme"], "dark");
    }

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
        // Uninstall drops the group it emptied, and with it the event key: an
        // empty `"Stop": [{"matcher":"*","hooks":[]}]` is meldr's litter, not the
        // user's configuration.
        assert!(
            root.pointer("/hooks/Stop").is_none(),
            "nothing of meldr's should remain: {root:#}"
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
    fn session_start_matcher_covers_resume_and_fork() {
        // `startup` alone meant a session resumed in a different pane never
        // re-recorded where that pane was, so it kept notifying the old one.
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let matcher = root
            .pointer("/hooks/SessionStart/0/matcher")
            .and_then(|v| v.as_str())
            .expect("SessionStart matcher must be set");
        assert_eq!(matcher, "startup|resume|clear|fork");
    }

    #[test]
    fn notification_matcher_excludes_non_blocking_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        install_claude_hooks(tmp.path(), false).unwrap();
        let root = read_back(tmp.path());
        let matcher = root
            .pointer("/hooks/Notification/0/matcher")
            .and_then(|v| v.as_str())
            .expect("Notification matcher must be set");
        for blocking in ["permission_prompt", "idle_prompt", "agent_needs_input"] {
            assert!(
                matcher.contains(blocking),
                "{matcher} should match {blocking}"
            );
        }
        // Under the old `*` matcher these lit a "waiting" tab for nothing.
        for noise in ["auth_success", "agent_completed", "quota_auto_resume"] {
            assert!(
                !matcher.contains(noise),
                "{matcher} should not match {noise}"
            );
        }
    }

    #[test]
    fn install_collapses_untagged_duplicates() {
        // The state actually found on this machine: every event carried two
        // identical entries and the `_meldr` markers had been stripped, so each
        // Stop fired twice and `hooks_installed` reported them missing.
        let tmp = tempfile::TempDir::new().unwrap();
        write_settings(
            tmp.path(),
            &json!({
                "hooks": {
                    "Stop": [{
                        "matcher": "*",
                        "hooks": [
                            {"type": "command", "command": "meldr claude-hook stop"},
                            {"type": "command", "command": "meldr claude-hook stop"}
                        ]
                    }]
                },
                "model": "opus"
            }),
        );

        assert_eq!(hook_state(tmp.path(), "Stop"), HookState::Duplicated(2));
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let groups = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        let total: usize = groups
            .iter()
            .map(|g| g.pointer("/hooks").unwrap().as_array().unwrap().len())
            .sum();
        assert_eq!(total, 1, "exactly one Stop hook must remain: {root:#}");
        assert_eq!(hook_state(tmp.path(), "Stop"), HookState::Ok);
        assert_eq!(root["model"], "opus", "unrelated settings preserved");
    }

    #[test]
    fn install_recognises_a_marker_on_the_matcher_object() {
        // Another shape seen in the wild: the marker sat on the group, not the entry.
        let tmp = tempfile::TempDir::new().unwrap();
        write_settings(
            tmp.path(),
            &json!({
                "hooks": {
                    "Stop": [{
                        "matcher": "*",
                        "_meldr": true,
                        "hooks": [{"type": "command", "command": "meldr claude-hook stop"}]
                    }]
                }
            }),
        );
        install_claude_hooks(tmp.path(), false).unwrap();
        let root = read_back(tmp.path());
        let groups = root.pointer("/hooks/Stop").unwrap().as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(
            groups[0].get(MELDR_MARKER).is_none(),
            "the stray group-level marker should be gone: {root:#}"
        );
        assert_eq!(hook_state(tmp.path(), "Stop"), HookState::Ok);
    }

    #[test]
    fn install_fixes_an_obsolete_matcher() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_settings(
            tmp.path(),
            &json!({
                "hooks": {
                    "SessionStart": [{
                        "matcher": "startup",
                        "hooks": [{"type": "command", "command": "meldr claude-hook session-start", "_meldr": true}]
                    }]
                }
            }),
        );
        assert!(matches!(
            hook_state(tmp.path(), "SessionStart"),
            HookState::WrongMatcher { .. }
        ));
        install_claude_hooks(tmp.path(), false).unwrap();
        assert_eq!(hook_state(tmp.path(), "SessionStart"), HookState::Ok);
    }

    #[test]
    fn install_preserves_user_hooks_in_other_groups() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_settings(
            tmp.path(),
            &json!({
                "hooks": {
                    "Stop": [
                        {"matcher": "*", "hooks": [
                            {"type": "command", "command": "meldr claude-hook stop"},
                            {"type": "command", "command": "bash ~/my-own-hook.sh"}
                        ]}
                    ]
                }
            }),
        );
        install_claude_hooks(tmp.path(), false).unwrap();

        let root = read_back(tmp.path());
        let all: Vec<String> = root
            .pointer("/hooks/Stop")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g.pointer("/hooks").unwrap().as_array().unwrap().clone())
            .map(|h| h["command"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(all.iter().any(|c| c == "bash ~/my-own-hook.sh"));
        assert_eq!(
            all.iter().filter(|c| c.starts_with("meldr ")).count(),
            1,
            "one meldr entry, user hook untouched: {all:?}"
        );
    }

    #[test]
    fn hook_state_reports_missing_for_an_empty_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(hook_state(tmp.path(), "Stop"), HookState::Missing);
    }

    #[test]
    fn untagged_meldr_command_counts_as_ours() {
        // Without this, `doctor` says "missing" about a hook that is firing.
        assert!(is_meldr_hook(&json!({"command": "meldr claude-hook stop"})));
        assert!(is_meldr_hook(
            &json!({"command": "meldr claude-hook notify"})
        ));
        assert!(!is_meldr_hook(&json!({"command": "bash ~/mine.sh"})));
        assert!(is_meldr_hook(
            &json!({"command": "bash ~/mine.sh", "_meldr": true})
        ));
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
    fn hook_state_detects_session_start_registration() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_ne!(hook_state(tmp.path(), "SessionStart"), HookState::Ok);
        install_claude_hooks(tmp.path(), false).unwrap();
        assert_eq!(hook_state(tmp.path(), "SessionStart"), HookState::Ok);
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
