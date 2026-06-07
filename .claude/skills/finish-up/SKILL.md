---
name: finish-up
description: Use when implementation is complete, verified, and ready to ship. Handles the full finish routine - commit, PR, wait for CI, merge, sync workspace, and remove the worktree. Auto-detects direct-push vs PR mode; handles rebase when main is ahead.
---

# Finish Up

End-to-end routine for shipping completed work. Auto-detects whether to push directly to main (dotfiles / personal repos) or use a full PR + CI flow (protected repos).

## Prerequisites

Before invoking this skill, all of the following MUST be true:
- Implementation is complete
- `cargo build` (or equivalent) passes
- `cargo test` (or equivalent) passes
- `cargo clippy` (or equivalent) shows no new warnings

If any prerequisite is unverified, run the verify-build skill first.

## Mode detection

**Direct-push mode** is used when ALL of the following are true:
- Working on the default branch (`main`/`master`) directly.
- No push-triggered CI workflows (no `.github/workflows/` for `on: push` events,
  OR the repo has a recent pattern of commits landing directly on main without PRs).
- No branch protection requiring PR: check with `gh api repos/{owner}/{repo}/branches/<main>/protection 2>/dev/null` — a 404 or empty response means unprotected.

If any precondition fails, fall back to the full PR flow (Steps 1-7).

You can override: invoke as `finish-up direct` to force direct-push, or `finish-up pr` to force PR flow.

---

## Steps

Execute in order. Stop and report if any step fails.

### 0. Sync with main

Run this BEFORE committing and AGAIN right before merge (main can move during CI).

```
MAIN=$(git symbolic-ref --short refs/remotes/origin/HEAD 2>/dev/null | sed 's@origin/@@')
MAIN=${MAIN:-main}
git fetch origin "$MAIN"
BEHIND=$(git rev-list --count HEAD..origin/"$MAIN")
```

- If `BEHIND > 0` AND we are on a **feature branch**: rebase onto origin/main:
  `git rebase origin/"$MAIN"`
  On conflict: run the **Auto-resolve obvious conflicts** subroutine below.
  After a successful rebase that moved commits:
  `git push --force-with-lease`   (NEVER plain --force)
- If `BEHIND > 0` AND we are on **main directly** (direct-push mode):
  `git pull --rebase origin "$MAIN"`
  On conflict: run the **Auto-resolve obvious conflicts** subroutine.

### 1. Commit all changes

- Stage all modified and new files relevant to the work (avoid secrets, .env, etc.)
- Check any pre-existing unstaged changes (e.g. settings.json) -- confirm with user
  whether to bundle them or leave unstaged before staging.
- Write a concise commit message summarizing the changes.
- Include `Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>`

### 2. [PR mode] Push and create PR

- Push the branch to origin with `-u`
- Create a PR with `gh pr create` targeting the default branch
- Include a summary and test plan in the PR body
- Capture the PR URL and number

### 3. [PR mode] Wait for CI

- Use `gh pr checks <PR_NUMBER> --watch` to wait for CI to complete
- If checks fail: investigate, fix, commit, push, and wait again
- Do NOT proceed until all checks pass

### 3.5. [PR mode] Re-sync before merge

- Run Step 0 again -- main may have moved during CI wait.

### 4. [PR mode] Merge the PR

- Verify ALL CI checks passed: `gh pr checks <PR_NUMBER>` -- every check must show `pass`
- `gh pr merge <PR_NUMBER> --squash --delete-branch --admin`
  (--admin bypasses branch protection only after all checks verified)
- Confirm merge succeeded

### 4-direct. [Direct mode] Push to main

- `git push origin "$MAIN"`
- Confirm push succeeded (exit code 0); if rejected because origin moved,
  re-run Step 0 and retry once.

### 5. Sync workspace

- Navigate to workspace root (parent of `worktrees/`)
- Run `meldr sync --all` to update all worktrees with the merged changes
- If not in a meldr workspace, skip this step

### 6. Remove the worktree

- Run `meldr worktree remove <branch>` to clean up the merged worktree
- If the branch name contains slashes, quote it appropriately
- If removal fails because we're inside the worktree, cd to workspace root first and retry
- If in direct-push mode on main (no worktree to remove), skip this step

### 7. Report completion

- Print the merged PR URL (or pushed commit SHA for direct mode)
- Confirm worktree was removed (or note it was skipped)
- One-sentence summary of what was shipped

---

## Auto-resolve obvious conflicts (subroutine)

User policy: "Figure out what their changes are and play ours on top. Only come back
to me if it is not super obvious."

For each file with conflict markers, classify and act:

### OBVIOUS -- resolve, `git add <file>`, continue

**Additive-only in append-style files**: both sides added DIFFERENT new content
(no overlapping lines) to a list/table/config block. Keep BOTH. Order: follow the
file's existing convention (alphabetical / chronological / insertion order).
Most common case in dotfiles + docs repos:
- zshrc headless-agents block, alias blocks
- cli-upgrades section list and upgrade case branches
- tmux/help shortcut rows
- README tables
- completions.zsh sections

**Lockfiles** (Cargo.lock, package-lock.json, poetry.lock, uv.lock,
pnpm-lock.yaml, Pipfile.lock): accept theirs via `git checkout --theirs <file>`,
then regenerate locally (cargo build / npm install / poetry lock --no-update / uv lock).
`git add` the regenerated file.

**Generated files**: accept theirs, re-run the generator.

**Pure whitespace / formatting** with zero semantic delta: accept theirs.

### ESCALATE -- stop, surface to user

- Both sides modified the SAME expression/value/default to DIFFERENT values.
- Both sides modified the same function body non-additively.
- Rename on one side, in-place edit on the other.
- Test assertions changed by both sides.
- Anything where keeping both produces broken or nonsensical code.

When escalating: do NOT `git rebase --abort`. Show the user:
1. `git status` and the conflicted file(s)
2. The conflict hunks (just the markers -- no wall of context)
3. One line: "ours wanted X, theirs wanted Y"

Ask which side wins, or ask for a manual edit. After the user resolves:
`git add <file>` then `git rebase --continue`. Repeat until rebase finishes.

---

## Error Handling

- If PR creation fails (e.g., no upstream): push first, then retry
- If CI fails: diagnose, fix, push, wait again -- do not merge broken code
- If merge conflicts on rebase: run the **Auto-resolve obvious conflicts** subroutine
- If direct push is rejected because origin moved: re-run Step 0 (pull --rebase), retry once
- If meldr commands are not available: skip steps 5-6 and inform the user
- Never use `--force` on push; always use `--force-with-lease` on feature branches
