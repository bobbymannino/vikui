//! Opens the window to create and edit saved filters (one at a time; opening it again
//! brings it to the front), loads the filters into it, and creates or saves what it
//! reports.
//!
//! Vikunja lists saved filters among the projects; each one's query comes from its own
//! request. After a save the main window's projects (and so the Projects menu) and the
//! open reminders windows reload, so they show the new name and the query's tasks.

use filter_editor_ui::{FilterEditorEvent, FilterEditorView};
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Global, WeakEntity};
use vikunja::{Client, FilterDraft, ProjectKind, SavedFilter, Token};

use crate::app_session::{self, SignedIn};

#[derive(Default)]
struct FilterEditorWindow(Option<(AnyWindowHandle, WeakEntity<FilterEditorView>)>);

impl Global for FilterEditorWindow {}

/// Open the window, or bring it to the front. Does nothing while signed out.
pub fn open(cx: &mut App) {
    if let Some((handle, view)) = cx.default_global::<FilterEditorWindow>().0.clone()
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
        crate::app_windows::WindowKind::FilterEditor,
        "Edit Saved Filters",
        720.,
        560.,
        cx,
    );
    let (handle, view) = match filter_editor_ui::open(options, cx) {
        Ok(opened) => opened,
        Err(err) => return log::error!("failed to open the filter editor: {err:#}"),
    };
    crate::app_windows::track(crate::app_windows::WindowKind::FilterEditor, handle, cx);
    cx.default_global::<FilterEditorWindow>().0 = Some((handle, view.downgrade()));
    load(handle, view.downgrade(), signed_in.clone(), cx);
    cx.subscribe(&view, move |view, event, cx| {
        let (filter_id, draft) = match event {
            FilterEditorEvent::Create(draft) => (None, draft.clone()),
            FilterEditorEvent::Update { filter_id, draft } => (Some(*filter_id), draft.clone()),
        };
        save(
            handle,
            view.downgrade(),
            filter_id,
            draft,
            signed_in.clone(),
            cx,
        );
    })
    .detach();
}

/// Open the window on `filter_id`'s saved filter, or bring it to the front showing it. Does
/// nothing while signed out.
pub fn edit(filter_id: i64, cx: &mut App) {
    open(cx);
    let Some((handle, view)) = cx
        .try_global::<FilterEditorWindow>()
        .and_then(|open| open.0.clone())
    else {
        return;
    };
    let Some(view) = view.upgrade() else {
        return;
    };
    handle
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| view.show_filter(filter_id, window, cx));
        })
        .ok();
}

fn load(
    handle: AnyWindowHandle,
    view: WeakEntity<FilterEditorView>,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let session = signed_in.session;
    let load = cx.background_spawn(async move { session.call(load_filters) });
    cx.spawn(async move |cx| {
        let result = load.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok(filters) => {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |view, cx| view.set_filters(filters, window, cx));
                        })
                        .ok();
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to load saved filters to edit: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_load_error(format!("Couldn't load the saved filters: {err}"), cx);
                    });
                }
            }
        });
    })
    .detach();
}

/// Every saved filter with its query: Vikunja lists them among the projects, without
/// their queries.
fn load_filters(client: &Client, token: &Token) -> Result<Vec<SavedFilter>, vikunja::Error> {
    client
        .projects(token)?
        .iter()
        .filter_map(|project| match project.kind() {
            ProjectKind::SavedFilter { filter_id } => Some(filter_id),
            ProjectKind::Project | ProjectKind::Favorites => None,
        })
        .map(|filter_id| client.saved_filter(token, filter_id))
        .collect()
}

/// Create a saved filter (`filter_id` is `None`) or save over one.
fn save(
    handle: AnyWindowHandle,
    view: WeakEntity<FilterEditorView>,
    filter_id: Option<i64>,
    draft: FilterDraft,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let session = signed_in.session;
    let save = cx.background_spawn(async move {
        session.call(|client, token| match filter_id {
            None => client.create_saved_filter(token, &draft),
            Some(filter_id) => client.update_saved_filter(token, filter_id, &draft),
        })
    });
    cx.spawn(async move |cx| {
        let result = save.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok(filter) => {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |view, cx| view.saved(filter, window, cx));
                        })
                        .ok();
                    crate::reload_projects(cx);
                    crate::reminders_window::reload(cx);
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to save saved filter {filter_id:?}: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_error(format!("Couldn't save the filter: {err}"), cx);
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
            filter_editor_ui::init(cx);
            open(cx);
            assert!(cx.windows().is_empty(), "signed out: no filters to edit");

            app_session::set(session(), Rc::new(|_| {}), cx);
            open(cx);
            open(cx);
            assert_eq!(cx.windows().len(), 1, "opening it again reuses it");
        });
    }
}
