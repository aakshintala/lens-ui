# Permissions, elicitations & sharing

The permission path: form/URL elicitations, the `/resolve` REST path,
`target_session_id` child→ancestor mirror routing, the codex permission hook,
per-session sharing + the policy editor + identity.

**Status:** Draft, 2026-06-23. Written fresh against omnigent `0.3.0.dev0`.
**Depends on:** the state model (reads `SessionState.pending_elicitations`
(plural — see §2), `permission_level`, `owner`, `presence`).
**Seams to:** the transcript doc (docks the approval widget at the composer;
transcript shows the record marker only), the application shell (the "you
don't own this session" affordances + the shared-session indicator on the
card), the typed client (the elicitations + hooks + permissions subservices).

---

## 1. Scope & boundaries

**This document owns:**

- **The elicitation lifecycle** — from `response.elicitation_request` through
  the user's verdict (§2).
- **The elicitation widget** — the form/url renderer + the composer dock
  (with the transcript doc; this document owns behavior, transcript owns
  placement) (§3).
- **The reply paths** — `POST /events type=approval` vs. `POST /elicitations/
  {id}/resolve`; when to use which (§4).
- **`target_session_id` mirror routing** — a child's elicitation mirroring
  into the parent's stream (§5).
- **Native permission hooks** — claude-native `hooks/permission-request`,
  codex `hooks/codex-elicitation-request` (§6).
- **Sharing & permissions** — per-session grants (`PUT /permissions`), owner
  (`GET /owner`), `__public__` (§7).
- **The policy editor** — server-wide + per-session policies, the catalog,
  evaluate (§8).
- **Identity & ownership chrome** — `GET /v1/me`, the "shared" indicator, the
  "you don't own this" affordance (§9).

**This document does NOT own:**

- The composer UI (the application shell — this document owns the *widget's
  behavior*, the shell owns where it docks).
- The card's connection/host/owner badges (the application shell — this
  document supplies the data).
- Elicitation event parsing (the typed client — this document consumes the
  typed `ElicitationRequestEvent`).
- Policy enforcement (server-side — this document surfaces the editor + the
  evaluate endpoint, the server runs the policy).

---

## 2. The elicitation lifecycle

`response.elicitation_request` arrives with `ElicitationRequestParams`:

```rust
pub struct ElicitationRequestParams {
    pub mode: ElicitationMode,             // Form | Url
    pub message: String,                   // human-readable prompt
    pub requested_schema: Option<Value>,   // JSON Schema (form mode)
    pub url: Option<String>,               // url mode — currently a RELATIVE same-origin
                                           // approval page: `/approve/{session_id}/{elicitation_id}`
                                           // (approval.py:209). NOT an external OAuth URL today,
                                           // though the MCP schema prose is generic. Must be
                                           // validated (see §3/§4) and resolved against base_url.
    pub phase: Option<String>,             // pre_tool_use | post_tool_use | input | output
    pub policy_name: Option<String>,       // the policy that triggered
    pub content_preview: Option<String>,   // truncated preview of the underlying request
    pub target_session_id: Option<SessionId>, // mirror routing (§5)
}

pub enum ElicitationMode { Form, Url }
```

The state model folds this into `SessionState.pending_elicitations` (a
`Vec`/map — see plural note below). The
transcript's job (transcript doc §18) is to show a `⏸ awaiting approval`
marker at the gating position and, for the focused session, dock the widget
(per §3 below) at the composer. The application shell lights the board "needs
you" badge for unfocused pending-elicitation sessions.

