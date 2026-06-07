# Meldr Roadmap

## Vision

Meldr is the virtual monorepo for AI teams — keep repos separate, work as one. Unified worktree management, coordinated PRs, and first-class AI agent integration across multi-repo projects.

## Recently Completed

- [x] **Smart sync with conflict detection**
  Default strategy changed from `theirs` to `safe`. Pre-sync conflict detection via `git merge-tree --write-tree`. Dry-run mode, parallel fetch, sync snapshots with undo, per-package strategy overrides, selective sync (--only/--exclude), summary table, and sync logging.

- [x] **Status dashboard (`meldr status`)**
  Color-coded workspace dashboard showing per-package, per-worktree: branch name, clean/dirty state, ahead/behind counts vs remote, last commit summary, and sync state (synced/stale/conflict). Table format with color coding (green = clean/synced, yellow = dirty/stale, red = conflict/behind).

- [x] **Package groups & universal filtering**
  Tag packages into named groups (`groups = ["backend", "rust"]`) and target commands at subsets. `--group <name>` filters to packages with that tag. `--only` and `--exclude` flags extended to all commands: `sync`, `exec`, `worktree add`, `worktree remove`, `status`.

- [x] **`meldr pr` — cross-repo PR automation**
  `meldr pr create` creates coordinated, linked pull requests across all dirty packages in a worktree via `gh`. Auto-links PRs with cross-references in the body. Supports `--title`, `--body`, `--draft`. `meldr pr status` shows open/merged/closed state, CI status, and review state across packages.

- [x] **Hook system**
  User-defined hooks on workspace lifecycle events: `post_sync`, `post_worktree_create`, `pre_remove`, `post_pr`. Defined in `meldr.toml` at workspace level with optional per-package overrides. Hooks run in the package worktree directory; failures warn but don't block the parent operation.

- [x] **`meldr doctor`**
  Detect stale tmux windows, orphaned worktrees, missing packages, config validation. `--apply` flag to auto-fix. Subactions: `claude`, `worktrees`, `tmux`.

## Priority 1 — Wow & Differentiation

Features that make someone try meldr, gasp, and tell someone else. No other multi-repo tool touches these.

### 1.1 `meldr watch` — Live Agent Status in tmux

A daemon that detects when AI agents finish or need input, then signals the correct tmux window tab to flash — across all agents, not just Claude Code.

**The problem it solves:** You have 3 agent panes in a 9-pane meldr layout. Claude Code already blinks the window tab (via `Stop`/`Notification` hooks → `@cc_status`). But Cursor, Codex, Antigravity, and OpenCode are silent when they finish. You have to switch to each pane to check.

**What it does:**
- `meldr watch start` — starts a background daemon
- `meldr watch stop` / `meldr watch status`
- Monitors agent panes in all active meldr worktree windows via periodic `tmux capture-pane`
- Detects idle state per agent type (cursor prompt for shells, Cursor's status bar change, Codex's prompt return)
- Sets `@cc_status` on the window (using the same `done`/`waiting` values your tmux.conf already renders) and optionally plays a sound
- For Claude: delegates entirely to the existing `claude-notify.sh` hooks — no double-counting
- Writes per-pane state to `~/.cache/claude-agents/<session>.json` (same format as claude-notify.sh)
- Optional: adds a compact per-worktree summary to `status-right` (e.g. `auth: ✓⏳✓`)

**meldr.toml config:**
```toml
[settings]
watch_poll_ms = 2000        # how often to sample panes (default: 2000)
watch_sound = true          # play sound on state change (default: true)
watch_status_right = false  # inject agent summary into tmux status-right
```

**Why meldr owns this:** Only meldr knows the layout — which panes are agent panes, which window is which worktree, which branch. Generic tmux watchers can't distinguish a "Cursor pane on feat/auth" from a random shell.

### 1.2 `meldr worktree pull-pr <number>` — PR to Isolated Environment

One command to check out any open GitHub PR across any of the workspace packages into a fresh coordinated worktree, with the full 9-pane layout ready to review.

**The problem it solves:** Reviewing a PR that touches multiple repos means manually checking out branches in each, opening editors, remembering context. `tree-me`'s single-repo `wt pr 1234` is the most-cited "wow" feature in the worktree space. Meldr can do the multi-repo version.

**What it does:**
1. `meldr worktree pull-pr 42` — scans all packages for a PR matching that number (by branch pattern or direct `gh pr view`)
2. For packages with a matching PR/branch, creates coordinated worktrees under the PR's branch name
3. Opens the 9-pane tmux layout for that worktree with an agent pre-loaded for review
4. `meldr worktree pull-pr 42 --package meldr` — scopes to a specific package's PR

**Also enables `meldr worktree pull-pr <url>`** — paste a GitHub PR URL, meldr figures out which package it belongs to.

**Why meldr owns this:** Single-repo tools create one worktree. If PR #42 in `meldr` depends on PR #18 in `meldr-web`, meldr can check out both together in one coordinated environment. Zero other tools do this.

### 1.3 Per-Worktree Runtime Isolation

Assigns deterministic port ranges and scoped environment variables per worktree, eliminating dev server collisions when running multiple worktrees simultaneously.

**The problem it solves:** The single most-cited practical blocker for parallel AI development. You can't run `npm run dev` in both `feat/auth` and `feat/payments` simultaneously — they fight over port 3000. Git worktrees solve file isolation; meldr is the only tool positioned to solve runtime isolation because it knows all active worktrees.

