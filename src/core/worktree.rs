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
    pkg_names: &[String],
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
            let target = format!("{}.{}", window_id, i);
            tmux.send_keys(&target, &format!("cd {}", wt_path.display()))?;
            if config.should_launch_agent() {
                tmux.send_keys(&target, &config.agent_command)?;
            }
            pane_mappings.insert(i.to_string(), pkg_name.clone());
        }

        tmux_windows.push(window_id);
    } else {
        let custom_layout = global_config.and_then(|gc| gc.layouts.get(&config.layout));

        for pkg_name in pkg_names {
            let wt_path = workspace::worktree_path(workspace_root, branch, pkg_name);
            if !wt_path.exists() {
                eprintln!("Warning: worktree path for '{}' does not exist, skipping", pkg_name);
                continue;
            }
            let wt_path_str = wt_path.to_string_lossy().to_string();

            let window_name =
                expand_template(&config.window_name_template, ws_name, branch, pkg_name);

            let dev =
                tmux.create_dev_window(&window_name, &wt_path_str, config, custom_layout)?;

            if let Some(ref editor_pane) = dev.editor {
                tmux.send_keys(editor_pane, &config.editor)?;
                pane_mappings.insert(format!("{}:editor", pkg_name), editor_pane.clone());
            }

            if config.should_launch_agent() {
                if let Some(ref agent_pane) = dev.agent {
                    tmux.send_keys(agent_pane, &config.agent_command)?;
                }
            }
            if let Some(ref agent_pane) = dev.agent {
                pane_mappings.insert(format!("{}:agent", pkg_name), agent_pane.clone());
            }

            tmux_windows.push(dev.window_id);
        }
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

    let branch_dir = workspace::worktrees_dir(workspace_root).join(branch);
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
            Err((name, e)) => errors.push(format!("{}: {}", name, e)),
        }
    }

    if !errors.is_empty() && created.is_empty() {
        return Err(MeldrError::Git(format!(
            "Failed to create any worktrees: {}",
            errors.join(", ")
        )));
    }

    for error in &errors {
        eprintln!("Warning: {}", error);
    }

    let (tmux_window, pane_mappings) = if needs_tmux {
        setup_tmux_window(tmux, manifest, workspace_root, branch, &created, config)?
    } else {
        (None, HashMap::new())
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

/// Create tmux window(s) for a worktree. Returns (window_id, pane_mappings).
fn setup_tmux_window(
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    workspace_root: &Path,
    branch: &str,
    packages: &[String],
    config: &EffectiveConfig,
) -> Result<(Option<String>, HashMap<String, String>)> {
    let ws_name = &manifest.workspace.name;
    let window_name = format!("{}/{}", ws_name, branch);
    let mut pane_mappings = HashMap::new();

    if let Some(ref lo) = manifest.layout {
        // Layout override: custom pane arrangement from manifest
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
            let target = format!("{}.{}", window_id, i);
            tmux.send_keys(&target, &format!("cd {}", wt_path.display()))?;
            if config.should_launch_agent() {
                tmux.send_keys(&target, &config.agent_command)?;
            }
            pane_mappings.insert(i.to_string(), pkg_name.clone());
        }

        Ok((Some(window_id), pane_mappings))
    } else {
        // Dev window: single package uses its worktree dir, multi uses branch dir
        let cwd = if packages.len() == 1 {
            workspace::worktree_path(workspace_root, branch, &packages[0])
        } else {
            workspace::worktrees_dir(workspace_root).join(branch)
        };
        let cwd_str = cwd.to_string_lossy().into_owned();

        let dev = tmux.create_dev_window(&window_name, &cwd_str)?;

        tmux.send_keys(&dev.nvim, "nvim .")?;
        if config.should_launch_agent() {
            tmux.send_keys(&dev.agent, &config.agent_command)?;
        }

        pane_mappings.insert("nvim".to_string(), dev.nvim);
        pane_mappings.insert("agent".to_string(), dev.agent);
        Ok((Some(dev.window_id), pane_mappings))
    }
}

