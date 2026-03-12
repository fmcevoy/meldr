use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;

use crate::core::config::{EffectiveConfig, GlobalConfig};
use crate::core::state::{WorkspaceState, WorktreeState};
use crate::core::workspace::{self, Manifest};
use crate::error::{MeldrError, Result};
use crate::git::GitOps;
use crate::tmux::TmuxOps;

/// Expand template variables in a string.
///
/// Replaces `{ws}`, `{branch}`, and `{pkg}` with their respective values.
pub fn expand_template(template: &str, ws: &str, branch: &str, pkg: &str) -> String {
    template
        .replace("{ws}", ws)
        .replace("{branch}", branch)
        .replace("{pkg}", pkg)
}

/// Result of setting up tmux windows for a worktree.
struct TmuxSetupResult {
    tmux_window: Option<String>,
    pane_mappings: HashMap<String, String>,
}

/// Create tmux windows and panes for a set of packages in a worktree branch.
///
/// Handles both manifest layout overrides and per-package dev window layouts.
/// Skips packages whose worktree path does not exist on disk.
fn setup_tmux_windows(
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
) -> Result<TmuxSetupResult> {
    let ws_name = &manifest.workspace.name;
    let mut tmux_windows = Vec::new();
    let mut pane_mappings = HashMap::new();

    if let Some(ref lo) = manifest.layout {
        let window_name = expand_template(&config.window_name_template, ws_name, branch, "");
        let window_id = tmux.create_window(&window_name)?;

        for _ in 1..lo.panes.len() {
            tmux.split_window(&window_id)?;
        }

        let layout = crate::tmux::TmuxLayout {
            definition: lo.definition.clone(),
            pane_names: lo.panes.clone(),
        };
        tmux.apply_layout(&window_id, &layout)?;

        for (i, pkg_name) in lo.panes.iter().enumerate() {
            if pkg_name.is_empty() {
                continue;
            }
            let wt_path = workspace::worktree_path(workspace_root, branch, pkg_name);
            let target = format!("{window_id}.{i}");
            tmux.send_keys(&target, &format!("cd {}", wt_path.display()))?;
            if config.should_launch_agent() {
                tmux.send_keys(&target, &config.agent_command)?;
            }
            pane_mappings.insert(i.to_string(), pkg_name.clone());
        }

        tmux_windows.push(window_id);
    } else {
        let custom_layout = global_config.and_then(|gc| gc.layouts.get(&config.layout));
        let wt_dir = workspace::worktree_branch_dir(workspace_root, branch);
        let wt_dir_str = wt_dir.to_string_lossy().to_string();

        let window_name = expand_template(&config.window_name_template, ws_name, branch, "");

        let dev = tmux.create_dev_window(&window_name, &wt_dir_str, config, custom_layout)?;

        if let Some(ref editor_pane) = dev.editor {
            tmux.send_keys(editor_pane, &config.editor)?;
            pane_mappings.insert("editor".to_string(), editor_pane.clone());
        }

        if config.should_launch_agent()
            && let Some(ref agent_pane) = dev.agent
        {
            tmux.send_keys(agent_pane, &config.agent_command)?;
        }
        if let Some(ref agent_pane) = dev.agent {
            pane_mappings.insert("agent".to_string(), agent_pane.clone());
        }

        tmux_windows.push(dev.window_id);
    }

    let tmux_window = match tmux_windows.len() {
        0 => None,
        1 => Some(tmux_windows.into_iter().next().unwrap()),
        _ => Some(tmux_windows.join(",")),
    };

    Ok(TmuxSetupResult {
        tmux_window,
        pane_mappings,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn add_worktree(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
) -> Result<()> {
    if state.get_worktree(branch).is_some() {
        return Err(MeldrError::WorktreeAlreadyExists(branch.to_string()));
    }

    let needs_tmux = config.should_use_tmux();

    if needs_tmux && !tmux.is_inside_tmux() {
        return Err(MeldrError::NotInTmux);
    }

    let branch_dir = workspace::worktree_branch_dir(workspace_root, branch);
    std::fs::create_dir_all(&branch_dir)?;

    let results: Vec<_> = manifest
        .packages
        .par_iter()
        .map(|pkg| {
            let repo_path = workspace::package_path(workspace_root, &pkg.name);
            let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
            match git.worktree_add(&repo_path, &wt_path, branch) {
                Ok(()) => Ok(pkg.name.clone()),
                Err(e) => Err((pkg.name.clone(), e)),
            }
        })
        .collect();

    let mut created = Vec::new();
    let mut errors = Vec::new();
    for result in results {
        match result {
            Ok(name) => created.push(name),
            Err((name, e)) => errors.push(format!("{name}: {e}")),
        }
    }

    if !errors.is_empty() && created.is_empty() {
        return Err(MeldrError::Git(format!(
            "Failed to create any worktrees: {}",
            errors.join(", ")
        )));
    }

    for error in &errors {
        eprintln!("Warning: {error}");
    }

    let setup = if needs_tmux {
        setup_tmux_windows(
            tmux,
            manifest,
            workspace_root,
            branch,
            config,
            global_config,
        )?
    } else {
        TmuxSetupResult {
            tmux_window: None,
            pane_mappings: HashMap::new(),
        }
    };

    state.add_worktree(
        branch,
        WorktreeState {
            branch: branch.to_string(),
            tmux_window: setup.tmux_window,
            pane_mappings: setup.pane_mappings,
        },
    );
    state.save(workspace_root)?;

    Ok(())
}

pub fn remove_worktree(
    git: &dyn GitOps,
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    force: bool,
) -> Result<()> {
    if state.get_worktree(branch).is_none() {
        return Err(MeldrError::WorktreeNotFound(branch.to_string()));
    }

    if !force {
        for pkg in &manifest.packages {
            let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
            if wt_path.exists()
                && let Ok(true) = git.is_dirty(&wt_path)
            {
                return Err(MeldrError::DirtyWorktree(
                    branch.to_string(),
                    pkg.name.clone(),
                ));
            }
        }
    }

    // Capture tmux window ID before modifying state
    let tmux_window_id = state
        .get_worktree(branch)
        .and_then(|wt| wt.tmux_window.clone());

    // Remove git worktrees for ALL packages BEFORE killing the tmux window.
    // If we kill the tmux window first and the user is running this command
    // from within that window, the process gets terminated before cleanup.
    for pkg in &manifest.packages {
        let repo_path = workspace::package_path(workspace_root, &pkg.name);
        let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
        if wt_path.exists()
            && let Err(e) = git.worktree_remove(&repo_path, &wt_path, force)
        {
            eprintln!(
                "Warning: Failed to remove worktree for '{}': {}",
                pkg.name, e
            );
        }
    }

    let branch_dir = workspace::worktree_branch_dir(workspace_root, branch);
    if branch_dir.exists() {
        let _ = std::fs::remove_dir_all(&branch_dir);
    }

    state.remove_worktree(branch);
    state.save(workspace_root)?;

    // Kill tmux window LAST — after all worktrees are removed and state is saved.
    // This way even if killing the window terminates this process, cleanup is complete.
    if let Some(ref window_id) = tmux_window_id
        && let Err(e) = tmux.kill_window(window_id)
    {
        eprintln!("Warning: Could not kill tmux window '{window_id}': {e}");
    }

    Ok(())
}

pub fn open_worktree(
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    global_config: Option<&GlobalConfig>,
) -> Result<()> {
    let wt_state = state
        .get_worktree(branch)
        .ok_or_else(|| MeldrError::WorktreeNotFound(branch.to_string()))?;

    // If tmux windows are still alive, just select the first one
    if let Some(ref window_ids) = wt_state.tmux_window {
        let first = window_ids.split(',').next().unwrap_or("");
        if !first.is_empty() && tmux.has_window(first) {
            tmux.select_window(first)?;
            return Ok(());
        }
    }

    // Windows are gone — recreate them
    if !config.should_use_tmux() {
        return Err(MeldrError::Tmux(
            "Cannot open worktree windows: tmux is disabled via --no-tabs or config".to_string(),
        ));
    }

    if !tmux.is_inside_tmux() {
        return Err(MeldrError::NotInTmux);
    }

    let setup = setup_tmux_windows(
        tmux,
        manifest,
        workspace_root,
        branch,
        config,
        global_config,
    )?;

    state.add_worktree(
        branch,
        WorktreeState {
            branch: branch.to_string(),
            tmux_window: setup.tmux_window,
            pane_mappings: setup.pane_mappings,
        },
    );
    state.save(workspace_root)?;

    Ok(())
}

pub fn list_worktrees(state: &WorkspaceState) -> Vec<&WorktreeState> {
    state.worktrees.values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::EffectiveConfig;
    use crate::core::state::WorkspaceState;
    use crate::core::workspace::{Manifest, PackageEntry, WorkspaceInfo};
    use crate::error::Result;
    use crate::tmux::{DevWindowPanes, TmuxLayout, TmuxOps};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Tracks all tmux calls for assertions
    #[derive(Debug, Default)]
    struct TmuxCall {
        create_dev_window: Vec<(String, String)>, // (name, cwd)
        create_window: Vec<String>,               // name
        split_window: Vec<String>,                // window
        send_keys: Vec<(String, String)>,         // (target, keys)
    }

    struct MockTmux {
        calls: Mutex<TmuxCall>,
    }

    impl MockTmux {
        fn new() -> Self {
            Self {
                calls: Mutex::new(TmuxCall::default()),
            }
        }

        #[allow(dead_code)]
        fn calls(&self) -> std::sync::MutexGuard<'_, TmuxCall> {
            self.calls.lock().unwrap()
        }
    }

    impl TmuxOps for MockTmux {
        fn is_inside_tmux(&self) -> bool {
            true
        }

        fn create_window(&self, name: &str) -> Result<String> {
            self.calls
                .lock()
                .unwrap()
                .create_window
                .push(name.to_string());
            Ok("@99".to_string())
        }

        fn split_window(&self, window: &str) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .split_window
                .push(window.to_string());
            Ok(())
        }

        fn apply_layout(&self, _window: &str, _layout: &TmuxLayout) -> Result<()> {
            Ok(())
        }

        fn send_keys(&self, target: &str, keys: &str) -> Result<()> {
            self.calls
                .lock()
                .unwrap()
                .send_keys
                .push((target.to_string(), keys.to_string()));
            Ok(())
        }

        fn kill_window(&self, _window: &str) -> Result<()> {
            Ok(())
        }

        fn create_dev_window(
            &self,
            name: &str,
            cwd: &str,
            _config: &EffectiveConfig,
            _custom_layout: Option<&crate::core::config::LayoutDef>,
        ) -> Result<DevWindowPanes> {
            self.calls
                .lock()
                .unwrap()
                .create_dev_window
                .push((name.to_string(), cwd.to_string()));
            Ok(DevWindowPanes {
                window_id: "@100".to_string(),
                editor: Some("@100.0".to_string()),
                agent: Some("%1".to_string()),
                terms: vec![
                    "%2".to_string(),
                    "%3".to_string(),
                    "%4".to_string(),
                    "%5".to_string(),
                ],
            })
        }

        fn has_window(&self, _window: &str) -> bool {
            false
        }
        fn select_window(&self, _window: &str) -> Result<()> {
            Ok(())
        }
    }

    struct MockGit;

    impl GitOps for MockGit {
        fn clone_repo(&self, _url: &str, _path: &Path) -> Result<()> {
            Ok(())
        }
        fn worktree_add(&self, _repo: &Path, _dest: &Path, _branch: &str) -> Result<()> {
            Ok(())
        }
        fn worktree_remove(&self, _repo: &Path, _path: &Path, _force: bool) -> Result<()> {
            Ok(())
        }
        fn is_dirty(&self, _path: &Path) -> Result<bool> {
            Ok(false)
        }
        fn fetch(&self, _path: &Path, _remote: &str) -> Result<()> {
            Ok(())
        }
        fn rebase(
            &self,
            _path: &Path,
            _onto: &str,
            _strategy: &str,
            _autostash: bool,
        ) -> Result<()> {
            Ok(())
        }
        fn merge(&self, _path: &Path, _branch: &str, _strategy: &str) -> Result<()> {
            Ok(())
        }
        fn status_porcelain(&self, _path: &Path) -> Result<String> {
            Ok(String::new())
        }
        fn detect_default_branch(&self, _path: &Path, _remote: &str) -> Option<String> {
            None
        }
        fn ensure_remote_tracking(&self, _path: &Path, _remote: &str) -> Result<()> {
            Ok(())
        }
        fn divergence(&self, _path: &Path, _upstream: &str) -> Result<(u32, u32)> {
            Ok((0, 0))
        }
        fn check_merge_conflicts(&self, _path: &Path, _upstream: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn current_head(&self, _path: &Path) -> Result<String> {
            Ok("mock_sha".to_string())
        }
        fn reset_hard(&self, _path: &Path, _commit: &str) -> Result<()> {
            Ok(())
        }
        fn fast_forward_branch(&self, _repo: &Path, _branch: &str, _remote: &str) -> Result<()> {
            Ok(())
        }
    }

    fn test_manifest(packages: &[&str]) -> Manifest {
        Manifest {
            workspace: WorkspaceInfo {
                name: "test-ws".to_string(),
            },
            settings: Default::default(),
            layout: None,
            packages: packages
                .iter()
                .map(|name| PackageEntry {
                    name: name.to_string(),
                    url: format!("https://example.com/{name}.git"),
                    branch: None,
                    remote: None,
                    sync_strategy: None,
                })
                .collect(),
        }
    }

    fn setup_workspace(packages: &[&str]) -> (tempfile::TempDir, Manifest) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::fs::create_dir_all(root.join("worktrees")).unwrap();
        std::fs::create_dir_all(root.join(".meldr")).unwrap();

        for pkg in packages {
            std::fs::create_dir_all(root.join("packages").join(pkg)).unwrap();
        }

        let manifest = test_manifest(packages);
        (tmp, manifest)
    }

    // --- Removal tests with order tracking ---

    static ORDER_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn reset_order_counter() {
        ORDER_COUNTER.store(0, Ordering::SeqCst);
    }

    fn next_order() -> usize {
        ORDER_COUNTER.fetch_add(1, Ordering::SeqCst)
    }

    struct OrderTrackingGit {
        removed_packages: Mutex<Vec<(String, usize)>>,
    }

    impl OrderTrackingGit {
        fn new() -> Self {
            Self {
                removed_packages: Mutex::new(Vec::new()),
            }
        }

        fn removed(&self) -> Vec<(String, usize)> {
            self.removed_packages.lock().unwrap().clone()
        }
    }

    impl GitOps for OrderTrackingGit {
        fn clone_repo(&self, _url: &str, _path: &Path) -> Result<()> {
            Ok(())
        }
        fn worktree_add(&self, _repo: &Path, _dest: &Path, _branch: &str) -> Result<()> {
            Ok(())
        }
        fn worktree_remove(&self, _repo: &Path, path: &Path, _force: bool) -> Result<()> {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            self.removed_packages
                .lock()
                .unwrap()
                .push((name, next_order()));
            Ok(())
        }
        fn is_dirty(&self, _path: &Path) -> Result<bool> {
            Ok(false)
        }
        fn fetch(&self, _path: &Path, _remote: &str) -> Result<()> {
            Ok(())
        }
        fn rebase(
            &self,
            _path: &Path,
            _onto: &str,
            _strategy: &str,
            _autostash: bool,
        ) -> Result<()> {
            Ok(())
        }
        fn merge(&self, _path: &Path, _branch: &str, _strategy: &str) -> Result<()> {
            Ok(())
        }
        fn status_porcelain(&self, _path: &Path) -> Result<String> {
            Ok(String::new())
        }
        fn detect_default_branch(&self, _path: &Path, _remote: &str) -> Option<String> {
            None
        }
        fn ensure_remote_tracking(&self, _path: &Path, _remote: &str) -> Result<()> {
            Ok(())
        }
        fn divergence(&self, _path: &Path, _upstream: &str) -> Result<(u32, u32)> {
            Ok((0, 0))
        }
        fn check_merge_conflicts(&self, _path: &Path, _upstream: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        fn current_head(&self, _path: &Path) -> Result<String> {
            Ok("mock_sha".to_string())
        }
        fn reset_hard(&self, _path: &Path, _commit: &str) -> Result<()> {
            Ok(())
        }
        fn fast_forward_branch(&self, _repo: &Path, _branch: &str, _remote: &str) -> Result<()> {
            Ok(())
        }
    }

    struct OrderTrackingTmux {
        kill_order: Mutex<Option<usize>>,
    }

    impl OrderTrackingTmux {
        fn new() -> Self {
            Self {
                kill_order: Mutex::new(None),
            }
        }

        fn kill_order(&self) -> Option<usize> {
            *self.kill_order.lock().unwrap()
        }
    }

    impl TmuxOps for OrderTrackingTmux {
        fn is_inside_tmux(&self) -> bool {
            true
        }
        fn create_window(&self, _name: &str) -> Result<String> {
            Ok("@99".to_string())
        }
        fn split_window(&self, _window: &str) -> Result<()> {
            Ok(())
        }
        fn apply_layout(&self, _window: &str, _layout: &TmuxLayout) -> Result<()> {
            Ok(())
        }
        fn send_keys(&self, _target: &str, _keys: &str) -> Result<()> {
            Ok(())
        }
        fn kill_window(&self, _window: &str) -> Result<()> {
            *self.kill_order.lock().unwrap() = Some(next_order());
            Ok(())
        }
        fn create_dev_window(
            &self,
            _name: &str,
            _cwd: &str,
            _config: &EffectiveConfig,
            _custom_layout: Option<&crate::core::config::LayoutDef>,
        ) -> Result<DevWindowPanes> {
            Ok(DevWindowPanes {
                window_id: "@100".to_string(),
                editor: Some("@100.0".to_string()),
                agent: Some("%1".to_string()),
                terms: vec![
                    "%2".to_string(),
                    "%3".to_string(),
                    "%4".to_string(),
                    "%5".to_string(),
                ],
            })
        }
        fn has_window(&self, _window: &str) -> bool {
            false
        }
        fn select_window(&self, _window: &str) -> Result<()> {
            Ok(())
        }
    }

    fn worktree_state_with_window(branch: &str, window: &str) -> WorktreeState {
        WorktreeState {
            branch: branch.to_string(),
            tmux_window: Some(window.to_string()),
            pane_mappings: HashMap::new(),
        }
    }

    fn worktree_state_no_window(branch: &str) -> WorktreeState {
        WorktreeState {
            branch: branch.to_string(),
            tmux_window: None,
            pane_mappings: HashMap::new(),
        }
    }

    #[test]
    fn test_remove_worktree_removes_all_packages() {
        let packages = &["frontend", "backend", "shared"];
        let (tmp, manifest) = setup_workspace(packages);
        let git = OrderTrackingGit::new();
        let tmux = MockTmux::new();
        let mut state = WorkspaceState::default();

        state.add_worktree("feat-rm", worktree_state_with_window("feat-rm", "@50"));

        for pkg in packages {
            std::fs::create_dir_all(tmp.path().join("worktrees").join("feat-rm").join(pkg))
                .unwrap();
        }

        remove_worktree(
            &git,
            &tmux,
            &manifest,
            &mut state,
            tmp.path(),
            "feat-rm",
            false,
        )
        .unwrap();

        let removed = git.removed();
        assert_eq!(
            removed.len(),
            3,
            "should remove worktrees for ALL 3 packages, got: {removed:?}"
        );
        let removed_names: Vec<&str> = removed.iter().map(|(n, _)| n.as_str()).collect();
        for pkg in packages {
            assert!(
                removed_names.contains(pkg),
                "package '{pkg}' should have been removed, got: {removed_names:?}"
            );
        }

        assert!(
            state.get_worktree("feat-rm").is_none(),
            "worktree should be removed from state"
        );
        assert!(
            !tmp.path().join("worktrees").join("feat-rm").exists(),
            "branch directory should be cleaned up"
        );
    }

    #[test]
    fn test_remove_worktree_kills_tmux_after_git_cleanup() {
        reset_order_counter();

        let packages = &["frontend", "backend"];
        let (tmp, manifest) = setup_workspace(packages);
        let git = OrderTrackingGit::new();
        let tmux = OrderTrackingTmux::new();
        let mut state = WorkspaceState::default();

        state.add_worktree(
            "feat-order",
            worktree_state_with_window("feat-order", "@60"),
        );

        for pkg in packages {
            std::fs::create_dir_all(tmp.path().join("worktrees").join("feat-order").join(pkg))
                .unwrap();
        }

        remove_worktree(
            &git,
            &tmux,
            &manifest,
            &mut state,
            tmp.path(),
            "feat-order",
            false,
        )
        .unwrap();

        let removed = git.removed();
        assert_eq!(removed.len(), 2, "both packages should be removed");

        let max_git_order = removed.iter().map(|(_, ord)| *ord).max().unwrap();
        let kill_order = tmux
            .kill_order()
            .expect("tmux kill_window should have been called");

        assert!(
            kill_order > max_git_order,
            "tmux kill_window (order={kill_order}) must happen AFTER all git worktree removals (last git order={max_git_order})",
        );
    }

    #[test]
    fn test_remove_worktree_nonexistent_fails() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let git = MockGit;
        let tmux = MockTmux::new();
        let mut state = WorkspaceState::default();

        let result = remove_worktree(
            &git,
            &tmux,
            &manifest,
            &mut state,
            tmp.path(),
            "no-such",
            false,
        );
        assert!(result.is_err(), "removing nonexistent worktree should fail");
    }

    #[test]
    fn test_remove_worktree_no_tmux_window() {
        let packages = &["frontend"];
        let (tmp, manifest) = setup_workspace(packages);
        let git = OrderTrackingGit::new();
        let tmux = OrderTrackingTmux::new();
        let mut state = WorkspaceState::default();

        state.add_worktree("feat-notab", worktree_state_no_window("feat-notab"));

        std::fs::create_dir_all(
            tmp.path()
                .join("worktrees")
                .join("feat-notab")
                .join("frontend"),
        )
        .unwrap();

        remove_worktree(
            &git,
            &tmux,
            &manifest,
            &mut state,
            tmp.path(),
            "feat-notab",
            false,
        )
        .unwrap();

        let removed = git.removed();
        assert_eq!(removed.len(), 1, "should still remove git worktree");
        assert!(
            tmux.kill_order().is_none(),
            "should not call kill_window when no tmux window"
        );
        assert!(state.get_worktree("feat-notab").is_none());
    }

    #[test]
    fn test_expand_template_all_vars() {
        let result = expand_template("{ws}/{branch}:{pkg}", "myws", "feature-x", "frontend");
        assert_eq!(result, "myws/feature-x:frontend");
    }

    #[test]
    fn test_expand_template_no_pkg() {
        let result = expand_template("{ws}/{branch}", "myws", "main", "");
        assert_eq!(result, "myws/main");
    }

    #[test]
    fn test_expand_template_custom() {
        let result = expand_template("[{branch}] {pkg}", "ws", "dev", "api");
        assert_eq!(result, "[dev] api");
    }

    // --- Sync tests ---

    /// A configurable mock for GitOps that allows per-path control over return values
    /// and records all calls for verification.
    struct ConfigurableMockGit {
        divergences: Mutex<HashMap<String, (u32, u32)>>,
        conflicts: Mutex<HashMap<String, Vec<String>>>,
        heads: Mutex<HashMap<String, String>>,
        fetch_should_fail: Mutex<Vec<String>>,
        rebase_should_fail: Mutex<Vec<String>>,
        rebase_calls: Mutex<Vec<(String, String, String)>>,
        merge_calls: Mutex<Vec<(String, String, String)>>,
        fetch_calls: Mutex<Vec<String>>,
        reset_calls: Mutex<Vec<(String, String)>>,
        conflict_check_calls: Mutex<Vec<String>>,
        fast_forward_calls: Mutex<Vec<(String, String, String)>>,
    }

    impl ConfigurableMockGit {
        fn new() -> Self {
            Self {
                divergences: Mutex::new(HashMap::new()),
                conflicts: Mutex::new(HashMap::new()),
                heads: Mutex::new(HashMap::new()),
                fetch_should_fail: Mutex::new(Vec::new()),
                rebase_should_fail: Mutex::new(Vec::new()),
                rebase_calls: Mutex::new(Vec::new()),
                merge_calls: Mutex::new(Vec::new()),
                fetch_calls: Mutex::new(Vec::new()),
                reset_calls: Mutex::new(Vec::new()),
                conflict_check_calls: Mutex::new(Vec::new()),
                fast_forward_calls: Mutex::new(Vec::new()),
            }
        }

        fn set_divergence(&self, path: &Path, ahead: u32, behind: u32) {
            self.divergences
                .lock()
                .unwrap()
                .insert(path.to_string_lossy().to_string(), (ahead, behind));
        }

        fn set_conflicts(&self, path: &Path, files: Vec<String>) {
            self.conflicts
                .lock()
                .unwrap()
                .insert(path.to_string_lossy().to_string(), files);
        }

        #[allow(dead_code)] // Available for future test scenarios
        fn set_head(&self, path: &Path, sha: &str) {
            self.heads
                .lock()
                .unwrap()
                .insert(path.to_string_lossy().to_string(), sha.to_string());
        }

        fn add_fetch_failure(&self, path: &Path) {
            self.fetch_should_fail
                .lock()
                .unwrap()
                .push(path.to_string_lossy().to_string());
        }

        fn add_rebase_failure(&self, path: &Path) {
            self.rebase_should_fail
                .lock()
                .unwrap()
                .push(path.to_string_lossy().to_string());
        }
    }

    impl GitOps for ConfigurableMockGit {
        fn clone_repo(&self, _url: &str, _path: &Path) -> Result<()> {
            Ok(())
        }
        fn worktree_add(&self, _repo: &Path, _dest: &Path, _branch: &str) -> Result<()> {
            Ok(())
        }
        fn worktree_remove(&self, _repo: &Path, _path: &Path, _force: bool) -> Result<()> {
            Ok(())
        }
        fn is_dirty(&self, _path: &Path) -> Result<bool> {
            Ok(false)
        }
        fn status_porcelain(&self, _path: &Path) -> Result<String> {
            Ok(String::new())
        }
        fn ensure_remote_tracking(&self, _path: &Path, _remote: &str) -> Result<()> {
            Ok(())
        }

        fn fetch(&self, path: &Path, _remote: &str) -> Result<()> {
            let key = path.to_string_lossy().to_string();
            self.fetch_calls.lock().unwrap().push(key.clone());
            if self.fetch_should_fail.lock().unwrap().contains(&key) {
                return Err(MeldrError::Git(format!("fetch failed for {key}")));
            }
            Ok(())
        }

        fn rebase(&self, path: &Path, onto: &str, strategy: &str, _autostash: bool) -> Result<()> {
            let key = path.to_string_lossy().to_string();
            self.rebase_calls.lock().unwrap().push((
                key.clone(),
                onto.to_string(),
                strategy.to_string(),
            ));
            if self.rebase_should_fail.lock().unwrap().contains(&key) {
                return Err(MeldrError::Git(format!("rebase failed for {key}")));
            }
            Ok(())
        }

        fn merge(&self, path: &Path, branch: &str, strategy: &str) -> Result<()> {
            let key = path.to_string_lossy().to_string();
            self.merge_calls
                .lock()
                .unwrap()
                .push((key, branch.to_string(), strategy.to_string()));
            Ok(())
        }

        fn detect_default_branch(&self, _path: &Path, _remote: &str) -> Option<String> {
            None
        }

        fn divergence(&self, path: &Path, _upstream: &str) -> Result<(u32, u32)> {
            let key = path.to_string_lossy().to_string();
            let divs = self.divergences.lock().unwrap();
            Ok(*divs.get(&key).unwrap_or(&(0, 0)))
        }

        fn check_merge_conflicts(&self, path: &Path, _upstream: &str) -> Result<Vec<String>> {
            let key = path.to_string_lossy().to_string();
            self.conflict_check_calls.lock().unwrap().push(key.clone());
            let conflicts = self.conflicts.lock().unwrap();
            Ok(conflicts.get(&key).cloned().unwrap_or_default())
        }

        fn current_head(&self, path: &Path) -> Result<String> {
            let key = path.to_string_lossy().to_string();
            let heads = self.heads.lock().unwrap();
            Ok(heads
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "default_sha".to_string()))
        }

        fn reset_hard(&self, path: &Path, commit: &str) -> Result<()> {
            let key = path.to_string_lossy().to_string();
            self.reset_calls
                .lock()
                .unwrap()
                .push((key, commit.to_string()));
            Ok(())
        }

        fn fast_forward_branch(&self, repo: &Path, branch: &str, remote: &str) -> Result<()> {
            self.fast_forward_calls.lock().unwrap().push((
                repo.to_string_lossy().to_string(),
                branch.to_string(),
                remote.to_string(),
            ));
            Ok(())
        }
    }

    /// Helper to create worktree directories on disk so sync doesn't skip them.
    fn create_worktree_dirs(root: &Path, branch: &str, packages: &[&str]) {
        for pkg in packages {
            let wt_path = crate::core::workspace::worktree_path(root, branch, pkg);
            std::fs::create_dir_all(&wt_path).unwrap();
        }
    }

    #[test]
    fn test_sync_basic_rebase() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        let be_wt = crate::core::workspace::worktree_path(root, "feature-x", "backend");
        git.set_divergence(&fe_wt, 0, 3);
        git.set_divergence(&be_wt, 0, 5);

        let config = EffectiveConfig::default();
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 2);
        for o in &outcomes {
            assert_eq!(
                o.status,
                SyncStatus::Synced,
                "package {} should be Synced",
                o.package
            );
        }
        // Verify rebase was called for both
        let rebase_calls = git.rebase_calls.lock().unwrap();
        assert_eq!(rebase_calls.len(), 2);
    }

    #[test]
    fn test_sync_already_up_to_date() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();
        // divergence defaults to (0, 0) = already up to date

        let config = EffectiveConfig::default();
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 2);
        for o in &outcomes {
            assert_eq!(
                o.status,
                SyncStatus::UpToDate,
                "package {} should be UpToDate",
                o.package
            );
        }
        // No rebase calls
        assert!(git.rebase_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn test_sync_missing_worktree_skipped() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        // Don't create worktree dirs - they should be skipped

        let git = ConfigurableMockGit::new();
        let config = EffectiveConfig::default();
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 2);
        for o in &outcomes {
            match &o.status {
                SyncStatus::Skipped(reason) => {
                    assert!(
                        reason.contains("worktree does not exist"),
                        "unexpected skip reason: {reason}"
                    );
                }
                other => panic!("expected Skipped, got {:?} for {}", other, o.package),
            }
        }
    }

    #[test]
    fn test_sync_dry_run_no_changes() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 0, 2);

        let config = EffectiveConfig::default();
        let options = SyncOptions {
            dry_run: true,
            ..Default::default()
        };

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, SyncStatus::Synced); // would sync
        // Fetch should have happened
        assert!(!git.fetch_calls.lock().unwrap().is_empty());
        // But no rebase or merge calls
        assert!(git.rebase_calls.lock().unwrap().is_empty());
        assert!(git.merge_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn test_sync_only_filter() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 0, 1);

        let config = EffectiveConfig::default();
        let options = SyncOptions {
            only: vec!["frontend".to_string()],
            ..Default::default()
        };

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].package, "frontend");
    }

    #[test]
    fn test_sync_exclude_filter() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 0, 1);

        let config = EffectiveConfig::default();
        let options = SyncOptions {
            exclude: vec!["backend".to_string()],
            ..Default::default()
        };

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].package, "frontend");
    }

    #[test]
    fn test_sync_safe_strategy_detects_conflicts() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 2, 3); // ahead > 0
        git.set_conflicts(
            &fe_wt,
            vec!["src/main.rs".to_string(), "Cargo.toml".to_string()],
        );

        let config = EffectiveConfig::default(); // sync_strategy = "safe"
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0].status {
            SyncStatus::Conflict(files) => {
                assert_eq!(files.len(), 2);
                assert!(files.contains(&"src/main.rs".to_string()));
                assert!(files.contains(&"Cargo.toml".to_string()));
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        // Should not have called rebase
        assert!(git.rebase_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn test_sync_safe_strategy_no_conflicts_proceeds() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 2, 3); // ahead > 0
        // No conflicts set - defaults to empty

        let config = EffectiveConfig::default(); // sync_strategy = "safe"
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, SyncStatus::Synced);
        // Conflict check should have been called
        assert_eq!(git.conflict_check_calls.lock().unwrap().len(), 1);
        // Rebase should have been called
        assert_eq!(git.rebase_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_sync_safe_strategy_no_local_commits_proceeds() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 0, 5); // ahead = 0, behind > 0

        let config = EffectiveConfig::default(); // sync_strategy = "safe"
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, SyncStatus::Synced);
        // Conflict check should NOT have been called (ahead == 0)
        assert!(git.conflict_check_calls.lock().unwrap().is_empty());
        // Rebase should have been called
        assert_eq!(git.rebase_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_sync_theirs_strategy_skips_conflict_check() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 3, 2); // ahead > 0

        let config = EffectiveConfig::default();
        let options = SyncOptions {
            strategy_override: Some("theirs".to_string()),
            ..Default::default()
        };

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, SyncStatus::Synced);
        // No conflict check should have been made
        assert!(git.conflict_check_calls.lock().unwrap().is_empty());
        // Rebase should have been called with "theirs" strategy
        let rebase_calls = git.rebase_calls.lock().unwrap();
        assert_eq!(rebase_calls.len(), 1);
        assert_eq!(rebase_calls[0].2, "theirs");
    }

    #[test]
    fn test_sync_merge_method() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 0, 2);

        let config = EffectiveConfig::default();
        let options = SyncOptions {
            method_override: Some("merge".to_string()),
            ..Default::default()
        };

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, SyncStatus::Synced);
        assert_eq!(outcomes[0].method, "merge");
        // Merge should be called, not rebase
        assert!(git.rebase_calls.lock().unwrap().is_empty());
        assert_eq!(git.merge_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_sync_fetch_failure() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();
        let fe_repo = crate::core::workspace::package_path(root, "frontend");
        git.add_fetch_failure(&fe_repo);

        let be_wt = crate::core::workspace::worktree_path(root, "feature-x", "backend");
        git.set_divergence(&be_wt, 0, 1);

        let config = EffectiveConfig::default();
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 2);

        let fe_outcome = outcomes.iter().find(|o| o.package == "frontend").unwrap();
        match &fe_outcome.status {
            SyncStatus::Failed(msg) => assert!(
                msg.contains("fetch failed"),
                "expected fetch failure message, got: {msg}"
            ),
            other => panic!("expected Failed for frontend, got {other:?}"),
        }

        let be_outcome = outcomes.iter().find(|o| o.package == "backend").unwrap();
        assert_eq!(be_outcome.status, SyncStatus::Synced);
    }

    #[test]
    fn test_sync_rebase_failure() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        git.set_divergence(&fe_wt, 0, 3);
        git.add_rebase_failure(&fe_wt);

        let config = EffectiveConfig::default();
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 1);
        match &outcomes[0].status {
            SyncStatus::Failed(msg) => assert!(
                msg.contains("rebase failed"),
                "expected rebase failure, got: {msg}"
            ),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn test_sync_per_package_strategy() {
        let (tmp, _) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        // Create a manifest where frontend has sync_strategy="theirs" override
        let manifest = Manifest {
            workspace: WorkspaceInfo {
                name: "test-ws".to_string(),
            },
            settings: Default::default(),
            layout: None,
            packages: vec![
                PackageEntry {
                    name: "frontend".to_string(),
                    url: "https://example.com/frontend.git".to_string(),
                    branch: None,
                    remote: None,
                    sync_strategy: Some("theirs".to_string()),
                },
                PackageEntry {
                    name: "backend".to_string(),
                    url: "https://example.com/backend.git".to_string(),
                    branch: None,
                    remote: None,
                    sync_strategy: None,
                },
            ],
        };

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        let be_wt = crate::core::workspace::worktree_path(root, "feature-x", "backend");
        git.set_divergence(&fe_wt, 1, 2); // ahead > 0 to trigger strategy check
        git.set_divergence(&be_wt, 1, 2);

        let config = EffectiveConfig::default(); // global strategy = "safe"
        let options = SyncOptions::default();

        let outcomes =
            sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();
        assert_eq!(outcomes.len(), 2);

        // Frontend uses "theirs" strategy - no conflict check
        // Backend uses "safe" strategy - conflict check happens
        let conflict_checks = git.conflict_check_calls.lock().unwrap();
        let be_wt_str = be_wt.to_string_lossy().to_string();
        let fe_wt_str = fe_wt.to_string_lossy().to_string();
        assert!(
            conflict_checks.contains(&be_wt_str),
            "backend should have conflict check (safe strategy)"
        );
        assert!(
            !conflict_checks.contains(&fe_wt_str),
            "frontend should NOT have conflict check (theirs strategy)"
        );

        // Frontend rebase should use "theirs" strategy
        let rebase_calls = git.rebase_calls.lock().unwrap();
        let fe_rebase = rebase_calls
            .iter()
            .find(|(p, _, _)| p == &fe_wt_str)
            .unwrap();
        assert_eq!(fe_rebase.2, "theirs");

        // Backend rebase should use "manual" strategy (safe -> manual mapping)
        let be_rebase = rebase_calls
            .iter()
            .find(|(p, _, _)| p == &be_wt_str)
            .unwrap();
        assert_eq!(be_rebase.2, "manual");
    }

    #[test]
    fn test_undo_sync() {
        let (tmp, _manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();

        let snapshot = crate::core::sync_history::SyncSnapshot {
            timestamp: 1234567890,
            branch: "feature-x".to_string(),
            packages: {
                let mut m = HashMap::new();
                m.insert("frontend".to_string(), "abc123".to_string());
                m.insert("backend".to_string(), "def456".to_string());
                m
            },
        };

        let results = undo_sync(&git, root, "feature-x", &snapshot).unwrap();
        assert_eq!(results.len(), 2);
        for (_, result) in &results {
            assert!(result.is_ok(), "reset should succeed");
        }

        let reset_calls = git.reset_calls.lock().unwrap();
        assert_eq!(reset_calls.len(), 2);

        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        let be_wt = crate::core::workspace::worktree_path(root, "feature-x", "backend");
        let fe_wt_str = fe_wt.to_string_lossy().to_string();
        let be_wt_str = be_wt.to_string_lossy().to_string();

        let fe_reset = reset_calls.iter().find(|(p, _)| p == &fe_wt_str).unwrap();
        assert_eq!(fe_reset.1, "abc123");

        let be_reset = reset_calls.iter().find(|(p, _)| p == &be_wt_str).unwrap();
        assert_eq!(be_reset.1, "def456");
    }

    #[test]
    fn test_sync_fast_forwards_bare_repo_main() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();
        let fe_wt = crate::core::workspace::worktree_path(root, "feature-x", "frontend");
        let be_wt = crate::core::workspace::worktree_path(root, "feature-x", "backend");
        git.set_divergence(&fe_wt, 0, 3);
        git.set_divergence(&be_wt, 0, 5);

        let config = EffectiveConfig::default();
        let options = SyncOptions::default();

        sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();

        // Verify fast_forward_branch was called for each package's bare repo
        let ff_calls = git.fast_forward_calls.lock().unwrap();
        assert_eq!(ff_calls.len(), 2, "fast_forward_branch should be called for each package");

        let fe_repo = crate::core::workspace::package_path(root, "frontend");
        let be_repo = crate::core::workspace::package_path(root, "backend");

        let fe_ff = ff_calls.iter().find(|(r, _, _)| r == &fe_repo.to_string_lossy().to_string());
        assert!(fe_ff.is_some(), "should fast-forward frontend bare repo");
        assert_eq!(fe_ff.unwrap().1, "main", "should fast-forward the default branch");
        assert_eq!(fe_ff.unwrap().2, "origin", "should use the configured remote");

        let be_ff = ff_calls.iter().find(|(r, _, _)| r == &be_repo.to_string_lossy().to_string());
        assert!(be_ff.is_some(), "should fast-forward backend bare repo");
    }

    #[test]
    fn test_sync_skips_fast_forward_on_fetch_failure() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let root = tmp.path();
        create_worktree_dirs(root, "feature-x", &["frontend", "backend"]);

        let git = ConfigurableMockGit::new();
        let fe_repo = crate::core::workspace::package_path(root, "frontend");
        git.add_fetch_failure(&fe_repo);

        let config = EffectiveConfig::default();
        let options = SyncOptions::default();

        sync_worktree(&git, &manifest, root, "feature-x", &config, &options).unwrap();

        // fast_forward should only be called for backend (frontend fetch failed)
        let ff_calls = git.fast_forward_calls.lock().unwrap();
        assert_eq!(ff_calls.len(), 1, "should skip fast-forward for failed fetch");
        let be_repo = crate::core::workspace::package_path(root, "backend");
        assert_eq!(ff_calls[0].0, be_repo.to_string_lossy().to_string());
    }
}

