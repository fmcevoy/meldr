pub mod snapshot;

use std::path::PathBuf;
use std::process::Command;
#[cfg(test)]
use std::sync::Mutex;

use crate::core::config::{EffectiveConfig, LayoutDef};
use crate::error::{MeldrError, Result};
use crate::trace;

use snapshot::{TmuxSnapshot, is_pane_id, is_window_id};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    // ── notification surface ─────────────────────────────────────────────────
    //
    // These default to "no tmux" so the many narrow test doubles in this crate
    // don't have to restate them; the implementations that matter (`RealTmux`,
    // `RecordingTmux`) override every one.

    /// Unset a window- or pane-scoped user option via `tmux set-option -u`.
    fn unset_user_option(&self, _scope: OptionScope, _target: &str, _key: &str) -> Result<()> {
        Err(MeldrError::NotInTmux)
    }

    /// Read a window- or pane-scoped user option. `Ok(None)` when unset.
    fn show_user_option(
        &self,
        _scope: OptionScope,
        _target: &str,
        _key: &str,
    ) -> Result<Option<String>> {
        Err(MeldrError::NotInTmux)
    }

    /// Every pane on the server, plus the server's identity, from one `list-panes -a`.
    ///
    /// This is the only sanctioned way to learn where a pane lives: a row is
    /// internally consistent, so a pane id always carries the window it is really in.
    fn snapshot(&self) -> Result<TmuxSnapshot> {
        Err(MeldrError::NotInTmux)
    }

    /// `(pane_index, pane_id)` for one window, in index order.
    ///
    /// Used to turn a positional target like `@3.0` into the stable `%N` that
    /// survives panes being reordered, killed, or moved.
    fn list_window_panes(&self, _window: &str) -> Result<Vec<(u32, String)>> {
        Err(MeldrError::NotInTmux)
    }

    /// `(pane_id, value)` of a pane-scoped user option across one window; the value
    /// is an empty string where the option is unset.
    fn window_pane_options(&self, _window: &str, _key: &str) -> Result<Vec<(String, String)>> {
        Err(MeldrError::NotInTmux)
    }
}

/// Reject a target that tmux would silently misinterpret.
///
/// `tmux set-option -w -t '' …` exits 0 and applies to whatever window happens to be
/// focused, and a positional `@3.0` re-binds to whichever pane currently sits at that
/// index. Both produce a flash on the wrong thing with no error anywhere, so every
/// targeted call is gated on the id actually being a `%N` / `@N`.
fn check_target(scope: OptionScope, target: &str) -> Result<()> {
    let ok = match scope {
        OptionScope::Window => is_window_id(target),
        OptionScope::Pane => is_pane_id(target),
    };
    if ok {
        return Ok(());
    }
    let kind = match scope {
        OptionScope::Window => "window id (@N)",
        OptionScope::Pane => "pane id (%N)",
    };
    Err(MeldrError::Tmux(format!(
        "refusing to target {target:?}: not a {kind}"
    )))
}

#[derive(Default)]
pub struct RealTmux {
    /// Socket of the server named by `$TMUX`, when there is one.
    ///
    /// A hook may run under a non-default socket, and `$TMUX` is the only thing that
    /// says which. Passing it explicitly beats relying on the default socket, which
    /// would silently address a different server.
    socket: Option<PathBuf>,
}

impl RealTmux {
    /// Talk to the default socket.
    pub fn new() -> Self {
        Self { socket: None }
    }

    /// Talk to the server named by `$TMUX`, falling back to the default socket.
    pub fn from_env() -> Self {
        let socket = std::env::var("TMUX")
            .ok()
            .as_deref()
            .and_then(snapshot::parse_tmux_env)
            .map(|(sock, _pid)| sock);
        Self { socket }
    }

