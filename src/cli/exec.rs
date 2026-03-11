use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use rayon::prelude::*;

use crate::core::config::EffectiveConfig;
use crate::core::workspace::{self, Manifest};
use crate::error::{MeldrError, Result};

enum OutputLine {
    Stdout(String, String),
    Stderr(String, String),
    Done(String, Option<i32>),
    Error(String, String),
}

pub fn run(
    workspace_root: &Path,
    cwd: &Path,
    command: &[String],
    config: &EffectiveConfig,
    interactive: bool,
) -> Result<()> {
    let manifest = Manifest::load(workspace_root)?;

    if manifest.packages.is_empty() {
        println!("No packages in workspace.");
        return Ok(());
    }

    let worktree_dir_name = workspace::detect_current_worktree_dir(workspace_root, cwd)
        .ok_or_else(|| {
            MeldrError::Config(
                "meldr exec must be run from within a worktree directory.".to_string(),
            )
        })?;

    let cmd_str = command.join(" ");

    let shell_args: Vec<String> = if interactive {
        vec!["-i".to_string(), "-c".to_string(), cmd_str]
    } else {
        vec!["-c".to_string(), cmd_str]
    };

    let worktree_base = workspace::worktrees_dir(workspace_root).join(&worktree_dir_name);

    let (tx, rx) = mpsc::channel::<OutputLine>();

    let packages: Vec<_> = manifest.packages.clone();
    let shell = config.shell.clone();

    thread::spawn(move || {
        packages.par_iter().for_each(|pkg| {
            let tx = tx.clone();
            let pkg_path = worktree_base.join(&pkg.name);
            let name = pkg.name.clone();

            let child = Command::new(&shell)
                .args(&shell_args)
                .current_dir(&pkg_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(OutputLine::Error(name, e.to_string()));
                    return;
                }
            };

            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();

            let stdout_name = name.clone();
            let stdout_tx = tx.clone();
            let stdout_handle = thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(l) => {
                            let _ = stdout_tx.send(OutputLine::Stdout(stdout_name.clone(), l));
                        }
                        Err(_) => break,
                    }
                }
            });

            let stderr_name = name.clone();
            let stderr_tx = tx.clone();
            let stderr_handle = thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    match line {
                        Ok(l) => {
                            let _ = stderr_tx.send(OutputLine::Stderr(stderr_name.clone(), l));
                        }
                        Err(_) => break,
                    }
                }
            });

            let _ = stdout_handle.join();
            let _ = stderr_handle.join();

            let status = child.wait().ok().and_then(|s| s.code());
            let _ = tx.send(OutputLine::Done(name, status));
        });
    });

    let pkg_count = manifest.packages.len();
    let mut completed = 0;

    for msg in rx {
        match msg {
            OutputLine::Stdout(name, line) => {
                println!("[{}] {}", name, line);
            }
            OutputLine::Stderr(name, line) => {
                eprintln!("[{}] {}", name, line);
            }
            OutputLine::Done(name, status) => {
                if let Some(code) = status
                    && code != 0
                {
                    eprintln!("[{}] exited with code {}", name, code);
                }
                completed += 1;
                if completed == pkg_count {
                    break;
                }
            }
            OutputLine::Error(name, err) => {
                eprintln!("[{}] failed to execute: {}", name, err);
                completed += 1;
                if completed == pkg_count {
                    break;
                }
            }
        }
    }

    Ok(())
}
