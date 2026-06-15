//! Zfirot desktop application entry point.

mod app;
mod components;
mod state;

use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

use crate::state::Boot;

fn main() {
    // Load a local .env (if present) before wiring the GitHub adapter.
    let _ = dotenvy::dotenv();

    let window = WindowBuilder::new()
        .with_title("Zfirot")
        .with_always_on_top(false);

    LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .with_context(Boot::from_env())
        .launch(app::App);
}
