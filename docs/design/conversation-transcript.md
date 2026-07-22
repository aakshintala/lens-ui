# Conversation transcript

The surface that turns the state model's canonical `Item` list into the thing
a human reads and acts in. Owns *how items render*, the progressive-render
semantics, and the live-turn surface.

**Status:** Draft, 2026-06-23.
**Depends on:** the application architecture & state model document (reads
`Vec<Item>` + `StreamScratch` + the pure render-time transforms).
**Seams to:** the workspace & terminals, agent definition, permissions &
elicitations, sub-agent topology, application shell documents.

Framework-neutral; framework divergence points isolated in §16.

---

## 1. Scope & boundaries

**This document owns:**

- The **render pipeline** — canonical `Item`s → `ViewBlock`s via pure projection
  (§3).
- The **transcript skeleton** and the live→settled **turn lifecycle** (§4).
- **Streaming/progressive render** (§5).
- **Content rendering** — markdown vs verbatim channels; assistant & user
  message rendering (§6).
- **Per-item-kind rendering** — reasoning (§7), tool spans + output handling
  (§8), native tools (§9), compaction (§10), minor items (§11), resource
  events (§12), agent-changed marker (§13).
- **The agent's `session.todos` rendered inline** (§14) — distinct from the
  Bridge's planning todos.
- The **live turn** — optimistic input, steering, interrupt (§15).
- The **scrolling surface** — auto-scroll, virtualization, anchoring (§16).
- **Edge states** — empty, disk-paint, hydration (§17).

**This document does NOT own:**

- The wire / reconnect / SSE normalization (the typed client); the domain
  model, reducer, persistence, command flow, liveness (the state model).
- The **Review tab** — a true cumulative git diff of the working tree vs.
  base, with inline comments that steer the agent. Sourced from the workspace
  doc's `changes` + `diff/{path}` endpoints. The workspace & terminals
  document owns it. This document owns only the *per-edit* diff inside a tool
  span (§8.4).
- **Permission policy / elicitation lifecycle / the response widget** (the
  permissions document). This document hosts the component and positions it
  (§18-seam).
- The **sub-agent topology UI** (rail/tree/list, hierarchy navigation) — the
  sub-agent topology document. This document owns only the in-transcript
  representation of a child's presence (§8.6 / §18-seam).
- The **card visual** (layout, the wave effect) — the application shell. This
  document only asserts the card is coarse metadata, never the live renderer
  (§18).

### Load-bearing decisions

| # | Decision | Section |
|---|----------|---------|
| ① | Render pipeline = pure view-projection → `ViewBlock` | §3 |
| ② | Flat stream + turn affordances; live→settled work collapse | §4 |
| ③ | Streaming = progressive markdown, safe-prefix | §5 |
| ④ | Markdown for prose; verbatim/structured for machine output | §6 |
| ⑤ | Tool spans = per-tool archetypes + generic fallback | §8 |
| ⑥ | `AgentChanged` in-transcript marker; the conversation keeps its history across a switch | §13 |
| ⑦ | Agent `session.todos` render inline (not in a tray, not in Bridge) | §14 |

---

## 2. Terms

- **Canonical `Item`** — the state model's durable, deduped, on-disk conversation
  unit (`ItemKind`: `Message`, `FunctionCall`, `FunctionCallOutput`,
  `Reasoning`, `NativeTool`, `Compaction`, `SlashCommand`, `TerminalCommand`,
  `ResourceEvent`, `Error`, `AgentChanged`). A **persistence contract** (SQLite,
  dedup key by item **`id`** — persisted `ConversationItem` has **no
  `sequence_number`**). `Error` is a persisted error-banner item
  (`ErrorData{source: llm|execution|tool, code, message}`,
  `entities/conversation.py`) that mirrors `response.error` so the banner
  survives reconnect/refresh — it is operator-visible metadata, not content fed
  to the next turn.
- **`ViewBlock`** — this document's render-only unit (§3). Ephemeral, never
  persisted, never on the wire → free to change shape with zero migration.
- **Turn** — one user message + the agent's whole response (reasoning, tool
  spans, parallel batches, text), keyed by `Item.ctx.turn`.
- **Work section** — the collapsible group of a turn's reasoning + tool spans
  (§4).

---

## 3. The render pipeline — ① (pure view-projection)

