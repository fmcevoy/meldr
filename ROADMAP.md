# Meldr Roadmap

## Priority 1 — Table Stakes (competitive parity)

- [ ] **Repo groups / subset targeting**
  Allow tagging packages into groups (e.g., `frontend`, `backend`) and targeting commands at subsets: `meldr exec --group frontend`, `meldr sync --only api,auth`. Every mature multi-repo tool supports this.

- [ ] **Pinned revisions / manifest locking**
  Let `meldr.toml` pin packages to specific commits, tags, or branches. Add `meldr lock` to snapshot current state and `meldr.lock` for reproducible workspace setup across teams.

- [ ] **Cross-repo PR automation**
  `meldr pr` creates coordinated, linked pull requests across all dirty packages in a worktree. Supports GitHub and GitLab. Reduces the pain of multi-repo feature branches.

- [ ] **Richer status dashboard**
  Expand `meldr status` to show: ahead/behind counts, sync state vs remote, last commit summary, branch tracking info — all in a color-coded table (similar to `gita ll`).

## Priority 2 — Safety & Usability

- [ ] **Smart sync with conflict detection**
  Warn or refuse when sync would cause non-trivial merges instead of silently applying `--strategy theirs`. Offer interactive resolution or at minimum surface a clear warning.

- [ ] **Manifest sharing / URL-based init**
  `meldr init --from <url>` clones a manifest repo and sets up the workspace from a shared `meldr.toml`. Enables team onboarding in one command.

- [ ] **Hook system**
  User-defined hooks on workspace events: post-sync, post-add, post-worktree-create. Useful for running `npm install`, applying patches, or triggering builds after operations.

## Priority 3 — Configurability (de-hardcode)

- [x] **Editor command** — configurable via `$EDITOR`/`$VISUAL` env, `MELDR_EDITOR` env, config setting, or `meldr config set editor`. Default: `"nvim ."`.

- [x] **Default branch detection** — auto-detects via `git symbolic-ref refs/remotes/origin/HEAD`. Falls back to per-package `branch`, then config `default_branch`, then `"main"`.

- [x] **Default remote name** — configurable per-package (`remote` field in `[[package]]`), globally via config, or `MELDR_REMOTE` env. Default: `"origin"`.

- [x] **Shell for exec** — respects `$SHELL` env, `MELDR_SHELL` env, or config setting. Default: `"sh"`.

- [x] **Tmux layout configuration** — three built-in presets (`default`, `minimal`, `editor-only`) plus custom layouts via tmux command snippets in global config. Selected via `layout` setting.

- [ ] **Directory names** — hardcoded `"packages"`, `"worktrees"`, `".meldr"` in `workspace.rs` and `state.rs`. Configurable in `[workspace]` section. *(deferred: highly invasive, requires threading config through all path helpers)*

- [x] **Extensible settings keys** — `VALID_SETTINGS_KEYS` expanded to include all new fields: `editor`, `default_branch`, `remote`, `shell`, `layout`, `window_name`.

- [x] **Status colors** — `console` crate already respects `NO_COLOR` env var. No code change needed.

- [x] **Window naming pattern** — configurable template via `window_name` setting. Variables: `{ws}`, `{branch}`, `{pkg}`. Default: `"{ws}/{branch}:{pkg}"`.

## Priority 4 — Nice to Have

- [ ] **Topological task ordering**
  If packages have inter-dependencies, `meldr exec` runs commands in dependency order. Optional dependency declaration in `meldr.toml`.

- [ ] **Export / import workspace**
  `meldr export` saves workspace definition (packages, groups, pins) to a portable file. `meldr import` restores it.

- [ ] **Shallow clone support**
  `meldr package add --depth 1` for faster initial cloning of large repositories.

---

## Design: Tmux Layout Configuration

### Problem
The current dev window layout is a hardcoded 6-pane arrangement (nvim + agent + 4 terminals) with fixed split percentages (35%, 30%, 50%). Users cannot adjust pane sizes, remove panes, or use different arrangements.

### Approach: Tmux Config Snippets

Rather than inventing a layout DSL, let users provide raw tmux commands. This is familiar to tmux users, infinitely flexible, and avoids meldr needing to model every possible layout.

### Config Schema

In `~/.config/meldr/config.toml` (global) or `meldr.toml` (per-workspace):

```toml
# Named layout presets
[layouts.default]
# Each step is a tmux command run against the window.
# {{window}} = window id, {{cwd}} = package worktree path
# Panes are numbered in creation order: first pane is 0 (created with the window).
setup = [
  # Split right for agent — full-height right column
  "split-window -t {{window}}.0 -h -p 35 -c {{cwd}}",
  # Split left pane below for terminal row
  "split-window -t {{window}}.0 -v -p 30 -c {{cwd}}",
  # Split terminal row below
  "split-window -t {{window}}.2 -v -p 50 -c {{cwd}}",
  # Split terminal row right (top)
  "split-window -t {{window}}.2 -h -p 50 -c {{cwd}}",
  # Split terminal row right (bottom)
  "split-window -t {{window}}.3 -h -p 50 -c {{cwd}}",
  # Focus nvim pane
  "select-pane -t {{window}}.0",
]

# Which panes get commands sent to them
[layouts.default.panes]
editor = { index = 0, command = "{{editor}} ." }
agent  = { index = 1, command = "{{agent}}" }
# Panes 2-5 are free terminals — no command sent

# A minimal layout for users who just want editor + agent
[layouts.minimal]
setup = [
  "split-window -t {{window}}.0 -h -p 40 -c {{cwd}}",
  "select-pane -t {{window}}.0",
]

[layouts.minimal.panes]
editor = { index = 0, command = "{{editor}} ." }
agent  = { index = 1, command = "{{agent}}" }
```

### Usage

```toml
# In meldr.toml or global config
[settings]
layout = "minimal"       # Use a named preset
editor = "nvim ."        # Or "code .", "hx .", etc.
```

### Template Variables

| Variable | Resolves to |
|----------|-------------|
| `{{window}}` | Tmux window ID (e.g., `@5`) |
| `{{cwd}}` | Package worktree path |
| `{{editor}}` | Configured editor command |
| `{{agent}}` | Configured agent command |
| `{{pkg}}` | Package name |
| `{{branch}}` | Worktree branch name |
| `{{ws}}` | Workspace name |

### Defaults

When no layout is specified, meldr uses the current 6-pane layout — behavior is unchanged. The current layout becomes a built-in preset called `"default"`.

### Why Raw Tmux Commands

1. **Zero learning curve** — tmux users already know the syntax
2. **Full power** — any tmux feature (send-keys, set-option, resize-pane) works
3. **Portable** — users can copy snippets from their existing tmux configs
4. **Debuggable** — users can test commands manually in tmux first
5. **No abstraction leaks** — we don't need to model pane trees, percentages, or directions
