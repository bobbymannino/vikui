//! The user's preferences while the app runs, loaded at startup and saved on change.
//!
//! Disk access runs in the background. Saves take a lock and write whatever is current at
//! that moment, so quick successive changes can't leave an older value on disk. Changes
//! that come in bursts, such as a window being dragged, can wait and save once
//! ([`update_soon`]); anything still waiting is saved when the app quits.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Global, Task};
use preferences::Preferences;

/// How long [`update_soon`] waits for the burst of changes to end before saving.
const SAVE_DELAY: Duration = Duration::from_millis(750);

/// The current preferences, shared with background saves.
#[derive(Clone, Default)]
struct Current(Arc<Mutex<Preferences>>);

impl Global for Current {}

/// A save [`update_soon`] scheduled and hasn't made yet.
#[derive(Default)]
struct PendingSave(Option<Task<()>>);

impl Global for PendingSave {}

/// Load the saved preferences, then run `loaded` with them. Call once at startup; until
/// they load, [`get`] returns the defaults.
pub fn load(cx: &mut App, loaded: impl FnOnce(&Preferences, &mut App) + 'static) {
    cx.set_global(Current::default());
    cx.on_app_quit(|cx| {
        if cx.default_global::<PendingSave>().0.take().is_some() {
            // Quitting waits only briefly for its handlers, so write now rather than in
            // the background; the file is small.
            save_now(&Arc::clone(&cx.default_global::<Current>().0));
        }
        async {}
    })
    .detach();
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
    // This save includes any waiting change.
    cx.default_global::<PendingSave>().0 = None;
    cx.background_spawn(async move { save_now(&current) })
        .detach();
}

/// Change the preferences now and save them once changes stop coming for a moment.
pub fn update_soon(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    let current = Arc::clone(&cx.default_global::<Current>().0);
    change(&mut lock(&current));
    // Replacing the waiting save cancels it, so the delay restarts with each change.
    let save = cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DELAY).await;
        cx.update(|cx| {
            cx.default_global::<PendingSave>().0 = None;
            cx.background_spawn(async move { save_now(&current) })
                .detach();
        });
    });
    cx.default_global::<PendingSave>().0 = Some(save);
}

fn save_now(current: &Mutex<Preferences>) {
    // Tests change preferences too; keep them away from the real file.
    if cfg!(test) {
        return;
    }
    if let Err(err) = lock(current).save() {
        log::warn!("couldn't save preferences: {err}");
    }
}

/// The preferences are plain data, so a panic mid-change can't leave them inconsistent.
fn lock(preferences: &Mutex<Preferences>) -> std::sync::MutexGuard<'_, Preferences> {
    preferences.lock().unwrap_or_else(PoisonError::into_inner)
}
