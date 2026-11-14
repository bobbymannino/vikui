// Release builds on Windows are GUI apps: don't open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod views;

use gpui_kit::component::Theme;
use gpui_kit::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        Theme::sync_system_appearance(None, cx);
        quit_when_last_window_closes(cx);

        open_initial_window(cx);
    });
}

/// Show the main window if the user is signed in, otherwise the login window.
///
/// The keychain read can block (or prompt), so it runs off the main thread.
fn open_initial_window(cx: &mut App) {
    let load = cx.background_spawn(async { login::credentials::load() });
    cx.spawn(async move |cx| {
        let signed_in = match load.await {
            Ok(credentials) => credentials.is_some(),
            Err(err) => {
                log::warn!("treating user as signed out: {err:#}");
                false
            }
        };
        cx.update(|cx| {
            if signed_in {
                open_overview(cx);
            } else {
                views::login_window::open(cx);
            }
            cx.activate(true);
        });
    })
    .detach();
}

pub(crate) fn open_overview(cx: &mut App) {
    let options = window_options("Vikui", 960., 640., cx);
    if let Err(err) = overview::open(options, cx) {
        log::error!("failed to open main window: {err:#}");
    }
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
