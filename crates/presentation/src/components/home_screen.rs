use dioxus::prelude::*;
use domain::{filter_home, HomeFilter, Project, RepoRef};

/// How many recent projects to show before the user clicks "Show more".
const INITIAL_VISIBLE: usize = 6;

/// The home screen: a search box over the discovered projects with a gated
/// direct-open action. Emits `on_open_discovered` when a matching project is
/// clicked, and `on_open_goto` when the go-to action is triggered. Callback-only
/// — it neither fetches nor persists anything.
///
/// Typing filters the visible projects by case-insensitive substring on
/// `owner/name`. When nothing matches but the query is a valid `owner/repo`, a
/// single "Go to" action appears (Enter also triggers it); when it is not a
/// valid repo path, a quiet hint is shown instead. The pure decision lives in
/// [`filter_home`].
#[component]
pub fn HomeScreen(
    projects: Vec<Project>,
    tracked_repos: Vec<RepoRef>,
    on_open_discovered: EventHandler<RepoRef>,
    on_open_goto: EventHandler<RepoRef>,
) -> Element {
    let mut show_all = use_signal(|| false);
    let mut query = use_signal(String::new);

    let raw = query.read().clone();
    let outcome = filter_home(&raw, &projects);

    // The repo to open if the user presses Enter — only set while the gated
    // "Go to" action is showing. Cloned into the keydown closure (it owns a
    // non-`Copy` `RepoRef`); `on_open_goto` is a `Copy` handle.
    let goto_on_enter = match &outcome {
        HomeFilter::GoTo(repo) => Some(repo.clone()),
        _ => None,
    };

    // Build the result body outside the `rsx!` markup: all branching and list
    // computation lives here so the markup block below stays declarative.
    let body = match outcome {
        HomeFilter::Filtered(matches) => {
            let total = matches.len();
            let visible = if show_all() {
                total
            } else {
                total.min(INITIAL_VISIBLE)
            };

            // Tracked repos already discovered are dropped: a repo present in
            // both lists renders once, under the discovered grid.
            let discovered_repos: Vec<RepoRef> = projects.iter().map(|p| p.repo.clone()).collect();
            let de_duped_tracked: Vec<RepoRef> = tracked_repos
                .iter()
                .filter(|repo| !discovered_repos.contains(repo))
                .cloned()
                .collect();
            // The Tracked section only shows on an unfiltered home.
            let show_tracked = raw.trim().is_empty() && !de_duped_tracked.is_empty();

            rsx! {
                if !matches.is_empty() {
                    div { class: "mb-8",
                        h2 { class: "text-lg font-semibold mb-4", "Recent projects" }
                        div { class: "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4",
                            for project in matches.into_iter().take(visible) {
                                ProjectCard { project, on_open: on_open_discovered }
                            }
                        }
                        if total > INITIAL_VISIBLE && !show_all() {
                            div { class: "flex justify-center mt-4",
                                button {
                                    class: "btn btn-ghost btn-sm",
                                    onclick: move |_| show_all.set(true),
                                    "Show more"
                                }
                            }
                        }
                    }
                }
                if show_tracked {
                    div { class: "border-t pt-8",
                        h2 { class: "text-lg font-semibold mb-4", "Tracked" }
                        div { class: "space-y-2",
                            for repo in de_duped_tracked {
                                button {
                                    class: "w-full text-left px-4 py-3 hover:bg-base-200 rounded transition",
                                    onclick: move |_| on_open_discovered.call(repo.clone()),
                                    span { class: "icon-[lucide--link] size-4 inline-block mr-2 opacity-60" }
                                    "{repo}"
                                }
                            }
                        }
                    }
                }
            }
        }
        HomeFilter::GoTo(repo) => {
            let label = format!("Go to {repo}");
            rsx! {
                div { class: "w-full max-w-sm",
                    button {
                        class: "btn btn-primary w-full",
                        onclick: move |_| on_open_goto.call(repo.clone()),
                        "{label}"
                    }
                }
            }
        }
        HomeFilter::Hint => {
            let no_projects_yet = projects.is_empty() && raw.trim().is_empty();
            rsx! {
                if no_projects_yet {
                    div { class: "hero bg-base-100 rounded-box py-16",
                        div { class: "hero-content text-center",
                            div {
                                span { class: "icon-[lucide--folder-open] size-12 opacity-40" }
                                h2 { class: "text-lg font-semibold mt-4", "No projects yet" }
                                p { class: "text-sm opacity-70",
                                    "No repositories were found for this token. Type a full owner/repo above to open one directly."
                                }
                            }
                        }
                    }
                } else {
                    p { class: "text-sm opacity-60 max-w-sm",
                        "No matches — type a full owner/repo to open it directly."
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "min-h-screen bg-base-200 p-6",
            header { class: "mb-6",
                h1 { class: "text-2xl font-bold", "Recent projects" }
                p { class: "text-sm opacity-70",
                    "Search your projects, or type a full owner/repo to open it directly."
                }
            }

            div { class: "mb-6 w-full max-w-sm",
                p { class: "text-sm font-medium mb-1", "Search or open a repository" }
                input {
                    r#type: "text",
                    class: "input w-full",
                    placeholder: "owner/repo",
                    value: "{raw}",
                    oninput: move |evt| {
                        query.set(evt.value());
                        show_all.set(false);
                    },
                    onkeydown: move |evt| {
                        if evt.key() == Key::Enter {
                            if let Some(repo) = goto_on_enter.clone() {
                                on_open_goto.call(repo);
                            }
                        }
                    },
                }
            }

            {body}
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