**What it does:**
- Each worktree gets a deterministic port offset: `base_port + (worktree_index × offset)` (default offset: 1000)
- `meldr worktree ports` — shows current assignment table
- Injects `MELDR_PORT_OFFSET`, `MELDR_PORT_BASE`, `MELDR_WORKTREE_INDEX` into each worktree's tmux session environment
- `post_worktree_create` hook can use these to template `.env.local`:
  ```toml
  [hooks]
  post_worktree_create = ["sed \"s/PORT=3000/PORT=$((3000 + $MELDR_PORT_OFFSET))/\" .env.example > .env.local"]
  ```
- `meldr.toml` config:
  ```toml
  [settings]
  port_base = 3000
  port_offset = 1000   # each worktree gets base + (index * offset)
  ```

**Why meldr owns this:** Requires knowing the full set of active worktrees and their ordering. No single-repo tool can do this.

## Priority 2 — Team Readiness & Polish

### 2.1 `meldr exec` with Live Parallel Dashboard

Transform `meldr exec` from "interleaved output dump" to a live per-package progress grid.

**What it does:** `meldr exec --progress -- cargo build` renders a live updating table:
```
  Package      Status        Duration   Last line
  ──────────────────────────────────────────────────────────
  meldr        ✓ done        12.3s
  meldr-web    ⠼ running     8.1s       > webpack building...
  meldr-api    ⠸ running     8.1s       > cargo compiling (47/112)
  meldr-auth   ✗ failed      6.8s       > error: missing dep
```
- Rows live-update in place (cursor controls, using the existing `console` crate)
- Failed rows show their last N output lines
- Summary line on completion: `4 done, 1 failed in 14.2s`

### 2.2 AGENTS.md Hierarchy Management

`AGENTS.md` is now the cross-editor agent instruction standard (Claude Code, Codex CLI, Cursor, Gemini, Copilot, Kiro, Windsurf all read it). Multi-repo teams need workspace-level instructions that flow into per-package files.

**What it does:**
- `meldr agents init` — scaffolds workspace-level `AGENTS.md` + per-package stubs with auto-injected context (package name, language, repo URL, workspace name)
- `meldr agents sync` — propagates workspace-level sections into each package's `AGENTS.md`; package sections take precedence
- `meldr agents check` — validates all packages have `AGENTS.md`, reports drift
- Respects `--group` filtering: `meldr agents sync --group rust` updates only Rust packages

**Strategic value:** Positions meldr as the coordination layer for the emerging agentic development standard. "Set up your workspace once, every agent in every repo gets consistent instructions."

### 2.3 Lock File (`meldr.lock`)

Pin exact commit SHAs per package for reproducible workspaces.

**Format:**
```toml
# meldr.lock — auto-generated, do not edit manually
[packages.meldr]
revision = "a1b2c3d4e5f6..."
locked_at = "2026-06-07T10:00:00Z"
```

**Commands:**
- `meldr lock` — snapshot current HEADs to `meldr.lock`
- `meldr sync --locked` — restore exact state from lock file
- Lock file is committed to version control

### 2.4 Manifest Sharing / URL-Based Init

One-command team onboarding.

- `meldr init --from <url>` clones a manifest repo and sets up workspace from shared `meldr.toml` (and `meldr.lock` if present)
- Enables: "clone this, run `meldr init --from <url>`, you're set up in 2 minutes"

### 2.5 `meldr.local.toml`

Gitignored local overrides for personal preferences.

- Same schema as `meldr.toml` `[settings]` section
- Use case: personal editor preference, agent choice, layout, port base — without polluting the shared manifest
- Added to `.gitignore` on `meldr init`

**Config precedence chain (highest to lowest):**
1. CLI flags
2. Environment variables (`MELDR_*`, `$EDITOR`, `$VISUAL`)
3. `meldr.local.toml` (workspace-local, gitignored) — NEW
4. `meldr.toml` `[settings]` (workspace, committed)
5. `~/.config/meldr/config.toml` (global)
6. Built-in defaults

### 2.6 Claude Code Plugin

Bundle meldr skills into an installable Claude Code plugin.

- `claude-plugin/` directory in the meldr repo with plugin manifest
- Bundles existing skills (`meldr-ops`, `meldr-workflow`, `verify-build`) plus new skills for `meldr watch`, `meldr pr`, `meldr agents`
- `claude plugin add <path-or-url>` installs it
- No MCP server needed — the CLI is the integration surface

## Priority 3 — Advanced Coordination

### 3.1 Atomic Merge Orchestration

Extend `meldr pr` with dependency-aware merge:
- Declare merge ordering in `meldr.toml` (`merge_after = ["meldr-shared"]`)
- `meldr pr merge` — merges PRs in declared order; halts if any PR fails CI or review
- Uses `gh pr merge --auto` under the hood

### 3.2 Cross-Repo CI Coordination

Enable testing the combined state of changes across repos before merge.
- `meldr pr` triggers a "combined CI" check
- Reference GitHub Action workflow that checks out all PR branches together
- Teams adopt the Action in a coordination repo

### 3.3 Pinned Revisions

Pin packages to specific commits, tags, or branches in `meldr.toml`:
```toml
[[package]]
name = "meldr"
pin = "v0.3.0"    # or commit SHA, or branch
```
Works with the lock file from P2 for full reproducibility.

## Priority 4 — Someday

- **Sparse checkout per worktree** — configure sparse checkout scoped to the files an agent needs, avoiding 10GB+ disk usage when scaling to many large-repo worktrees
- **Topological task ordering** — run `meldr exec` in dependency order if packages have inter-dependencies
- **Export/import workspace** — portable workspace definitions
- **Shallow clone support** — `meldr package add --depth 1` for large repos
- **Configurable directory names** — rename `packages/`, `worktrees/`, `.meldr/`
- **Mobile/remote monitoring** — access agent status from outside the machine (Cloudflare Tunnel / Tailscale approach)
