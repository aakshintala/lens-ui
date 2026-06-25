# Lens — STATUS

Lean, living status for the Lens design effort. Keep this doc **small**: it holds
the current forward-looking state only. **Full dated session entries live in
[`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)** — write each session's detail there
and roll older "Recent" pointers off this page as they age.

---

## Open threads & next up

- **Doc walkthrough complete** (all 11 design docs in `docs/design/` reviewed);
  every surfaced decision is resolved or consciously deferred.
- **Deferred, with a clean seam:**
  - **Notifications v2** — server-side push for the *fully-quit* case (needs an
    upstream omnigent push channel; v1 covers resident/backgrounded, shell §17.4).
  - **Server-stability spike** (capability §0.8) still **gates the project** — if
    `omnigent server` is unstable, the Rust-sidecar contingency reopens the framework.
  - **Markdown renderer** — the one real build risk (hand-rolled
    `pulldown-cmark`→gpui + sanitization; framework §4.1).
- **Tunables for the verification pass:** auto-sleep threshold (~10m), poll cadence
  (~10s), ring-buffer size (10 MB), transcript truncation tiers, `cost_samples` cadence.
- **Small undecided UX:** terminal-`transfer` UX, managed-provider selection,
  policy/skill in-app authoring, multi-depth breadcrumb, exact-vs-range version pin.
- **Build artifact:** render icons are unicode placeholders — ship a real
  status + harness-provider icon set.

## Recent

- **2026-06-24** — grilling pass + 11-doc walkthrough + first local renders;
  16 harnesses, lifecycle reshape (Sleep/Archive reclaim), cost two-axis,
  Concierge floating panel, Bridge Inbox layout, residency + notifications,
  new card design. → [`STATUS-ARCHIVE.md`](./STATUS-ARCHIVE.md)
