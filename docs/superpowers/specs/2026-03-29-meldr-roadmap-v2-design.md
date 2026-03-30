# Meldr Roadmap v2 — Design Spec

## Vision

Meldr is the virtual monorepo for AI teams — keep repos separate, work as one. Unified worktree management, coordinated PRs, and first-class AI agent integration across multi-repo projects.

## Context

This roadmap restructure is informed by research across 10 dimensions: meldr's architecture, git worktree AI workflows, multi-repo tool landscape, AI agent orchestration patterns, tmux dev environments, cross-repo PR coordination, monorepo vs polyrepo tradeoffs, Claude Code extensibility, workspace manifest formats, and actual usage patterns.

### Key findings

1. Meldr is the only tool that creates coordinated worktrees across multiple repos for a single feature branch. No other multi-repo tool uses worktrees; no other worktree tool handles multiple repos.
2. The bottleneck in AI-assisted development has shifted from code generation to verification and coordination. Pre-merge cross-repo CI, atomic merge orchestration, and automated rollback are genuinely unsolved problems.
3. Claude Code and meldr are complementary: meldr is infrastructure (git, tmux, state), Claude Code is task execution (editing, skills, verification).
4. Table-stakes features (status dashboard, groups, hooks) are missing and block adoption more than any advanced feature.
5. The tmux + agent pane integration is meldr's core value proposition and should be preserved and extended, not abstracted away.

### Strategic direction

- **Own the substrate, not the agents.** Let Claude Code, Cursor, Conductor, and others manage agent lifecycles. Meldr manages the workspace those agents operate in.
- **Target audience**: Open source community. Features should prioritize first impressions, "wow factor," and extensibility.
- **Framing**: "Virtual monorepo for AI teams" — the tmux/agent features aren't secondary, they ARE the product.

## Priority 1 — Adoption & Differentiation

The items that make someone clone meldr, try it, and tell someone else about it. Mix of polish and headline features.

### 1.1 Status dashboard (`meldr status`)

Expand `meldr status` into a color-coded workspace dashboard.

**Displays per package per worktree:**
- Branch name
- Clean/dirty state
- Ahead/behind counts vs remote
- Last commit summary (short SHA + message)
- Sync state: synced (local matches remote tracking branch), stale (remote has commits not yet synced into the worktree), or conflict (known unresolved conflicts from last sync attempt)

**Design notes:**
- Benchmark is `gita ll` — meldr's version should be at least as good, plus worktree-aware
- Table format with color coding (green = clean/synced, yellow = dirty/stale, red = conflict/behind)
- "Stale" is determined by comparing local HEAD against remote tracking branch, not by time elapsed
- This is the feature people screenshot and share

### 1.2 Package groups & universal filtering

Allow tagging packages into named groups and targeting commands at subsets.

**Manifest changes:**
```toml
[[package]]
name = "meldr"
url = "https://github.com/fmcevoy/meldr"
groups = ["core", "rust"]

[[package]]
name = "meldr-web"
url = "https://github.com/fmcevoy/meldr-web.git"
groups = ["frontend", "node"]
```

Groups are tags on packages. The `--group` flag matches any package that has that tag. There is no separate "profiles" concept — named groups on packages are sufficient. If you want a shorthand for a set of packages, tag them all with the same group name.

**CLI changes:**
- `--group <name>` filters to packages with that group tag (can be repeated: `--group rust --group core`)
- `--only <pkg1>,<pkg2>` filters by package name (already exists for sync, extend everywhere)
- `--exclude <pkg>` excludes packages (already exists for sync, extend everywhere)
- These flags work on ALL commands: `sync`, `exec`, `worktree add`, `worktree remove`, `status`

**Implementation note:** The `--only`/`--exclude` flags currently only exist on `sync`. Extending to all commands requires threading a `PackageFilter` through every command handler. This is a non-trivial refactor touching `cli/` and `core/` modules — scope accordingly.

### 1.3 `meldr pr` (minimal cross-repo PRs)

Create coordinated, linked pull requests across all dirty packages in a worktree.