On verdict (or timeout / cancel / turn-end), `response.elicitation_resolved`
arrives — but it carries **only `type` + `elicitation_id`, no verdict**
(`ElicitationResolvedEvent`, `schemas.py:2936-2962`); approvals are not persisted
as conversation items. So Lens must **record the verdict locally at submit time**
(when it sends the `ElicitationResult`) and key the transcript marker off that
local record: `✓ approved` / `✗ denied` / `↯ cancelled`. When `resolved` arrives
**without** a prior local verdict (resolved by another client, a timeout, or
turn-end), default the marker to `↯ cancelled` ("resolved elsewhere / timed out").
The state model clears `pending_elicitation`; the Bridge badge decrements in
lockstep and also clears idempotently when `pending_elicitations_count` polls
`N → 0` (state model §11).

**Pending state is plural.** `SessionResponse.pending_elicitations` is a `list`
(`schemas.py:1630`), and a fan-out parent mirrors multiple child prompts at once
(§5). The state model holds a `Vec`/map keyed by `elicitation_id`/
`target_session_id`, not a single `Option`; the composer docks one focused prompt
while the count drives badges.

---

## 3. The elicitation widget

Docked at the **composer** (always on-screen, never above the fold):

- **Binary (no `requested_schema` or `{approve: boolean}`):** `Allow | Deny |
  Cancel` over `content_preview`.
- **Form (`mode=form` with `requested_schema`):** a panel above the composer
  rendering the JSON Schema as a form. Submit returns the form values in
  `ElicitationResult.content`.
- **URL (`mode=url`):** an "Authorize ↗" button. **Today the current omnigent
  producer sets a RELATIVE same-origin approval page** (`/approve/{session_id}/
  {elicitation_id}`, `approval.py:209`), NOT an arbitrary external OAuth URL —
  even though the MCP schema prose describes url-mode generically as "external".
  Lens must therefore run `validate_elicitation_url` before acting:
  - If the value is **relative**, resolve it against the connection's `base_url`
    and open `{base_url}/approve/…` (the same-origin server approval page).
  - If a future server emits an **absolute** URL, validate scheme (`https` only —
    block `javascript:`/`file:`/`data:`) and surface the origin to the user;
    **never blindly `open()`** an unvalidated absolute URL.
  The `POST /elicitations/{id}/resolve` endpoint is the preferred reply path for
  url-mode (cleaner than `POST /events`).

The transcript doc owns the visual placement; this document owns the widget's
behavior + the reply submission. `validate_elicitation_url` mirrors framework
§2.5's `validate_link_url`/`validate_image_ref` boundary — the same scheme/origin
guard, applied to elicitation URLs.

---

## 4. Reply paths

Two replies are supported (capability map §0.3):

- **`POST /v1/sessions/{id}/events` with `type == "approval"`** — body
  `{elicitation_id, action: accept|decline|cancel, content?}` in the
  `SessionEventInput.data` envelope. The original path; confirmed.
- **`POST /v1/sessions/{id}/elicitations/{elicitation_id}/resolve`** — RESTful
  counterpart, body `ElicitationResult{action, content?}`. Preferred for
  url-mode OAuth (deep-linkable + clean); also usable for form mode.

**When to use which:**

| Case | Path |
|---|---|
| form-mode elicitation resolved in-session | `POST /events` — typed client's `SessionEventInput::Approval` |
| url-mode OAuth callback | `POST /resolve` — clean REST, no need to thread `data` |
| Deep-link from a notification / external approval page | `POST /resolve` — the `GET /v1/sessions/{id}/elicitations/{elicitation_id}` endpoint backs the standalone approval page |

The typed client exposes both as typed functions; this document picks based on
mode.

---

## 5. `target_session_id` mirror routing

0.2.0 net-new: `ElicitationRequestParams.target_session_id` is `Some` when a
child session's elicitation is **mirrored up into an ancestor's stream** —
e.g. a sub-agent hits a decision point and the parent's user needs to
approve. The `target_session_id` tells the consumer which session's resolve
endpoint owns the elicitation.

**Lens-side handling:**

- If the focused session is the **target**, render the elicitation widget
  normally; the transcript shows `⏸ awaiting approval (from sub-agent X)`.