    /// Server pid recorded in `$TMUX`, used to detect an environment left over from
    /// a server that has since died and had its socket path reused.
    fn env_server_pid(&self) -> Option<u32> {
        std::env::var("TMUX")
            .ok()
            .as_deref()
            .and_then(snapshot::parse_tmux_env)
            .map(|(_sock, pid)| pid)
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let mut full: Vec<&str> = Vec::with_capacity(args.len() + 2);
        let sock = self.socket.as_ref().map(|p| p.to_string_lossy());
        if let Some(sock) = sock.as_deref() {
            full.push("-S");
            full.push(sock);
        }
        full.extend_from_slice(args);

        trace::trace_cmd("tmux", &full, None);

        let output = Command::new("tmux")
            .args(&full)
            .output()
            .map_err(|e| MeldrError::Tmux(format!("Failed to run tmux: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(MeldrError::Tmux(stderr))
        }
    }

    /// Like `run`, but keeps trailing/interior whitespace — `list-panes` rows may end
    /// in an empty field that `trim` would eat.
    fn run_raw(&self, args: &[&str]) -> Result<String> {
        let mut full: Vec<&str> = Vec::with_capacity(args.len() + 2);
        let sock = self.socket.as_ref().map(|p| p.to_string_lossy());
        if let Some(sock) = sock.as_deref() {
            full.push("-S");
            full.push(sock);
        }
        full.extend_from_slice(args);

        trace::trace_cmd("tmux", &full, None);

        let output = Command::new("tmux")
            .args(&full)
            .output()
            .map_err(|e| MeldrError::Tmux(format!("Failed to run tmux: {e}")))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(MeldrError::Tmux(stderr))
        }
    }

    #[allow(dead_code)] // Layout variant available for tmux window configuration
    fn create_default_layout(&self, name: &str, cwd: &str) -> Result<DevWindowPanes> {
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

        let (window_id, p0) = self.new_window_capturing(name, cwd)?;

        // Split off the bottom 1/3 (full width) → P1 below P0.
        let p1 = self.split(&p0, "-v", 33, cwd)?;
        // Split P1 in half vertically → P2 (bottom row).
        let p2 = self.split(&p1, "-v", 50, cwd)?;

        // Top row: split P0 into 3 equal columns (P0 | P3 | P4).
        let p3 = self.split(&p0, "-h", 67, cwd)?;
        let p4 = self.split(&p3, "-h", 50, cwd)?;

        // Middle row: split P1 into 3 equal columns (P1 | P5 | P6).
        let p5 = self.split(&p1, "-h", 67, cwd)?;
        let p6 = self.split(&p5, "-h", 50, cwd)?;

        // Bottom row: split P2 into 3 equal columns (P2 | P7 | P8).
        let p7 = self.split(&p2, "-h", 67, cwd)?;
        let p8 = self.split(&p7, "-h", 50, cwd)?;

        // Focus the editor pane.
        self.run(&["select-pane", "-t", &p1])?;

        Ok(DevWindowPanes {
            window_id,
            editor: Some(p1),
            agents: vec![p0, p3, p4],
            terms: vec![p5, p6, p2, p7, p8],
        })
    }

    /// Create a window and capture **both** its id and its first pane's real id.
    ///
    /// `new-window` only reports one format by default and the old code took the
    /// window id, then addressed pane 0 as `@N.0`. That is a positional target: it
    /// resolves to whatever pane currently sits at index 0, so once a pane was
    /// killed, swapped or broken out, everything recorded about "pane 0" pointed at
    /// a different pane — including the agent-pane identity in `state.json`.
    fn new_window_capturing(&self, name: &str, cwd: &str) -> Result<(String, String)> {
        let out = self.run(&[
            "new-window",
            "-n",
            name,
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}\t#{pane_id}",
        ])?;
        let (window_id, pane_id) = out
            .trim()
            .split_once('\t')
            .ok_or_else(|| MeldrError::Tmux(format!("unexpected new-window output: {out:?}")))?;
        if !is_window_id(window_id) || !is_pane_id(pane_id) {
            return Err(MeldrError::Tmux(format!(
                "new-window returned unusable ids: window {window_id:?}, pane {pane_id:?}"
            )));
        }
        Ok((window_id.to_string(), pane_id.to_string()))
    }

