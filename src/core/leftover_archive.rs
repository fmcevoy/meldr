use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Report returned by [`archive_leftover`].
pub struct LeftoverArchiveReport {
    /// `(pkg_name, archive_dir)` pairs — one per package that had dirty content archived.
    pub archived_packages: Vec<(String, PathBuf)>,
    /// Non-fatal warnings collected during the operation.
    pub warnings: Vec<String>,
}

/// Archive uncommitted and untracked file content from dirty worktree packages.
///
/// For each `(pkg_name, wt_path, dirty_rel_paths)` tuple, this function:
/// 1. Creates `<home>/.meldr/archive/leftover/<branch>/<timestamp>/<pkg_name>/`.
/// 2. Copies each dirty file that exists on disk into the archive, preserving
///    the relative path structure. Files that are no longer on disk (deleted)
///    are silently skipped — their removal is captured in `CHANGES.patch`.
/// 3. Writes `CHANGES.patch` with the `git diff HEAD` output for that package
///    when a non-empty diff is present in `diffs`.
///
/// # Design
///
/// - **Infallible**: every filesystem failure is appended to `warnings` and never
///   returned as an error. The removal still proceeds.
/// - **Copy, not move**: uses `fs::copy` so the worktree state is preserved until
///   `git worktree remove` completes. Cross-filesystem paths are handled correctly.
/// - **Dependency-injectable**: `home` and `timestamp` are parameters so tests can
///   pass a `TempDir` and a fixed string without touching global state.
///
/// # Arguments
///
/// - `home` — the user's home directory.
/// - `branch` — the branch name being removed (used as the first path component).
/// - `packages` — `(pkg_name, wt_path, dirty_rel_paths)` tuples. Relative paths
///   are resolved against `wt_path` to find the source file.
/// - `diffs` — `pkg_name → git diff HEAD` text; only non-empty values are written.
/// - `timestamp` — archive subdirectory name (injectable for deterministic tests).
pub fn archive_leftover(
    home: &Path,
    branch: &str,
    packages: &[(String, PathBuf, Vec<PathBuf>)],
    diffs: &HashMap<String, String>,
    timestamp: &str,
) -> LeftoverArchiveReport {
    let mut report = LeftoverArchiveReport {
        archived_packages: Vec::new(),
        warnings: Vec::new(),
    };

    if packages.is_empty() {
        return report;
    }

    let archive_branch_ts = home
        .join(".meldr")
        .join("archive")
        .join("leftover")
        .join(branch)
        .join(timestamp);

    for (pkg_name, wt_path, dirty_rel_paths) in packages {
        let pkg_archive = archive_branch_ts.join(pkg_name);
        let mut archived_any = false;

        for rel_path in dirty_rel_paths {
            let src = wt_path.join(rel_path);
            if !src.exists() {
                // Deleted file — captured in CHANGES.patch; nothing to copy.
                continue;
            }

            let dest = pkg_archive.join(rel_path);
            let parent = dest.parent().unwrap_or(pkg_archive.as_path());
            if let Err(e) = std::fs::create_dir_all(parent) {
                report.warnings.push(format!(
                    "leftover-archive: could not create dir '{}': {e}",
                    parent.display()
                ));
                continue;
            }

            match std::fs::copy(&src, &dest) {
                Ok(_) => archived_any = true,
                Err(e) => report.warnings.push(format!(
                    "leftover-archive: could not copy '{}' → '{}': {e}",
                    src.display(),
                    dest.display()
                )),
            }
        }

        // Write CHANGES.patch when there is non-empty diff output.
        if let Some(diff) = diffs.get(pkg_name.as_str())
            && !diff.is_empty()
        {
            if let Err(e) = std::fs::create_dir_all(&pkg_archive) {
                report.warnings.push(format!(
                    "leftover-archive: could not create dir '{}': {e}",
                    pkg_archive.display()
                ));
            } else {
                let patch_path = pkg_archive.join("CHANGES.patch");
                match std::fs::write(&patch_path, diff.as_bytes()) {
                    Ok(()) => archived_any = true,
                    Err(e) => report.warnings.push(format!(
                        "leftover-archive: could not write patch '{}': {e}",
                        patch_path.display()
                    )),
                }
            }
        }

        if archived_any {
            report
                .archived_packages
                .push((pkg_name.clone(), pkg_archive));
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn make_package(
        wt_root: &Path,
        pkg_name: &str,
        files: &[(&str, &[u8])],
    ) -> (String, PathBuf, Vec<PathBuf>) {
        let wt_path = wt_root.join(pkg_name);
        fs::create_dir_all(&wt_path).unwrap();
        let mut rel_paths = Vec::new();
        for (rel, content) in files {
            let full = wt_path.join(rel);
            if let Some(p) = full.parent() {
                fs::create_dir_all(p).unwrap();
            }
            fs::write(&full, content).unwrap();
            rel_paths.push(PathBuf::from(rel));
        }
        (pkg_name.to_string(), wt_path, rel_paths)
    }

    #[test]
    fn archive_leftover_copies_files_preserving_structure() {
        let home = TempDir::new().unwrap();
        let wt_root = TempDir::new().unwrap();
        let pkg = make_package(
            wt_root.path(),
            "frontend",
            &[("new.txt", b"hello"), ("subdir/work.rs", b"fn main() {}")],
        );
        let packages = vec![pkg];
        let diffs = HashMap::new();

        let report = archive_leftover(home.path(), "feat-x", &packages, &diffs, "1234567890");

        assert!(
            report.warnings.is_empty(),
            "warnings: {:?}",
            report.warnings
        );
        assert_eq!(report.archived_packages.len(), 1);

        let pkg_archive = home
            .path()
            .join(".meldr/archive/leftover/feat-x/1234567890/frontend");
        assert_eq!(fs::read(pkg_archive.join("new.txt")).unwrap(), b"hello");
        assert_eq!(
            fs::read(pkg_archive.join("subdir/work.rs")).unwrap(),
            b"fn main() {}"
        );
    }

    #[test]
    fn archive_leftover_writes_changes_patch() {
        let home = TempDir::new().unwrap();
        let wt_root = TempDir::new().unwrap();
        let pkg = make_package(wt_root.path(), "backend", &[("mod.rs", b"code")]);
        let packages = vec![pkg.clone()];
        let mut diffs = HashMap::new();
        diffs.insert("backend".to_string(), "--- a/old\n+++ b/new\n".to_string());

        let report = archive_leftover(home.path(), "feat-x", &packages, &diffs, "ts");

        assert!(report.warnings.is_empty());
        let patch = home
            .path()
            .join(".meldr/archive/leftover/feat-x/ts/backend/CHANGES.patch");
        assert!(patch.exists());
        assert_eq!(fs::read_to_string(patch).unwrap(), "--- a/old\n+++ b/new\n");
    }

    #[test]
    fn archive_leftover_skips_clean_packages() {
        let home = TempDir::new().unwrap();
        let wt_root = TempDir::new().unwrap();
        // dirty package
        let dirty = make_package(wt_root.path(), "dirty", &[("file.txt", b"work")]);
        // clean package: empty dirty_rel_paths
        let clean = ("clean".to_string(), wt_root.path().join("clean"), vec![]);
        let diffs = HashMap::new();

        let report = archive_leftover(home.path(), "feat", &[dirty, clean], &diffs, "ts");

        assert_eq!(report.archived_packages.len(), 1);
        assert_eq!(report.archived_packages[0].0, "dirty");
        assert!(
            !home
                .path()
                .join(".meldr/archive/leftover/feat/ts/clean")
                .exists()
        );
    }

    #[test]
    fn archive_leftover_warns_on_missing_source_file() {
        let home = TempDir::new().unwrap();
        let wt_root = TempDir::new().unwrap();
        let wt_path = wt_root.path().join("pkg");
        fs::create_dir_all(&wt_path).unwrap();
        // Report a non-existent dirty path (simulates a deleted file)
        let packages = vec![("pkg".to_string(), wt_path, vec![PathBuf::from("ghost.txt")])];
        let diffs = HashMap::new();

        let report = archive_leftover(home.path(), "b", &packages, &diffs, "ts");

        // Ghost file: silently skipped (it's a deleted file, captured by patch)
        assert!(
            report.warnings.is_empty(),
            "deleted files should be silently skipped"
        );
        assert!(report.archived_packages.is_empty());
    }

    #[test]
    fn archive_leftover_empty_packages_returns_empty_report() {
        let home = TempDir::new().unwrap();
        let diffs = HashMap::new();
        let report = archive_leftover(home.path(), "b", &[], &diffs, "ts");
        assert!(report.archived_packages.is_empty());
        assert!(report.warnings.is_empty());
        assert!(
            !home.path().join(".meldr/archive").exists(),
            "no archive dir should be created"
        );
    }

    #[test]
    fn archive_leftover_no_duplicate_dirs_on_same_timestamp() {
        let home = TempDir::new().unwrap();
        let wt_root = TempDir::new().unwrap();
        let pkg1 = make_package(wt_root.path(), "pkg1", &[("a.txt", b"a")]);
        let pkg2 = make_package(wt_root.path(), "pkg2", &[("b.txt", b"b")]);
        let diffs = HashMap::new();

        let r1 = archive_leftover(home.path(), "b", &[pkg1], &diffs, "same-ts");
        let r2 = archive_leftover(home.path(), "b", &[pkg2], &diffs, "same-ts");

        // Both archives exist at the same timestamp dir — no collision error.
        assert!(r1.warnings.is_empty());
        assert!(r2.warnings.is_empty());
        assert!(
            home.path()
                .join(".meldr/archive/leftover/b/same-ts/pkg1/a.txt")
                .exists()
        );
        assert!(
            home.path()
                .join(".meldr/archive/leftover/b/same-ts/pkg2/b.txt")
                .exists()
        );
    }

    #[test]
    fn archive_leftover_empty_diff_does_not_write_patch() {
        let home = TempDir::new().unwrap();
        let wt_root = TempDir::new().unwrap();
        let pkg = make_package(wt_root.path(), "pkg", &[("f.txt", b"data")]);
        let mut diffs = HashMap::new();
        diffs.insert("pkg".to_string(), String::new()); // empty diff

        let report = archive_leftover(home.path(), "b", &[pkg], &diffs, "ts");

        assert!(report.warnings.is_empty());
        let patch = home
            .path()
            .join(".meldr/archive/leftover/b/ts/pkg/CHANGES.patch");
        assert!(!patch.exists(), "empty diff should not write CHANGES.patch");
    }
}