- If a **child** is the target and the parent is focused, the widget docks in
  the parent's composer; the verdict resolves via
  `POST /v1/sessions/{target_session_id}/elicitations/{id}/resolve` — NOT
  via the parent's events endpoint. The typed client routes accordingly.
- The Bridge surfaces mirrored elicitations with a "from sub-agent X"
  label so the user knows where the decision is coming from.

---

## 6. Native permission hooks

**Four** specialized endpoints feed the same elicitation UI (all server-initiated
— the harness-side adapter POSTs them inbound; Lens never calls them):

- **Generic / claude-native: `POST /v1/sessions/{id}/hooks/permission-request`** —
  the server parks a Future on this hook; Lens's elicitation widget resolves it.
  Identical UX to the `response.elicitation_request` path; the hook is the wire
  mechanism.
- **Codex: `POST /v1/sessions/{id}/hooks/codex-elicitation-request`** — codex-specific.
- **Antigravity: `POST /v1/sessions/{id}/hooks/antigravity-elicitation-request`**
  (`openapi.json:5739`).
- **Cursor: `POST /v1/sessions/{id}/hooks/cursor-permission-request`**
  (`openapi.json:5821`).

The typed client surfaces all four as the same typed `ElicitationRequest`
shape; this document treats them uniformly. The elicitation UI and the
`external_elicitation_resolved` race handling (a TUI can win the approval before
Lens does — clear the widget when the resolved event arrives without a local
verdict) must tolerate **all four** harness sources, not just the first two.

---

## 7. Sharing & permissions

omnigent supports per-session grants:

| Method | Path | Purpose |
|---|---|---|
| `PUT` | `/v1/sessions/{id}/permissions` | grant — `{user_id, level}` where **`level` is 1–3 only** (`GrantPermissionRequest.level = Field(ge=1, le=3)`, `schemas.py:1905`): 1=read, 2=edit, 3=manage; `__public__` capped at read |
| `GET` | `/v1/sessions/{id}/permissions` | list grants |
| `DELETE` | `/v1/sessions/{id}/permissions/{target_user_id}` | revoke |
| `GET` | `/v1/sessions/{id}/owner` | the owner identity |

**Grant levels are 1–3, not 1–4.** Owner (`LEVEL_OWNER = 4`) is creator/admin-
derived and **cannot be granted** — the grant route 403s on attempts to modify
owner permissions (`sessions.py:18054-18113`). So the sharing dialog offers
read/edit/manage; ownership is conferred only at creation. Share-link requires
≥ manage (3), which is correct.

The **sharing dialog** (a card action — shell §5.3 kebab "Share link") exposes
grant + revoke + the public-read toggle. The owner readout lives in the
session header (shell §7.4).

**Lens is a single user *of* each remote server** (capability map decision E):
when connected to an authed remote server, the user identity comes from
`GET /v1/me`. A session may be owned by a teammate; Lens surfaces "you don't
own this session" / "shared by X" affordances via:

- `permission_level < 4` → "Switch agent" disabled. **This is a Lens UI policy
  (decision J), stricter than the API** — the `switch-agent` route's actual floor
  is `LEVEL_EDIT (2)` (`sessions.py:14214`), not owner. Lens chooses owner-only as
  product policy; the server would accept an editor. Also idle-only (disabled
  while a turn is running) and hidden for sub-agent sessions (agent-definition §7).
- `permission_level < 2` → no composer (read-only).
- `owner != me` → "shared" indicator on the card + in the session header.

The **identity** driving these — `GET /v1/me` — is stored on the
`ConnectionApp` (state model §9) and is per-connection.

---

## 8. The policy editor

Policies stack across three levels: **server-wide** (admin), **per-agent**
(developer in the YAML spec), **per-session** (you). Stricter session rules
are checked first (capability map §0.3).

