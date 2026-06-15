use dioxus::prelude::*;

/// A daisyUI loading spinner with an optional caption. Purely presentational —
/// callback-only, it makes no application or API calls; callers decide where to
/// place it (a button, a panel, a full screen). `label` doubles as the
/// accessible name so a screen reader announces the in-flight work.
#[component]
pub fn Spinner(#[props(default)] label: Option<String>) -> Element {
    let aria = label.clone().unwrap_or_else(|| "Loading".into());
    rsx! {
        div { class: "flex items-center gap-2",
            span {
                class: "loading loading-spinner loading-lg",
                role: "status",
                "aria-label": "{aria}",
            }
            if let Some(label) = label {
                span { class: "text-sm opacity-70", "{label}" }
            }
        }
    }
}

/// A full-height, centered [`Spinner`] for whole-screen loading states. Reserves
/// the full viewport height so swapping it for the resolved screen causes no
/// layout jump.
#[component]
pub fn LoadingScreen(#[props(default)] label: Option<String>) -> Element {
    rsx! {
        div { class: "min-h-screen bg-base-200 grid place-items-center",
            Spinner { label }
        }
    }
}
