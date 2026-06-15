use dioxus::prelude::*;

/// A full-width error banner that renders a (possibly long) message on its own
/// lines and turns any `http(s)` URL into a clickable link. Callback-only: it
/// takes the message as a prop and renders it, with no application access.
#[component]
pub fn ErrorBanner(message: String) -> Element {
    rsx! {
        div { role: "alert", class: "alert alert-error items-start",
            span { class: "icon-[lucide--circle-alert] size-5 shrink-0 mt-0.5" }
            div { class: "text-sm leading-relaxed whitespace-pre-line",
                for segment in segments(&message) {
                    match segment {
                        Segment::Text(text) => rsx! { "{text}" },
                        Segment::Link(url) => rsx! {
                            a { class: "link link-neutral font-medium break-all", href: "{url}", "{url}" }
                        },
                    }
                }
            }
        }
    }
}

/// One run of a message: either plain text or a URL to linkify.
enum Segment {
    Text(String),
    Link(String),
}

/// Split a message into text and URL segments. A URL starts at `http://` or
/// `https://` and runs until the next whitespace; a trailing sentence
/// punctuation mark is kept as text so links stay clean.
fn segments(message: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut rest = message;

    while let Some(start) = rest.find("http://").or_else(|| rest.find("https://")) {
        if start > 0 {
            segments.push(Segment::Text(rest[..start].to_string()));
        }

        let after = &rest[start..];
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        let mut url = &after[..end];
        let trailing = url.len() - url.trim_end_matches(['.', ',', ')', ']', ';', ':']).len();
        url = &url[..url.len() - trailing];

        segments.push(Segment::Link(url.to_string()));
        rest = &after[url.len()..];
    }

    if !rest.is_empty() {
        segments.push(Segment::Text(rest.to_string()));
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(segments: &[Segment]) -> Vec<(&'static str, String)> {
        segments
            .iter()
            .map(|segment| match segment {
                Segment::Text(text) => ("text", text.clone()),
                Segment::Link(url) => ("link", url.clone()),
            })
            .collect()
    }

    #[test]
    fn splits_text_and_url_with_trailing_punctuation() {
        let message = "Create a token at https://github.com/settings/tokens, then restart.";

        let parts = rendered(&segments(message));

        assert_eq!(
            parts,
            vec![
                ("text", "Create a token at ".to_string()),
                ("link", "https://github.com/settings/tokens".to_string()),
                ("text", ", then restart.".to_string()),
            ]
        );
    }

    #[test]
    fn plain_text_is_a_single_segment() {
        let parts = rendered(&segments("No token configured."));

        assert_eq!(parts, vec![("text", "No token configured.".to_string())]);
    }
}
