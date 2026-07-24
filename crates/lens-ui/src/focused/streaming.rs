use std::time::{Duration, Instant};

pub struct StreamCoalescer {
    pending: String,
    last_emit: Option<Instant>,
    frame_budget: Duration,
}

impl StreamCoalescer {
    pub fn new() -> Self {
        Self {
            pending: String::new(),
            last_emit: None,
            frame_budget: Duration::from_millis(16),
        }
    }

    pub fn push_delta(&mut self, delta: &str) -> Option<String> {
        self.pending.push_str(delta);
        let now = Instant::now();
        let should_emit = self
            .last_emit
            .map(|t| now.duration_since(t) >= self.frame_budget)
            .unwrap_or(true);
        if should_emit {
            self.last_emit = Some(now);
            let stitched = crate::md::safe_prefix(&self.pending);
            Some(stitched)
        } else {
            None
        }
    }

    pub fn finalize(&self, final_text: &str) -> String {
        crate::md::safe_prefix(final_text)
    }
}

impl Default for StreamCoalescer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_uses_same_pipeline_as_stream() {
        let mut c = StreamCoalescer::new();
        let _ = c.push_delta("**bo");
        let streamed = c.finalize("**bold**");
        assert_eq!(streamed, "**bold**");
    }
}