/// Open tmux windows for an existing worktree (session restore).
pub fn open_worktree(
    tmux: &dyn TmuxOps,
    manifest: &Manifest,
    state: &mut WorkspaceState,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
) -> Result<()> {
    if state.get_worktree(branch).is_none() {
        return Err(MeldrError::WorktreeNotFound(branch.to_string()));
    }

    if !config.should_use_tmux() {
        return Ok(());
    }

    if !tmux.is_inside_tmux() {
        return Err(MeldrError::NotInTmux);
    }

    // Collect package names that have worktree dirs on disk
    let packages: Vec<String> = manifest
        .packages
        .iter()
        .filter(|pkg| workspace::worktree_path(workspace_root, branch, &pkg.name).exists())
        .map(|pkg| pkg.name.clone())
        .collect();

    if packages.is_empty() {
        return Ok(());
    }

    let (tmux_window, pane_mappings) =
        setup_tmux_window(tmux, manifest, workspace_root, branch, &packages, config)?;

    // Update state with new tmux window ID
    state.add_worktree(
        branch,
        WorktreeState {
            branch: branch.to_string(),
            tmux_window,
            pane_mappings,
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
            if wt_path.exists() {
                if let Ok(true) = git.is_dirty(&wt_path) {
                    return Err(MeldrError::DirtyWorktree(
                        branch.to_string(),
                        pkg.name.clone(),
                    ));
                }
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
        if wt_path.exists() {
            if let Err(e) = git.worktree_remove(&repo_path, &wt_path, force) {
                eprintln!(
                    "Warning: Failed to remove worktree for '{}': {}",
                    pkg.name, e
                );
            }
        }
    }

    let branch_dir = workspace::worktrees_dir(workspace_root).join(branch);
    if branch_dir.exists() {
        let _ = std::fs::remove_dir_all(&branch_dir);
    }

    state.remove_worktree(branch);
    state.save(workspace_root)?;

    // Kill tmux window LAST — after all worktrees are removed and state is saved.
    // This way even if killing the window terminates this process, cleanup is complete.
    if let Some(ref window_id) = tmux_window_id {
        if let Err(e) = tmux.kill_window(window_id) {
            eprintln!("Warning: Could not kill tmux window '{}': {}", window_id, e);
        }
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

    let packages: Vec<String> = manifest.packages.iter().map(|p| p.name.clone()).collect();
    let setup = setup_tmux_windows(
        tmux, manifest, workspace_root, branch, &packages, config, global_config,
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

    /// Tracks all tmux calls for assertions
    #[derive(Debug, Default)]
    struct TmuxCall {
        create_dev_window: Vec<(String, String)>,       // (name, cwd)
        create_window: Vec<String>,                      // name
        split_window: Vec<String>,                       // window
        send_keys: Vec<(String, String)>,                // (target, keys)
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

        fn calls(&self) -> std::sync::MutexGuard<'_, TmuxCall> {
            self.calls.lock().unwrap()
        }
    }

    impl TmuxOps for MockTmux {
        fn is_inside_tmux(&self) -> bool {
            true
        }

        fn create_window(&self, name: &str) -> Result<String> {
            self.calls.lock().unwrap().create_window.push(name.to_string());
            Ok("@99".to_string())
        }

        fn split_window(&self, window: &str) -> Result<()> {
            self.calls.lock().unwrap().split_window.push(window.to_string());
            Ok(())
        }

        fn apply_layout(&self, _window: &str, _layout: &TmuxLayout) -> Result<()> {
            Ok(())
        }

        fn send_keys(&self, target: &str, keys: &str) -> Result<()> {
            self.calls.lock().unwrap().send_keys.push((target.to_string(), keys.to_string()));
            Ok(())
        }

        fn kill_window(&self, _window: &str) -> Result<()> {
            Ok(())
        }

        fn create_dev_window(&self, name: &str, cwd: &str) -> Result<DevWindowPanes> {
            self.calls.lock().unwrap().create_dev_window.push((name.to_string(), cwd.to_string()));
            Ok(DevWindowPanes {
                window_id: "@100".to_string(),
                nvim: "@100.0".to_string(),
                agent: "%1".to_string(),
                terms: vec!["%2".to_string(), "%3".to_string(), "%4".to_string(), "%5".to_string()],
            })
        }
    }

    struct MockGit;

    impl GitOps for MockGit {
        fn clone_repo(&self, _url: &str, _path: &Path) -> Result<()> { Ok(()) }
        fn worktree_add(&self, _repo: &Path, _dest: &Path, _branch: &str) -> Result<()> { Ok(()) }
        fn worktree_remove(&self, _repo: &Path, _path: &Path, _force: bool) -> Result<()> { Ok(()) }
        fn is_dirty(&self, _path: &Path) -> Result<bool> { Ok(false) }
        fn fetch(&self, _path: &Path) -> Result<()> { Ok(()) }
        fn rebase(&self, _path: &Path, _onto: &str, _strategy: &str, _autostash: bool) -> Result<()> { Ok(()) }
        fn merge(&self, _path: &Path, _branch: &str, _strategy: &str) -> Result<()> { Ok(()) }
        fn pull_ff_only(&self, _path: &Path) -> Result<()> { Ok(()) }
        fn status_porcelain(&self, _path: &Path) -> Result<String> { Ok(String::new()) }
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
                    url: format!("https://example.com/{}.git", name),
                    branch: None,
                })
                .collect(),
        }
    }

    fn tmux_config() -> EffectiveConfig {
        EffectiveConfig {
            no_tabs: false,
            no_agent: false,
            ..Default::default()
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

    #[test]
    fn test_single_package_creates_dev_window() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let tmux = MockTmux::new();
        let git = MockGit;
        let config = tmux_config();
        let mut state = WorkspaceState::default();

        add_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-1", &config).unwrap();

        let calls = tmux.calls();
        assert_eq!(calls.create_dev_window.len(), 1, "should create exactly one dev window");
        assert_eq!(calls.create_dev_window[0].0, "test-ws/feat-1");
        assert!(
            calls.create_dev_window[0].1.ends_with("worktrees/feat-1/frontend"),
            "single package dev window cwd should be the package worktree dir, got: {}",
            calls.create_dev_window[0].1,
        );
        assert!(calls.create_window.is_empty(), "should not create plain windows");
        assert!(calls.split_window.is_empty(), "should not split windows");

        // Pane mappings should use consistent keys regardless of package count
        let wt = state.get_worktree("feat-1").unwrap();
        assert!(wt.pane_mappings.contains_key("nvim"), "should use 'nvim' key");
        assert!(wt.pane_mappings.contains_key("agent"), "should use 'agent' key");
    }

    #[test]
    fn test_multi_package_creates_one_dev_window() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let tmux = MockTmux::new();
        let git = MockGit;
        let config = tmux_config();
        let mut state = WorkspaceState::default();

        add_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-2", &config).unwrap();

        let calls = tmux.calls();
        assert_eq!(
            calls.create_dev_window.len(), 1,
            "multi-package should create exactly one dev window (not one per package)"
        );
        assert_eq!(calls.create_dev_window[0].0, "test-ws/feat-2");
        assert!(
            calls.create_dev_window[0].1.ends_with("worktrees/feat-2"),
            "multi-package dev window cwd should be the branch dir, got: {}",
            calls.create_dev_window[0].1,
        );
        assert!(calls.create_window.is_empty(), "should not create plain windows");
        assert!(calls.split_window.is_empty(), "should not split windows (no pane-per-package)");
    }

    #[test]
    fn test_multi_package_sends_nvim_and_agent() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let tmux = MockTmux::new();
        let git = MockGit;
        let config = tmux_config();
        let mut state = WorkspaceState::default();

        add_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-3", &config).unwrap();

        let calls = tmux.calls();
        let nvim_keys: Vec<_> = calls.send_keys.iter().filter(|(_, k)| k == "nvim .").collect();
        assert_eq!(nvim_keys.len(), 1, "should launch nvim once");
        assert_eq!(nvim_keys[0].0, "@100.0", "nvim should be in the nvim pane");

        let agent_keys: Vec<_> = calls.send_keys.iter().filter(|(_, k)| k.contains("claude")).collect();
        assert_eq!(agent_keys.len(), 1, "should launch agent once");
        assert_eq!(agent_keys[0].0, "%1", "agent should be in the agent pane");
    }

    #[test]
    fn test_multi_package_state_has_one_window() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let tmux = MockTmux::new();
        let git = MockGit;
        let config = tmux_config();
        let mut state = WorkspaceState::default();

        add_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-4", &config).unwrap();

        let wt = state.get_worktree("feat-4").unwrap();
        assert_eq!(wt.tmux_window, Some("@100".to_string()), "should store single window ID");
        assert!(!wt.tmux_window.as_ref().unwrap().contains(','), "should not have comma-separated window IDs");
    }

    #[test]
    fn test_no_tabs_skips_tmux() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let tmux = MockTmux::new();
        let git = MockGit;
        let config = EffectiveConfig {
            no_tabs: true,
            ..Default::default()
        };
        let mut state = WorkspaceState::default();

        add_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-5", &config).unwrap();

        let calls = tmux.calls();
        assert!(calls.create_dev_window.is_empty(), "no-tabs should skip tmux entirely");
        assert!(calls.create_window.is_empty());

        let wt = state.get_worktree("feat-5").unwrap();
        assert!(wt.tmux_window.is_none());
    }

    #[test]
    fn test_open_worktree_creates_dev_window() {
        let (tmp, manifest) = setup_workspace(&["frontend", "backend"]);
        let git = MockGit;
        let no_tabs_config = EffectiveConfig {
            no_tabs: true,
            ..Default::default()
        };
        let mut state = WorkspaceState::default();

        // Create worktree without tmux
        add_worktree(&git, &MockTmux::new(), &manifest, &mut state, tmp.path(), "feat-open", &no_tabs_config).unwrap();
        assert!(state.get_worktree("feat-open").unwrap().tmux_window.is_none());

        // Create worktree dirs on disk (normally git does this)
        std::fs::create_dir_all(tmp.path().join("worktrees/feat-open/frontend")).unwrap();
        std::fs::create_dir_all(tmp.path().join("worktrees/feat-open/backend")).unwrap();

        // Now open with tmux
        let tmux = MockTmux::new();
        let config = tmux_config();
        open_worktree(&tmux, &manifest, &mut state, tmp.path(), "feat-open", &config).unwrap();

        let calls = tmux.calls();
        assert_eq!(calls.create_dev_window.len(), 1, "open should create a dev window");
        assert!(
            calls.create_dev_window[0].1.ends_with("worktrees/feat-open"),
            "multi-package open should use branch dir, got: {}",
            calls.create_dev_window[0].1,
        );

        let wt = state.get_worktree("feat-open").unwrap();
        assert!(wt.tmux_window.is_some(), "state should be updated with tmux window");
        assert!(wt.pane_mappings.contains_key("nvim"));
        assert!(wt.pane_mappings.contains_key("agent"));
    }

    #[test]
    fn test_open_nonexistent_worktree_fails() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let tmux = MockTmux::new();
        let config = tmux_config();
        let mut state = WorkspaceState::default();

        let result = open_worktree(&tmux, &manifest, &mut state, tmp.path(), "no-such", &config);
        assert!(result.is_err(), "open should fail for nonexistent worktree");
    }

    // --- Removal tests with order tracking ---

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Global counter for ordering assertions across mock objects
    static ORDER_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn reset_order_counter() {
        ORDER_COUNTER.store(0, Ordering::SeqCst);
    }

    fn next_order() -> usize {
        ORDER_COUNTER.fetch_add(1, Ordering::SeqCst)
    }

    /// A mock that tracks which packages had worktree_remove called and in what order
    struct OrderTrackingGit {
        removed_packages: Mutex<Vec<(String, usize)>>, // (path, order)
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
        fn clone_repo(&self, _url: &str, _path: &Path) -> Result<()> { Ok(()) }
        fn worktree_add(&self, _repo: &Path, _dest: &Path, _branch: &str) -> Result<()> { Ok(()) }
        fn worktree_remove(&self, _repo: &Path, path: &Path, _force: bool) -> Result<()> {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            self.removed_packages.lock().unwrap().push((name, next_order()));
            Ok(())
        }
        fn is_dirty(&self, _path: &Path) -> Result<bool> { Ok(false) }
        fn fetch(&self, _path: &Path) -> Result<()> { Ok(()) }
        fn rebase(&self, _path: &Path, _onto: &str, _strategy: &str, _autostash: bool) -> Result<()> { Ok(()) }
        fn merge(&self, _path: &Path, _branch: &str, _strategy: &str) -> Result<()> { Ok(()) }
        fn pull_ff_only(&self, _path: &Path) -> Result<()> { Ok(()) }
        fn status_porcelain(&self, _path: &Path) -> Result<String> { Ok(String::new()) }
    }

    /// A tmux mock that tracks when kill_window is called relative to git operations
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
        fn is_inside_tmux(&self) -> bool { true }
        fn create_window(&self, _name: &str) -> Result<String> { Ok("@99".to_string()) }
        fn split_window(&self, _window: &str) -> Result<()> { Ok(()) }
        fn apply_layout(&self, _window: &str, _layout: &TmuxLayout) -> Result<()> { Ok(()) }
        fn send_keys(&self, _target: &str, _keys: &str) -> Result<()> { Ok(()) }
        fn kill_window(&self, _window: &str) -> Result<()> {
            *self.kill_order.lock().unwrap() = Some(next_order());
            Ok(())
        }
        fn create_dev_window(&self, _name: &str, _cwd: &str) -> Result<DevWindowPanes> {
            Ok(DevWindowPanes {
                window_id: "@100".to_string(),
                nvim: "@100.0".to_string(),
                agent: "%1".to_string(),
                terms: vec!["%2".to_string(), "%3".to_string(), "%4".to_string(), "%5".to_string()],
            })
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

        // Register the worktree in state with a tmux window
        state.add_worktree("feat-rm", worktree_state_with_window("feat-rm", "@50"));

        // Create worktree dirs on disk (simulating what git worktree add does)
        for pkg in packages {
            std::fs::create_dir_all(
                tmp.path().join("worktrees").join("feat-rm").join(pkg),
            )
            .unwrap();
        }

        remove_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-rm", false).unwrap();

        let removed = git.removed();
        assert_eq!(
            removed.len(),
            3,
            "should remove worktrees for ALL 3 packages, got: {:?}",
            removed,
        );
        let removed_names: Vec<&str> = removed.iter().map(|(n, _)| n.as_str()).collect();
        for pkg in packages {
            assert!(
                removed_names.contains(pkg),
                "package '{}' should have been removed, got: {:?}",
                pkg,
                removed_names,
            );
        }

        // State should be cleared
        assert!(state.get_worktree("feat-rm").is_none(), "worktree should be removed from state");

        // Branch dir should be gone
        assert!(
            !tmp.path().join("worktrees").join("feat-rm").exists(),
            "branch directory should be cleaned up",
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

        state.add_worktree("feat-order", worktree_state_with_window("feat-order", "@60"));

        for pkg in packages {
            std::fs::create_dir_all(
                tmp.path().join("worktrees").join("feat-order").join(pkg),
            )
            .unwrap();
        }

        remove_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-order", false).unwrap();

        let removed = git.removed();
        assert_eq!(removed.len(), 2, "both packages should be removed");

        let max_git_order = removed.iter().map(|(_, ord)| *ord).max().unwrap();
        let kill_order = tmux.kill_order().expect("tmux kill_window should have been called");

        assert!(
            kill_order > max_git_order,
            "tmux kill_window (order={}) must happen AFTER all git worktree removals (last git order={})",
            kill_order,
            max_git_order,
        );
    }

    #[test]
    fn test_remove_worktree_nonexistent_fails() {
        let (tmp, manifest) = setup_workspace(&["frontend"]);
        let git = MockGit;
        let tmux = MockTmux::new();
        let mut state = WorkspaceState::default();

        let result = remove_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "no-such", false);
        assert!(result.is_err(), "removing nonexistent worktree should fail");
    }

    #[test]
    fn test_remove_worktree_no_tmux_window() {
        let packages = &["frontend"];
        let (tmp, manifest) = setup_workspace(packages);
        let git = OrderTrackingGit::new();
        let tmux = OrderTrackingTmux::new();
        let mut state = WorkspaceState::default();

        // Add worktree without tmux window (e.g., created with --no-tabs)
        state.add_worktree("feat-notab", worktree_state_no_window("feat-notab"));

        std::fs::create_dir_all(
            tmp.path().join("worktrees").join("feat-notab").join("frontend"),
        )
        .unwrap();

        remove_worktree(&git, &tmux, &manifest, &mut state, tmp.path(), "feat-notab", false).unwrap();

        let removed = git.removed();
        assert_eq!(removed.len(), 1, "should still remove git worktree");
        assert!(tmux.kill_order().is_none(), "should not call kill_window when no tmux window");
        assert!(state.get_worktree("feat-notab").is_none());
    }
}

