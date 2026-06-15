use dioxus::prelude::*;
use domain::{Project, RepoRef};

/// How many recent projects to show before the user clicks "Show more".
const INITIAL_VISIBLE: usize = 6;

/// The home screen: a grid of recent projects to open. Emits `on_open` with the
/// chosen repository. Callback-only — it neither fetches nor persists anything.
#[component]
pub fn HomeScreen(projects: Vec<Project>, on_open: EventHandler<RepoRef>) -> Element {
    let mut show_all = use_signal(|| false);

    let total = projects.len();
    let visible = if show_all() {
        total
    } else {
        total.min(INITIAL_VISIBLE)
    };

    rsx! {
        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "mb-6",
                h1 { class: "text-2xl font-bold", "Recent projects" }
                p { class: "text-sm opacity-70", "Pick a repository to open its board." }
            }

            if projects.is_empty() {
                div { class: "hero bg-base-100 rounded-box py-16",
                    div { class: "hero-content text-center",
                        div {
                            span { class: "icon-[lucide--folder-open] size-12 opacity-40" }
                            h2 { class: "text-lg font-semibold mt-4", "No projects yet" }
                            p { class: "text-sm opacity-70",
                                "No repositories were found for this token."
                            }
                        }
                    }
                }
            } else {
                div { class: "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4",
                    for project in projects.iter().take(visible).cloned() {
                        ProjectCard { project, on_open }
                    }
                }

                if total > INITIAL_VISIBLE && !show_all() {
                    div { class: "flex justify-center mt-6",
                        button {
                            class: "btn btn-ghost btn-sm",
                            onclick: move |_| show_all.set(true),
                            "Show more"
                        }
                    }
                }
            }
        }
    }
}

/// A single recent-project card. Clicking it emits the repository to open.
#[component]
fn ProjectCard(project: Project, on_open: EventHandler<RepoRef>) -> Element {
    let repo = project.repo.clone();

    rsx! {
        button {
            class: "card card-compact bg-base-100 shadow-sm hover:shadow-md transition-shadow text-left cursor-pointer",
            onclick: move |_| on_open.call(repo.clone()),
            div { class: "card-body",
                div { class: "flex items-center gap-2",
                    span { class: "icon-[lucide--book-marked] size-5 opacity-70" }
                    h3 { class: "card-title text-sm", "{project.repo}" }
                }
            }
        }
    }
}
