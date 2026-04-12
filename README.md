# Meldr

Workspace management tool for multi-repo projects with git worktrees and tmux integration.

## Install

```bash
cargo install --path .
```

Ensure `~/.cargo/bin` is in your PATH.

## Quick Start

### One-shot workspace creation

```bash
meldr create my-project \
  -r https://github.com/org/frontend.git \
  -r https://github.com/org/backend.git \
  -b feature-auth \
  -a claude
```

### Step-by-step

```bash
mkdir my-project && cd my-project
meldr init
meldr package add https://github.com/org/frontend.git https://github.com/org/backend.git
meldr worktree add feature-auth
```

## Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `meldr init` | | Initialize workspace in current directory |
| `meldr create <name>` | | One-shot: init + add packages + create worktree |
| `meldr package add <urls...>` | `pkg` | Clone and register packages |
| `meldr package remove <names...>` | `pkg` | Remove packages |
| `meldr package list` | `pkg` | List registered packages |
| `meldr worktree add <branch>` | `wt` | Create worktrees for all packages |
| `meldr worktree remove [branch]` | `wt` | Remove worktrees (auto-detects branch from cwd; checks for dirty state) |
| `meldr worktree open <branch>` | `wt` | Reopen tmux windows for an existing worktree (e.g. after a crash) |
| `meldr worktree list` | `wt` | List active worktrees |
| `meldr status` | `st` | Show workspace dashboard |
| `meldr sync [branch]` | | Sync worktree with upstream |
| `meldr exec <command...>` | | Run command in every package's worktree directory |
| `meldr pr create` | | Create linked PRs across packages |
| `meldr pr status` | | Show PR state across packages |
| `meldr config set <key> <value>` | | Set workspace or global config |
| `meldr config get <key>` | | Get a config value |
| `meldr config unset <key>` | | Remove a config value |
| `meldr config list` | | Show effective configuration |
| `meldr config show` | | Show where each setting value comes from |
| `meldr prompt-check` | | Exit 0 if cwd's branch matches the worktree (for shell prompts) |

Subcommand names can be abbreviated (e.g. `meldr wt a` for `meldr worktree add`).

### Global flags

| Flag | Description |
|------|-------------|
| `--no-agent` | Skip agent launch in tmux panes |
| `--no-tabs` | Skip tmux window/pane creation entirely |

### Per-command flags

| Flag | Commands | Description |
|------|----------|-------------|
| `--only <pkgs>` | `sync`, `exec`, `worktree add`, `worktree remove`, `status`, `pr create`, `pr status` | Only include these packages (comma-separated) |
| `--exclude <pkgs>` | same as `--only` | Exclude these packages (comma-separated) |
| `--group <name>` | same as `--only` | Filter by package group (comma-separated, repeatable) |
| `--leader <pkg>` | `create`, `worktree add` | Package to `cd` into for the AI agent pane (prompts interactively if omitted) |
| `--force` | `worktree remove` | Remove even with uncommitted changes |
| `-i`, `--interactive` | `exec` | Launch an interactive shell so aliases and rc files are loaded |
| `--global` | `config set/get/unset/list` | Apply to `~/.meldr/config.toml` instead of workspace |
| `--merge` | `sync` | Use merge instead of rebase |
| `--strategy <s>` | `sync` | Override sync strategy (`safe`, `theirs`, `ours`, `manual`) |
| `--dry-run` | `sync` | Preview what sync would do without making changes |
| `--undo` | `sync` | Roll back to the pre-sync snapshot |
| `--all` | `sync` | Sync all active worktrees |
| `--draft` | `pr create` | Create PRs as drafts |

## Directory Layout

```
my-project/
  meldr.toml              # Workspace manifest
  .meldr/
    state.json            # Runtime state (tmux mappings)
  packages/
    frontend/             # Cloned repo (main branch)
    backend/
  worktrees/
    feature-auth/
      frontend/           # Git worktree
      backend/
```

## Sync

