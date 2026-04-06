# Meldr Roadmap

## Vision

Meldr is the virtual monorepo for AI teams — keep repos separate, work as one. Unified worktree management, coordinated PRs, and first-class AI agent integration across multi-repo projects.

## Recently Completed

- [x] **Smart sync with conflict detection**
  Default strategy changed from `theirs` to `safe`. Pre-sync conflict detection via `git merge-tree --write-tree`. Dry-run mode, parallel fetch, sync snapshots with undo, per-package strategy overrides, selective sync (--only/--exclude), summary table, and sync logging.

## Priority 1 — Adoption & Differentiation

- [x] **Status dashboard (`meldr status`)**
  Color-coded workspace dashboard showing per-package, per-worktree: branch name, clean/dirty state, ahead/behind counts vs remote, last commit summary, and sync state (synced/stale/conflict). Benchmarked against `gita ll` — worktree-aware. Table format with color coding (green = clean/synced, yellow = dirty/stale, red = conflict/behind).

- [x] **Package groups & universal filtering**
  Tag packages into named groups (`groups = ["backend", "rust"]`) and target commands at subsets. `--group <name>` filters to packages with that tag. `--only` and `--exclude` flags extended to all commands: `sync`, `exec`, `worktree add`, `worktree remove`, `status`.

- [x] **`meldr pr` — cross-repo PR automation**
  `meldr pr create` creates coordinated, linked pull requests across all dirty packages in a worktree via `gh`. Auto-links PRs with cross-references in the body. Supports `--title`, `--body`, `--draft`. `meldr pr status` shows open/merged/closed state, CI status, and review state across packages.

- [x] **Hook system**
  User-defined hooks on workspace lifecycle events: `post_sync`, `post_worktree_create`, `pre_remove`, `post_pr`. Defined in `meldr.toml` at workspace level with optional per-package overrides. Hooks run in the package worktree directory; failures warn but don't block the parent operation.

## Priority 2 — Team Readiness & Integration

- [ ] **Lock file (`meldr.lock`)**
  Pin exact commit SHAs per package for reproducible workspaces. `meldr lock` snapshots current HEADs. `meldr sync --locked` restores exact state from lock file. Lock file committed to version control.

- [ ] **Manifest sharing / URL-based init**
  `meldr init --from <url>` clones a manifest repo and sets up workspace from shared `meldr.toml` (and `meldr.lock` if present). One-command team onboarding.

- [ ] **`meldr doctor` & richer status**
  Detect stale tmux windows, orphaned worktrees, missing packages, config validation. Richer status additions: branch tracking info, last sync timestamp, stale worktree warnings.

- [ ] **`meldr.local.toml`**
  Gitignored local overrides for personal preferences (editor, agent, layout) without polluting the shared manifest. Added to `.gitignore` on `meldr init`. Sits between env vars and workspace `[settings]` in the config precedence chain.

- [ ] **Claude Code plugin**
  Bundle meldr skills into an installable Claude Code plugin. `claude-plugin/` directory in the meldr repo with plugin manifest and bundled skills (`meldr-ops`, `meldr-workflow`, `verify-build`, plus skills for `meldr pr` and `meldr status`). Distribute via the meldr repo — no MCP server needed.

## Priority 3 — Advanced Coordination

- [ ] **Atomic merge orchestration**
  Extend `meldr pr` with dependency-aware merge: `meldr pr merge` merges all PRs in a changeset in declared order. Halt-and-notify if any PR fails CI or review. Uses `gh pr merge --auto` under the hood.

- [ ] **Cross-repo CI coordination**
  Enable testing the combined state of changes across repos before merge. `meldr pr` triggers a "combined CI" check via a reference GitHub Action workflow that checks out all PR branches together.

- [ ] **Pinned revisions**
  Pin packages to specific commits, tags, or branches in `meldr.toml` via a `pin` field. Works with the lock file from P2 for full reproducibility.

## Priority 4 — Someday

Kept on the roadmap. Low priority, may be revisited if demand surfaces.

- [ ] **Topological task ordering** — run `meldr exec` in dependency order if packages have inter-dependencies
- [ ] **Export/import workspace** — portable workspace definitions
- [ ] **Shallow clone support** — `meldr package add --depth 1` for large repos
- [ ] **Configurable directory names** — rename `packages/`, `worktrees/`, `.meldr/`
