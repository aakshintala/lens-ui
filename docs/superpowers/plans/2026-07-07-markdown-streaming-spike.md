# Streaming-Markdown Stable-Identity Spike — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a throwaway gpui harness that answers the framework §4.1 go/no-go — does `gpui-component`'s markdown component stream with stable element identity (no remount), enforce safe-prefix, and admit the Lens link/image sanitization boundary without a fork.

**Architecture:** A disposable binary crate at `spikes/markdown-stream/`. Pure-logic modules (`replay`, `sanitize`, safe-prefix wrapper) are TDD'd with unit tests over stable in-crate interfaces. The gpui `render` + `probe` layer replays a captured/synthesized delta stream into one retained gpui-component markdown entity and is verified by running the harness and reading instrumentation (the spike's actual deliverable). External-crate API specifics (`gpui-component`, `mdstitch`) are discovered in a dedicated task, not fabricated.

**Tech Stack:** Rust (edition 2024), gpui + `gpui-component` 0.5.1, `mdstitch` 0.1, `pulldown-cmark` 0.13.4 (+ a to-cmark reserializer), `pulldown-cmark-to-cmark`.

## Global Constraints

- Edition `2024`, `rust-version = "1.91"` (workspace `Cargo.toml`).
- The crate is an automatic workspace member via `members = ["spikes/*"]`.
- **The crate MUST NOT opt into `lints.workspace`** — spikes are deliberately outside the production lint wall (root `Cargo.toml` comment). No `[lints] workspace = true`.
- Throwaway: no production quality bar, no clippy-clean gate, discarded after the findings doc.
- Do **not** touch `crates/` or reconcile with the §3 gpui `0.2.2` pin — use whatever gpui version `gpui-component` 0.5.1 pins.
- Sanitization policy (spec §5 / transcript §6.1): allowed link schemes `http`, `https`, `mailto`, `file`; neutralize `javascript:`, `data:`, unknown. External-URL images are **not** inlined; artifact-scheme images may inline.

---

### Task 1: Crate skeleton + dependency spike + static markdown render

Stands up the crate, resolves the `gpui-component` / gpui version tangle, and gets a window rendering a **static** markdown string. Its real product is a short note recording the actual gpui-component markdown API (constructor + update call + how it takes a string), which Task 5 consumes.

**Files:**
- Create: `spikes/markdown-stream/Cargo.toml`
- Create: `spikes/markdown-stream/src/main.rs`
- Create: `spikes/markdown-stream/NOTES.md` (running API-discovery + findings scratch)

**Interfaces:**
- Consumes: nothing (first task).
- Produces: a runnable `cargo run -p markdown-stream` window; `NOTES.md` recording the gpui-component markdown component's real API — the exact type name, how a view constructs it, and the call that sets/updates its source text. Task 5 relies on these names.

- [ ] **Step 1: Create the crate manifest**

`spikes/markdown-stream/Cargo.toml`:

```toml
[package]
name = "markdown-stream"
version = "0.0.0"
edition = "2024"
publish = false

# NOTE: deliberately NO `[lints] workspace = true` — spikes live outside the
# production lint wall (see root Cargo.toml).

[dependencies]
gpui-component = "0.5.1"
mdstitch = "0.1"
pulldown-cmark = "0.13"
pulldown-cmark-to-cmark = "*"    # resolve exact version at build (Step 3)
# gpui itself: use the version gpui-component re-exports / pins. If gpui-component
# does NOT re-export gpui, add a matching `gpui = "<same rev>"` here after Step 3.
```

- [ ] **Step 2: Minimal window scaffold**

`spikes/markdown-stream/src/main.rs` — a gpui app that opens a window rendering a static markdown string via gpui-component. Because the exact gpui-component API is unknown until built, write the smallest thing the crate docs/examples show and iterate:

```rust
// Minimal goal: open a gpui window, render this string as formatted markdown
// using gpui-component's markdown component.
const SAMPLE: &str = "# Hello\n\nSome **bold** and a list:\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n";

fn main() {
    // Fill in from gpui-component's own examples (see NOTES.md Step 4):
    //   - Application/App bootstrap
    //   - open_window
    //   - a root view whose render() returns the gpui-component Markdown element
    //     built from SAMPLE.
    unimplemented!("wire per gpui-component example, then record API in NOTES.md")
}
```

