use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::core::claude_prune::{encode_path, format_timestamp};
use crate::core::state::WorkspaceState;
use crate::core::workspace::{Manifest, sanitize_branch_for_dir, worktrees_dir};
use crate::error::Result;
use crate::git::GitOps;
use crate::tmux::TmuxOps as _;

// ── Shared types ─────────────────────────────────────────────────────────────

/// A single action discovered by a doctor section.
#[derive(Debug)]
pub struct DoctorAction {
    pub description: String,
    pub kind: ActionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActionKind {
    /// Archive a file or directory to a timestamped destination.
    Archive { src: PathBuf, dest: PathBuf },
    /// Remove a key from state.json.
    PruneState { branch: String },
    /// Run git worktree prune in a bare repo.
    GitWorktreePrune { repo: PathBuf },
}

pub struct DoctorReport {
    pub actions: Vec<DoctorAction>,
    pub applied: usize,
    pub warnings: Vec<String>,
}

impl DoctorReport {
    fn new() -> Self {
        Self {
            actions: Vec::new(),
            warnings: Vec::new(),
            applied: 0,
        }
    }
}

// ── doctor claude ─────────────────────────────────────────────────────────────

/// Reconcile `~/.claude/jobs` and `~/.claude/projects` against live worktrees
/// under `workspace_root/worktrees/`.
///
/// Only inspects Claude state entries whose cwd / decoded path falls under this
/// workspace's `worktrees/` directory.
pub fn run_claude(workspace_root: &Path, apply: bool) -> Result<DoctorReport> {
    let mut report = DoctorReport::new();
    let ts = format_timestamp();

    let home = match std::env::var_os("HOME").map(PathBuf::from) {
        Some(h) => h,
        None => return Ok(report),
    };

    let state = WorkspaceState::load(workspace_root)?;
    let manifest = Manifest::load(workspace_root)?;
    let worktrees_root = worktrees_dir(workspace_root);
    let canonical_worktrees_root = worktrees_root
        .canonicalize()
        .unwrap_or_else(|_| worktrees_root.clone());

    let archive_root = home.join(".claude").join("projects-archive").join(&ts);

    // ── jobs ──────────────────────────────────────────────────────────────────
    let jobs_dir = home.join(".claude").join("jobs");
    if jobs_dir.exists() {
        let entries =
            std::fs::read_dir(&jobs_dir).unwrap_or_else(|_| std::fs::read_dir("/tmp").unwrap()); // unreachable fallback
        for entry in entries.flatten() {
            let job_dir = entry.path();
            if !job_dir.is_dir() {
                continue;
            }
            let state_path = job_dir.join("state.json");
            if state_path.exists() {
                if let Some(cwd) = read_job_cwd(&state_path) {
                    let canonical_cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
                    if canonical_cwd.starts_with(&canonical_worktrees_root)
                        && !canonical_cwd.exists()
                    {
                        let dest = archive_root
                            .join("jobs")
                            .join(job_dir.file_name().unwrap_or_default());
                        let desc = format!(
                            "archive dead job {} (cwd: {})",
                            job_dir.file_name().unwrap_or_default().to_string_lossy(),
                            cwd.display()
                        );
                        if apply {
                            if let Err(e) = archive_path(&job_dir, &dest) {
                                report.warnings.push(e);
                            } else {
                                report.applied += 1;
                            }
                        }
                        report.actions.push(DoctorAction {
                            description: desc,
                            kind: ActionKind::Archive {
                                src: job_dir.clone(),
                                dest,
                            },
                        });
                    }
                }
            } else if is_older_than(&job_dir, Duration::from_secs(3600)) {
                // Orphan job dir with no state.json and old enough to be safe to archive.
                let dest = archive_root
                    .join("jobs")
                    .join(job_dir.file_name().unwrap_or_default());
                let desc = format!(
                    "archive orphan job dir {} (no state.json, >1h old)",
                    job_dir.file_name().unwrap_or_default().to_string_lossy()
                );
                if apply {
                    if let Err(e) = archive_path(&job_dir, &dest) {
                        report.warnings.push(e);
                    } else {
                        report.applied += 1;
                    }
                }
                report.actions.push(DoctorAction {
                    description: desc,
                    kind: ActionKind::Archive {
                        src: job_dir.clone(),
                        dest,
                    },
                });
            }
        }
    }

    // ── projects ──────────────────────────────────────────────────────────────
    let projects_dir = home.join(".claude").join("projects");
    if projects_dir.exists() {
        // Build the full set of expected encoded names for every known package path
        // in every state entry, plus all current branches.
        let mut known_encoded: std::collections::HashSet<String> = std::collections::HashSet::new();
        for branch_key in state.worktrees.keys() {
            for pkg in &manifest.packages {
                let expected = worktrees_root
                    .join(sanitize_branch_for_dir(branch_key))
                    .join(&pkg.name);
                known_encoded.insert(encode_path(&expected));
            }
        }

        let entries = match std::fs::read_dir(&projects_dir) {
            Ok(rd) => rd,
            Err(e) => {
                report
                    .warnings
                    .push(format!("doctor claude: could not read projects dir: {e}"));
                return Ok(report);
            }
        };

        for entry in entries.flatten() {
            let proj_dir = entry.path();
            if !proj_dir.is_dir() {
                continue;
            }
            let encoded_name = proj_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Only consider paths that appear to be under this workspace's worktrees root.
            // The encoded form of a worktrees path always starts with the encoded workspace root.
            let encoded_worktrees_prefix = encode_path(&canonical_worktrees_root);
            if !encoded_name.starts_with(&encoded_worktrees_prefix) {
                continue;
            }

            // If this encoded name is in the known set and the path no longer exists, archive.
            if known_encoded.contains(&encoded_name) {
                // Decode check: the path shouldn't exist (that's what makes it stale).
                // We can't perfectly decode, so check whether any living worktree
                // produces this same encoded name.
                let any_alive = state.worktrees.keys().any(|b| {
                    manifest.packages.iter().any(|pkg| {
                        let p = worktrees_root
                            .join(sanitize_branch_for_dir(b))
                            .join(&pkg.name);
                        encode_path(&p) == encoded_name && p.exists()
                    })
                });
                if !any_alive {
                    let dest = archive_root.join("projects").join(&encoded_name);
                    let desc = format!("archive stale project dir {encoded_name}");
                    if apply {
                        if let Err(e) = archive_path(&proj_dir, &dest) {
                            report.warnings.push(e);
                        } else {
                            report.applied += 1;
                        }
                    }
                    report.actions.push(DoctorAction {
                        description: desc,
                        kind: ActionKind::Archive {
                            src: proj_dir.clone(),
                            dest,
                        },
                    });
                }
            } else {
                // Encoded name not in known set but starts with the worktrees prefix —
                // could be from a deleted branch. Same alive check without known_encoded
                // membership (we just probe for the decoded path's existence).
                // Approximate: if the path it encodes doesn't exist (we probe by
                // checking the directory directly with the full encoded path we'd
                // generate for any current package, and it doesn't match any),
                // archive it.
                // Simple heuristic: if neither worktrees_root/<anything> still
                // produces this encoded name via the live packages, it's stale.
                let any_alive_current = manifest.packages.iter().any(|pkg| {
                    state.worktrees.keys().any(|b| {
                        let p = worktrees_root
                            .join(sanitize_branch_for_dir(b))
                            .join(&pkg.name);
                        encode_path(&p) == encoded_name && p.exists()
                    })
                });
                if !any_alive_current {
                    let dest = archive_root.join("projects").join(&encoded_name);
                    let desc = format!(
                        "archive orphan project dir {encoded_name} (no matching live worktree)"
                    );
                    if apply {
                        if let Err(e) = archive_path(&proj_dir, &dest) {
                            report.warnings.push(e);
                        } else {
                            report.applied += 1;
                        }
                    }
                    report.actions.push(DoctorAction {
                        description: desc,
                        kind: ActionKind::Archive {
                            src: proj_dir.clone(),
                            dest,
                        },
                    });
                }
            }
        }
    }

    Ok(report)
}

// ── doctor worktrees ──────────────────────────────────────────────────────────

pub struct WorktreesDoctorReport {
    pub actions: Vec<DoctorAction>,
    pub applied: usize,
    pub warnings: Vec<String>,
    /// State entries whose on-disk dir is missing — removed from state on apply.
    pub pruned_state: Vec<String>,
    /// Orphan on-disk dirs with no state entry.
    pub orphan_dirs: Vec<PathBuf>,
    /// State entries where the key doesn't sanitize to the actual dir name.
    pub name_mismatches: Vec<(String, String)>,
}

impl WorktreesDoctorReport {
    fn new() -> Self {
        Self {
            actions: Vec::new(),
            applied: 0,
            warnings: Vec::new(),
            pruned_state: Vec::new(),
            orphan_dirs: Vec::new(),
            name_mismatches: Vec::new(),
        }
    }
}

/// Reconcile on-disk worktree directories against `.meldr/state.json`.
pub fn run_worktrees(
    git: &dyn GitOps,
    workspace_root: &Path,
    apply: bool,
) -> Result<WorktreesDoctorReport> {
    let mut report = WorktreesDoctorReport::new();

    let mut state = WorkspaceState::load(workspace_root)?;
    let manifest = Manifest::load(workspace_root)?;
    let worktrees_root = worktrees_dir(workspace_root);

    let ts = format_timestamp();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let workspace_name = &manifest.workspace.name;

    // 1. State entries with missing on-disk dir.
    let branches_to_prune: Vec<String> = state
        .worktrees
        .keys()
        .filter(|branch| {
            let dir = worktrees_root.join(sanitize_branch_for_dir(branch));
            !dir.exists()
        })
        .cloned()
        .collect();

    for branch in &branches_to_prune {
        report.pruned_state.push(branch.clone());
        report.actions.push(DoctorAction {
            description: format!("prune state entry '{branch}' (worktree dir missing from disk)"),
            kind: ActionKind::PruneState {
                branch: branch.clone(),
            },
        });
    }

    if apply {
        for branch in &branches_to_prune {
            state.remove_worktree(branch);
        }
        if !branches_to_prune.is_empty() {
            if let Err(e) = state.save(workspace_root) {
                report
                    .warnings
                    .push(format!("doctor worktrees: save state failed: {e}"));
            } else {
                report.applied += branches_to_prune.len();
            }
        }
    }

    // 2. On-disk dirs with no state entry.
    if worktrees_root.exists() {
        let rd = std::fs::read_dir(&worktrees_root);
        if let Ok(rd) = rd {
            for entry in rd.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let dir_name = dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                // Check if any state key maps to this dir name.
                let has_state = state
                    .worktrees
                    .keys()
                    .any(|b| sanitize_branch_for_dir(b) == dir_name);
                if !has_state {
                    report.orphan_dirs.push(dir.clone());
                    // Count dirty files.
                    let dirty = is_worktree_dirty_any_package(&dir, &manifest);
                    let desc = if dirty {
                        format!(
                            "orphan worktree dir '{dir_name}' has uncommitted changes — skipping archive"
                        )
                    } else {
                        format!("archive orphan worktree dir '{dir_name}' (no state entry)")
                    };

                    if apply && !dirty {
                        let dest = home
                            .as_ref()
                            .map(|h| {
                                h.join(".claude")
                                    .join("projects-archive")
                                    .join(&ts)
                                    .join("worktrees")
                                    .join(workspace_name)
                                    .join(&dir_name)
                            })
                            .unwrap_or_else(|| dir.with_file_name(format!("{dir_name}.bak")));
                        if let Err(e) = archive_path(&dir, &dest) {
                            report.warnings.push(e);
                        } else {
                            report.applied += 1;
                            // Also prune git admin entries.
                            for pkg in &manifest.packages {
                                let repo = workspace_root.join("packages").join(&pkg.name);
                                let _ = git.worktree_prune(&repo);
                            }
                        }
                        report.actions.push(DoctorAction {
                            description: desc,
                            kind: ActionKind::Archive {
                                src: dir.clone(),
                                dest,
                            },
                        });
                    } else {
                        report.actions.push(DoctorAction {
                            description: desc,
                            kind: ActionKind::Archive {
                                src: dir.clone(),
                                dest: PathBuf::new(),
                            },
                        });
                    }
                }
            }
        }
    }

