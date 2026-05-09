use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{MeldrError, Result};

pub const DEFAULT_AGENT: &str = "claude";
pub const DEFAULT_MODE: &str = "full";
pub const DEFAULT_SYNC_METHOD: &str = "rebase";
pub const DEFAULT_SYNC_STRATEGY: &str = "safe";
pub const DEFAULT_EDITOR: &str = "nvim .";
pub const DEFAULT_BRANCH: &str = "main";
pub const DEFAULT_REMOTE: &str = "origin";
pub const DEFAULT_SHELL: &str = "sh";
pub const DEFAULT_LAYOUT: &str = "default";
pub const DEFAULT_WINDOW_NAME: &str = "{ws}/{branch}:{pkg}";

pub(crate) fn default_agent() -> String {
    DEFAULT_AGENT.to_string()
}

pub(crate) fn default_mode() -> String {
    DEFAULT_MODE.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub defaults: GlobalDefaults,
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    #[serde(default)]
    pub layouts: HashMap<String, LayoutDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalDefaults {
    #[serde(default = "default_agent")]
    pub agent: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub root_dir: Option<String>,
    #[serde(default)]
    pub editor: Option<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub layout: Option<String>,
    #[serde(default)]
    pub window_name: Option<String>,
}

impl Default for GlobalDefaults {
    fn default() -> Self {
        Self {
            agent: default_agent(),
            mode: default_mode(),
            root_dir: None,
            editor: None,
            default_branch: None,
            remote: None,
            shell: None,
            layout: None,
            window_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub command: String,
}

/// A custom tmux layout defined as a sequence of tmux commands with template variables.
///
/// Template variables: `{{window}}`, `{{cwd}}`, `{{editor}}`, `{{agent}}`,
/// `{{pkg}}`, `{{branch}}`, `{{ws}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutDef {
    /// Tmux commands to run after window creation (e.g., split-window, select-pane).
    pub setup: Vec<String>,
    /// Pane index where the editor command is sent. If `None`, no editor is launched.
    #[serde(default)]
    pub editor_pane: Option<usize>,
    /// Pane index where the agent command is sent. If `None`, no agent is launched.
    #[serde(default)]
    pub agent_pane: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub agent: String,
    pub mode: String,
    pub agent_command: String,
    pub sync_method: String,
    pub sync_strategy: String,
    pub no_agent: bool,
    pub no_tabs: bool,
    pub editor: String,
    pub default_branch: String,
    pub remote: String,
    pub shell: String,
    pub layout: String,
    pub window_name_template: String,
    pub leader_package: Option<String>,
}

impl Default for EffectiveConfig {
    fn default() -> Self {
        Self {
            agent: DEFAULT_AGENT.to_string(),
            mode: DEFAULT_MODE.to_string(),
            agent_command: DEFAULT_AGENT.to_string(),
            sync_method: DEFAULT_SYNC_METHOD.to_string(),
            sync_strategy: DEFAULT_SYNC_STRATEGY.to_string(),
            no_agent: false,
            no_tabs: false,
            editor: DEFAULT_EDITOR.to_string(),
            default_branch: DEFAULT_BRANCH.to_string(),
            remote: DEFAULT_REMOTE.to_string(),
            shell: DEFAULT_SHELL.to_string(),
            layout: DEFAULT_LAYOUT.to_string(),
            window_name_template: DEFAULT_WINDOW_NAME.to_string(),
            leader_package: None,
        }
    }
}

impl EffectiveConfig {
    pub fn should_launch_agent(&self) -> bool {
        !self.no_agent && self.mode == "full"
    }

    pub fn should_use_tmux(&self) -> bool {
        !self.no_tabs && self.mode != "no-tabs"
    }
}

#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub no_agent: bool,
    pub no_tabs: bool,
}

pub fn global_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".meldr")
}

pub fn global_config_path() -> PathBuf {
    global_config_dir().join("config.toml")
}

