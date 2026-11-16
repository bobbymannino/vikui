//! The signed-in session, for app-wide actions that need the server (such as adding a
//! reminder from the menu bar). Unset while signed out.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{App, Global};
use login::Session;

/// Runs when the server rejects the session for good.
pub type OnSessionExpired = Rc<dyn Fn(&mut App)>;

#[derive(Clone)]
pub struct SignedIn {
    pub session: Arc<Session>,
    pub on_session_expired: OnSessionExpired,
}

#[derive(Default)]
struct Current(Option<SignedIn>);

impl Global for Current {}

pub fn set(session: Arc<Session>, on_session_expired: OnSessionExpired, cx: &mut App) {
    cx.set_global(Current(Some(SignedIn {
        session,
        on_session_expired,
    })));
}

pub fn clear(cx: &mut App) {
    cx.set_global(Current(None));
}

pub fn get(cx: &App) -> Option<SignedIn> {
    cx.try_global::<Current>()
        .and_then(|current| current.0.clone())
}
