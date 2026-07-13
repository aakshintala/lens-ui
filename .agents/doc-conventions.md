# Doc conventions (reference)

The convention already in use in `docs/design/`. Follow it for new docs.

## Directory taxonomy (`docs/`)

Two kinds of docs, split by lifecycle:

- **Living docs** — named by subject, edited in place: `design/` (architecture,
  plus `renders/` and the June whole-design `review/`), `adr/`, `STATUS.md`.
- **Dated artifacts** — `YYYY-MM-DD-<topic>.md`, immutable once their session
  ends: `specs/` (feature/design specs that feed plans), `plans/`
  (implementation plans), `spikes/` (spike reports + `captures/`),
  `handoffs/`.

Tool-named directories are banned — skills that default their output elsewhere
(e.g. `docs/superpowers/…`) write to `docs/specs/` and `docs/plans/` instead.

## Design docs (`docs/design/`)

- **Named by what they describe** — the filename says the subject.
- **Behavior & contract first**, framework specifics only where they matter.
- **Cite ground truth** — every endpoint/event assertion cites the openapi path
  or schema. Vanished internal design docs are NOT cited.
- **Pin-and-verify** — keep a "what would break if X changes" seams section.
- **Shell vs content split** — the shell doc owns containers/chrome; surface
  docs own the content that fills the slots, written container-agnostic.

## Status & handoffs

- `docs/STATUS.md` is small and forward-looking only — current state.
- Dated session detail rolls into `docs/STATUS-ARCHIVE.md`.
- Session handoffs live in `docs/handoffs/YYYY-MM-DD-<topic>.md`.
