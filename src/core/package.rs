use std::path::Path;

use crate::core::workspace::{self, Manifest, PackageEntry};
use crate::error::{MeldrError, Result};
use crate::git::GitOps;

pub fn add_packages(
    git: &dyn GitOps,
    manifest: &mut Manifest,
    workspace_root: &Path,
    urls: &[String],
) -> Result<Vec<String>> {
    let packages_dir = workspace::packages_dir(workspace_root);
    std::fs::create_dir_all(&packages_dir)?;

    let mut added = Vec::new();
    let mut errors = Vec::new();

    for url in urls {
        let name = workspace::derive_package_name(url);

        if manifest.find_package(&name).is_some() {
            errors.push(format!("Package '{name}' already exists, skipping"));
            continue;
        }

        let dest = packages_dir.join(&name);
        match git.clone_repo(url, &dest) {
            Ok(()) => {
                let entry = PackageEntry {
                    name: name.clone(),
                    url: url.clone(),
                    branch: None,
                    remote: None,
                    sync_strategy: None,
                };
                manifest.add_package(entry)?;
                added.push(name);
            }
            Err(e) => {
                errors.push(format!("Failed to clone '{url}': {e}"));
                let _ = std::fs::remove_dir_all(&dest);
            }
        }
    }

    if !added.is_empty() {
        manifest.save(workspace_root)?;
    }

    for error in &errors {
        eprintln!("Warning: {error}");
    }

    Ok(added)
}

pub fn remove_packages(
    manifest: &mut Manifest,
    workspace_root: &Path,
    names: &[String],
) -> Result<Vec<String>> {
    let mut removed = Vec::new();

    for name in names {
        match manifest.remove_package(name) {
            Ok(_entry) => {
                let pkg_path = workspace::package_path(workspace_root, name);
                if pkg_path.exists() {
                    std::fs::remove_dir_all(&pkg_path)?;
                }
                removed.push(name.clone());
            }
            Err(MeldrError::PackageNotFound(n)) => {
                eprintln!("Warning: Package '{n}' not found, skipping");
            }
            Err(e) => return Err(e),
        }
    }

    if !removed.is_empty() {
        manifest.save(workspace_root)?;
    }

    Ok(removed)
}

pub fn list_packages(manifest: &Manifest) -> &[PackageEntry] {
    &manifest.packages
}
