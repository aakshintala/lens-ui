# Workspace, environments, resources & terminals

The workspace surface: the session's files, env-scoped fs/diff/search/shell
endpoints, the terminal WS attach, the Review tab, and the resource-rail
navigator model. Owns the **data**; the application shell owns the
**navigator surface** (resource rail) per the shell-vs-content split.

**Status:** Draft, 2026-06-23. No pre-existing doc to supersede; written fresh
against omnigent `0.3.0.dev0`.
**Depends on:** the state model (reads `SessionState.workspace`,
`git_branch`, `host_type`, `host_id`, `sandbox_status`; the typed client's
resource sub-services).
**Seams to:** the application shell (the resource-rail navigator panel + the
Review tab container), the transcript doc (per-edit tool-span diff vs. review
diff).

---

## 1. Scope & boundaries

**This document owns:**

- **The environment model** — one primary env per session (`"default"`) +
  optional terminal-scoped envs; the `environment_id` that threads every
  workspace endpoint (§2).
- **Filesystem access** — read / write / edit / delete via the env-scoped
  endpoints; file tree provider (§3).
- **Diff computation** — `diff/{relative_path}` returns `{before, after}`;
  Lens computes hunks client-side (§4).
- **Search** — server-side search via the env-scoped `POST …/search` (§5).
- **Shell** — one-shot command execution (§6).
- **File resources** — upload / list / download / artifact content (§7).
- **Review tab data** — the cumulative git diff of the working tree vs. base,
  driven by `GET changes` + `GET diff/{path}`, with inline comment authoring
  (§8).
- **Terminals** — the WS attach client, retained-emulator reconnect decision
  (capability map §0.7-C), and terminal lifecycle (§9).
- **Worktree provider** — pluggable; default `git worktree`; the new-session
  repo rows (§10).

**This document does NOT own:**

- The navigator panel UI (the application shell).
- The transcript's per-edit tool-span diff (the transcript doc).
- The terminal as a live PTY *rendering* (a working-area tab the shell hosts;
  this document owns the data contract and lifecycle semantics).
- Host filesystem browsing (the server-lifecycle document owns the `/v1/hosts/
  {id}/filesystem` path — useful for new-session repo picking).

---

## 2. The environment model

