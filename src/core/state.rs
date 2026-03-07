use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkspaceState {
    #[serde(default)]
    pub worktrees: HashMap<String, WorktreeState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeState {
    pub branch: String,
    pub tmux_window: Option<String>,
    pub pane_mappings: HashMap<String, String>,
}

impl WorkspaceState {
    fn state_path(workspace_root: &Path) -> PathBuf {
        workspace_root.join(".meldr").join("state.json")
    }

    pub fn load(workspace_root: &Path) -> Result<Self> {
        let path = Self::state_path(workspace_root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let state: Self = serde_json::from_str(&content)?;
        Ok(state)
    }

    pub fn save(&self, workspace_root: &Path) -> Result<()> {
        let dir = workspace_root.join(".meldr");
        std::fs::create_dir_all(&dir)?;
        let path = Self::state_path(workspace_root);
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn add_worktree(&mut self, branch: &str, state: WorktreeState) {
        self.worktrees.insert(branch.to_string(), state);
    }

    pub fn remove_worktree(&mut self, branch: &str) -> Option<WorktreeState> {
        self.worktrees.remove(branch)
    }

    pub fn get_worktree(&self, branch: &str) -> Option<&WorktreeState> {
        self.worktrees.get(branch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_roundtrip() {
        let mut state = WorkspaceState::default();
        let mut pane_mappings = HashMap::new();
        pane_mappings.insert("0".to_string(), "frontend".to_string());
        pane_mappings.insert("1".to_string(), "backend".to_string());

        state.add_worktree(
            "feature-auth",
            WorktreeState {
                branch: "feature-auth".to_string(),
                tmux_window: Some("@5".to_string()),
                pane_mappings,
            },
        );

        let json = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: WorkspaceState = serde_json::from_str(&json).unwrap();

        let wt = deserialized.get_worktree("feature-auth").unwrap();
        assert_eq!(wt.tmux_window, Some("@5".to_string()));
        assert_eq!(wt.pane_mappings.len(), 2);
    }

    #[test]
    fn test_remove_worktree() {
        let mut state = WorkspaceState::default();
        state.add_worktree(
            "branch",
            WorktreeState {
                branch: "branch".to_string(),
                tmux_window: None,
                pane_mappings: HashMap::new(),
            },
        );
        assert!(state.remove_worktree("branch").is_some());
        assert!(state.get_worktree("branch").is_none());
    }
}
