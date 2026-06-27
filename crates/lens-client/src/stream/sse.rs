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
    buf: Vec<u8>,
}

impl SseParser {
    /// Feed a byte chunk; return any frames completed (`\n\n`-terminated) by it.
    /// A trailing partial frame stays buffered for the next chunk.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<SseFrame> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(idx) = find_double_newline(&self.buf) {
            let block_bytes: Vec<u8> = self.buf.drain(..idx + 2).collect();
            let block = String::from_utf8_lossy(&block_bytes)
                .trim_end_matches('\n')
                .to_string();
            if let Some(frame) = parse_block(&block) {
                out.push(frame);
            }
        }
        out
    }

    /// Flush a trailing complete frame at EOF (server closed without a final `\n\n`).
    pub(crate) fn finish(&mut self) -> Vec<SseFrame> {
        let rest = std::mem::take(&mut self.buf);
        if rest.is_empty() {
            return Vec::new();
        }
        let block = String::from_utf8_lossy(&rest).trim().to_string();
        parse_block(&block).into_iter().collect()
    }
}

fn find_double_newline(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
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

impl SseFrame {
    /// Peek `sequence_number` off the frame's data JSON without full typing.
    /// `None` when absent, null, or unparseable — only seq-bearing live frames
    /// (heartbeats, response deltas) carry it (typed-client §7 / plan decision 3).
    #[allow(dead_code)] // reconnect reader (Plan 3b-2b Task 5)
    pub(crate) fn sequence_number(&self) -> Option<u64> {
        #[derive(serde::Deserialize)]
        struct SeqPeek {
            sequence_number: Option<u64>,
        }
        serde_json::from_str::<SeqPeek>(&self.data)
            .ok()?
            .sequence_number
    }
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
    fn multibyte_utf8_split_across_chunks_is_not_corrupted() {
        // "café" UTF-8: 63 61 66 c3 a9 — split inside é: chunk1 ends with 0xC3, chunk2 starts with 0xA9.
        let payload = r#"{"delta":"café"}"#;
        let full = format!("event: response.output_text.delta\ndata: {payload}\n\n");
        let bytes = full.as_bytes();
        let split_at = bytes
            .windows(2)
            .position(|w| w == [0xc3, 0xa9])
            .expect("é in payload")
            + 1;
        assert_eq!(bytes[split_at - 1], 0xc3);
        assert_eq!(bytes[split_at], 0xa9);
        let mut p = SseParser::default();
        assert!(p.push(&bytes[..split_at]).is_empty());
        let frames = p.push(&bytes[split_at..]);
        assert_eq!(frames.len(), 1);
        assert!(
            !frames[0].data.contains('\u{FFFD}'),
            "split UTF-8 must not produce replacement chars: {:?}",
            frames[0].data
        );
        assert_eq!(frames[0].data, payload);
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

    #[test]
    fn frame_sequence_number_peeks_data_json() {
        let f = SseFrame {
            event: "response.output_text.delta".into(),
            data: r#"{"sequence_number":7,"delta":"hi"}"#.into(),
        };
        assert_eq!(f.sequence_number(), Some(7));

        let no_seq = SseFrame {
            event: "x".into(),
            data: r#"{"id":"item_1"}"#.into(),
        };
        assert_eq!(no_seq.sequence_number(), None);

        let null_seq = SseFrame {
            event: "x".into(),
            data: r#"{"sequence_number":null}"#.into(),
        };
        assert_eq!(null_seq.sequence_number(), None);

        let junk = SseFrame {
            event: "x".into(),
            data: "not json".into(),
        };
        assert_eq!(junk.sequence_number(), None);
    }
}
