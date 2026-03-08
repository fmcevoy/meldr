# Meldr Gap Analysis: Workspace Infrastructure for the Agentic Era

## What Meldr Is

Meldr is a **workspace orchestration layer that sits above AI agents**. It
doesn't compete with Claude Code, Cursor, or Codex — it provides the
multi-repo infrastructure they run inside. Its job:

1. **Isolation** — Git worktrees per branch, per package, so agents don't collide
2. **Environment** — Tmux windows/panes with nvim + agent + terminals per package
3. **Lifecycle** — Create, sync, and tear down worktrees across repos atomically
4. **Execution** — Run commands in parallel across all packages (`meldr exec`)
5. **Configuration** — 5-layer config system (CLI → env → workspace → global → defaults)

This is the right abstraction. Agents handle code generation; meldr handles
the workspace they operate in. The question is: what does the workspace layer
need to do better to support the way agents are actually being used?

---

## What Meldr Does Today (Concrete)

| Capability | Implementation |
|---|---|
| Multi-repo worktree management | `worktrees/{branch}/{package}/` layout, parallel creation via rayon |
| Tmux environment orchestration | 5-pane dev layout per package (nvim + agent + 4 terminals), or custom layouts |
| Agent launching | Configurable command per agent type, launched into tmux panes |
| Parallel cross-package execution | `meldr exec` runs commands across all `packages/` dirs concurrently |
| Branch synchronization | `meldr sync` with rebase/merge strategies, autostash, per-package |
| State tracking | `.meldr/state.json` maps worktrees → tmux window/pane IDs |
| Safety checks | Dirty-state detection before worktree removal, duplicate prevention |
| One-shot setup | `meldr create` clones repos + creates worktrees + launches agents |

**What meldr tracks:** which packages exist, which branches have worktrees,
which tmux windows belong to which worktrees.

**What meldr does NOT track:** what agents are doing, whether they finished,
what they produced, or what task they were assigned.

---

## Gap Analysis

Gaps are framed from the perspective of **what a workspace layer above agents
should provide**. Meldr isn't trying to be an agent — it's trying to be the
best possible infrastructure for agents to work in.

---

### 1. Workspace-Level Context Propagation (CRITICAL)

**The problem:** Agents are only as good as the context they receive. Meldr
creates the worktree but sends agents in blind — they don't know about the
workspace structure, sibling packages, or cross-repo relationships.

**What meldr knows that agents don't:**
- Which packages exist and their relationships
- Which branches are active and who's working on what
- The workspace-level configuration and conventions
- What other agents have been doing in other worktrees

**What the workspace layer should provide:**

| Gap | What it means for meldr | Priority |
|---|---|---|
| **Workspace-aware context files** | Generate/propagate CLAUDE.md or AGENTS.md per worktree that includes workspace structure, package list, related branches. 72.6% of Claude Code projects rely on these. AGENTS.md is now a Linux Foundation standard. | P0 |
| **MCP server for workspace state** | Expose workspace topology (packages, worktrees, branches, dirty status) via MCP so agents can query it at runtime. MCP is becoming the standard interface — Moon and Rush already ship MCP servers. Agents in worktree A could ask "what branches exist? what packages are in this workspace?" | P0 |
| **Cross-package relationship context** | When launching an agent in `worktrees/feature-auth/api/`, tell it about `worktrees/feature-auth/web/` and that both are part of the same feature. Today agents have no idea sibling packages exist. | P1 |
| **Task context injection** | When creating a worktree for a specific task, include the task description, relevant files, architectural constraints. The workspace layer knows what the work is — it should tell the agent. | P1 |

**Why this is #1:** ETH Zurich (March 2026) and Cognition (Devin) independently
found that agent failures almost always stem from missing context. The workspace
tool is the *natural* owner of structural context — it knows things no
individual agent can discover on its own.

---

### 2. Agent Lifecycle Awareness (CRITICAL)

**The problem:** Meldr launches agents into tmux panes and then goes silent. It
has no idea if the agent is still running, finished successfully, errored out,
or is waiting for input. This makes meldr a "fire and forget" launcher rather
than an orchestration layer.