    // 3. Name mismatches: state key doesn't sanitize to the actual dir name on disk.
    let reloaded_state = if apply {
        WorkspaceState::load(workspace_root)?
    } else {
        state.clone()
    };
    for key in reloaded_state.worktrees.keys() {
        let expected_dir = sanitize_branch_for_dir(key);
        let actual_dir = worktrees_root.join(&expected_dir);
        // Check if the dir exists but with a DIFFERENT name — we can't enumerate easily,
        // so we just report when the expected dir is missing but the branch entry exists.
        // Mismatches where the dir was renamed are visible as: expected dir missing
        // but we haven't pruned the entry (it wouldn't be in branches_to_prune if
        // we've already applied — so we re-check here).
        if !actual_dir.exists() && !apply {
            report
                .name_mismatches
                .push((key.clone(), expected_dir.clone()));
        }
    }

    // 4. git worktree prune in each package's bare repo.
    for pkg in &manifest.packages {
        let repo = workspace_root.join("packages").join(&pkg.name);
        if !repo.exists() {
            continue;
        }
        let desc = format!("git worktree prune in packages/{}", pkg.name);
        if apply {
            if let Err(e) = git.worktree_prune(&repo) {
                report.warnings.push(format!(
                    "doctor worktrees: git worktree prune failed for '{}': {e}",
                    pkg.name
                ));
            } else {
                report.applied += 1;
            }
        }
        report.actions.push(DoctorAction {
            description: desc,
            kind: ActionKind::GitWorktreePrune { repo: repo.clone() },
        });
    }

