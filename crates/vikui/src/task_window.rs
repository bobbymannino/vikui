//! A window showing one task, opened from a reminders window by double clicking a row,
//! pressing Cmd/Ctrl+Enter on it, its context menu, or the details panel's "Open in new
//! window" button. Also opens a task's page in the web browser.
//!
//! Each task gets at most one window; opening it again brings that window to the front.
//! The window asks its reminders window to save changes, and is updated when they land.

use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Global, WeakEntity};
use reminder_details::ReminderDetails;
use reminder_details_ui::{ReminderDetailsEvent, ReminderDetailsView};
use reminders::Reminder;
use vikunja::{ServerUrl, Task};

/// Saves a task done (or not done): `(task_id, done)`.
pub type SetDone = Rc<dyn Fn(i64, bool, &mut App)>;

struct Opened {
    handle: AnyWindowHandle,
    view: WeakEntity<ReminderDetailsView>,
    /// The reminder the window was opened from, to rebuild its details after a save.
    reminder: Reminder,
}

/// Open task windows by task id.
#[derive(Default)]
struct TaskWindows(HashMap<i64, Opened>);

impl Global for TaskWindows {}

/// Show `details` (of `reminder` and its task) in a window of its own, or bring the
/// task's window to the front if it is already open. `set_done` saves the task; `server`
/// hosts its web page.
pub fn open(
    reminder: &Reminder,
    details: &ReminderDetails,
    server: ServerUrl,
    set_done: SetDone,
    cx: &mut App,
) {
    let task_id = reminder.task_id;
    let windows = &mut cx.default_global::<TaskWindows>().0;
    windows.retain(|_, opened| opened.view.upgrade().is_some());
    if let Some(handle) = windows.get(&task_id).map(|opened| opened.handle)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }

    let options = crate::window_options(reminder.task_title.clone(), 440., 600., cx);
    let opened = gpui_kit::open_window(options, cx, |_, cx| {
        cx.new(|cx| {
            let mut view = ReminderDetailsView::new_window();
            view.set_details(details, cx);
            view
        })
    });
    let (handle, view) = match opened {
        Ok(opened) => opened,
        Err(err) => return log::error!("failed to open task window: {err:#}"),
    };
    cx.subscribe(&view, move |_, event, cx| match *event {
        ReminderDetailsEvent::SetDone { task_id, done } => set_done(task_id, done, cx),
        ReminderDetailsEvent::OpenInBrowser { task_id } => open_in_browser(&server, task_id, cx),
        ReminderDetailsEvent::Close | ReminderDetailsEvent::OpenInWindow { .. } => {}
    })
    .detach();
    cx.default_global::<TaskWindows>().0.insert(
        task_id,
        Opened {
            handle,
            view: view.downgrade(),
            reminder: reminder.clone(),
        },
    );
}

/// Open `task_id`'s page on `server` in the default web browser.
pub fn open_in_browser(server: &ServerUrl, task_id: i64, cx: &App) {
    cx.open_url(server.task_page(task_id).as_str());
}

/// Show `task` as saved in its window, if one is open. `details` builds the window's
/// details from its reminder and the saved task.
pub fn saved(task: &Task, details: impl FnOnce(&Reminder, &Task) -> ReminderDetails, cx: &mut App) {
    let Some(opened) = cx
        .try_global::<TaskWindows>()
        .and_then(|windows| windows.0.get(&task.id))
    else {
        return;
    };
    let Some(view) = opened.view.upgrade() else {
        return;
    };
    let reminder = Reminder {
        task_done: task.done,
        ..opened.reminder.clone()
    };
    let info = details(&reminder, task);
    view.update(cx, |view, cx| view.set_details(&info, cx));
    if let Some(opened) = cx.default_global::<TaskWindows>().0.get_mut(&task.id) {
        opened.reminder = reminder;
    }
}

/// Show why saving `task_id` failed in its window, if one is open.
pub fn failed(task_id: i64, message: &str, cx: &mut App) {
    let Some(view) = cx
        .try_global::<TaskWindows>()
        .and_then(|windows| windows.0.get(&task_id))
        .and_then(|opened| opened.view.upgrade())
    else {
        return;
    };
    let message = message.to_owned();
    view.update(cx, |view, cx| view.set_error(message, cx));
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    use super::*;

    fn task(done: bool) -> Task {
        Task {
            id: 7,
            title: "Pay rent".into(),
            done,
            ..Task::default()
        }
    }

    fn details(reminder: &Reminder, task: &Task) -> ReminderDetails {
        ReminderDetails::new(reminder, task, Some("Home"))
    }

    #[gpui_kit::test]
    fn opens_one_window_per_task_and_follows_saves(cx: &mut TestAppContext) {
        let reminder = reminders::from_tasks(&[task(false)]).remove(0);
        let saves = Rc::new(RefCell::new(Vec::new()));
        let set_done: SetDone = {
            let saves = Rc::clone(&saves);
            Rc::new(move |task_id, done, _| saves.borrow_mut().push((task_id, done)))
        };
        cx.update(|cx| {
            gpui_kit::init(cx);
            let info = details(&reminder, &task(false));
            let server = ServerUrl::parse("https://tasks.example.com").unwrap();
            open(&reminder, &info, server.clone(), Rc::clone(&set_done), cx);
            open(&reminder, &info, server, Rc::clone(&set_done), cx);
        });
        let handle = cx.update(|cx| {
            assert_eq!(cx.windows().len(), 1, "opening again reuses the window");
            cx.windows()[0]
        });
        let has_label = |cx: &mut TestAppContext, label: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(label).is_some()
            })
            .unwrap()
        };
        assert!(has_label(cx, "Task details"));
        assert!(has_label(cx, "Pay rent"));
        assert!(has_label(cx, "Project: Home"));

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("toggle-task-done", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(*saves.borrow(), [(7, true)]);

        cx.update_window(handle, |_, window, cx| {
            window.click("open-task-in-browser", cx);
        })
        .unwrap();
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://tasks.example.com/tasks/7")
        );

        cx.update(|cx| saved(&task(true), details, cx));
        assert!(has_label(cx, "Status: Done"));
        assert!(has_label(cx, "Mark as not done"));

        cx.update(|cx| failed(7, "Server unavailable", cx));
        assert!(has_label(
            cx,
            "Couldn't update the task: Server unavailable"
        ));
    }
}
