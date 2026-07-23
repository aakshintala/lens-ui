#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutolinkHit {
    pub range: std::ops::Range<usize>,
    pub target: AutolinkTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutolinkTarget {
    Url(String),
    FilePath { path: String, line: Option<u32> },
}

pub fn scan_prose_autolinks(prose: &str) -> Vec<AutolinkHit> {
    let mut hits = Vec::new();
    for (idx, token) in prose.split_whitespace().enumerate() {
        let _ = idx;
        if token.starts_with("http://") || token.starts_with("https://") {
            hits.push(AutolinkHit {
                range: find_token_range(prose, token),
                target: AutolinkTarget::Url(
                    token
                        .trim_end_matches(&['.', ',', ';'][..])
                        .to_string(),
                ),
            });
        } else if looks_like_path_token(token) {
            let clean = token.trim_end_matches(&['.', ',', ';'][..]);
            let (path, line) = split_path_line(clean);
            hits.push(AutolinkHit {
                range: find_token_range(prose, token),
                target: AutolinkTarget::FilePath { path, line },
            });
        }
    }
    hits
}

fn looks_like_path_token(token: &str) -> bool {
    // path-shaped: has a separator or a known code extension, not a bare word.
    (token.contains('/') || token.ends_with(".rs") || token.ends_with(".md"))
        && !token.contains("://")
}

fn split_path_line(token: &str) -> (String, Option<u32>) {
    if let Some((path, line)) = token.rsplit_once(':')
        && let Ok(n) = line.parse::<u32>()
    {
        return (path.to_string(), Some(n));
    }
    (token.to_string(), None)
}

// Byte range of `token` within `prose`. `split_whitespace` yields subslices of
// `prose`, so recover the offset by pointer arithmetic (no re-search needed).
fn find_token_range(prose: &str, token: &str) -> std::ops::Range<usize> {
    let start = token.as_ptr() as usize - prose.as_ptr() as usize;
    start..start + token.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_path_autolink_in_prose() {
        let prose = "see src/parser.rs:10 for details";
        let hits = scan_prose_autolinks(prose);
        assert_eq!(hits.len(), 1);
        match &hits[0].target {
            AutolinkTarget::FilePath { path, line } => {
                assert_eq!(path, "src/parser.rs");
                assert_eq!(*line, Some(10));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn url_autolink_in_prose() {
        let prose = "visit https://example.com/docs.";
        let hits = scan_prose_autolinks(prose);
        assert_eq!(hits.len(), 1);
        match &hits[0].target {
            AutolinkTarget::Url(url) => assert_eq!(url, "https://example.com/docs"),
            other => panic!("{other:?}"),
        }
    }
}
