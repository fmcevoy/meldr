use std::io::Read;
use std::path::PathBuf;

use crate::core::claude_hooks::sidecar::now_secs;
use crate::core::claude_hooks::{
    ClearTarget, HookPayload, clear, dispatch_notify, dispatch_session_start, dispatch_stop,
    resolve_for_selftest,
};
use crate::error::Result;
use crate::tmux::RealTmux;

/// Options for `meldr claude-hook clear`, which expires a flash.
#[derive(Debug, Default, Clone)]
pub struct ClearArgs {
    pub pane: Option<String>,
    pub window: Option<String>,
    pub all_panes: bool,
    /// Expire regardless of the recorded deadline — what a tmux `after-select-*`
    /// hook wants, since the user has now looked at the thing.
    pub now: bool,
}

pub fn state_dir() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| crate::error::MeldrError::Config("cannot determine HOME".into()))?;
    let dir = home.join(".cache/claude-agents");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn run(event: &str, clear_args: &ClearArgs) -> Result<()> {
    // Address the server named by $TMUX when there is one; a hook may well be
    // running under a non-default socket.
    let tmux = RealTmux::from_env();

    match event {
        "clear" => run_clear(&tmux, clear_args),
        "selftest" => run_selftest(&tmux),
        "register-launcher" => {
            // Retired: the launcher registry was a second, always-staler copy of
            // what `tmux list-panes` already knows. Kept as a no-op for one release
            // because shell wrappers in the wild still call it.
            eprintln!(
                "meldr claude-hook register-launcher: no longer needed and does nothing — \
                 remove the claude() wrapper from your shell rc"
            );
            Ok(())
        }
        _ => {
            let dir = state_dir()?;
            let mut stdin_buf = String::new();
            std::io::stdin().read_to_string(&mut stdin_buf).unwrap_or(0);
            let payload = HookPayload::from_json(&stdin_buf);

            match event {
                "session-start" => dispatch_session_start(&payload, &dir, &tmux),
                "stop" => dispatch_stop(&payload, &dir, &tmux),
                "notify" => dispatch_notify(&payload, &dir, &tmux),
                other => {
                    eprintln!("meldr claude-hook: unknown event '{other}'");
                    Ok(())
                }
            }
        }
    }
}

fn run_clear(tmux: &RealTmux, args: &ClearArgs) -> Result<()> {
    let target = match (args.pane.as_deref(), args.window.as_deref()) {
        (Some(pane), Some(window)) => ClearTarget::Pane {
            pane_id: pane.to_string(),
            window_id: window.to_string(),
        },
        (Some(pane), None) => {
            // Look the window up so the aggregate can still be recomputed.
            let window = tmux
                .snapshot()
                .ok()
                .and_then(|s| s.pane(pane).map(|r| r.window_id.clone()))
                .ok_or_else(|| {
                    crate::error::MeldrError::Tmux(format!(
                        "cannot find pane {pane}; pass --window explicitly"
                    ))
                })?;
            ClearTarget::Pane {
                pane_id: pane.to_string(),
                window_id: window,
            }
        }
        (None, Some(window)) => ClearTarget::Window {
            window_id: window.to_string(),
            all_panes: args.all_panes,
        },
        (None, None) => {
            return Err(crate::error::MeldrError::Config(
                "claude-hook clear needs --pane and/or --window".into(),
            ));
        }
    };

    use crate::tmux::TmuxOps as _;
    clear(tmux, &target, now_secs(), args.now)
}

/// Report what the resolver makes of the current process, and fail if it cannot
/// place itself. `meldr doctor hooks` runs this through a nested shell so the
/// process-tree walk is exercised the way a real hook exercises it.
fn run_selftest(tmux: &RealTmux) -> Result<()> {
    let dir = state_dir()?;
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    let resolved = resolve_for_selftest(Some(&cwd), &dir, tmux);
    let json = serde_json::json!({
        "pane": resolved.pane().map(|p| p.pane_id.clone()),
        "window": resolved.window_id(),
        "window_name": resolved.window_name(),
        "tier": resolved.tier.as_str(),
        "source": format!("{:?}", resolved.source),
    });
    println!("{json}");

    if resolved.window_id().is_none() {
        return Err(crate::error::MeldrError::Tmux(
            "resolver could not place this process in any tmux pane".into(),
        ));
    }
    Ok(())
}