**Behavior:**
1. Detect which packages in the current worktree have uncommitted or unpushed changes
2. For each dirty package, push the branch and create a PR via `gh pr create`
3. Auto-link PRs with cross-references in the body:
   ```
   Part of coordinated change across ws-meldr:
   - fmcevoy/meldr#42
   - fmcevoy/meldr-web#18
   ```
4. Support `--title`, `--body`, `--draft` flags
5. Respect `--only`/`--exclude`/`--group` filtering

**Scope boundaries:**
- GitHub only (via `gh` CLI). GitLab is a stretch goal, not P1.
- No atomic merge coordination — PRs are independent once created. Atomic merge is P3.
- No cross-repo CI triggering — that's P3.

**Also includes `meldr pr status`:** Show the state of all PRs in the current worktree's changeset — open/merged/closed, CI status, review state. Pairs naturally with the status dashboard.

**This is the headline differentiator.** No other multi-repo tool creates linked PRs across repos.

### 1.4 Hook system

User-defined hooks on workspace lifecycle events.

**Supported hooks:**
- `post_sync` — after `meldr sync` completes
- `post_worktree_create` — after `meldr worktree add` creates worktrees
- `pre_remove` — before `meldr worktree remove` tears down worktrees
- `post_pr` — after `meldr pr` creates PRs

**Manifest format:**
```toml
[hooks]
post_sync = ["npm install", "cargo fetch"]
post_worktree_create = ["mise install", "npm install"]
```

Per-package hook overrides are specified inline on the `[[package]]` entry via an optional `hooks` table:

```toml
[[package]]
name = "meldr"
url = "https://github.com/fmcevoy/meldr"
[package.hooks]
post_worktree_create = ["cargo fetch"]

[[package]]
name = "meldr-web"
url = "https://github.com/fmcevoy/meldr-web.git"
[package.hooks]
post_worktree_create = ["npm install"]
```

When a per-package hook is set, it replaces (not appends to) the workspace-level hook for that event. This avoids the `[package.meldr.hooks]` syntax which is incompatible with `[[package]]` TOML arrays.

**Design notes:**
- Hooks run in the package directory (cd'd into the worktree)
- Hooks run sequentially per package, packages run in parallel
- Hook failures are reported but don't block the parent operation (warn, don't fail)
- This makes meldr extensible without bloating the core

## Priority 2 — Team Readiness & Integration

Make meldr viable for teams and deepen the AI integration story.

### 2.1 Lock file (`meldr.lock`)

Pin exact commit SHAs per package for reproducible workspaces.

**Format:**
```toml
# meldr.lock — auto-generated, do not edit manually
[packages.meldr]
revision = "a1b2c3d4e5f6..."
remote = "origin"
locked_at = "2026-03-29T10:00:00Z"

[packages.meldr-web]
revision = "f6e5d4c3b2a1..."
remote = "origin"
locked_at = "2026-03-29T10:00:00Z"
```

**Commands:**
- `meldr lock` — snapshot current HEADs to `meldr.lock`
- `meldr sync --locked` — restore exact state from lock file
- Lock file is committed to version control

### 2.2 Manifest sharing / URL-based init

One-command team onboarding.

- `meldr init --from <url>` clones a manifest repo and sets up workspace from shared `meldr.toml` (and `meldr.lock` if present)
- Enables: "clone this, run `meldr init --from <url>`, you're set up"

### 2.3 Richer status & `meldr doctor`

Build on P1's status dashboard:

- **Richer status**: branch tracking info, last sync timestamp, stale worktree warnings
- **`meldr doctor`**: detect stale tmux windows (stored ID but window gone), orphaned worktrees (directory exists but not in state), missing packages, config validation

### 2.4 `meldr.local.toml`

Gitignored local overrides for personal preferences.

- Same schema as `meldr.toml` `[settings]` section
- Use case: personal editor preference, agent choice, layout — without polluting the shared manifest
- Added to `.gitignore` on `meldr init`