// --- Sync types ---

/// Status of a single package sync operation.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncStatus {
    /// Successfully synced.
    Synced,
    /// Already up to date (0 behind).
    UpToDate,
    /// Skipped with a reason.
    Skipped(String),
    /// Conflicts detected (list of conflicting files).
    Conflict(Vec<String>),
    /// Failed with an error message.
    Failed(String),
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncStatus::Synced => write!(f, "synced"),
            SyncStatus::UpToDate => write!(f, "up-to-date"),
            SyncStatus::Skipped(reason) => write!(f, "skipped: {reason}"),
            SyncStatus::Conflict(files) => write!(f, "conflict: {}", files.join(", ")),
            SyncStatus::Failed(msg) => write!(f, "failed: {msg}"),
        }
    }
}

/// Outcome of syncing a single package.
#[derive(Debug, Clone)]
pub struct PackageSyncOutcome {
    pub package: String,
    pub status: SyncStatus,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub method: String,
    #[allow(dead_code)] // Retained for future sync history tracking
    pub previous_head: Option<String>,
}

/// Options for controlling sync behavior.
#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    pub method_override: Option<String>,
    pub strategy_override: Option<String>,
    pub dry_run: bool,
    pub only: Vec<String>,
    pub exclude: Vec<String>,
}

