pub mod config_cmd;
pub mod create;
pub mod exec;
pub mod init;
pub mod package;
pub mod prompt_check;
pub mod status;
pub mod sync;
pub mod worktree;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "meldr",
    version,
    about = "Workspace management for multi-repo projects with git worktrees and tmux"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Skip launching AI agents in tmux panes
    #[arg(long, global = true)]
    pub no_agent: bool,

    /// Skip tmux window/pane creation entirely
    #[arg(long, global = true)]
    pub no_tabs: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new meldr workspace in the current directory
    Init {
        /// Workspace name (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Create a workspace: init + add packages + create worktree in one shot
    Create {
        /// Name for the new workspace directory
        name: String,
        /// Git repository URLs to add as packages
        #[arg(short = 'r', long = "repo")]
        repos: Vec<String>,
        /// Create a worktree on this branch after adding packages
        #[arg(short, long)]
        branch: Option<String>,
        /// Override the default AI agent
        #[arg(short, long)]
        agent: Option<String>,
    },

    /// Manage packages (repositories) in the workspace
    #[command(alias = "pkg")]
    Package {
        #[command(subcommand)]
        action: PackageAction,
    },

    /// Manage git worktrees across all packages
    #[command(alias = "wt")]
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },

    /// Show workspace status dashboard
    #[command(alias = "st")]
    Status,

    /// Run a command in every package's worktree directory (must be run from within a worktree)
    Exec {
        /// Launch an interactive shell so aliases and rc files are loaded
        #[arg(short, long)]
        interactive: bool,

        /// Command and arguments to execute
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },

    /// Sync worktree branches with upstream (fetch + rebase/merge)
    Sync {
        /// Branch to sync (auto-detected from cwd if omitted)
        branch: Option<String>,
        /// Sync all active worktrees
        #[arg(long)]
        all: bool,
        /// Override merge strategy (safe, theirs, ours, manual)
        #[arg(long)]
        strategy: Option<String>,
        /// Use merge instead of rebase
        #[arg(long)]
        merge: bool,
        /// Preview what sync would do without making changes
        #[arg(long)]
        dry_run: bool,
        /// Only sync these packages (comma-separated)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Exclude these packages from sync (comma-separated)
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<String>,
        /// Undo the last sync (reset to pre-sync state)
        #[arg(long)]
        undo: bool,
    },

    /// Check if current worktree branch matches expected branch (for shell prompts)
    PromptCheck,

    /// View or modify workspace configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
pub enum PackageAction {
    /// Clone and register new packages from git URLs
    Add {
        /// Git repository URLs
        #[arg(required = true)]
        urls: Vec<String>,
    },
    /// Remove packages from workspace
    Remove {
        /// Package names to remove
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// List all registered packages
    List,
}

#[derive(Subcommand)]
pub enum WorktreeAction {
    /// Create worktrees for a feature branch across all packages
    Add {
        /// Branch name for the worktrees
        branch: String,
    },
    /// Remove worktrees for a branch (checks for dirty state). Auto-detects branch when run from within a worktree directory.
    Remove {
        /// Branch name to remove (auto-detected from cwd if omitted)
        branch: Option<String>,
        /// Force removal even with uncommitted changes
        #[arg(long)]
        force: bool,
    },
    /// Reopen tmux windows for an existing worktree (e.g. after a crash)
    Open {
        /// Branch name of the worktree to open
        branch: String,
    },
    /// List all active worktrees
    List,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Set a configuration value
    Set {
        /// Setting key (agent, mode, sync_method, sync_strategy, editor, default_branch, remote, shell, layout, window_name)
        key: String,
        /// Setting value
        value: String,
        /// Apply to global config (~/.meldr/config.toml) instead of workspace
        #[arg(long)]
        global: bool,
    },
    /// Get a configuration value
    Get {
        /// Setting key to read
        key: String,
        /// Read from global config (~/.meldr/config.toml) instead of workspace
        #[arg(long)]
        global: bool,
    },
    /// Remove a configuration value
    Unset {
        /// Setting key to remove
        key: String,
        /// Remove from global config (~/.meldr/config.toml) instead of workspace
        #[arg(long)]
        global: bool,
    },
    /// Show effective configuration from all layers
    List {
        /// Show only global config (~/.meldr/config.toml)
        #[arg(long)]
        global: bool,
    },
    /// Show where each setting value comes from
    Show,
}