- [ ] **Step 3: Resolve deps and discover the API**

Run: `cargo build -p markdown-stream`
Expected: resolves `gpui-component` 0.5.1 and pulls its gpui version. If gpui types aren't in scope, inspect `cargo tree -p markdown-stream | rg gpui` to learn the gpui version, add it to `Cargo.toml` matching that exact version, rebuild.
Then read the gpui-component markdown example/source (`cargo doc -p gpui-component --open`, or the crate's `examples/`) to learn the real markdown component API.

- [ ] **Step 4: Record the API in NOTES.md**

Write down, verbatim from the built crate: the markdown component type name, how a view builds it from a `&str`/`String`, and the method (if any) that mutates its source in place vs. rebuilding. Task 5 depends on this.

- [ ] **Step 5: Make the static render actually run**

Replace the `unimplemented!()` with the real bootstrap. Run: `cargo run -p markdown-stream`
Expected: a window shows SAMPLE with a heading, bold, a bullet list, and a highlighted code block. Eyeball it.

- [ ] **Step 6: Commit**

```bash
git add spikes/markdown-stream/
git commit -m "spike(markdown): crate skeleton + static gpui-component markdown render"
```

---

### Task 2: `replay` — delta chunker + frame accumulator (pure, TDD)

Turns a source string (a design-doc fixture or extracted `.sse` deltas) into a sequence of deltas and the running accumulation that the renderer sees each frame.

**Files:**
- Create: `spikes/markdown-stream/src/replay.rs`
- Modify: `spikes/markdown-stream/src/main.rs` (add `mod replay;`)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `pub fn deltas(src: &str, chunk_chars: usize) -> Vec<String>` — splits `src` into successive chunks of ~`chunk_chars` characters (respecting `char` boundaries, never splitting a UTF-8 codepoint).
  - `pub fn accumulate(deltas: &[String]) -> Vec<String>` — running prefixes: element `i` is the concatenation of `deltas[0..=i]`. Task 5 feeds each accumulation to the renderer per frame.

- [ ] **Step 1: Write the failing tests**

`spikes/markdown-stream/src/replay.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_cover_source_and_respect_char_boundaries() {
        let src = "héllo wörld";              // multibyte chars
        let d = deltas(src, 3);
        assert_eq!(d.concat(), src);           // lossless
        assert!(d.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn accumulate_yields_growing_prefixes() {
        let d = vec!["ab".to_string(), "cd".to_string(), "ef".to_string()];
        let acc = accumulate(&d);
        assert_eq!(acc, vec!["ab", "abcd", "abcdef"]);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p markdown-stream replay`
Expected: FAIL — `deltas` / `accumulate` not found.

- [ ] **Step 3: Implement**

```rust
/// Split `src` into successive chunks of about `chunk_chars` characters,
/// never splitting a UTF-8 codepoint.
pub fn deltas(src: &str, chunk_chars: usize) -> Vec<String> {
    let chunk_chars = chunk_chars.max(1);
    let chars: Vec<char> = src.chars().collect();
    chars
        .chunks(chunk_chars)
        .map(|c| c.iter().collect())
        .collect()
}

/// Running prefixes: element i = concat of deltas[0..=i].
pub fn accumulate(deltas: &[String]) -> Vec<String> {
    let mut acc = String::new();
    let mut out = Vec::with_capacity(deltas.len());
    for d in deltas {
        acc.push_str(d);
        out.push(acc.clone());
    }
    out
}
```

Add `mod replay;` to `main.rs`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p markdown-stream replay`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add spikes/markdown-stream/src/replay.rs spikes/markdown-stream/src/main.rs
git commit -m "spike(markdown): replay delta chunker + frame accumulator"
```

---

### Task 3: `sanitize` — link/image boundary (pure, TDD)

Enforces the Lens sanitization policy as a pre-parse transform: parse markdown, neutralize disallowed link schemes and external-image inlines, reserialize to markdown.

**Files:**
- Create: `spikes/markdown-stream/src/sanitize.rs`
- Modify: `spikes/markdown-stream/src/main.rs` (add `mod sanitize;`)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `pub fn sanitize(md: &str) -> String` — returns markdown with disallowed link URLs replaced by the sentinel `about:blank` and external-URL images converted to inert text `[image: <url>]`. Allowed-scheme links and artifact-scheme images pass through unchanged. Task 5 applies it before `mdstitch`.

- [ ] **Step 1: Write the failing tests**

`spikes/markdown-stream/src/sanitize.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutralizes_javascript_link_scheme() {
        let out = sanitize("[click](javascript:alert(1))");
        assert!(!out.contains("javascript:"));
        assert!(out.contains("about:blank"));
    }

    #[test]
    fn keeps_https_link() {
        let out = sanitize("[docs](https://example.com/x)");
        assert!(out.contains("https://example.com/x"));
    }

    #[test]
    fn external_image_not_inlined() {
        let out = sanitize("![alt](https://evil.test/tracker.png)");
        assert!(!out.contains("!["));                       // no image syntax
        assert!(out.contains("[image: https://evil.test/tracker.png]"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p markdown-stream sanitize`
Expected: FAIL — `sanitize` not found.

- [ ] **Step 3: Implement**

Parse with `pulldown-cmark`, rewrite link/image events, reserialize with `pulldown-cmark-to-cmark`. The allowlist and sentinels come from Global Constraints.

```rust
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};

const ALLOWED_SCHEMES: [&str; 4] = ["http", "https", "mailto", "file"];

fn scheme_allowed(url: &str) -> bool {
    match url.split_once(':') {
        Some((scheme, _)) => ALLOWED_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()),
        None => true, // relative / fragment links have no scheme — allow
    }
}

/// Apply the Lens link/image policy, returning sanitized markdown.
pub fn sanitize(md: &str) -> String {
    let parser = Parser::new_ext(md, Options::all());
    let mut events: Vec<Event> = Vec::new();
    // When we drop an image, we also drop its alt-text events until the matching end.
    let mut dropping_image_depth: usize = 0;

    for ev in parser {
        if dropping_image_depth > 0 {
            match ev {
                Event::Start(Tag::Image { .. }) => dropping_image_depth += 1,
                Event::End(TagEnd::Image) => dropping_image_depth -= 1,
                _ => {} // swallow alt-text
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                let url = if scheme_allowed(&dest_url) {
                    dest_url
                } else {
                    CowStr::Borrowed("about:blank")
                };
                events.push(Event::Start(Tag::Link { link_type, dest_url: url, title, id }));
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                // External image: do not inline — emit inert text instead.
                events.push(Event::Text(CowStr::from(format!("[image: {dest_url}]"))));
                dropping_image_depth = 1;
            }
            other => events.push(other),
        }
    }

    let mut out = String::new();
    pulldown_cmark_to_cmark::cmark(events.into_iter(), &mut out)
        .expect("reserialize sanitized markdown");
    out
}
```

Add `mod sanitize;` to `main.rs`. If `pulldown-cmark-to-cmark`'s `cmark` signature differs in the resolved version, adjust the call (record the version in `NOTES.md`); the event-rewrite logic is the point.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p markdown-stream sanitize`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add spikes/markdown-stream/src/sanitize.rs spikes/markdown-stream/src/main.rs
git commit -m "spike(markdown): link/image sanitization boundary (pre-parse transform)"
```

---

### Task 4: `safe_prefix` wrapper over `mdstitch` (TDD)

A stable in-crate interface for the safe-prefix step so the renderer doesn't couple to mdstitch's exact API and so behavior is unit-tested.

**Files:**
- Create: `spikes/markdown-stream/src/safe_prefix.rs`
- Modify: `spikes/markdown-stream/src/main.rs` (add `mod safe_prefix;`)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `pub fn safe_prefix(accumulated: &str) -> String` — returns markdown whose trailing unterminated construct is closed (delegates to `mdstitch`). Task 5 calls it each frame after `sanitize`.

- [ ] **Step 1: Write the failing tests**

`spikes/markdown-stream/src/safe_prefix.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::{Options, Parser, Event, Tag, TagEnd};

    fn has_balanced_strong(md: &str) -> bool {
        let mut open = 0i32;
        for ev in Parser::new_ext(md, Options::all()) {
            match ev {
                Event::Start(Tag::Strong) => open += 1,
                Event::End(TagEnd::Strong) => open -= 1,
                _ => {}
            }
        }
        open == 0
    }

    #[test]
    fn closes_unterminated_bold() {
        // Raw "**bold" parses with an unbalanced Strong; safe_prefix must fix it.
        let fixed = safe_prefix("**bold");
        assert!(has_balanced_strong(&fixed));
    }

    #[test]
    fn passes_through_wellformed() {
        let fixed = safe_prefix("plain text");
        assert!(fixed.contains("plain text"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p markdown-stream safe_prefix`
Expected: FAIL — `safe_prefix` not found.

- [ ] **Step 3: Implement**

Delegate to mdstitch. The exact entry point (`mdstitch::stitch`, a `Stitcher`, etc.) is resolved from `cargo doc -p mdstitch`; record it in `NOTES.md`. Shape:

```rust
/// Close the trailing unterminated markdown construct in `accumulated`
/// (safe-prefix streaming, transcript §5). Delegates to mdstitch.
pub fn safe_prefix(accumulated: &str) -> String {
    // Replace with mdstitch's real entry point once confirmed from cargo doc.
    mdstitch::stitch(accumulated)
}
```

Add `mod safe_prefix;` to `main.rs`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p markdown-stream safe_prefix`
Expected: PASS (2 tests). If mdstitch's API differs, fix the one call; the tests assert behavior, not the API.

- [ ] **Step 5: Commit**

```bash
git add spikes/markdown-stream/src/safe_prefix.rs spikes/markdown-stream/src/main.rs
git commit -m "spike(markdown): safe_prefix wrapper over mdstitch"
```

---

### Task 5: `render` + `probe` — streaming pipeline into one retained markdown entity

Wires `replay → sanitize → safe_prefix → gpui-component markdown` on a frame timer, feeding **one retained** markdown entity keyed by a fixed id, and instruments element-build / parse counts + frame time. This is the core of the spike; verified by running it and reading the probe, not by unit assertions.

**Files:**
- Create: `spikes/markdown-stream/src/render.rs`
- Create: `spikes/markdown-stream/src/probe.rs`
- Create: `spikes/markdown-stream/fixtures/gfm-stress.md` (copied/curated from `docs/design/framework.md`)
- Modify: `spikes/markdown-stream/src/main.rs` (mount the streaming view; add `mod render; mod probe;`)

**Interfaces:**
- Consumes: `replay::deltas`, `replay::accumulate`, `sanitize::sanitize`, `safe_prefix::safe_prefix`; the gpui-component markdown API recorded in Task 1 `NOTES.md`.
- Produces: a runnable streaming harness; `probe::Probe` with `note_frame(build_count: usize, frame_us: u128)` and a `summary()` printed on completion.

- [ ] **Step 1: Curate the stress fixture**

Copy a GFM-heavy slice of `docs/design/framework.md` (tables, fenced code, nested lists, links) into `spikes/markdown-stream/fixtures/gfm-stress.md`. Ensure it contains at least one table, two fenced code blocks of different languages, a nested list, and inline links.

- [ ] **Step 2: Write the probe**

`spikes/markdown-stream/src/probe.rs`:

```rust
#[derive(Default)]
pub struct Probe {
    frames: usize,
    total_builds: usize,
    max_frame_us: u128,
}

impl Probe {
    pub fn note_frame(&mut self, build_count: usize, frame_us: u128) {
        self.frames += 1;
        self.total_builds += build_count;
        self.max_frame_us = self.max_frame_us.max(frame_us);
    }

    pub fn summary(&self) -> String {
        let avg = if self.frames == 0 { 0 } else { self.total_builds / self.frames };
        format!(
            "frames={} avg_builds/frame={} max_frame_us={}",
            self.frames, avg, self.max_frame_us
        )
    }
}
```

The "builds" signal: instrument the render path so each construction of a markdown block element increments a counter for the frame (e.g. a thread-local or an `Rc<Cell<usize>>` bumped where the element tree is built). Record in `NOTES.md` exactly where the counter is incremented — that provenance is what makes the verdict trustworthy.

- [ ] **Step 3: Build the streaming view**

`spikes/markdown-stream/src/render.rs`: a gpui view that

1. loads the fixture, computes `accumulate(deltas(fixture, 8))`,
2. on a ~16ms timer advances one accumulation step,
3. sets the markdown entity's source to `safe_prefix(sanitize(&accum))` **without reconstructing the entity** (mutate in place, per the Task 1 API),
4. records `probe.note_frame(...)` each tick,
5. prints `probe.summary()` when the stream ends.

Use the gpui-component markdown API from `NOTES.md`. Keep the markdown entity in the view's state; never recreate it on tick.

- [ ] **Step 4: Run and observe**

Run: `cargo run -p markdown-stream`
Expected: the fixture streams in with markdown formatting appearing progressively; no per-token flicker of closed constructs; on completion the terminal prints a `probe` summary. Record the summary in `NOTES.md`.
**Key reading:** `avg_builds/frame` should be small and roughly constant (O(changed blocks)), not growing with document length (which would indicate a full rebuild every frame).

- [ ] **Step 5: Commit**

```bash
git add spikes/markdown-stream/src/render.rs spikes/markdown-stream/src/probe.rs \
        spikes/markdown-stream/fixtures/ spikes/markdown-stream/src/main.rs \
        spikes/markdown-stream/NOTES.md
git commit -m "spike(markdown): streaming pipeline + build-count/frame-time probe"
```

---

### Task 6: Adversarial scenario — held scroll + held selection + finalize swap

Stages the remount failure modes so "no remount" is demonstrated, not assumed. Verified by running + reading the probe and eyeballing (optionally a GIF).

**Files:**
- Modify: `spikes/markdown-stream/src/render.rs` (add scroll-hold, selection-hold, finalize swap)
- Create: `spikes/markdown-stream/fixtures/adversarial.md` (sanitization + worst-case safe-prefix)
- Modify: `spikes/markdown-stream/NOTES.md` (record results)

**Interfaces:**
- Consumes: everything from Task 5.
- Produces: the final observed verdict inputs (scroll/selection stability across the swap; safe-prefix + sanitization behavior on the adversarial fixture).

- [ ] **Step 1: Author the adversarial fixture**

`spikes/markdown-stream/fixtures/adversarial.md` — include: a `[x](javascript:alert(1))` link, a `[y](data:text/html,...)` link, an `![img](https://evil.test/a.png)` external image, an autolinkable `src/main.rs` path, and (at the very end, unterminated) an open `**`, an open code fence ` ``` `, and a half-written table row `| a | b`.

- [ ] **Step 2: Wrap the surface in a scrollable container and hold position**

In `render.rs`, put the markdown entity inside a scroll container. When streaming passes ~50% of frames, programmatically set the scroll offset to the top and keep it pinned while deltas continue appending at the bottom. Record whether the offset stays put across subsequent ticks.

- [ ] **Step 3: Hold a text selection across appends**

If gpui-component's markdown exposes selection, select a range in already-rendered text before the stream finishes; if it does not expose selection, record that as a finding in `NOTES.md` and fall back to scroll-hold + visual flicker as the identity evidence.

- [ ] **Step 4: Trigger the finalize swap**

At stream end, re-render the same item as a "finalized" `Message` (feed the raw full text without `safe_prefix`, same entity id). Record: does the swap visibly change anything (flicker / scroll jump / selection loss)? Per transcript §5 it should be a **visual no-op**.

- [ ] **Step 5: Run and record**

Run: `cargo run -p markdown-stream -- --adversarial`  (add a simple arg toggle to load `adversarial.md`)
Expected + record in `NOTES.md`: (a) scroll offset unchanged across finalize; (b) selection survived (or "selection API absent"); (c) no closed-construct flicker; (d) `javascript:`/`data:` links inert, external image not inlined, file path autolinked; (e) probe `avg_builds/frame` stayed flat. Optionally record a GIF via the browser/gif tooling.

- [ ] **Step 6: Commit**

```bash
git add spikes/markdown-stream/src/render.rs spikes/markdown-stream/fixtures/adversarial.md \
        spikes/markdown-stream/NOTES.md
git commit -m "spike(markdown): adversarial scroll/selection/finalize + sanitization scenario"
```

---

### Task 7: Findings doc + STATUS/framework update

Turn `NOTES.md` observations into the durable verdict; the harness itself is discarded conceptually (left in `spikes/` as a record, not maintained).

**Files:**
- Create: `docs/spikes/2026-07-07-markdown-streaming.md`
- Modify: `docs/design/framework.md` (§4.1 verdict line)
- Modify: `docs/STATUS.md` (Open threads / Recent)

**Interfaces:**
- Consumes: `NOTES.md` from Tasks 1–6.
- Produces: the PASS/PARTIAL/FAIL verdict + evidence.

- [ ] **Step 1: Write the findings doc**

`docs/spikes/2026-07-07-markdown-streaming.md`, mirroring `docs/spikes/2026-06-25-transport-stability.md`'s style: verdict (PASS / PARTIAL / FAIL), evidence per §4.1 residual (a stable identity — scroll/selection across swap + `avg_builds/frame`; d safe-prefix behavior; b sanitization sufficiency), the gpui-component/gpui versions used, and — if not PASS — which §4.1 fallback-ladder rung applies (and whether vendoring just the markdown module suffices).

- [ ] **Step 2: Update framework §4.1**

Add a one-line dated verdict to `docs/design/framework.md` §4.1 (the "UNVERIFIED, the key spike question" line becomes the verdict).

- [ ] **Step 3: Update STATUS**

Move the "Markdown renderer" bullet in `docs/STATUS.md` from deferred to a Recent entry with the verdict + link to the findings doc.

- [ ] **Step 4: Commit**

```bash
git add docs/spikes/2026-07-07-markdown-streaming.md docs/design/framework.md docs/STATUS.md
git commit -m "docs(spike): record markdown-streaming stable-identity verdict"
```

---

## Self-Review

**Spec coverage:**
- §1 pass/fail (PASS/PARTIAL/FAIL + fallback rung) → Task 7.
- §1(a) stable identity → Tasks 5 (build-count probe) + 6 (scroll/selection/finalize).
- §1(d) safe-prefix → Task 4 (unit) + Tasks 5–6 (observed, incl. unterminated EOF constructs).
- §1(b) sanitization → Task 3 (unit) + Task 6 (observed on adversarial fixture).
- §2 architecture / module split → Tasks 1–5 (one module each).
- §2 no lint-wall opt-in → Global Constraints + Task 1 manifest.
- §2 gpui-pin non-reconciliation → Global Constraints + Task 1.
- §3 corpus (synthesized primary, adversarial, real-capture smoke) → Task 5 fixture + Task 6 adversarial; real-capture `.sse` replay is available via `replay::deltas` over extracted deltas (optional secondary run, noted in Task 5 Step 4).
- §4 evidence (instrumentation + adversarial) → Tasks 5–6.
- §5 sanitization policy (allowlist + sentinels) → Global Constraints + Task 3.
- §7 non-goals (virtualization, vendoring, pin reconciliation) → excluded; none tasked.
- §8 deliverable → Task 7.

**Placeholder scan:** External-crate API specifics (`gpui-component` markdown type, `mdstitch` entry point, `pulldown-cmark-to-cmark` `cmark` signature) are intentionally resolved at build via `NOTES.md` discovery rather than fabricated — flagged explicitly in Tasks 1/3/4, not silent TODOs. Our own module interfaces are concrete and testable. The one `unimplemented!()` (Task 1 Step 2) is a scaffold explicitly replaced in Step 5.

**Type consistency:** `deltas`/`accumulate` (Task 2) consumed by Task 5; `sanitize` (Task 3) and `safe_prefix` (Task 4) both `&str -> String`, composed as `safe_prefix(sanitize(&accum))` in Task 5; `Probe::note_frame`/`summary` (Task 5) reused in Task 6. Names consistent across tasks.
