use std::collections::HashMap;
use std::path::Path;

use crate::core::config::{self, CliOverrides};
use crate::core::workspace::Manifest;
use crate::error::Result;

pub fn set(workspace_root: &Path, key: &str, value: &str) -> Result<()> {
    config::config_set(workspace_root, key, value)?;
    println!("Set {} = {}", key, value);
    Ok(())
}

pub fn get(workspace_root: &Path, key: &str) -> Result<()> {
    match config::config_get(workspace_root, key)? {
        Some(value) => println!("{} = {}", key, value),
        None => println!("{} is not set", key),
    }
    Ok(())
}

pub fn list(workspace_root: &Path) -> Result<()> {
    let global = config::load_global_config()?;
    let manifest = Manifest::load(workspace_root)?;
    let cli = CliOverrides::default();
    let env = HashMap::new();
    let effective = config::resolve_config(&global, &manifest.settings, &cli, &env);

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
    Ok(())
}
