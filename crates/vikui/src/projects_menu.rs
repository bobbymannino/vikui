//! The menu bar's Projects menu: every project and saved filter, to open in the
//! reminders window. Cmd/Ctrl+1 to 9 open the first nine.
//!
//! The list follows the main window's: it is filled whenever the projects load, and
//! emptied on sign-out.

use gpui_kit::{App, Global, KeyBinding, MenuItem};
use reminders::{Destination, Source};
use vikunja::Project;

use crate::app_session;

// The `Action` derive names GPUI as `gpui`; this resolves it through the Kit.
use gpui_kit as gpui;

/// Open the `index`th entry of the Projects menu.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = vikui, no_json)]
pub struct GoToProject {
    pub index: usize,
}

/// How many entries get a Cmd/Ctrl+number shortcut.
const SHORTCUTS: usize = 9;

#[derive(Default)]
struct ProjectsMenu(Vec<Destination>);

impl Global for ProjectsMenu {}

/// Register the action and its key bindings. Call once at startup, before the menu bar
/// is installed so it shows the shortcuts.
pub fn init(cx: &mut App) {
    for index in 0..SHORTCUTS {
        let keystrokes = format!("secondary-{}", index + 1);
        let action = Box::new(GoToProject { index });
        // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`.
        match KeyBinding::load(&keystrokes, action, None, false, None, cx) {
            Ok(binding) => cx.bind_keys([binding]),
            Err(err) => log::error!("invalid key binding {keystrokes}: {err}"),
        }
    }
    cx.on_action(|action: &GoToProject, cx| go_to(action.index, cx));
}

/// List `projects` (from the server) in the menu.
pub fn set(projects: &[Project], cx: &mut App) {
    let destinations = reminders::destinations(projects);
    if cx
        .try_global::<ProjectsMenu>()
        .is_some_and(|menu| menu.0 == destinations)
    {
        return;
    }
    cx.set_global(ProjectsMenu(destinations));
    crate::app_menu::refresh(cx);
}

/// Empty the menu, e.g. on sign-out.
pub fn clear(cx: &mut App) {
    set(&[], cx);
}

/// The menu's items: projects, then saved filters, or a placeholder while none are
/// loaded.
pub fn items(cx: &App) -> Vec<MenuItem> {
    let destinations = cx
        .try_global::<ProjectsMenu>()
        .map_or(&[][..], |menu| &menu.0);
    if destinations.is_empty() {
        return vec![MenuItem::action("No Projects", GoToProject { index: 0 }).disabled(true)];
    }
    let mut items = Vec::with_capacity(destinations.len() + 1);
    let mut after_project = false;
    for (index, destination) in destinations.iter().enumerate() {
        let is_filter = matches!(destination.source, Source::SavedFilter { .. });
        // Saved filters come after the projects, set apart.
        if is_filter && after_project {
            items.push(MenuItem::separator());
        }
        after_project = !is_filter;
        items.push(MenuItem::action(
            destination.name.clone(),
            GoToProject { index },
        ));
    }
    items
}

/// Open the `index`th entry in the reminders window. Does nothing while signed out.
fn go_to(index: usize, cx: &mut App) {
    let Some(source) = cx
        .try_global::<ProjectsMenu>()
        .and_then(|menu| menu.0.get(index))
        .map(|destination| destination.source.clone())
    else {
        return;
    };
    let Some(signed_in) = app_session::get(cx) else {
        return;
    };
    let on_session_expired = signed_in.on_session_expired;
    // Deferred: a key binding's action runs while GPUI is still updating the window it
    // was pressed in, and bringing that window to the front from inside would fail.
    cx.defer(move |cx| {
        crate::reminders_window::open(
            &source,
            signed_in.session,
            move |cx| on_session_expired(cx),
            cx,
        );
    });
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui_kit::test::TestEventExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};
    use login::{Credentials, Session};
    use vikunja::{ServerUrl, Token};

    use super::*;

    fn project(id: i64, title: &str) -> Project {
        Project {
            id,
            title: title.into(),
            description: String::new(),
            identifier: String::new(),
            parent_project_id: 0,
            is_archived: false,
            is_favorite: false,
        }
    }

    /// Background work never runs (the test doesn't park), so nothing is requested.
    fn session() -> Arc<Session> {
        Arc::new(Session::new(Credentials {
            server: ServerUrl::parse("http://127.0.0.1:9").unwrap(),
            username: "test".into(),
            password: "test".into(),
            token: Token::new("test"),
            refresh_token: None,
        }))
    }

    fn names(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                MenuItem::Action { name, .. } => name.to_string(),
                MenuItem::Separator => "-".to_owned(),
                _ => "?".to_owned(),
            })
            .collect()
    }

    #[gpui_kit::test]
    fn lists_projects_and_opens_them_by_shortcut(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            reminders_ui::init(cx);
            init(cx);
            assert_eq!(names(&items(cx)), ["No Projects"]);
            set(
                &[
                    project(1, "Home"),
                    project(-3, "This week"),
                    project(2, "Work"),
                ],
                cx,
            );
            assert_eq!(names(&items(cx)), ["Home", "Work", "-", "This week"]);
        });

        // Signed out, a shortcut does nothing.
        cx.update(|cx| {
            cx.dispatch_action(&GoToProject { index: 1 });
            assert_eq!(cx.windows().len(), 0, "signed out: nothing opens");
            app_session::set(session(), Rc::new(|_| {}), cx);
            cx.dispatch_action(&GoToProject { index: 0 });
        });
        cx.run_until_parked();
        let handle = cx.update(|cx| {
            assert_eq!(cx.windows().len(), 1);
            cx.windows()[0]
        });
        // Cmd/Ctrl+2 opens Work beside Home.
        cx.update_window(handle, |_, window, cx| {
            window.key_down("secondary-2", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let ids: Vec<i64> = crate::reminders_window::shown_sources(cx)
                .into_iter()
                .filter_map(|source| match source {
                    Source::Project { id, .. } => Some(id),
                    _ => None,
                })
                .collect();
            assert_eq!(ids, [1, 2]);

            clear(cx);
            assert_eq!(names(&items(cx)), ["No Projects"]);
        });
    }
}
