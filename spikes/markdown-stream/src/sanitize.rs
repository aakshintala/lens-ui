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
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let url = if scheme_allowed(&dest_url) {
                    dest_url
                } else {
                    CowStr::Borrowed("about:blank")
                };
                events.push(Event::Start(Tag::Link {
                    link_type,
                    dest_url: url,
                    title,
                    id,
                }));
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
        assert!(!out.contains("![")); // no image syntax
        assert!(out.contains("[image: https://evil.test/tracker.png]"));
    }
}
