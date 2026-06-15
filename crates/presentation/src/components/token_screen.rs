use dioxus::prelude::*;

/// A paste-token screen: the developer enters a fine-grained Personal Access
/// Token and it is emitted via `on_submit`. Callback-only — it never touches the
/// secure store or GitHub; the page wires it to the application use-cases. An
/// optional `error` (e.g. an invalid or insufficient-permission token) is shown
/// inline.
#[component]
pub fn TokenScreen(on_submit: EventHandler<String>, error: Option<String>) -> Element {
    let mut token = use_signal(String::new);
    let is_blank = token.read().trim().is_empty();

    rsx! {
        div { class: "min-h-screen bg-base-200 flex items-center justify-center p-6",
            div { class: "card w-full max-w-md bg-base-100 shadow-md",
                div { class: "card-body",
                    h2 { class: "card-title", "Connect to GitHub" }
                    p { class: "text-sm opacity-70",
                        "Paste a fine-grained Personal Access Token. It is saved to your operating system's secure store and reused on every launch."
                    }
                    label { class: "form-control w-full",
                        div { class: "label",
                            span { class: "label-text", "Personal Access Token" }
                        }
                        input {
                            r#type: "password",
                            class: "input input-bordered w-full",
                            placeholder: "github_pat_…",
                            value: "{token}",
                            oninput: move |evt| token.set(evt.value()),
                        }
                    }
                    if let Some(message) = error {
                        div { class: "alert alert-error text-sm", "{message}" }
                    }
                    div { class: "card-actions justify-end mt-2",
                        button {
                            class: "btn btn-primary",
                            disabled: is_blank,
                            onclick: move |_| on_submit.call(token.read().clone()),
                            "Save token"
                        }
                    }
                }
            }
        }
    }
}