    /// Run `split-window` with a percentage and capture the new pane id.
    fn split(&self, target: &str, direction: &str, pct: u32, cwd: &str) -> Result<String> {
        let pct_str = pct.to_string();
        self.run(&[
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
    fn create_minimal_layout(&self, name: &str, cwd: &str) -> Result<DevWindowPanes> {
        // Layout:
        // +-------------------+-----------+
        // |                   |           |
        // |    editor (0)     | agent (1) |
        // |                   |           |
        // +-------------------+-----------+

        let (window_id, pane0) = self.new_window_capturing(name, cwd)?;

        let agent_pane = self.run(&[
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

        self.run(&["select-pane", "-t", &pane0])?;

        Ok(DevWindowPanes {
            window_id,
            editor: Some(pane0),
            agents: vec![agent_pane],
            terms: vec![],
        })
    }

    #[allow(dead_code)] // Layout variant available for tmux window configuration
    fn create_editor_only_layout(&self, name: &str, cwd: &str) -> Result<DevWindowPanes> {
        // Single pane — editor only
        let (window_id, pane0) = self.new_window_capturing(name, cwd)?;

        Ok(DevWindowPanes {
            window_id,
            editor: Some(pane0),
            agents: vec![],
            terms: vec![],
        })
    }

    fn create_custom_layout(
        &self,
        name: &str,
        cwd: &str,
        layout_def: &LayoutDef,
        config: &EffectiveConfig,
    ) -> Result<DevWindowPanes> {
        let (window_id, pane0) = self.new_window_capturing(name, cwd)?;

        // Track pane IDs as they're created. Pane 0 comes back with the window.
        let mut pane_ids = vec![pane0];

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

            let result = self.run(&args)?;

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
        let window_id = self.run(&["new-window", "-n", name, "-P", "-F", "#{window_id}"])?;
        Ok(window_id)
    }

    fn split_window(&self, window: &str) -> Result<()> {
        self.run(&["split-window", "-t", window])?;
        Ok(())
    }

    fn apply_layout(&self, window: &str, layout: &TmuxLayout) -> Result<()> {
        self.run(&["select-layout", "-t", window, &layout.definition])?;
        Ok(())
    }

    fn send_keys(&self, target: &str, keys: &str) -> Result<()> {
        self.run(&["send-keys", "-t", target, keys, "Enter"])?;
        Ok(())
    }

    fn kill_window(&self, window: &str) -> Result<()> {
        self.run(&["kill-window", "-t", window])?;
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
            self.create_custom_layout(name, cwd, layout_def, config)?
        } else {
            match config.layout.as_str() {
                "minimal" => self.create_minimal_layout(name, cwd)?,
                "editor-only" => self.create_editor_only_layout(name, cwd)?,
                _ => self.create_default_layout(name, cwd)?,
            }
        };

        // Disable bell and activity monitoring per window so build-script output
        // in terminal panes doesn't false-positive flash the tab (F7b).
        let _ = self.run(&[
            "set-window-option",
            "-t",
            &dev.window_id,
            "monitor-bell",
            "off",
        ]);
        let _ = self.run(&[
            "set-window-option",
            "-t",
            &dev.window_id,
            "monitor-activity",
            "off",
        ]);

        Ok(dev)
    }

    fn has_window(&self, window: &str) -> bool {
        self.run(&["has-session", "-t", window]).is_ok()
    }

    fn select_window(&self, window: &str) -> Result<()> {
        self.run(&["select-window", "-t", window])?;
        Ok(())
    }

    fn find_window_id_by_name(&self, name: &str) -> Option<String> {
        let output = self
            .run(&["list-windows", "-a", "-F", "#{window_id} #{window_name}"])
            .ok()?;
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
        self.run(&["display-message", "-p", "-t", pane_id, "1"])
            .is_ok()
    }

    fn set_user_option(
        &self,
        scope: OptionScope,
        target: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        check_target(scope, target)?;
        let flag = match scope {
            OptionScope::Window => "-w",
            OptionScope::Pane => "-p",
        };
        self.run(&["set-option", flag, "-t", target, key, value])?;
        Ok(())
    }

    fn unset_user_option(&self, scope: OptionScope, target: &str, key: &str) -> Result<()> {
        check_target(scope, target)?;
        let flag = match scope {
            OptionScope::Window => "-wu",
            OptionScope::Pane => "-pu",
        };
        // Unsetting an option that was never set exits non-zero on some tmux
        // versions; that is not a failure for our purposes.
        let _ = self.run(&["set-option", flag, "-t", target, key]);
        Ok(())
    }

    fn show_user_option(
        &self,
        scope: OptionScope,
        target: &str,
        key: &str,
    ) -> Result<Option<String>> {
        check_target(scope, target)?;
        let flag = match scope {
            OptionScope::Window => "-wqv",
            OptionScope::Pane => "-pqv",
        };
        let out = self.run(&["show-options", flag, "-t", target, key])?;
        Ok(Some(out).filter(|s| !s.is_empty()))
    }

    fn run_shell_bg(&self, cmd: &str) -> Result<()> {
        self.run(&["run-shell", "-b", cmd])?;
        Ok(())
    }

    fn snapshot(&self) -> Result<TmuxSnapshot> {
        let stamp_fmt = snapshot::stamp_format();
        let stamp_out = self.run(&["display-message", "-p", &stamp_fmt])?;
        let stamp = snapshot::parse_stamp(&stamp_out)
            .ok_or_else(|| MeldrError::Tmux(format!("unparseable server stamp: {stamp_out:?}")))?;

        // Two queries, so that each field whose value could contain the separator
        // is the last thing on its line. See `snapshot::pane_format`.
        let pane_fmt = snapshot::pane_format();
        let panes_out = self.run_raw(&["list-panes", "-a", "-F", &pane_fmt])?;
        let cwd_fmt = snapshot::cwd_format();
        let cwds_out = self.run_raw(&["list-panes", "-a", "-F", &cwd_fmt])?;
        let panes = snapshot::parse_list_panes(&panes_out, &cwds_out);

        // A `$TMUX` naming a different server pid than the one that just answered
        // means the variable outlived its server — the socket path was reused. The
        // snapshot is still good; anything derived from the environment is not.
        let env_stale = matches!(self.env_server_pid(), Some(pid) if pid != stamp.pid);

        Ok(TmuxSnapshot {
            stamp,
            panes,
            env_stale,
        })
    }

    fn list_window_panes(&self, window: &str) -> Result<Vec<(u32, String)>> {
        check_target(OptionScope::Window, window)?;
        let out = self.run(&["list-panes", "-t", window, "-F", "#{pane_index} #{pane_id}"])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let (idx, id) = l.trim().split_once(' ')?;
                Some((idx.parse().ok()?, id.to_string()))
            })
            .collect())
    }

    fn window_pane_options(&self, window: &str, key: &str) -> Result<Vec<(String, String)>> {
        check_target(OptionScope::Window, window)?;
        let fmt = format!("#{{pane_id}} #{{{key}}}");
        let out = self.run(&["list-panes", "-t", window, "-F", &fmt])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let l = l.trim_end();
                let (id, value) = l.split_once(' ').unwrap_or((l, ""));
                is_pane_id(id).then(|| (id.to_string(), value.to_string()))
            })
            .collect())
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

