//! An isolated, self-cleaning tmux server for integration tests.
//!
//! Isolation is via `TMUX_TMPDIR` rather than `tmux -L`, deliberately: meldr under
//! test has to find the same server *without being told where it is*, because a
//! Claude background job runs with no `$TMUX` at all. Pointing meldr at a socket
//! explicitly would test a path production never takes.
//!
//! Two tmux quirks this fixture has to respect:
//!
//! - `TMUX_TMPDIR` must name a directory that **exists**; tmux silently falls back
//!   to `/tmp` when it does not, which would quietly point a test at the
//!   developer's own server.
//! - tmux 3.3a rewrites control characters in `-F` output to `_`, so formats here
//!   use a printable separator. Only tmux ids are ever separated by it.
//!
//! Every server gets a unique directory and is killed on `Drop`, so tests can run
//! in parallel and a panicking test cannot poison the next run — the previous
//! fixture shared one default-socket server between tests under fixed session
//! names, which leaked on failure and then collided.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static SEQ: AtomicU32 = AtomicU32::new(0);

pub struct TmuxServer {
    tmpdir: PathBuf,
    /// Kept so the directory outlives the server.
    _guard: tempfile::TempDir,
}

impl TmuxServer {
    /// Start a detached server no other test can see.
    pub fn new() -> Self {
        let guard = tempfile::TempDir::new().expect("tempdir");
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let tmpdir = guard
            .path()
            .join(format!("tmux-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&tmpdir).expect("tmux tmpdir");

        let server = Self {
            tmpdir,
            _guard: guard,
        };
        server.tmux(&["start-server"]).expect("start-server");
        server
    }

    /// `TMUX_TMPDIR` value that makes any `tmux` process address this server.
    pub fn tmpdir(&self) -> &Path {
        &self.tmpdir
    }

    /// Run a tmux command against this server, returning trimmed stdout.
    pub fn tmux(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new("tmux")
            .env("TMUX_TMPDIR", &self.tmpdir)
            .args(args)
            .output()
            .map_err(|e| format!("spawn tmux {args:?}: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    pub fn tmux_ok(&self, args: &[&str]) -> String {
        self.tmux(args)
            .unwrap_or_else(|e| panic!("tmux {args:?} failed: {e}"))
    }

    /// First session, created lazily. Returns `(window_id, pane_id)`.
    pub fn new_session(&self, cwd: &str) -> (String, String) {
        let out = self.tmux_ok(&[
            "new-session",
            "-d",
            "-s",
            "main",
            "-x",
            "200",
            "-y",
            "50",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}|#{pane_id}",
        ]);
        split_pair(&out)
    }

    /// A new window, which also becomes the active one.
    pub fn new_window(&self, cwd: &str) -> (String, String) {
        let out = self.tmux_ok(&[
            "new-window",
            "-t",
            "main",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{window_id}|#{pane_id}",
        ]);
        split_pair(&out)
    }

    /// Split `pane` without moving focus. Returns the new pane id.
    pub fn split(&self, pane: &str, cwd: &str) -> String {
        self.tmux_ok(&[
            "split-window",
            "-t",
            pane,
            "-d",
            "-c",
            cwd,
            "-P",
            "-F",
            "#{pane_id}",
        ])
    }

    pub fn active_window(&self) -> String {
        self.tmux_ok(&["display-message", "-p", "#{window_id}"])
    }

    pub fn select_window(&self, window: &str) {
        self.tmux_ok(&["select-window", "-t", window]);
    }

    pub fn window_of(&self, pane: &str) -> String {
        self.tmux_ok(&["display-message", "-p", "-t", pane, "#{window_id}"])
    }

    pub fn server_pid(&self) -> String {
        self.tmux_ok(&["display-message", "-p", "#{pid}"])
    }

    /// `@cc_status` on a window; empty string when unset.
    pub fn cc_status(&self, window: &str) -> String {
        self.tmux(&["show-options", "-wqv", "-t", window, "@cc_status"])
            .unwrap_or_default()
    }

    /// `@cc_pane_status` on a pane; empty string when unset.
    pub fn cc_pane_status(&self, pane: &str) -> String {
        self.tmux(&["show-options", "-pqv", "-t", pane, "@cc_pane_status"])
            .unwrap_or_default()
    }

    pub fn set_pane_option(&self, pane: &str, key: &str, value: &str) {
        self.tmux_ok(&["set-option", "-p", "-t", pane, key, value]);
    }

    /// Every `(pane_id, window_id)` on the server.
    pub fn panes(&self) -> Vec<(String, String)> {
        self.tmux_ok(&["list-panes", "-a", "-F", "#{pane_id}|#{window_id}"])
            .lines()
            .filter_map(|l| l.split_once('|'))
            .map(|(p, w)| (p.to_string(), w.to_string()))
            .collect()
    }

    pub fn break_pane(&self, pane: &str) -> String {
        self.tmux_ok(&["break-pane", "-d", "-s", pane]);
        self.window_of(pane)
    }
}

impl Default for TmuxServer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
    }
}

fn split_pair(out: &str) -> (String, String) {
    let (a, b) = out
        .trim()
        .split_once('|')
        .unwrap_or_else(|| panic!("expected two '|'-separated ids, got {out:?}"));
    (a.to_string(), b.to_string())
}

/// How the meldr binary should be invoked inside a test.
pub fn meldr_bin() -> PathBuf {
    // `CARGO_BIN_EXE_meldr` is set by cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_meldr"))
}

/// Environment every hook invocation in the tests needs.
///
/// `MELDR_CC_TIMEOUT` is large so a flash does not expire mid-assertion, and the
/// `MELDR_*` / `CLAUDE_CODE_CHILD_SESSION` variables are explicitly cleared: the
/// test runner may itself be a Claude session, and inheriting its variables would
/// silently change which resolver tier runs.
pub struct HookRun<'a> {
    server: &'a TmuxServer,
    home: PathBuf,
    env: Vec<(String, String)>,
}

