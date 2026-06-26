# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

---

## Open threads & next up

- **lens-client Foundation: DONE** (Plan 1 executed, 9 commits `043214e..f12050f`) —
  crate skeleton/error/ids/connection, typify codegen (`generated.rs`, 88 schemas,
  rustfmt-canonical via xtask), HTTP core + contract gate + ready-ladder handshake.
  16 serverless tests, clippy/fmt clean, live handshake green vs pinned `0.3.0.dev0`.
  Both seam reviews applied (gpt-5.5 codegen; gemini-3.1-pro final → 3 error-soundness
  fixes). Gotchas in memory `lens-client-foundation-gotchas`.
- **lens-client REST surface (Plans 2a–2e): DONE** (executed subagent-driven,
  composer-2.5 build + Opus per-task review + gpt-5.5 cross-family; 31 commits
  `b69e3d8..299ff72`). 2a=events write path; 2b=sessions read; 2c=lifecycle;
  2d=resources/terminals/comments; 2e=registries. 47 serverless tests, clippy
  `--all-targets` + fmt clean, `generated.rs` untouched. Live-verified vs pinned
  `0.3.0.dev0`: send_event, sessions read (get/list/child), create→patch→delete
  lifecycle. Reads are typed wrappers (private fields + getters, **no `Value` to
  consumers**); writes reuse generated request types. Cross-family review caught
  4 real shape bugs (hosts `{hosts}` envelope, directories `{object,path}` no-id,
  policy id nullable, resources `Value` leak) — all fixed.
  - **Deferred from 2a–2e (no consumer / need runner-backed live capture):**
    `Sessions::items()` (→ Plan 3 typed item union), list endpoints with unknown
    envelopes (`environments`, `terminals`, `changed_files`, `list_runners`), and
    ⚠ minimal wrappers (FileContent/ShellResult/FileResource/Host/Policy*) — grow
    getters + verify field names with golden captures when the state-model consumes
    them. Full rollup in `.superpowers/sdd/progress.md`.
- **Checkpoint RESOLVED (2026-06-25): build Plan 3 on `0.3.0.dev0` now.** No signal on
  `0.3.0` timing or whether it'll materially change the API — not worth idling a project
  with a live server to extract ground truth from. Treat dev0 instability as a *planning
  input*, not a blocker. **Plan 3 approach (decided):**
  1. **Golden-SSE capture spike FIRST** — capture real streams from the live pinned server
     (status events, `child_session` updates, the conversation-item union, deltas) and model
     the typed taxonomy from those bytes, NOT from the under-specified openapi. (The 4 review
     bugs in 2c–2e were all guessed-envelope mistakes; capture-from-bytes is the fix, applied
     up front to the riskiest layer.)
  2. **Split by stability** — reader-thread + reconnect plumbing is already de-risked (transport
     spike: subscribe-first + mid-stream-drop recovery), build confidently; gate only the
     semantic event union on the captures.
  3. **Stand up contract-drift CI** (outstanding B6) — the passive alarm that makes tracking
     dev0 safe when `0.3.0` eventually tags.
  - composer-2.5 is weakest on temporal/stateful logic (`[[composer-delegation-profile]]`) — Plan 3
    is exactly that, so **per-task cross-family review returns here** (was relaxed for the static
    REST surface). Mind the Cursor-credit cost (`[[review-spend-policy]]`).
  - Now on branch `feat/lens-client-streaming` (off `main` @ `78fdaa3`).
- **Doc walkthrough complete** (all 11 design docs in `docs/design/` reviewed);
  every surfaced decision is resolved or consciously deferred.
- **Deferred, with a clean seam:**
  - **Notifications v2** — server-side push for the *fully-quit* case (needs an
    upstream omnigent push channel; v1 covers resident/backgrounded, shell §17.4).
  - **Server-stability spike** (capability §0.8) — **trending PASS; the
    Rust-sidecar contingency does not reopen.** Full findings:
    [`docs/spikes/2026-06-25-transport-stability.md`](./spikes/2026-06-25-transport-stability.md).
    Warm cold-start ~1.6s, ready ladder <5ms; runs agents end-to-end; live SSE
    parses clean (subscribe-first/no-replay); **mid-stream-drop reconnect
    recovers with zero persisted-item loss** (typed-client §7); failures typed
    (`runner_failed_to_start`); daemon/runner lifecycle confirms
    server-lifecycle §3.1/§6. Not separately driven: server-crash reconnect
    (P7), RSS under sustained load. Throwaway harness discarded.
  - **Markdown renderer** — the one real build risk (hand-rolled
    `pulldown-cmark`→gpui + sanitization; framework §4.1).
- **Tunables for the verification pass:** auto-sleep threshold (~10m), poll cadence
  (~10s), ring-buffer size (10 MB), transcript truncation tiers, `cost_samples` cadence.
- **Small undecided UX:** terminal-`transfer` UX, managed-provider selection,
  policy/skill in-app authoring, multi-depth breadcrumb, exact-vs-range version pin.
- **Build artifact:** render icons are unicode placeholders — ship a real
  status + harness-provider icon set.

## Recent

- **2026-06-25 (eve)** — lens-client **REST surface 2a–2e executed** end-to-end
  (subagent-driven: composer-2.5 build, Opus per-task review, gpt-5.5 cross-family
  at seams + one consolidated 2c–2e review). 31 commits, 47 tests, live-verified.
  Review caught/fixed 4 real response-shape bugs. Cross-family review cadence
  relaxed to one consolidated pass mid-drive to conserve Cursor credits.
- **2026-06-25 (pm)** — omnigent contract-pinning decided (ADR-0001: freeze a
  commit, not the moving `0.3.0.dev0`; lock to release tags from `0.3.0`).
  Confirmed the "removed" elicitation/permission routes were only hidden from
  the openapi reference (`include_in_schema=False`), still ap-web-used → still
  contract. lens-client foundation brainstormed → spec
  (`typed-client-implementation.md`, decisions D1–D4: typify one-shot codegen;
  sync/blocking, no tokio; local xtask verification; coarse dev0 gate) → Plan 1
  written. Fixed two `typed-client.md` drifts (stale ~8 stream cap; async→sync).
- **2026-06-25** — Cargo workspace stood up (edition 2024, spikes/ vs crates/
  lint wall); omnigent pinned-source install + `installing-omnigent-from-source`
  skill; **transport-stability spike** (throwaway harness, Opus-spec →
  composer-2.5 build → gpt-5.5 review → live-run): validated cold-start, SSE
  parse/taxonomy, subscribe-first; confirmed daemon/runner lifecycle
  (server-lifecycle §3.1). Reconnect probes (P6/P7) next to close the §0.8 gate.
- **2026-06-24** — grilling pass + 11-doc walkthrough + first local renders;
  16 harnesses, lifecycle reshape (Sleep/Archive reclaim), cost two-axis,
  Concierge floating panel, Bridge Inbox layout, residency + notifications,
  new card design. → [`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)
