# /codebase-flow

Document and describe the main flows in the meldr codebase. Outputs two files:

- **`docs/codebase-flow.json`** — machine-readable flow data; feed this to Claude for instant codebase context when working on features or bugfixes
- **`docs/codebase-flow.html`** — interactive node graph for humans; click flows to highlight the path, hover nodes for details

## Inspired by

[@DaveJ on X](https://x.com/DaveJ): “Ask Claude to document and describe the main flows in your app and output in a single page html + json data file. Incredibly useful for humans and the JSON file is very useful for explaining the flow to the LLM when working on new features/bugfixes.”

## Usage

- `/codebase-flow` — regenerate both files from current source
- `/codebase-flow sync` — update only the sync flow
- `/codebase-flow <file>` — trace data flow starting from a specific source file

## How to use the JSON with Claude

When starting work on a new feature or bugfix, include the JSON in your prompt:

```
Here is the meldr codebase flow map: <contents of docs/codebase-flow.json>

I want to add X...
```

This gives Claude instant architectural context without reading every source file.

## Instructions

1. **Read** the relevant source files:
   - Always: `src/main.rs`, `src/cli/mod.rs`, `CLAUDE.md`
   - Per command: `src/cli/<cmd>.rs` + relevant sections of `src/core/worktree.rs`

2. **Write `docs/codebase-flow.json`** with this structure:
   - `layers`: the abstraction layers (entry, cli, core, git, tmux) with colors
   - `nodes`: every module/file with `id`, `label`, `layer`, `file`, `description`
   - `edges`: directed connections between nodes
   - `flows`: named command paths, each with ordered `steps` (`node`, `label`, `detail`)
   - `config`: config resolution layers and keys
   - `agents`: built-in agent list
   - `directory_layout`: workspace directory structure

3. **Write `docs/codebase-flow.html`** — self-contained interactive page:
   - Embeds the JSON data as a JS constant
   - SVG node graph: nodes in columns by layer, bezier curve edges
   - Right panel: clickable flow list + numbered step details
   - Hover nodes for description tooltip
   - Click a flow → highlight nodes + edges on that path, dim everything else
   - Dark theme: `--bg:#0d1117`, `--surface:#161b22`
   - Layer colors: Entry=`#8b949e`, CLI=`#388bfd`, Core=`#3fb950`, Git=`#d29922`, Tmux=`#bc8cff`

4. **Report** paths to both files.

## Column layout for the SVG graph

| Column | Layer | x position |
|--------|-------|------------|
| Entry  | entry | 90px       |
| CLI    | cli   | 270px      |
| Core   | core  | 460px      |
| Git/Tmux | git, tmux | 650px |

Nodes within each column are evenly distributed vertically over 560px height with 48px top/bottom padding. Node size: 152×26px.
