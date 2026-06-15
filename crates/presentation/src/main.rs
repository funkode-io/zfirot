//! Zfirot desktop application entry point.

mod app;
mod components;
mod state;

use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

fn main() {
    let window = WindowBuilder::new()
        .with_title("Zfirot")
        .with_always_on_top(false);

    LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .with_context(state::poll_interval_from_env())
        .launch(app::App);
}
