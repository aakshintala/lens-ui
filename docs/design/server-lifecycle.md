# Server lifecycle

The connection + server-side lifecycle: spawning/supervising a local
`omnigent server`, connecting to remote servers as a client, managing hosts
+ runners + managed-sandbox provisioning, running the per-connection auth
handshake (with the typed client), and crash recovery.

**Status:** Draft, 2026-06-23. Written fresh against omnigent `0.3.0.dev0`.
**Depends on:** the typed client (the `Connection`, `Auth`, `Client`, contract
gate).
**Feeds:** the application shell (the health surface, first-run wizard, ＋ Add
connection flow — shell §17.1 / §17.2; the connection-auth UX).

---

## 1. Scope & boundaries

**This document owns:**

- **The connections model** — `ConnectionApp` per omnigent server; local spawn
  vs. remote connect vs. managed-sandbox (§2).
- **Local server spawn/supervise** — Lens as a parent process for
  `omnigent server` on the Mac; lifecycle, crash detection, restart (§3).
- **Connection-auth model** — per-connection `Auth` envelope; how the user
  adds a remote connection with a bearer / cookie / forwarded-email (§4).
- **Hosts registry** — `GET /v1/hosts` (read-only; there is **no**
  `POST`/`DELETE /v1/hosts` — registration is daemon/tunnel-based), per-host
  filesystem browse, the new-session repo-picking affordance (§5).
- **Runner launch** — `POST /v1/hosts/{id}/runners`, the `LaunchRunnerRequest`
  shape, when Lens launches a runner vs. when it lets the server (§6).
- **Managed-sandbox provisioning** — `host_type: "managed"` on session
  create; the server provisions (Modal/Daytona/Islo); `session.sandbox_status`
  events; Lens's role is to *request*, not supervise (§7).
- **Contract gate integration** — `GET /api/version` refuses-on-mismatch (NOT
  `/v1/info`, which has no version); this document owns the *surface* (the
  "wrong version" UI); the typed client owns the *mechanism* (§8).
- **Crash recovery** — what happens when a spawned local server dies; what
  happens when a remote connection drops (§9).
- **Bootstrap / first-run UX backend** — what the server-lifecycle doc does
  behind the shell's first-run wizard (§10).

**This document does NOT own:**

