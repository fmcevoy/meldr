use crate::core::install_hooks;
use crate::error::Result;

pub fn run(dry_run: bool, uninstall: bool) -> Result<()> {
    let home = dirs::home_dir()
        .ok_or_else(|| crate::error::MeldrError::Config("cannot determine HOME".into()))?;

    if uninstall {
        let path = install_hooks::uninstall_claude_hooks(&home, dry_run)?;
        if !dry_run {
            println!("meldr hooks removed from {}", path.display());
        }
        return Ok(());
    }

    if dry_run {
        println!("Would update: ~/.claude/settings.json");
        println!("  Stop       → meldr claude-hook stop");
        println!("  Notify     → meldr claude-hook notify");
        println!("  SessionStart → meldr claude-hook session-start");
        install_hooks::install_claude_hooks(&home, true)?;
    } else {
        let settings_path = install_hooks::install_claude_hooks(&home, false)?;
        println!("Updated: {}", settings_path.display());

        // Remove the legacy bash script installed by previous meldr versions.
        install_hooks::remove_legacy_notify_script(&home);

        // Warn if the legacy fmcevoy_tools script is still present.
        if install_hooks::legacy_session_start_symlink_present(&home) {
            eprintln!(
                "warning: ~/.claude/claude-session-start.sh still exists from a previous \
                 setup (fmcevoy_tools). meldr now owns the SessionStart hook via \
                 'meldr claude-hook session-start'. You can delete that file."
            );
        }

        println!();
        println!("Claude Code hooks wired.");
        println!();
        println!("No shell wrapper is needed. If your shell rc still defines a claude()");
        println!("function that exports MELDR_TMUX_PANE or MELDR_TMUX_WINDOW_ID, delete it —");
        println!("those variables are no longer read and a stale one misdirects notifications.");
    }

    Ok(())
}
