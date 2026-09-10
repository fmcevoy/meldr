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

/// Field separator for `list-panes -F`.
///
/// It has to be a *printable* character: tmux 3.3a rewrites control characters in
/// format output to `_`, so a tab or an ASCII unit separator silently collapses
/// into the data (3.6b passes them through, which is how that went unnoticed
/// locally and was caught by the Debian test image).
///
/// A printable separator can legitimately occur in a path or a window name, so no
/// field that might contain one is ever parsed *between* two separators — see
/// `pane_format` and `cwd_format`.
pub const FS: char = '|';

/// Fields whose values cannot contain `FS`: a pid is digits, and tmux ids are
/// `%N` / `@N` / `$N`. The last field is a window name, taken as the remainder of
/// the line, so a `|` a user puts in a window name cannot shift anything.
///
/// `pane_current_command` sits among the constrained fields because it is a
/// process name; a `|` there is not achievable in practice.
pub fn pane_format() -> String {
    [
        "#{pane_pid}",
        "#{pane_id}",
        "#{window_id}",
        "#{session_id}",
        "#{pane_current_command}",
        "#{window_name}",
    ]
    .join(&FS.to_string())
}

/// Working directories get their own query so the path can be the remainder of
/// the line. Paths may contain any byte but `/` and NUL, `|` very much included.
pub fn cwd_format() -> String {
    format!("#{{pane_id}}{FS}#{{pane_current_path}}")
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

/// Parse the output of `list-panes -a -F <pane_format()>`, filling in each pane's
/// directory from `cwds` (the output of `<cwd_format()>`).
///
/// Malformed lines are skipped rather than failing the whole snapshot — one weird
/// pane should not blind the resolver to every other pane.
pub fn parse_list_panes(text: &str, cwds: &str) -> Vec<PaneRow> {
    let cwd_by_pane = parse_cwds(cwds);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| parse_pane_line(l, &cwd_by_pane))
        .collect()
}

/// `pane_id` → working directory. The path is the remainder of the line, so a `|`
/// inside it is harmless.
pub fn parse_cwds(text: &str) -> std::collections::HashMap<String, PathBuf> {
    text.lines()
        .filter_map(|l| {
            let (pane, path) = l.split_once(FS)?;
            is_pane_id(pane).then(|| (pane.to_string(), PathBuf::from(path)))
        })
        .collect()
}

