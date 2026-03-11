use console::style;

/// Print a shell command to stderr in dim style for transparency.
/// `program` is e.g. "git" or "tmux", `args` are the arguments,
/// `cwd` is an optional working directory to show.
pub fn trace_cmd(program: &str, args: &[&str], cwd: Option<&str>) {
    let cmd = format!("{} {}", program, shell_join(args));
    let line = if let Some(dir) = cwd {
        format!("  $ {} (in {})", cmd, dir)
    } else {
        format!("  $ {}", cmd)
    };
    eprintln!("{}", style(line).dim());
}

fn shell_join(args: &[&str]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(' ') || a.contains('$') || a.contains('{') || a.contains('#') || a.contains('\'') {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_join_with_single_quotes() {
        let args = ["echo", "it's", "fine"];
        assert_eq!(shell_join(&args), "echo 'it'\\''s' fine");
    }

    #[test]
    fn test_shell_join_plain_args() {
        let args = ["git", "status", "--porcelain"];
        assert_eq!(shell_join(&args), "git status --porcelain");
    }

    #[test]
    fn test_shell_join_with_spaces() {
        let args = ["echo", "hello world"];
        assert_eq!(shell_join(&args), "echo 'hello world'");
    }
}
