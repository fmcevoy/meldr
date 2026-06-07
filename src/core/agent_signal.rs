use std::path::{Path, PathBuf};

use crate::error::{MeldrError, Result};

/// The canonical notify script, bundled into the binary at compile time.
pub const NOTIFY_SCRIPT: &str = include_str!("../assets/meldr-agent-notify.sh");

/// Install the bundled script to `~/.local/share/meldr/meldr-agent-notify.sh`
/// with executable permissions. Returns the installed path.
pub fn install_script(home: &Path) -> Result<PathBuf> {
    let dir = home.join(".local/share/meldr");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join("meldr-agent-notify.sh");
    std::fs::write(&dest, NOTIFY_SCRIPT)?;
    set_executable(&dest)?;
    Ok(dest)
}

/// Returns true if the installed script matches the version bundled in this binary.
pub fn is_script_current(home: &Path) -> bool {
    let installed = home.join(".local/share/meldr/meldr-agent-notify.sh");
    std::fs::read_to_string(&installed).ok().as_deref() == Some(NOTIFY_SCRIPT)
}

fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(MeldrError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_script_sets_executable_mode() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::TempDir::new().unwrap();
        let dest = install_script(home.path()).unwrap();
        let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(mode & 0o755, 0o755, "script must be executable");
    }

    #[test]
    fn test_install_script_writes_embedded_content() {
        let home = tempfile::TempDir::new().unwrap();
        let dest = install_script(home.path()).unwrap();
        let on_disk = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(on_disk, NOTIFY_SCRIPT);
    }

    #[test]
    fn test_is_script_current_true_after_install() {
        let home = tempfile::TempDir::new().unwrap();
        install_script(home.path()).unwrap();
        assert!(is_script_current(home.path()));
    }

    #[test]
    fn test_is_script_current_false_when_stale() {
        let home = tempfile::TempDir::new().unwrap();
        let path = home.path().join(".local/share/meldr/meldr-agent-notify.sh");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "old content").unwrap();
        assert!(!is_script_current(home.path()));
    }
}
