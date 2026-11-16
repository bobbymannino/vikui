//! Opens the reminders window for a project or filter, and the search window for tasks in
//! every project, and fills them in. There is one of each at a time: opening another
//! project replaces the reminders window, and a new search reuses the search window.
//!
//! The window's details side panel is a [`ReminderDetailsView`], filled from the tasks
//! loaded for the list. Marking a task done from the panel, the list or a task window
//! saves it here. Tasks open in windows of their own through [`task_window`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global, WeakEntity};
use login::Session;
use reminder_details::ReminderDetails;
use reminder_details_ui::{ReminderDetailsEvent, ReminderDetailsView};
use reminders::Reminder;
use reminders::Source;
use reminders_ui::{RemindersEvent, RemindersView};
use vikunja::{Client, Task};

use crate::task_window::{self, SetDone};

/// What one window has loaded, shared by its list and details panel.
type Tasks = Rc<RefCell<Loaded>>;

/// Tasks by id, and project titles by id for naming each task's project.
#[derive(Default)]
struct Loaded {
    tasks: HashMap<i64, Task>,
    projects: HashMap<i64, String>,
}

impl Loaded {
    /// Details for `reminder`, which must belong to `task`.
    fn details(&self, reminder: &Reminder, task: &Task) -> ReminderDetails {
        let project = self.projects.get(&task.project_id).map(String::as_str);
        ReminderDetails::new(reminder, task, project)
    }
}

/// An open window and what loads it, so opening another source replaces or reuses it
/// instead of piling up windows.
struct Opened {
    handle: AnyWindowHandle,
    view: WeakEntity<RemindersView>,
    load: Rc<Loader>,
}

impl Opened {
    fn source(&self) -> Source {
        self.load.source.borrow().clone()
    }
}

/// The reminders window (a project or filter) and the search window.
#[derive(Default)]
struct RemindersWindow {
    browse: Option<Opened>,
    search: Option<Opened>,
}

impl Global for RemindersWindow {}

/// Runs when the server rejects the session for good.
type OnSessionExpired = Rc<dyn Fn(&mut App)>;

/// Only one reminders window is open at a time: opening the source it already lists
/// brings it to the front, and opening another replaces it in the same place.
///
/// `on_session_expired` runs if the server rejects the session for good.
pub fn open(
    source: &Source,
    session: Arc<Session>,
    on_session_expired: impl Fn(&mut App) + 'static,
    cx: &mut App,
) {
    let previous = cx
        .default_global::<RemindersWindow>()
        .browse
        .take()
        .filter(|opened| opened.view.upgrade().is_some());
    let mut bounds = None;
    if let Some(previous) = previous {
        let same = previous.source() == *source;
        let alive = previous
            .handle
            .update(cx, |_, window, _| {
                if same {
                    window.activate_window();
                } else {
                    bounds = Some(window.window_bounds());
                    window.remove_window();
                }
            })
            .is_ok();
        if same && alive {
            cx.default_global::<RemindersWindow>().browse = Some(previous);
            return;
        }
    }

    let title = source.title();
    let mut options = crate::window_options(title.clone(), 1200., 560., cx);
    if bounds.is_some() {
        options.window_bounds = bounds;
    }
    let (handle, view) = match reminders_ui::open(options, title, None, cx) {
        Ok(opened) => opened,
        Err(err) => {
            log::error!("failed to open reminders window: {err:#}");
            return;
        }
    };
    let opened = fill(
        handle,
        &view,
        source,
        session,
        Rc::new(on_session_expired),
        cx,
    );
    cx.default_global::<RemindersWindow>().browse = Some(opened);
}

/// Show the tasks matching `text` in every project in the search window, opening it if
/// needed.
///
/// `on_session_expired` runs if the server rejects the session for good.
pub fn open_search(
    text: &str,
    session: Arc<Session>,
    on_session_expired: impl Fn(&mut App) + 'static,
    cx: &mut App,
) {
    let source = Source::Search(text.to_owned());
    let open = cx
        .default_global::<RemindersWindow>()
        .search
        .as_ref()
        .filter(|opened| opened.view.upgrade().is_some())
        .map(|opened| (opened.handle, opened.view.clone(), Rc::clone(&opened.load)));
    if let Some((handle, view, load)) = open
        && let Some(view) = view.upgrade()
        && handle
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.set_server_search(text, window, cx));
                window.activate_window();
            })
            .is_ok()
    {
        load.search(source, cx);
        return;
    }

    let options = crate::window_options(source.title(), 1200., 560., cx);
    let (handle, view) = match reminders_ui::open_search(options, text, cx) {
        Ok(opened) => opened,
        Err(err) => {
            log::error!("failed to open search window: {err:#}");
            return;
        }
    };
    let opened = fill(
        handle,
        &view,
        &source,
        session,
        Rc::new(on_session_expired),
        cx,
    );
    let load = Rc::clone(&opened.load);
    cx.default_global::<RemindersWindow>().search = Some(opened);
    cx.subscribe(&view, move |_, event, cx| {
        if let RemindersEvent::Search(text) = event {
            load.search(Source::Search(text.to_string()), cx);
        }
    })
    .detach();
}

