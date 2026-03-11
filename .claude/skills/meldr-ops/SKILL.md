# /meldr

Execute meldr CLI operations from natural language. Translate user intent into the correct meldr commands.

## Command Reference

### Workspace lifecycle

| Intent | Command |
|--------|---------|
| Initialize workspace | `meldr init` |
| Create workspace (one-shot) | `meldr create <name> -r <url> [-r <url>...] [-b <branch>] [-a <agent>]` |

### Package management (`pkg`)

| Intent | Command |
|--------|---------|
| Add packages | `meldr package add <url> [<url>...]` |
| Remove packages | `meldr package remove <name> [<name>...]` |
| List packages | `meldr package list` |

### Worktree management (`wt`)

| Intent | Command |
|--------|---------|
| Create worktrees for branch | `meldr worktree add <branch>` |
| Remove worktrees for branch | `meldr worktree remove <branch>` |
| List worktrees | `meldr worktree list` |

### Sync

| Intent | Command |
|--------|---------|
| Sync current worktree | `meldr sync` |
| Sync specific branch | `meldr sync <branch>` |
| Sync all worktrees | `meldr sync --all` |
| Dry run | `meldr sync --dry-run` |
| Sync subset of packages | `meldr sync --only <pkg1>,<pkg2>` |
| Exclude packages | `meldr sync --exclude <pkg>` |
| Use merge instead of rebase | `meldr sync --merge` |
| Override strategy | `meldr sync --strategy <safe\|theirs\|ours\|manual>` |
| Undo last sync | `meldr sync --undo` |

### Other

| Intent | Command |
|--------|---------|
| Show status | `meldr status` |
| Run command across packages | `meldr exec <command...>` |
| Set config | `meldr config set <key> <value>` |
| Get config | `meldr config get <key>` |
| List config | `meldr config list` |

### Global flags

- `--no-agent` — skip agent launch in tmux panes
- `--no-tabs` — skip tmux window/pane creation

## Directory Structure

```
<workspace>/
  meldr.toml              # Workspace manifest
  .meldr/
    state.json            # Runtime state
  packages/
    <pkg>/                # Cloned repos (main branch)
  worktrees/
    <branch>/
      <pkg>/              # Git worktrees
```

## Configuration keys

`agent`, `mode`, `sync_method`, `sync_strategy`, `editor`, `default_branch`, `remote`, `shell`, `layout`, `window_name`

## Manifest format (meldr.toml)

```toml
[workspace]
name = "my-project"

[settings]
agent = "claude"
mode = "full"
sync_method = "rebase"
sync_strategy = "safe"

[[package]]
name = "frontend"
url = "https://github.com/org/frontend.git"
branch = "main"
```

## Instructions

When the user makes a request:
1. Identify which meldr command(s) to run
2. Confirm the command if it's destructive (remove, sync with strategy override)
3. Execute the command(s)
4. Report the result
