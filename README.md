# Meldr

Workspace management tool for multi-repo projects with git worktrees and tmux integration.

## Prerequisites

- **Rust** (1.85+ for edition 2024)
- **Git** (2.20+)
- **tmux** (optional, for tab/pane management)

## Build & Test

```bash
cargo build
cargo test
```

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
| `meldr worktree remove <branch>` | `wt` | Remove worktrees (checks for dirty state) |
| `meldr worktree list` | `wt` | List active worktrees |
| `meldr status` | `st` | Show workspace dashboard |
| `meldr sync [branch]` | | Sync worktree with upstream |
| `meldr exec <command...>` | | Run command in all packages |
| `meldr config set <key> <value>` | | Set workspace config |
| `meldr config get <key>` | | Get workspace config |
| `meldr config list` | | Show effective configuration |

### Global flags

| Flag | Description |
|------|-------------|
| `--no-agent` | Skip agent launch in tmux panes |
| `--no-tabs` | Skip tmux window/pane creation entirely |

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

## Manifest Format (meldr.toml)

```toml
[workspace]
name = "my-project"

[settings]
# agent = "claude"          # "claude" | "cursor" | "none"
# mode = "full"             # "full" | "no-agent" | "no-tabs"
# sync_method = "rebase"    # "rebase" | "merge"
# sync_strategy = "theirs"  # "theirs" | "ours" | "manual"
# editor = "nvim ."         # editor command (or uses $EDITOR/$VISUAL)
# default_branch = "main"   # fallback branch for sync
# remote = "origin"         # default git remote
# shell = "sh"              # shell for exec (or uses $SHELL)
# layout = "default"        # "default" | "minimal" | "editor-only"
# window_name = "{ws}/{branch}:{pkg}"  # tmux window name template

[[package]]
name = "frontend"
url = "https://github.com/org/frontend.git"
branch = "main"
# remote = "origin"         # per-package remote override

[[package]]
name = "backend"
url = "https://github.com/org/backend.git"
```

## Configuration

### Settings

| Key | Default | Env vars | Description |
|-----|---------|----------|-------------|
| `agent` | `claude` | `MELDR_AGENT` | AI agent to launch |
| `mode` | `full` | `MELDR_MODE` | `full`, `no-agent`, or `no-tabs` |
| `sync_method` | `rebase` | | `rebase` or `merge` |
| `sync_strategy` | `theirs` | | `theirs`, `ours`, or `manual` |
| `editor` | `nvim .` | `MELDR_EDITOR`, `$VISUAL`, `$EDITOR` | Editor command for tmux panes |
| `default_branch` | `main` | `MELDR_DEFAULT_BRANCH` | Fallback branch for sync (auto-detected when possible) |
| `remote` | `origin` | `MELDR_REMOTE` | Default git remote name |
| `shell` | `sh` | `MELDR_SHELL`, `$SHELL` | Shell used by `meldr exec` |
| `layout` | `default` | `MELDR_LAYOUT` | Tmux layout preset |
| `window_name` | `{ws}/{branch}:{pkg}` | | Tmux window name template |

### Configuration Layering

Configuration is resolved in order (highest priority first):

1. **CLI flags** (`--no-agent`, `--no-tabs`)
2. **Environment variables** (`MELDR_*`, `$EDITOR`, `$VISUAL`, `$SHELL`)
3. **Workspace settings** (`meldr.toml [settings]`)
4. **Global config** (`~/.config/meldr/config.toml`)
5. **Built-in defaults**

### Global Config (~/.config/meldr/config.toml)

```toml
[defaults]
agent = "claude"
editor = "code ."
layout = "minimal"
shell = "/bin/zsh"

[agents.claude]
command = "claude"

[agents.cursor]
command = "cursor ."

# Custom tmux layout presets
[layouts.wide]
setup = [
  "split-window -t {{window}}.0 -h -p 30 -c {{cwd}} -P -F '#{pane_id}'",
  "select-pane -t {{window}}.0",
]
editor_pane = 0
agent_pane = 1
```

## Tmux Integration

When running inside tmux (default mode), `meldr worktree add` creates a development environment for each package with editor, agent, and terminal panes.

### Built-in Layout Presets

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

### Custom Layouts

Define custom layouts in `~/.config/meldr/config.toml` using raw tmux commands:

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

Then select it: `meldr config set layout my-layout`

### Layout Override (per-workspace)

For fully custom tmux layouts using layout definitions:

```toml
[layout]
definition = "1bc3,168x45,0,0{112x45,0,0,55x45,113,0}"
panes = ["frontend", "backend"]
```

## Sync

```bash
# Sync current worktree (auto-detected from cwd)
meldr sync

# Sync specific branch
meldr sync feature-auth

# Sync all worktrees
meldr sync --all

# Use merge instead of rebase
meldr sync --merge

# Custom strategy
meldr sync --strategy ours
```

The sync command auto-detects the default branch from the remote when possible, falling back to the configured `default_branch` (default: `main`). Per-package `branch` and `remote` overrides in `meldr.toml` are respected.
