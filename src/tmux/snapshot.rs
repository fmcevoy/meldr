//! A single consistent view of every pane on a tmux server.
//!
//! The old resolver asked tmux a question per tier — "does this pane exist?",
//! "what window is it in?" — and stitched the answers together with values read
//! from the environment. Those answers could disagree with each other and with
//! the environment, which is how a flash ended up on a window that had nothing to
//! do with the pane that produced it.
//!
//! Everything here comes from **one** `list-panes -a` call, so a `PaneRow` is
//! internally consistent by construction: a pane id always carries the window it
//! was actually in at snapshot time.
//!
//! The server is also stamped (`#{pid}`, `#{start_time}`). tmux hands out pane ids
//! (`%N`) from zero for each new server, so `%3` written down last month can name a
//! completely unrelated live pane today. Anything persisted has to record the stamp
//! and be rejected when it no longer matches.

use std::path::{Path, PathBuf};

/// Field separator for `list-panes -F`. ASCII unit separator: it cannot appear in
/// a path, command name, or window name, unlike a tab or space.
pub const FS: char = '\u{1f}';

/// The `list-panes -a -F` format string that produces one parseable `PaneRow`.
pub fn pane_format() -> String {
    [
        "#{pane_pid}",
        "#{pane_id}",
        "#{window_id}",
        "#{session_id}",
        "#{pane_current_path}",
        "#{pane_current_command}",
        "#{window_name}",
    ]
    .join(&FS.to_string())
}

/// The `display-message -p` format string that produces a `ServerStamp`.
pub fn stamp_format() -> String {
    format!("#{{pid}}{FS}#{{start_time}}")
}

/// One live pane, as tmux saw it at snapshot time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneRow {
    /// PID of the pane's foreground process group leader (normally its shell).
    pub pane_pid: u32,
    pub pane_id: String,
    pub window_id: String,
    pub session_id: String,
    pub cwd: PathBuf,
    pub current_command: String,
    pub window_name: String,
}

/// Identifies one run of one tmux server. Persisted references are only valid
/// while this still matches, because pane ids restart at `%0` per server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerStamp {
    pub pid: u32,
    pub start_time: u64,
}

/// Every pane on one server, plus that server's identity.
#[derive(Debug, Clone)]
pub struct TmuxSnapshot {
    pub stamp: ServerStamp,
    pub panes: Vec<PaneRow>,
    /// True when `$TMUX` named a server pid that is not the one we just talked to.
    /// The environment is then a leftover from a dead server and must not be
    /// trusted, even though the snapshot itself is fine.
    pub env_stale: bool,
}

impl TmuxSnapshot {
    pub fn pane(&self, pane_id: &str) -> Option<&PaneRow> {
        self.panes.iter().find(|p| p.pane_id == pane_id)
    }

    pub fn by_pid(&self, pid: u32) -> Option<&PaneRow> {
        self.panes.iter().find(|p| p.pane_pid == pid)
    }

    pub fn panes_in_window<'a>(&'a self, window_id: &'a str) -> impl Iterator<Item = &'a PaneRow> {
        self.panes.iter().filter(move |p| p.window_id == window_id)
    }

    pub fn window_name(&self, window_id: &str) -> String {
        self.panes_in_window(window_id)
            .next()
            .map(|p| p.window_name.clone())
            .unwrap_or_default()
    }
}

/// Parse the output of `list-panes -a -F <pane_format()>`.
///
/// Malformed lines are skipped rather than failing the whole snapshot — one weird
/// pane should not blind the resolver to every other pane.
pub fn parse_list_panes(text: &str) -> Vec<PaneRow> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_pane_line)
        .collect()
}

fn parse_pane_line(line: &str) -> Option<PaneRow> {
    let mut it = line.splitn(7, FS);
    let pane_pid: u32 = it.next()?.trim().parse().ok()?;
    let pane_id = it.next()?.to_string();
    let window_id = it.next()?.to_string();
    let session_id = it.next()?.to_string();
    let cwd = PathBuf::from(it.next()?);
    let current_command = it.next()?.to_string();
    // Window names may contain anything except the separator; take the remainder.
    let window_name = it.next().unwrap_or_default().to_string();

    if !is_pane_id(&pane_id) || !is_window_id(&window_id) {
        return None;
    }
    Some(PaneRow {
        pane_pid,
        pane_id,
        window_id,
        session_id,
        cwd,
        current_command,
        window_name,
    })
}

