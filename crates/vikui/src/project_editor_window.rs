//! Opens the window to create and edit projects (one at a time; opening it again brings
//! it to the front), loads the projects into it, and creates or saves what it reports.
//!
//! After a save the main window's projects (and so the Projects menu) and the open
//! reminders windows reload, so they show the new name, colour or place in the tree.

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Global, WeakEntity};
use project_editor_ui::{ProjectEditorEvent, ProjectEditorView};
use vikunja::ProjectDraft;

use crate::app_session::{self, SignedIn};

#[derive(Default)]
struct ProjectEditorWindow(Option<(AnyWindowHandle, WeakEntity<ProjectEditorView>)>);

impl Global for ProjectEditorWindow {}

/// Open the window, or bring it to the front. Does nothing while signed out.
pub fn open(cx: &mut App) {
    if let Some((handle, view)) = cx.default_global::<ProjectEditorWindow>().0.clone()
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
        crate::app_windows::WindowKind::ProjectEditor,
        "Edit Projects",
        720.,
        560.,
        cx,
    );
    let (handle, view) = match project_editor_ui::open(options, cx) {
        Ok(opened) => opened,
        Err(err) => return log::error!("failed to open the project editor: {err:#}"),
    };
    crate::app_windows::track(crate::app_windows::WindowKind::ProjectEditor, handle, cx);
    cx.default_global::<ProjectEditorWindow>().0 = Some((handle, view.downgrade()));
    load(handle, view.downgrade(), signed_in.clone(), cx);
    cx.subscribe(&view, move |view, event, cx| {
        let (project_id, draft) = match event {
            ProjectEditorEvent::Create(draft) => (None, draft.clone()),
            ProjectEditorEvent::Update { project_id, draft } => (Some(*project_id), draft.clone()),
        };
        save(
            handle,
            view.downgrade(),
            project_id,
            draft,
            signed_in.clone(),
            cx,
        );
    })
    .detach();
}

/// Open the window on `project_id`'s project, or bring it to the front showing it. Does
/// nothing while signed out.
pub fn edit(project_id: i64, cx: &mut App) {
    open(cx);
    let Some((handle, view)) = cx
        .try_global::<ProjectEditorWindow>()
        .and_then(|open| open.0.clone())
    else {
        return;
    };
    let Some(view) = view.upgrade() else {
        return;
    };
    handle
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| view.show_project(project_id, window, cx));
        })
        .ok();
}

fn load(
    handle: AnyWindowHandle,
    view: WeakEntity<ProjectEditorView>,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let session = signed_in.session;
    let load = cx.background_spawn(async move { session.call(vikunja::Client::projects) });
    cx.spawn(async move |cx| {
        let result = load.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok(projects) => {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |view, cx| view.set_projects(projects, window, cx));
                        })
                        .ok();
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to load projects to edit: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_load_error(format!("Couldn't load the projects: {err}"), cx);
                    });
                }
            }
        });
    })
    .detach();
}

/// Create a project (`project_id` is `None`) or save over one.
fn save(
    handle: AnyWindowHandle,
    view: WeakEntity<ProjectEditorView>,
    project_id: Option<i64>,
    draft: ProjectDraft,
    signed_in: SignedIn,
    cx: &mut App,
) {
    let session = signed_in.session;
    let save = cx.background_spawn(async move {
        session.call(|client, token| match project_id {
            None => client.create_project(token, &draft),
            Some(project_id) => client.update_project(token, project_id, &draft),
        })
    });
    cx.spawn(async move |cx| {
        let result = save.await;
        cx.update(|cx| {
            let Some(view) = view.upgrade() else {
                return;
            };
            match result {
                Ok(project) => {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |view, cx| view.saved(project, window, cx));
                        })
                        .ok();
                    crate::reload_projects(cx);
                    crate::reminders_window::reload(cx);
                }
                Err(vikunja::Error::Unauthorized) => (signed_in.on_session_expired)(cx),
                Err(err) => {
                    log::warn!("failed to save project {project_id:?}: {err:#}");
                    view.update(cx, |view, cx| {
                        view.set_error(format!("Couldn't save the project: {err}"), cx);
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
            project_editor_ui::init(cx);
            open(cx);
            assert!(cx.windows().is_empty(), "signed out: no projects to edit");

            app_session::set(session(), Rc::new(|_| {}), cx);
            open(cx);
            open(cx);
            assert_eq!(cx.windows().len(), 1, "opening it again reuses it");
        });
    }
}