- The wire, endpoints, reconnect protocol, dedup (the typed client).
- The session registry / liveness / pump tasks (the state model).
- The UI chrome / "Add connection" dialog visual (the application shell — this
  document owns the backend that the shell's wizard calls).
- Agent execution (the server + runner do that; Lens never).

---

## 2. The connections model

```rust
pub struct ConnectionApp {
    pub conn: Connection,                    // (id, base_url, auth, info)
    pub client: Arc<Client>,                 // the typed client, constructed by this layer
    pub sessions: HashMap<SessionId, SessionHandle>,  // state-model registry
    pub pinned: HashSet<SessionId>,
    pub active_set: ActiveSet,
    pub health: ConnectionHealth,
    pub poll_task: Option<JoinHandle<()>>,   // the list-poll task (state model §10)
    pub mode: ConnectionMode,                 // spawned | remote-connect | managed-sandbox
}

pub enum ConnectionMode {
    Spawned { child: Child,                   // the local `omnigent server` process
              hermetic_uv_env: PathBuf,       // §3
              pending_restart: bool },
    RemoteConnect,                            // pure client; nothing spawned
    ManagedSandbox,                            // pure client; server provisions the host
}

pub enum ConnectionHealth {
    Up { info: ServerInfo, last_heartbeat: i64 },
    Reconnecting { since: i64, attempts: u32 },
    Down { since: i64, reason: DownReason },   // crashed / refused / auth-expired / contract-mismatch
}
```

Lens holds **N `ConnectionApp`s at once** (capability map decision E). The
local spawned one uses `Auth::None`; remote-connect ones use
`Auth::Bearer/Cookie/ForwardedEmail` per the user's input. Managed-sandbox
connections are `RemoteConnect`-shaped from Lens's side — the *server*
provisions the sandbox host, not Lens.

---

## 3. Local server spawn/supervise

For the local Mac case, Lens spawns `omnigent server` as a child process
and supervises it. This is Lens's only "daemon-like" job — *launch and
supervise*, never execute (capability map §0.2).

**The local server is always-on baseline infrastructure.** Lens spawns and
supervises it on first run **regardless of which work-connections the user
adds** (even a remote-only or managed-only user), because the **Concierge** can
only live on the local server — its runner must reach Lens's local Bridge MCP,
and Lens must control `~/.omnigent/agents/` to write its spec (state model
§12.3). A pure-remote user still gets a local server hosting the Concierge + a
local scratch workspace; remote/managed connections are layered on top
(onboarding §10; shell §17.2).

### 3.1 First-run bootstrap

- **The hermetic `uv` env.** Lens does **not** rely on the user's system
  Python — it bootstraps a pinned, hermetic `uv` environment on first launch
  (download + lock `omnigent==0.3.0.dev0` into Lens's app-support directory; show
  progress; never touch system Python). The capability map §0.8 calls this
  out as the fix for the "is Python installed? is it the right version?" jank
  the existing omnigent installer has.
- **Supervise the HOST DAEMON, not the server alone (0.3 stack change).** In
  omnigent 0.3 the local stack is daemon-fronted: `_ensure_backend` calls
  `_ensure_host_daemon` **first** (`cli.py:2385`, `:2421`), and the host daemon
  (`python -m omnigent.host._daemon_entry`, `cli.py:2236`) is what
  (a) runs `ensure_local_omnigent_server()` to bring the server up **and**
  (b) registers the local host + spawns the runner tunnels. **`omnigent server
  start` alone does NOT start the daemon** — and without the daemon the
  workspace/terminal/filesystem routes (which proxy to the runner) fail, and the
  Concierge has no runner. So Lens supervises the daemon as the parent process,
  not the bare server.
- **The spawn command (pin the actual argv).** Lens execs the daemon via
  `<hermetic-python> -m omnigent.host._daemon_entry` (the local mode is what
  cold-boots the server) — **not** `omnigent server start`. Pass explicit
  `--database-uri` and `--artifact-location` (Lens app-support paths) rather than
  relying on cwd/config discovery. The child inherits Lens's stdout/stderr pipes
  for log capture.
- **Ready-detection ladder.** Poll in order: `GET /health` (process is live) →
  `GET /api/version` (the contract pin matches, §8) → `GET /v1/info` (capability/
  auth posture). Only after `/api/version` passes does the connection flip to
  `Up`. **`/v1/info` is not the version source** — it carries no version field.
- **Failure modes during bootstrap** — `uv` env fetch fails (network), the
  `omnigent` package fails to install (broken release), the **daemon** fails to
  start, or the server-within-the-daemon fails (port collision, missing tmux).
  Each surfaces an actionable error in the first-run wizard (the shell owns the
  visual; this document owns the detected state + remediation steps).

### 3.2 Steady-state supervise

- **Health monitor** — a heartbeat watcher on the server (the typed client's
  SSE heartbeat on each open stream + a periodic `GET /health`/`GET /api/version`
  ping). If the daemon/server process dies, the watcher flips
  `ConnectionHealth::Down` with the reason.
- **Auto-restart** — on unexpected child-process exit, Lens restarts the daemon
  once after a short backoff. If the restart fails, the connection stays Down; the
  shell surfaces "Local server down — retry / diagnose" (§9).
- **Graceful shutdown — on full quit (⌘Q) only, daemon-first.** Lens is a
  **resident** menu-bar app (shell §17.4): **closing the window (⌘W) does NOT
  tear down the local stack** — it must stay alive so the Concierge, the
  background poll, and native notifications keep working while Lens is
  backgrounded. Only **⌘Q** shuts it down, in order: **stop the host daemon
  first, then the server** (the daemon owns the runner tunnels; tearing down the
  server out from under a live daemon orphans runners). Send `SIGTERM`, wait
  briefly, escalate to `SIGKILL` if it hangs. Sessions running on the server
  continue regardless (a later Lens start brings the stack back; `--resume`
  semantics recover sessions — the state model reconciles from `GET /items`).

### 3.3 What lives in the spawned env

The `uv` env carries:
- `omnigent` (pinned to the contract-gate version `0.3.0.dev0`, §8)
- `tmux` (the runner's PTY substrate; verified README §Quick start)
- Anything `omnigent` declares as a runtime dep (e.g. its own version of
  `python>=3.12`)

Lens **does not** ship its own Python runtime — it uses the user's Python via
the `uv` env, which is hermetic to the env. Cross-platform sandbox handling
(the Linux case uses `bwrap` instead of seatbelt; macOS uses the built-in
`sandbox` (`seatbelt`)) is omnigent's concern, not Lens's.

---

## 4. Connection-auth model (remote + managed-sandbox)

For remote and managed-sandbox connections, Lens is a pure client — no
supervise. The user adds a connection in the shell's first-run / add-
connection flow with:

| Input | `Auth` enum variant | Used for |
|---|---|---|
| (none — local) | `None` | the spawned local server |
| Bearer token (paste) | `Bearer { token }` | an internal dev workspace that hands out bearer tokens |
| Cookie (paste from browser) | `Cookie { value }` | an OIDC cookie set by a corporate IdP; Lens carries the cookie header |
| Forwarded email | `ForwardedEmail { email }` | `X-Forwarded-Email` — omnigent's default `header` auth mode (`auth.py`; header name configurable via `OMNIGENT_AUTH_HEADER`, the Databricks/Cloudflare-Access convention) |

> **`X-Forwarded-Email` is trusted-proxy auth, not a credential** — the header
> is meant to be injected by an authenticating reverse proxy that strips any
> client-supplied copy. Lens setting it directly is only meaningful on a
> **trusted network / direct-header dev box** (your internal-dev-workspace use
> case), where the server trusts the header. In a **proxied corporate-IdP**
> deployment Lens can't assert an identity past the proxy — the real credential
> is the **OIDC cookie** the proxy mints (`Cookie { value }`). Treat
> `ForwardedEmail` as the trusted-network shape and `Cookie`/`Bearer` as the
> over-the-internet shapes.

**Lens never owns the auth flow itself** — it stores the credential
per-connection (in an OS keychain on macOS, encrypted at rest) and presents
it on every HTTP request + WS upgrade. Re-login happens off-app (browser,
CLI) and the user re-pastes the new credential; Lens doesn't refresh tokens.

The typed client's `Auth` enum (typed client §2) is the wire-side type;
this document owns the *user-facing* add-connection flow that produces it.

---

## 5. Hosts registry

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/hosts` | list registered hosts |
| `GET` | `/v1/hosts/{host_id}` | one host |
| `POST` | `/v1/hosts/{id}/directories` | create a directory on the host (owner-scoped) |
| `GET` | `/v1/hosts/{id}/filesystem[/{path}]` | browse host filesystem |

**There is NO `POST /v1/hosts` or `DELETE /v1/hosts/{id}`.** Hosts are read-only
over REST (`GET` only). A host appears in the registry by **registering itself via
an outbound WS tunnel** (`omnigent host` / `host_tunnel.py`) or via managed
provisioning — not via a REST create. Host *creation* in the workspace sense is
`POST …/directories` (a folder) and `POST …/runners` (a runner), not host CRUD.

**Per-host filesystem browse** is the useful net-new affordance for the new-
session dialog — instead of typing a workspace path, the user browses the
host's filesystem tree (server-side). The shell's new-session repo rows
(shell §7.6) use this endpoint to populate the picker. This document owns the
typed-client surface (the typed client exposes `Hosts::filesystem()` as a
typed fn); the shell owns the visual.

**Remote-host registration** — for a dedicated dev box, the host registers
*itself* by running `omnigent host` (out-of-band, on the remote box), which opens
the outbound WS tunnel to the server; the server then RPC-launches a runner on it
(§6). Lens does **not** POST a host record — there is no such endpoint. Lens's
role is to surface the registered hosts and drive runner launch.

---

## 6. Runner launch

`POST /v1/hosts/{host_id}/runners` with `LaunchRunnerRequest`:

```rust
pub struct LaunchRunnerRequest {
    pub session_id: SessionId,
    pub workspace: String,           // absolute path on the host
    pub git: Option<SessionGitOptions>,  // branch_name + base_branch — creates a worktree
}
```

**Who triggers it:**

- **External + dedicated host** — Lens's new-session dialog (shell §7.6)
  picks a host + a workspace + optional git worktree params; this document
  fires the `POST /runners`. The server launches a runner on the host that
  binds to the session (atomic PATCH /v1/sessions/{id} runner_id).
- **External + local** — the local server embeds its own runner (capability
  map §0.2), no launch needed.
- **Managed** — Lens doesn't `POST /runners`. The server provisions a
  sandbox host and launches a runner on it (§7).

**Wake / resume a slept session.** Waking a session whose runner was reclaimed
(Sleep → `stop_session`) is a runner *relaunch*, not just a reconnect. Sequence:
1. (optional) `GET /v1/runners/{runner_id}/status` → `{runner_id, online}` to see
   if the prior runner is already back.
2. `POST /v1/hosts/{host_id}/runners` (`LaunchRunnerRequest{session_id, workspace,
   git?}`) to relaunch the runner.
3. `PATCH /v1/sessions/{id}` with the new `runner_id` to rebind the session.
4. Reconnect the SSE stream (typed client §7) and reconcile from snapshot +
   `GET /items`.
Handle a `409` on relaunch (a runner is already coming up) by polling
`runners/{id}/status` and rebinding rather than launching a second runner.

---

## 7. Managed-sandbox provisioning

`host_type: "managed"` on `SessionCreateRequest` (capability map §0.3) —
the server provisions a sandbox host (Modal / Daytona / Islo per README)
*and* launches a runner on it, all server-side. Lens:

- Requests it via the new-session dialog's **"host type: managed"** selector
  (shell §7.6; the server-lifecycle doc owns the underlying call).
- Subscribes to `session.sandbox_status` events for provisioning progress.
  **The stage enum is `provisioning → cloning → starting → connecting → ready`,
  or `failed`** (`SandboxStatus`, `openapi.json` ~2943; "sandbox provision →
  repository clone → host startup → runner connect → ready"). **There is no
  `queued` stage.** On reconnect, seed the indicator from the snapshot's
  `sandbox_status` field (the event itself is transient/SSE-only).
- Does **not** supervise — the server owns the sandbox lifecycle; Lens just
  connects to the resulting session.

The application shell surfaces the provisioning state on the card as a
sandbox badge ("sandbox provisioning…"). Lens can offer "cancel" —
`DELETE /v1/sessions/{id}` tears down the session + the sandbox.

---

## 8. Contract gate integration

The typed client owns the *mechanism* (typed client §8): `GET /api/version`,
compare against `PINNED_OMNIGENT_VERSION`, return `Err(ContractMismatch)` on
mismatch. (`/v1/info` carries no version — it is the capability/auth probe.) This
document owns the *surface* — what the user sees:

- **First-run / new-connection:** the contract gate runs at handshake. On
  mismatch, the "add connection" flow fails with a visible "wrong omnigent
  version — Lens expects 0.3.0.dev0, server reports X.Y.Z" message and two
  remediation paths:
  - "Upgrade Lens" (if the server is newer than Lens's pin) → link to
    upgrade.
  - "Downgrade omnigent" (if Lens is newer than the server) → instructions
  (e.g. `uv tool install omnigent==0.3.0.dev0`).
- **Drift at runtime:** unlikely (the server reports version on handshake
  only), but if the contract shape changes mid-session (e.g. via a server
  hot-reload), the typed client's startup-taxonomy-diff at next reconnect
  catches it and flips the connection to `Down { reason: ContractMismatch }`.

**Contract-mismatch never silently continues.** The whole point of the gate
(capability map §0.1) is a UI bug must never masquerade as a server bug —
the user sees the real reason.

---

## 9. Crash / disconnect recovery

### 9.1 Local spawned server dies

- The child-process watcher flips `ConnectionHealth::Down` with the reason
  (exit code, signal).
- Lens attempts one auto-restart (§3.2). If it fails, the connection stays
  Down; the shell surfaces "Local server down — retry / diagnose".
- **Session state is safe** — every session on the dead server was
  persisted server-side; when the server comes back, the state model's
  reconnect (snapshot + `GET /items` + dedup) reconciles cleanly. Only
  in-flight live-typing deltas are lost (the SSE stream was no-replay);
  the transcript's `↻ reconnected` break marks the gap.
- **In-flight elicitations** — a pending elicitation on a session whose
  server crashed may or may not survive; the state model treats
  `response.elicitation_resolved` as the cleanup signal and clears on
  reconnect if the elicitation is gone from the snapshot.

### 9.2 Remote-connect / managed-sandbox drops

- The typed client's reconnect loop handles transient network blips
  transparently (the typed client §7). The user sees a thin amber "↻
  Reconnecting" indicator only if it lingers.
- If the reconnect gives up (`ClientError::Disconnected`), the
  `ConnectionHealth::Down` flips; this document surfaces "Remote server
  unreachable — retry / switch to read-only". Sessions on a dead remote
  still have their local SQLite snapshots — the state model lets the user
  read their history offline (read-only-transcript).
- For **managed-sandbox** drops — the sandbox host died; the server is still
  up (it's a different process). The session on Lens's side just loses its
  stream; the server's session might still exist (it depends on the sandbox
  provider). The shell surfaces "Sandbox host offline — wait for
  re-provisioning" if `session.sandbox_status` indicates the server is
  bringing a new sandbox up.

---

## 10. Bootstrap / first-run UX backend

The shell's first-run wizard (shell §17.2) — "add your first connection" —
is backed by this document. **Lens always bootstraps the local server first**
(§3, the Concierge's home), then adds the user's first *work* connection:

| Shell flow choice | What this document does |
|---|---|
| **Local** | bootstrap the `uv` env (§3.1); spawn the **host daemon** (`-m omnigent.host._daemon_entry`, which brings up the server + runner tunnels); ready ladder `GET /health` → `GET /api/version` (contract gate, §8) → `GET /v1/info`; flip to `Up`. |
| **Remote** | prompt for base URL + auth method (§4); construct `Connection { base_url, auth, … }`; typed-client `Client::new`; contract gate; flip to `Up`. |
| **Managed sandbox** | remote-connect-shaped (the server is already up); prompt for the remote server URL + auth; the user creates managed-sandbox sessions via the new-session dialog (§7). |

---

## 11. Open questions

- **`uv` env relocation** — if the user moves Lens's app-support directory
  (rare), the env path breaks. The env path is stored in a config, not
  hard-coded, so a re-bootstrap is the recovery.
- **Server log capture** — Lens captures the spawned server's stdout/stderr
  for diagnostics; the surfaced UI (a "Server logs" view) is the shell's
  job. The rotation / retention policy is a forward call.
- **Remote host install of `omnigent host`** — out-of-band (the user SSHs
  in and runs the install). Lens could automate it via SSH; deferred.
- **Managed-sandbox provider selection** — the new-session dialog picks
  "managed" but doesn't (yet) pick Modal vs. Daytona vs. Islo; the server
  selects. Whether Lens surfaces the provider choice is a future call.