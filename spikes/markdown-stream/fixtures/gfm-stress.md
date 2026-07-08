# Streaming Markdown Stress Fixture

This fixture exercises the **full GFM surface** the transcript spec (§6.1) calls
_IN_: headings, **bold**, _italic_, `inline code`, [links](https://example.com),
ordered/unordered/nested lists, task lists, tables, blockquotes, and fenced code
with multiple languages. It is chunked into deltas and streamed at a frame tick.

## 1. Prose with inline constructs

The quick **brown fox** jumps over the _lazy dog_ while reading `Cargo.toml` and
visiting [the docs](https://example.com/docs) and emailing <mailto:a@b.com>. A
path like `src/main.rs` should render verbatim inside code. Mixing **bold _and
italic_** and `code with **stars**` stresses the safe-prefix boundary when a
delta lands mid-construct.

## 2. Lists

- top level one
- top level two
  - nested a
  - nested b
    - deep i
    - deep ii
- top level three

1. first
2. second
   1. sub-first
   2. sub-second
3. third

Task list:

- [x] discover the gpui-component markdown API
- [x] confirm stable-identity mechanism
- [ ] verify streaming at runtime
- [ ] verify sanitization boundary

## 3. A table

| harness   | reasoning folded? | streams markdown? | notes                    |
|-----------|-------------------|-------------------|--------------------------|
| claude    | yes               | yes               | folds into output_text   |
| cursor    | no (SDK key)      | yes               | real reasoning deltas    |
| codex     | n/a               | n/a               | quarantined on this box  |

## 4. Blockquote

> The board is the one real spike — ordinal slots + adaptive packing.
> The differentiator is concurrent *warm state*, not concurrent *display*.

## 5. Fenced code, multiple languages

```rust
fn main() {
    let msg = "stable identity keyed by ElementId";
    println!("{msg}");
}
```

```python
def safe_prefix(accumulated: str) -> str:
    # close the trailing unterminated construct
    return accumulated
```

```bash
cargo run -p markdown-stream -- --stream
```

## 6. More prose to add streaming volume

Streaming already renders markdown, so the finalize swap (StreamingMessage →
canonical Message) is a near visual no-op. Format every **closed** markdown
construct immediately as text streams; hold the **open** trailing construct as
plain until it closes, then promote. Reflow is bounded to the trailing line; no
incomplete-syntax flicker. Coalesce deltas to a frame tick (~60 fps); never
re-render per token. Stable widget identity across streaming→finalized: key by
response/item id; diff in place, never unmount/remount.

### 6.1 Repeated section for length

The wedge survives, precisely located: it is single-server, single-warm-stream,
chat-shaped. Lens is multi-server, N-warm-streams (every session live off-thread
→ zero switch latency, cards always live), board-shaped. A fork buys a mature
widget toolkit but forces a rewrite of the connection model, live-state fan-out,
and navigation, and re-crosses the type boundary.

### 6.2 And once more

`gpui-component` 0.5.1 ships a native Markdown component with tree-sitter syntax
highlighting, plus virtualized `List`/`Table` and form inputs. The widget-risk
axis where the React alternative was ahead is now largely matched on the gpui
side, permissively licensed, no IPC cost.
