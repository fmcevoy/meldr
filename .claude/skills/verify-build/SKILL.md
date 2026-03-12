# /verify-build

Run the full build verification pipeline for meldr. Fail fast at any step.

## Usage

```
/verify-build              # Full pipeline including Docker integration tests
/verify-build --skip-docker  # Skip Docker integration tests
```

## Steps

Run each step sequentially. Stop on first failure and report which step failed.

1. **Build** — `cargo build --manifest-path meldr/Cargo.toml`
2. **Lint** — `cargo clippy --manifest-path meldr/Cargo.toml --all-targets -- -D warnings`
3. **Format** — `cargo fmt --manifest-path meldr/Cargo.toml --check`
4. **Unit tests** — `cargo test --manifest-path meldr/Cargo.toml --bin meldr`
5. **Integration tests** — `cd meldr && ./run-docker-tests.sh` (skip if `--skip-docker` is in the arguments)

## Output

After all steps complete (or on failure), report a summary:

```
Build Verification:
  ✓ build
  ✓ lint
  ✓ format
  ✓ unit-tests
  ✗ integration-tests — [error details]
```

If all steps pass, confirm with: **All checks passed.**
