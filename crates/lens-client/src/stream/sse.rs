//! Pure SSE wire-framing parser (`event: …\ndata: …\n\n`). No I/O — the reader
//! thread (stream::reader) feeds byte chunks in; this splits them into frames.
//! Live-tail, no-replay (transport spike §4): framing must wait for a full
//! `\n\n`-terminated frame, never match on a raw substring.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseFrame {
    pub event: String,
    pub data: String,
}

#[derive(Default)]
pub(crate) struct SseParser {
    buf: String,
}

impl SseParser {
    /// Feed a byte chunk; return any frames completed (`\n\n`-terminated) by it.
    /// A trailing partial frame stays buffered for the next chunk.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<SseFrame> {
        // Bytes are UTF-8 SSE text; lossy is safe — control framing is ASCII.
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        while let Some(idx) = self.buf.find("\n\n") {
            let block: String = self.buf.drain(..idx + 2).collect();
            if let Some(frame) = parse_block(block.trim_end_matches('\n')) {
                out.push(frame);
            }
        }
        out
    }

    /// Flush a trailing complete frame at EOF (server closed without a final `\n\n`).
    pub(crate) fn finish(&mut self) -> Vec<SseFrame> {
        let rest = std::mem::take(&mut self.buf);
        parse_block(rest.trim()).into_iter().collect()
    }
}

/// Parse one `event:`/`data:` block. Multiple `data:` lines join with `\n`
/// (SSE spec). Returns None for comment-only/empty blocks (e.g. `:` keepalives).
fn parse_block(block: &str) -> Option<SseFrame> {
    let mut event = String::new();
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(v) = line.strip_prefix("event:") {
            event = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push(v.strip_prefix(' ').unwrap_or(v));
        }
    }
    if event.is_empty() && data.is_empty() {
        return None;
    }
    Some(SseFrame {
        event,
        data: data.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_single_frame() {
        let mut p = SseParser::default();
        let frames = p.push(b"event: session.status\ndata: {\"status\":\"idle\"}\n\n");
        assert_eq!(
            frames,
            vec![SseFrame {
                event: "session.status".into(),
                data: "{\"status\":\"idle\"}".into()
            }]
        );
    }

    #[test]
    fn handles_a_frame_split_across_two_chunks() {
        let mut p = SseParser::default();
        assert!(
            p.push(b"event: response.completed\ndata: {\"a\":1}")
                .is_empty()
        );
        let frames = p.push(b"\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "response.completed");
        assert_eq!(frames[0].data, "{\"a\":1}");
    }

    #[test]
    fn parses_the_full_happy_path_fixture() {
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        // The captured happy-path turn has 25 frames (13 distinct event types).
        assert_eq!(frames.len(), 25);
        assert_eq!(frames[0].event, "session.heartbeat");
        assert!(frames.iter().any(|f| f.event == "response.completed"));
        // Every frame parsed a non-empty event name and JSON-object data.
        assert!(
            frames
                .iter()
                .all(|f| !f.event.is_empty() && f.data.starts_with('{'))
        );
    }
}
