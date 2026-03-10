# Meldr Roadmap

## Recently Completed

- [x] **Smart sync with conflict detection** (was Priority 2)
  Default strategy changed from `theirs` to `safe`. Pre-sync conflict detection via `git merge-tree --write-tree`. Dry-run mode, parallel fetch, sync snapshots with undo, per-package strategy overrides, selective sync (--only/--exclude), summary table, and sync logging.

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

- [x] **Smart sync with conflict detection**
  Warn or refuse when sync would cause non-trivial merges instead of silently applying `--strategy theirs`. Offer interactive resolution or at minimum surface a clear warning.

- [ ] **Manifest sharing / URL-based init**
  `meldr init --from <url>` clones a manifest repo and sets up the workspace from a shared `meldr.toml`. Enables team onboarding in one command.

- [ ] **Hook system**
  User-defined hooks on workspace events: post-sync, post-add, post-worktree-create. Useful for running `npm install`, applying patches, or triggering builds after operations.

## Priority 3 — Remaining Configurability

- [ ] **Directory names** — hardcoded `"packages"`, `"worktrees"`, `".meldr"` in `workspace.rs` and `state.rs`. Configurable in `[workspace]` section. Requires threading config through all path helpers.

## Priority 4 — Remote Control

- [ ] **Create remote control sessions**
  `meldr remote` creates a remote control session for a worktree, enabling external tools and editors to interact with the workspace remotely.

- [ ] **Name remote control sessions after worktree**
  Remote control sessions automatically inherit the name of the worktree they are associated with, making it easy to identify and manage multiple concurrent sessions.

## Priority 5 — Nice to Have

- [ ] **Topological task ordering**
  If packages have inter-dependencies, `meldr exec` runs commands in dependency order. Optional dependency declaration in `meldr.toml`.

- [ ] **Export / import workspace**
  `meldr export` saves workspace definition (packages, groups, pins) to a portable file. `meldr import` restores it.

- [ ] **Shallow clone support**
  `meldr package add --depth 1` for faster initial cloning of large repositories.
