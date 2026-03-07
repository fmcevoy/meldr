use std::path::Path;

use crate::core::workspace::Manifest;
use crate::error::Result;
use crate::git::GitOps;

pub fn add(git: &dyn GitOps, workspace_root: &Path, urls: &[String]) -> Result<()> {
    let mut manifest = Manifest::load(workspace_root)?;
    let added = crate::core::package::add_packages(git, &mut manifest, workspace_root, urls)?;
    if added.is_empty() {
        println!("No packages were added.");
    } else {
        for name in &added {
            println!("Added package '{}'", name);
        }
    }
    Ok(())
}

pub fn remove(workspace_root: &Path, names: &[String]) -> Result<()> {
    let mut manifest = Manifest::load(workspace_root)?;
    let removed = crate::core::package::remove_packages(&mut manifest, workspace_root, names)?;
    if removed.is_empty() {
        println!("No packages were removed.");
    } else {
        for name in &removed {
            println!("Removed package '{}'", name);
        }
    }
    Ok(())
}

pub fn list(workspace_root: &Path) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;
    let packages = crate::core::package::list_packages(&manifest);
    if packages.is_empty() {
        println!("No packages in workspace.");
    } else {
        for pkg in packages {
            let branch = pkg.branch.as_deref().unwrap_or("(default)");
            println!("  {} ({}) [{}]", pkg.name, pkg.url, branch);
        }
    }
    Ok(())
}
