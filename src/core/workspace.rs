use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{MeldrError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub workspace: WorkspaceInfo,
    #[serde(default, skip_serializing_if = "Settings::is_empty")]
    pub settings: Settings,
    #[serde(default)]
    pub layout: Option<LayoutOverride>,
    #[serde(default, rename = "package", skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<PackageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sync_method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sync_strategy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_name: Option<String>,
}

impl Settings {
    pub fn is_empty(&self) -> bool {
        self.agent.is_empty()
            && self.mode.is_empty()
            && self.sync_method.is_empty()
            && self.sync_strategy.is_empty()
            && self.editor.is_none()
            && self.default_branch.is_none()
            && self.remote.is_none()
            && self.shell.is_none()
            && self.layout.is_none()
            && self.window_name.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutOverride {
    pub definition: String,
    pub panes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl Manifest {
    pub fn new(name: &str) -> Self {
        Self {
            workspace: WorkspaceInfo {
                name: name.to_string(),
            },
            settings: Settings::default(),
            layout: None,
            packages: Vec::new(),
        }
    }

    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = workspace_root.join("meldr.toml");
        if !path.exists() {
            return Err(MeldrError::NotAWorkspace(workspace_root.to_path_buf()));
        }
        let content = std::fs::read_to_string(&path)?;
        let manifest: Manifest = toml::from_str(&content)?;
        Ok(manifest)
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let path = workspace_root.join("meldr.toml");
        let content =
            toml::to_string_pretty(self).map_err(|e| MeldrError::InvalidManifest(e.to_string()))?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn save_initial(&self, workspace_root: &Path) -> Result<()> {
        let path = workspace_root.join("meldr.toml");
        let serialized =
            toml::to_string_pretty(self).map_err(|e| MeldrError::InvalidManifest(e.to_string()))?;

        let defaults_comment = concat!(
            "\n# Uncomment to override defaults:\n",
            "# agent = \"claude\"          # \"claude\" | \"cursor\" | \"none\"\n",
            "# mode = \"full\"             # \"full\" | \"no-agent\" | \"no-tabs\"\n",
            "# sync_method = \"rebase\"    # \"rebase\" | \"merge\"\n",
            "# sync_strategy = \"theirs\"  # \"theirs\" | \"ours\" | \"manual\"\n",
            "# editor = \"nvim .\"         # editor command (or uses $EDITOR/$VISUAL)\n",
            "# default_branch = \"main\"   # fallback branch for sync\n",
            "# remote = \"origin\"         # default git remote\n",
            "# shell = \"sh\"              # shell for exec (or uses $SHELL)\n",
            "# layout = \"default\"        # \"default\" | \"minimal\" | \"editor-only\"\n",
            "# window_name = \"{ws}/{branch}:{pkg}\"  # tmux window name template\n",
        );

        // Insert defaults comment right after [settings] block (or after [workspace] if no settings)
        let content = if let Some(pos) = serialized.find("\n[[package]]") {
            let mut result = String::with_capacity(serialized.len() + defaults_comment.len());
            result.push_str(&serialized[..pos]);
            result.push_str(defaults_comment);
            result.push_str(&serialized[pos..]);
            result
        } else {
            let mut result = serialized;
            result.push_str(defaults_comment);
            result
        };

        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn find_package(&self, name: &str) -> Option<&PackageEntry> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn add_package(&mut self, entry: PackageEntry) -> Result<()> {
        if self.find_package(&entry.name).is_some() {
            return Err(MeldrError::PackageAlreadyExists(entry.name));
        }
        self.packages.push(entry);
        Ok(())
    }

    pub fn remove_package(&mut self, name: &str) -> Result<PackageEntry> {
        let idx = self
            .packages
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| MeldrError::PackageNotFound(name.to_string()))?;
        Ok(self.packages.remove(idx))
    }
}

pub fn derive_package_name(url: &str) -> String {
    let url = url.trim_end_matches('/');
    let name = url
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git");
    name.to_string()
}

pub fn packages_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("packages")
}

pub fn worktrees_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("worktrees")
}

pub fn package_path(workspace_root: &Path, name: &str) -> PathBuf {
    packages_dir(workspace_root).join(name)
}

/// Sanitize a branch name for use as a filesystem directory name.
///
/// Replaces `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`, and spaces with `-`.
/// Collapses consecutive `-` into a single `-` and trims leading/trailing `-`.
pub fn sanitize_branch_for_dir(branch: &str) -> String {
    let sanitized: String = branch
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '-',
            _ => c,
        })
        .collect();
    // Collapse consecutive dashes and trim
    let mut result = String::with_capacity(sanitized.len());
    let mut prev_dash = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_dash {
                result.push('-');
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    result.trim_matches('-').to_string()
}