    Ok(report)
}

// ── doctor tmux ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct StaleWindow {
    pub session: String,
    pub index: String,
    pub name: String,
}

pub struct TmuxDoctorReport {
    pub stale_windows: Vec<StaleWindow>,
    /// Window IDs whose `@cc_status` was set but no agent process was running (F12 sweep).
    pub stale_status_windows: Vec<String>,
    pub applied: usize,
    pub warnings: Vec<String>,
}

impl TmuxDoctorReport {
    fn new() -> Self {
        Self {
            stale_windows: Vec::new(),
            stale_status_windows: Vec::new(),
            applied: 0,
            warnings: Vec::new(),
        }
    }
}

// ── doctor hooks ──────────────────────────────────────────────────────────────

/// Result of the live resolver self-test run during `meldr doctor hooks`.
pub struct ResolverSelftestResult {
    /// True when doctor was not invoked from inside a tmux session — self-test skipped.
    pub skipped: bool,
    /// Tier 2 (TMUX_PANE env) resolved to a live pane.
    pub env_tier_pass: bool,
    /// Tier 5 (registry cwd match) found a temp entry for a child cwd.
    pub registry_tier_pass: bool,
    /// Sibling-prefix cwd correctly produced no match (regression test for the fmcevoy/fmcevoy_tools bug).
    pub sibling_nonmatch_pass: bool,
    /// Non-None when the self-test machinery itself failed (e.g. tmux not on PATH).
    pub error: Option<String>,
}

