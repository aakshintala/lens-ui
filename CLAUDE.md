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
- **MANDATORY** Review diversity — route reviews through `cursor-delegate` to a
  model family *other* than the author's (`gpt-5.5`, `gemini-3.5`). Every
  non-trivial change gets ≥1 review from a different family than wrote it.

## Skills

- **DEFAULT** When a pattern or chore recurs (≈2nd time, or clearly will),
  capture it as a skill via the `writing-skills` skill — iteratively, as we hit
  them. Skills live in `.claude/skills/` (Claude-only accelerator).

## Memory

- **DEFAULT** Persist durable learnings to the file-based memory dir and keep
  `MEMORY.md` current: decisions, gotchas, conventions, and user preferences —
  not what the repo/git already records. Save when something non-obvious is
  established; no automation.