/// Ensure the global config directory and default config file exist.
/// Creates `~/.meldr/config.toml` with commented-out defaults if missing.
pub fn ensure_global_config() -> Result<()> {
    let dir = global_config_dir();
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    let path = dir.join("config.toml");
    if !path.exists() {
        let default_content = concat!(
            "# Meldr global configuration\n",
            "# These defaults apply to all workspaces unless overridden.\n",
            "#\n",
            "# [defaults]\n",
            "# agent = \"claude\"\n",
            "# mode = \"full\"\n",
            "# editor = \"nvim .\"\n",
            "# default_branch = \"main\"\n",
            "# remote = \"origin\"\n",
            "# shell = \"sh\"\n",
            "# layout = \"default\"\n",
            "# window_name = \"{ws}/{branch}:{pkg}\"\n",
            "#\n",
            "# Built-in agents and their default commands (override to customise):\n",
            "#\n",
            "# [agents.claude]\n",
            "# command = \"claude --dangerously-skip-permissions\"\n",
            "#\n",
            "# [agents.cursor]\n",
            "# command = \"cursor agent --yolo\"\n",
            "#\n",
            "# [agents.gemini]\n",
            "# command = \"gemini --yolo\"\n",
            "#\n",
            "# [agents.codex]\n",
            "# command = \"codex --approval-mode full-auto\"\n",
            "#\n",
            "# [agents.opencode]\n",
            "# command = \"opencode\"\n",
            "#\n",
            "# [agents.pi]\n",
            "# command = \"pi\"\n",
            "#\n",
            "# [agents.kiro]\n",
            "# command = \"kiro-cli chat --trust-all-tools\"\n",
            "#\n",
            "# [agents.kiro-tui]\n",
            "# command = \"kiro-cli --tui\"\n",
            "#\n",
            "# [agents.deepseek-tui]\n",
            "# command = \"deepseek-tui\"\n",
            "#\n",
            "# [agents.devin]\n",
            "# command = \"devin --permission-mode bypass\"\n",
        );
        std::fs::write(&path, default_content)?;
    }
    Ok(())
}

