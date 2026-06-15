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

    while let Some(start) = next_url_start(rest) {
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

/// The byte index of the earliest `http://` or `https://` in `text`, if any.
///
/// Both schemes must be considered together: scanning for one first would skip
/// past an earlier URL of the other scheme and leave it unlinked.
fn next_url_start(text: &str) -> Option<usize> {
    let http = text.find("http://");
    let https = text.find("https://");
    match (http, https) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
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

    #[test]
    fn linkifies_each_url_when_both_schemes_appear() {
        // The https URL comes first, so an http-first scan would wrongly leave
        // it unlinked. Both must be linkified, in order.
        let message = "See https://secure.example and http://plain.example now.";

        let parts = rendered(&segments(message));

        assert_eq!(
            parts,
            vec![
                ("text", "See ".to_string()),
                ("link", "https://secure.example".to_string()),
                ("text", " and ".to_string()),
                ("link", "http://plain.example".to_string()),
                ("text", " now.".to_string()),
            ]
        );
    }
}
