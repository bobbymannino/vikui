// Release builds on Windows are GUI apps: don't open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gpui_kit::component::Theme;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size,
};
use login::{Credentials, Session};
use login_ui::Prompt;
use projects_ui::{LoadProjects, ProjectsEvent, ProjectsView};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Components load their icons from the app's asset source; without this they render blank.
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(|cx| {
        gpui_kit::init(cx);
        projects_ui::init(cx);
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
        let credentials = load.await.unwrap_or_else(|err| {
            log::warn!("treating user as signed out: {err:#}");
            None
        });
        cx.update(|cx| {
            match credentials {
                Some(credentials) => open_overview(credentials, cx),
                None => open_login(Prompt::default(), cx),
            }
            cx.activate(true);
        });
    })
    .detach();
}

fn open_login(prompt: Prompt, cx: &mut App) {
    let options = window_options("Sign in – Vikui", 480., 560., cx);
    if let Err(err) = login_ui::open(options, prompt, cx, open_overview) {
        log::error!("failed to open login window: {err:#}");
    }
}

fn open_overview(credentials: Credentials, cx: &mut App) {
    let options = window_options("Vikui", 960., 640., cx);
    let session = Arc::new(Session::new(credentials));
    let load_projects: LoadProjects = {
        let session = Arc::clone(&session);
        Arc::new(move || session.call(vikunja::Client::projects))
    };
    let result = overview_ui::open(options, cx, move |window, cx| {
        let main_window = window.window_handle();
        let view = cx.new(|cx| ProjectsView::new(load_projects, window, cx));
        let mut signed_out = false;
        cx.subscribe(&view, move |_, event, cx| match event {
            ProjectsEvent::SessionExpired if !signed_out => {
                signed_out = true;
                sign_out(&session, main_window, cx);
            }
            ProjectsEvent::SessionExpired => {}
        })
        .detach();
        view.into()
    });
    if let Err(err) = result {
        log::error!("failed to open main window: {err:#}");
    }
}

/// Forget the expired session and send the user back to the sign-in window.
fn sign_out(session: &Session, main_window: AnyWindowHandle, cx: &mut App) {
    cx.background_spawn(async {
        if let Err(err) = login::credentials::clear() {
            log::warn!("failed to clear expired credentials: {err:#}");
        }
    })
    .detach();

    let credentials = session.credentials();
    open_login(
        Prompt {
            notice: Some("Your session has expired. Sign in again to continue.".into()),
            server: Some(credentials.server.to_string().into()),
            username: Some(credentials.username.into()),
        },
        cx,
    );
    // Close the main window only after the sign-in window exists, so the app doesn't quit.
    main_window
        .update(cx, |_, window, _| window.remove_window())
        .ok();
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
fn window_options(title: &'static str, width: f32, height: f32, cx: &App) -> WindowOptions {
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
