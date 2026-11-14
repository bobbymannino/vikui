//! Opens a reminders window for a project, filter or search and fills it in.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _};
use login::Session;
use reminders::Source;

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
    let options = crate::window_options(title.clone(), 900., 560., cx);
    let view = match reminders_ui::open(options, title, None, cx) {
        Ok((_, view)) => view,
        Err(err) => {
            log::error!("failed to open reminders window: {err:#}");
            return;
        }
    };

    let query = source.query();
    let load = cx.background_spawn(async move {
        session
            .call(|client, token| client.tasks(token, &query))
            .map(|tasks| reminders::from_tasks(&tasks))
    });
    cx.spawn(async move |cx| {
        let result = load.await;
        cx.update(|cx| match result {
            Ok(reminders) => view.update(cx, |view, cx| view.set_reminders(&reminders, cx)),
            Err(vikunja::Error::Unauthorized) => on_session_expired(cx),
            Err(err) => {
                log::warn!("failed to load reminders: {err:#}");
                view.update(cx, |view, cx| view.set_error(err.to_string(), cx));
            }
        });
    })
    .detach();
}