**What the workspace layer should track:**

| Gap | What it means for meldr | Priority |
|---|---|---|
| **Process-level awareness** | Detect whether the agent process in a tmux pane is still running, exited cleanly, or crashed. tmux provides `pane_dead` and exit codes. | P0 |
| **Git-based progress signals** | Monitor worktree git state changes (new commits, file modifications) as a proxy for agent activity. Meldr already has `git status --porcelain` for dirty detection — extend it to detect "work being done." | P0 |
| **Completion detection** | Know when an agent is done. Could be process exit, a sentinel file (`.meldr/done`), or git commit matching a convention. Enables "what's still running?" queries. | P1 |
| **Event hooks** | When an agent finishes, trigger configurable actions: run tests, create PR, notify user, start next worktree. The workspace layer is the right place for this — it manages the worktree lifecycle already. | P1 |

**Why this matters:** Without lifecycle awareness, the user is the orchestrator.
They manually check each tmux pane, decide what's done, and kick off next
steps. Meldr should replace that manual coordination.

---

### 3. Multi-Repo Branch Lifecycle (HIGH)

**The problem:** Meldr handles worktree creation and sync, but the full branch
lifecycle — from creation through merge and cleanup — has gaps, especially the
multi-repo coordination that is meldr's unique value.

**What's already working:** `meldr wt add` (create), `meldr sync` (rebase/merge),
`meldr wt remove` (cleanup with safety checks).

**What the workspace layer should add:**

| Gap | What it means for meldr | Priority |
|---|---|---|
| **Cross-repo conflict detection** | Before merging, check if branches in different packages will conflict with each other or with main. Meldr owns the multi-repo view — no other tool can do this. | P1 |
| **Coordinated PR creation** | `meldr pr create` generates linked PRs across all packages in a worktree. Include cross-references ("this PR is part of workspace feature-auth, see also: api#42, web#43"). | P1 |
| **Post-merge cleanup** | After PRs merge, auto-remove worktrees and branches across all packages. Extend existing `meldr wt remove` with merge-triggered automation. | P1 |
| **Cross-repo atomic merge** | Merge branches across all packages together or roll back all. No tool does this well — it's a natural extension of meldr's multi-repo worktree model. | P2 |
| **Dependency-ordered operations** | If package B depends on A, sync/merge/test A first. Requires knowing package dependencies (currently not tracked). | P2 |

**Why this matters:** AI agents produce 154% larger PRs (DORA Report). When 5
agents finish work across 3 repos, someone has to coordinate the merge. That
should be the workspace layer.

---

### 4. Enhanced Status & Dashboard (HIGH)

**The problem:** `meldr status` shows workspace name, package dirty states, and
worktree tmux window IDs. That's git-level status. For an orchestration layer,
the dashboard should answer: "what's happening across my workspace right now?"

**What `meldr status` should show:**

| Gap | What it adds | Priority |
|---|---|---|
| **Agent state per worktree** | Running / finished / errored / idle — per package, per worktree. Derived from tmux pane state + git activity. | P0 |
| **Recent git activity** | Last commit time, number of new commits since worktree creation, files changed. More useful than just "dirty" bit. | P1 |
| **Cross-worktree view** | Show all active worktrees, all agents, all packages in a single table. Current `status` is text-based; a structured table or TUI would help at scale. | P1 |
| **Sync state** | How far behind upstream each worktree is. Already doing this in `sync` — surface it in `status`. | P2 |

**Why this matters:** The value of an orchestration layer scales with
visibility. When you have 5 worktrees with 10 agents across 3 repos, you need
a single view of the whole system.

---

### 5. Task-to-Worktree Mapping (MEDIUM-HIGH)

**The problem:** Meldr's data model is branches and packages. There's no concept
of *why* a worktree exists — what task or feature it's serving. This makes
meldr a workspace manager, not a work manager.

**What the workspace layer could add:**

