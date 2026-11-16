//! The user's preferences while the app runs, loaded at startup and saved on change.
//!
//! Disk access runs in the background. Saves take a lock and write whatever is current at
//! that moment, so quick successive changes can't leave an older value on disk.

use std::sync::{Arc, Mutex, PoisonError};

use gpui_kit::{App, AppContext as _, Global};
use preferences::Preferences;

/// The current preferences, shared with background saves.
#[derive(Clone, Default)]
struct Current(Arc<Mutex<Preferences>>);

impl Global for Current {}

/// Load the saved preferences, then run `loaded` with them. Call once at startup; until
/// they load, [`get`] returns the defaults.
pub fn load(cx: &mut App, loaded: impl FnOnce(&Preferences, &mut App) + 'static) {
    cx.set_global(Current::default());
    let load = cx.background_spawn(async { Preferences::load() });
    cx.spawn(async move |cx| {
        let preferences = load.await;
        cx.update(|cx| {
            *lock(&cx.global::<Current>().0) = preferences.clone();
            loaded(&preferences, cx);
        });
    })
    .detach();
}

/// The current preferences.
pub fn get(cx: &App) -> Preferences {
    cx.try_global::<Current>()
        .map_or_else(Preferences::default, |current| lock(&current.0).clone())
}

/// Change the preferences and save them in the background.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    let current = Arc::clone(&cx.default_global::<Current>().0);
    change(&mut lock(&current));
    cx.background_spawn(async move {
        let preferences = lock(&current);
        if let Err(err) = preferences.save() {
            log::warn!("couldn't save preferences: {err}");
        }
    })
    .detach();
}

/// The preferences are plain data, so a panic mid-change can't leave them inconsistent.
fn lock(preferences: &Mutex<Preferences>) -> std::sync::MutexGuard<'_, Preferences> {
    preferences.lock().unwrap_or_else(PoisonError::into_inner)
}