```bash
meldr sync                        # Sync current worktree (auto-detected from cwd)
meldr sync feature-auth           # Sync specific branch
meldr sync --all                  # Sync all worktrees
meldr sync --dry-run              # Preview what would happen
meldr sync --only frontend,auth   # Only sync specific packages
meldr sync --exclude legacy-api   # Exclude packages
meldr sync --merge                # Use merge instead of rebase
meldr sync --strategy theirs      # Override strategy
meldr sync --undo                 # Undo the last sync
```

### Sync strategies

| Strategy | Default | Behavior |
|----------|---------|----------|
| `safe` | Yes | Checks for conflicts before syncing. Refuses if conflicts detected. |
| `theirs` | | Auto-resolves conflicts in favor of upstream (`-Xtheirs`). |
| `ours` | | Auto-resolves conflicts in favor of local changes (`-Xours`). |
| `manual` | | Git stops on conflicts for manual resolution. |

### Safety features

- **Pre-sync snapshots** — HEAD SHAs saved to `.meldr/sync-snapshots/`; roll back with `meldr sync --undo`
- **Conflict detection** — `safe` strategy checks for conflicts before attempting sync (Git 2.38+)
- **Parallel fetch** — all package fetches run concurrently
- **Summary table** — color-coded post-sync status with ahead/behind counts
- **Sync log** — operations logged to `.meldr/sync-log.jsonl`

Auto-detects default branch from remote, falling back to configured `default_branch`. Per-package `branch`, `remote`, and `sync_strategy` overrides are respected.

## Configuration

| Key | Default | Env var | Description |
|-----|---------|---------|-------------|
| `agent` | `claude` | `MELDR_AGENT` | AI agent to launch (see built-ins below) |
| `mode` | `full` | `MELDR_MODE` | `full`, `no-agent`, or `no-tabs` |
| `sync_method` | `rebase` | | `rebase` or `merge` |
| `sync_strategy` | `safe` | | `safe`, `theirs`, `ours`, or `manual` |
| `editor` | `nvim .` | `MELDR_EDITOR`, `$VISUAL`, `$EDITOR` | Editor command |
| `default_branch` | `main` | `MELDR_DEFAULT_BRANCH` | Fallback branch for sync |
| `remote` | `origin` | `MELDR_REMOTE` | Default git remote |
| `shell` | `sh` | `MELDR_SHELL`, `$SHELL` | Shell for `meldr exec` |
| `layout` | `default` | `MELDR_LAYOUT` | Tmux layout preset |
| `window_name` | `{ws}/{branch}:{pkg}` | | Tmux window name template |
| `leader_package` | (none) | `MELDR_LEADER_PACKAGE` | Package the AI agent `cd`s into on launch |

### Built-in AI agents

| Name | Default command | Description |
|------|-----------------|-------------|
| `claude` | `claude --dangerously-skip-permissions` | Anthropic Claude Code |
| `cursor` | `cursor agent --yolo` | Cursor AI agent |
| `gemini` | `gemini --yolo` | Google Gemini CLI |
| `codex` | `codex --approval-mode full-auto` | OpenAI Codex CLI |
| `opencode` | `opencode` | OpenCode CLI |
| `pi` | `pi` | Pi coding agent |
| `kiro` | `kiro-cli chat --trust-all-tools` | AWS Kiro CLI |
| `kiro-tui` | `kiro-cli --tui` | AWS Kiro (TUI mode) |
| `deepseek-tui` | `deepseek-tui` | DeepSeek TUI |

Override any command in `~/.meldr/config.toml` under `[agents.<name>]`, or register custom agents by name — an unknown agent name is run verbatim as the shell command.

### Configuration layering (highest priority first)

1. **CLI flags** (`--no-agent`, `--no-tabs`)
2. **Environment variables** (`MELDR_*`, `$EDITOR`, `$VISUAL`, `$SHELL`)
3. **Workspace settings** (`meldr.toml [settings]`)
4. **Global config** (`~/.meldr/config.toml`)
5. **Built-in defaults**

