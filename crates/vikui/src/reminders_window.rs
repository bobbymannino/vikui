//! Opens a reminders window for a project, filter or search and fills it in.
//!
//! The window's details side panel is a [`ReminderDetailsView`], filled from the tasks
//! loaded for the list. Marking a task done from the panel or the list saves it here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, WeakEntity};
use login::Session;
use reminder_details::ReminderDetails;
use reminder_details_ui::{ReminderDetailsEvent, ReminderDetailsView};
use reminders::Reminder;
use reminders::Source;
use reminders_ui::{RemindersEvent, RemindersView};
use vikunja::{Client, Task};

/// Open a window listing the reminders from `source`, loading them in the background.
///
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

/// Runs when the server rejects the session for good.
type OnSessionExpired = Rc<dyn Fn(&mut App)>;

/// `on_session_expired` runs if the server rejects the session for good.
pub fn open(
    source: &Source,
    session: Arc<Session>,
    on_session_expired: impl Fn(&mut App) + 'static,
    cx: &mut App,
) {
    let on_session_expired: OnSessionExpired = Rc::new(on_session_expired);
    let title = source.title();
    let options = crate::window_options(title.clone(), 1200., 560., cx);
    let (handle, view) = match reminders_ui::open(options, title, None, cx) {
        Ok(opened) => opened,
        Err(err) => {
            log::error!("failed to open reminders window: {err:#}");
            return;
        }
    };
    let tasks: Tasks = Rc::default();
    attach_details_panel(
        handle,
        &view,
        Rc::clone(&tasks),
        Arc::clone(&session),
        Rc::clone(&on_session_expired),
        cx,
    );

    let load = Rc::new(Loader {
        source: source.clone(),
        session,
        view: view.downgrade(),
        tasks,
        on_session_expired,
    });
    load.run(cx);
    cx.subscribe(&view, move |_, event, cx| {
        if *event == RemindersEvent::Refresh {
            load.run(cx);
        }
    })
    .detach();
}

/// One finished load: the list's reminders, their tasks and project titles by id.
struct Load {
    reminders: Vec<Reminder>,
    tasks: Vec<Task>,
    projects: HashMap<i64, String>,
}

/// Loads a window's tasks into its list, on open and on every refresh.
struct Loader {
    source: Source,
    session: Arc<Session>,
    view: WeakEntity<RemindersView>,
    tasks: Tasks,
    on_session_expired: OnSessionExpired,
}

impl Loader {
    fn run(self: &Rc<Self>, cx: &mut App) {
        let source = self.source.clone();
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
/// it is marked done, and close it on request.
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

    cx.subscribe(view, {
        let details = details.clone();
        let tasks = Rc::clone(&tasks);
        let shown = Rc::clone(&shown);
        move |_, event, cx| {
            let RemindersEvent::ShowDetails(reminder) = event else {
                return;
            };
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
    })
    .detach();

    // Marking done from the list (keyboard) saves the same way as from the panel.
    cx.subscribe(view, {
        let targets = SaveTargets {
            view: view.downgrade(),
            details: details.downgrade(),
            tasks: Rc::clone(&tasks),
            shown: Rc::clone(&shown),
        };
        let session = Arc::clone(&session);
        let on_session_expired = Rc::clone(&on_session_expired);
        move |_, event, cx| {
            if let RemindersEvent::SetDone { task_id, done } = *event {
                save_done(
                    targets.clone(),
                    Arc::clone(&session),
                    Rc::clone(&on_session_expired),
                    task_id,
                    done,
                    cx,
                );
            }
        }
    })
    .detach();

    let view = view.downgrade();
    let weak_details = details.downgrade();
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
        ReminderDetailsEvent::SetDone { task_id, done } => {
            let targets = SaveTargets {
                view: view.clone(),
                details: weak_details.clone(),
                tasks: Rc::clone(&tasks),
                shown: Rc::clone(&shown),
            };
            save_done(
                targets,
                Arc::clone(&session),
                Rc::clone(&on_session_expired),
                task_id,
                done,
                cx,
            );
        }
        ReminderDetailsEvent::OpenInWindow { .. } => {}
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
        self.tasks.borrow_mut().tasks.insert(id, task);
    }
}