/// Fetch all packages from their remotes without rebasing/merging any worktree.
#[allow(dead_code)] // Planned for use in future fetch subcommand
pub fn fetch_packages(
    git: &dyn GitOps,
    manifest: &Manifest,
    workspace_root: &Path,
    config: &EffectiveConfig,
) -> Result<()> {
    for pkg in &manifest.packages {
        let repo_path = workspace::package_path(workspace_root, &pkg.name);
        let remote = pkg.remote.as_deref().unwrap_or(&config.remote);
        eprintln!("Fetching {}...", pkg.name);
        git.fetch(&repo_path, remote)?;
        // Fast-forward the bare repo's default branch to match remote
        let detected = git.detect_default_branch(&repo_path, remote);
        let default_branch = pkg
            .branch
            .as_deref()
            .or(detected.as_deref())
            .unwrap_or(&config.default_branch);
        let _ = git.fast_forward_branch(&repo_path, default_branch, remote);
    }
    Ok(())
}

/// Check which worktrees have packages that are behind upstream.
/// Returns a list of (branch, package, behind_count) tuples.
/// This does NOT fetch — it only checks against already-fetched refs.
pub fn check_worktree_staleness(
    git: &dyn GitOps,
    manifest: &Manifest,
    workspace_root: &Path,
    worktree_branches: &[String],
    config: &EffectiveConfig,
) -> Vec<(String, String, u32)> {
    let mut stale = Vec::new();
    for branch in worktree_branches {
        for pkg in &manifest.packages {
            let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
            if !wt_path.exists() {
                continue;
            }
            let repo_path = workspace::package_path(workspace_root, &pkg.name);
            let remote = pkg.remote.as_deref().unwrap_or(&config.remote);
            let detected = git.detect_default_branch(&repo_path, remote);
            let default_branch = pkg
                .branch
                .as_deref()
                .or(detected.as_deref())
                .unwrap_or(&config.default_branch);
            let upstream = format!("{remote}/{default_branch}");
            if let Ok((_ahead, behind)) = git.divergence(&wt_path, &upstream)
                && behind > 0
            {
                stale.push((branch.clone(), pkg.name.clone(), behind));
            }
        }
    }
    stale
}

