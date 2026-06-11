use std::process::Command;
#[cfg(test)]
use std::sync::Mutex;

use crate::core::config::{EffectiveConfig, LayoutDef};
use crate::error::{MeldrError, Result};
use crate::trace;

#[derive(Debug, Clone)]
pub struct TmuxLayout {
    pub definition: String,
    #[allow(dead_code)]
    pub pane_names: Vec<String>,
}

/// Pane targets in the dev layout.
pub struct DevWindowPanes {
    #[allow(dead_code)]
    pub window_id: String,
    pub editor: Option<String>,
    pub agents: Vec<String>,
    #[allow(dead_code)]
    pub terms: Vec<String>,
}

/// Scope for `set-option` / `set-option -u` (unset) calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionScope {
    /// `-w` — window-scoped user option.
    Window,
    /// `-p` — pane-scoped user option.
    Pane,
}

pub trait TmuxOps: Send + Sync {
    fn is_inside_tmux(&self) -> bool;
    fn create_window(&self, name: &str) -> Result<String>;
    fn split_window(&self, window: &str) -> Result<()>;
    fn apply_layout(&self, window: &str, layout: &TmuxLayout) -> Result<()>;
    fn send_keys(&self, target: &str, keys: &str) -> Result<()>;
    fn kill_window(&self, window: &str) -> Result<()>;
    fn create_dev_window(
        &self,
        name: &str,
        cwd: &str,
        config: &EffectiveConfig,
        custom_layout: Option<&LayoutDef>,
    ) -> Result<DevWindowPanes>;
    /// Check whether a tmux window still exists.
    fn has_window(&self, window: &str) -> bool;
    /// Select (focus) an existing tmux window.
    fn select_window(&self, window: &str) -> Result<()>;
    /// Find a window's numeric ID (`@N`) by its display name, searching all sessions.
    /// Returns `None` if no window with that name exists.
    fn find_window_id_by_name(&self, name: &str) -> Option<String>;

    // ── notification / hook helpers ──────────────────────────────────────────

    /// Return true if the pane identified by `pane_id` (e.g. `%42`) currently exists.
    fn pane_exists(&self, pane_id: &str) -> bool;

    /// Run `tmux display-message -p -t <target> <format>` and return the output.
    /// Returns an error if the target does not exist or tmux is not running.
    fn display_message(&self, target: &str, format: &str) -> Result<String>;

    /// Set a window- or pane-scoped user option via `tmux set-option`.
    fn set_user_option(
        &self,
        scope: OptionScope,
        target: &str,
        key: &str,
        value: &str,
    ) -> Result<()>;

    /// Run `tmux run-shell -b <cmd>` — fire-and-forget background shell.
    fn run_shell_bg(&self, cmd: &str) -> Result<()>;
}

#[derive(Default)]
pub struct RealTmux;

impl RealTmux {
    pub fn new() -> Self {
        Self
    }