pub fn load_global_config() -> Result<GlobalConfig> {
    let path = global_config_path();
    if !path.exists() {
        return Ok(GlobalConfig::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let config: GlobalConfig = toml::from_str(&content)?;
    Ok(config)
}

pub fn resolve_config(
    global: &GlobalConfig,
    workspace_settings: &crate::core::workspace::Settings,
    cli: &CliOverrides,
    env_overrides: &HashMap<String, String>,
) -> EffectiveConfig {
    let mut config = EffectiveConfig {
        agent: global.defaults.agent.clone(),
        mode: global.defaults.mode.clone(),
        ..Default::default()
    };
    if let Some(ref v) = global.defaults.editor {
        config.editor = v.clone();
    }
    if let Some(ref v) = global.defaults.default_branch {
        config.default_branch = v.clone();
    }
    if let Some(ref v) = global.defaults.remote {
        config.remote = v.clone();
    }
    if let Some(ref v) = global.defaults.shell {
        config.shell = v.clone();
    }
    if let Some(ref v) = global.defaults.layout {
        config.layout = v.clone();
    }
    if let Some(ref v) = global.defaults.window_name {
        config.window_name_template = v.clone();
    }

    // Layer 3: Workspace settings
    if let Some(ref v) = workspace_settings.agent {
        config.agent = v.clone();
    }
    if let Some(ref v) = workspace_settings.mode {
        config.mode = v.clone();
    }
    if let Some(ref v) = workspace_settings.sync_method {
        config.sync_method = v.clone();
    }
    if let Some(ref v) = workspace_settings.sync_strategy {
        config.sync_strategy = v.clone();
    }
    if let Some(ref v) = workspace_settings.editor {
        config.editor = v.clone();
    }
    if let Some(ref v) = workspace_settings.default_branch {
        config.default_branch = v.clone();
    }
    if let Some(ref v) = workspace_settings.remote {
        config.remote = v.clone();
    }
    if let Some(ref v) = workspace_settings.shell {
        config.shell = v.clone();
    }
    if let Some(ref v) = workspace_settings.layout {
        config.layout = v.clone();
    }
    if let Some(ref v) = workspace_settings.window_name {
        config.window_name_template = v.clone();
    }
    if let Some(ref v) = workspace_settings.leader_package {
        config.leader_package = Some(v.clone());
    }

    // Layer 2: Environment variables
    if let Some(agent) = env_overrides.get("MELDR_AGENT") {
        config.agent = agent.clone();
    }
    if let Some(mode) = env_overrides.get("MELDR_MODE") {
        config.mode = mode.clone();
    }
    if let Some(v) = env_overrides.get("MELDR_EDITOR") {
        config.editor = v.clone();
    } else if let Some(v) = env_overrides.get("VISUAL") {
        config.editor = format!("{v} .");
    } else if let Some(v) = env_overrides.get("EDITOR") {
        config.editor = format!("{v} .");
    }
    if let Some(v) = env_overrides.get("MELDR_DEFAULT_BRANCH") {
        config.default_branch = v.clone();
    }
    if let Some(v) = env_overrides.get("MELDR_REMOTE") {
        config.remote = v.clone();
    }
    if let Some(v) = env_overrides.get("MELDR_SHELL") {
        config.shell = v.clone();
    } else if let Some(v) = env_overrides.get("SHELL") {
        config.shell = v.clone();
    }
    if let Some(v) = env_overrides.get("MELDR_LAYOUT") {
        config.layout = v.clone();
    }
    if let Some(v) = env_overrides.get("MELDR_LEADER_PACKAGE") {
        config.leader_package = Some(v.clone());
    }

    // Layer 1: CLI flags (these are independent — both can be true)
    config.no_agent = cli.no_agent;
    config.no_tabs = cli.no_tabs;

    // Resolve agent command: user config > built-in defaults > agent name
    config.agent_command = global
        .agents
        .get(&config.agent)
        .map(|a| a.command.clone())
        .unwrap_or_else(|| default_agent_command(&config.agent));

    config
}

#[allow(dead_code)]
pub struct AgentDef {
    pub name: &'static str,
    pub command: &'static str,
    pub description: &'static str,
}

pub const BUILTIN_AGENTS: &[AgentDef] = &[
    AgentDef {
        name: "claude",
        command: "claude --dangerously-skip-permissions",
        description: "Anthropic Claude Code",
    },
    AgentDef {
        name: "cursor",
        command: "cursor agent --yolo",
        description: "Cursor AI agent",
    },
    AgentDef {
        name: "gemini",
        command: "gemini --yolo",
        description: "Google Gemini CLI",
    },
    AgentDef {
        name: "codex",
        command: "codex --approval-mode full-auto",
        description: "OpenAI Codex CLI",
    },
    AgentDef {
        name: "opencode",
        command: "opencode",
        description: "OpenCode CLI",
    },
    AgentDef {
        name: "pi",
        command: "pi",
        description: "Pi coding agent",
    },
    AgentDef {
        name: "kiro",
        command: "kiro-cli chat --trust-all-tools",
        description: "AWS Kiro CLI",
    },
    AgentDef {
        name: "kiro-tui",
        command: "kiro-cli --tui",
        description: "AWS Kiro (TUI mode)",
    },
    AgentDef {
        name: "deepseek-tui",
        command: "deepseek-tui",
        description: "DeepSeek TUI",
    },
    AgentDef {
        name: "devin",
        command: "devin --permission-mode bypass",
        description: "Devin for Terminal",
    },
];

pub fn builtin_agent(name: &str) -> Option<&'static AgentDef> {
    BUILTIN_AGENTS.iter().find(|a| a.name == name)
}

#[allow(dead_code)]
pub fn builtin_agent_names() -> impl Iterator<Item = &'static str> {
    BUILTIN_AGENTS.iter().map(|a| a.name)
}

pub fn default_agent_command(agent: &str) -> String {
    builtin_agent(agent)
        .map(|a| a.command.to_string())
        .unwrap_or_else(|| agent.to_string())
}

const VALID_SETTINGS_KEYS: &[&str] = &[
    "agent",
    "mode",
    "sync_method",
    "sync_strategy",
    "editor",
    "default_branch",
    "remote",
    "shell",
    "layout",
    "window_name",
    "leader_package",
];