## Tmux Integration

When running inside tmux, `meldr worktree add` creates a development environment for each package.

### Layout presets

**`default`** — 6 panes: editor + agent + 4 terminals
```
+-------------------+-----------+
|                   |           |
|    editor (0)     | agent (1) |
|                   |           |
+--------+----------+           |
| t1 (2) | t3 (4)   |           |
+--------+----------+           |
| t2 (3) | t4 (5)   |           |
+--------+----------+-----------+
```

**`minimal`** — 2 panes: editor + agent
```
+-------------------+-----------+
|                   |           |
|    editor (0)     | agent (1) |
|                   |           |
+-------------------+-----------+
```

**`editor-only`** — single pane
```
+-------------------------------+
|                               |
|          editor (0)           |
|                               |
+-------------------------------+
```

### Custom layouts

Define in `~/.meldr/config.toml`:

```toml
[layouts.my-layout]
setup = [
  "split-window -t {{window}}.0 -h -p 40 -c {{cwd}} -P -F '#{pane_id}'",
  "split-window -t {{window}}.0 -v -p 20 -c {{cwd}} -P -F '#{pane_id}'",
  "select-pane -t {{window}}.0",
]
editor_pane = 0
agent_pane = 1
```

Template variables: `{{window}}`, `{{cwd}}`, `{{editor}}`, `{{agent}}`.

Select with: `meldr config set layout my-layout`

---

## Development

### Prerequisites

- **Rust** 1.88+ (edition 2024)
- **Git** 2.20+ (2.38+ recommended for conflict detection)
- **tmux** (optional, for integration tests and tab/pane management)
- **Docker** (for integration tests)

### Build & Test

```bash
cargo build                    # Compile
cargo clippy --all-targets -- -D warnings  # Lint
cargo fmt --check              # Format check
cargo test --bin meldr          # Unit tests
./run-docker-tests.sh          # Integration tests (Docker)
```

### CI Pipeline

CI runs on every push to `main` and every pull request:

| Stage | Jobs | Gate |
|-------|------|------|
| 1 | `build`, `lint`, `format` (parallel) | — |
| 2 | `unit-tests` | Stage 1 passes |
| 3 | `integration-tests` (Docker) | Stage 2 passes |

All integration tests run inside Docker for consistent, isolated environments.

### Manifest format (meldr.toml)

```toml
[workspace]
name = "my-project"

[settings]
# agent = "claude"          # any built-in (claude, cursor, gemini, codex, opencode, pi, kiro, kiro-tui, deepseek-tui) or a custom command
# mode = "full"             # "full" | "no-agent" | "no-tabs"
# sync_method = "rebase"    # "rebase" | "merge"
# sync_strategy = "safe"    # "safe" | "theirs" | "ours" | "manual"
# editor = "nvim ."         # editor command (or uses $EDITOR/$VISUAL)
# default_branch = "main"   # fallback branch for sync
# remote = "origin"         # default git remote
# shell = "sh"              # shell for exec (or uses $SHELL)
# layout = "default"        # "default" | "minimal" | "editor-only" | custom layout name
# window_name = "{ws}/{branch}:{pkg}"  # tmux window name template
# leader_package = "frontend"          # package the AI agent cd's into on launch

[[package]]
name = "frontend"
url = "https://github.com/org/frontend.git"
branch = "main"
groups = ["frontend", "node"]
# remote = "origin"         # per-package remote override

[[package]]
name = "api"
url = "https://github.com/org/api.git"
groups = ["backend", "rust"]
# sync_strategy = "theirs"  # per-package strategy override

[hooks]
post_sync = ["npm install"]
post_worktree_create = ["mise install"]
# pre_remove = []
# post_pr = []
```

Per-package hook overrides (replace workspace-level hooks for that event):

```toml
[[package]]
name = "api"
url = "https://github.com/org/api.git"
[package.hooks]
post_worktree_create = ["cargo fetch"]
```

