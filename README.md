# Meldr

Workspace management tool for multi-repo projects with git worktrees and tmux integration.

## Install

```bash
# From the git repository (standard install)
cargo install --git https://github.com/fmcevoy/meldr.git --force
```

Or from a local checkout:

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
| `meldr init [-n <name>]` | | Initialize workspace in current directory |
| `meldr create <name>` | | One-shot: init + add packages + create worktree |
| `meldr package add <urls...>` | `pkg` | Clone and register packages |
| `meldr package remove <names...>` | `pkg` | Remove packages |
| `meldr package list` | `pkg` | List registered packages |
| `meldr worktree add <branch>` | `wt` | Create worktrees for all packages |
| `meldr worktree remove [branch]` | `wt` | Remove worktrees (auto-detects branch from cwd; checks for dirty state) |
| `meldr worktree open <branch>` | `wt` | Reopen tmux windows for an existing worktree; reuses the window if still alive |
| `meldr worktree list` | `wt` | List active worktrees |
| `meldr worktree scan [--prune]` | `wt` | Rebuild `.meldr/state.json` from on-disk git worktrees (self-healing) |
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
| `meldr doctor [claude\|worktrees\|tmux] [--apply]` | | Detect and optionally fix stale tmux windows, orphaned worktrees, config issues |
| `meldr prompt-check` | | Exit 0 if cwd's branch matches the worktree (for shell prompts) |

Subcommand names can be abbreviated (e.g. `meldr wt a` for `meldr worktree add`). Reversed `<action> <resource>` order is also accepted — `meldr add package <url>` is silently rewritten to `meldr package add <url>`.

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
| `--dir <name>` | `worktree remove` | Target a specific worktree directory name instead of auto-detecting from cwd |
| `--no-claude-prune` | `worktree remove` | Skip archiving and purging Claude Code state for the removed worktree |
| `--apply` | `doctor` | Apply fixes automatically instead of dry-run preview |
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
| `claude_prune` | `true` | `MELDR_CLAUDE_PRUNE` | Archive and purge Claude Code state on `worktree remove` (claude agent only) |

### Claude state cleanup on worktree remove

When the workspace `agent` is `claude`, `meldr worktree remove` automatically archives each removed worktree's Claude Code project state before deleting the worktree:

1. **Archive** — the `~/.claude/projects/<encoded-path>/` directory and any matching `tasks/` and `file-history/` session dirs are moved to `~/.claude/projects-archive/<timestamp>/`, preserving all transcripts for later recovery.
2. **Purge** — `claude project purge -y <path>` is run to remove the stale config entry from `~/.claude.json`.

Failures in either step are printed as `Warning:` messages and never block the worktree removal itself.

**Opt out:** pass `--no-claude-prune` to a single remove, set `MELDR_CLAUDE_PRUNE=false` per-invocation, or add `claude_prune = false` to the workspace `[settings]` or global `[defaults]`.

### Built-in AI agents

| Name | Default command | Description |
|------|-----------------|-------------|
| `claude` | `claude agents` | Anthropic Claude Code |
| `cursor` | `cursor agent --yolo` | Cursor AI agent |
| `gemini` | `gemini --yolo` | Google Gemini CLI |
| `codex` | `codex --approval-mode full-auto` | OpenAI Codex CLI |
| `opencode` | `opencode` | OpenCode CLI |
| `pi` | `pi` | Pi coding agent |
| `kiro` | `kiro-cli chat --trust-all-tools` | AWS Kiro CLI |
| `kiro-tui` | `kiro-cli --tui` | AWS Kiro (TUI mode) |
| `deepseek-tui` | `deepseek-tui` | DeepSeek TUI |
| `devin` | `devin --permission-mode bypass` | Devin for Terminal |
| `antigravity` | `agy` | Google Antigravity CLI |

Override any command in `~/.meldr/config.toml` under `[agents.<name>]`, or register custom agents by name — an unknown agent name is run verbatim as the shell command.

### Configuration layering (highest priority first)

1. **CLI flags** (`--no-agent`, `--no-tabs`)
2. **Environment variables** (`MELDR_*`, `$EDITOR`, `$VISUAL`, `$SHELL`)
3. **Workspace settings** (`meldr.toml [settings]`)
4. **Global config** (`~/.meldr/config.toml`)
5. **Built-in defaults**

## Tmux Integration

When running inside tmux, `meldr worktree add` (and `worktree open`) creates a development environment for each package.

### Leader package

