use console::style;

/// Print a shell command to stderr in dim style for transparency.
/// `program` is e.g. "git" or "tmux", `args` are the arguments,
/// `cwd` is an optional working directory to show.
pub fn trace_cmd(program: &str, args: &[&str], cwd: Option<&str>) {
    let cmd = format!("{} {}", program, shell_join(args));
    let line = if let Some(dir) = cwd {
        format!("  $ {cmd} (in {dir})")
    } else {
        format!("  $ {cmd}")
    };
    eprintln!("{}", style(line).dim());
}

fn shell_join(args: &[&str]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(' ') || a.contains('$') || a.contains('{') || a.contains('#') {
                format!("'{a}'")
            } else {
                a.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