/// Load `source` into a newly opened window, and wire up its details panel, refresh and
/// new reminder button.
fn fill(
    handle: AnyWindowHandle,
    view: &Entity<RemindersView>,
    source: &Source,
    session: Arc<Session>,
    on_session_expired: OnSessionExpired,
    cx: &mut App,
) -> Opened {
    let tasks: Tasks = Rc::default();
    attach_details_panel(
        handle,
        view,
        Rc::clone(&tasks),
        Arc::clone(&session),
        Rc::clone(&on_session_expired),
        cx,
    );

    let load = Rc::new(Loader {
        source: RefCell::new(source.clone()),
        handle,
        session,
        view: view.downgrade(),
        tasks,
        on_session_expired,
    });
    load.run(cx);
    cx.subscribe(view, {
        let load = Rc::clone(&load);
        move |_, event, cx| match event {
            RemindersEvent::Refresh => load.run(cx),
            RemindersEvent::NewReminder => crate::new_reminder_window::open(cx),
            _ => {}
        }
    })
    .detach();
    Opened {
        handle,
        view: view.downgrade(),
        load,
    }
}

/// Reload the open reminders and search windows, e.g. after adding a reminder they may
/// list.
pub fn reload(cx: &mut App) {
    let loads: Vec<Rc<Loader>> = cx
        .try_global::<RemindersWindow>()
        .into_iter()
        .flat_map(|open| [open.browse.as_ref(), open.search.as_ref()])
        .flatten()
        .filter(|opened| opened.view.upgrade().is_some())
        .map(|opened| Rc::clone(&opened.load))
        .collect();
    for load in loads {
        load.run(cx);
    }
}

/// One finished load: the list's reminders, their tasks and project titles by id.
struct Load {
    reminders: Vec<Reminder>,
    tasks: Vec<Task>,
    projects: HashMap<i64, String>,
}

/// Loads a window's tasks into its list, on open and on every refresh.
struct Loader {
    /// Changes when the search window searches again.
    source: RefCell<Source>,
    handle: AnyWindowHandle,
    session: Arc<Session>,
    view: WeakEntity<RemindersView>,
    tasks: Tasks,
    on_session_expired: OnSessionExpired,
}

impl Loader {
    /// Load `source` instead, titling the window after it.
    fn search(self: &Rc<Self>, source: Source, cx: &mut App) {
        let title = source.title();
        *self.source.borrow_mut() = source;
        self.handle
            .update(cx, |_, window, _| window.set_window_title(&title))
            .ok();
        self.run(cx);
    }

    fn run(self: &Rc<Self>, cx: &mut App) {
        let source = self.source.borrow().clone();
        let session = Arc::clone(&self.session);
        let load = cx.background_spawn(async move {
            let tasks = session.call(|client, token| source.tasks(client, token))?;
            // Only names projects in the details panel, so the list still shows without it.
            let projects = session
                .call(Client::projects)
                .inspect_err(|err| log::warn!("failed to load project names: {err:#}"))
                .unwrap_or_default()
                .into_iter()
                .map(|project| (project.id, project.title))
                .collect();
            let reminders = reminders::from_tasks(&tasks);
            log::debug!(
                "{source:?}: {} tasks, {} reminders",
                tasks.len(),
                reminders.len()
            );
            Ok(Load {
                reminders,
                tasks,
                projects,
            })
        });
        let this = Rc::clone(self);
        cx.spawn(async move |cx| {
            let result = load.await;
            cx.update(|cx| this.finish(result, cx));
        })
        .detach();
    }

    fn finish(&self, result: Result<Load, vikunja::Error>, cx: &mut App) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        match result {
            Ok(Load {
                reminders,
                tasks,
                projects,
            }) => {
                let count = tasks.len();
                // Before `set_reminders`, which may ask for the open reminder's details.
                *self.tasks.borrow_mut() = Loaded {
                    tasks: tasks.into_iter().map(|task| (task.id, task)).collect(),
                    projects,
                };
                view.update(cx, |view, cx| {
                    view.set_reminders(&reminders, Some(count), cx);
                });
            }
            Err(vikunja::Error::Unauthorized) => (self.on_session_expired)(cx),
            Err(err) => {
                log::warn!("failed to load reminders: {err:#}");
                view.update(cx, |view, cx| view.set_error(err.to_string(), cx));
            }
        }
    }
}

