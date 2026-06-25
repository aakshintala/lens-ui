# Code style

Severity per `AGENTS.md`.

## Comments

- **MANDATORY** Comment the *why* and the non-obvious only — never narrate what
  the code literally does. A comment restating the line is noise; delete it.
- **DEFAULT** Prefer a clarifying name or type over a comment.

## Modularity

- **MANDATORY** Modularize so each unit is independently **testable and
  benchmarkable** — one clear purpose, a well-defined interface, explicit
  dependencies. (Pairs with functional-core/imperative-shell in `principles.md`.)
- **DEFAULT** When a file grows large it is usually doing too much — split it.
- **DEFAULT** Keep crate/module seams real: depend on interfaces, not internals.
