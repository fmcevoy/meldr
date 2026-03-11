use std::process::Command;

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
    pub window_id: String,
    pub editor: Option<String>,
    pub agent: Option<String>,
    #[allow(dead_code)]
    pub terms: Vec<String>,
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
        // Layout:
        // +-------------------+-----------+
        // |                   |           |
        // |    editor (0)     | agent (1) |
        // |                   |           |
        // +--------+----------+           |
        // | t1 (2) | t3 (4)   |           |
        // +--------+----------+           |
        // | t2 (3) | t4 (5)   |           |
        // +--------+----------+-----------+

        // Create window — pane 0 (editor)
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

        // Split right for agent — full-height right column, 35% width
        let agent_pane = Self::run(&[
            "split-window",
            "-t",
            &pane0,
            "-h",
            "-p",
            "35",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])?;

        // Split editor (pane 0) below for left terminal column — 30% height
        let t1_pane = Self::run(&[
            "split-window",
            "-t",
            &pane0,
            "-v",
            "-p",
            "30",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])?;

        // Split t1 below for t2
        let t2_pane = Self::run(&[
            "split-window",
            "-t",
            &t1_pane,
            "-v",
            "-p",
            "50",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])?;

        // Split t1 right for t3
        let t3_pane = Self::run(&[
            "split-window",
            "-t",
            &t1_pane,
            "-h",
            "-p",
            "50",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])?;

        // Split t2 right for t4
        let t4_pane = Self::run(&[
            "split-window",
            "-t",
            &t2_pane,
            "-h",
            "-p",
            "50",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])?;

        // Select the editor pane as active
        Self::run(&["select-pane", "-t", &pane0])?;

        Ok(DevWindowPanes {
            window_id,
            editor: Some(pane0),
            agent: Some(agent_pane),
            terms: vec![t1_pane, t2_pane, t3_pane, t4_pane],
        })
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
            agent: Some(agent_pane),
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
            agent: None,
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

        let agent_pane = layout_def.agent_pane.and_then(|i| pane_ids.get(i).cloned());

        Ok(DevWindowPanes {
            window_id,
            editor: editor_pane,
            agent: agent_pane,
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
        if let Some(layout_def) = custom_layout {
            return Self::create_custom_layout(name, cwd, layout_def, config);
        }

        match config.layout.as_str() {
            "minimal" => Self::create_minimal_layout(name, cwd),
            "editor-only" => Self::create_editor_only_layout(name, cwd),
            _ => Self::create_default_layout(name, cwd),
        }
    }

    fn has_window(&self, window: &str) -> bool {
        Self::run(&["has-session", "-t", window]).is_ok()
    }

    fn select_window(&self, window: &str) -> Result<()> {
        Self::run(&["select-window", "-t", window])?;
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
}