Canonical `Item`s do not map 1:1 to rendered units: a tool call pairs with its
output, in-progress text exists before it's an item, the `↻` break is no item
at all. So the transcript renders a **`ViewBlock`** projection — produced by
**pure, composable transforms** over `&[Item]` (+ the RAM-only
`StreamScratch` from the state model), extending the state model's render-time
transforms with pairing/grouping/merge.

```rust
pub enum ViewBlock<'a> {
    Item(&'a Item),                                   // passthrough: message, reasoning,
                                                      //   native_tool, compaction, slash, etc.
    ToolSpan  { call: &'a Item, output: Option<&'a Item> },  // paired by call_id
    SubAgentSpan { child: ChildRef<'a>, output: Option<&'a Item> },  // §8.6
    WorkSection { open: bool, blocks: Vec<ViewBlock<'a>> },  // collapsible reasoning+tools (§4)
    StreamingMessage(&'a str),                        // in-progress assistant text
    StreamingReasoning(&'a str),                      // in-progress reasoning
    OptimisticUser(&'a PendingUserMessage),           // pre-consumed user message (§15)
    CompactionMarker { summary: &'a str, tokens: Option<u64> },  // §10
    AgentChangedMarker { from: &'a AgentId, to: &'a AgentId },  // §13
    ReconnectBreak,                                   // the ↻ marker (§11)
}
```

**Why pure projection** (not items-direct, not a stateful reducer):

- **Items-direct** (widgets pair/group inline) smears pairing, parallel
  grouping, and optimistic-merge into widget code — untestable, and re-implemented
  per surface.
- **Stateful view reducer** reintroduces the second source of truth the state
  model deliberately rejected; two reducers to keep in sync.
- **Pure projection** keeps canonical items the single source of truth; the
  projection is derived/deterministic (no sync hazard), unit-testable,
  framework-neutral, and reused across surfaces by composing transform sets.

`ViewBlock` stays **thin** — mostly `Item(&Item)` passthroughs plus the
handful of composites above. The projection matches `ItemKind` **exhaustively**,
so a server-added item kind is a compile error, not silent drift.

**Transforms** (pure, composable; `pipe`d per surface): `pair_tool_spans`,
`group_work_section`, `merge_optimistic_user`, `flatten_sub_agents` (depth-1,
for peek), `hide_reasoning`, `with_agent_changed_markers`.

> **As-built (T-1, 2026-07-21 — `crates/lens-core/src/reduce/view.rs`).** The
> provisional enum above is the product intent; the implementation slice
> resolved these deviations (rationale in
> `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md` §3.1):
>
> - **`WorkSection { open, blocks }` → `{ response_id: &ResponseId, blocks }`.**
>   `open` is pure UI state (render/T-6 owns expansion); `response_id` (from T-0,
>   authoritative) is the stable grouping key. No `meta` field — the §4 chip
>   (duration/model/tokens/cost/transitions) needs per-turn data T-1 can't supply;
>   deferred whole to **T-6**.
> - **`CompactionMarker` / `AgentChangedMarker` dropped as variants** — they are
>   1:1 item-backed, so they ride as `Item(&Item)` passthroughs; render extracts
>   fields by matching `ItemKind`.
> - **`StreamingMessage(&str)` / `StreamingReasoning(&str)` → borrow the whole
>   accumulator** (`&MessageAcc` / `&ReasoningAcc`) — streaming needs
>   `message_id` for streaming→finalized identity and `summary_text`/`encrypted`
>   for the reasoning cases.
> - **`OptimisticUser` removed** — pending is composer-owned, not a projection
>   input (**T-7**); the projector takes no `pending`.
> - **`SubAgentSpan { child: ChildRef }` removed** — sub-agents are child
>   *sessions* (`session.child_session.*`), not a `ctx.depth` row; the in-transcript
>   span is its own slice **T-5**. `flatten_sub_agents` stays the identity stub.
> - **`ReconnectBreak` deferred to T-2** — zero-field marker with no backing item.
>
> Exhaustiveness is unaffected — the projector matches `ItemKind` with no wildcard.
> Pipeline is staged (filters → `project` → `group_work_section`), pure and
> borrow-only over `(items, scratch, active_response)`; `merge_text_for_display`
> (owned-return) is not wired in.

---

## 4. Transcript skeleton & turn lifecycle — ②