| Gap | What it means for meldr | Priority |
|---|---|---|
| **Task metadata on worktrees** | Associate a description, issue link, or spec file with each worktree. `meldr wt add feature-auth --task "Implement OAuth flow for API and Web"` | P1 |
| **Task state tracking** | planned → in-progress → reviewing → merged. Mirrors the worktree lifecycle meldr already manages. | P1 |
| **Spec file convention** | Look for `tasks.md` or `.meldr/task.md` in the workspace root. When creating a worktree, inject the relevant task description as agent context. | P2 |
| **Task dependency graph** | "Auth must finish before payment" — enables automatic sequencing of worktree creation. | P2 |

**Why this matters:** The emerging "spec-driven development" pattern
(requirements.md → design.md → tasks.md) needs infrastructure support.
The workspace layer is the natural place to map tasks to isolated environments.
This is what enables parallelism — decompose the work, assign each piece to
a worktree, let agents execute independently.

---

### 6. Runtime Isolation Beyond Filesystem (MEDIUM)

**The problem:** Git worktrees isolate files, but agents running dev servers or
tests in parallel will collide on ports, databases, and shared resources.

**What the workspace layer could provide:**

| Gap | What it means for meldr | Priority |
|---|---|---|
| **Port namespacing** | Assign unique port ranges per worktree (e.g., worktree 1 gets 3001/5433/8081, worktree 2 gets 3002/5434/8082). Inject as env vars. | P1 |
| **Environment variable isolation** | Set `PORT`, `DATABASE_URL`, etc. per worktree before launching agents. Meldr already manages the tmux env — extend it. | P1 |
| **Container wrapping (optional)** | Wrap each worktree in a lightweight container for full isolation. Opt-in for heavy use cases. Container Use (Dagger) and Devin do this. | P2 |
| **Disk budget warnings** | A 2GB codebase with 5 worktrees = 10GB+. Warn when approaching limits. | P3 |

**Why this matters:** This is the difference between "you can have 2 agents" and
"you can have 10 agents." File isolation is necessary but not sufficient for
true parallel execution with dev servers and tests.

---

### 7. Build & Validation Integration (MEDIUM)

**The problem:** No integration with build systems or CI. The workspace layer
should help validate agent output before it reaches review.

**What the workspace layer could provide:**

| Gap | What it means for meldr | Priority |
|---|---|---|
| **Pre-merge validation** | `meldr validate {worktree}` runs tests/lints across all packages in a worktree. Leverages `meldr exec` which already runs parallel commands. | P1 |
| **Affected package analysis** | Determine which packages were actually modified in a worktree and only validate those. Basic version: check git diff per package. | P1 |
| **CI status tracking** | Surface CI pass/fail per branch in `meldr status`. Query GitHub Actions / external CI via APIs. | P2 |
| **Build caching** | Cache test/build results per worktree to avoid re-running unchanged packages. Similar to Nx/Turborepo but at the workspace level. | P3 |

**Why this matters:** 67.3% of AI-generated PRs are rejected (LinearB). The
workspace layer can catch failures early by running validation before the agent
even creates a PR. `meldr exec` is the primitive; validation is the product.

---

### 8. Observability (LOWER)

**The problem:** No logging or metrics. When managing many agents, you need to
know what happened, not just what's happening now.

| Gap | What it means for meldr | Priority |
|---|---|---|
| **Workspace event log** | Log worktree creation, agent launch, sync operations, completion events to `.meldr/log`. | P2 |
| **Agent output capture** | Optionally capture tmux pane output to files for post-hoc review. tmux's `pipe-pane` makes this straightforward. | P2 |
| **Session history** | Which worktrees were created, how long they lasted, what was the outcome. Persisted across sessions. | P3 |

---

## What Meldr Should NOT Do

Staying disciplined about scope is as important as identifying gaps. Meldr is
a workspace layer, not:

- **An agent framework** — Don't reimplement Claude Code, CrewAI, or LangGraph.
  Agents handle code generation; meldr handles the workspace they work in.
- **A project management tool** — Don't build Jira. Task metadata should be
  lightweight (description + state), just enough to map work to worktrees.
- **A build system** — Don't compete with Nx/Turborepo on build graph
  optimization. Use `meldr exec` to invoke whatever build tool the project uses.
