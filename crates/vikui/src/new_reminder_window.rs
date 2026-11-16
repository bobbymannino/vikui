//! Opens the new reminder window (one at a time), fills in the projects and labels it
//! offers, and creates the reminder it asks for.
//!
//! On success the window closes and the open reminders window reloads, so a reminder
//! that belongs in it shows up.

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global, WeakEntity};
use jiff::tz::TimeZone;
use new_reminder::{Draft, Options};
use new_reminder_ui::{NewReminderEvent, NewReminderView};

use crate::app_session::{self, SignedIn};

#[derive(Default)]
struct NewReminderWindow(Option<(AnyWindowHandle, WeakEntity<NewReminderView>)>);

impl Global for NewReminderWindow {}

/// Open the new reminder window, or bring it to the front if it's open. Does nothing
/// while signed out.
pub fn open(cx: &mut App) {
    if let Some((handle, _)) = cx
        .try_global::<NewReminderWindow>()
        .and_then(|open| open.0.clone())
        .filter(|(_, view)| view.upgrade().is_some())
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let Some(signed_in) = app_session::get(cx) else {
        return;
    };
    let zone = TimeZone::system();
    let options = crate::window_options("New Reminder", 520., 640., cx);
    let (handle, view) = match new_reminder_ui::open(options, new_reminder::now_in(&zone), cx) {
        Ok(opened) => opened,
        Err(err) => return log::error!("failed to open new reminder window: {err:#}"),
    };
    cx.set_global(NewReminderWindow(Some((handle, view.downgrade()))));
    load_options(handle, &view, signed_in.clone(), cx);
    cx.subscribe(&view, move |view, event, cx| match event {
        NewReminderEvent::Cancel => close(handle, cx),
        NewReminderEvent::Create(draft) => {
            create(handle, view.downgrade(), draft, signed_in.clone(), cx);
        }
    })
    .detach();
}

fn load_options(
    handle: AnyWindowHandle,
    view: &Entity<NewReminderView>,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let session = signed_in.session;
    let load = cx.background_spawn(async move { session.call(Options::load) });
    let view = view.downgrade();
    cx.spawn(async move |cx| {
        let result = load.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok(options) => {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |view, cx| view.set_options(&options, window, cx));
                        })
                        .ok();
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to load new reminder options: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_load_error(
                            format!("Couldn't load projects and labels: {err}"),
                            cx,
                        );
                    });
                }
            }
        });
    })
    .detach();
}

fn create(
    handle: AnyWindowHandle,
    view: WeakEntity<NewReminderView>,
    draft: &Draft,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let (project_id, task) = match draft.to_task(&TimeZone::system()) {
        Ok(task) => task,
        Err(invalid) => {
            if let Some(view) = view.upgrade() {
                view.update(cx, |view, cx| view.set_error(invalid.message(), cx));
            }
            return;
        }
    };
    let session = signed_in.session;
    let save = cx.background_spawn(async move {
        let created =
            session.call(|client, token| new_reminder::create(client, token, project_id, &task))?;
        Ok::<_, vikunja::Error>((created, task.label_ids))
    });
    cx.spawn(async move |cx| {
        let result = save.await;
        cx.update(|cx| match result {
            Ok((created, label_ids)) => {
                let missing = new_reminder::missing_labels(&label_ids, &created);
                if !missing.is_empty() {
                    log::warn!("task {} was created without labels {missing:?}", created.id);
                }
                close(handle, cx);
                crate::reminders_window::reload(cx);
            }
            Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
            Err(err) => {
                log::warn!("failed to create reminder: {err:#}");
                if let Some(view) = view.upgrade() {
                    view.update(cx, |view, cx| {
                        view.set_error(format!("Couldn't create the reminder: {err}"), cx);
                    });
                }
            }
        });
    })
    .detach();
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
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
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
    fn opens_one_window_and_only_when_signed_in(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            new_reminder_ui::init(cx);
            crate::app_actions::init(cx);
            open(cx);
            assert!(cx.windows().is_empty(), "signed out: nothing to add to");

            app_session::set(session(), Rc::new(|_| {}), cx);
            open(cx);
            open(cx);
            assert_eq!(cx.windows().len(), 1, "opening again reuses the window");
        });

        // Escape closes it.
        let handle = cx.update(|cx| cx.windows()[0]);
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            window.render_frame(cx);
            window.key_down("escape", false, cx);
        })
        .unwrap();
        assert!(cx.update(|cx| cx.windows().is_empty()));
    }
}