pub fn worktree_path(workspace_root: &Path, branch: &str, package: &str) -> PathBuf {
    worktrees_dir(workspace_root)
        .join(sanitize_branch_for_dir(branch))
        .join(package)
}

/// Return the sanitized directory name for a branch under `worktrees/`.
pub fn worktree_branch_dir(workspace_root: &Path, branch: &str) -> PathBuf {
    worktrees_dir(workspace_root).join(sanitize_branch_for_dir(branch))
}

pub fn find_workspace_root(start: &Path) -> Result<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("meldr.toml").exists() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(MeldrError::NotAWorkspace(start.to_path_buf()));
        }
    }
}

/// Detect the current worktree by matching the cwd against the `worktrees/` directory.
///
/// Returns the sanitized directory name (e.g. `fm-whatever` for branch `fm/whatever`).
/// Callers should use [`resolve_branch_from_dir`] to map back to the real branch name.
pub fn detect_current_worktree_dir(workspace_root: &Path, cwd: &Path) -> Option<String> {
    let worktrees = worktrees_dir(workspace_root);
    if let Ok(stripped) = cwd.strip_prefix(&worktrees) {
        stripped
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
    } else {
        None
    }
}

/// Given a sanitized directory name (from `detect_current_worktree_dir`), find the
/// actual branch name by comparing against known branches.
pub fn resolve_branch_from_dir<'a>(dir_name: &str, branches: impl Iterator<Item = &'a str>) -> Option<String> {
    branches
        .into_iter()
        .find(|b| sanitize_branch_for_dir(b) == dir_name)
        .map(|b| b.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_package_name() {
        assert_eq!(
            derive_package_name("https://github.com/org/frontend.git"),
            "frontend"
        );
        assert_eq!(
            derive_package_name("https://github.com/org/backend"),
            "backend"
        );
        assert_eq!(
            derive_package_name("git@github.com:org/shared-lib.git"),
            "shared-lib"
        );
        assert_eq!(
            derive_package_name("https://github.com/org/repo.git/"),
            "repo"
        );
    }

    #[test]
    fn test_manifest_roundtrip() {
        let mut manifest = Manifest::new("test-project");
        manifest
            .add_package(PackageEntry {
                name: "frontend".to_string(),
                url: "https://github.com/org/frontend.git".to_string(),
                branch: Some("main".to_string()),
                remote: None,
            })
            .unwrap();

        let serialized = toml::to_string_pretty(&manifest).unwrap();
        let deserialized: Manifest = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.workspace.name, "test-project");
        assert_eq!(deserialized.packages.len(), 1);
        assert_eq!(deserialized.packages[0].name, "frontend");
    }

    #[test]
    fn test_manifest_parse_full() {
        let input = r#"
[workspace]
name = "my-project"

[settings]
agent = "claude"
mode = "full"

[[package]]
name = "frontend"
url = "https://github.com/org/frontend.git"
branch = "main"

[[package]]
name = "backend"
url = "https://github.com/org/backend.git"
"#;
        let manifest: Manifest = toml::from_str(input).unwrap();
        assert_eq!(manifest.workspace.name, "my-project");
        assert_eq!(manifest.packages.len(), 2);
        assert_eq!(manifest.packages[0].branch, Some("main".to_string()));
        assert_eq!(manifest.packages[1].branch, None);
    }

    #[test]
    fn test_duplicate_package() {
        let mut manifest = Manifest::new("test");
        manifest
            .add_package(PackageEntry {
                name: "pkg".to_string(),
                url: "url".to_string(),
                branch: None,
                remote: None,
            })
            .unwrap();

        let result = manifest.add_package(PackageEntry {
            name: "pkg".to_string(),
            url: "url2".to_string(),
            branch: None,
            remote: None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_package() {
        let mut manifest = Manifest::new("test");
        manifest
            .add_package(PackageEntry {
                name: "pkg".to_string(),
                url: "url".to_string(),
                branch: None,
                remote: None,
            })
            .unwrap();

        let removed = manifest.remove_package("pkg").unwrap();
        assert_eq!(removed.name, "pkg");
        assert!(manifest.packages.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_package() {
        let mut manifest = Manifest::new("test");
        assert!(manifest.remove_package("nope").is_err());
    }

    #[test]
    fn test_path_resolution() {
        let root = Path::new("/workspace");
        assert_eq!(
            package_path(root, "frontend"),
            PathBuf::from("/workspace/packages/frontend")
        );
        assert_eq!(
            worktree_path(root, "feature-x", "frontend"),
            PathBuf::from("/workspace/worktrees/feature-x/frontend")
        );
    }

    #[test]
    fn test_path_resolution_with_slashes() {
        let root = Path::new("/workspace");
        assert_eq!(
            worktree_path(root, "fm/whatever", "frontend"),
            PathBuf::from("/workspace/worktrees/fm-whatever/frontend")
        );
        assert_eq!(
            worktree_path(root, "fm/deep/branch", "backend"),
            PathBuf::from("/workspace/worktrees/fm-deep-branch/backend")
        );
    }

    #[test]
    fn test_sanitize_branch_for_dir() {
        assert_eq!(sanitize_branch_for_dir("fm/whatever"), "fm-whatever");
        assert_eq!(sanitize_branch_for_dir("feature-x"), "feature-x");
        assert_eq!(sanitize_branch_for_dir("a/b/c"), "a-b-c");
        assert_eq!(sanitize_branch_for_dir("branch:name"), "branch-name");
        assert_eq!(sanitize_branch_for_dir("has spaces"), "has-spaces");
        assert_eq!(sanitize_branch_for_dir("a//b"), "a-b");
        assert_eq!(sanitize_branch_for_dir("/leading"), "leading");
        assert_eq!(sanitize_branch_for_dir("trailing/"), "trailing");
        assert_eq!(sanitize_branch_for_dir("normal-branch"), "normal-branch");
    }

    #[test]
    fn test_worktree_branch_dir() {
        let root = Path::new("/workspace");
        assert_eq!(
            worktree_branch_dir(root, "fm/whatever"),
            PathBuf::from("/workspace/worktrees/fm-whatever")
        );
    }

    #[test]
    fn test_detect_current_worktree_dir() {
        let root = Path::new("/workspace");
        let cwd = Path::new("/workspace/worktrees/feature-auth/frontend");
        assert_eq!(
            detect_current_worktree_dir(root, cwd),
            Some("feature-auth".to_string())
        );

        let cwd_packages = Path::new("/workspace/packages/frontend");
        assert_eq!(detect_current_worktree_dir(root, cwd_packages), None);
    }

    #[test]
    fn test_detect_current_worktree_dir_sanitized() {
        let root = Path::new("/workspace");
        // After sanitization, fm/whatever becomes fm-whatever on disk
        let cwd = Path::new("/workspace/worktrees/fm-whatever/frontend");
        assert_eq!(detect_current_worktree_dir(root, cwd), Some("fm-whatever".to_string()));
    }

    #[test]
    fn test_resolve_branch_from_dir() {
        let branches = vec!["fm/whatever", "feature-x", "main"];
        assert_eq!(
            resolve_branch_from_dir("fm-whatever", branches.iter().copied()),
            Some("fm/whatever".to_string())
        );
        assert_eq!(
            resolve_branch_from_dir("feature-x", branches.iter().copied()),
            Some("feature-x".to_string())
        );
        assert_eq!(
            resolve_branch_from_dir("nonexistent", branches.iter().copied()),
            None
        );
    }

    #[test]
    fn test_layout_override_parse() {
        let input = r#"
[workspace]
name = "test"

[layout]
definition = "1bc3,168x45,0,0{112x45,0,0,55x45,113,0}"
panes = ["frontend", "backend"]
"#;
        let manifest: Manifest = toml::from_str(input).unwrap();
        let layout = manifest.layout.unwrap();
        assert_eq!(layout.panes.len(), 2);
    }
}