- **A CI/CD platform** — Don't replace GitHub Actions. Integrate with CI, don't
  replace it.
- **A container orchestrator** — Don't build mini-Kubernetes. Port namespacing
  via env vars covers 80% of cases. Container support should be opt-in.

The principle: **meldr provides the infrastructure and context that agents need,
and coordinates the multi-repo lifecycle that no single agent can manage.**

---

## Strategic Roadmap

### Phase 1: Make the Workspace Aware (infrastructure for agents)
1. **MCP server** — Expose workspace state so agents can self-coordinate
2. **Context file propagation** — CLAUDE.md/AGENTS.md with workspace structure
3. **Agent lifecycle detection** — Process state, git activity, completion signals
4. **Enhanced status** — Agent state + git state in a single dashboard view

### Phase 2: Coordinate the Multi-Repo Lifecycle (meldr's unique value)
5. **Coordinated PR creation** — Linked PRs across packages in one command
6. **Cross-repo conflict detection** — Proactive warnings before merge
7. **Task-to-worktree mapping** — Associate work descriptions with worktrees
8. **Pre-merge validation** — Run tests across affected packages

### Phase 3: Scale the Workspace (support 10+ agents)
9. **Port/environment isolation** — Env vars per worktree for runtime separation
10. **Event hooks** — Trigger actions on agent completion (tests, PRs, next task)
11. **Observability** — Event log, output capture, session history
12. **Cross-repo atomic merges** — All-or-nothing merge across packages

---

## Competitive Positioning

Meldr's role in the ecosystem:

```
┌─────────────────────────────────────────────────────────┐
│  HUMAN (task decomposition, review, decisions)          │
├─────────────────────────────────────────────────────────┤
│  MELDR (workspace layer)                                │
│  - Multi-repo worktree isolation                        │
│  - Tmux environment orchestration                       │
│  - Context propagation to agents                        │
│  - Branch lifecycle coordination                        │
│  - Status & observability                               │
├──────────┬──────────┬──────────┬────────────────────────┤
│ Claude   │ Cursor   │ Codex    │ (any future agent)     │
│ Code     │          │ CLI      │                        │
│ (agent)  │ (agent)  │ (agent)  │                        │
├──────────┴──────────┴──────────┴────────────────────────┤
│  GIT (worktrees, branches, repos)                       │
├─────────────────────────────────────────────────────────┤
│  FILE SYSTEM / CONTAINERS (isolation)                   │
└─────────────────────────────────────────────────────────┘
```

**vs. Agent orchestrators** (Claude Squad, Composio, Conductor):
These are agent-specific wrappers. They orchestrate one type of agent.
Meldr is agent-agnostic and adds multi-repo awareness.

**vs. Traditional workspace tools** (gita, meta, mu-repo):
These manage multiple repos but have no concept of agents, worktree-per-task
isolation, or workspace context propagation.

**vs. Monorepo tools** (Nx, Turborepo, Bazel):
These optimize builds in a single repo. Meldr manages isolated work
environments across multiple repos.

**Meldr's unique value:** It's the only tool that combines multi-repo worktree
management with agent-aware workspace orchestration. The closest competitor
pattern is Claude Squad (worktrees + tmux + agents), but Claude Squad is
Claude-specific and single-repo.

---

## Key Data Points

- 72.6% of Claude Code projects use CLAUDE.md for agent context
- AGENTS.md adoption: 29% reduction in median runtime, 17% fewer tokens
- Cursor 2.0: up to 8 concurrent agents, each needing isolated workspaces
- AI-generated PR rejection rate: 67.3% vs 15.6% manual — validation essential
- AI adoption → 154% increase in PR size — merge coordination critical
- MCP expected in 75% of gateway vendors by 2026
- 2GB codebase → ~10GB with 5 worktrees in 20 minutes
- Stripe runs 1,000+ merged agent PRs/week — workspace tooling is table-stakes

---

*Research compiled March 2026. Sources: DORA Report, LinearB, ETH Zurich,
SWE-bench, Anthropic, Cognition, Google ADK, Linux Foundation.*