All workspace endpoints are **environment-scoped** under
`/v1/sessions/{id}/resources/environments/{environment_id}/…` (the typed
client's §3). omnigent models **one primary environment per session**
(`"default"`), with optional terminal-scoped envs (terminals may run in a
distinct env from the primary — e.g. a sandboxed shell separate from the
agent's env).

```rust
pub struct Environment {
    pub id: String,            // "default" or a terminal-scoped id
    pub session_id: SessionId,
    pub connection_id: ConnectionId,
    // ... resource summary
}

pub enum EnvId { Default, TerminalScoped(String) }
```

A session has exactly one `Environment` in the common case; the model supports
N terminal-scoped envs, but only the shell/terminal paths use them. **The
workspace UI never asks the user to pick an environment** — `"default"` is
implicit; terminal-scoped envs are labeled on the terminal tab if non-default.

---

## 3. Filesystem access

Via the env-scoped endpoints (the typed client's Sessions subservice):

| Method | Path (env-scoped) | Purpose |
|---|---|---|
| `GET` | `…/environments/{env_id}/filesystem` | top-level listing (paginated) |
| `GET` | `…/environments/{env_id}/filesystem/{relative_path}` | read a file |
| `PUT` | `…/environments/{env_id}/filesystem/{relative_path}` | **write/replace** — body `{content}` (full-file write) |
| `PATCH` | `…/environments/{env_id}/filesystem/{relative_path}` | **search-replace edit** — body `{old_text, new_text}` |
| `DELETE` | `…/environments/{env_id}/filesystem/{relative_path}` | delete |

**`PUT` and `PATCH` are distinct verbs, not one combined "PATCH write/edit."**
`PUT {content}` does a full-file write/replace; `PATCH {old_text, new_text}` does a
text search-replace (`openapi.json:7100-7158`, `sessions.py:16473-16539`). Lens
must pick the verb by operation — never send a `{content}` body to `PATCH`. All of
these are environment-scoped and the listing is paginated.

The **file tree provider** is the typed client's `Sessions::resources()` —
Lens does not maintain a client-side fs index; the server is the source.

**The File-tab editor** (the surface that reads/writes these files) is a
**"comfortable editor" — top of band 2b** (syntax highlight, line numbers,
find/replace, multi-cursor, folding — all computed from the file bytes), **not an
IDE**. Band-3 language intelligence (completions, diagnostics, go-to-def) is
**structurally unavailable**: Lens is a pure REST/SSE/WS client, the worktree
lives on the omnigent host, and omnigent exposes no LSP-proxy endpoint — so there
is no language server Lens could talk to. The widget-tier decision, its build
plan (vendor-and-patch `gpui-component`'s code input; Zed's editor crate ruled
out as GPL/coupled), and the parked band-3 LSP-proxy contract dependency are
owned by **framework §4.4** (SSOT). This document owns only the **write path**:
edits persist via the §3 verbs — `PUT {content}` full write / `PATCH {old_text,
new_text}` search-replace. The user's own IDE, on the same worktree, is the
band-3 escape hatch.

- **Single root by default; sibling roots opt-in** (capability map decision A,
  resolved). The navigator's tree shows the **focused session's own worktree**
  by default — auto-showing every sibling worktree in its Group invites "which
  branch am I editing?" mistakes. The user can opt sibling roots in (the root
  set is then a derived view over `((group.sessions).workspace)`, not a new
  server concept). Genuine multi-worktree-in-one-session is an omnigent change
  or a faked parent directory of worktrees, not Lens's to simulate.
- **File watch** — the `session.changed_files.invalidated` event drives
  cache invalidation in the file tree + the Review tab. **It is environment-
  scoped** — invalidation applies to a specific `environment_id`, so a
  multi-env session must invalidate per-env, not globally. The typed client
  surfaces it; this document subscribes.

---

## 4. Diff computation

`GET /v1/sessions/{id}/resources/environments/{env_id}/diff/{relative_path}`
returns `{before, after}` strings — **not** a unified diff (verified). Lens
computes hunks client-side via `imara-diff` (Histogram diff), proven in a
reference GPUI app (the framework document's reconnaissance). Render with
native `uniform_list` virtualization; syntax highlight via `syntect`,
precomputed + cached in an `Arc`, not per-render.

**Two sources, deliberately** (transcript doc §8.4):

- **Transcript tool-span diff** — computed from the tool call's args
  (`old_string`/`new_string`); per-edit.
- **Review tab diff** — computed from the workspace `diff/{path}` endpoint;
  cumulative vs. base, the whole change.

This document owns the Review tab's diff source + computation. The transcript
owns the per-edit view.

---

## 5. Search

`POST /v1/sessions/{id}/resources/environments/{env_id}/search` — substring
+ glob include/exclude, capped at 500 hits. The server is the index — Lens
keeps no client index. The application shell's navigator search panel
(⌘⇧F) is where this surfaces; the **state model's** `items` table is a
separate cross-session conversation-content index (not file content search).

---

## 6. Shell

`POST /v1/sessions/{id}/resources/environments/{env_id}/shell` — one-shot
command execution returning `{stdout, stderr, exit, cwd}`. Not the terminal
pane — this is a quick-command surface (e.g. a "run this once" action from the
Review tab or the composer's `!`-bang prefix). The typed client's `Sessions`
subservice exposes it as a typed function.

---

## 7. File resources

| Method | Path | Purpose |
|---|---|---|
| `POST` | `…/resources/files` | upload a file resource |
| `GET` | `…/resources/files` | list |
| `GET` | `…/resources/files/{file_id}` | metadata |
| `GET` | `…/resources/files/{file_id}/content` | content |

Used for **attachments + multimodal input** (the composer's attachment flow)
and **artifact outputs** (the transcript's inline image rendering fetches via
`file_id` — transcript doc §6.1).

---

## 8. The Review tab

A **working-area singleton tab** per session (shell §8.3). Owned by the
application shell as a container; this document owns the data + comment
authoring.

### 8.1 Aggregate diff

- `GET /v1/sessions/{id}/resources/environments/{env_id}/changes` — flat
  changed-files list (path + status: created|modified|deleted).
- For each path, `GET …/diff/{relative_path}` → `{before, after}` →
  client-side hunks (§4).
- **Scroll-to-file** navigation from the volatile Changes tray (shell §14).
- **Per-hunk navigation**: `j/k` prev/next changed file, `n/p` prev/next hunk
  (shell §0.6 power-user keyboard model).

### 8.2 Inline comments + send-to-agent

- `POST /v1/sessions/{id}/comments` — author an anchored comment
  (`AddCommentRequest{path, start_index, end_index, anchor_content?, body}`).
  `anchor_content` is a plain-text snapshot used to re-anchor after edits.
- `PATCH /v1/sessions/{id}/comments/{comment_id}` — edit.
- `DELETE /v1/sessions/{id}/comments/{comment_id}` — withdraw.
- `POST /v1/sessions/{id}/comments/send` — bundle comments as structured
  feedback → the agent revises → a new version; addressed comments resolve.

The annotation engine (shell §16) is the cross-surface primitive; the Review
tab uses it for code diff. The engine handles **line-hash anchoring**
(sha256 over a 5-line window, re-anchor on change) — a stable comment anchor
that survives edits without a separate server round-trip.

### 8.3 Request-Changes bundling

A "Submit review" action bundles all open comments into a single
`POST /comments/send` payload — the agent receives one structured feedback
block and revises. This is the primary "review-then-steer" loop (capability
map §0.3, the highest-value control-room surface).

---

## 9. Terminals

REST/resource-event facts in this section are grounded in
`vendor/omnigent-0.5.1/openapi.json`: the terminal collection/item paths and
`SessionResourceObject`, `ResourceEventData`, resource-created/deleted, and
`SessionSupersededEvent` schemas. WS/internal behavior absent from OpenAPI was
audited at omnigent `08285468` in `server/routes/terminal_attach.py`,
`server/app.py`, `terminals/{ws_bridge,control_bridge}.py`, and
`server/routes/sessions.py`.

### 9.1 WS attach

`WS /v1/sessions/{id}/resources/terminals/{terminal_id}/attach` — **the `/v1`
prefix IS required.** The router declares the bare `/sessions/.../attach` path
(`terminal_attach.py:104-145`), but `create_app` mounts that router with
`prefix="/v1"` (`app.py:2041-2046`), so the external WS URL carries `/v1` (the
runner proxy + the `web` client both use the prefixed URL). The typed client's WS
client (typed client §5) owns the connection; `lens-terminal` owns retained
emulator state and terminal-local lifecycle.

- **Frames** — binary PTY bytes inbound/outbound; control is text JSON
  (`{"type":"resize", ...}`); a `read_only` query param gates write access.
- **Explicit native transport** — Lens requests `transport=pty`; omnigent's
  `control` default captures tmux history for xterm.js and must not be replayed
  into Lens's retained Ghostty engine.
- **Server-authoritative access** — `read_only=true` requires read access,
  drops binary input, retains resize, and uses `tmux attach -r`; interactive
  attach requires `LEVEL_OWNER`.
- **No replay guarantee** — the attach stream has no sequence or replay proof.
  Lens marks every successful post-establishment reconnect as a possible output
  gap.

### 9.2 Retained emulator reconnect (decision C — reconciled)

Lens keeps one bounded Ghostty emulator alive across a brief transport
interruption. There is no second raw-byte ring and no replay into a fresh
engine. The provisional per-terminal scrollback limit is 10 MB (10,000,000
bytes), allocated lazy, with oldest-first eviction and the visible grid always
preserved. On
reconnect:

1. The existing engine and viewport remain visible but read-only.
2. `lens-client` GETs the exact resource ID to verify liveness and checks the
   observed generation signals, then reattaches with `transport=pty` and the
   newest size.
3. Input is re-enabled only after access and resize are re-established.
4. A persistent marker states that output during the interruption may be
   missing.

Retry is automatic for 30 seconds with bounded exponential backoff. Queue
saturation never drops arbitrary PTY chunks: sustained saturation deliberately
disconnects into this same visible flow. `4404`/GET `404` means the terminal is
gone; `4405` immediately becomes `Detached` with an explicit reattach action
while tmux remains alive; `4500` and generic transport failures are retryable.

**Sleep is distinct.** A deliberate Sleep closes the WS and releases the
Ghostty engine/full scrollback. An open tab retains only an immutable final
viewport labeled `Session sleeping`. Wake reattaches only if the same observed
terminal generation survived; otherwise that viewport becomes `Detached`. The
missing immutable token leaves the narrow same-ID race recorded below. Sleep
has no gap marker and never creates a terminal. Scrollback is memory-only and
is also released on tab close or Lens exit.

Fleet policy tracks actual retained bytes. macOS memory warning trims oldest
history from least-recently-viewed hidden terminals first and inserts a visible
truncation marker. Critical pressure deliberately disconnects hidden LRU tabs,
preserves their final viewport, and exposes explicit reattach. The active tab is
kept live and trimmed only as a last resort.

### 9.3 Terminal lifecycle

- `GET /v1/sessions/{id}/resources/terminals` — list
- `POST /v1/sessions/{id}/resources/terminals` — create (optionally with
  `terminal_launch_args` from the session PATCH)
- `GET /v1/sessions/{id}/resources/terminals/{terminal_id}` — fetch one and
  verify that its tmux pane is still live
- `DELETE /v1/sessions/{id}/resources/terminals/{terminal_id}` — destroy

There is **no public transfer endpoint** in the pinned 0.5.1 OpenAPI contract.
Omnigent uses an internal, schema-hidden transfer during native `/clear`, then
publishes public `session.superseded` to the old session. `lens-ui` follows the
target session and tells `lens-terminal` to reattach the same `TerminalId`
under it; Lens never invokes transfer itself.

**Events:** `session.terminal.activity` (notification only — actual PTY bytes
come via the WS) + `session.terminal_pending` (0.2.0 — a terminal is about
to be created; gives the UI a chance to pre-paint), resource created/deleted,
and `session.superseded` for live `/clear` routing.

**Switch-agent resets terminals.** A live agent-switch
(`POST /switch-agent`, agent-definition §7) fires the server's
`_reset_runner_resources_after_switch`, so a session's terminals **drop and must
be replaced** after a swap — the transcript survives, the terminal process does
not. An `OpenOrCreate` tab may wait for the exact-key server-created successor,
show the old final frame, then install a fresh engine. An `Existing` tab never
adopts the successor.

The public target modes are `Existing { session_id, terminal_id }` and
`OpenOrCreate { session_id, key }`. `OpenOrCreate` discovers/creates only during
initial open; later deletion or unexplained disappearance becomes `Detached`
until explicit user action. `Ended` is reserved for positive process-exit
evidence, which 0.5.1 does not expose distinctly from deletion.

Omnigent can recreate a few server-owned terminal roles while reusing their
deterministic ID. Lens treats a second observed `resource.created` as a new
generation and does not mix it into the old engine. The live SSE event is also
normally persisted as a `ResourceEventData` item for reconnect discovery, but
that persistence is best-effort. An immutable server-provided terminal
generation ID therefore remains an upstream contract gap.

### 9.4 Shells vs. agent-terminals

Both render in the **same terminal widget** — a tmux PTY is a tmux PTY. Shells are
user-spawned; agent-terminals are the harness's TUI (e.g. `claude --resume`
for claude-native). Non-owners are read-only; the owner may write-attach.
**No separate renderer**; resource metadata/presentation supplies any icon or
label distinction.

---

## 10. Worktree provider

The new-session flow (shell §7.6) lets the user pick repos + branch policies.
The provider is pluggable:

- **Default: `git worktree`** — the omnigent server creates a worktree
  server-side via `git{branch_name, base_branch?}` on `SessionCreateRequest`.
- **Alternative providers** (e.g. a sparse-checkout variant, an internal
  monorepo-aware provider) bind behind the same seam — the workspace document
  exposes a `WorktreeProvider` trait; the shell's new-session dialog picks
  one per repo row.

`host_type: "external" | "managed"` threads through here: for `managed`, the
server provisions the sandbox host + the workspace, and the **workspace is a
repo URL** (the server clones it), **not a local filesystem path**; for
`external`, the user supplies `host_id` + a local-path `workspace`. The
new-session dialog must collect the right shape per `host_type` (URL field vs.
host filesystem picker). The state model's `SessionState.host_type` is the
source; this document reads it.

---

## 11. The `SessionResourceObject` typed union

The resource model:

```rust
pub enum SessionResourceObject {
    Environment(Environment),
    Terminal(TerminalResource),
    File(FileResource),
}
```

The data is this document's; the **navigator surface** (the resource rail)
is the application shell's — listed in shell §5.4 as a card connection-
state line, and in shell §8.2 as a navigator panel. The shell reads
`SessionState.resource.*` events and renders the rail; this document owns
the typed model the shell consumes.

---

## 12. Open questions

- **Measured memory budget** — 10 MB (10,000,000 bytes) per terminal is
  provisional. Release
  benchmarks on the available Apple Silicon machine must measure real resident
  memory with many hidden streaming tabs before the fleet soft budget is final.
- **Multi-root worktree navigation** — single root by default (decision A,
  resolved §3); when the user opts sibling roots in, the UX for cross-worktree
  search (one query, N roots) is a shell call.
- **Sparse-checkout provider** — the pluggable provider seam (§10) is pinned,
  but the sparse-checkout UX is not; defer to first build.
- **Terminal generation identity** — request an immutable generation/resource
  ID from omnigent so reconnect can prove that a same-ID server recreation is
  the same process rather than relying on best-effort persisted resource events.
