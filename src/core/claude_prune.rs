use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::core::sync_history;

/// Report returned by [`prune_for_worktree`].
pub struct ClaudePruneReport {
    /// Archive destinations that were created (one per matched project or job dir).
    pub archived: Vec<PathBuf>,
    /// Non-fatal warnings collected during the operation.
    pub warnings: Vec<String>,
}

/// Encode an absolute path the way Claude Code does for its `~/.claude/projects/` directory:
/// every `/` is replaced with `-`. No other escaping is applied.
pub fn encode_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "-")
}

/// Return the current Unix timestamp as a string, suitable for use as an archive dir name.
pub fn format_timestamp() -> String {
    sync_history::unix_timestamp().to_string()
}

/// Archive Claude Code state for a worktree that was just removed.
///
/// Scans `~/.claude/projects/` for project dirs whose encoded name matches any
/// package path under `worktree_branch_dir`, and `~/.claude/jobs/` for job dirs
/// whose `cwd` falls under `worktree_branch_dir`. Matched dirs are moved to
/// `$HOME/.claude/projects-archive/<timestamp>/`.
///
/// # Design
///
/// - **Infallible**: every filesystem failure is appended to `ClaudePruneReport::warnings`
///   and never returned as an error.
/// - **Archive first**: state is moved before any other action, so it is recoverable.
/// - **No shell-out**: does not invoke `claude project purge` — avoids the wall of
///   "No Claude Code project state found" warnings that the per-package call produced.
/// - **Dependency-injectable**: `home` is an explicit parameter so tests can pass a
///   tempdir without touching global env.
///
/// # Arguments
///
/// - `home` — the user's home directory (injectable for tests).
/// - `worktree_branch_dir` — the branch dir being removed (e.g. `<root>/worktrees/feat-x/`).
///   Used as a path prefix: any job whose `cwd` starts with this prefix is archived.
/// - `package_paths` — per-package worktree paths. Used to compute expected encoded
///   project-dir names via forward-encoding.
/// - `timestamp` — pre-computed archive subdir name (injectable for deterministic tests).
pub fn prune_for_worktree(
    home: &Path,
    worktree_branch_dir: &Path,
    package_paths: &[PathBuf],
    timestamp: &str,
) -> ClaudePruneReport {
    let mut report = ClaudePruneReport {
        archived: Vec::new(),
        warnings: Vec::new(),
    };

    if package_paths.is_empty() {
        return report;
    }

    let projects_dir = home.join(".claude").join("projects");
    let tasks_dir = home.join(".claude").join("tasks");
    let file_history_dir = home.join(".claude").join("file-history");
    let jobs_dir = home.join(".claude").join("jobs");
    let archive_root = home
        .join(".claude")
        .join("projects-archive")
        .join(timestamp);

    // Canonicalize the branch dir for reliable prefix-matching against job cwds.
    let canonical_branch_dir = worktree_branch_dir
        .canonicalize()
        .unwrap_or_else(|_| worktree_branch_dir.to_path_buf());

    // ── projects/ ───────────────────────────────────────────────────────────
    // Build the set of expected encoded names for all package paths.
    let encoded_names: std::collections::HashSet<String> =
        package_paths.iter().map(|p| encode_path(p)).collect();

    for encoded in &encoded_names {
        let project_dir = projects_dir.join(encoded);
        if !project_dir.exists() {
            continue;
        }

        let uuids = collect_uuids(&project_dir, &mut report.warnings);

        let archive_projects_dir = archive_root.join("projects");
        match std::fs::create_dir_all(&archive_projects_dir) {
            Err(e) => report.warnings.push(format!(
                "claude-prune: could not create archive dir '{}': {e}",
                archive_projects_dir.display()
            )),
            Ok(()) => {
                let dest = archive_projects_dir.join(encoded);
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

    // ── jobs/ ────────────────────────────────────────────────────────────────
    archive_stale_jobs(
        &jobs_dir,
        &canonical_branch_dir,
        &archive_root.join("jobs"),
        &mut report.archived,
        &mut report.warnings,
    );

    report
}

/// Walk `~/.claude/jobs/` and archive any job whose `state.json` `cwd` field is a
/// descendant of `branch_prefix`. Also archives job dirs that have no `state.json`
/// and are older than 1 hour (safe to treat as orphans).
fn archive_stale_jobs(
    jobs_dir: &Path,
    branch_prefix: &Path,
    archive_jobs_dir: &Path,
    archived: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) {
    if !jobs_dir.exists() {
        return;
    }
    let rd = match std::fs::read_dir(jobs_dir) {
        Ok(rd) => rd,
        Err(e) => {
            warnings.push(format!(
                "claude-prune: could not read jobs dir '{}': {e}",
                jobs_dir.display()
            ));
            return;
        }
    };

    for entry in rd.flatten() {
        let job_dir = entry.path();
        if !job_dir.is_dir() {
            continue;
        }
        let state_path = job_dir.join("state.json");
        if state_path.exists() {
            let should_archive = job_cwd_under_prefix(&state_path, branch_prefix, warnings);
            if should_archive {
                move_job_dir(&job_dir, archive_jobs_dir, archived, warnings);
            }
        } else if is_older_than_one_hour(&job_dir) {
            // No state.json and old enough — orphan job dir.
            move_job_dir(&job_dir, archive_jobs_dir, archived, warnings);
        }
    }
}

/// Read a job's `state.json` and return true if its `cwd` field is a descendant
/// of `branch_prefix` (canonicalized comparison).
fn job_cwd_under_prefix(
    state_path: &Path,
    branch_prefix: &Path,
    warnings: &mut Vec<String>,
) -> bool {
    let contents = match std::fs::read_to_string(state_path) {
        Ok(s) => s,
        Err(e) => {
            warnings.push(format!(
                "claude-prune: could not read '{}': {e}",
                state_path.display()
            ));
            return false;
        }
    };
    let val: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let cwd_str = match val.get("cwd").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return false,
    };
    let cwd = PathBuf::from(cwd_str);
    let canonical_cwd = cwd.canonicalize().unwrap_or(cwd);
    canonical_cwd.starts_with(branch_prefix)
}

/// Return true if `dir`'s metadata mtime is more than 1 hour ago.
fn is_older_than_one_hour(dir: &Path) -> bool {
    std::fs::metadata(dir)
        .and_then(|m| m.modified())
        .map(|mtime| {
            SystemTime::now()
                .duration_since(mtime)
                .unwrap_or(Duration::ZERO)
                > Duration::from_secs(3600)
        })
        .unwrap_or(false)
}

/// Move a job directory into the archive, creating the destination as needed.
fn move_job_dir(
    job_dir: &Path,
    archive_jobs_dir: &Path,
    archived: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) {
    if let Err(e) = std::fs::create_dir_all(archive_jobs_dir) {
        warnings.push(format!(
            "claude-prune: could not create archive dir '{}': {e}",
            archive_jobs_dir.display()
        ));
        return;
    }
    let name = job_dir.file_name().unwrap_or_default();
    let dest = archive_jobs_dir.join(name);
    match std::fs::rename(job_dir, &dest) {
        Ok(()) => archived.push(dest),
        Err(e) => warnings.push(format!(
            "claude-prune: could not archive '{}' → '{}': {e}",
            job_dir.display(),
            dest.display()
        )),
    }
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
    use tempfile::TempDir;

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

    fn make_job(home: &Path, id: &str, cwd: &Path) {
        let dir = home.join(".claude").join("jobs").join(id);
        fs::create_dir_all(&dir).unwrap();
        let state = serde_json::json!({ "cwd": cwd.to_string_lossy() });
        fs::write(dir.join("state.json"), state.to_string()).unwrap();
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
        let branch_dir = tmp.path().join("worktrees").join("feat");
        let report = prune_for_worktree(tmp.path(), &branch_dir, &[], "20260515-120000");
        assert!(report.archived.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_archives_project_dir_and_sibling_state() {
        let tmp = TempDir::new().unwrap();
        let branch_dir = tmp.path().join("worktrees").join("feat");
        let wt_path = branch_dir.join("pkg");

        let encoded = encode_path(&wt_path);
        let uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

        make_project_state(tmp.path(), &encoded, &[uuid]);
        make_task(tmp.path(), uuid);
        make_file_history(tmp.path(), uuid);

        let report = prune_for_worktree(
            tmp.path(),
            &branch_dir,
            std::slice::from_ref(&wt_path),
            "20260515-120000",
        );

        assert!(
            report.warnings.is_empty(),
            "unexpected warnings: {:?}",
            report.warnings
        );
        assert_eq!(report.archived.len(), 1);

        assert!(
            !tmp.path()
                .join(".claude")
                .join("projects")
                .join(&encoded)
                .exists()
        );

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
    fn test_no_project_dir_produces_no_warning() {
        let tmp = TempDir::new().unwrap();
        let branch_dir = tmp.path().join("worktrees").join("feat");
        let wt_path = branch_dir.join("pkg");

        let report = prune_for_worktree(
            tmp.path(),
            &branch_dir,
            std::slice::from_ref(&wt_path),
            "20260515-120000",
        );

        assert!(
            report.warnings.is_empty(),
            "expected no warnings, got: {:?}",
            report.warnings
        );
        assert!(report.archived.is_empty());
        assert!(!tmp.path().join(".claude").join("projects-archive").exists());
    }

    #[test]
    fn test_archives_job_whose_cwd_is_under_branch_dir() {
        let tmp = TempDir::new().unwrap();

        // Create an on-disk directory so canonicalize works.
        let branch_dir = tmp.path().join("worktrees").join("feat");
        let pkg_path = branch_dir.join("mypkg");
        fs::create_dir_all(&pkg_path).unwrap();

        // Job whose cwd is inside the worktree.
        let job_cwd = pkg_path.join("src");
        fs::create_dir_all(&job_cwd).unwrap();
        make_job(tmp.path(), "job-abc", &job_cwd);

        let report = prune_for_worktree(
            tmp.path(),
            &branch_dir,
            std::slice::from_ref(&pkg_path),
            "20260515-120000",
        );

        assert!(
            report.warnings.is_empty(),
            "unexpected warnings: {:?}",
            report.warnings
        );
        assert_eq!(report.archived.len(), 1);
        let expected_archive = tmp
            .path()
            .join(".claude")
            .join("projects-archive")
            .join("20260515-120000")
            .join("jobs")
            .join("job-abc");
        assert!(
            expected_archive.exists(),
            "job archive not found at {expected_archive:?}"
        );
        assert!(
            !tmp.path()
                .join(".claude")
                .join("jobs")
                .join("job-abc")
                .exists(),
            "original job dir should be gone"
        );
    }

    #[test]
    fn test_job_outside_branch_dir_not_archived() {
        let tmp = TempDir::new().unwrap();

        let branch_dir = tmp.path().join("worktrees").join("feat");
        let pkg_path = branch_dir.join("mypkg");
        fs::create_dir_all(&pkg_path).unwrap();

        // Job whose cwd is NOT inside the worktree.
        let other_dir = tmp.path().join("other-project").join("src");
        fs::create_dir_all(&other_dir).unwrap();
        make_job(tmp.path(), "job-xyz", &other_dir);

        let report = prune_for_worktree(
            tmp.path(),
            &branch_dir,
            std::slice::from_ref(&pkg_path),
            "20260515-120000",
        );

        assert!(report.warnings.is_empty());
        assert!(report.archived.is_empty());
        assert!(
            tmp.path()
                .join(".claude")
                .join("jobs")
                .join("job-xyz")
                .exists(),
            "unrelated job should remain"
        );
    }

    #[test]
    fn test_multiple_paths_all_processed() {
        let tmp = TempDir::new().unwrap();
        let branch_dir = tmp.path().join("worktrees").join("feat");

        let paths: Vec<PathBuf> = vec![branch_dir.join("pkg-a"), branch_dir.join("pkg-b")];

        for p in &paths {
            make_project_state(tmp.path(), &encode_path(p), &[]);
        }

        let report = prune_for_worktree(tmp.path(), &branch_dir, &paths, "20260515-120000");

        assert!(
            report.warnings.is_empty(),
            "warnings: {:?}",
            report.warnings
        );
        assert_eq!(report.archived.len(), 2);
    }
}
