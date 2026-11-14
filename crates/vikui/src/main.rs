// Release builds on Windows are GUI apps: don't open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod credentials;
mod views;

use gpui_kit::component::Theme;
use gpui_kit::{App, Bounds, WindowBounds, WindowOptions, px, size};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        Theme::sync_system_appearance(None, cx);
        quit_when_last_window_closes(cx);

        views::main_window::open(cx);
        cx.activate(true);
    });
}

/// macOS keeps apps alive with no windows; Vikui has nothing to show without one.
fn quit_when_last_window_closes(cx: &mut App) {
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

/// Centred window options with a sensible minimum size.
pub(crate) fn window_options(
    title: &'static str,
    width: f32,
    height: f32,
    cx: &App,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        window_min_size: Some(size(px(width * 0.75), px(height * 0.75))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}