**Flat stream + turn affordances.** The transcript is a flat list of
`ViewBlock`s (keeps streaming-append, virtualization, and cross-surface reuse
cheap), with turn affordances layered as *derived anchors*, not structural
containers (hard turn containers fight virtualization and the streaming-append
case).

**Live → settled collapse.** While a turn runs, the agent's work streams as
flat rows under a response rail (reasoning, tool spans, streaming text). When
the turn ends, reasoning + tool spans **auto-fold into one `WorkSection`**
rendered as a summary chip:

```
▸ worked for 8.1s · Sonnet-4.6 → Opus-4.8 · 12.4k tok · $0.04
```

The final assistant text stays visible; clicking the chip re-reveals the work.

- **Collapse timing:** the *latest* turn's work stays expanded until the
  *next user message* starts a new turn; only then does it collapse. (Older
  turns are always collapsed.) Avoids work vanishing the instant it completes.
- **No-final-text turns** (pure tool work): the chip alone represents the turn.
- **`AgentChanged` marker (0.2.0 — decision J):** an inline `⇄ agent Y → Z`
  marker at the swap point (visible live and inside the expanded work), and
  folded into the chip summary. The transcript **does not remount**; the
  conversation continues across the swap. §13.
- **Mid-stream model change** (via `session.model` event): an inline `⇄ model → X`
  marker at the switch point. A model switch is a meaningful affordance.
- **Per-turn metadata** (model · tokens · cost · duration) lives on the chip.

---

## 5. Streaming & progressive render — ③

**Progressive markdown with a safe prefix.**

- Format every **closed** markdown construct immediately as text streams.
- Hold the **open** trailing construct (unclosed bold, open code fence) as
  plain/pending until it closes, then promote. Reflow is bounded to the
  trailing line; no incomplete-syntax flicker.
- **Payoff:** because streaming already renders markdown, the finalize swap
  (`StreamingMessage` → canonical `Message`) is a near **visual no-op**.

**Locked mechanics (independent of strategy):**

- **Stable widget identity** across streaming→finalized — key by
  response/item id; diff in place, never unmount/remount (a remount is what
  causes a flash or scroll-jump).
- **Coalesce deltas to a frame tick** (~60 fps); never re-render per token.

The markdown **library** is a framework-divergence note (§16); the strategy is
neutral. User messages are *not* streamed (they arrive whole), so safe-prefix
applies only to assistant text and reasoning.

---

## 6. Content rendering — ④ (markdown vs verbatim)

Rendering mode is **per channel**, not global.

| Channel | Mode |
|---|---|
| Assistant message text, reasoning | **markdown** (prose) |
| Code inside fences | **syntax-highlighted** (markdown stops at the fence) |
| Tool output, terminal output, tool args, error text | **verbatim** (never markdown) — + structured per-archetype (§8) |
| User message text | **verbatim + backtick-gated** (§6.2) |

### 6.1 Assistant markdown feature surface

**IN:** GFM core — headings, bold/italic, ordered/unordered/nested lists,
**task lists** `- [ ]`, **tables**, blockquotes, links, inline code, fenced
code + **syntax highlighting**.

- **File-path autolink → editor.** Bare paths (`src/parser.rs`) become
  clickable. **This document** owns detect + paint + emit
  `navigate_to_file(path, line?)`; the click **handler** (resolve, open editor,
  scroll) is the workspace doc.
- **Inline images = IN, artifact-sourced only.** Render images resolving to
  omnigent-served artifacts (a `file_id` / workspace file via the authenticated
  API). A bare external `http(s)://` URL is **not auto-fetched** — renders as
  a link (privacy + safety: an agent emitting `![](http://tracker/?leak=…)`
  must not become a tracking-pixel/SSRF vector). Reused by multimodal
  `input_image` blocks and provider image-gen outputs.
- **DEFER:** math/LaTeX — rare here, heavy dep; render literally.
- **EXCLUDE:** raw HTML passthrough — escaped, never rendered.

### 6.2 User-message rendering — verbatim, backtick-gated

User messages render **verbatim** (deliberately asymmetric from assistant
text: the agent emits markdown by convention; the human pastes raw material —
paths, logs, code). Backticks are the "render richly" gate:

- **Outside backticks:** literal. Implicit inline markdown is NOT honored —
  `*em*`, `_it_`, `**bold**`, `#`, `-`/`1.`, `>`, tables all render literally.
  Whitespace preserved. **Paths/URLs autolinked.**