| Method | Path | Purpose |
|---|---|---|
| `GET/POST/PATCH/DELETE` | `/v1/policies[/{policy_id}]` | server-wide policies |
| `GET` | `/v1/policy-registry` | the policy catalog (what can be attached) |
| `GET/POST` | `/v1/sessions/{id}/policies` | session-scoped policies |
| `GET/DELETE` | `/v1/sessions/{id}/policies/{policy_id}` | one session policy |
| `POST` | `/v1/sessions/{id}/policies/evaluate` | dry-run evaluate a policy against hypothetical input |

**The policy editor surface** (full scope per capability map):

- **Catalog browse** — the registry's policy list with descriptions + factory
  params.
- **Attach to session/agent/server** — bindings.
- **Evaluate** — the dry-run against hypothetical input, useful for "what
  would this policy do if the agent tried X?"
- **Results** — recent evaluations + the per-session policy stack summary.

The editor is a **working-area tab** (singleton per focused session), peer
to Review and Bridge. Permissions document owns the tab's content; the shell
owns its placement as a working-area launcher.

---

## 9. Identity & ownership chrome

`GET /v1/me` returns the current user per connection. `GET /v1/sessions/{id}/
owner` returns the owner of a specific session.

- **Connection identity** — drives the user's name in the rail, the "me" in
  the presence/co-viewer list.
- **Session ownership** — drives:
  - The card's "shared by X" badge when `owner != me`.
  - The session header's "you don't own this" affordance.
  - The card kebab's "Share link" affordance (enabled only when
    `permission_level >= 3`).
- **Presence + ownership** — the wire `PresenceViewer` is **only**
  `{user_id, joined_at, idle}` (`schemas.py:2787-2804`) — there is **no
  `is_owner`/`display_name`/`last_seen_at`**. Owner identity is derived
  separately from `GET /v1/sessions/{id}/owner` + `GET /v1/me` +
  `permission_level`, then joined against the viewer list by `user_id`. The
  header's "X, Y also viewing" chrome highlights the owner via that derivation,
  not a presence field.

**Auth is a provider matrix, not a single flag.** The server's `auth_provider`
is one of (`server/auth.py:15-20`):
- **`header`** — trusts an upstream-injected identity header (e.g.
  `Cf-Access-Authenticated-User-Email`); requests without it 401 **unless** the
  server was started single-user (`OMNIGENT_LOCAL_SINGLE_USER=1`), where the
  user falls back to the reserved `"local"`.
- **`oidc`** — `__Host-ap_session` signed cookie from an OIDC auth-code+PKCE flow.
- **`accounts`** — same cookie machinery, minted by the built-in
  username+password `/auth/login` (first-user-is-admin, invite-only signup).

Lens reads the posture from `GET /v1/info` (`accounts_enabled`, `login_url`,
`needs_setup`) at handshake. On a `401`, Lens must surface the server's
`login_url` and route the user to authenticate (cookie or header), not silently
fail. In the local single-user case `owner == "local"` and the ownership
affordances collapse to a no-op (everything is "yours").

---

## 10. Open questions

- **Policy authoring UX** — the editor surfaces the catalog + attach + evaluate;
  authoring a new policy (writing the handler) is a filesystem pass (the
  registry was authored in YAML / Python). A form-based UI is future.
- **Public-read toggle** — `__public__` is supported; the UX for "anyone with
  the link can read" is a one-click toggle in the sharing dialog. The link
  itself is a Lens URL (e.g. `lens://session/{connection_id}/{session_id}`)
  — pinned when the deep-link handler lands.
- **Notification plumbing for remote approvals** — **resolved** (shell §17.4):
  Lens is a **resident** app (menu-bar) with a background poll, so a pending /
  mirrored elicitation fires a **native OS notification + `lens://elicitation/…`
  deep-link** whenever Lens is running or backgrounded. Only a *fully-quit* Lens
  (⌘Q) misses live notifications (caught up via the Inbox on relaunch); a
  server-side push channel for the fully-quit case is a v2 reach behind a clean
  seam.