/// Give the window a details panel: fill it when a reminder is opened, save the task when
/// it is marked done, close it on request, and open tasks in windows of their own.
fn attach_details_panel(
    handle: AnyWindowHandle,
    view: &Entity<RemindersView>,
    tasks: Tasks,
    session: Arc<Session>,
    on_session_expired: OnSessionExpired,
    cx: &mut App,
) {
    let details = cx.new(|_| ReminderDetailsView::new());
    view.update(cx, |view, cx| {
        view.set_details_panel(details.clone().into(), cx);
    });
    // The reminder the panel shows, to refresh it after a save.
    let shown: Rc<RefCell<Option<Reminder>>> = Rc::default();

    // Marking done from the list, the panel or a task window all save the same way.
    let server = session.server();
    let set_done: SetDone = {
        let targets = SaveTargets {
            view: view.downgrade(),
            details: details.downgrade(),
            tasks: Rc::clone(&tasks),
            shown: Rc::clone(&shown),
        };
        Rc::new(move |task_id, done, cx| {
            save_done(
                targets.clone(),
                Arc::clone(&session),
                Rc::clone(&on_session_expired),
                task_id,
                done,
                cx,
            );
        })
    };
    let open_task = {
        let tasks = Rc::clone(&tasks);
        let set_done = Rc::clone(&set_done);
        let server = server.clone();
        move |reminder: &Reminder, cx: &mut App| {
            let loaded = tasks.borrow();
            let Some(task) = loaded.tasks.get(&reminder.task_id) else {
                return log::warn!("no loaded task {} to open", reminder.task_id);
            };
            let info = loaded.details(reminder, task);
            drop(loaded);
            task_window::open(reminder, &info, server.clone(), Rc::clone(&set_done), cx);
        }
    };

    cx.subscribe(view, {
        let details = details.clone();
        let tasks = Rc::clone(&tasks);
        let shown = Rc::clone(&shown);
        let set_done = Rc::clone(&set_done);
        let open_task = open_task.clone();
        let server = server.clone();
        move |_, event, cx| match event {
            RemindersEvent::ShowDetails(reminder) => {
                let loaded = tasks.borrow();
                let Some(task) = loaded.tasks.get(&reminder.task_id) else {
                    log::warn!("no loaded task {} for reminder details", reminder.task_id);
                    return;
                };
                let info = loaded.details(reminder, task);
                drop(loaded);
                *shown.borrow_mut() = Some(reminder.clone());
                details.update(cx, |details, cx| details.set_details(&info, cx));
            }
            RemindersEvent::SetDone { task_id, done } => set_done(*task_id, *done, cx),
            RemindersEvent::OpenTask(reminder) => open_task(reminder, cx),
            RemindersEvent::OpenInBrowser { task_id } => {
                task_window::open_in_browser(&server, *task_id, cx);
            }
            RemindersEvent::EditTask { task_id } => crate::edit_task_window::open(*task_id, cx),
            RemindersEvent::Refresh | RemindersEvent::NewReminder | RemindersEvent::Search(_) => {}
        }
    })
    .detach();

    let view = view.downgrade();
    cx.subscribe(&details, move |_, event, cx| match *event {
        ReminderDetailsEvent::Close => {
            let Some(view) = view.upgrade() else {
                return;
            };
            handle
                .update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| view.close_details(window, cx));
                })
                .ok();
        }
        ReminderDetailsEvent::SetDone { task_id, done } => set_done(task_id, done, cx),
        ReminderDetailsEvent::Edit { task_id } => crate::edit_task_window::open(task_id, cx),
        ReminderDetailsEvent::OpenInBrowser { task_id } => {
            task_window::open_in_browser(&server, task_id, cx);
        }
        ReminderDetailsEvent::OpenInWindow { task_id } => {
            let reminder = shown
                .borrow()
                .clone()
                .filter(|reminder| reminder.task_id == task_id);
            if let Some(reminder) = reminder {
                open_task(&reminder, cx);
            }
        }
    })
    .detach();
}

/// What a save updates when it finishes.
#[derive(Clone)]
struct SaveTargets {
    view: WeakEntity<RemindersView>,
    details: WeakEntity<ReminderDetailsView>,
    tasks: Tasks,
    shown: Rc<RefCell<Option<Reminder>>>,
}

