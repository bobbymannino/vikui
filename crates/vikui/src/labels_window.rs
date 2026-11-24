//! Opens the labels window (one at a time; opening it again brings it to the front),
//! loads every label into it, and creates or saves the labels it reports.
//!
//! After a save, open reminders windows reload so they show the new name and colour.

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Global, WeakEntity};
use labels_ui::{LabelsEvent, LabelsView};
use vikunja::LabelDraft;

use crate::app_session::{self, SignedIn};

#[derive(Default)]
struct LabelsWindow(Option<(AnyWindowHandle, WeakEntity<LabelsView>)>);

impl Global for LabelsWindow {}

/// Open the labels window, or bring it to the front. Does nothing while signed out.
pub fn open(cx: &mut App) {
    if let Some((handle, view)) = cx.default_global::<LabelsWindow>().0.clone()
        && view.upgrade().is_some()
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let Some(signed_in) = app_session::get(cx) else {
        return;
    };
    let options = crate::app_windows::options(
        crate::app_windows::WindowKind::Labels,
        "Labels",
        640.,
        480.,
        cx,
    );
    let (handle, view) = match labels_ui::open(options, cx) {
        Ok(opened) => opened,
        Err(err) => return log::error!("failed to open labels window: {err:#}"),
    };
    crate::app_windows::track(crate::app_windows::WindowKind::Labels, handle, cx);
    cx.default_global::<LabelsWindow>().0 = Some((handle, view.downgrade()));
    load(handle, view.downgrade(), signed_in.clone(), cx);
    cx.subscribe(&view, move |view, event, cx| {
        let (label_id, draft) = match event {
            LabelsEvent::Create(draft) => (None, draft.clone()),
            LabelsEvent::Update { label_id, draft } => (Some(*label_id), draft.clone()),
        };
        save(
            handle,
            view.downgrade(),
            label_id,
            draft,
            signed_in.clone(),
            cx,
        );
    })
    .detach();
}

fn load(handle: AnyWindowHandle, view: WeakEntity<LabelsView>, signed_in: SignedIn, cx: &mut App) {
    let session = signed_in.session;
    let load = cx.background_spawn(async move { session.call(vikunja::Client::labels) });
    cx.spawn(async move |cx| {
        let result = load.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok(labels) => {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |view, cx| view.set_labels(labels, window, cx));
                        })
                        .ok();
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to load labels: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_load_error(format!("Couldn't load the labels: {err}"), cx);
                    });
                }
            }
        });
    })
    .detach();
}

/// Create a label (`label_id` is `None`) or save over one.
fn save(
    handle: AnyWindowHandle,
    view: WeakEntity<LabelsView>,
    label_id: Option<i64>,
    draft: LabelDraft,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let session = signed_in.session;
    let save = cx.background_spawn(async move {
        session.call(|client, token| match label_id {
            None => client.create_label(token, &draft),
            Some(label_id) => client.update_label(token, label_id, &draft),
        })
    });
    cx.spawn(async move |cx| {
        let result = save.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok(label) => {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |view, cx| view.saved(label, window, cx));
                        })
                        .ok();
                    crate::reminders_window::reload(cx);
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to save label {label_id:?}: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_error(format!("Couldn't save the label: {err}"), cx);
                    });
                }
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
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
    fn opens_one_window_and_only_when_signed_in(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            labels_ui::init(cx);
            open(cx);
            assert!(cx.windows().is_empty(), "signed out: no labels to show");

            app_session::set(session(), Rc::new(|_| {}), cx);
            open(cx);
            open(cx);
            assert_eq!(cx.windows().len(), 1, "opening it again reuses it");
        });
    }
}