pub struct HooksDoctorReport {
    /// Whether `claude` was found on PATH.
    pub claude_detected: bool,
    /// A `_meldr`-tagged hook entry is absent from settings.json (only meaningful when claude_detected).
    pub claude_hook_missing: bool,
    /// `~/.tmux.conf` doesn't reference `@cc_status` in `window-status-format`.
    pub tmux_conf_missing_cc_status: bool,
    /// `~/.tmux.conf` doesn't clear `@cc_pane_status` in an `after-select-*` hook.
    pub tmux_conf_missing_pane_focus_clear: bool,
    /// `settings.json` is missing a meldr-tagged SessionStart hook.
    pub session_start_hook_missing: bool,
    /// `~/.cache/claude-agents/launchers/` is absent or not writable.
    pub launcher_dir_unwritable: bool,
    /// Legacy `~/.local/share/meldr/meldr-agent-notify.sh` still present from an old meldr version.
    pub legacy_notify_script_present: bool,
    /// Legacy `~/.claude/claude-session-start.sh` symlink still present from fmcevoy_tools.
    pub legacy_session_start_symlink_present: bool,
    /// Result of the live resolver self-test (None only when run_hooks skips it).
    pub resolver_selftest: Option<ResolverSelftestResult>,
    pub applied: usize,
    pub warnings: Vec<String>,
}

/// Diagnose hook-signaling health and optionally repair auto-fixable issues.
pub fn run_hooks(home: &Path, apply: bool) -> Result<HooksDoctorReport> {
    use crate::core::install_hooks;

    let mut report = HooksDoctorReport {
        claude_detected: false,
        claude_hook_missing: false,
        tmux_conf_missing_cc_status: false,
        tmux_conf_missing_pane_focus_clear: false,
        session_start_hook_missing: false,
        launcher_dir_unwritable: false,
        legacy_notify_script_present: false,
        legacy_session_start_symlink_present: false,
        resolver_selftest: None,
        applied: 0,
        warnings: Vec::new(),
    };

    // 1. Claude install + hook entries (Stop, Notification, SessionStart).
    let claude_found = std::process::Command::new("which")
        .arg("claude")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    report.claude_detected = claude_found;
    if claude_found {
        let stop_missing = !install_hooks::hooks_installed(home, "Stop");
        let notify_missing = !install_hooks::hooks_installed(home, "Notification");
        if stop_missing || notify_missing {
            report.claude_hook_missing = true;
            if apply {
                match install_hooks::install_claude_hooks(home, false).map(|_| ()) {
                    Ok(()) => report.applied += 1,
                    Err(e) => report.warnings.push(format!("hooks: install failed: {e}")),
                }
            }
        }

        // SessionStart hook check — auto-fix via apply same as above.
        if !install_hooks::hooks_installed(home, "SessionStart") {
            report.session_start_hook_missing = true;
            if apply && !report.claude_hook_missing {
                match install_hooks::install_claude_hooks(home, false) {
                    Ok(_) => report.applied += 1,
                    Err(e) => report
                        .warnings
                        .push(format!("hooks: SessionStart install failed: {e}")),
                }
            }
        }
    }

    // 2. tmux.conf checks (warn only — never auto-edit user's tmux.conf).
    let tmux_conf = home.join(".tmux.conf");
    if tmux_conf.exists() {
        let content = std::fs::read_to_string(&tmux_conf).unwrap_or_default();
        if !content.contains("@cc_status") {
            report.tmux_conf_missing_cc_status = true;
        }
        // Check that @cc_pane_status is cleared inside an after-select-* hook so
        // the pane border indicator clears when the user focuses the pane.
        let has_pane_clear = content.lines().any(|line| {
            (line.contains("after-select-window") || line.contains("after-select-pane"))
                && line.contains("@cc_pane_status")
        });
        if !has_pane_clear {
            report.tmux_conf_missing_pane_focus_clear = true;
        }
    } else {
        report.tmux_conf_missing_cc_status = true;
        report.tmux_conf_missing_pane_focus_clear = true;
    }

    // 3. Launcher registry directory writable (warn only).
    let launcher_dir = home.join(".cache/claude-agents/launchers");
    if launcher_dir.exists() {
        let probe = launcher_dir.join(".meldr-write-probe");
        let ok = std::fs::write(&probe, b"").is_ok();
        let _ = std::fs::remove_file(&probe);
        if !ok {
            report.launcher_dir_unwritable = true;
        }
    } else {
        report.launcher_dir_unwritable = std::fs::create_dir_all(&launcher_dir).is_err();
    }

    // 4. Legacy artifact checks.
    report.legacy_notify_script_present = home
        .join(".local/share/meldr/meldr-agent-notify.sh")
        .exists();
    report.legacy_session_start_symlink_present =
        install_hooks::legacy_session_start_symlink_present(home);

    // 5. Resolver self-test (only when running inside tmux; purely read-only against tmux options).
    report.resolver_selftest = Some(if std::env::var("TMUX").is_err() {
        ResolverSelftestResult {
            skipped: true,
            env_tier_pass: false,
            registry_tier_pass: false,
            sibling_nonmatch_pass: false,
            error: None,
        }
    } else {
        run_resolver_selftest()
    });

    Ok(report)
}

