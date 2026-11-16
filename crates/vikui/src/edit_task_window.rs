//! Opens a window to edit one task (one per task; opening it again brings it to the
//! front), fills it in from the server, and saves only what the user changed.
//!
//! On success the window closes, the open reminders window reloads and the task's own
//! window, if open, shows the saved task.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Global, WeakEntity};
use jiff::tz::TimeZone;
use new_reminder::{Draft, Edit, Options};
use new_reminder_ui::{NewReminderEvent, NewReminderView};
use vikunja::Task;

use crate::app_session::{self, SignedIn};

#[derive(Default)]
struct EditWindows(HashMap<i64, (AnyWindowHandle, WeakEntity<NewReminderView>)>);

impl Global for EditWindows {}

/// What a window needs once its task has loaded.
#[derive(Default)]
struct Loaded {
    edit: Option<Edit>,
    /// Project titles by id, to name the task's project after a save.
    projects: HashMap<i64, String>,
}

type Shared = Rc<RefCell<Loaded>>;

/// Open a window to edit `task_id`, or bring its window to the front. Does nothing
/// while signed out.
pub fn open(task_id: i64, cx: &mut App) {
    let windows = &mut cx.default_global::<EditWindows>().0;
    windows.retain(|_, (_, view)| view.upgrade().is_some());
    if let Some(handle) = windows.get(&task_id).map(|(handle, _)| *handle)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let Some(signed_in) = app_session::get(cx) else {
        return;
    };
    let options = crate::window_options("Edit Task", 520., 640., cx);
    let (handle, view) = match new_reminder_ui::open_edit(options, cx) {
        Ok(opened) => opened,
        Err(err) => return log::error!("failed to open edit task window: {err:#}"),
    };
    cx.default_global::<EditWindows>()
        .0
        .insert(task_id, (handle, view.downgrade()));
    let shared: Shared = Rc::default();
    load(
        task_id,
        handle,
        view.downgrade(),
        Rc::clone(&shared),
        signed_in.clone(),
        cx,
    );
    cx.subscribe(&view, move |view, event, cx| match event {
        NewReminderEvent::Cancel => close(handle, cx),
        NewReminderEvent::Save(draft) => {
            save(
                handle,
                view.downgrade(),
                draft,
                &shared,
                signed_in.clone(),
                cx,
            );
        }
        // Only a new reminder window creates.
        NewReminderEvent::Create(_) => {}
    })
    .detach();
}

fn load(
    task_id: i64,
    handle: AnyWindowHandle,
    view: WeakEntity<NewReminderView>,
    shared: Shared,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let session = signed_in.session;
    let load = cx.background_spawn(async move {
        session
            .call(|client, token| Ok((Options::load(client, token)?, client.task(token, task_id)?)))
    });
    cx.spawn(async move |cx| {
        let result = load.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok((options, task)) => {
                    let zone = TimeZone::system();
                    let edit = Edit::new(&task, &zone);
                    let draft = edit.original.clone();
                    *shared.borrow_mut() = Loaded {
                        edit: Some(edit),
                        projects: options
                            .projects
                            .iter()
                            .map(|project| (project.id, project.title.clone()))
                            .collect(),
                    };
                    handle
                        .update(cx, |_, window, cx| {
                            window.set_window_title(&format!("Edit – {}", task.title));
                            view.update(cx, |view, cx| {
                                view.set_options(&options, window, cx);
                                view.set_draft(&draft, window, cx);
                            });
                        })
                        .ok();
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to load task {task_id} to edit: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_load_error(format!("Couldn't load the task: {err}"), cx);
                    });
                }
            }
        });
    })
    .detach();
}

fn save(
    handle: AnyWindowHandle,
    view: WeakEntity<NewReminderView>,
    draft: &Draft,
    shared: &Shared,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let Some(edit) = shared.borrow().edit.clone() else {
        return;
    };
    let update = match edit.update(draft, &TimeZone::system()) {
        Ok(update) => update,
        Err(invalid) => return show_error(&view, invalid.message().to_owned(), cx),
    };
    if update.is_empty() {
        return close(handle, cx);
    }
    let session = signed_in.session;
    let task_id = edit.task_id;
    let added = update.add_label_ids.clone();
    let save = cx.background_spawn(async move {
        session.call(|client, token| new_reminder::save(client, token, task_id, &update))
    });
    let shared = Rc::clone(shared);
    cx.spawn(async move |cx| {
        let result = save.await;
        cx.update(|cx| match result {
            Ok(task) => {
                let missing = new_reminder::missing_labels(&added, &task);
                if !missing.is_empty() {
                    log::warn!("task {task_id} was saved without labels {missing:?}");
                }
                close(handle, cx);
                crate::reminders_window::reload(cx);
                show_in_task_window(&task, &shared, cx);
            }
            Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
            Err(err) => {
                log::warn!("failed to save task {task_id}: {err:#}");
                show_error(&view, format!("Couldn't save the task: {err}"), cx);
            }
        });
    })
    .detach();
}

fn show_error(view: &WeakEntity<NewReminderView>, message: String, cx: &mut App) {
    if let Some(view) = view.upgrade() {
        view.update(cx, |view, cx| view.set_error(message, cx));
    }
}

fn show_in_task_window(task: &Task, shared: &Shared, cx: &mut App) {
    let project = shared.borrow().projects.get(&task.project_id).cloned();
    crate::task_window::replace(task, project.as_deref(), cx);
}

/// Close the window. Deferred: the request may come from inside the window's own update.
fn close(handle: AnyWindowHandle, cx: &mut App) {
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::TestAppContext;
    use login::{Credentials, Session};
    use vikunja::{ServerUrl, Token};

    use super::*;

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

    #[gpui_kit::test]
    fn opens_one_window_per_task_and_only_when_signed_in(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            new_reminder_ui::init(cx);
            open(7, cx);
            assert!(cx.windows().is_empty(), "signed out: nothing to edit");

            app_session::set(session(), Rc::new(|_| {}), cx);
            open(7, cx);
            open(7, cx);
            assert_eq!(
                cx.windows().len(),
                1,
                "opening the same task again reuses it"
            );
            open(8, cx);
            assert_eq!(cx.windows().len(), 2, "each task gets its own window");
        });
    }
}
