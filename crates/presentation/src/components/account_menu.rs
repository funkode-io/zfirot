use dioxus::prelude::*;
use domain::Viewer;

/// The signed-in account menu: shows the Viewer's avatar and `@login` (a
/// generic icon while the Viewer is loading or could not be fetched), and
/// holds the account-level actions — Change token (emits `on_change_token`)
/// and Sign out, guarded by a confirm dialog naming exactly what is removed
/// and what is kept. Callback-only: it neither fetches nor persists anything;
/// confirming Sign out just emits `on_sign_out`.
#[component]
pub fn AccountMenu(
    viewer: Option<Viewer>,
    on_change_token: EventHandler<()>,
    on_sign_out: EventHandler<()>,
) -> Element {
    let mut confirm_open = use_signal(|| false);
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
                li {
                    a {
                        onclick: move |_| on_change_token.call(()),
                        span { class: "icon-[lucide--key-round] size-4" }
                        "Change token…"
                    }
                }
                li {
                    button {
                        class: "text-error",
                        onclick: move |_| confirm_open.set(true),
                        span { class: "icon-[lucide--log-out] size-4" }
                        "Sign out"
                    }
                }
            }
        }
        if confirm_open() {
            SignOutConfirmDialog {
                on_cancel: move |_| confirm_open.set(false),
                on_confirm: move |_| {
                    confirm_open.set(false);
                    on_sign_out.call(());
                },
            }
        }
    }
}

/// The confirm dialog gating "Sign out", naming exactly what it removes
/// (token, tracked repos, last-opened project, cached projects, cached
/// boards) and what it keeps (theme, view mode), per ADR 0005.
#[component]
fn SignOutConfirmDialog(on_cancel: EventHandler<()>, on_confirm: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: "modal modal-open",
            role: "dialog",
            aria_modal: "true",
            aria_labelledby: "sign-out-dialog-title",
            div { class: "modal-box",
                h3 { id: "sign-out-dialog-title", class: "text-lg font-bold", "Sign out?" }
                p { class: "py-2 text-sm",
                    "This removes your Personal Access Token, tracked repos, the "
                    "last-opened project, cached recent projects, and cached "
                    "board snapshots."
                }
                p { class: "text-sm opacity-70",
                    "Your theme and view mode are kept."
                }
                div { class: "modal-action",
                    button {
                        class: "btn btn-ghost",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-error",
                        onclick: move |_| on_confirm.call(()),
                        "Sign out"
                    }
                }
            }
            div {
                class: "modal-backdrop",
                onclick: move |_| on_cancel.call(()),
            }
        }
    }
}
