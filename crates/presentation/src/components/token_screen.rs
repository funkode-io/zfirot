use dioxus::prelude::*;

/// A paste-token screen: the developer enters a fine-grained Personal Access
/// Token and it is emitted via `on_submit`. Callback-only — it never touches the
/// secure store or GitHub; the page wires it to the application use-cases. An
/// optional `error` (e.g. an invalid or insufficient-permission token) is shown
/// inline. While `saving` is set the submit is in flight: the button shows a
/// spinner and is disabled so the token is not validated twice.
///
/// `title`/`description` default to the first-launch "Connect to GitHub" copy;
/// the Change-token (Rotate) view passes its own explanatory copy instead. When
/// `on_cancel` is set a Cancel button shows alongside Save, so the Change-token
/// view can return to wherever the user was without touching the stored token —
/// the first-launch screen has no such path since there is nowhere to return to.
/// `highlighted_permission` names one of the required permissions below to mark
/// distinctly (e.g. an Under-scoped token's missing grant) — `None` renders the
/// list plainly.
#[component]
pub fn TokenScreen(
    on_submit: EventHandler<String>,
    error: Option<String>,
    #[props(default)] saving: bool,
    #[props(default)] title: Option<String>,
    #[props(default)] description: Option<String>,
    #[props(default)] on_cancel: Option<EventHandler<()>>,
    #[props(default)] highlighted_permission: Option<String>,
) -> Element {
    let mut token = use_signal(String::new);
    let is_blank = token.read().trim().is_empty();
    let title = title.unwrap_or_else(|| "Connect to GitHub".to_string());
    let description = description.unwrap_or_else(|| {
        "Zfirot reads your project board from GitHub. Create a fine-grained Personal Access Token, grant it the permissions below, then paste it here. It is saved to your operating system's secure store and reused on every launch.".to_string()
    });
    let required_permissions = [
        "Issues — Read and write",
        "Pull requests — Read-only",
        "Contents — Read-only",
    ];

    rsx! {
        div { class: "min-h-screen bg-base-100 flex items-center justify-center p-6",
            div { class: "card w-full max-w-md bg-base-200 shadow-md",
                div { class: "card-body",
                    h2 { class: "card-title", "{title}" }
                    // The Change-token flow (when `on_cancel` is set) renders its
                    // "why" text as a distinct info banner, per the Change-token
                    // view's spec; the first-launch screen keeps its plain intro
                    // paragraph unchanged.
                    if on_cancel.is_some() {
                        div { class: "alert alert-info text-sm", "{description}" }
                    } else {
                        p { class: "text-sm opacity-70", "{description}" }
                    }
                    a {
                        class: "link link-primary text-sm inline-flex items-center gap-1",
                        href: "https://github.com/settings/personal-access-tokens/new",
                        span { class: "icon-[lucide--external-link] size-4" }
                        "Create a fine-grained token on GitHub"
                    }
                    div { class: "rounded-box bg-base-300 p-3 text-sm",
                        p { class: "font-medium mb-1", "Required repository permissions:" }
                        ul { class: "list-disc list-inside opacity-80",
                            for permission in required_permissions {
                                if names_permission(permission, highlighted_permission.as_deref()) {
                                    li { class: "font-semibold text-warning list-none flex items-center gap-1",
                                        span { class: "icon-[lucide--alert-triangle] size-4" }
                                        "{permission} (missing)"
                                    }
                                } else {
                                    li { "{permission}" }
                                }
                            }
                        }
                        if let Some(missing) = highlighted_permission.as_deref().filter(|missing| {
                            !required_permissions
                                .iter()
                                .any(|permission| names_permission(permission, Some(missing)))
                        }) {
                            p { class: "text-warning font-medium mt-2",
                                "Also missing: {missing}"
                            }
                        }
                    }
                    div { class: "flex flex-col gap-1 w-full",
                        label { class: "text-sm font-medium", "Personal Access Token" }
                        input {
                            r#type: "password",
                            class: "input w-full",
                            placeholder: "github_pat_…",
                            value: "{token.read()}",
                            oninput: move |evt| token.set(evt.value()),
                        }
                    }
                    if let Some(message) = error {
                        div { class: "alert alert-error text-sm", "{message}" }
                    }
                    div { class: "card-actions justify-end mt-2",
                        if let Some(on_cancel) = on_cancel {
                            button {
                                class: "btn btn-ghost",
                                disabled: saving,
                                onclick: move |_| on_cancel.call(()),
                                "Cancel"
                            }
                        }
                        button {
                            class: "btn btn-primary",
                            disabled: is_blank || saving,
                            onclick: move |_| on_submit.call(token.read().clone()),
                            if saving {
                                span { class: "loading loading-spinner" }
                                "Validating…"
                            } else {
                                "Save token"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Whether a required-permission line (e.g. `"Issues — Read and write"`) is the
/// one a Forbidden error named as missing. A loose, case-insensitive
/// containment match in either direction since GitHub's own wording and this
/// screen's copy are not guaranteed to align character-for-character — e.g. a
/// missing permission of `"Issues"` matches the `"Issues — Read and write"`
/// line, and a fuller `"Issues: Read and write"` matches too: punctuation
/// (`:`, `—`, `-`, ...) is normalized to spaces before comparing so wording
/// differences in separators never block a real match.
fn names_permission(permission_line: &str, missing_permission: Option<&str>) -> bool {
    match missing_permission {
        Some(missing) if !missing.trim().is_empty() => {
            let line = normalize_for_matching(permission_line);
            let missing = normalize_for_matching(missing);
            line.contains(&missing) || missing.contains(&line)
        }
        _ => false,
    }
}

/// Lowercase, with every run of non-alphanumeric characters collapsed to a
/// single space, so `"Issues: Read and write"` and `"Issues — Read and write"`
/// compare as the same words regardless of punctuation.
fn normalize_for_matching(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut last_was_space = false;
    for ch in text.to_lowercase().chars() {
        if ch.is_alphanumeric() {
            normalized.push(ch);
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_permission_named_by_its_leading_word() {
        assert!(names_permission("Issues — Read and write", Some("Issues")));
    }

    #[test]
    fn matches_case_insensitively() {
        assert!(names_permission(
            "Pull requests — Read-only",
            Some("pull requests")
        ));
    }

    #[test]
    fn matches_despite_different_punctuation_between_the_wordings() {
        assert!(names_permission(
            "Issues — Read and write",
            Some("Issues: Read and write")
        ));
    }

    #[test]
    fn does_not_match_a_different_permission() {
        assert!(!names_permission("Contents — Read-only", Some("Issues")));
    }

    #[test]
    fn does_not_match_when_nothing_is_missing() {
        assert!(!names_permission("Issues — Read and write", None));
    }
}
