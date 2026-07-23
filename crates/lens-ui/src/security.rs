const MAX_URL_LEN: usize = 8 * 1024;

#[derive(Debug)]
pub enum LinkVerdict {
    AllowOpenUrl,
    NavigateToFile { path: String, line: Option<u32> },
    Strip,
}

pub enum ImageVerdict {
    AllowArtifactImg { url: String },
    RenderAsLink { url: String },
    Strip,
}

pub fn validate_link_url(url: &str) -> LinkVerdict {
    if url.starts_with("stitch:incomplete-link") {
        return LinkVerdict::Strip;
    }
    if url.len() > MAX_URL_LEN {
        return LinkVerdict::Strip;
    }
    if url.chars().any(|c| c.is_control()) {
        return LinkVerdict::Strip;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("file:")
        || lower.starts_with("vbscript:")
    {
        return LinkVerdict::Strip;
    }
    if let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        if rest.is_empty() || rest.contains(' ') {
            return LinkVerdict::Strip;
        }
        return LinkVerdict::AllowOpenUrl;
    }
    if looks_like_file_path(url) {
        let (path, line) = split_path_line(url);
        return LinkVerdict::NavigateToFile { path, line };
    }
    LinkVerdict::Strip
}

pub fn validate_image_ref(url: &str) -> ImageVerdict {
    if url.contains("..") || url.starts_with('/') {
        return ImageVerdict::Strip;
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("data:")
        || lower.starts_with("http://")
        || lower.starts_with("https://")
    {
        return ImageVerdict::RenderAsLink {
            url: url.to_string(),
        };
    }
    if lower.starts_with("lens-artifact://") && !lower.contains("..") {
        return ImageVerdict::AllowArtifactImg {
            url: url.to_string(),
        };
    }
    ImageVerdict::Strip
}

fn looks_like_file_path(url: &str) -> bool {
    url.starts_with("./")
        || url.starts_with("../")
        || url.contains('/')
        || url.ends_with(".rs")
        || url.ends_with(".md")
}

fn split_path_line(url: &str) -> (String, Option<u32>) {
    if let Some((path, line)) = url.rsplit_once(':')
        && let Ok(n) = line.parse::<u32>()
    {
        return (path.to_string(), Some(n));
    }
    (url.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_javascript() {
        assert!(matches!(
            validate_link_url("javascript:alert(1)"),
            LinkVerdict::Strip
        ));
    }

    #[test]
    fn strips_stitch_incomplete() {
        assert!(matches!(
            validate_link_url("stitch:incomplete-link"),
            LinkVerdict::Strip
        ));
    }

    #[test]
    fn file_path_navigates() {
        match validate_link_url("src/parser.rs:42") {
            LinkVerdict::NavigateToFile { path, line } => {
                assert_eq!(path, "src/parser.rs");
                assert_eq!(line, Some(42));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn remote_image_renders_as_link() {
        assert!(matches!(
            validate_image_ref("https://tracker.example/x.png"),
            ImageVerdict::RenderAsLink { .. }
        ));
    }

    #[test]
    fn strips_data_link() {
        assert!(matches!(
            validate_link_url("data:text/html,<script>alert(1)</script>"),
            LinkVerdict::Strip
        ));
    }

    #[test]
    fn strips_file_link() {
        assert!(matches!(
            validate_link_url("file:///etc/passwd"),
            LinkVerdict::Strip
        ));
    }

    #[test]
    fn strips_vbscript() {
        assert!(matches!(
            validate_link_url("vbscript:msgbox(1)"),
            LinkVerdict::Strip
        ));
    }

    #[test]
    fn allows_https() {
        assert!(matches!(
            validate_link_url("https://example.com/path"),
            LinkVerdict::AllowOpenUrl
        ));
    }

    #[test]
    fn strips_oversized_url() {
        let url = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(matches!(validate_link_url(&url), LinkVerdict::Strip));
    }

    #[test]
    fn strips_path_traversal_image() {
        assert!(matches!(
            validate_image_ref("../secret.png"),
            ImageVerdict::Strip
        ));
    }

    #[test]
    fn strips_absolute_path_image() {
        assert!(matches!(
            validate_image_ref("/etc/passwd"),
            ImageVerdict::Strip
        ));
    }

    #[test]
    fn data_image_renders_as_link_not_fetch() {
        assert!(matches!(
            validate_image_ref("data:image/png;base64,abc"),
            ImageVerdict::RenderAsLink { .. }
        ));
    }

    #[test]
    fn artifact_image_allowed() {
        assert!(matches!(
            validate_image_ref("lens-artifact://session/abc/img.png"),
            ImageVerdict::AllowArtifactImg { .. }
        ));
    }
}
