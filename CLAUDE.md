# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Meldr

Workspace management tool for multi-repo projects with git worktrees and tmux integration. Written in Rust (edition 2024, requires 1.88+). Creates coordinated worktrees across multiple repos for a single feature branch, with AI agent panes in tmux.

## Commands

```bash
cargo build                                    # Build
cargo clippy --all-targets -- -D warnings      # Lint (treat warnings as errors)
cargo fmt --check                              # Format check
cargo test --bin meldr                         # Unit tests
./run-docker-tests.sh                          # Integration tests (requires Docker)
```

CI runs these in order: build + lint + format (parallel) → unit tests → integration tests.

## Architecture

Four layers, each behind a trait for testability:

- **`cli/`** — Clap-based command dispatch. One file per command (init, create, package, worktree, sync, status, exec, config_cmd, prompt_check). Commands call into `core/`.
- **`core/`** — Business logic. `workspace.rs` parses `meldr.toml`. `config.rs` resolves settings across 5 layers (CLI > env > workspace > global > defaults). `worktree.rs` is the largest file (~2100 lines) — handles worktree creation/removal, sync with conflict detection, parallel fetch, and tmux setup. `state.rs` manages `.meldr/state.json`. `sync_history.rs` handles snapshots for undo.
- **`git/`** — `GitOps` trait abstracts all git operations. `RealGit` implementation uses subprocess calls to the `git` CLI (no libgit2). Key operations: bare clone, worktree add/remove, fetch, rebase/merge with strategies, conflict detection via `git merge-tree --write-tree`.
- **`tmux/`** — `TmuxOps` trait abstracts tmux. Three preset layouts (default 6-pane, minimal 2-pane, editor-only) plus custom layouts via template variables. Creates windows with editor + agent + terminal panes.

`main.rs` handles argument rewriting (reversed `<action> <resource>` patterns) and routes to CLI handlers.

## Key design decisions

- **Bare clones in `packages/`, worktrees in `worktrees/<branch>/<pkg>/`** — enables multiple worktrees per repo without branch switching
- **Parallel fetch via Rayon, sequential sync** — maximizes fetch speed, enforces ordering on state changes
- **Pre-sync snapshots** — saved to `.meldr/sync-snapshots/` for undo without git reflog complexity
- **Conflict detection before rebase** — "safe" strategy uses `git merge-tree --write-tree` (Git 2.38+)
- **7 built-in agents** — claude, cursor, gemini, codex, opencode, pi, kiro (configurable via global config)

## Roadmap

Current roadmap spec: `docs/superpowers/specs/2026-03-29-meldr-roadmap-v2-design.md`
