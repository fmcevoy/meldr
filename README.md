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
| `--verbose` | Verbose output |
| `--quiet` | Suppress non-error output |

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

[[package]]
name = "frontend"
url = "https://github.com/org/frontend.git"
branch = "main"

[[package]]
name = "backend"
url = "https://github.com/org/backend.git"
```

## Tmux Integration

When running inside tmux (default mode), `meldr worktree add` will:

1. Capture your current tmux layout (or use a layout override from `meldr.toml`)
2. Create a new tmux window named `meldr:<branch>`
3. Split panes to match the captured layout
4. `cd` each pane into the corresponding worktree package directory
5. Optionally launch an AI coding agent in each pane

### Layout Override

```toml
[layout]
definition = "1bc3,168x45,0,0{112x45,0,0,55x45,113,0}"
panes = ["frontend", "backend"]
```

## Configuration Layering

Configuration is resolved in order (highest priority first):

1. **CLI flags** (`--no-agent`, `--no-tabs`)
2. **Environment variables** (`MELDR_AGENT`, `MELDR_MODE`)
3. **Workspace settings** (`meldr.toml [settings]`)
4. **Global config** (`~/.config/meldr/config.toml`)
5. **Built-in defaults** (agent=claude, mode=full, sync=rebase/theirs)

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
