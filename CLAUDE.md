# Lens — Claude guide

**Read `AGENTS.md` first** — it holds the shared rules and the index into
`.agents/`. This file adds Claude-only rules.

## Delegation (Claude-only)

- **MANDATORY** Default subagent work — exploration, codegen, mechanical
  refactors, test-writing, codebase search — to `cursor-delegate` on
  **`composer-2.5`**.
- **MANDATORY** Escalate to a Claude **Opus** subagent (the `Agent` tool) only
  for Opus-level work: architecture/design, cross-doc ambiguity resolution,
  security-sensitive code, final review synthesis.
- **MANDATORY** Review diversity — every non-trivial change gets ≥1 review from
  a model family *other* than the author's.
- **MANDATORY** Route **all `gpt-5.5` delegation through `codex`**
  (`codex exec -s read-only`), **not** `cursor-delegate` — codex is the free
  gpt-5.5 path (memory `codex-as-reviewer`). Other families (`gemini-3.5`) still
  route through `cursor-delegate`.

## Skills

- **DEFAULT** When a pattern or chore recurs (≈2nd time, or clearly will),
  capture it as a skill via the `writing-skills` skill — iteratively, as we hit
  them. Skills live in `.claude/skills/` (Claude-only accelerator).

## Memory

- **DEFAULT** Persist durable learnings to the file-based memory dir and keep
  `MEMORY.md` current: decisions, gotchas, conventions, and user preferences —
  not what the repo/git already records. Save when something non-obvious is
  established; no automation.