**Full config precedence chain (highest to lowest):**
1. CLI flags (`--no-agent`, `--no-tabs`)
2. Environment variables (`MELDR_*`, `$EDITOR`, `$VISUAL`, `$SHELL`)
3. `meldr.local.toml` (workspace-local, gitignored) — NEW
4. `meldr.toml` `[settings]` (workspace, committed)
5. `~/.config/meldr/config.toml` (global)
6. Built-in defaults

### 2.5 Claude Code plugin

Bundle meldr skills into an installable Claude Code plugin.

- Create a Claude Code plugin directory structure with manifest and skill files
- Package existing skills (`meldr-ops`, `meldr-workflow`, `verify-build`) plus any new skills for `meldr pr`, `meldr status` etc.
- Distribute via the meldr repo — users install with `claude plugin add <path-or-url>`
- No MCP server needed — the CLI is the integration surface
- Concrete deliverable: a `claude-plugin/` directory in the meldr repo with the plugin manifest and bundled skills

## Priority 3 — Advanced Coordination

The hard problems. This is where meldr becomes true infrastructure for multi-repo development.

### 3.1 Atomic merge orchestration

Extend `meldr pr` with coordinated merge capabilities.

- Dependency ordering: declare that one package's PR must merge before another's
- `meldr pr merge` — merges all PRs in a changeset in declared order
- Halt-and-notify if any PR fails CI or review
- Uses `gh pr merge --auto` under the hood

### 3.2 Cross-repo CI coordination

Enable testing the combined state of changes across repos before merge.

- `meldr pr` can trigger a "combined CI" check
- Provide a reference GitHub Action workflow that checks out all PR branches together
- Teams adopt the Action in a coordination repo
- This is the hardest technical item on the roadmap

### 3.3 Pinned revisions

Pin packages to specific commits, tags, or branches in `meldr.toml`.

```toml
[[package]]
name = "meldr"
url = "https://github.com/fmcevoy/meldr"
pin = "v0.3.0"        # tag
# pin = "a1b2c3d"     # commit
# pin = "develop"     # branch
```

Works with lock file from P2 for full reproducibility.

## Priority 4 — Someday

Kept on the roadmap. Low priority, may be revisited if demand surfaces.

- **Topological task ordering** — run `meldr exec` in dependency order if packages have inter-dependencies
- **Export/import workspace** — portable workspace definitions
- **Shallow clone support** — `meldr package add --depth 1` for large repos
- **Configurable directory names** — rename `packages/`, `worktrees/`, `.meldr/`

## Backward Compatibility

All new fields added to `PackageEntry` (`groups`, `pin`, `hooks`) use `#[serde(default)]` so existing `meldr.toml` files continue to parse without changes. This is critical for manifest sharing (2.2) where manifests may be consumed by users on different meldr versions. New fields are always optional.

## Changes From Previous Roadmap

| Old Item | Change |
|----------|--------|
| Smart sync with conflict detection | Already completed, no change |
| Repo groups / subset targeting | Stays P1, expanded: universal filtering + groups |
| Pinned revisions / manifest locking | Split: lock file → P2, pinned revisions → P3 |
| Cross-repo PR automation | Stays P1, scoped to minimal linked PRs |
| Richer status dashboard | Split: basic dashboard → P1, richer + doctor → P2 |
| Manifest sharing / URL-based init | Stays P2 |
| Hook system | Promoted P2 → P1 |
| Directory names configurability | Demoted to P4 |
| Topological task ordering | Demoted to P4 |
| Export/import workspace | Demoted to P4 |
| Shallow clone support | Demoted to P4 |

**New additions:**
| Item | Priority | Rationale |
|------|----------|-----------|
| `meldr.local.toml` | P2 | Follows mise/Docker Compose pattern for personal overrides |
| Claude Code plugin | P2 | Bundle skills for automatic discovery, no MCP needed |
| Atomic merge orchestration | P3 | Extends `meldr pr` with dependency-aware merge |
| Cross-repo CI coordination | P3 | The hardest unsolved problem in multi-repo development |
