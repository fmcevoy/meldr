use std::io::Read;

use crate::core::claude_hooks::{
    HookPayload, dispatch_notify, dispatch_register_launcher, dispatch_session_start, dispatch_stop,
};
use crate::error::Result;
use crate::tmux::RealTmux;

pub fn run(event: &str) -> Result<()> {
    let home = dirs::home_dir()
        .ok_or_else(|| crate::error::MeldrError::Config("cannot determine HOME".into()))?;
    let state_dir = home.join(".cache/claude-agents");
    std::fs::create_dir_all(&state_dir)?;

    let tmux = RealTmux::new();

    match event {
        "register-launcher" => dispatch_register_launcher(&state_dir, &tmux),
        _ => {
            // All other events read hook JSON from stdin.
            let mut stdin_buf = String::new();
            std::io::stdin().read_to_string(&mut stdin_buf).unwrap_or(0);
            let payload = HookPayload::from_json(&stdin_buf);

            match event {
                "session-start" => dispatch_session_start(&payload, &state_dir, &tmux),
                "stop" => dispatch_stop(&payload, &state_dir, &tmux),
                "notify" => dispatch_notify(&payload, &state_dir, &tmux),
                other => {
                    eprintln!("meldr claude-hook: unknown event '{other}'");
                    Ok(())
                }
            }
        }
    }
}
