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
- **NEXT: execute lens-client REST surface (Plans 2a–2e)** (fresh session, subagent-driven) —
  `docs/superpowers/plans/2026-06-25-lens-client-plan2{a,b,c,d,e}-*.md`. 2a=events write
  path (fully specified); 2b=sessions read; 2c=lifecycle; 2d=resources/terminals/comments;
  2e=registries. Decision baked in: reads return **typed wrappers (private fields +
  typed getters), never `serde_json::Value` to consumers**; grow accessors lazily from
  omnigent source (2d/2e carry ⚠ field-verify notes). Writes reuse generated request types.
- **⚠ CHECKPOINT before Plan 3 (SSE taxonomy/state-model):** reassess tracking
  `0.3.0.dev0` vs waiting for a `0.3.0` release tag. The REST surface (2a–2e) is cheap +
  re-vendor-safe; the streaming taxonomy encodes the semantically-unstable parts (3 status
  vocabularies, partial-merge child summaries, the events body) where dev0 churn is
  expensive. Build REST now; gate the taxonomy investment on a conscious dev0-vs-0.3.0 call.
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