/// A `TmuxOps` implementation for unit tests.
///
/// It records calls, but it also *stores* user options, so code that writes an
/// option and then reads it back (the window-status aggregate) behaves as it would
/// against a real server. Target validation matches `RealTmux` exactly: an empty or
/// positional target is refused rather than recorded, so a test asserting on
/// `set_calls` fails when the code under test would have flashed the wrong thing.
#[cfg(test)]
pub struct RecordingTmux {
    /// Pane IDs that exist. All others return false from `pane_exists`.
    pub live_panes: Vec<String>,
    /// Recorded `set_user_option` calls: `(scope, target, key, value)`.
    pub set_calls: Mutex<Vec<(OptionScope, String, String, String)>>,
    /// Recorded `unset_user_option` calls: `(scope, target, key)`.
    pub unset_calls: Mutex<Vec<(OptionScope, String)>>,
    /// Recorded `run_shell_bg` calls.
    pub bg_calls: Mutex<Vec<String>>,
    /// Panes returned by `snapshot()` / used by `window_pane_options()`.
    pub panes: Vec<snapshot::PaneRow>,
    pub stamp: snapshot::ServerStamp,
    pub env_stale: bool,
    /// When set, any tmux call whose first argument contains this substring fails.
    pub fail_on: Option<String>,
    /// Number of calls refused because the target was empty or positional.
    pub refused_targets: Mutex<Vec<String>>,
    /// Live option store: `(scope, target, key) -> value`.
    opts: Mutex<std::collections::HashMap<(OptionScope, String, String), String>>,
}