    fn run(args: &[&str]) -> Result<String> {
        trace::trace_cmd("tmux", args, None);

        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(|e| MeldrError::Tmux(format!("Failed to run tmux: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(MeldrError::Tmux(stderr))
        }
    }

    #[allow(dead_code)] // Layout variant available for tmux window configuration
    fn create_default_layout(name: &str, cwd: &str) -> Result<DevWindowPanes> {
        // Layout: 9 panes — 3 claude (top 2/3) + 6 terminals (bottom 1/3 in 2×3 grid).
        // The middle-row leftmost terminal runs the configured editor (e.g. nvim).
        //
        // +-----------+-----------+-----------+
        // | claude P0 | claude P3 | claude P4 |   top 2/3
        // +-----------+-----------+-----------+
        // | nvim  P1  |  term P5  |  term P6  |   1/2 of bottom 1/3
        // +-----------+-----------+-----------+
        // |  term P2  |  term P7  |  term P8  |   1/2 of bottom 1/3
        // +-----------+-----------+-----------+

        let window_id = Self::run(&[
            "new-window",
            "-n",
            name,
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}",
        ])?;
        let p0 = format!("{window_id}.0");

        // Split off the bottom 1/3 (full width) → P1 below P0.
        let p1 = Self::split(&p0, "-v", 33, cwd)?;
        // Split P1 in half vertically → P2 (bottom row).
        let p2 = Self::split(&p1, "-v", 50, cwd)?;

        // Top row: split P0 into 3 equal columns (P0 | P3 | P4).
        let p3 = Self::split(&p0, "-h", 67, cwd)?;
        let p4 = Self::split(&p3, "-h", 50, cwd)?;

        // Middle row: split P1 into 3 equal columns (P1 | P5 | P6).
        let p5 = Self::split(&p1, "-h", 67, cwd)?;
        let p6 = Self::split(&p5, "-h", 50, cwd)?;

        // Bottom row: split P2 into 3 equal columns (P2 | P7 | P8).
        let p7 = Self::split(&p2, "-h", 67, cwd)?;
        let p8 = Self::split(&p7, "-h", 50, cwd)?;

        // Focus the editor pane.
        Self::run(&["select-pane", "-t", &p1])?;

        Ok(DevWindowPanes {
            window_id,
            editor: Some(p1),
            agents: vec![p0, p3, p4],
            terms: vec![p5, p6, p2, p7, p8],
        })
    }

    /// Run `split-window` with a percentage and capture the new pane id.
    fn split(target: &str, direction: &str, pct: u32, cwd: &str) -> Result<String> {
        let pct_str = pct.to_string();
        Self::run(&[
            "split-window",
            "-t",
            target,
            direction,
            "-p",
            &pct_str,
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])
    }

    #[allow(dead_code)] // Layout variant available for tmux window configuration
    fn create_minimal_layout(name: &str, cwd: &str) -> Result<DevWindowPanes> {
        // Layout:
        // +-------------------+-----------+
        // |                   |           |
        // |    editor (0)     | agent (1) |
        // |                   |           |
        // +-------------------+-----------+

        let window_id = Self::run(&[
            "new-window",
            "-n",
            name,
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}",
        ])?;
        let pane0 = format!("{window_id}.0");

        let agent_pane = Self::run(&[
            "split-window",
            "-t",
            &pane0,
            "-h",
            "-p",
            "40",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])?;

        Self::run(&["select-pane", "-t", &pane0])?;

        Ok(DevWindowPanes {
            window_id,
            editor: Some(pane0),
            agents: vec![agent_pane],
            terms: vec![],
        })
    }

    #[allow(dead_code)] // Layout variant available for tmux window configuration
    fn create_editor_only_layout(name: &str, cwd: &str) -> Result<DevWindowPanes> {
        // Single pane — editor only
        let window_id = Self::run(&[
            "new-window",
            "-n",
            name,
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}",
        ])?;
        let pane0 = format!("{window_id}.0");

        Ok(DevWindowPanes {
            window_id,
            editor: Some(pane0),
            agents: vec![],
            terms: vec![],
        })
    }

    fn create_custom_layout(
        name: &str,
        cwd: &str,
        layout_def: &LayoutDef,
        config: &EffectiveConfig,
    ) -> Result<DevWindowPanes> {
        let window_id = Self::run(&[
            "new-window",
            "-n",
            name,
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}",
        ])?;

        // Track pane IDs as they're created. Pane 0 is created with the window.
        let mut pane_ids = vec![format!("{}.0", window_id)];

        for step in &layout_def.setup {
            let expanded = step
                .replace("{{window}}", &window_id)
                .replace("{{cwd}}", cwd)
                .replace("{{editor}}", &config.editor)
                .replace("{{agent}}", &config.agent_command);

            // Parse the expanded command into args and run
            let args: Vec<&str> = expanded.split_whitespace().collect();
            if args.is_empty() {
                continue;
            }

            let result = Self::run(&args)?;

            // If it was a split-window with -P -F, capture the pane ID
            if args.first() == Some(&"split-window") && expanded.contains("#{pane_id}") {
                pane_ids.push(result);
            }
        }

        let editor_pane = layout_def
            .editor_pane
            .and_then(|i| pane_ids.get(i).cloned());

        let agents = layout_def
            .agent_pane
            .and_then(|i| pane_ids.get(i).cloned())
            .map(|p| vec![p])
            .unwrap_or_default();

        Ok(DevWindowPanes {
            window_id,
            editor: editor_pane,
            agents,
            terms: vec![],
        })
    }
}

