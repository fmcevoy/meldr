use crate::core::workspace::{Manifest, PackageEntry, WorkspaceHooks};
use rayon::prelude::*;
use std::path::PathBuf;
use std::process::Command;

/// Resolve which hooks to run for a given event and package.
/// Per-package hooks replace (not append to) workspace-level hooks.
#[allow(dead_code)]
pub fn resolve_hooks_for_event(
    event: &str,
    ws_hooks: &WorkspaceHooks,
    pkg_hooks: &WorkspaceHooks,
) -> Vec<String> {
    let pkg_cmds = match event {
        "post_sync" => &pkg_hooks.post_sync,
        "post_worktree_create" => &pkg_hooks.post_worktree_create,
        "pre_remove" => &pkg_hooks.pre_remove,
        "post_pr" => &pkg_hooks.post_pr,
        _ => return vec![],
    };
    if !pkg_cmds.is_empty() {
        return pkg_cmds.clone();
    }
    match event {
        "post_sync" => ws_hooks.post_sync.clone(),
        "post_worktree_create" => ws_hooks.post_worktree_create.clone(),
        "pre_remove" => ws_hooks.pre_remove.clone(),
        "post_pr" => ws_hooks.post_pr.clone(),
        _ => vec![],
    }
}

/// Run hooks for the given event across all packages in parallel.
/// Each package's hooks run sequentially. Hook failures warn but don't block.
#[allow(dead_code)]
pub fn run_hooks(
    event: &str,
    manifest: &Manifest,
    packages: &[&PackageEntry],
    work_dir_fn: impl Fn(&str) -> PathBuf + Sync,
) {
    packages.par_iter().for_each(|pkg| {
        let cmds = resolve_hooks_for_event(event, &manifest.hooks, &pkg.hooks);
        if cmds.is_empty() {
            return;
        }
        let dir = work_dir_fn(&pkg.name);
        if !dir.exists() {
            return;
        }
        for cmd in &cmds {
            eprintln!("[hook] {}: running '{}' in {}", pkg.name, cmd, dir.display());
            let result = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(&dir)
                .status();
            match result {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    eprintln!(
                        "[hook] WARNING: '{}' in {} exited with {}",
                        cmd, pkg.name, status
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[hook] WARNING: '{}' in {} failed: {}",
                        cmd, pkg.name, e
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hooks(post_sync: Vec<&str>, post_worktree_create: Vec<&str>) -> WorkspaceHooks {
        WorkspaceHooks {
            post_sync: post_sync.into_iter().map(String::from).collect(),
            post_worktree_create: post_worktree_create.into_iter().map(String::from).collect(),
            pre_remove: vec![],
            post_pr: vec![],
        }
    }

    #[test]
    fn test_resolve_hooks_workspace_level() {
        let ws_hooks = make_hooks(vec!["npm install"], vec![]);
        let pkg_hooks = WorkspaceHooks::default();
        let resolved = resolve_hooks_for_event("post_sync", &ws_hooks, &pkg_hooks);
        assert_eq!(resolved, vec!["npm install"]);
    }

    #[test]
    fn test_resolve_hooks_package_override_replaces() {
        let ws_hooks = make_hooks(vec!["npm install", "cargo fetch"], vec![]);
        let pkg_hooks = make_hooks(vec!["cargo build"], vec![]);
        let resolved = resolve_hooks_for_event("post_sync", &ws_hooks, &pkg_hooks);
        assert_eq!(resolved, vec!["cargo build"]);
    }

    #[test]
    fn test_resolve_hooks_no_hooks() {
        let ws_hooks = WorkspaceHooks::default();
        let pkg_hooks = WorkspaceHooks::default();
        let resolved = resolve_hooks_for_event("post_sync", &ws_hooks, &pkg_hooks);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_hooks_unknown_event() {
        let ws_hooks = make_hooks(vec!["npm install"], vec![]);
        let pkg_hooks = WorkspaceHooks::default();
        let resolved = resolve_hooks_for_event("unknown_event", &ws_hooks, &pkg_hooks);
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_resolve_hooks_different_events_independent() {
        let ws_hooks = make_hooks(vec!["npm install"], vec!["mise install"]);
        let pkg_hooks = make_hooks(vec!["cargo build"], vec![]);
        // post_sync uses package override
        assert_eq!(
            resolve_hooks_for_event("post_sync", &ws_hooks, &pkg_hooks),
            vec!["cargo build"]
        );
        // post_worktree_create falls through to workspace (pkg has empty)
        assert_eq!(
            resolve_hooks_for_event("post_worktree_create", &ws_hooks, &pkg_hooks),
            vec!["mise install"]
        );
    }
}
