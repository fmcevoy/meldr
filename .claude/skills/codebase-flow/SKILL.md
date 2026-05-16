# /codebase-flow

Generate an interactive HTML flow diagram of any codebase — or regenerate the meldr architecture visualization.

## Usage

- `/codebase-flow` — Regenerate `docs/codebase-flow.html` for the current state of the meldr codebase
- `/codebase-flow <command>` — Generate a focused flow for a specific command (e.g. `/codebase-flow sync`)
- `/codebase-flow <file>` — Trace data flow starting from a specific source file

## What it produces

A self-contained HTML file (`docs/codebase-flow.html`) with:

- **Architecture tab** — layer dependencies (CLI → Core → Git → Tmux)
- **Command flows tab** — flowcharts for each CLI command
- **Config resolution tab** — 5-layer config priority chain
- **Worktree lifecycle tab** — state machine + directory layout

## Approach (based on CJ Hess / wrode/flow-diagram pattern)

Instead of ASCII diagrams, Claude reads the actual source and generates interactive HTML with Mermaid.js flowcharts. The HTML is self-contained and git-trackable — it's the living documentation artifact.

## Instructions

1. **Read the relevant source files**:
   - Always read: `src/main.rs`, `src/cli/mod.rs`, `CLAUDE.md`
   - For command flows: read `src/cli/<command>.rs` + relevant sections of `src/core/worktree.rs`
   - For deep dives: read the specific core file being traced

2. **Generate the HTML** — follow the structure in `docs/codebase-flow.html`:
   - Use Mermaid.js from CDN (`https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js`)
   - Dark theme matching GitHub dark mode palette
   - Tab navigation (vanilla JS, no framework)
   - Color-code by layer: CLI=blue `#388bfd`, Core=green `#3fb950`, Git=orange `#d29922`, Tmux=purple `#bc8cff`

3. **Save** the result to `docs/codebase-flow.html`

4. **Report** the path and a one-line summary of what changed

## Color coding convention

| Layer | Color | Mermaid classDef |
|-------|-------|------------------|
| CLI (`cli/`) | `#388bfd` blue | `:::cli` |
| Core (`core/`) | `#3fb950` green | `:::core` |
| Git (`git/`) | `#d29922` orange | `:::git` |
| Tmux (`tmux/`) | `#bc8cff` purple | `:::tmux` |
| Entry points | `#8b949e` gray | `:::entry` |

## Notes

- `docs/codebase-flow.html` is pre-generated from the current main branch.
- Re-run `/codebase-flow` after significant architecture changes to keep it current.
- For focused traces, prefer generating `docs/flow-<topic>.html` rather than overwriting the main one.
