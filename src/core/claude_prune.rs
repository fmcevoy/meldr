use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::core::sync_history;

/// Report returned by [`prune_for_removed_paths`].
pub struct ClaudePruneReport {
    /// Archive destinations that were created (one per matched project dir).
    pub archived: Vec<PathBuf>,
    /// Worktree paths for which `claude project purge` ran successfully.
    pub purged: Vec<PathBuf>,
    /// Non-fatal warnings collected during the operation.
    pub warnings: Vec<String>,
}

/// Encode an absolute path the way Claude Code does for its `~/.claude/projects/` directory:
/// every `/` is replaced with `-`. No other escaping is applied.
fn encode_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "-")
}

/// Return the current Unix timestamp as a string, suitable for use as an archive dir name.
pub fn format_timestamp() -> String {
    sync_history::unix_timestamp().to_string()
}

/// Archive Claude Code project state for `paths` that were just removed, then run
/// `claude project purge -y` to clean up residual config entries.
///
/// # Design
///
/// - **Infallible**: every filesystem or subprocess failure is appended to
///   `ClaudePruneReport::warnings` and never returned as an error, so this
///   function cannot block the surrounding worktree-removal flow.
/// - **Archive first, purge second**: the projects dir is moved to
///   `$HOME/.claude/projects-archive/<timestamp>/` before purge runs, so state
///   is recoverable even if the command fails.
/// - **Dependency-injectable**: `home` and `claude_bin` are explicit parameters
///   so tests can pass a tempdir and a shim script without touching global env.
///
/// # Arguments
///
/// - `home` — the user's home directory (injectable for tests).
/// - `claude_bin` — path or name of the `claude` binary (injectable for tests).
/// - `paths` — absolute per-package worktree paths that were just removed.
/// - `timestamp` — pre-computed archive subdir name (injectable for deterministic tests).
pub fn prune_for_removed_paths(
    home: &Path,
    claude_bin: &OsStr,
    paths: &[PathBuf],
    timestamp: &str,
) -> ClaudePruneReport {
    let mut report = ClaudePruneReport {
        archived: Vec::new(),
        purged: Vec::new(),
        warnings: Vec::new(),
    };

    if paths.is_empty() {
        return report;
    }

    let projects_dir = home.join(".claude").join("projects");
    let tasks_dir = home.join(".claude").join("tasks");
    let file_history_dir = home.join(".claude").join("file-history");
    let archive_root = home
        .join(".claude")
        .join("projects-archive")
        .join(timestamp);

    for path in paths {
        let encoded = encode_path(path);
        let project_dir = projects_dir.join(&encoded);

        if project_dir.exists() {
            // Collect session UUIDs from *.jsonl filenames before moving.
            let uuids = collect_uuids(&project_dir, &mut report.warnings);

            // Archive the project dir.
            let archive_projects_dir = archive_root.join("projects");
            match std::fs::create_dir_all(&archive_projects_dir) {
                Err(e) => report.warnings.push(format!(
                    "claude-prune: could not create archive dir '{}': {e}",
                    archive_projects_dir.display()
                )),
                Ok(()) => {
                    let dest = archive_projects_dir.join(&encoded);
                    match std::fs::rename(&project_dir, &dest) {
                        Err(e) => report.warnings.push(format!(
                            "claude-prune: could not archive '{}' → '{}': {e}",
                            project_dir.display(),
                            dest.display()
                        )),
                        Ok(()) => report.archived.push(dest),
                    }
                }
            }

            // Archive sibling tasks/ and file-history/ entries for the same session UUIDs.
            for uuid in &uuids {
                archive_subdir(
                    &tasks_dir,
                    uuid,
                    &archive_root.join("tasks"),
                    &mut report.warnings,
                );
                archive_subdir(
                    &file_history_dir,
                    uuid,
                    &archive_root.join("file-history"),
                    &mut report.warnings,
                );
            }
        }

        // Run `claude project purge -y <path>` to remove the config entry from
        // ~/.claude.json regardless of whether a project dir was found.
        match std::process::Command::new(claude_bin)
            .args(["project", "purge", "-y"])
            .arg(path)
            .output()
        {
            Ok(out) if out.status.success() => {
                report.purged.push(path.clone());
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                report.warnings.push(format!(
                    "claude-prune: `claude project purge` failed for '{}': {stderr}",
                    path.display()
                ));
            }
            Err(e) => {
                report.warnings.push(format!(
                    "claude-prune: could not run `claude project purge` for '{}': {e}",
                    path.display()
                ));
            }
        }
    }

    report
}

