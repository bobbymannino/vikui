//! App-wide actions and their key bindings: quitting, closing and arranging windows,
//! refreshing whatever the focused window shows and opening settings.
//!
//! These bindings have no context, so a view's own binding for the same keys wins.

use gpui_kit::{Action, App, KeyBinding, Window};

gpui_kit::actions!(
    vikui,
    [
        Quit,
        CloseWindow,
        Minimize,
        Zoom,
        Hide,
        HideOthers,
        ShowAll,
        Refresh,
        OpenSettings,
    ]
);

/// Key bindings for the app-wide actions; `secondary-` is Cmd on macOS, Ctrl elsewhere.
fn bindings() -> Vec<(&'static str, Box<dyn Action>)> {
    fn bind(keystrokes: &'static str, action: impl Action) -> (&'static str, Box<dyn Action>) {
        (keystrokes, Box::new(action))
    }
    vec![
        bind("secondary-q", Quit),
        bind("secondary-w", CloseWindow),
        bind("secondary-r", Refresh),
        bind("secondary-,", OpenSettings),
        // Minimizing and hiding are macOS conventions; elsewhere the window manager owns
        // those shortcuts.
        #[cfg(target_os = "macos")]
        bind("secondary-m", Minimize),
        #[cfg(target_os = "macos")]
        bind("secondary-h", Hide),
        #[cfg(target_os = "macos")]
        bind("alt-secondary-h", HideOthers),
    ]
}

/// Register the app-wide actions and their key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    for (keystrokes, action) in bindings() {
        // `KeyBinding::new` only accepts explicit modifiers; `load` resolves `secondary-`.
        match KeyBinding::load(keystrokes, action, None, false, None, cx) {
            Ok(binding) => cx.bind_keys([binding]),
            Err(err) => log::error!("invalid key binding {keystrokes}: {err}"),
        }
    }

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &CloseWindow, cx| with_active_window(cx, |window, _| window.remove_window()));
    cx.on_action(|_: &Minimize, cx| with_active_window(cx, |window, _| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| with_active_window(cx, |window, _| window.zoom_window()));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &OpenSettings, cx| crate::settings_window::open(cx));
    cx.on_action(|_: &Refresh, cx| {
        // Each list view has its own refresh action; only the focused one handles it.
        cx.dispatch_action(&projects_ui::Refresh);
        cx.dispatch_action(&reminders_ui::Refresh);
    });
}

fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App)) {
    if let Some(handle) = cx.active_window() {
        handle.update(cx, |_, window, cx| f(window, cx)).ok();
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn every_binding_is_valid(cx: &mut TestAppContext) {
        cx.update(|cx| {
            for (keystrokes, action) in bindings() {
                assert!(
                    KeyBinding::load(keystrokes, action, None, false, None, cx).is_ok(),
                    "{keystrokes}"
                );
            }
        });
    }
}
