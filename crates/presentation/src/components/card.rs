use dioxus::prelude::*;

/// Background tint of the card surface.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CardBackground {
    /// `bg-base-100`
    #[default]
    Base100,
    /// `bg-base-200`
    Base200,
}

impl CardBackground {
    fn class(self) -> &'static str {
        match self {
            CardBackground::Base100 => "bg-base-100",
            CardBackground::Base200 => "bg-base-200",
        }
    }
}

/// Shadow depth of the card.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CardElevation {
    /// `shadow-sm`
    #[default]
    Sm,
    /// `shadow-md`
    Md,
}

impl CardElevation {
    fn class(self) -> &'static str {
        match self {
            CardElevation::Sm => "shadow-sm",
            CardElevation::Md => "shadow-md",
        }
    }
}

/// The reusable daisyUI card shell: the `card … shadow-*` wrapper plus its
/// `card-body`, in one place. Callback-only — it renders the `children` it is
/// given and emits `on_click` when set, with no application or API access.
///
/// `background`, `elevation`, and `compact` select the common variants; `class`
/// appends extras such as `hover:shadow-md`, `w-full max-w-md`, or a highlight
/// ring. When `on_click` is supplied the shell renders as a focusable `button`
/// (a clickable card); otherwise it renders as a plain `div`.
#[component]
pub fn Card(
    #[props(default)] background: CardBackground,
    #[props(default)] elevation: CardElevation,
    #[props(default = true)] compact: bool,
    #[props(default)] class: String,
    on_click: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    let shell = shell_class(background, elevation, compact, &class);

    match on_click {
        Some(handler) => rsx! {
            button { class: "{shell}", onclick: move |e| handler.call(e),
                div { class: "card-body", {children} }
            }
        },
        None => rsx! {
            div { class: "{shell}",
                div { class: "card-body", {children} }
            }
        },
    }
}

/// Build the outer shell class string from the card variant props. Keeping this
/// pure lets the class composition be unit-tested without a renderer.
fn shell_class(
    background: CardBackground,
    elevation: CardElevation,
    compact: bool,
    extra: &str,
) -> String {
    let mut class = String::from("card");
    if compact {
        class.push_str(" card-compact");
    }
    class.push(' ');
    class.push_str(background.class());
    class.push(' ');
    class.push_str(elevation.class());

    let extra = extra.trim();
    if !extra.is_empty() {
        class.push(' ');
        class.push_str(extra);
    }

    class
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shell_matches_the_compact_base100_card() {
        let class = shell_class(
            CardBackground::default(),
            CardElevation::default(),
            true,
            "",
        );

        assert_eq!(class, "card card-compact bg-base-100 shadow-sm");
    }

    #[test]
    fn base200_compact_shell_matches_the_slice_and_issue_cards() {
        let class = shell_class(CardBackground::Base200, CardElevation::Sm, true, "");

        assert_eq!(class, "card card-compact bg-base-200 shadow-sm");
    }

    #[test]
    fn extras_are_appended_after_the_shell() {
        let class = shell_class(
            CardBackground::Base100,
            CardElevation::Sm,
            true,
            "hover:shadow-md transition-shadow",
        );

        assert_eq!(
            class,
            "card card-compact bg-base-100 shadow-sm hover:shadow-md transition-shadow"
        );
    }

    #[test]
    fn non_compact_md_shell_matches_the_token_card() {
        let class = shell_class(
            CardBackground::Base100,
            CardElevation::Md,
            false,
            "w-full max-w-md",
        );

        assert_eq!(class, "card bg-base-100 shadow-md w-full max-w-md");
    }
}
