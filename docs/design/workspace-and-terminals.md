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
- **Terminals** — the WS attach client, the ring-buffer reconnect decision
  (capability map §0.7-C), the transfer lifecycle (§9).
- **Worktree provider** — pluggable; default `git worktree`; the new-session
  repo rows (§10).

**This document does NOT own:**

- The navigator panel UI (the application shell).
- The transcript's per-edit tool-span diff (the transcript doc).
- The terminal as a live PTY *rendering* (a working-area tab the shell hosts;
  this document owns the WS data stream + the reconnect buffer).
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

### 9.1 WS attach

`WS /v1/sessions/{id}/resources/terminals/{terminal_id}/attach` — **the `/v1`
prefix IS required.** The router declares the bare `/sessions/.../attach` path
(`terminal_attach.py:103-130`), but `create_app` mounts that router with
`prefix="/v1"` (`app.py:1635-1642`), so the external WS URL carries `/v1` (the
runner proxy + ap-web both use the prefixed URL). The typed client's WS client
(typed client §5) owns the connection; this document owns the Lens-side ring
buffer + the lifecycle.

- **Frames** — binary PTY bytes inbound/outbound; control is text JSON
  (`{"type":"resize", ...}`); a `read_only` query param gates write access.
- **Read-only by default** — `tmux attach -r`. Write attach requires
  `LEVEL_OWNER`.
- **No replay buffer** — live attach only. Reconnect loses scrollback.

### 9.2 Ring-buffer reconnect (decision C — LOCKED)

**The Lens-side ring buffer** keeps scrollback across reconnects. The user-
facing pain of "lost scrollback on reconnect" is high; a ring buffer
(bounded, e.g. 10 MB per terminal — the size is an implementation detail,
not load-bearing on the spec) keeps the live tail visible across stream
interruptions. On reconnect:

1. Stream re-attaches via WS.
2. The ring buffer's contents paint immediately.
3. Live bytes resume appending; the buffer is a circular tail, so only the
   recent N bytes survive (older scrollback ages out).

**Visual cue on reconnect:** a `↻ reconnected` hairline in the terminal
scrollback, mirroring the transcript's reconnect break (transcript doc §11).
Only shown if the buffer's tail < wall-clock-since-disconnect (i.e. we *know*
we missed something); if the buffer holds the whole gap, no cue.

**Scope: brief reconnects only — not a deliberate Sleep.** The ring buffer
covers transient stream blips. It does **not** survive a Lens **Sleep**: Sleep
closes Lens-local observation and sends best-effort `stop_session` (state model
§3), so the server may terminate the tmux PTY. This is why auto-sleep is
**terminal-aware** — a session with live/recent terminal activity is not
"quiet" and is excluded from auto-sleep, so Lens doesn't request stop for a
terminal you were watching.

### 9.3 Terminal lifecycle

- `GET /v1/sessions/{id}/resources/terminals` — list
- `POST /v1/sessions/{id}/resources/terminals` — create (optionally with
  `terminal_launch_args` from the session PATCH)
- `DELETE /v1/sessions/{id}/resources/terminals/{terminal_id}` — destroy
- `POST /v1/sessions/{id}/resources/terminals/{terminal_id}/transfer` — move
  a terminal to another session without closing it (live `/clear` rotation).
  This is a 0.2.0 net-new affordance: when a session's context fills and
  compaction fires, a long-running terminal can be transferred to a fresh
  session — preserves the shell, renews the conversation.

**Events:** `session.terminal.activity` (notification only — actual PTY bytes
come via the WS) + `session.terminal_pending` (0.2.0 — a terminal is about
to be created; gives the UI a chance to pre-paint).

**Switch-agent resets terminals.** A live agent-switch
(`POST /switch-agent`, agent-definition §7) fires the server's
`_reset_runner_resources_after_switch`, so a session's terminals **drop and must
re-attach** after a swap — the transcript survives, the terminals do not. The
terminal tab should show a `↻ re-attaching` state and reconnect when the new
runner's terminals come up.

### 9.4 Shells vs. agent-terminals

Both render in the **same terminal widget** — a tmux PTY is a tmux PTY; the
distinction is purely the `kind` label on the terminal resource. Shells are
user-spawned; agent-terminals are the harness's TUI (e.g. `claude --resume`
for claude-native). Agent-terminals are read-only by default for
non-owners; the owner may write-attach. **No separate UI**; the tab's icon
distinguishes shell vs. agent-terminal.

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

- **Ring buffer size** — 10 MB is a starting point; tune against real
  scrollback usage. The spec is agnostic to the size; the contract is "bounded
  tail + reconnect-safe."
- **Multi-root worktree navigation** — single root by default (decision A,
  resolved §3); when the user opts sibling roots in, the UX for cross-worktree
  search (one query, N roots) is a shell call.
- **Sparse-checkout provider** — the pluggable provider seam (§10) is pinned,
  but the sparse-checkout UX is not; defer to first build.
- **Terminal `transfer` UX** — when a terminal transfers to a new session,
  the old session's terminal tab should close (or re-route); the new session's
  terminal list gains it. The state-model + shell coordination here is
  straightforward but not pinned.