pub fn config_set(workspace_root: &Path, key: &str, value: &str) -> Result<()> {
    if !VALID_SETTINGS_KEYS.contains(&key) {
        return Err(MeldrError::Config(format!(
            "Unknown setting '{}'. Valid keys: {}",
            key,
            VALID_SETTINGS_KEYS.join(", ")
        )));
    }
    let manifest_path = workspace_root.join("meldr.toml");
    let content = std::fs::read_to_string(&manifest_path)?;
    let mut doc: toml::Table = toml::from_str(&content)?;

    let settings = doc
        .entry("settings")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));

    if let toml::Value::Table(table) = settings {
        table.insert(key.to_string(), toml::Value::String(value.to_string()));
    }

    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| MeldrError::Config(e.to_string()))?;
    std::fs::write(&manifest_path, new_content)?;
    Ok(())
}

pub fn config_get(workspace_root: &Path, key: &str) -> Result<Option<String>> {
    let manifest_path = workspace_root.join("meldr.toml");
    let content = std::fs::read_to_string(&manifest_path)?;
    let doc: toml::Table = toml::from_str(&content)?;

    if let Some(toml::Value::Table(settings)) = doc.get("settings")
        && let Some(toml::Value::String(val)) = settings.get(key)
    {
        return Ok(Some(val.clone()));
    }
    Ok(None)
}

pub fn config_unset(workspace_root: &Path, key: &str) -> Result<()> {
    if !VALID_SETTINGS_KEYS.contains(&key) {
        return Err(MeldrError::Config(format!(
            "Unknown setting '{}'. Valid keys: {}",
            key,
            VALID_SETTINGS_KEYS.join(", ")
        )));
    }
    let manifest_path = workspace_root.join("meldr.toml");
    let content = std::fs::read_to_string(&manifest_path)?;
    let mut doc: toml::Table = toml::from_str(&content)?;

    if let Some(toml::Value::Table(table)) = doc.get_mut("settings") {
        table.remove(key);
    }

    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| MeldrError::Config(e.to_string()))?;
    std::fs::write(&manifest_path, new_content)?;
    Ok(())
}

/// Valid keys for the `[defaults]` section in global config.
const VALID_GLOBAL_KEYS: &[&str] = &[
    "agent",
    "mode",
    "editor",
    "default_branch",
    "remote",
    "shell",
    "layout",
    "window_name",
];

pub fn global_config_set(key: &str, value: &str) -> Result<()> {
    if !VALID_GLOBAL_KEYS.contains(&key) {
        return Err(MeldrError::Config(format!(
            "Unknown setting '{}'. Valid keys: {}",
            key,
            VALID_GLOBAL_KEYS.join(", ")
        )));
    }
    ensure_global_config()?;
    let path = global_config_path();
    let content = std::fs::read_to_string(&path)?;
    let mut doc: toml::Table = toml::from_str(&content)?;

    let defaults = doc
        .entry("defaults")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));

    if let toml::Value::Table(table) = defaults {
        table.insert(key.to_string(), toml::Value::String(value.to_string()));
    }

    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| MeldrError::Config(e.to_string()))?;
    std::fs::write(&path, new_content)?;
    Ok(())
}

pub fn global_config_get(key: &str) -> Result<Option<String>> {
    let path = global_config_path();
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let doc: toml::Table = toml::from_str(&content)?;

    if let Some(toml::Value::Table(defaults)) = doc.get("defaults")
        && let Some(toml::Value::String(val)) = defaults.get(key)
    {
        return Ok(Some(val.clone()));
    }
    Ok(None)
}

pub fn global_config_unset(key: &str) -> Result<()> {
    if !VALID_GLOBAL_KEYS.contains(&key) {
        return Err(MeldrError::Config(format!(
            "Unknown setting '{}'. Valid keys: {}",
            key,
            VALID_GLOBAL_KEYS.join(", ")
        )));
    }
    ensure_global_config()?;
    let path = global_config_path();
    let content = std::fs::read_to_string(&path)?;
    let mut doc: toml::Table = toml::from_str(&content)?;

    if let Some(toml::Value::Table(table)) = doc.get_mut("defaults") {
        table.remove(key);
    }

    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| MeldrError::Config(e.to_string()))?;
    std::fs::write(&path, new_content)?;
    Ok(())
}

