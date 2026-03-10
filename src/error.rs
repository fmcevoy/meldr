use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum MeldrError {
    #[error("Not a meldr workspace: no meldr.toml found in {0}")]
    NotAWorkspace(PathBuf),

    #[error("Workspace already initialized at {0}")]
    AlreadyInitialized(PathBuf),

    #[error("Package '{0}' not found in workspace")]
    PackageNotFound(String),

    #[error("Package '{0}' already exists in workspace")]
    PackageAlreadyExists(String),

    #[error("Worktree '{0}' already exists")]
    WorktreeAlreadyExists(String),

    #[error("Worktree '{0}' not found")]
    WorktreeNotFound(String),

    #[error("Worktree '{0}' has uncommitted changes in {1}. Use --force to override.")]
    DirtyWorktree(String, String),

    #[error("Not inside a tmux session. Use --no-tabs to skip tmux integration.")]
    NotInTmux,

    #[error("Git error: {0}")]
    Git(String),

    #[error("Tmux error: {0}")]
    Tmux(String),

    #[error("Clone failed for {url}: {reason}")]
    CloneFailed { url: String, reason: String },

    #[error("Config error: {0}")]
    Config(String),

    #[error("No sync snapshot found for branch '{0}'")]
    NoSyncSnapshot(String),

    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    TomlDeserialize(#[from] toml::de::Error),

    #[error(transparent)]
    TomlSerialize(#[from] toml::ser::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, MeldrError>;
