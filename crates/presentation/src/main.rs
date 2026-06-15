//! Zfirot desktop application entry point.

mod app;
mod components;
mod state;

use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

use crate::state::Boot;

fn main() {
    // Load a local .env (if present) before wiring the GitHub adapter. A missing
    // file is fine; a malformed/unreadable one is reported so startup failures
    // are diagnosable rather than surfacing later as a misleading "no token".
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(err) if err.not_found() => {}
        Err(err) => eprintln!("Warning: could not load .env: {err}"),
    }

    let window = WindowBuilder::new()
        .with_title("Zfirot")
        .with_always_on_top(false);

    LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .with_context(Boot::from_env())
        .launch(app::App);
}