/// Mark `task_id` done (or not) on the server, then update the list and the panel.
fn save_done(
    targets: SaveTargets,
    session: Arc<Session>,
    on_session_expired: OnSessionExpired,
    task_id: i64,
    done: bool,
    cx: &mut App,
) {
    let save = cx.background_spawn(async move {
        session.call(|client, token| client.set_task_done(token, task_id, done))
    });
    cx.spawn(async move |cx| {
        let result = save.await;
        cx.update(|cx| match result {
            Ok(task) => targets.saved(task, cx),
            Err(vikunja::Error::Unauthorized) => on_session_expired(cx),
            Err(err) => {
                log::warn!("failed to update task {task_id}: {err:#}");
                task_window::failed(task_id, &err.to_string(), cx);
                if let Some(details) = targets.details.upgrade() {
                    details.update(cx, |details, cx| {
                        if details.task_id() == Some(task_id) {
                            details.set_error(err.to_string(), cx);
                        }
                    });
                }
            }
        });
    })
    .detach();
}

impl SaveTargets {
    fn saved(&self, task: Task, cx: &mut App) {
        let (id, done) = (task.id, task.done);
        if let Some(view) = self.view.upgrade() {
            view.update(cx, |view, cx| view.set_task_done(id, done, cx));
        }
        let mut shown = self.shown.borrow_mut();
        if let (Some(reminder), Some(details)) = (
            shown.as_mut().filter(|reminder| reminder.task_id == id),
            self.details.upgrade(),
        ) {
            reminder.task_done = done;
            let info = self.tasks.borrow().details(reminder, &task);
            details.update(cx, |details, cx| {
                if details.task_id() == Some(id) {
                    details.set_details(&info, cx);
                }
            });
        }
        drop(shown);
        task_window::saved(
            &task,
            |reminder, task| self.tasks.borrow().details(reminder, task),
            cx,
        );
        self.tasks.borrow_mut().tasks.insert(id, task);
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use login::Credentials;
    use vikunja::{ServerUrl, Token};

    use super::*;

    fn project(id: i64, title: &str) -> Source {
        Source::Project {
            id,
            title: title.into(),
            subproject_ids: Vec::new(),
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

    /// The open windows, and the source the reminders window lists.
    fn state(cx: &mut TestAppContext) -> (Vec<AnyWindowHandle>, Option<Source>) {
        cx.update(|cx| {
            let source = cx
                .try_global::<RemindersWindow>()
                .and_then(|open| open.browse.as_ref())
                .map(Opened::source);
            (cx.windows(), source)
        })
    }

    #[gpui_kit::test]
    fn keeps_one_reminders_window_open(cx: &mut TestAppContext) {
        let session = session();
        let open_source = |source: Source, cx: &mut TestAppContext| {
            let session = Arc::clone(&session);
            cx.update(|cx| open(&source, session, |_| {}, cx));
        };
        cx.update(|cx| {
            gpui_kit::init(cx);
            reminders_ui::init(cx);
        });

        open_source(project(1, "Home"), cx);
        let (first, _) = state(cx);
        open_source(project(1, "Home"), cx);
        assert_eq!(
            state(cx),
            (first.clone(), Some(project(1, "Home"))),
            "reopening reuses it"
        );

        open_source(project(2, "Work"), cx);
        let (windows, source) = state(cx);
        assert_eq!(windows.len(), 1, "another project replaces the window");
        assert_ne!(windows, first);
        assert_eq!(source, Some(project(2, "Work")));
    }

    #[gpui_kit::test]
    fn keeps_one_search_window_beside_the_reminders_window(cx: &mut TestAppContext) {
        let session = session();
        cx.update(|cx| {
            gpui_kit::init(cx);
            reminders_ui::init(cx);
            open(&project(1, "Home"), Arc::clone(&session), |_| {}, cx);
            open_search("rent", Arc::clone(&session), |_| {}, cx);
        });
        let search_source = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                cx.try_global::<RemindersWindow>()
                    .and_then(|open| open.search.as_ref())
                    .map(Opened::source)
            })
        };
        let (windows, source) = state(cx);
        assert_eq!(windows.len(), 2, "a search doesn't replace the project");
        assert_eq!(source, Some(project(1, "Home")));
        assert_eq!(search_source(cx), Some(Source::Search("rent".into())));

        cx.update(|cx| open_search("car", Arc::clone(&session), |_| {}, cx));
        let (again, _) = state(cx);
        assert_eq!(again, windows, "searching again reuses the search window");
        assert_eq!(search_source(cx), Some(Source::Search("car".into())));
    }
}