pub fn sync_worktree(
    git: &dyn GitOps,
    manifest: &Manifest,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    options: &SyncOptions,
) -> Result<Vec<PackageSyncOutcome>> {
    let method = options
        .method_override
        .as_deref()
        .unwrap_or(&config.sync_method);
    let global_strategy = options
        .strategy_override
        .as_deref()
        .unwrap_or(&config.sync_strategy);

    // Filter packages based on --only / --exclude
    let packages: Vec<&crate::core::workspace::PackageEntry> = manifest
        .packages
        .iter()
        .filter(|pkg| {
            if !options.only.is_empty() {
                return options.only.iter().any(|o| o == &pkg.name);
            }
            if !options.exclude.is_empty() {
                return !options.exclude.iter().any(|e| e == &pkg.name);
            }
            true
        })
        .collect();

    // Phase 1: Parallel fetch
    let fetch_results: Vec<(
        &crate::core::workspace::PackageEntry,
        std::result::Result<(), String>,
    )> = packages
        .par_iter()
        .map(|pkg| {
            let repo_path = workspace::package_path(workspace_root, &pkg.name);
            let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);
            if !wt_path.exists() {
                return (*pkg, Err("worktree does not exist".to_string()));
            }
            let remote = pkg.remote.as_deref().unwrap_or(&config.remote);
            match git.fetch(&repo_path, remote) {
                Ok(()) => (*pkg, Ok(())),
                Err(e) => (*pkg, Err(e.to_string())),
            }
        })
        .collect();

    // Phase 1.5: Fast-forward each bare repo's default branch to match remote.
    // Without this, the bare repo's main drifts behind origin/main, causing
    // stale worktree bases and detached HEAD warnings.
    for (pkg, fetch_result) in &fetch_results {
        if fetch_result.is_err() {
            continue;
        }
        let repo_path = workspace::package_path(workspace_root, &pkg.name);
        let remote = pkg.remote.as_deref().unwrap_or(&config.remote);
        let detected = git.detect_default_branch(&repo_path, remote);
        let default_branch = pkg
            .branch
            .as_deref()
            .or(detected.as_deref())
            .unwrap_or(&config.default_branch);
        // Best-effort: fails gracefully if not a fast-forward or branch is checked out by a worktree
        let _ = git.fast_forward_branch(&repo_path, default_branch, remote);
    }

    // Phase 2: Sequential analysis + sync per package
    let mut outcomes = Vec::new();

    for (pkg, fetch_result) in &fetch_results {
        let repo_path = workspace::package_path(workspace_root, &pkg.name);
        let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);

        // Handle missing worktree
        if !wt_path.exists() {
            outcomes.push(PackageSyncOutcome {
                package: pkg.name.clone(),
                status: SyncStatus::Skipped("worktree does not exist".to_string()),
                ahead: None,
                behind: None,
                method: method.to_string(),
                previous_head: None,
            });
            continue;
        }

        // Handle fetch failure
        if let Err(e) = fetch_result {
            outcomes.push(PackageSyncOutcome {
                package: pkg.name.clone(),
                status: SyncStatus::Failed(format!("fetch failed: {e}")),
                ahead: None,
                behind: None,
                method: method.to_string(),
                previous_head: None,
            });
            continue;
        }

        let remote = pkg.remote.as_deref().unwrap_or(&config.remote);

        // Resolve default branch
        let detected;
        let default_branch = if let Some(ref b) = pkg.branch {
            b.as_str()
        } else {
            detected = git.detect_default_branch(&repo_path, remote);
            detected.as_deref().unwrap_or(&config.default_branch)
        };

        let upstream = format!("{remote}/{default_branch}");

        // Get divergence info
        let (ahead, behind) = git.divergence(&wt_path, &upstream).unwrap_or((0, 0));

        // Record current HEAD for snapshot
        let previous_head = git.current_head(&wt_path).ok();

        // Already up to date
        if behind == 0 {
            outcomes.push(PackageSyncOutcome {
                package: pkg.name.clone(),
                status: SyncStatus::UpToDate,
                ahead: Some(ahead),
                behind: Some(0),
                method: method.to_string(),
                previous_head,
            });
            continue;
        }

        // Resolve per-package strategy (package override > global)
        let strategy = pkg.sync_strategy.as_deref().unwrap_or(global_strategy);

        // "safe" strategy: check for conflicts before proceeding
        if strategy == "safe" && ahead > 0 {
            let conflicts = git
                .check_merge_conflicts(&wt_path, &upstream)
                .unwrap_or_default();
            if !conflicts.is_empty() {
                outcomes.push(PackageSyncOutcome {
                    package: pkg.name.clone(),
                    status: SyncStatus::Conflict(conflicts),
                    ahead: Some(ahead),
                    behind: Some(behind),
                    method: method.to_string(),
                    previous_head,
                });
                continue;
            }
        }

        // Dry run — don't actually sync
        if options.dry_run {
            outcomes.push(PackageSyncOutcome {
                package: pkg.name.clone(),
                status: SyncStatus::Synced, // would sync
                ahead: Some(ahead),
                behind: Some(behind),
                method: method.to_string(),
                previous_head,
            });
            continue;
        }

        // Determine the git strategy flag to pass
        // "safe" = no -X flag (git stops naturally on conflict)
        let git_strategy = if strategy == "safe" {
            "manual"
        } else {
            strategy
        };

        // Perform the sync
        let result = if method == "merge" {
            git.merge(&wt_path, &upstream, git_strategy)
        } else {
            git.rebase(&wt_path, &upstream, git_strategy, true)
        };

        match result {
            Ok(()) => {
                outcomes.push(PackageSyncOutcome {
                    package: pkg.name.clone(),
                    status: SyncStatus::Synced,
                    ahead: Some(ahead),
                    behind: Some(behind),
                    method: method.to_string(),
                    previous_head,
                });
            }
            Err(e) => {
                outcomes.push(PackageSyncOutcome {
                    package: pkg.name.clone(),
                    status: SyncStatus::Failed(e.to_string()),
                    ahead: Some(ahead),
                    behind: Some(behind),
                    method: method.to_string(),
                    previous_head,
                });
            }
        }
    }

    Ok(outcomes)
}

/// Undo the last sync for a branch by resetting packages to their pre-sync HEADs.
pub fn undo_sync(
    git: &dyn GitOps,
    workspace_root: &Path,
    branch: &str,
    snapshot: &crate::core::sync_history::SyncSnapshot,
) -> Result<Vec<(String, std::result::Result<(), String>)>> {
    let mut results = Vec::new();
    for (pkg_name, sha) in &snapshot.packages {
        let wt_path = workspace::worktree_path(workspace_root, branch, pkg_name);
        if !wt_path.exists() {
            results.push((pkg_name.clone(), Err("worktree does not exist".to_string())));
            continue;
        }
        match git.reset_hard(&wt_path, sha) {
            Ok(()) => results.push((pkg_name.clone(), Ok(()))),
            Err(e) => results.push((pkg_name.clone(), Err(e.to_string()))),
        }
    }
    Ok(results)
}