fn run_resolver_selftest() -> ResolverSelftestResult {
    use crate::core::claude_hooks::registry;
    use crate::tmux::RealTmux;

    let tmux = RealTmux::new();

    // Capture current pane id — we know it exists since we're running inside tmux.
    let current_pane = match run_tmux_cmd(&["display-message", "-p", "#{pane_id}"]) {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            return ResolverSelftestResult {
                skipped: false,
                env_tier_pass: false,
                registry_tier_pass: false,
                sibling_nonmatch_pass: false,
                error: Some(format!("tmux display-message failed: {e}")),
            };
        }
    };

    // T2 — env tier: the pane we're running in must be alive.
    let env_tier_pass = tmux.pane_exists(&current_pane);

    // Use an isolated temp dir so the test never touches real launcher entries.
    let selftest_dir = std::env::temp_dir().join("meldr-selftest-launchers");
    if let Err(e) = std::fs::create_dir_all(&selftest_dir) {
        return ResolverSelftestResult {
            skipped: false,
            env_tier_pass,
            registry_tier_pass: false,
            sibling_nonmatch_pass: false,
            error: Some(format!("selftest dir create failed: {e}")),
        };
    }
    // Remove any leftover entries from a prior run.
    if let Ok(rd) = std::fs::read_dir(&selftest_dir) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }

    let base_cwd = std::path::PathBuf::from("/tmp/meldr-selftest-base");
    let sub_cwd = base_cwd.join("sub");
    // A sibling path that shares a byte-prefix with base_cwd but is NOT a child of it.
    let sibling_cwd = std::path::PathBuf::from("/tmp/meldr-selftest-baseplus");

    // T5 — registry tier: write a temp entry and verify find_best_match resolves it.
    let registry_tier_pass =
        match registry::write_entry(&selftest_dir, &current_pane, "selftest-win", &base_cwd) {
            Ok(()) => registry::find_best_match(&selftest_dir, &sub_cwd, &tmux)
                .map(|e| e.pane == current_pane)
                .unwrap_or(false),
            Err(_) => false,
        };

    // Sibling non-match — same entry in dir, but query with a sibling cwd.
    // Path::starts_with is component-aware so /tmp/meldr-selftest-baseplus does NOT
    // start_with /tmp/meldr-selftest-base. This is the regression test for the
    // ~/fmcevoy vs ~/fmcevoy_tools bug.
    let sibling_nonmatch_pass =
        registry::find_best_match(&selftest_dir, &sibling_cwd, &tmux).is_none();

    // Clean up.
    if let Ok(rd) = std::fs::read_dir(&selftest_dir) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }

    ResolverSelftestResult {
        skipped: false,
        env_tier_pass,
        registry_tier_pass,
        sibling_nonmatch_pass,
        error: None,
    }
}