pub fn sync_worktree(
    git: &dyn GitOps,
    manifest: &Manifest,
    workspace_root: &Path,
    branch: &str,
    config: &EffectiveConfig,
    method_override: Option<&str>,
    strategy_override: Option<&str>,
) -> Result<()> {
    let method = method_override.unwrap_or(&config.sync_method);
    let strategy = strategy_override.unwrap_or(&config.sync_strategy);

    for pkg in &manifest.packages {
        let repo_path = workspace::package_path(workspace_root, &pkg.name);
        let wt_path = workspace::worktree_path(workspace_root, branch, &pkg.name);

        if !wt_path.exists() {
            eprintln!(
                "Warning: Worktree for '{}' on branch '{}' does not exist, skipping",
                pkg.name, branch
            );
            continue;
        }

        let remote = pkg.remote.as_deref().unwrap_or(&config.remote);

        println!("Syncing {}...", pkg.name);
        git.fetch(&repo_path, remote)?;

        // Resolve default branch: explicit > auto-detect > config fallback
        let detected;
        let default_branch = if let Some(ref b) = pkg.branch {
            b.as_str()
        } else {
            detected = git.detect_default_branch(&repo_path, remote);
            detected.as_deref().unwrap_or(&config.default_branch)
        };

        // Rebase/merge against the remote-tracking branch (bare repos have no local checkout)
        let upstream = format!("{}/{}", remote, default_branch);
        if method == "merge" {
            git.merge(&wt_path, &upstream, strategy)?;
        } else {
            git.rebase(&wt_path, &upstream, strategy, true)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