- `` `inline code` `` → inline code chip.
- ` ```lang ` fenced → syntax-highlighted by language.
- ` ```markdown ` / ` ```md ` → **rendered as formatted markdown** (the user's
  explicit opt-in).
- ` ``` ` untagged → **plain monospace**.

### 6.3 Security boundary — link/image sanitization (uniform across channels)

Every link and image this document renders — assistant markdown (§6.1), user
autolinks and opt-in ` ```markdown ` blocks (§6.2), and any URL surfaced from
content — passes the **same boundary** `framework.md` §2.5 defines:
`validate_link_url` / `validate_image_ref`. Concretely:

- **Block dangerous schemes** — `javascript:`, `file:`, `data:` never become
  clickable or auto-fetched, in *either* assistant or user channels (the §6.2
  user autolink path is not exempt — a pasted `javascript:` URL must not arm).
- **Images** — only artifact-sourced (`file_id` / workspace file via the
  authenticated API) auto-fetch; bare external `http(s)` images render as links,
  never tracking-pixel fetches (§6.1). Apply path-traversal/symlink guards on
  artifact refs.
- This is the same guard `permissions-and-elicitations.md` applies to
  elicitation `params.url` (`validate_elicitation_url`) — one boundary, reused.

---

## 7. Reasoning

The typed client brackets reasoning (`ReasoningStarted` → synthetic
`ReasoningClosed`); the `Reasoning` item carries `full_text`, `summary_text`,
and `encrypted`.

- **Live:** stream the thinking in a small auto-scrolling **capped** region,
  auto-expanded while active, labeled `💭 thinking…`.
- **On close:** collapse to `💭 thought for Ns` (inside the work section,
  which itself collapses after settle per §4).
- **Expanded:** show `summary_text`; `show full reasoning ↗` reveals
  `full_text`. When a harness gives only full text, show that.
- **Encrypted** (`encrypted: true`): `🔒 thought for Ns · reasoning hidden` —
  no expand (no plaintext was received); duration still shown.

---

## 8. Tool spans — ⑤

**Per-tool specialized renderers + generic fallback.** A renderer registry
keyed by canonical tool name; registered tools render bespoke, anything
unregistered (MCP `mcp__*`, provider-native, future/unknown) falls back to the
generic card.

### 8.1 Rendering archetypes (the v1 bespoke set)

| Archetype | Tools | Render |
|---|---|---|
| **File view** | Read, NotebookRead | line-numbered peek |
| **Diff** | Edit, Write, MultiEdit, NotebookEdit | colored +/− hunks |
| **Terminal** | Bash, run_command | ANSI-fidelity output + exit code |
| **Match list** | Grep, Glob, file_search | file:line hits + counts |
| **Checklist** | TodoWrite | checkboxes — renders inline; ties to `session.todos` (§14) |
| **Generic** | MCP + anything unregistered | icon · name · args summary · status · collapsible output |

Registry targets the Claude tool vocabulary first (primary harness), extensible
per-harness; differing names (codex/openai-agents/pi/goose-native/antigravity
/qwen) map onto the same archetypes or fall back. **Not in this registry:**
sub-agent spawn (`Task`) → §8.6; provider-native tools → §9.

### 8.2 Status

Each span carries a status. **The wire enum is `in_progress` / `completed` /
`action_required` / `incomplete`** (`schemas.py:2648-2651`, e.g. a function_call
item's `status: "action_required"`) — **not** `pending`/`running`/`error`. Map
them: `in_progress` = the live→settled running state (§4); `action_required` =
the elicitation path (§18-seam); `completed` = settled; `incomplete` =
interrupted/partial (render the partial, see §11). A tool *error* is not a status
value — it arrives as a `response.error` / persisted `Error` item and gets the
§11 treatment in-span.

### 8.3 Output handling

Cap with **head/tail truncation + a size badge** (not a max-height scrollbox —
that keeps all lines in the DOM, fights virtualization, nests scrolling, and
hides the conclusion). Keeping the **tail** matters: for command output the
result line (`passed`/`FAILED`) is what you need.

**Three size tiers** (tunable defaults; line count + byte backstop):

| Tier | Size | "Show full" |
|---|---|---|
| Small | ≤ 15 lines (≤ 2 KB) | render fully inline, no truncation |
| Medium | 16–60 lines (≤ 10 KB) | **grow inline** |
| Large | > 60 lines or > 10 KB | **route to a surface** |

**Per-archetype tail rule:** Terminal → head + tail (keep result/exit); File
view → head only; Match list → first N + "+K more"; Diff → full if small,
else collapse unchanged context; Generic → head only.

**"Show full" destinations:** Read → **editor tab** (always, regardless of
size — a file's home); large diff → **Review tab** (workspace doc); terminal /
match / generic → **inline-grow**; image / computer-call screenshot → may
**dock into the Canvas tab** (§19-capability-map §0.6) when it warrants a
persistent visual.

### 8.4 The diff source seam

The transcript tool-span diff is computed **client-side from the tool call's
own args** (`old_string`/`new_string` or `content`) — NOT the workspace
`diff/{path}` endpoint. That endpoint (returns `{before, after}`, cumulative
vs. base) feeds the **Review tab** (workspace doc). Two sources, deliberately:
transcript diff = this edit; review diff = the whole change.

### 8.5 Terminal fidelity & the static/live distinction

A `Bash` tool result is a **static capture** — stdout/stderr/exit the agent
already received (a `FunctionCallOutput` item). Render it with **terminal
fidelity** (ANSI color/style parsing, monospace, terminal chrome) — a plain
box would show `\x1b[..]` garbage. It is **distinct from the interactive
Terminal tab** (a live tmux PTY over WS, workspace doc); a Bash result can't
route *into* the live tab. **Seam affordance:** a Bash result offers "⊕ open
interactive terminal in this cwd" → hands to the workspace doc to spawn a
real PTY.

### 8.6 Sub-agent spawn span

The sub-agent topology document owns the overall topology UI; this document
owns only the in-transcript representation, designed **not to prejudge it**:

- Renders as a **collapsed spawn span** in the work section:
  `🤖 <agent> · status · N tools · $cost`, with a **live summary** from
  `ChildSessionSummary` (current task / `last_message_preview` /
  `pending_elicitations_count`) while running. The richer 0.2.0 summary
  enables per-child status pills.
- **On completion the child's final output/summary lands in the transcript** —
  a spawn is a `Task` call whose *result is the child's output*, so it renders
  like a tool result, with the same head/tail truncation (§8.3).
- **`▾ peek`** reveals the child's work inline (depth-1 `flatten_sub_agents`).
- **`open ↗`** navigates to the child's first-class session (free via the
  state model's `navigate_to_session`).
- Collapses into the work section after settle, like any work.

---

## 9. Native tools

Provider-native tools (`NativeTool { tool_type, data }`; `data` is provider-
controlled) follow the same shape as harness tools: **decode the high-value
few + best-effort fallback**, mostly reusing §8 archetypes.

| `tool_type` | Render |
|---|---|
| `web_search_call` | result list |
| `image_generation_call` | inline image (reuse §6.1) |
| `code_interpreter_call` | terminal (reuse) |
| `file_search_call` | match list (reuse) |
| `computer_call` | **screenshot rendered inline** (reuse inline-image); may dock to Canvas for a persistent visual |
| `mcp_call` / unknown | generic fallback |

**Design rule: best-effort decode, graceful fallback.** If expected `data`
fields are absent (provider schema drift), drop to the generic JSON card —
never hard-fail.

---

## 10. Compaction

**Marker only; pre-compaction history stays fully visible.** A divider marks
where the model switched to working from a summary; all prior turns remain
above it; the `compaction` item's summary is expandable on the marker. (The
transcript is the *human's* record, not a mirror of the model's context
window — folding the user's own history fights how scrollback is read.) Perf
of always-visible history is handled by virtualization (§16), not by hiding
semantics. A `compacting…` in-progress indicator shows while
`CompactionInProgress`. **`CompactionFailed` leaves NO permanent marker** —
the source says dismiss the spinner without a marker, because the history was
not modified (`CompactionFailedEvent`, `schemas.py:3207-3215`). Just remove the
`compacting…` indicator; do not write a failure marker into the transcript.

---

## 11. Minor item kinds & markers

- **SlashCommand** → pill marking a slash invocation (`/skill`, `/model`,
  `/compact`).
- **TerminalCommand** → `❯ !cmd` card; a *user-typed* REPL `!` escape, distinct
  from the agent's Bash tool.
- **Error** → typed marker showing `code` + `message`; falls back to `code`
  when `message` is blank (per `ErrorInfo`).
- **Retry** → transient `↻ retrying · attempt N/M · in Ns`; replaced by the
  result when it lands.
- **ReconnectBreak** → `↻ reconnected` hairline break on a non-clean gap
  (mid-stream unpersisted text is gone).

---

## 12. Resource events

The **resource rail** is an **application-shell** navigator surface listing
the session's live `SessionResourceObject`s (env|terminal|file); the resource
*data model* stays with the workspace doc. This document decides which
resource events render inline:

- **File artifact** → **inline downloadable card** (a produced file belongs in
  the flow).
- **Terminal lifecycle** (`terminal.activity` + 0.2.0 `terminal_pending`) →
  **inline actionable marker** + rail. Marker offers `attach ↗` (open the live
  PTY if running). A created long-running terminal is worth surfacing.
- **Env lifecycle** → **rail only** (infrastructure — `default` env,
  terminal-scoped envs; rarely actionable).

tmux scrollback is the only server-side record — there is no client REST endpoint
for captured stdout. Lens keeps one bounded emulator while attached and across
brief reconnects, but deliberately does not persist terminal contents (workspace
doc / capability map §0.7-C).

---

## 13. The AgentChanged marker — ⑥ (decision J)

When `session.agent_changed` fires (state model §12.2), the reducer pushes an
`AgentChanged { from, to, at }` item. **The event carries only `agent_id` +
`agent_name`** (`schemas.py:2218-2221`) — there is no `from` on the wire, so the
reducer **synthesizes `from` from the prior reducer state** and allocates a
**synthetic local item id** (this marker does not arrive from `GET /items` on a
later reconnect — it must be re-synthesized from the snapshot's current agent).
This document renders it as an inline
`⇄ agent Y → Z` marker in the transcript — visible live (during a
switch-agent) and inside the expanded work section after settle. The
conversation **does not remount**; the user sees the prior agent's turns and
the new agent's turns in one scrollback, with the `AgentChanged` marker as
the boundary.

The card and the composer re-render from the updated scalar (new agent_id,
new model, new controls) — handled by the application shell. This document's
job is just the marker + the unbroken transcript history.

---

## 14. The agent's `session.todos` rendered inline — ⑦

`session.todos` (content/status/activeForm) are **the agent's own per-session
task list** — distinct from the Bridge's planning todos (state model
§11). They render **inline in the chat**, in three forms:

1. **Live checklist card** — a `TodoWrite`-archetype tool span (§8.1) renders
   the full list with checkboxes when the agent emits it.
2. **Inline task-completion markers** — when a todo's status transitions to
   `completed`, a `✓ <activeForm>` timeline marker drops into the transcript
   (completions only — tasks in_progress/pending don't get markers; they're
   live in the checklist card).
3. **Inline expanded summary in the work-section chip** — the turn chip
   (§4) shows `activeForm` of the active task alongside the model/tokens/cost
   line, so the per-turn summary reflects what the agent was doing.

The **planning todos** (long-term, cross-session, user-authored or
Concierge-triaged) live in the Bridge — they are never rendered in this
transcript. The two concepts stay separated (capability map §0.6).

The application shell's **volatile Tasks tray** (shell §14) summarizes the
agent's `session.todos` compactly above the composer — it's a folded view of
the same data, not a separate todo list. Clicking a tray item deep-links into
the transcript at the relevant point.

---

## 15. The live turn — input & control — D

- **Optimistic user bubble:** render solid immediately with a transient
  `⋯ sending` tick; on `session.input.consumed` it **settles** (faint `✓`,
  then just a normal message). On POST failure → `⚠ failed · retry`.
- **Composer always sends** — never morphs — so the user can queue/fire
  prompts anytime, including mid-turn.
- **Esc is the only interrupt.** No on-screen Stop button (it would steal the
  send affordance). Cancel is driven by the echoed `session.interrupted` +
  `response.incomplete`.
- **No steering tag.** A message sent while a turn runs looks like any send;
  whether it steers into the current turn or starts the next is not surfaced
  (it doesn't matter to the user).

---

## 16. The scrolling surface — E

- **Stick-to-bottom, don't yank.** Auto-follow the stream while at the bottom;
  the moment the user scrolls up, auto-follow **pauses**. The only affordance
  is a `↓ N new · jump to latest` pill shown **only when scrolled up**; click
  or scroll-to-bottom resumes.

**Four contracts:**

1. **Scroll anchoring** — when a streaming message finalizes (§5) or an item
   above the viewport changes height, hold the viewport on a stable anchor.
2. **Virtualization** (framework-divergence, §16) — long transcripts render
   windowed (visible items + buffer).
3. **Variable heights** — expanded tool spans / code / images → measure-and-
   cache per item; collapse-by-default work sections keep most items small.
4. **New-session jump** — opening a session lands at the **bottom** (latest).

---

## 17. Edge states

- **Empty session:** clean empty state; composer ready; agent greeting if the
  spec defines one.
- **Disk-paint → reconcile:** on open/wake, paint from SQLite **instantly**,
  then reconcile — the **transport-only** typed-client reconnect restores snapshot
  chrome, and the actor forward-catches-up the transcript from `GET /items`,
  merged by item **`id`** (D19; the reader no longer replays items). Content is
  **flash-free** (items keyed by **`id`** — persisted items
  carry no `sequence_number`; `sequence_number` dedup applies only to the live
  SSE overlap window, typed client §7 — diffed in place, no remount; `↻` only on
  a real gap). Show a **debounced `syncing…` indicator** during reconcile — only
  if it takes >~150 ms.
  The focused render window reads finalized items from `TranscriptStore` (disk)
  **steady-state** — not only on open — via an **id-keyed-upsert `RowSource`**
  (reuse retained row entities, never clear-recreate; flash-free handoff
  spike-proven, D23); the live tail above the committed watermark comes from the
  actor's scratch, and finalized items are append-only (no below-watermark
  invalidation).
- **Historical hydration:** items from `GET /items` run through the **same**
  `ViewBlock` projection as live — scrollback looks identical whether it
  streamed live or loaded from disk/server. No separate "history renderer."

---

## 18. Seams (boundaries to sibling documents)

- **Permissions & elicitations — approval docks at the composer.** When the
  agent is blocked on an `ElicitationRequest` (session `Waiting` status), the
  response UI docks at the **composer** (always on-screen, never above the
  fold): binary → `Allow/Deny/Cancel` over `content_preview`; form → a panel
  above the composer (JSON-Schema); url → "Authorize ↗". The **transcript
  shows a record only** — a `⏸ awaiting approval` marker at the gating
  position → a permanent `✓ approved / ✗ denied`. **The resolved event carries
  no verdict** (`response.elicitation_resolved` has only `elicitation_id`,
  `schemas.py:2936-2962`), so the verdict comes from the *locally-submitted*
  `ElicitationResult`. When the prompt resolves with **no local verdict on
  record** — resolved by another client, a timeout, or turn-end — render a
  `↯ cancelled` (or `↯ timed out`) marker rather than guessing approve/deny.
  Off-screen/unfocused sessions light the board "needs you" badge.
  `ElicitationRequestParams.target_session_id` means a child's elicitation may
  mirror up to the parent's transcript — the marker displays "⏸ awaiting
  approval (from sub-agent X)" in that case. **The permissions document owns**
  the response widget + policy + lifecycle; this document docks it and emits
  `approval{action,content}`.
- **Sub-agent topology** — §8.6 (in-transcript span only; topology UI deferred
  to the sub-agent topology document).
- **Workspace & terminals** — the per-edit tool-span diff (ours) vs. the
  Review tab's true git diff (theirs, §8.4); file-path open-handler (§6.1);
  terminal attach / "open interactive terminal" (§8.5, §12).
- **Agent definition** — model / reasoning-effort / collaboration-mode surfaced
  in the turn chip (§4); switch-agent handoff renders the marker here but the
  agent-definition document owns the mechanism.
- **Application shell** — the **resource rail** (§12); the **card preview** is
  coarse metadata, **not the live renderer.** Card visual design is the shell;
  this document only asserts the card never drives the transcript renderer.
- **Bridge** — the agent `session.todos` (rendered inline here, §14) are
  distinct from the Bridge's planning todos. A Bridge reply that
  routes into a session via `POST /comments/send` surfaces in the transcript
  as structured feedback (a benign special-case comment block, not a normal
  user message).
- **Canvas** — when a transcript visual (image-gen output, a computer_call
  screenshot, a code-interpreter plot) warrants persisting as a visual rather
  than an inline image, the transcript offers a "↗ dock to Canvas" affordance
  (§8.3). The Canvas surface (shell §12) receives the payload.

### Surface reuse

The transcript renderer is reused by the **Chat column** (full), a
**read-only History view** (no composer/interrupt), and the **floating
Concierge panel** (transcript + mini-composer in a compact floating container —
shell §13). The **Review tab** and **card preview** are *not* reuses of this
renderer.

---

## 19. Framework-divergence notes

1. **Progressive re-render** (§5) — in-place diff with stable identity. gpui
   entity update vs. an alternative runtime's reconciliation with stable keys.
2. **Markdown / syntax-highlight library** (§6) — gpui markdown + a Rust
   highlighter (tree-sitter / syntect) vs. a JS markdown lib. The safe-prefix
   *strategy* is neutral; the library is not. The framework document's GPUI
   recon notes markdown is the roughest edge — budget a hand-rolled
   `pulldown-cmark`→element renderer + a link/image sanitization boundary.
3. **Virtualization** (§16) — **SPIKED 2026-07-08 → resolved.** gpui
   `uniform_list` assumes uniform row heights and does not fit the
   variable-height transcript — but gpui's native **`list()` / `ListState`** is a
   measure-and-cache variable-height virtualizer purpose-built for chat logs
   (`ListAlignment::Bottom`), and it **satisfies all four §16 contracts**:
   windowing (`renders ≪ N`), variable heights, off-screen-above **anchoring**
   (`logical_scroll_top()` held under a height mutation above the viewport),
   and jump-to-bottom — plus stable identity across recycle and full-height
   markdown-row nesting. No custom virtualizer or fork needed; `list()` (not
   `uniform_list`) is the primitive. gpui-component's virtualized list was tested
   side-by-side and does **not** fit (no bottom-anchoring, no logical-anchor
   readout). Findings:
   [`docs/spikes/2026-07-07-transcript-virtualization.md`](../spikes/2026-07-07-transcript-virtualization.md).

---

## 20. Decisions ledger

| Decision | Choice |
|---|---|
| Render pipeline | Pure view-projection → `ViewBlock` (not items-direct, not stateful reducer) |
| Skeleton | Flat stream + turn affordances; live→settled work collapse |
| AgentChanged | Inline `⇄ agent Y → Z` marker; transcript does not remount (decision J) |
| Streaming text | Progressive markdown, safe-prefix; finalize = visual no-op |
| Render channels | Prose = markdown; machine output = verbatim/structured |
| Assistant markdown | GFM core + task lists + tables + file-path autolink + artifact-only inline images; no math; no HTML |
| User messages | Verbatim + backtick-gated; autolink; asymmetric |
| Reasoning | Stream-capped live; summary→full expand; encrypted placeholder |
| Tool spans | Per-tool archetypes (file/diff/terminal/match/checklist) + generic fallback |
| Output | Head/tail truncation; 3 size tiers; Read→editor, big diff→Review, image/screenshot→Canvas, else inline-grow |
| Bash | Terminal-fidelity static capture; ⊕ open-interactive-terminal seam |
| Native tools | Decode high-value + best-effort fallback; computer_call screenshot inline + Canvas option |
| Compaction | Marker only; history stays; expandable summary |
| Resource events | File inline / terminal inline-actionable + rail / env rail-only |
| Live turn | Optimistic bubble; composer always sends; Esc-only interrupt; no steering tag |
| Scrolling | Stick-don't-yank; jump-pill when scrolled up; 4 contracts |
| Disk-paint | Instant + debounced `syncing…` |
| Approval | Docks at composer; transcript = record marker; target_session_id mirrors child elicitation into parent |
| Sub-agent | Collapsed span + live summary + output-in-transcript + peek + navigate; topology deferred |
| `session.todos` | Rendered inline — checklist card + completion markers + work-section chip (distinct from Bridge planning todos) |
| Review tab | Workspace doc (true git diff + comments) — not this document |

---

## 21. Open questions

- **Truncation thresholds** (§8.3: 15 / 60 lines, 2 / 10 KB) and the
  `syncing…` debounce (~150 ms) are starting values — tune in the verification
  pass.
- **Math/LaTeX** (§6.1) — render literally for v1; revisit if agents emit it.
- **Review tab + inline-comment-to-agent** (§1) — flagged for the workspace
  doc.
- **Card preview + wave effect** — flagged for the application shell.
- **Canvas docking affordance** — when an inline visual warrants persisting as
  a Canvas block, the dock is "↗ dock to Canvas"; the exact MCP payload the
  agent uses to author Canvas blocks is the agent-definition document's call.