fn parse_pane_line(
    line: &str,
    cwd_by_pane: &std::collections::HashMap<String, PathBuf>,
) -> Option<PaneRow> {
    let mut it = line.splitn(6, FS);
    let pane_pid: u32 = it.next()?.trim().parse().ok()?;
    let pane_id = it.next()?.to_string();
    let window_id = it.next()?.to_string();
    let session_id = it.next()?.to_string();
    let current_command = it.next()?.to_string();
    // Window names may contain the separator; take the remainder of the line.
    let window_name = it.next().unwrap_or_default().to_string();

    if !is_pane_id(&pane_id) || !is_window_id(&window_id) {
        return None;
    }
    let cwd = cwd_by_pane.get(&pane_id).cloned().unwrap_or_default();
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

    /// One `pane_format()` line: pid, pane, window, session, command, name.
    fn line(fields: &[&str]) -> String {
        fields.join(&FS.to_string())
    }

    /// One `cwd_format()` line.
    fn cwd_line(pane: &str, cwd: &str) -> String {
        format!("{pane}{FS}{cwd}")
    }

    #[test]
    fn parses_a_full_row() {
        let panes = line(&[
            "13793",
            "%9",
            "@1",
            "$0",
            "claude.exe",
            "ws-meldr/improvement:",
        ]);
        let cwds = cwd_line("%9", "/home/u/ws/meldr");
        let rows = parse_list_panes(&panes, &cwds);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.pane_pid, 13793);
        assert_eq!(r.pane_id, "%9");
        assert_eq!(r.window_id, "@1");
        assert_eq!(r.session_id, "$0");
        assert_eq!(r.cwd, PathBuf::from("/home/u/ws/meldr"));
        assert_eq!(r.current_command, "claude.exe");
        // Colons and slashes in window names must survive: they are exactly what
        // makes a name unusable as a `-t` target, which is why we key on `@N`.
        assert_eq!(r.window_name, "ws-meldr/improvement:");
    }

    #[test]
    fn a_separator_in_the_window_name_is_harmless() {
        // The name is the remainder of the line, so a user's `|` cannot shift
        // fields. tmux 3.3a forced a printable separator, which makes this matter.
        let panes = line(&["1", "%0", "@0", "$0", "zsh", "left | right"]);
        let rows = parse_list_panes(&panes, &cwd_line("%0", "/tmp"));
        assert_eq!(rows[0].window_name, "left | right");
    }

    #[test]
    fn a_separator_in_the_path_is_harmless() {
        // Paths get their own query for this reason: `|` is legal in a filename.
        let panes = line(&["1", "%0", "@0", "$0", "zsh", "w"]);
        let rows = parse_list_panes(&panes, &cwd_line("%0", "/tmp/od|d/proj"));
        assert_eq!(rows[0].cwd, PathBuf::from("/tmp/od|d/proj"));
    }

    #[test]
    fn a_pane_with_no_cwd_line_still_parses() {
        let panes = line(&["1", "%0", "@0", "$0", "zsh", "w"]);
        let rows = parse_list_panes(&panes, "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cwd, PathBuf::new());
    }

    #[test]
    fn skips_malformed_rows_but_keeps_the_rest() {
        let good = line(&["1", "%0", "@0", "$0", "zsh", "w"]);
        let bad_pid = line(&["x", "%1", "@0", "$0", "zsh", "w"]);
        let text = format!("{good}\nnot-a-row\n\n{bad_pid}");
        let rows = parse_list_panes(&text, "");
        assert_eq!(rows.len(), 1, "only the well-formed row survives");
        assert_eq!(rows[0].pane_id, "%0");
    }

    #[test]
    fn rejects_rows_whose_ids_are_positional() {
        let text = line(&["1", "@0.0", "@0", "$0", "zsh", "w"]);
        assert!(parse_list_panes(&text, "").is_empty());
    }

    #[test]
    fn parse_cwds_ignores_non_pane_keys() {
        let map = parse_cwds(&format!("{}\n@0{FS}/nope\ngarbage", cwd_line("%3", "/a")));
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("%3"), Some(&PathBuf::from("/a")));
    }

    #[test]
    fn the_separator_is_printable() {
        // tmux 3.3a rewrites control characters in `-F` output to `_`, silently
        // merging fields. Locally (3.6b) they pass through, so this invariant has
        // to be asserted rather than observed.
        assert!(
            !FS.is_control(),
            "separator {FS:?} would be mangled by tmux 3.3a"
        );
        assert!(FS.is_ascii());
    }

    #[test]
    fn formats_contain_no_control_characters() {
        for fmt in [pane_format(), cwd_format(), stamp_format()] {
            assert!(
                !fmt.chars().any(char::is_control),
                "format {fmt:?} contains a control character"
            );
        }
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
        let panes = format!(
            "{}\n{}\n{}",
            line(&["10", "%0", "@0", "$0", "zsh", "one"]),
            line(&["11", "%1", "@1", "$0", "claude", "two"]),
            line(&["12", "%2", "@1", "$0", "zsh", "two"]),
        );
        let cwds = format!(
            "{}\n{}\n{}",
            cwd_line("%0", "/a"),
            cwd_line("%1", "/b"),
            cwd_line("%2", "/b")
        );
        let snap = TmuxSnapshot {
            stamp: ServerStamp {
                pid: 1,
                start_time: 2,
            },
            panes: parse_list_panes(&panes, &cwds),
            env_stale: false,
        };
        assert_eq!(snap.pane("%1").unwrap().window_id, "@1");
        assert_eq!(snap.pane("%1").unwrap().cwd, PathBuf::from("/b"));
        assert_eq!(snap.by_pid(12).unwrap().pane_id, "%2");
        assert_eq!(snap.panes_in_window("@1").count(), 2);
        assert_eq!(snap.window_name("@1"), "two");
        assert!(snap.pane("%9").is_none());
    }
}