#[cfg(test)]
impl RecordingTmux {
    pub fn new(live_panes: Vec<String>) -> Self {
        Self {
            live_panes,
            set_calls: Mutex::new(Vec::new()),
            unset_calls: Mutex::new(Vec::new()),
            bg_calls: Mutex::new(Vec::new()),
            panes: Vec::new(),
            stamp: snapshot::ServerStamp {
                pid: 1000,
                start_time: 1,
            },
            env_stale: false,
            fail_on: None,
            refused_targets: Mutex::new(Vec::new()),
            opts: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Populate the snapshot. Panes listed here are automatically "live".
    pub fn with_panes(mut self, panes: Vec<snapshot::PaneRow>) -> Self {
        for p in &panes {
            if !self.live_panes.contains(&p.pane_id) {
                self.live_panes.push(p.pane_id.clone());
            }
        }
        self.panes = panes;
        self
    }

    pub fn with_stamp(mut self, pid: u32, start_time: u64) -> Self {
        self.stamp = snapshot::ServerStamp { pid, start_time };
        self
    }

    pub fn with_env_stale(mut self, stale: bool) -> Self {
        self.env_stale = stale;
        self
    }

    /// Make tmux calls fail — used to prove failures surface instead of being eaten.
    pub fn failing(mut self, what: &str) -> Self {
        self.fail_on = Some(what.to_string());
        self
    }

    /// Read back a stored option, as a real server would report it.
    pub fn opt(&self, scope: OptionScope, target: &str, key: &str) -> Option<String> {
        self.opts
            .lock()
            .unwrap()
            .get(&(scope, target.to_string(), key.to_string()))
            .cloned()
    }

    fn guard(&self, scope: OptionScope, target: &str) -> Result<()> {
        if let Err(e) = check_target(scope, target) {
            self.refused_targets
                .lock()
                .unwrap()
                .push(target.to_string());
            return Err(e);
        }
        Ok(())
    }

    fn fail_if(&self, what: &str) -> Result<()> {
        match &self.fail_on {
            Some(f) if what.contains(f.as_str()) => {
                Err(MeldrError::Tmux(format!("injected failure for {what}")))
            }
            _ => Ok(()),
        }
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
    fn set_user_option(
        &self,
        scope: OptionScope,
        target: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        self.guard(scope, target)?;
        self.fail_if("set-option")?;
        self.set_calls.lock().unwrap().push((
            scope,
            target.to_string(),
            key.to_string(),
            value.to_string(),
        ));
        self.opts.lock().unwrap().insert(
            (scope, target.to_string(), key.to_string()),
            value.to_string(),
        );
        Ok(())
    }
    fn unset_user_option(&self, scope: OptionScope, target: &str, key: &str) -> Result<()> {
        self.guard(scope, target)?;
        self.unset_calls
            .lock()
            .unwrap()
            .push((scope, format!("{target} {key}")));
        self.opts
            .lock()
            .unwrap()
            .remove(&(scope, target.to_string(), key.to_string()));
        Ok(())
    }
    fn show_user_option(
        &self,
        scope: OptionScope,
        target: &str,
        key: &str,
    ) -> Result<Option<String>> {
        self.guard(scope, target)?;
        Ok(self.opt(scope, target, key))
    }
    fn run_shell_bg(&self, cmd: &str) -> Result<()> {
        self.fail_if("run-shell")?;
        self.bg_calls.lock().unwrap().push(cmd.to_string());
        Ok(())
    }
    fn snapshot(&self) -> Result<TmuxSnapshot> {
        self.fail_if("list-panes")?;
        Ok(TmuxSnapshot {
            stamp: self.stamp,
            panes: self.panes.clone(),
            env_stale: self.env_stale,
        })
    }
    fn list_window_panes(&self, window: &str) -> Result<Vec<(u32, String)>> {
        self.guard(OptionScope::Window, window)?;
        Ok(self
            .panes
            .iter()
            .filter(|p| p.window_id == window)
            .enumerate()
            .map(|(i, p)| (i as u32, p.pane_id.clone()))
            .collect())
    }
    fn window_pane_options(&self, window: &str, key: &str) -> Result<Vec<(String, String)>> {
        self.guard(OptionScope::Window, window)?;
        Ok(self
            .panes
            .iter()
            .filter(|p| p.window_id == window)
            .map(|p| {
                let v = self
                    .opt(OptionScope::Pane, &p.pane_id, key)
                    .unwrap_or_default();
                (p.pane_id.clone(), v)
            })
            .collect())
    }
}

#[cfg(test)]
mod tmux_tests {
    use super::*;

    fn row(pane: &str, win: &str) -> snapshot::PaneRow {
        snapshot::PaneRow {
            pane_pid: 1,
            pane_id: pane.to_string(),
            window_id: win.to_string(),
            session_id: "$0".to_string(),
            cwd: std::path::PathBuf::from("/w"),
            current_command: "zsh".to_string(),
            window_name: "w".to_string(),
        }
    }

    #[test]
    fn refuses_empty_window_target() {
        // The exact shape of the original bug: `set-option -w -t ''` exits 0 on a
        // real server and lands on whatever window is focused.
        let t = RecordingTmux::new(vec![]);
        let err = t
            .set_user_option(OptionScope::Window, "", "@cc_status", "done")
            .unwrap_err();
        assert!(err.to_string().contains("refusing to target"));
        assert!(t.set_calls.lock().unwrap().is_empty());
        assert_eq!(t.refused_targets.lock().unwrap().len(), 1);
    }

    #[test]
    fn refuses_positional_pane_target() {
        let t = RecordingTmux::new(vec![]);
        assert!(
            t.set_user_option(OptionScope::Pane, "@1.0", "@cc_pane_status", "done")
                .is_err()
        );
        assert!(t.set_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn refuses_scope_mismatch() {
        let t = RecordingTmux::new(vec![]);
        assert!(
            t.set_user_option(OptionScope::Window, "%1", "@cc_status", "done")
                .is_err(),
            "a pane id is not a window target"
        );
        assert!(
            t.set_user_option(OptionScope::Pane, "@1", "@cc_pane_status", "done")
                .is_err(),
            "a window id is not a pane target"
        );
    }

    #[test]
    fn options_round_trip_and_unset() {
        let t = RecordingTmux::new(vec!["%1".into()]);
        t.set_user_option(OptionScope::Pane, "%1", "@cc_pane_status", "waiting")
            .unwrap();
        assert_eq!(
            t.show_user_option(OptionScope::Pane, "%1", "@cc_pane_status")
                .unwrap()
                .as_deref(),
            Some("waiting")
        );
        t.unset_user_option(OptionScope::Pane, "%1", "@cc_pane_status")
            .unwrap();
        assert_eq!(
            t.show_user_option(OptionScope::Pane, "%1", "@cc_pane_status")
                .unwrap(),
            None
        );
    }

    #[test]
    fn window_pane_options_reports_unset_as_empty() {
        let t = RecordingTmux::new(vec![]).with_panes(vec![row("%1", "@1"), row("%2", "@1")]);
        t.set_user_option(OptionScope::Pane, "%1", "@cc_pane_status", "done")
            .unwrap();
        let got = t.window_pane_options("@1", "@cc_pane_status").unwrap();
        assert_eq!(
            got,
            vec![
                ("%1".to_string(), "done".to_string()),
                ("%2".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn injected_failure_surfaces() {
        let t = RecordingTmux::new(vec!["%1".into()]).failing("set-option");
        assert!(
            t.set_user_option(OptionScope::Pane, "%1", "@cc_pane_status", "done")
                .is_err()
        );
    }

    #[test]
    fn check_target_accepts_real_ids() {
        assert!(check_target(OptionScope::Window, "@12").is_ok());
        assert!(check_target(OptionScope::Pane, "%7").is_ok());
    }
}