/// Parse the output of `display-message -p <stamp_format()>`.
pub fn parse_stamp(text: &str) -> Option<ServerStamp> {
    let (pid, start) = text.trim().split_once(FS)?;
    Some(ServerStamp {
        pid: pid.trim().parse().ok()?,
        start_time: start.trim().parse().unwrap_or(0),
    })
}

/// Parse `$TMUX`, which tmux sets to `<socket path>,<server pid>,<session>`.
///
/// Split from the right: a socket path may legitimately contain a comma, the two
/// trailing numeric fields may not.
pub fn parse_tmux_env(value: &str) -> Option<(PathBuf, u32)> {
    let mut parts = value.rsplitn(3, ',');
    let _session = parts.next()?;
    let pid: u32 = parts.next()?.trim().parse().ok()?;
    let socket = parts.next()?;
    if socket.is_empty() {
        return None;
    }
    Some((PathBuf::from(socket), pid))
}

/// True for a real tmux pane id (`%12`).
///
/// Deliberately rejects `@1.0` and `sess:1.0`. Those are *positional* targets that
/// tmux accepts happily but re-binds to whatever pane currently sits at that index,
/// so using one as a stored identity silently points at the wrong pane later.
pub fn is_pane_id(s: &str) -> bool {
    matches!(s.strip_prefix('%'), Some(rest) if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// True for a real tmux window id (`@3`).
pub fn is_window_id(s: &str) -> bool {
    matches!(s.strip_prefix('@'), Some(rest) if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Resolve `path` through symlinks when it exists, else normalise it lexically.
///
/// Needed because tmux reports `pane_current_path` as a realpath while a hook
/// payload's `cwd` may still be a symlinked spelling — on macOS `/tmp/x` vs
/// `/private/tmp/x`. A worktree that has just been removed no longer canonicalises,
/// hence the lexical fallback.
pub fn normalize(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when `ancestor` is `path` or contains it, comparing whole path components
/// so that `/home/u/fmcevoy` does not match `/home/u/fmcevoy_tools`.
pub fn is_ancestor_or_equal(ancestor: &Path, path: &Path) -> bool {
    normalize(path).starts_with(normalize(ancestor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(fields: &[&str]) -> String {
        fields.join(&FS.to_string())
    }

    #[test]
    fn parses_a_full_row() {
        let text = line(&[
            "13793",
            "%9",
            "@1",
            "$0",
            "/home/u/ws/meldr",
            "claude.exe",
            "ws-meldr/improvement:",
        ]);
        let rows = parse_list_panes(&text);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.pane_pid, 13793);
        assert_eq!(r.pane_id, "%9");
        assert_eq!(r.window_id, "@1");
        assert_eq!(r.cwd, PathBuf::from("/home/u/ws/meldr"));
        assert_eq!(r.current_command, "claude.exe");
        // Colons and slashes in window names must survive intact — they are what
        // makes a name unusable as a `-t` target, which is why we key on `@N`.
        assert_eq!(r.window_name, "ws-meldr/improvement:");
    }

    #[test]
    fn window_name_may_contain_spaces_and_separators_of_other_kinds() {
        let text = line(&["1", "%0", "@0", "$0", "/tmp", "zsh", "my window: a/b\tc"]);
        let rows = parse_list_panes(&text);
        assert_eq!(rows[0].window_name, "my window: a/b\tc");
    }

    #[test]
    fn skips_malformed_rows_but_keeps_the_rest() {
        let good = line(&["1", "%0", "@0", "$0", "/tmp", "zsh", "w"]);
        let text = format!(
            "{good}\nnot-a-row\n\n{}",
            line(&["x", "%1", "@0", "$0", "/", "z", "w"])
        );
        let rows = parse_list_panes(&text);
        assert_eq!(rows.len(), 1, "only the well-formed row survives");
        assert_eq!(rows[0].pane_id, "%0");
    }

    #[test]
    fn rejects_rows_whose_ids_are_positional() {
        let text = line(&["1", "@0.0", "@0", "$0", "/tmp", "zsh", "w"]);
        assert!(parse_list_panes(&text).is_empty());
    }

    #[test]
    fn parses_stamp() {
        assert_eq!(
            parse_stamp(&format!("6428{FS}1788989026")),
            Some(ServerStamp {
                pid: 6428,
                start_time: 1788989026
            })
        );
    }

    #[test]
    fn parses_tmux_env() {
        let (sock, pid) = parse_tmux_env("/private/tmp/tmux-501/default,6428,0").unwrap();
        assert_eq!(sock, PathBuf::from("/private/tmp/tmux-501/default"));
        assert_eq!(pid, 6428);
    }

    #[test]
    fn parses_tmux_env_with_comma_in_socket_path() {
        let (sock, pid) = parse_tmux_env("/tmp/od,d/sock,99,0").unwrap();
        assert_eq!(sock, PathBuf::from("/tmp/od,d/sock"));
        assert_eq!(pid, 99);
    }

    #[test]
    fn rejects_garbage_tmux_env() {
        assert!(parse_tmux_env("").is_none());
        assert!(parse_tmux_env("nonsense").is_none());
        assert!(parse_tmux_env(",1,0").is_none());
    }

    #[test]
    fn pane_id_shape() {
        assert!(is_pane_id("%0"));
        assert!(is_pane_id("%42"));
        assert!(!is_pane_id("%"));
        assert!(!is_pane_id(""));
        assert!(!is_pane_id("@1"));
        // The bug this guard exists for: a window.index target is not a pane id.
        assert!(!is_pane_id("@1.0"));
        assert!(!is_pane_id("1"));
        assert!(!is_pane_id("%1.0"));
    }

    #[test]
    fn window_id_shape() {
        assert!(is_window_id("@0"));
        assert!(is_window_id("@100"));
        assert!(!is_window_id("@"));
        assert!(!is_window_id(""));
        assert!(!is_window_id("%1"));
        assert!(!is_window_id("@1.0"));
        assert!(!is_window_id("ws/feat"));
    }

    #[test]
    fn ancestor_matching_is_component_aware() {
        // Regression for the ~/fmcevoy vs ~/fmcevoy_tools sibling-prefix bug.
        let base = Path::new("/home/u/fmcevoy");
        assert!(is_ancestor_or_equal(base, Path::new("/home/u/fmcevoy")));
        assert!(is_ancestor_or_equal(base, Path::new("/home/u/fmcevoy/sub")));
        assert!(!is_ancestor_or_equal(
            base,
            Path::new("/home/u/fmcevoy_tools/p")
        ));
        assert!(!is_ancestor_or_equal(base, Path::new("/home/u")));
    }

    #[test]
    fn normalize_strips_dot_components_when_path_is_absent() {
        let p = normalize(Path::new("/nonexistent-meldr/./a/b/../c"));
        assert_eq!(p, PathBuf::from("/nonexistent-meldr/a/c"));
    }

    #[test]
    fn snapshot_lookups() {
        let panes = parse_list_panes(&format!(
            "{}\n{}\n{}",
            line(&["10", "%0", "@0", "$0", "/a", "zsh", "one"]),
            line(&["11", "%1", "@1", "$0", "/b", "claude", "two"]),
            line(&["12", "%2", "@1", "$0", "/b", "zsh", "two"]),
        ));
        let snap = TmuxSnapshot {
            stamp: ServerStamp {
                pid: 1,
                start_time: 2,
            },
            panes,
            env_stale: false,
        };
        assert_eq!(snap.pane("%1").unwrap().window_id, "@1");
        assert_eq!(snap.by_pid(12).unwrap().pane_id, "%2");
        assert_eq!(snap.panes_in_window("@1").count(), 2);
        assert_eq!(snap.window_name("@1"), "two");
        assert!(snap.pane("%9").is_none());
    }
}
