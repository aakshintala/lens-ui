# Task 3 (T3-2 Boundary + autolink) — Report

## Status

**DONE** — both plan commits landed; gate green.

## Commits

| SHA | Message |
|-----|---------|
| `345aa1a` | `feat(security): validate_link_url + validate_image_ref boundary` |
| `5416773` | `feat(security): paint-time P5/P6 gates + file-path autolink emit` |

## Test summary

`cargo test -p lens-ui` → **242 passed** (227 unit + 15 integration, 2 real-window ignored); `security_adversarial` 14/14; `cargo check -p lens-ui -p lens-app`; `cargo clippy -p lens-ui -p lens-core --all-targets` and with `--features probe` → **0 warnings**.

## What shipped

### `security.rs` (strict workspace lints)
- `validate_link_url` → `LinkVerdict::{AllowOpenUrl, NavigateToFile, Strip}`
- `validate_image_ref` → `ImageVerdict::{AllowArtifactImg, RenderAsLink, Strip}`
- 13 unit tests in-module + extended hostile coverage in integration fixtures

### P5 — image gate (`md/node.rs:~609`)
- Replaced unvalidated `img(image.url)` with `validate_image_ref` match
- **Zero** non-comment `img(` calls remain in `node.rs` (static adversarial test enforces)
- Artifact refs → `[artifact: …]` placeholder (§8 deferral, no fetch)
- Remote/data refs → `[image: …]` placeholder (no fetch)
- Strip → no element pushed

### P6 — link strip (`md/inline.rs:359`, `md/node.rs` link-mark paint)
- Click path routes through `validate_link_url`; `NavigateToFile` → `emit_navigate_to_file`
- Paint-time: links failing validation omit `LinkMark` (no underline/color, not in `links` vec → not clickable)

### Autolink + emit
- `focused/autolink.rs` — `scan_prose_autolinks` (URL + file-path tokens)
- `focused/content_events.rs` — thread-local `ContentUiEvent` sink + `emit_navigate_to_file`

## Scheme allow / deny lists

### `validate_link_url`

| Verdict | Condition |
|---------|-----------|
| **AllowOpenUrl** | `http://` or `https://` prefix (case-insensitive); non-empty path/rest; no spaces; no control chars; len ≤ 8 KiB |
| **NavigateToFile** | Path-shaped: starts `./` or `../`, contains `/`, or ends `.rs`/`.md`; optional `:line` suffix (u32) |
| **Strip** | `javascript:`, `data:`, `file:`, `vbscript:` (case-insensitive); `stitch:incomplete-link`; len > 8 KiB; any control char; empty http(s) rest; http(s) rest with space; all other schemes/bare hosts |

### `validate_image_ref`

| Verdict | Condition |
|---------|-----------|
| **AllowArtifactImg** | `lens-artifact://` prefix (case-insensitive), no `..` in URL |
| **RenderAsLink** | `http://`, `https://`, or `data:` prefix (placeholder text, never `img()`) |
| **Strip** | contains `..`; starts with `/`; any other scheme/path |

## Hostile-input matrix coverage

| Vector | Defense |
|--------|---------|
| `javascript:` | Strip (link paint + click) |
| `data:` | Strip (link); RenderAsLink (image, no fetch) |
| `vbscript:` | Strip |
| `file:` | Strip |
| `stitch:incomplete-link` | Strip |
| Control chars in URL | Strip (added beyond plan verbatim code) |
| `![](http…)` / `![](data:…)` | P5 placeholder, no `img()` |
| Path traversal image (`..`, `/`) | Strip |
| Embedded HTML `<a>`/`<img>` | P4 escaped source (pre-existing) + boundary backstop |
| File-path autolink `src/parser.rs:42` | `scan_prose_autolinks` + `NavigateToFile` verdict |
| Unicode scheme case (`JAVASCRIPT:`) | Strip via `to_ascii_lowercase()` |

## Concerns / gaps

1. **`mailto:` not allowed** — design brief mentions "http/https/mailto etc." but plan verbatim code only allows http(s) + file-path navigation. `mailto:user@example.com` → Strip. Task 4 / elicitation may need explicit mailto allow.
2. **File-path traversal not blocked at boundary** — `../secret.rs` passes `looks_like_file_path` → `NavigateToFile`; workspace doc handler must reject escapes (§8 deferral, emit-only seam).
3. **Symlink escape** — framework §2.5 mentions symlink guards for images; not implemented (artifact API absent).
4. **Unicode homoglyph schemes** — only ASCII lowercase normalization; e.g. lookalike Unicode in scheme prefix may bypass if it doesn't normalize to blocked ASCII (low risk for `open_url`, but not formally closed).
5. **P5/P6 paint integration** — boundary + static `img(` grep tested; no headless GPUI render asserting stripped link marks on a live `MarkdownView` (would need render-tree introspection or real-window probe).
6. **Autolink paint not wired** — scanner + emit exist; user-message renderer integration is Task 4 (`user_content.rs`).
7. **Real-window probe** — not run (sandbox); probe code untouched.
8. **Cross-family review (grok-4.5)** — not executed in this session; recommend before merge.

## Files touched

- Create: `crates/lens-ui/src/security.rs`, `focused/content_events.rs`, `focused/autolink.rs`, `tests/security_adversarial.rs`
- Modify: `lib.rs`, `focused/mod.rs`, `md/node.rs`, `md/inline.rs`

## Fix pass

**Commit:** `e9026db` — `fix(security): T3-2 review — fail-closed NavigateToFile + bound image-ref len + test-only event sink (I1/I2/I3)`

### I1 — fail-closed `parse_workspace_file_ref`
- Replaced permissive `looks_like_file_path` + `split_path_line` with `parse_workspace_file_ref`; only clean relative workspace paths (optional `:line`) become `NavigateToFile`.
- Hostile paths (`../.ssh/id_rsa`, `//evil.example/a`, `ftp://evil/x`, `/etc/passwd`, `custom:foo/bar`, `\\server\share\f.rs`) → **Strip**.
- Legit refs (`src/parser.rs:42`, `src/parser.rs`, `README.md`) → **NavigateToFile**.

### I2 — test-only event sink
- `content_events::SINK` push gated behind `#[cfg(test)]`; production `emit_navigate_to_file` is a no-op with `// TODO(T-4/nav)` comment.

### I3 — image-ref length cap
- `validate_image_ref` rejects `url.len() > MAX_URL_LEN` before any non-Strip verdict.

### Gate
- `cargo test -p lens-ui` → **253 passed** (230 unit + 23 integration, 2 real-window ignored); `security_adversarial` **15/15**; `cargo check` + `cargo clippy` (with/without `--features probe`) → **0 warnings**.
