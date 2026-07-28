use dioxus::prelude::*;
use domain::Viewer;

/// The signed-in account menu: shows the Viewer's avatar and `@login` (a
/// generic icon while the Viewer is loading or could not be fetched), and
/// holds the account-level actions — Change token, Sign out — added by later
/// tickets. Callback-only: it neither fetches nor persists anything.
#[component]
pub fn AccountMenu(viewer: Option<Viewer>) -> Element {
    // The trigger carries the accessible name (who is signed in, or that the
    // Viewer hasn't loaded/could not be fetched yet) so assistive tech
    // announces it even though the visible "@login" text itself only shows
    // once the dropdown is opened; the avatar image stays decorative (empty
    // `alt`) so it is not announced twice alongside the label.
    let account_label = match &viewer {
        Some(viewer) => format!("Account: signed in as @{}", viewer.login),
        None => "Account".to_string(),
    };

    rsx! {
        div { class: "dropdown dropdown-end",
            div {
                tabindex: "0",
                role: "button",
                class: "btn btn-ghost btn-sm gap-2",
                title: "{account_label}",
                aria_label: "{account_label}",
                if let Some(viewer) = &viewer {
                    div { class: "avatar",
                        div { class: "w-6 rounded-full",
                            img { src: "{viewer.avatar_url}", alt: "" }
                        }
                    }
                } else {
                    div { class: "avatar placeholder",
                        div { class: "bg-neutral text-neutral-content rounded-full w-6",
                            span { class: "icon-[lucide--user] size-4" }
                        }
                    }
                }
                span { class: "icon-[lucide--chevron-down] size-3" }
            }
            ul {
                tabindex: "0",
                class: "dropdown-content menu p-2 shadow bg-base-200 rounded-box w-64 mt-1 z-10",
                li { class: "menu-title text-xs",
                    if let Some(viewer) = &viewer {
                        span { "Signed in as @{viewer.login}" }
                    } else {
                        span { "Signed in" }
                    }
                }
            }
        }
    }
}
