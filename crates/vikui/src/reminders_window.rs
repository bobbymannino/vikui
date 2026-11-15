//! Opens a reminders window for a project, filter or search and fills it in.
//!
//! The window's details side panel is a [`ReminderDetailsView`], filled from the tasks
//! loaded for the list.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity};
use login::Session;
use reminder_details::ReminderDetails;
use reminder_details_ui::{ReminderDetailsEvent, ReminderDetailsView};
use reminders::Source;
use reminders_ui::{RemindersEvent, RemindersView};
use vikunja::Task;

/// Open a window listing the reminders from `source`, loading them in the background.
///
/// `on_session_expired` runs if the server rejects the session for good.
pub fn open(
    source: &Source,
    session: Arc<Session>,
    on_session_expired: impl FnOnce(&mut App) + 'static,
    cx: &mut App,
) {
    let title = source.title();
    let options = crate::window_options(title.clone(), 1200., 560., cx);
    let (handle, view) = match reminders_ui::open(options, title, None, cx) {
        Ok(opened) => opened,
        Err(err) => {
            log::error!("failed to open reminders window: {err:#}");
            return;
        }
    };
    let tasks: Rc<RefCell<HashMap<i64, Task>>> = Rc::default();
    attach_details_panel(handle, &view, Rc::clone(&tasks), cx);

    let source = source.clone();
    let load = cx.background_spawn(async move {
        session
            .call(|client, token| source.tasks(client, token))
            .map(|tasks| {
                let reminders = reminders::from_tasks(&tasks);
                log::debug!(
                    "{source:?}: {} tasks, {} reminders",
                    tasks.len(),
                    reminders.len()
                );
                (reminders, tasks)
            })
    });
    cx.spawn(async move |cx| {
        let result = load.await;
        cx.update(|cx| match result {
            Ok((reminders, loaded)) => {
                let count = loaded.len();
                *tasks.borrow_mut() = loaded.into_iter().map(|task| (task.id, task)).collect();
                view.update(cx, |view, cx| {
                    view.set_reminders(&reminders, Some(count), cx);
                });
            }
            Err(vikunja::Error::Unauthorized) => on_session_expired(cx),
            Err(err) => {
                log::warn!("failed to load reminders: {err:#}");
                view.update(cx, |view, cx| view.set_error(err.to_string(), cx));
            }
        });
    })
    .detach();
}

/// Give the window a details panel: fill it when a reminder is opened, close it on request.
fn attach_details_panel(
    handle: AnyWindowHandle,
    view: &Entity<RemindersView>,
    tasks: Rc<RefCell<HashMap<i64, Task>>>,
    cx: &mut App,
) {
    let details = cx.new(|_| ReminderDetailsView::new());
    view.update(cx, |view, cx| {
        view.set_details_panel(details.clone().into(), cx);
    });

    cx.subscribe(view, {
        let details = details.clone();
        move |_, RemindersEvent::ShowDetails(reminder), cx| {
            let Some(task) = tasks.borrow().get(&reminder.task_id).cloned() else {
                log::warn!("no loaded task {} for reminder details", reminder.task_id);
                return;
            };
            let info = ReminderDetails::new(reminder, &task);
            details.update(cx, |details, cx| details.set_details(&info, cx));
        }
    })
    .detach();

    let view = view.downgrade();
    cx.subscribe(&details, move |_, ReminderDetailsEvent::Close, cx| {
        let Some(view) = view.upgrade() else {
            return;
        };
        handle
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.close_details(window, cx));
            })
            .ok();
    })
    .detach();
}
