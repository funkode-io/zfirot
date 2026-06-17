//! Zfirot desktop application entry point.

mod app;
mod components;
mod state;

use dioxus::desktop::tao::window::Icon;
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

/// The ZF monogram window icon, bundled as a PNG (built from `assets/logo.svg`
/// via `make icon`). Shown in the OS dock/taskbar and window chrome for a plain
/// `cargo run` or a release build. Note that `dx serve` (`make dev`) manages its
/// own dev process, so this icon does not apply there.
const ICON_PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Decode the bundled PNG into a `tao` window icon. Returns `None` (no custom
/// icon, OS default) if the embedded bytes ever fail to decode, so a bad asset
/// can never crash startup.
fn window_icon() -> Option<Icon> {
    let image = image::load_from_memory(ICON_PNG).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).ok()
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let window = WindowBuilder::new()
        .with_title("Zfirot")
        .with_window_icon(window_icon())
        .with_always_on_top(false);

    LaunchBuilder::desktop()
        .with_cfg(Config::new().with_window(window))
        .launch(app::App);
}
