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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_cover_source_and_respect_char_boundaries() {
        let src = "héllo wörld"; // multibyte chars
        let d = deltas(src, 3);
        assert_eq!(d.concat(), src); // lossless
        assert!(d.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn accumulate_yields_growing_prefixes() {
        let d = vec!["ab".to_string(), "cd".to_string(), "ef".to_string()];
        let acc = accumulate(&d);
        assert_eq!(acc, vec!["ab", "abcd", "abcdef"]);
    }
}
