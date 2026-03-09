use std::path::Path;
use std::process::Command;

use rayon::prelude::*;

use crate::core::config::EffectiveConfig;
use crate::core::workspace::{self, Manifest};
use crate::error::{MeldrError, Result};

pub fn run(
    workspace_root: &Path,
    cwd: &Path,
    command: &[String],
    config: &EffectiveConfig,
    interactive: bool,
) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;

    if manifest.packages.is_empty() {
        println!("No packages in workspace.");
        return Ok(());
    }

    let worktree_dir_name = workspace::detect_current_worktree_dir(workspace_root, cwd)
        .ok_or_else(|| {
            MeldrError::Config(
                "meldr exec must be run from within a worktree directory.".to_string(),
            )
        })?;

    let cmd_str = command.join(" ");

    let shell_args: Vec<&str> = if interactive {
        vec!["-i", "-c", &cmd_str]
    } else {
        vec!["-c", &cmd_str]
    };

    let worktree_base = workspace::worktrees_dir(workspace_root).join(&worktree_dir_name);

    let results: Vec<_> = manifest
        .packages
        .par_iter()
        .map(|pkg| {
            let pkg_path = worktree_base.join(&pkg.name);
            let output = Command::new(&config.shell)
                .args(&shell_args)
                .current_dir(&pkg_path)
                .output();
            (pkg.name.clone(), output)
        })
        .collect();

    for (name, result) in results {
        println!("--- {} ---", name);
        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stdout.is_empty() {
                    print!("{}", stdout);
                }
                if !stderr.is_empty() {
                    eprint!("{}", stderr);
                }
                if !output.status.success() {
                    eprintln!("(exit code: {})", output.status.code().unwrap_or(-1));
                }
            }
            Err(e) => {
                eprintln!("Failed to execute: {}", e);
            }
        }
    }

    Ok(())
}
