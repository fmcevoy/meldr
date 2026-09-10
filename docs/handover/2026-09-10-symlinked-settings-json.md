# Hooks were being written through a symlinked `settings.json`

**Date:** 2026-09-10
**Branch:** `claude/reading-comprehension-c7hlcv`
**Commit:** `16af873`
**Companion change:** `fmcevoy/fmcevoy_tools` — see that repo's `docs/handover/2026-09-10-setup-overlay-and-config-drift.md`. **This change should merge first** (see [Rollout order](#rollout-order)).

---

## The bug

`resolve_settings_path` in `src/core/install_hooks.rs` canonicalised `~/.claude/settings.json` before writing:

```rust
fn resolve_settings_path(home: &Path) -> Result<PathBuf> {
    let candidate = home.join(".claude/settings.json");
    if candidate.exists() {
        std::fs::canonicalize(&candidate).map_err(MeldrError::Io)
    } else {
        Ok(candidate)
    }
}
```

That path is very often a symlink into a dotfiles repo. Canonicalising it meant every `meldr install-hooks` wrote **through** the link, into tracked source.

This was not theoretical. `fmcevoy_tools` had meldr's hook block committed in `mac_setup/configs/claude/settings.json` — `_meldr: true` markers, canonical matchers, and serde's alphabetical key ordering across the whole file, which is the tell that meldr wrote it rather than a human.

The canonicalise arrived in #43 with no comment, no test, and no stated rationale. It reads as incidental rather than a deliberate decision to preserve the symlink.

---

## The fix

Delete the canonicalise:

```rust
fn resolve_settings_path(home: &Path) -> Result<PathBuf> {
    Ok(home.join(".claude/settings.json"))
}
```

That is the whole change. It works because of how `write_json_atomic` already behaves — it writes a temp file and `rename`s it over the destination. POSIX `rename` replaces the **link itself**, not its target:

| | Write lands on | Repo | Symlink |
|---|---|---|---|
| With canonicalise | the link's target | **dirtied** | preserved |
| Without | the link path | untouched | replaced by a real file |

So the outcome is:

- The dotfiles repo is left alone.
- The settings that were reachable through the link survive, because `read_settings` reads them before the write.
- The path self-heals: after one install it is a normal file, and stays one.

`Result` is kept on the return type so no caller changes.

---

## Test

`install_never_writes_through_a_symlinked_settings_file` builds a temp `$HOME` and a temp "repo", symlinks `~/.claude/settings.json` at a tracked file containing `{"theme": "dark"}`, runs `install_claude_hooks`, and asserts:

1. the tracked file is byte-for-byte unchanged;
2. the live path is no longer a symlink;
3. all three events in `MELDR_HOOKS` report `HookState::Ok`;
4. `theme` is still `dark` — settings reachable only through the link were not lost.

The test was confirmed to fail with the canonicalise restored, and the failure output is the bug itself: the tracked file comes back carrying the full hook block.

---

## Verification

- `cargo build` — clean
- `cargo clippy --all-targets -- -D warnings` — clean
- `cargo fmt --check` — clean
- `cargo test --bin meldr` — 383 passed, 0 failed

**Not run:** `./run-docker-tests.sh` (integration tests need Docker).

---

## Also changed

`README.md`, under **Wire Claude Code hooks**, now states that a symlinked `settings.json` is replaced by a real file rather than written through, and why: hooks are machine state and do not belong in tracked source.

---

## Rollout order

`fmcevoy_tools` `setup.sh` Step 10 installs meldr with `cargo install --git … --force` from **`main`**. Until this merges, every laptop keeps building the canonicalising binary. On a machine whose private overlay re-links `~/.claude/settings.json` into `~/fmcevoy`, that binary writes the hook block into the *private* repo instead — the same bug, relocated.

1. Merge this branch.
2. Then merge the `fmcevoy_tools` branch.
3. Then re-run `setup.sh` per laptop.

---

## Related, not addressed here

The `fmcevoy_tools` side stops linking `~/.claude/settings.json` into the repo at all, and merges a template into it instead. The two changes are independent — either alone closes the public-repo leak — but only this one protects a laptop whose *private* overlay owns that path, since meldr cannot know which repo a symlink points into.