/// Read `<project_dir>/*.jsonl` and return the stems (session UUIDs).
fn collect_uuids(project_dir: &Path, warnings: &mut Vec<String>) -> Vec<String> {
    let rd = match std::fs::read_dir(project_dir) {
        Ok(rd) => rd,
        Err(e) => {
            warnings.push(format!(
                "claude-prune: could not read '{}': {e}",
                project_dir.display()
            ));
            return Vec::new();
        }
    };
    rd.flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            s.strip_suffix(".jsonl").map(|stem| stem.to_string())
        })
        .collect()
}

/// Move `<src_parent>/<name>` into `<dest_parent>/<name>`, creating `dest_parent` as needed.
/// Silently skips missing sources; appends to `warnings` on other failures.
fn archive_subdir(src_parent: &Path, name: &str, dest_parent: &Path, warnings: &mut Vec<String>) {
    let src = src_parent.join(name);
    if !src.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dest_parent) {
        warnings.push(format!(
            "claude-prune: could not create archive dir '{}': {e}",
            dest_parent.display()
        ));
        return;
    }
    let dest = dest_parent.join(name);
    if let Err(e) = std::fs::rename(&src, &dest) {
        warnings.push(format!(
            "claude-prune: could not archive '{}' → '{}': {e}",
            src.display(),
            dest.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn write_shim(dir: &Path, exit_code: i32) -> PathBuf {
        let shim = dir.join("claude");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\necho \"$@\" >> \"{}/claude-calls.log\"\nexit {exit_code}\n",
                dir.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
        shim
    }

    fn make_project_state(home: &Path, encoded: &str, uuids: &[&str]) {
        let proj_dir = home.join(".claude").join("projects").join(encoded);
        fs::create_dir_all(&proj_dir).unwrap();
        for uuid in uuids {
            fs::write(proj_dir.join(format!("{uuid}.jsonl")), b"{}").unwrap();
        }
    }

    fn make_task(home: &Path, uuid: &str) {
        let dir = home.join(".claude").join("tasks").join(uuid);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("task.json"), b"{}").unwrap();
    }

    fn make_file_history(home: &Path, uuid: &str) {
        let dir = home.join(".claude").join("file-history").join(uuid);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("history.json"), b"{}").unwrap();
    }

    #[test]
    fn test_encode_path_replaces_slashes() {
        let p = PathBuf::from("/Users/foo/ws/worktrees/feat/pkg");
        assert_eq!(encode_path(&p), "-Users-foo-ws-worktrees-feat-pkg");
    }

    #[test]
    fn test_encode_path_preserves_dots_and_underscores() {
        let p = PathBuf::from("/Users/foo/.claude/some_pkg");
        assert_eq!(encode_path(&p), "-Users-foo-.claude-some_pkg");
    }

    #[test]
    fn test_no_paths_returns_empty_report() {
        let tmp = TempDir::new().unwrap();
        let shim = write_shim(tmp.path(), 0);
        let report = prune_for_removed_paths(tmp.path(), shim.as_os_str(), &[], "20260515-120000");
        assert!(report.archived.is_empty());
        assert!(report.purged.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_archives_project_dir_and_sibling_state() {
        let tmp = TempDir::new().unwrap();
        let shim = write_shim(tmp.path(), 0);

        let wt_path = PathBuf::from("/Users/foo/ws/worktrees/feat/pkg");
        let encoded = encode_path(&wt_path);
        let uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

        make_project_state(tmp.path(), &encoded, &[uuid]);
        make_task(tmp.path(), uuid);
        make_file_history(tmp.path(), uuid);

        let report = prune_for_removed_paths(
            tmp.path(),
            shim.as_os_str(),
            std::slice::from_ref(&wt_path),
            "20260515-120000",
        );

        assert!(
            report.warnings.is_empty(),
            "unexpected warnings: {:?}",
            report.warnings
        );
        assert_eq!(report.archived.len(), 1);
        assert_eq!(report.purged.len(), 1);

        // Original project dir must be gone.
        assert!(
            !tmp.path()
                .join(".claude")
                .join("projects")
                .join(&encoded)
                .exists()
        );

        // Archive destination must exist.
        let archive_proj = tmp
            .path()
            .join(".claude")
            .join("projects-archive")
            .join("20260515-120000")
            .join("projects")
            .join(&encoded);
        assert!(
            archive_proj.exists(),
            "archive dir not found: {archive_proj:?}"
        );

        // Sibling task and file-history must be archived.
        let archive_task = tmp
            .path()
            .join(".claude")
            .join("projects-archive")
            .join("20260515-120000")
            .join("tasks")
            .join(uuid);
        assert!(archive_task.exists(), "task archive not found");

        let archive_fh = tmp
            .path()
            .join(".claude")
            .join("projects-archive")
            .join("20260515-120000")
            .join("file-history")
            .join(uuid);
        assert!(archive_fh.exists(), "file-history archive not found");
    }

    #[test]
    fn test_no_project_dir_still_runs_purge() {
        let tmp = TempDir::new().unwrap();
        let shim = write_shim(tmp.path(), 0);

        let wt_path = PathBuf::from("/Users/foo/ws/worktrees/feat/pkg");
        let report =
            prune_for_removed_paths(tmp.path(), shim.as_os_str(), &[wt_path], "20260515-120000");

        assert!(
            report.warnings.is_empty(),
            "warnings: {:?}",
            report.warnings
        );
        assert!(report.archived.is_empty());
        assert_eq!(report.purged.len(), 1);

        // Archive dir must NOT be created when nothing matched.
        assert!(!tmp.path().join(".claude").join("projects-archive").exists());
    }

    #[test]
    fn test_purge_failure_is_a_warning_not_an_error() {
        let tmp = TempDir::new().unwrap();
        let shim = write_shim(tmp.path(), 1); // exit 1

        let wt_path = PathBuf::from("/Users/foo/ws/worktrees/feat/pkg");
        let report =
            prune_for_removed_paths(tmp.path(), shim.as_os_str(), &[wt_path], "20260515-120000");

        assert_eq!(report.purged.len(), 0);
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("claude project purge"));
    }

    #[test]
    fn test_missing_claude_binary_is_a_warning() {
        let tmp = TempDir::new().unwrap();
        // Point at a path that doesn't exist.
        let nonexistent = tmp.path().join("no-such-claude");
        let wt_path = PathBuf::from("/Users/foo/ws/worktrees/feat/pkg");
        let report = prune_for_removed_paths(
            tmp.path(),
            nonexistent.as_os_str(),
            &[wt_path],
            "20260515-120000",
        );

        assert_eq!(report.purged.len(), 0);
        assert_eq!(report.warnings.len(), 1);
        assert!(
            report.warnings[0].contains("could not run"),
            "unexpected warning: {}",
            report.warnings[0]
        );
    }

    #[test]
    fn test_multiple_paths_all_processed() {
        let tmp = TempDir::new().unwrap();
        let shim = write_shim(tmp.path(), 0);

        let paths: Vec<PathBuf> = vec![
            PathBuf::from("/Users/foo/ws/worktrees/feat/pkg-a"),
            PathBuf::from("/Users/foo/ws/worktrees/feat/pkg-b"),
        ];

        for p in &paths {
            make_project_state(tmp.path(), &encode_path(p), &[]);
        }

        let report =
            prune_for_removed_paths(tmp.path(), shim.as_os_str(), &paths, "20260515-120000");

        assert!(
            report.warnings.is_empty(),
            "warnings: {:?}",
            report.warnings
        );
        assert_eq!(report.archived.len(), 2);
        assert_eq!(report.purged.len(), 2);
    }
}
