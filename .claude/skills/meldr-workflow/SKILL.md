# /meldr-workflow

Orchestrate a full development workflow: from sync through implementation to merged PR.

## Usage

```
/meldr-workflow <task description>
```

## Workflow Steps

### 1. Sync
```bash
meldr sync
```
Ensure the workspace is up to date before starting.

### 2. Create feature worktree
```bash
meldr worktree add <branch-name>
```
Derive the branch name from the task description (e.g., `fix-login-bug`, `add-search-feature`).

### 3. Implement
Execute the user's task in the worktree. Navigate to the appropriate package directories under `worktrees/<branch>/` and make the required changes.

### 4. Verify build
Run `/verify-build` to ensure all checks pass (build, lint, format, unit tests, integration tests). Fix any failures before proceeding.

### 5. Commit and push
For each package with changes:
```bash
cd worktrees/<branch>/<package>
git add <changed-files>
git commit -m "<descriptive message>"
git push -u origin <branch>
```

### 6. Create PR
Ask the user to review the PR description before creating it:
```bash
gh pr create --title "<title>" --body "<body>"
```
This is the **only step requiring user confirmation**.

### 7. Enable auto-merge
```bash
gh pr merge --auto --squash
```
GitHub will merge automatically when CI passes. No further user confirmation needed.

### 8. Wait for merge
Check PR status periodically:
```bash
gh pr view <number> --json state
```

### 9. Cleanup
Once merged:
```bash
meldr worktree remove <branch>
meldr sync
```

## Key Decisions

- **Auto-merge enabled** — once the PR is created and approved, `gh pr merge --auto --squash` handles the merge when CI passes
- **User confirmation only at PR creation** — all other steps are automated
- **Fix before proceeding** — if `/verify-build` fails, fix the issues and re-verify before committing
- **Branch naming** — derive from task: lowercase, hyphen-separated, descriptive (e.g., `add-user-auth`, `fix-sync-race`)
