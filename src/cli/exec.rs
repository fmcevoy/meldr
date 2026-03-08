use std::path::Path;
use std::process::Command;

use rayon::prelude::*;

use crate::core::config::EffectiveConfig;
use crate::core::workspace::{self, Manifest};
use crate::error::Result;

pub fn run(workspace_root: &Path, command: &[String], config: &EffectiveConfig) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;

    if manifest.packages.is_empty() {
        println!("No packages in workspace.");
        return Ok(());
    }

    let cmd_str = command.join(" ");

    let results: Vec<_> = manifest
        .packages
        .par_iter()
        .map(|pkg| {
            let pkg_path = workspace::package_path(workspace_root, &pkg.name);
            let output = Command::new(&config.shell)
                .args(["-c", &cmd_str])
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
                    eprintln!(
                        "(exit code: {})",
                        output.status.code().unwrap_or(-1)
                    );
                }
            }
            Err(e) => {
                eprintln!("Failed to execute: {}", e);
            }
        }
    }

    Ok(())
}
