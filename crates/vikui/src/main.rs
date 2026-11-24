// Release builds on Windows are GUI apps: don't open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_actions;
mod app_issues;
mod app_menu;
mod app_preferences;
mod app_session;
mod app_theme;
mod app_tray;
mod app_windows;
mod edit_task_window;
mod filter_editor_window;
mod labels_window;
mod new_reminder_window;
mod project_editor_window;
mod projects_menu;
mod reminders_window;
mod settings_window;
mod task_window;
mod theme_menu;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use app_windows::WindowKind;
use gpui_kit::component::Theme;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Global, WeakEntity};
use login::{Credentials, Session};
use login_ui::Prompt;
use overview_ui::OverviewEvent;
use projects_ui::{LoadProjects, ProjectsEvent, ProjectsView};
use reminders::Source;

/// The app's display name.
const APP_NAME: &str = "Vikui";

/// The app's reverse-DNS identifier, shared with the macOS bundle and Linux desktop entry.
const APP_ID: &str = "dev.bobman.vikui";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Components load their icons from the app's asset source; without this they render blank.
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(|cx| {
        gpui_kit::init(cx);
        app_theme::init(cx);
        app_actions::init(cx);
        projects_menu::init(cx);
        theme_menu::init(cx);
        overview_ui::init(cx);
        projects_ui::init(cx);
        reminders_ui::init(cx);
        new_reminder_ui::init(cx);
        labels_ui::init(cx);
        project_editor_ui::init(cx);
        filter_editor_ui::init(cx);
        // Last, so the menus show every key binding.
        app_menu::init(cx);
        Theme::sync_system_appearance(None, cx);
        quit_when_last_window_closes(cx);
        // Remove what the last update left behind (the old executable on Windows).
        cx.background_spawn(async { updater::clean_up() }).detach();

        // Windows restore their table layouts from the preferences, so open them after,
        // in the saved theme.
        app_preferences::load(cx, |preferences, cx| {
            app_theme::apply(&preferences.theme, cx);
            // Check the saved theme in the menu bar.
            app_menu::refresh(cx);
            open_initial_window(cx);
            if preferences.check_for_updates_on_open {
                settings_window::check_on_launch(cx);
            }
        });
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
    let options = app_windows::options(WindowKind::Login, "Sign in – Vikui", 480., 560., cx);
    let handle = login_ui::open(options, prompt, cx, open_overview)
        .inspect_err(|err| log::error!("failed to open login window: {err:#}"))
        .ok()?;
    app_windows::track(WindowKind::Login, handle, cx);
    Some(handle)
}

fn open_overview(credentials: Credentials, cx: &mut App) {
    let session = Arc::new(Session::new(credentials));
    // Every window of this session reports expiry here; only the first one signs out.
    let signed_out = Rc::new(Cell::new(false));
    let on_session_expired: app_session::OnSessionExpired = Rc::new({
        let session = Arc::clone(&session);
        move |cx| {
            if !signed_out.replace(true) {
                sign_out(&session, cx);
            }
        }
    });
    app_session::set(Arc::clone(&session), Rc::clone(&on_session_expired), cx);
    open_main_window(
        app_session::SignedIn {
            session,
            on_session_expired,
        },
        cx,
    );
    app_tray::apply(cx);
}

/// The main window's projects list, to reload after a project or filter changes.
#[derive(Default)]
struct MainProjects(Option<WeakEntity<ProjectsView>>);

impl Global for MainProjects {}

/// Load the main window's projects (and so the Projects menu) again, if it is open.
fn reload_projects(cx: &mut App) {
    let Some(view) = cx
        .try_global::<MainProjects>()
        .and_then(|main| main.0.as_ref())
        .and_then(WeakEntity::upgrade)
    else {
        return;
    };
    let Some(handle) = app_windows::find(WindowKind::Main, cx) else {
        return;
    };
    // Deferred: the save that asks may finish while another window is updating.
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.refresh(window, cx));
            })
            .ok();
    });
}

/// Bring the main window, which lists the projects, to the front, or reopen it if it was
/// closed. Signed out, the sign-in window stands in for it.
fn open_projects(cx: &mut App) {
    // Deferred: a key binding's action runs while GPUI is still updating the window it
    // was pressed in, and bringing a window to the front from inside would fail.
    cx.defer(|cx| {
        let signed_in = app_session::get(cx);
        let kind = if signed_in.is_some() {
            WindowKind::Main
        } else {
            WindowKind::Login
        };
        if let Some(handle) = app_windows::find(kind, cx)
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        match signed_in {
            Some(signed_in) => open_main_window(signed_in, cx),
            None => {
                open_login(Prompt::default(), cx);
            }
        }
        cx.activate(true);
    });
}

/// Open the main window, listing the projects of the signed-in session.
fn open_main_window(signed_in: app_session::SignedIn, cx: &mut App) {
    let options = app_windows::options(WindowKind::Main, APP_NAME, 960., 640., cx);
    let app_session::SignedIn {
        session,
        on_session_expired: expire,
    } = signed_in;
    let load_projects: LoadProjects = {
        let session = Arc::clone(&session);
        Arc::new(move || session.call(vikunja::Client::projects))
    };
    let search_session = Arc::clone(&session);
    let search_expire = Rc::clone(&expire);
    let result = overview_ui::open(options, cx, move |window, cx| {
        let view = cx.new(|cx| {
            let mut view = ProjectsView::new(load_projects, window, cx);
            let layout = app_preferences::get(cx).tables.projects;
            view.set_table_layout(&layout, cx);
            view
        });
        cx.set_global(MainProjects(Some(view.downgrade())));
        cx.subscribe(&view, move |_, event, cx| {
            let source = match event {
                ProjectsEvent::SessionExpired => return expire(cx),
                ProjectsEvent::Loaded(projects) => return projects_menu::set(projects, cx),
                ProjectsEvent::EditProject { id } => return project_editor_window::edit(*id, cx),
                ProjectsEvent::EditSavedFilter { filter_id } => {
                    return filter_editor_window::edit(*filter_id, cx);
                }
                ProjectsEvent::TableLayoutChanged(layout) => {
                    let layout = layout.clone();
                    return app_preferences::update(cx, |preferences| {
                        preferences.tables.projects = layout;
                    });
                }
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
            let expire = Rc::clone(&expire);
            reminders_window::open(&source, Arc::clone(&session), move |cx| expire(cx), cx);
        })
        .detach();
        view.into()
    });
    match result {
        Ok((handle, overview)) => {
            app_windows::track(WindowKind::Main, handle, cx);
            cx.subscribe(&overview, move |_, event, cx| match event {
                OverviewEvent::Search(text) => {
                    let expire = Rc::clone(&search_expire);
                    reminders_window::open_search(
                        text,
                        Arc::clone(&search_session),
                        move |cx| expire(cx),
                        cx,
                    );
                }
                OverviewEvent::OpenSettings => settings_window::open(cx),
            })
            .detach();
        }
        Err(err) => log::error!("failed to open main window: {err:#}"),
    }
}

/// Forget the expired session, close its windows and go back to the sign-in window.
fn sign_out(session: &Session, cx: &mut App) {
    app_session::clear(cx);
    projects_menu::clear(cx);
    app_tray::apply(cx);
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

/// macOS keeps apps alive with no windows; Vikui has nothing to show without one, unless
/// its tray icon shows.
fn quit_when_last_window_closes(cx: &mut App) {
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() && !app_tray::is_shown(cx) {
            cx.quit();
        }
    })
    .detach();
}
