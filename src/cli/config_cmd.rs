use std::collections::HashMap;
use std::path::Path;

use crate::core::config::{self, CliOverrides};
use crate::core::workspace::Manifest;
use crate::error::Result;

pub fn set(workspace_root: Option<&Path>, key: &str, value: &str, global: bool) -> Result<()> {
    if global {
        config::global_config_set(key, value)?;
        println!("Set {} = {} (global)", key, value);
    } else {
        let root = require_workspace(workspace_root)?;
        config::config_set(root, key, value)?;
        println!("Set {} = {} (workspace)", key, value);
    }
    Ok(())
}

pub fn get(workspace_root: Option<&Path>, key: &str, global: bool) -> Result<()> {
    if global {
        match config::global_config_get(key)? {
            Some(value) => println!("{}", value),
            None => println!("{} is not set (global)", key),
        }
    } else {
        let root = require_workspace(workspace_root)?;
        match config::config_get(root, key)? {
            Some(value) => println!("{}", value),
            None => println!("{} is not set (workspace)", key),
        }
    }
    Ok(())
}

pub fn unset(workspace_root: Option<&Path>, key: &str, global: bool) -> Result<()> {
    if global {
        config::global_config_unset(key)?;
        println!("Unset {} (global)", key);
    } else {
        let root = require_workspace(workspace_root)?;
        config::config_unset(root, key)?;
        println!("Unset {} (workspace)", key);
    }
    Ok(())
}

pub fn list(workspace_root: Option<&Path>, global: bool) -> Result<()> {
    if global {
        let gc = config::global_config_list()?;
        println!("Global configuration (~/.meldr/config.toml):");
        println!("  agent = {}", gc.defaults.agent);
        println!("  mode = {}", gc.defaults.mode);
        print_opt("  editor", &gc.defaults.editor);
        print_opt("  default_branch", &gc.defaults.default_branch);
        print_opt("  remote", &gc.defaults.remote);
        print_opt("  shell", &gc.defaults.shell);
        print_opt("  layout", &gc.defaults.layout);
        print_opt("  window_name", &gc.defaults.window_name);
    } else {
        let root = require_workspace(workspace_root)?;
        let global_cfg = config::load_global_config()?;
        let manifest = Manifest::load(root)?;
        let cli = CliOverrides::default();
        let env = HashMap::new();
        let effective = config::resolve_config(&global_cfg, &manifest.settings, &cli, &env);

        println!("Effective configuration:");
        println!("  agent = {}", effective.agent);
        println!("  agent_command = {}", effective.agent_command);
        println!("  mode = {}", effective.mode);
        println!("  sync_method = {}", effective.sync_method);
        println!("  sync_strategy = {}", effective.sync_strategy);
        println!("  editor = {}", effective.editor);
        println!("  default_branch = {}", effective.default_branch);
        println!("  remote = {}", effective.remote);
        println!("  shell = {}", effective.shell);
        println!("  layout = {}", effective.layout);
        println!("  window_name = {}", effective.window_name_template);
    }
    Ok(())
}

pub fn show(workspace_root: Option<&Path>) -> Result<()> {
    let root = require_workspace(workspace_root)?;
    let global_cfg = config::load_global_config()?;
    let manifest = Manifest::load(root)?;

    let keys = [
        "agent", "mode", "sync_method", "sync_strategy",
        "editor", "default_branch", "remote", "shell", "layout", "window_name",
    ];

    println!("Configuration sources (workspace > global > default):\n");

    for key in &keys {
        let ws_val = ws_setting(&manifest.settings, key);
        let global_val = global_setting(&global_cfg.defaults, key);
        let default_val = default_for(key);

        let (effective, source) = if let Some(ref v) = ws_val {
            (v.as_str(), "workspace")
        } else if let Some(ref v) = global_val {
            (v.as_str(), "global")
        } else {
            (default_val, "default")
        };

        println!("  {} = {} ({})", key, effective, source);
    }
    Ok(())
}

fn require_workspace<'a>(root: Option<&'a Path>) -> Result<&'a Path> {
    root.ok_or_else(|| {
        crate::error::MeldrError::Config(
            "Not in a meldr workspace. Use --global for global config.".to_string(),
        )
    })
}

fn print_opt(label: &str, val: &Option<String>) {
    match val {
        Some(v) => println!("{} = {}", label, v),
        None => println!("{} = (not set)", label),
    }
}

fn ws_setting(settings: &crate::core::workspace::Settings, key: &str) -> Option<String> {
    match key {
        "agent" if !settings.agent.is_empty() => Some(settings.agent.clone()),
        "mode" if !settings.mode.is_empty() => Some(settings.mode.clone()),
        "sync_method" if !settings.sync_method.is_empty() => Some(settings.sync_method.clone()),
        "sync_strategy" if !settings.sync_strategy.is_empty() => Some(settings.sync_strategy.clone()),
        "editor" => settings.editor.clone(),
        "default_branch" => settings.default_branch.clone(),
        "remote" => settings.remote.clone(),
        "shell" => settings.shell.clone(),
        "layout" => settings.layout.clone(),
        "window_name" => settings.window_name.clone(),
        _ => None,
    }
}

fn global_setting(defaults: &config::GlobalDefaults, key: &str) -> Option<String> {
    match key {
        "agent" if defaults.agent != config::DEFAULT_AGENT => Some(defaults.agent.clone()),
        "mode" if defaults.mode != config::DEFAULT_MODE => Some(defaults.mode.clone()),
        "editor" => defaults.editor.clone(),
        "default_branch" => defaults.default_branch.clone(),
        "remote" => defaults.remote.clone(),
        "shell" => defaults.shell.clone(),
        "layout" => defaults.layout.clone(),
        "window_name" => defaults.window_name.clone(),
        _ => None,
    }
}

fn default_for(key: &str) -> &'static str {
    match key {
        "agent" => config::DEFAULT_AGENT,
        "mode" => config::DEFAULT_MODE,
        "sync_method" => config::DEFAULT_SYNC_METHOD,
        "sync_strategy" => config::DEFAULT_SYNC_STRATEGY,
        "editor" => config::DEFAULT_EDITOR,
        "default_branch" => config::DEFAULT_BRANCH,
        "remote" => config::DEFAULT_REMOTE,
        "shell" => config::DEFAULT_SHELL,
        "layout" => config::DEFAULT_LAYOUT,
        "window_name" => config::DEFAULT_WINDOW_NAME,
        _ => "(unknown)",
    }
}