/// Collect environment variable overrides relevant to meldr configuration.
pub fn collect_env_overrides() -> HashMap<String, String> {
    let mut env = HashMap::new();
    for key in &[
        "MELDR_AGENT",
        "MELDR_MODE",
        "MELDR_EDITOR",
        "MELDR_DEFAULT_BRANCH",
        "MELDR_REMOTE",
        "MELDR_SHELL",
        "MELDR_LAYOUT",
        "MELDR_LEADER_PACKAGE",
        "VISUAL",
        "EDITOR",
        "SHELL",
    ] {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }
    env
}

pub fn global_config_list() -> Result<GlobalConfig> {
    ensure_global_config()?;
    load_global_config()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::workspace::Settings;

    #[test]
    fn test_default_config() {
        let config = EffectiveConfig::default();
        assert_eq!(config.agent, "claude");
        assert_eq!(config.mode, "full");
        assert_eq!(config.agent_command, "claude");
        assert_eq!(config.sync_method, "rebase");
        assert_eq!(config.sync_strategy, "safe");
        assert!(!config.no_agent);
        assert!(!config.no_tabs);
    }

    #[test]
    fn test_default_agent_commands() {
        assert_eq!(
            default_agent_command("claude"),
            "claude --dangerously-skip-permissions"
        );
        assert_eq!(default_agent_command("cursor"), "cursor agent --yolo");
        assert_eq!(default_agent_command("gemini"), "gemini --yolo");
        assert_eq!(
            default_agent_command("codex"),
            "codex --approval-mode full-auto"
        );
        assert_eq!(default_agent_command("opencode"), "opencode");
        assert_eq!(default_agent_command("pi"), "pi");
        assert_eq!(
            default_agent_command("kiro"),
            "kiro-cli chat --trust-all-tools"
        );
        assert_eq!(default_agent_command("kiro-tui"), "kiro-cli --tui");
        assert_eq!(default_agent_command("deepseek-tui"), "deepseek-tui");
        assert_eq!(
            default_agent_command("devin"),
            "devin --permission-mode bypass"
        );
        assert_eq!(default_agent_command("custom-agent"), "custom-agent");
    }

    #[test]
    fn test_builtin_agent_names() {
        let names: Vec<&str> = builtin_agent_names().collect();
        assert_eq!(names.len(), 10);
        assert!(names.contains(&"claude"));
        assert!(names.contains(&"cursor"));
        assert!(names.contains(&"gemini"));
        assert!(names.contains(&"codex"));
        assert!(names.contains(&"opencode"));
        assert!(names.contains(&"pi"));
        assert!(names.contains(&"kiro"));
        assert!(names.contains(&"kiro-tui"));
        assert!(names.contains(&"deepseek-tui"));
        assert!(names.contains(&"devin"));
    }

    #[test]
    fn test_builtin_agent_lookup() {
        let claude = builtin_agent("claude").unwrap();
        assert_eq!(claude.command, "claude --dangerously-skip-permissions");
        assert_eq!(claude.description, "Anthropic Claude Code");
        assert!(builtin_agent("nonexistent").is_none());
    }

    #[test]
    fn test_kiro_tui_builtin_agent() {
        let agent = builtin_agent("kiro-tui").expect("kiro-tui should be a built-in");
        assert_eq!(agent.name, "kiro-tui");
        assert_eq!(agent.command, "kiro-cli --tui");
        assert_eq!(agent.description, "AWS Kiro (TUI mode)");
    }

    #[test]
    fn test_deepseek_tui_builtin_agent() {
        let agent = builtin_agent("deepseek-tui").expect("deepseek-tui should be a built-in");
        assert_eq!(agent.name, "deepseek-tui");
        assert_eq!(agent.command, "deepseek-tui");
        assert_eq!(agent.description, "DeepSeek TUI");
    }

    #[test]
    fn test_default_kiro_tui_command_resolved() {
        let global = GlobalConfig::default();
        let workspace = Settings {
            agent: Some("kiro-tui".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "kiro-tui");
        assert_eq!(config.agent_command, "kiro-cli --tui");
    }

    #[test]
    fn test_default_deepseek_tui_command_resolved() {
        let global = GlobalConfig::default();
        let workspace = Settings {
            agent: Some("deepseek-tui".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "deepseek-tui");
        assert_eq!(config.agent_command, "deepseek-tui");
    }

    #[test]
    fn test_devin_builtin_agent() {
        let agent = builtin_agent("devin").expect("devin should be a built-in");
        assert_eq!(agent.name, "devin");
        assert_eq!(agent.command, "devin --permission-mode bypass");
        assert_eq!(agent.description, "Devin for Terminal");
    }

    #[test]
    fn test_default_devin_command_resolved() {
        let global = GlobalConfig::default();
        let workspace = Settings {
            agent: Some("devin".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "devin");
        assert_eq!(config.agent_command, "devin --permission-mode bypass");
    }

    #[test]
    fn test_devin_user_override_wins_over_builtin() {
        let mut agents = HashMap::new();
        agents.insert(
            "devin".to_string(),
            AgentConfig {
                command: "devin --permission-mode bypass --custom-flag".to_string(),
            },
        );
        let global = GlobalConfig {
            agents,
            ..Default::default()
        };
        let workspace = Settings {
            agent: Some("devin".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "devin");
        assert_eq!(
            config.agent_command,
            "devin --permission-mode bypass --custom-flag"
        );
    }

    #[test]
    fn test_kiro_tui_user_override_wins_over_builtin() {
        let mut agents = HashMap::new();
        agents.insert(
            "kiro-tui".to_string(),
            AgentConfig {
                command: "kiro-cli --tui --custom-flag".to_string(),
            },
        );
        let global = GlobalConfig {
            agents,
            ..Default::default()
        };
        let workspace = Settings {
            agent: Some("kiro-tui".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "kiro-tui");
        assert_eq!(config.agent_command, "kiro-cli --tui --custom-flag");
    }

    #[test]
    fn test_unknown_agent_falls_back_to_name() {
        assert_eq!(default_agent_command("my-custom"), "my-custom");
    }

    #[test]
    fn test_config_precedence() {
        let global = GlobalConfig {
            defaults: GlobalDefaults {
                agent: "cursor".to_string(),
                mode: "full".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let workspace = Settings {
            agent: Some("claude".into()),
            mode: Some("no-tabs".into()),
            ..Default::default()
        };

        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "claude");
        assert_eq!(config.mode, "no-tabs");
    }

    #[test]
    fn test_cli_overrides_are_independent() {
        let global = GlobalConfig::default();
        let workspace = Settings::default();

        let cli = CliOverrides {
            no_agent: true,
            no_tabs: true,
        };
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert!(config.no_agent);
        assert!(config.no_tabs);
        assert!(!config.should_launch_agent());
        assert!(!config.should_use_tmux());
    }

    #[test]
    fn test_no_agent_alone() {
        let global = GlobalConfig::default();
        let workspace = Settings::default();

        let cli = CliOverrides {
            no_agent: true,
            no_tabs: false,
        };
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert!(!config.should_launch_agent());
        assert!(config.should_use_tmux());
    }

    #[test]
    fn test_env_overrides_workspace() {
        let global = GlobalConfig::default();
        let workspace = Settings {
            agent: Some("cursor".into()),
            ..Default::default()
        };

        let cli = CliOverrides::default();
        let mut env = HashMap::new();
        env.insert("MELDR_AGENT".to_string(), "none".to_string());

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "none");
    }

    #[test]
    fn test_default_claude_command_resolved() {
        let global = GlobalConfig::default(); // no [agents.claude] entry
        let workspace = Settings::default();
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "claude");
        assert_eq!(
            config.agent_command,
            "claude --dangerously-skip-permissions"
        );
    }

    #[test]
    fn test_default_cursor_command_resolved() {
        let global = GlobalConfig::default();
        let workspace = Settings {
            agent: Some("cursor".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "cursor");
        assert_eq!(config.agent_command, "cursor agent --yolo");
    }

    #[test]
    fn test_agent_command_resolution() {
        let mut agents = HashMap::new();
        agents.insert(
            "cursor".to_string(),
            AgentConfig {
                command: "cursor .".to_string(),
            },
        );
        let global = GlobalConfig {
            defaults: GlobalDefaults {
                agent: "cursor".to_string(),
                ..Default::default()
            },
            agents,
            ..Default::default()
        };

        let workspace = Settings::default();
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        // Explicit agent config overrides the built-in default
        assert_eq!(config.agent_command, "cursor .");
    }

    #[test]
    fn test_new_config_defaults() {
        let config = EffectiveConfig::default();
        assert_eq!(config.editor, "nvim .");
        assert_eq!(config.default_branch, "main");
        assert_eq!(config.remote, "origin");
        assert_eq!(config.shell, "sh");
        assert_eq!(config.layout, "default");
        assert_eq!(config.window_name_template, "{ws}/{branch}:{pkg}");
    }

    #[test]
    fn test_editor_from_env_visual() {
        let global = GlobalConfig::default();
        let workspace = Settings::default();
        let cli = CliOverrides::default();
        let mut env = HashMap::new();
        env.insert("VISUAL".to_string(), "code".to_string());

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.editor, "code .");
    }

    #[test]
    fn test_meldr_editor_overrides_visual() {
        let global = GlobalConfig::default();
        let workspace = Settings::default();
        let cli = CliOverrides::default();
        let mut env = HashMap::new();
        env.insert("VISUAL".to_string(), "code".to_string());
        env.insert("MELDR_EDITOR".to_string(), "hx .".to_string());

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.editor, "hx .");
    }

    #[test]
    fn test_shell_from_env() {
        let global = GlobalConfig::default();
        let workspace = Settings::default();
        let cli = CliOverrides::default();
        let mut env = HashMap::new();
        env.insert("SHELL".to_string(), "/bin/zsh".to_string());

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.shell, "/bin/zsh");
    }

    #[test]
    fn test_workspace_settings_override_global() {
        let global = GlobalConfig {
            defaults: GlobalDefaults {
                editor: Some("code .".to_string()),
                layout: Some("minimal".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let workspace = Settings {
            editor: Some("hx .".to_string()),
            ..Default::default()
        };

        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.editor, "hx .");
        assert_eq!(config.layout, "minimal");
    }

    #[test]
    fn test_valid_settings_keys_expanded() {
        // Verify all expected keys are present
        assert!(VALID_SETTINGS_KEYS.contains(&"editor"));
        assert!(VALID_SETTINGS_KEYS.contains(&"default_branch"));
        assert!(VALID_SETTINGS_KEYS.contains(&"remote"));
        assert!(VALID_SETTINGS_KEYS.contains(&"shell"));
        assert!(VALID_SETTINGS_KEYS.contains(&"layout"));
        assert!(VALID_SETTINGS_KEYS.contains(&"window_name"));

        // Also verify the core keys are present
        assert!(VALID_SETTINGS_KEYS.contains(&"agent"));
        assert!(VALID_SETTINGS_KEYS.contains(&"mode"));
        assert!(VALID_SETTINGS_KEYS.contains(&"sync_method"));
        assert!(VALID_SETTINGS_KEYS.contains(&"sync_strategy"));

        // Verify the total count matches expectations (no accidental duplicates or missing keys)
        assert_eq!(
            VALID_SETTINGS_KEYS.len(),
            11,
            "Expected exactly 11 valid settings keys"
        );

        // Verify that invalid keys are not accepted by config_set
        let tmp = tempfile::TempDir::new().unwrap();
        let manifest = "[workspace]\nname = \"test\"\n";
        std::fs::write(tmp.path().join("meldr.toml"), manifest).unwrap();
        let result = config_set(tmp.path(), "nonexistent_key", "value");
        assert!(result.is_err(), "config_set should reject unknown keys");
    }

    #[test]
    fn test_valid_global_keys() {
        // Global keys should be a subset of settings keys (minus sync_method, sync_strategy)
        for key in VALID_GLOBAL_KEYS {
            assert!(
                VALID_SETTINGS_KEYS.contains(key),
                "Global key '{key}' should also be a valid settings key"
            );
        }
        assert!(!VALID_GLOBAL_KEYS.contains(&"sync_method"));
        assert!(!VALID_GLOBAL_KEYS.contains(&"sync_strategy"));
    }

    #[test]
    fn test_config_set_and_unset() {
        let tmp = tempfile::TempDir::new().unwrap();
        let manifest = r#"
[workspace]
name = "test"

[settings]
agent = "cursor"
"#;
        std::fs::write(tmp.path().join("meldr.toml"), manifest).unwrap();

        // Verify initial value via API
        assert_eq!(
            config_get(tmp.path(), "agent").unwrap(),
            Some("cursor".to_string())
        );

        // Set a new value
        config_set(tmp.path(), "editor", "code .").unwrap();
        assert_eq!(
            config_get(tmp.path(), "editor").unwrap(),
            Some("code .".to_string())
        );

        // Verify the TOML file on disk contains the set values
        let on_disk = std::fs::read_to_string(tmp.path().join("meldr.toml")).unwrap();
        let doc: toml::Table = toml::from_str(&on_disk).unwrap();
        let settings = doc["settings"].as_table().unwrap();
        assert_eq!(settings["agent"].as_str().unwrap(), "cursor");
        assert_eq!(settings["editor"].as_str().unwrap(), "code .");

        // Unset agent
        config_unset(tmp.path(), "agent").unwrap();
        assert_eq!(config_get(tmp.path(), "agent").unwrap(), None);

        // Verify disk contents after unset — agent key should be gone
        let on_disk_after = std::fs::read_to_string(tmp.path().join("meldr.toml")).unwrap();
        let doc_after: toml::Table = toml::from_str(&on_disk_after).unwrap();
        let settings_after = doc_after["settings"].as_table().unwrap();
        assert!(
            !settings_after.contains_key("agent"),
            "agent key should be removed from disk"
        );
        // editor should still be present
        assert_eq!(settings_after["editor"].as_str().unwrap(), "code .");
        // workspace name should be preserved
        assert_eq!(doc_after["workspace"]["name"].as_str().unwrap(), "test");
    }

    #[test]
    fn test_leader_package_from_workspace_settings() {
        let global = GlobalConfig::default();
        let workspace = Settings {
            leader_package: Some("frontend".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.leader_package.as_deref(), Some("frontend"));
    }

    #[test]
    fn test_leader_package_env_overrides_workspace() {
        let global = GlobalConfig::default();
        let workspace = Settings {
            leader_package: Some("frontend".into()),
            ..Default::default()
        };
        let cli = CliOverrides::default();
        let mut env = HashMap::new();
        env.insert("MELDR_LEADER_PACKAGE".to_string(), "backend".to_string());

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.leader_package.as_deref(), Some("backend"));
    }

    #[test]
    fn test_leader_package_defaults_to_none() {
        let global = GlobalConfig::default();
        let workspace = Settings::default();
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert!(config.leader_package.is_none());
    }

    #[test]
    fn test_leader_package_settings_key_is_valid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let manifest = "[workspace]\nname = \"test\"\n";
        std::fs::write(tmp.path().join("meldr.toml"), manifest).unwrap();

        config_set(tmp.path(), "leader_package", "frontend").unwrap();
        assert_eq!(
            config_get(tmp.path(), "leader_package").unwrap(),
            Some("frontend".to_string())
        );
    }

    #[test]
    fn test_leader_package_roundtrips_through_manifest() {
        let input = r#"
[workspace]
name = "ws"

[settings]
leader_package = "api"
"#;
        let manifest: crate::core::workspace::Manifest = toml::from_str(input).unwrap();
        assert_eq!(manifest.settings.leader_package.as_deref(), Some("api"));
    }

    #[test]
    fn test_config_unset_invalid_key() {
        let tmp = tempfile::TempDir::new().unwrap();
        let manifest = "[workspace]\nname = \"test\"\n";
        std::fs::write(tmp.path().join("meldr.toml"), manifest).unwrap();

        let result = config_unset(tmp.path(), "bogus");
        assert!(result.is_err());
    }
}