impl<'a> HookRun<'a> {
    pub fn new(server: &'a TmuxServer, home: &Path) -> Self {
        Self {
            server,
            home: home.to_path_buf(),
            env: Vec::new(),
        }
    }

    /// Add an environment variable for subsequent runs. Borrowing rather than
    /// consuming so one runner can serve a whole test with per-call overrides.
    pub fn env(&mut self, key: &str, value: &str) -> &mut Self {
        self.env.retain(|(k, _)| k != key);
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    /// Run `meldr claude-hook <event>` from a shell nested `depth` levels inside
    /// `pane`, feeding `payload` on stdin, and wait for it to finish.
    ///
    /// The nesting is the point: Claude executes a hook through `sh -c`, so the
    /// process-tree walk must cross intermediate processes to reach the pane. A
    /// direct invocation would exercise a shorter chain than production ever does.
    pub fn in_pane(&self, pane: &str, depth: usize, event: &str, payload: &str) -> String {
        let dir = tempfile::TempDir::new().unwrap();
        let done = dir.path().join("done");
        let log = dir.path().join("log");

        let mut cmd = format!(
            "{} claude-hook {event}",
            shq(&meldr_bin().to_string_lossy())
        );
        for _ in 0..depth {
            cmd = format!("sh -c {}", shq(&cmd));
        }

        let mut exports = format!(
            "HOME={} MELDR_CC_TIMEOUT=300",
            shq(&self.home.to_string_lossy())
        );
        for (k, v) in &self.env {
            exports.push(' ');
            exports.push_str(&format!("{k}={}", shq(v)));
        }

        // `env -u` rather than relying on the pane's environment: the tmux server
        // inherited whatever started it.
        let full = format!(
            "printf %s {} | env -u CLAUDE_CODE_CHILD_SESSION -u MELDR_TMUX_PANE \
             -u MELDR_TMUX_WINDOW_ID -u MELDR_AGENT_SESSION {exports} {cmd} > {} 2>&1; \
             touch {}",
            shq(payload),
            shq(&log.to_string_lossy()),
            shq(&done.to_string_lossy()),
        );

        self.server
            .tmux_ok(&["send-keys", "-t", pane, &full, "Enter"]);
        wait_for(&done, Duration::from_secs(20));
        std::fs::read_to_string(&log).unwrap_or_default()
    }

    /// Run the hook *outside* any pane — the shape of a Claude background job,
    /// whose daemon has no pane ancestry and no `$TMUX`.
    pub fn detached(&self, event: &str, payload: &str, cwd: &str) -> std::process::Output {
        let mut c = Command::new(meldr_bin());
        c.arg("claude-hook")
            .arg(event)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("TMUX_TMPDIR", self.server.tmpdir())
            .env("MELDR_CC_TIMEOUT", "300")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("CLAUDE_CODE_CHILD_SESSION")
            .env_remove("MELDR_TMUX_PANE")
            .env_remove("MELDR_TMUX_WINDOW_ID")
            .env_remove("MELDR_AGENT_SESSION");
        for (k, v) in &self.env {
            c.env(k, v);
        }
        c.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = c.spawn().expect("spawn meldr");
        {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
        }
        child.wait_with_output().expect("meldr output")
    }
}

/// Shell-quote for the single-quoted form used above.
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn wait_for(path: &Path, timeout: Duration) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {}", path.display());
}

/// An empty, existing `TMUX_TMPDIR` with no server in it — the "not inside tmux,
/// and no server to find" condition. Must exist; see the note at the top of this
/// module about tmux's fallback to `/tmp`.
pub fn empty_tmux_tmpdir() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("tempdir")
}

/// Unix seconds, for building sidecar fixtures.
pub fn now_secs_for_test() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Poll until `f` is true, or panic. For the one genuinely time-dependent
/// behaviour (a flash expiring), so the test does not hard-code a sleep.
pub fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}
