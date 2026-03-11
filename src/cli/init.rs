use std::path::Path;

use crate::core::config;
use crate::core::workspace::Manifest;
use crate::error::{MeldrError, Result};

pub fn run(workspace_root: &Path, name: Option<&str>) -> Result<()> {
    let manifest_path = workspace_root.join("meldr.toml");
    if manifest_path.exists() {
        return Err(MeldrError::AlreadyInitialized(workspace_root.to_path_buf()));
    }

    let workspace_name = name
        .map(|n| n.to_string())
        .or_else(|| {
            workspace_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "workspace".to_string());

    let manifest = Manifest::new(&workspace_name);
    manifest.save_initial(workspace_root)?;

    std::fs::create_dir_all(workspace_root.join("packages"))?;
    std::fs::create_dir_all(workspace_root.join("worktrees"))?;
    std::fs::create_dir_all(workspace_root.join(".meldr"))?;

    // Ensure global config directory exists
    config::ensure_global_config()?;

    println!("Initialized meldr workspace '{workspace_name}'");
    Ok(())
}
