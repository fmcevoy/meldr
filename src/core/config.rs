use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{MeldrError, Result};

pub const DEFAULT_AGENT: &str = "claude";
pub const DEFAULT_MODE: &str = "full";
pub const DEFAULT_SYNC_METHOD: &str = "rebase";
pub const DEFAULT_SYNC_STRATEGY: &str = "theirs";

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalDefaults {
    #[serde(default = "default_agent")]
    pub agent: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub root_dir: Option<String>,
}

impl Default for GlobalDefaults {
    fn default() -> Self {
        Self {
            agent: default_agent(),
            mode: default_mode(),
            root_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub command: String,
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

pub fn global_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("meldr")
        .join("config.toml")
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
    let mut config = EffectiveConfig::default();

    // Layer 4: Global config
    config.agent = global.defaults.agent.clone();
    config.mode = global.defaults.mode.clone();

    // Layer 3: Workspace settings
    if !workspace_settings.agent.is_empty() {
        config.agent = workspace_settings.agent.clone();
    }
    if !workspace_settings.mode.is_empty() {
        config.mode = workspace_settings.mode.clone();
    }
    if !workspace_settings.sync_method.is_empty() {
        config.sync_method = workspace_settings.sync_method.clone();
    }
    if !workspace_settings.sync_strategy.is_empty() {
        config.sync_strategy = workspace_settings.sync_strategy.clone();
    }

    // Layer 2: Environment variables
    if let Some(agent) = env_overrides.get("MELDR_AGENT") {
        config.agent = agent.clone();
    }
    if let Some(mode) = env_overrides.get("MELDR_MODE") {
        config.mode = mode.clone();
    }

    // Layer 1: CLI flags (these are independent — both can be true)
    config.no_agent = cli.no_agent;
    config.no_tabs = cli.no_tabs;

    // Resolve agent command
    config.agent_command = global
        .agents
        .get(&config.agent)
        .map(|a| a.command.clone())
        .unwrap_or_else(|| config.agent.clone());

    config
}

const VALID_SETTINGS_KEYS: &[&str] = &["agent", "mode", "sync_method", "sync_strategy"];

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

    let new_content = toml::to_string_pretty(&doc).map_err(|e| MeldrError::Config(e.to_string()))?;
    std::fs::write(&manifest_path, new_content)?;
    Ok(())
}

pub fn config_get(workspace_root: &Path, key: &str) -> Result<Option<String>> {
    let manifest_path = workspace_root.join("meldr.toml");
    let content = std::fs::read_to_string(&manifest_path)?;
    let doc: toml::Table = toml::from_str(&content)?;

    if let Some(toml::Value::Table(settings)) = doc.get("settings") {
        if let Some(toml::Value::String(val)) = settings.get(key) {
            return Ok(Some(val.clone()));
        }
    }
    Ok(None)
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
        assert_eq!(config.sync_method, "rebase");
        assert_eq!(config.sync_strategy, "theirs");
        assert!(!config.no_agent);
        assert!(!config.no_tabs);
    }

    #[test]
    fn test_config_precedence() {
        let global = GlobalConfig {
            defaults: GlobalDefaults {
                agent: "cursor".to_string(),
                mode: "full".to_string(),
                root_dir: None,
            },
            agents: HashMap::new(),
        };

        let workspace = Settings {
            agent: "claude".to_string(),
            mode: "no-tabs".to_string(),
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
            agent: "cursor".to_string(),
            ..Default::default()
        };

        let cli = CliOverrides::default();
        let mut env = HashMap::new();
        env.insert("MELDR_AGENT".to_string(), "none".to_string());

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent, "none");
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
        };

        let workspace = Settings::default();
        let cli = CliOverrides::default();
        let env = HashMap::new();

        let config = resolve_config(&global, &workspace, &cli, &env);
        assert_eq!(config.agent_command, "cursor .");
    }
}
