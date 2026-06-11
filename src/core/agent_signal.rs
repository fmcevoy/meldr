// Deprecated: this module previously managed the meldr-agent-notify.sh bash script.
// Hook handling is now done entirely in Rust via `meldr claude-hook`.
// All public items are stubs that do nothing; the module will be removed in a
// follow-up cleanup once all call sites in doctor.rs are updated.
use std::path::{Path, PathBuf};

use crate::error::Result;

/// No-op: the bash script no longer exists.
#[allow(dead_code)]
pub fn install_script(_home: &Path) -> Result<PathBuf> {
    Ok(_home.join(".local/share/meldr/meldr-agent-notify.sh"))
}

/// Always returns true: there is no script to be stale.
#[allow(dead_code)]
pub fn is_script_current(_home: &Path) -> bool {
    true
}