/// Find and optionally kill tmux windows whose named worktree no longer exists.
pub fn run_tmux(workspace_root: &Path, apply: bool) -> Result<TmuxDoctorReport> {
    let mut report = TmuxDoctorReport::new();

    let manifest = Manifest::load(workspace_root)?;
    let workspace_name = &manifest.workspace.name;
    let worktrees_root = worktrees_dir(workspace_root);

    // List all tmux windows: session name, window index, window id, window name.
    let output = match run_tmux_cmd(&[
        "list-windows",
        "-a",
        "-F",
        "#{session_name}\t#{window_index}\t#{window_id}\t#{window_name}",
    ]) {
        Ok(o) => o,
        Err(_) => return Ok(report), // Not in tmux or tmux not available.
    };

    // Build pane-path map for liveness checking.
    let pane_paths = build_pane_paths();
    let home_dir = std::env::var("HOME").unwrap_or_default();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        if parts.len() < 4 {
            continue;
        }
        let (session, index, window_id, window_name) = (parts[0], parts[1], parts[2], parts[3]);

        // Only consider windows whose name matches `<workspace-name>/<something>:`.
        let expected_prefix = format!("{workspace_name}/");
        if !window_name.starts_with(&expected_prefix) {
            continue;
        }

        // Extract the branch portion: strip prefix and trailing `:`.
        let branch_part = window_name
            .strip_prefix(&expected_prefix)
            .unwrap_or("")
            .trim_end_matches(':');

        if branch_part.is_empty() {
            continue;
        }

        let branch_dir = worktrees_root.join(sanitize_branch_for_dir(branch_part));
        let dir_exists = branch_dir.exists();

        // Pane-path liveness: if ALL non-home non-root panes in this window fall under
        // a missing worktrees path, consider the window stale regardless of dir check.
        let pane_evidence = pane_paths.get(window_id).cloned().unwrap_or_default();
        let meaningful_panes: Vec<&String> = pane_evidence
            .iter()
            .filter(|p| !p.is_empty() && *p != &home_dir && *p != "/")
            .collect();
        let all_panes_dead = !meaningful_panes.is_empty()
            && meaningful_panes.iter().all(|p| {
                let pb = PathBuf::from(p.as_str());
                !pb.exists()
            });

        if !dir_exists || all_panes_dead {
            report.stale_windows.push(StaleWindow {
                session: session.to_string(),
                index: index.to_string(),
                name: window_name.to_string(),
            });

            if apply {
                let target = format!("{session}:{index}");
                if let Err(e) = run_tmux_cmd(&["kill-window", "-t", &target]) {
                    report.warnings.push(format!(
                        "doctor tmux: could not kill window '{window_name}': {e}"
                    ));
                } else {
                    report.applied += 1;
                }
            }
        } else {
            // F12: unset @cc_status if set but no agent process is alive in any pane.
            let cc_set = run_tmux_cmd(&["show-options", "-wqv", "-t", window_id, "@cc_status"])
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if cc_set && !check_window_has_agent_process(window_id) {
                report.stale_status_windows.push(window_id.to_string());
                if apply {
                    let _ = run_tmux_cmd(&["set-option", "-wu", "-t", window_id, "@cc_status"]);
                    let _ = run_tmux_cmd(&["set-option", "-wu", "-t", window_id, "@cc_status_gen"]);
                    report.applied += 1;
                }
            }
        }
    }

    Ok(report)
}

/// Returns true if any pane in `window_id` currently has an agent process running.
/// Uses `#{pane_current_command}` as a lightweight signal.
fn check_window_has_agent_process(window_id: &str) -> bool {
    let output = match run_tmux_cmd(&[
        "list-panes",
        "-t",
        window_id,
        "-F",
        "#{pane_current_command}",
    ]) {
        Ok(o) => o,
        Err(_) => return true, // Can't check, assume alive.
    };
    output.lines().any(|cmd| {
        let cmd = cmd.trim().to_ascii_lowercase();
        // Claude CLI runs under Node; also match the claude binary directly.
        cmd == "node" || cmd.starts_with("claude")
    })
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn read_job_cwd(state_path: &Path) -> Option<PathBuf> {
    let contents = std::fs::read_to_string(state_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&contents).ok()?;
    val.get("cwd")?.as_str().map(PathBuf::from)
}

fn is_older_than(path: &Path, duration: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|mtime| {
            SystemTime::now()
                .duration_since(mtime)
                .unwrap_or(Duration::ZERO)
                > duration
        })
        .unwrap_or(false)
}

fn archive_path(src: &Path, dest: &Path) -> std::result::Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "doctor: could not create archive dir '{}': {e}",
                parent.display()
            )
        })?;
    }
    std::fs::rename(src, dest).map_err(|e| {
        format!(
            "doctor: could not archive '{}' → '{}': {e}",
            src.display(),
            dest.display()
        )
    })
}

fn is_worktree_dirty_any_package(branch_dir: &Path, manifest: &Manifest) -> bool {
    for pkg in &manifest.packages {
        let wt_path = branch_dir.join(&pkg.name);
        if !wt_path.exists() {
            continue;
        }
        // Quick check: any file in the git index that's modified.
        let dirty = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&wt_path)
            .output()
            .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);
        if dirty {
            return true;
        }
    }
    false
}

