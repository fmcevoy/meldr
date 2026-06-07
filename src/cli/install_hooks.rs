use crate::core::{agent_signal, install_hooks};
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

    // Install the bundled script.
    if dry_run {
        println!("Would install: ~/.local/share/meldr/meldr-agent-notify.sh");
        println!("Would update:  ~/.claude/settings.json");
        install_hooks::install_claude_hooks(&home, true)?;
    } else {
        let script_dest = agent_signal::install_script(&home)?;
        println!("Installed: {}", script_dest.display());

        let settings_path = install_hooks::install_claude_hooks(&home, false)?;
        println!("Updated:   {}", settings_path.display());
        println!();
        println!("Claude Code hooks wired. Reload with: tmux source-file ~/.tmux.conf");
    }

    Ok(())
}