impl TmuxOps for RealTmux {
    fn is_inside_tmux(&self) -> bool {
        std::env::var("TMUX").is_ok()
    }

    fn create_window(&self, name: &str) -> Result<String> {
        let window_id = Self::run(&["new-window", "-n", name, "-P", "-F", "#{window_id}"])?;
        Ok(window_id)
    }

    fn split_window(&self, window: &str) -> Result<()> {
        Self::run(&["split-window", "-t", window])?;
        Ok(())
    }

    fn apply_layout(&self, window: &str, layout: &TmuxLayout) -> Result<()> {
        Self::run(&["select-layout", "-t", window, &layout.definition])?;
        Ok(())
    }

    fn send_keys(&self, target: &str, keys: &str) -> Result<()> {
        Self::run(&["send-keys", "-t", target, keys, "Enter"])?;
        Ok(())
    }

    fn kill_window(&self, window: &str) -> Result<()> {
        Self::run(&["kill-window", "-t", window])?;
        Ok(())
    }

    fn create_dev_window(
        &self,
        name: &str,
        cwd: &str,
        config: &EffectiveConfig,
        custom_layout: Option<&LayoutDef>,
    ) -> Result<DevWindowPanes> {
        let dev = if let Some(layout_def) = custom_layout {
            Self::create_custom_layout(name, cwd, layout_def, config)?
        } else {
            match config.layout.as_str() {
                "minimal" => Self::create_minimal_layout(name, cwd)?,
                "editor-only" => Self::create_editor_only_layout(name, cwd)?,
                _ => Self::create_default_layout(name, cwd)?,
            }
        };

        // Disable bell and activity monitoring per window so build-script output
        // in terminal panes doesn't false-positive flash the tab (F7b).
        let _ = Self::run(&[
            "set-window-option",
            "-t",
            &dev.window_id,
            "monitor-bell",
            "off",
        ]);
        let _ = Self::run(&[
            "set-window-option",
            "-t",
            &dev.window_id,
            "monitor-activity",
            "off",
        ]);

        Ok(dev)
    }

    fn has_window(&self, window: &str) -> bool {
        Self::run(&["has-session", "-t", window]).is_ok()
    }

    fn select_window(&self, window: &str) -> Result<()> {
        Self::run(&["select-window", "-t", window])?;
        Ok(())
    }

    fn find_window_id_by_name(&self, name: &str) -> Option<String> {
        let output =
            Self::run(&["list-windows", "-a", "-F", "#{window_id} #{window_name}"]).ok()?;
        for line in output.lines() {
            if let Some((id, wname)) = line.split_once(' ')
                && wname == name
            {
                return Some(id.to_string());
            }
        }
        None
    }

    fn pane_exists(&self, pane_id: &str) -> bool {
        // `display-message -p` on a non-existent target exits non-zero.
        Self::run(&["display-message", "-p", "-t", pane_id, "1"]).is_ok()
    }

    fn display_message(&self, target: &str, format: &str) -> Result<String> {
        Self::run(&["display-message", "-p", "-t", target, format])
    }

    fn set_user_option(
        &self,
        scope: OptionScope,
        target: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let flag = match scope {
            OptionScope::Window => "-w",
            OptionScope::Pane => "-p",
        };
        Self::run(&["set-option", flag, "-t", target, key, value])?;
        Ok(())
    }

    fn run_shell_bg(&self, cmd: &str) -> Result<()> {
        Self::run(&["run-shell", "-b", cmd])?;
        Ok(())
    }
}

#[allow(dead_code)]
pub struct NoopTmux;