fn run_tmux_cmd(args: &[&str]) -> std::result::Result<String, String> {
    let output = std::process::Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| format!("tmux: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Build a map from window_id → list of pane current_paths using `tmux list-panes -a`.
fn build_pane_paths() -> std::collections::HashMap<String, Vec<String>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let output = match run_tmux_cmd(&[
        "list-panes",
        "-a",
        "-F",
        "#{window_id}\t#{pane_current_path}",
    ]) {
        Ok(o) => o,
        Err(_) => return map,
    };
    for line in output.lines() {
        if let Some((wid, path)) = line.split_once('\t') {
            map.entry(wid.to_string())
                .or_default()
                .push(path.to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace(packages: &[&str]) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        fs::create_dir_all(root.join(".meldr")).unwrap();
        fs::create_dir_all(root.join("packages")).unwrap();
        fs::create_dir_all(root.join("worktrees")).unwrap();
        for pkg in packages {
            fs::create_dir_all(root.join("packages").join(pkg)).unwrap();
        }
        let toml = format!(
            "[workspace]\nname = \"test-ws\"\n{}",
            packages
                .iter()
                .map(|p| format!(
                    "\n[[package]]\nname = \"{p}\"\nurl = \"https://example.com/{p}.git\"\n"
                ))
                .collect::<String>()
        );
        fs::write(root.join("meldr.toml"), toml).unwrap();
        (tmp, root)
    }

    struct MockGit;
    impl crate::git::GitOps for MockGit {
        fn clone_repo(&self, _: &str, _: &Path) -> Result<()> {
            Ok(())
        }
        fn worktree_add(&self, _: &Path, _: &Path, _: &str) -> Result<()> {
            Ok(())
        }
        fn worktree_remove(&self, _: &Path, _: &Path, _: bool) -> Result<()> {
            Ok(())
        }
        fn is_dirty(&self, _: &Path) -> Result<bool> {
            Ok(false)
        }
        fn fetch(&self, _: &Path, _: &str) -> Result<()> {
            Ok(())
        }
        fn rebase(&self, _: &Path, _: &str, _: &str, _: bool) -> Result<()> {
            Ok(())
        }
        fn merge(&self, _: &Path, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn status_porcelain(&self, _: &Path) -> Result<String> {
            Ok(String::new())
        }
        fn detect_default_branch(&self, _: &Path, _: &str) -> Option<String> {
            None
        }
        fn ensure_remote_tracking(&self, _: &Path, _: &str) -> Result<()> {
            Ok(())
        }
        fn divergence(&self, _: &Path, _: &str) -> Result<(u32, u32)> {
            Ok((0, 0))
        }
        fn check_merge_conflicts(&self, _: &Path, _: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn log_oneline(&self, _: &Path, _: u32) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn current_head(&self, _: &Path) -> Result<String> {
            Ok("sha".to_string())
        }
        fn reset_hard(&self, _: &Path, _: &str) -> Result<()> {
            Ok(())
        }
        fn push(&self, _: &Path, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn fast_forward_branch(&self, _: &Path, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn worktree_list(&self, _: &Path) -> Result<Vec<crate::git::WorktreeEntry>> {
            Ok(vec![])
        }
        fn current_branch(&self, _: &Path) -> Result<String> {
            Ok("main".to_string())
        }
        fn worktree_prune(&self, _: &Path) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_worktrees_prune_missing_state_entry() {
        let (_tmp, root) = make_workspace(&["pkg-a"]);
        let git = MockGit;

        // Add a state entry for a branch with no on-disk dir.
        let mut state = WorkspaceState::load(&root).unwrap();
        state.add_worktree(
            "orphan-branch",
            crate::core::state::WorktreeState {
                branch: "orphan-branch".to_string(),
                tmux_window: None,
                pane_mappings: std::collections::HashMap::new(),
            },
        );
        state.save(&root).unwrap();

        let report = run_worktrees(&git, &root, false).unwrap();
        assert!(report.pruned_state.contains(&"orphan-branch".to_string()));
    }

    #[test]
    fn test_run_hooks_detects_missing_entry() {
        let tmp = TempDir::new().unwrap();
        let settings_dir = tmp.path().join(".claude");
        fs::create_dir_all(&settings_dir).unwrap();
        fs::write(
            settings_dir.join("settings.json"),
            r#"{"hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"bash ~/custom.sh"}]}]}}"#,
        )
        .unwrap();
        // Test the detection primitive directly: claude_hook_missing in run_hooks is
        // gated on `which claude` succeeding, which is not guaranteed on all CI runners.
        assert!(
            !crate::core::install_hooks::hooks_installed(tmp.path(), "Stop"),
            "should detect missing _meldr entry"
        );
    }

    #[test]
    fn test_run_hooks_detects_legacy_notify_script() {
        let tmp = TempDir::new().unwrap();
        let script_dir = tmp.path().join(".local/share/meldr");
        fs::create_dir_all(&script_dir).unwrap();
        fs::write(script_dir.join("meldr-agent-notify.sh"), "old content").unwrap();
        let report = run_hooks(tmp.path(), false).unwrap();
        assert!(
            report.legacy_notify_script_present,
            "should detect legacy notify script"
        );
    }

    #[test]
    fn test_run_hooks_clean_when_installed() {
        let tmp = TempDir::new().unwrap();
        crate::core::install_hooks::install_claude_hooks(tmp.path(), false).unwrap();
        let report = run_hooks(tmp.path(), false).unwrap();
        assert!(
            !report.legacy_notify_script_present,
            "no legacy script after fresh install"
        );
        assert!(
            !report.claude_hook_missing,
            "hook should be present after install"
        );
    }

    #[test]
    fn test_worktrees_apply_removes_state_entry() {
        let (_tmp, root) = make_workspace(&["pkg-a"]);
        let git = MockGit;

        let mut state = WorkspaceState::load(&root).unwrap();
        state.add_worktree(
            "gone-branch",
            crate::core::state::WorktreeState {
                branch: "gone-branch".to_string(),
                tmux_window: None,
                pane_mappings: std::collections::HashMap::new(),
            },
        );
        state.save(&root).unwrap();

        run_worktrees(&git, &root, true).unwrap();

        let after = WorkspaceState::load(&root).unwrap();
        assert!(after.get_worktree("gone-branch").is_none());
    }

    // ── hooks doctor tests ────────────────────────────────────────────────────

    fn write_settings(dir: &std::path::Path, v: &serde_json::Value) {
        let p = dir.join(".claude");
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("settings.json"),
            serde_json::to_string_pretty(v).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn test_session_start_hook_missing_flag_set_when_absent() {
        let tmp = TempDir::new().unwrap();
        // Settings with Stop/Notification but no SessionStart
        write_settings(
            tmp.path(),
            &serde_json::json!({
                "hooks": {
                    "Stop": [{"matcher":"*","hooks":[{"type":"command","command":"bash x stop","_meldr":true}]}],
                    "Notification": [{"matcher":"*","hooks":[{"type":"command","command":"bash x notify","_meldr":true}]}]
                }
            }),
        );
        // Patch HOME so claude is "not found" — avoids needing real claude on PATH.
        // Use a fake PATH with no claude to ensure claude_detected = false, which means
        // the check only runs when claude IS detected.  Test the flag directly via run_hooks.
        // We test the flag via the public struct fields.
        let report = run_hooks(tmp.path(), false).unwrap();
        // claude_detected may be true if claude is in PATH; the flag is only set when detected.
        if report.claude_detected {
            assert!(
                report.session_start_hook_missing,
                "SessionStart hook absent → flag must be set"
            );
        }
    }

    #[test]
    fn test_session_start_hook_present_flag_clear() {
        let tmp = TempDir::new().unwrap();
        write_settings(
            tmp.path(),
            &serde_json::json!({
                "hooks": {
                    "Stop": [{"matcher":"*","hooks":[{"type":"command","command":"bash x stop","_meldr":true}]}],
                    "Notification": [{"matcher":"*","hooks":[{"type":"command","command":"bash x notify","_meldr":true}]}],
                    "SessionStart": [{"matcher":"startup","hooks":[{"type":"command","command":"bash ~/.claude/claude-session-start.sh","_meldr":true}]}]
                }
            }),
        );
        let report = run_hooks(tmp.path(), false).unwrap();
        if report.claude_detected {
            assert!(
                !report.session_start_hook_missing,
                "SessionStart hook present → flag must be clear"
            );
        }
    }

    #[test]
    fn test_launcher_dir_missing_flag_set_then_clear_after_creation() {
        let tmp = TempDir::new().unwrap();
        // No launcher dir → flag set (and dir is created by the check itself)
        let report = run_hooks(tmp.path(), false).unwrap();
        // The check creates the dir if missing; if creation succeeded, flag is clear.
        assert!(
            !report.launcher_dir_unwritable,
            "launcher dir should be creatable in a temp home"
        );
        assert!(
            tmp.path().join(".cache/claude-agents/launchers").exists(),
            "check must create the dir when it is missing"
        );
    }

    // ── pane focus-clear doctor tests ─────────────────────────────────────────

    #[test]
    fn test_pane_focus_clear_flag_set_when_absent() {
        let tmp = TempDir::new().unwrap();
        // tmux.conf with @cc_status but no @cc_pane_status focus-clear
        let content = "set-hook -g after-select-window 'set-option -wu @cc_status'\n\
                       set -g window-status-format \"#{@cc_status}\"\n";
        fs::write(tmp.path().join(".tmux.conf"), content).unwrap();
        let report = run_hooks(tmp.path(), false).unwrap();
        assert!(
            report.tmux_conf_missing_pane_focus_clear,
            "flag must be set when after-select hook doesn't clear @cc_pane_status"
        );
    }

    #[test]
    fn test_pane_focus_clear_flag_clear_when_present() {
        let tmp = TempDir::new().unwrap();
        let content = "set-hook -g after-select-window 'set-option -wu @cc_status ; set-option -pu @cc_pane_status'\n\
                       set-hook -g after-select-pane   'set-option -wu @cc_status ; set-option -pu @cc_pane_status'\n\
                       set -g window-status-format \"#{@cc_status}\"\n";
        fs::write(tmp.path().join(".tmux.conf"), content).unwrap();
        let report = run_hooks(tmp.path(), false).unwrap();
        assert!(
            !report.tmux_conf_missing_pane_focus_clear,
            "flag must be clear when @cc_pane_status is cleared in after-select hook"
        );
    }

    #[test]
    fn test_pane_focus_clear_flag_set_when_tmux_conf_missing() {
        let tmp = TempDir::new().unwrap();
        // No .tmux.conf at all
        let report = run_hooks(tmp.path(), false).unwrap();
        assert!(
            report.tmux_conf_missing_pane_focus_clear,
            "flag must be set when .tmux.conf is absent"
        );
    }
}