The **leader package** is the package directory that AI agent panes `cd` into on launch — useful when you have multiple packages in a worktree but want agents focused on one. Set it via `--leader <pkg>`, the `MELDR_LEADER_PACKAGE` env var, or `leader_package` in workspace settings. If none is configured, meldr prompts interactively with a fuzzy picker.

### Layout presets

**`default`** — 9 panes: 3 agent (top 2/3) + editor + 5 terminals (bottom 1/3)
```
+-----------+-----------+-----------+
| agent  P0 | agent  P3 | agent  P4 |   all three cd into leader package + run agent
+-----------+-----------+-----------+
| editor P1 |  term  P5 |  term  P6 |   P1 runs $EDITOR and is focused on open
+-----------+-----------+-----------+
|  term  P2 |  term  P7 |  term  P8 |
+-----------+-----------+-----------+
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

Template variables: `{{window}}`, `{{cwd}}`, `{{editor}}`, `{{agent}}`, `{{pkg}}`, `{{branch}}`, `{{ws}}`.

Select with: `meldr config set layout my-layout`

### Claude Code tab-flash notifications

When the `agent` is `claude`, meldr can flash the tmux tab when a Claude session
finishes — showing `done` (green) or `waiting` (orange for `AskUserQuestion` /
`needs input:` prompts).

**1. Wire Claude Code hooks (one-time setup)**

```bash
meldr install-hooks
```

This writes `meldr claude-hook stop|notify|session-start` entries into
`~/.claude/settings.json`. On first install it also removes the legacy
`meldr-agent-notify.sh` bash script if present.

**2. Register the launcher wrapper in `~/.zshrc`**

```bash
# Print the snippet, then paste it into ~/.zshrc
meldr install-hooks --print-shell-snippet
```

The snippet wraps the `claude` command so meldr records the current tmux pane
before each `claude agents` invocation. This lets the resolver map new sessions
to the pane that launched them even when `TMUX_PANE` is ambiguous.

**3. Add tab-flash indicators to `~/.tmux.conf`**

```tmux
set -g window-status-format " #I:#W#{?#{==:#{@cc_status},done},#[bg=#f7768e fg=#1a1b26 bold]  ✓ ,#{?#{==:#{@cc_status},waiting},#[bg=#e0af68 fg=#1a1b26 bold]  ⏳ ,}} "
set -g window-status-current-format " #I:#W#{?#{==:#{@cc_status},done},#[bg=#f7768e fg=#1a1b26 bold]  ✓ ,#{?#{==:#{@cc_status},waiting},#[bg=#e0af68 fg=#1a1b26 bold]  ⏳ ,}} "

# Clear the indicator when you switch to the window/pane.
set-hook -g after-select-window 'set-option -wu @cc_status ; set-option -pu @cc_pane_status'
set-hook -g after-select-pane   'set-option -wu @cc_status ; set-option -pu @cc_pane_status'
```

**Verify setup**

```bash
meldr doctor hooks        # checks hooks + runs resolver self-test inside tmux
meldr doctor hooks --apply # auto-fixes missing hook entries
```

The resolver self-test covers:

- Tier 2 (env): `TMUX_PANE` resolves to a live pane
- Tier 5 (registry): launcher-entry cwd match works correctly
- Sibling non-match: `/tmp/foo` launcher does **not** match a session under `/tmp/foobar`
  (regression test for the path-component boundary bug)

**`meldr claude-hook` subcommands** (called automatically by Claude Code — not normally invoked by hand)

| Subcommand | Called by |
|---|---|
| `meldr claude-hook session-start` | Claude `SessionStart` hook |
| `meldr claude-hook stop` | Claude `Stop` hook |
| `meldr claude-hook notify` | Claude `Notification` hook |
| `meldr claude-hook register-launcher` | `claude()` shell wrapper before `claude agents` |

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
# agent = "claude"          # any built-in (claude, cursor, gemini, codex, opencode, pi, kiro, kiro-tui, deepseek-tui, devin, antigravity) or a custom command
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

# Override the tmux layout for this workspace (applies a raw tmux layout string
# and runs agent_command in each listed pane after cd-ing into the worktree)
# [layout]
# definition = "tiled"          # any tmux layout string
# panes = ["frontend", "api"]   # package names — one pane per entry
```

Per-package hook overrides (replace workspace-level hooks for that event):

```toml
[[package]]
name = "api"
url = "https://github.com/org/api.git"
[package.hooks]
post_worktree_create = ["cargo fetch"]
```