impl TmuxOps for NoopTmux {
    fn is_inside_tmux(&self) -> bool {
        false
    }
    fn create_window(&self, _name: &str) -> Result<String> {
        Err(MeldrError::NotInTmux)
    }
    fn split_window(&self, _window: &str) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }
    fn apply_layout(&self, _window: &str, _layout: &TmuxLayout) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }
    fn send_keys(&self, _target: &str, _keys: &str) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }
    fn kill_window(&self, _window: &str) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }
    fn create_dev_window(
        &self,
        _name: &str,
        _cwd: &str,
        _config: &EffectiveConfig,
        _custom_layout: Option<&LayoutDef>,
    ) -> Result<DevWindowPanes> {
        Err(MeldrError::NotInTmux)
    }
    fn has_window(&self, _window: &str) -> bool {
        false
    }
    fn select_window(&self, _window: &str) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }
    fn find_window_id_by_name(&self, _name: &str) -> Option<String> {
        None
    }
    fn pane_exists(&self, _pane_id: &str) -> bool {
        false
    }
    fn display_message(&self, _target: &str, _format: &str) -> Result<String> {
        Err(MeldrError::NotInTmux)
    }
    fn set_user_option(
        &self,
        _scope: OptionScope,
        _target: &str,
        _key: &str,
        _value: &str,
    ) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }
    fn run_shell_bg(&self, _cmd: &str) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }
}

/// A `TmuxOps` implementation that records calls for use in unit tests,
/// and allows stubbing `pane_exists` and `display_message`.
#[cfg(test)]
pub struct RecordingTmux {
    /// Pane IDs that exist. All others return false from `pane_exists`.
    pub live_panes: Vec<String>,
    /// Stubbed `display_message` responses keyed by `"target\x00format"`.
    pub display_stubs: std::collections::HashMap<String, String>,
    /// Recorded `set_user_option` calls: `(scope, target, key, value)`.
    pub set_calls: Mutex<Vec<(OptionScope, String, String, String)>>,
    /// Recorded `run_shell_bg` calls.
    pub bg_calls: Mutex<Vec<String>>,
}

#[cfg(test)]
impl RecordingTmux {
    pub fn new(live_panes: Vec<String>) -> Self {
        Self {
            live_panes,
            display_stubs: std::collections::HashMap::new(),
            set_calls: Mutex::new(Vec::new()),
            bg_calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with_display(mut self, target: &str, format: &str, response: &str) -> Self {
        self.display_stubs
            .insert(format!("{target}\x00{format}"), response.to_string());
        self
    }
}

#[cfg(test)]
impl TmuxOps for RecordingTmux {
    fn is_inside_tmux(&self) -> bool {
        true
    }
    fn create_window(&self, _name: &str) -> Result<String> {
        Ok("@1".to_string())
    }
    fn split_window(&self, _window: &str) -> Result<()> {
        Ok(())
    }
    fn apply_layout(&self, _window: &str, _layout: &TmuxLayout) -> Result<()> {
        Ok(())
    }
    fn send_keys(&self, _target: &str, _keys: &str) -> Result<()> {
        Ok(())
    }
    fn kill_window(&self, _window: &str) -> Result<()> {
        Ok(())
    }
    fn create_dev_window(
        &self,
        _name: &str,
        _cwd: &str,
        _config: &EffectiveConfig,
        _custom_layout: Option<&LayoutDef>,
    ) -> Result<DevWindowPanes> {
        Ok(DevWindowPanes {
            window_id: "@1".to_string(),
            editor: None,
            agents: vec!["%1".to_string()],
            terms: vec![],
        })
    }
    fn has_window(&self, _window: &str) -> bool {
        true
    }
    fn select_window(&self, _window: &str) -> Result<()> {
        Ok(())
    }
    fn find_window_id_by_name(&self, _name: &str) -> Option<String> {
        None
    }
    fn pane_exists(&self, pane_id: &str) -> bool {
        self.live_panes.iter().any(|p| p == pane_id)
    }
    fn display_message(&self, target: &str, format: &str) -> Result<String> {
        let key = format!("{target}\x00{format}");
        self.display_stubs.get(&key).cloned().ok_or_else(|| {
            MeldrError::Tmux(format!("no stub for display_message({target}, {format})"))
        })
    }
    fn set_user_option(
        &self,
        scope: OptionScope,
        target: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        self.set_calls.lock().unwrap().push((
            scope,
            target.to_string(),
            key.to_string(),
            value.to_string(),
        ));
        Ok(())
    }
    fn run_shell_bg(&self, cmd: &str) -> Result<()> {
        self.bg_calls.lock().unwrap().push(cmd.to_string());
        Ok(())
    }
}
