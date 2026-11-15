// Release builds on Windows are GUI apps: don't open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_actions;
mod app_menu;
mod reminders_window;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::Theme;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, SharedString, WindowBounds, WindowOptions, px,
    size,
};
use login::{Credentials, Session};
use login_ui::Prompt;
use projects_ui::{LoadProjects, ProjectsEvent, ProjectsView};
use reminders::Source;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Components load their icons from the app's asset source; without this they render blank.
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(|cx| {
        gpui_kit::init(cx);
        app_actions::init(cx);
        projects_ui::init(cx);
        reminders_ui::init(cx);
        // Last, so the menus show every key binding.
        app_menu::init(cx);
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
                None => {
                    open_login(Prompt::default(), cx);
                }
            }
            cx.activate(true);
        });
    })
    .detach();
}

fn open_login(prompt: Prompt, cx: &mut App) -> Option<AnyWindowHandle> {
    let options = window_options("Sign in – Vikui", 480., 560., cx);
    login_ui::open(options, prompt, cx, open_overview)
        .inspect_err(|err| log::error!("failed to open login window: {err:#}"))
        .ok()
}

fn open_overview(credentials: Credentials, cx: &mut App) {
    let options = window_options("Vikui", 960., 640., cx);
    let session = Arc::new(Session::new(credentials));
    let load_projects: LoadProjects = {
        let session = Arc::clone(&session);
        Arc::new(move || session.call(vikunja::Client::projects))
    };
    // Every window of this session reports expiry here; only the first one signs out.
    let signed_out = Rc::new(Cell::new(false));
    let expire = move |session: &Session, cx: &mut App| {
        if !signed_out.replace(true) {
            sign_out(session, cx);
        }
    };
    let result = overview_ui::open(options, cx, move |window, cx| {
        let view = cx.new(|cx| ProjectsView::new(load_projects, window, cx));
        cx.subscribe(&view, move |_, event, cx| {
            let source = match event {
                ProjectsEvent::SessionExpired => return expire(&session, cx),
                ProjectsEvent::OpenProject {
                    id,
                    title,
                    subproject_ids,
                } => Source::Project {
                    id: *id,
                    title: title.to_string(),
                    subproject_ids: subproject_ids.clone(),
                },
                ProjectsEvent::OpenSavedFilter { filter_id, title } => Source::SavedFilter {
                    filter_id: *filter_id,
                    title: title.to_string(),
                },
            };
            let expire = expire.clone();
            let expired_session = Arc::clone(&session);
            reminders_window::open(
                &source,
                Arc::clone(&session),
                move |cx| expire(&expired_session, cx),
                cx,
            );
        })
        .detach();
        view.into()
    });
    if let Err(err) = result {
        log::error!("failed to open main window: {err:#}");
    }
}

/// Forget the expired session, close its windows and go back to the sign-in window.
fn sign_out(session: &Session, cx: &mut App) {
    cx.background_spawn(async {
        if let Err(err) = login::credentials::clear() {
            log::warn!("failed to clear expired credentials: {err:#}");
        }
    })
    .detach();

    let credentials = session.credentials();
    let login = open_login(
        Prompt {
            notice: Some("Your session has expired. Sign in again to continue.".into()),
            server: Some(credentials.server.to_string().into()),
            username: Some(credentials.username.into()),
        },
        cx,
    );
    // Close the session's windows only after the sign-in window exists, so the app
    // doesn't quit in between.
    for window in cx.windows() {
        if Some(window) != login {
            window
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
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
fn window_options(
    title: impl Into<SharedString>,
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
