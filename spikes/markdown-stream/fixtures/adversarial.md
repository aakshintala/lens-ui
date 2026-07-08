# Adversarial fixture — sanitization + safe-prefix worst case

## Dangerous links (must be neutralized to about:blank)

- A [click me](javascript:alert(1)) link with a `javascript:` scheme.
- A [data link](data:text/html,<script>alert(1)</script>) with a `data:` scheme.
- A safe [normal link](https://example.com/ok) that must survive untouched.
- A relative [local link](./notes.md) with no scheme — allowed.

## External image (must NOT inline — becomes inert text)

Here is an external tracker image: ![tracker](https://evil.test/pixel.png) that
must render as `[image: …]` text, never an inline image request.

## File-path autolink

A bare path like src/main.rs and crates/lens-client/src/lib.rs.

## Safe-prefix worst case — unterminated constructs at the very end

The following constructs are deliberately left OPEN at end-of-stream, so the
final delta lands mid-construct. Watch whether they flicker or render cleanly:

Some text then an open bold **that never closes and a half table row

| col a | col b
