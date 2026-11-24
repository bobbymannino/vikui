//! App-wide actions and their key bindings: quitting, closing and arranging windows,
//! refreshing whatever the focused window shows, adding a reminder, searching every
//! project, opening the projects, labels or settings window or editing projects or saved filters, and filing a bug report or
//! feature request.
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
        CheckForUpdates,
        NewReminder,
        OpenProjects,
        OpenLabels,
        EditProjects,
        EditSavedFilters,
        SearchAllTasks,
        ReportBug,
        RequestFeature,
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
        bind("secondary-n", NewReminder),
        // Like Mail's Cmd+0 for its main window; beside Cmd/Ctrl+1 to 9 for the projects.
        bind("secondary-0", OpenProjects),
        // Mod+F searches within a window; with Shift, every project from any window.
        bind("secondary-shift-f", SearchAllTasks),
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
    cx.on_action(|_: &CheckForUpdates, cx| crate::settings_window::check_for_updates(cx));
    cx.on_action(|_: &NewReminder, cx| crate::new_reminder_window::open(cx));
    cx.on_action(|_: &OpenProjects, cx| crate::open_projects(cx));
    cx.on_action(|_: &OpenLabels, cx| crate::labels_window::open(cx));
    cx.on_action(|_: &EditProjects, cx| crate::project_editor_window::open(cx));
    cx.on_action(|_: &EditSavedFilters, cx| crate::filter_editor_window::open(cx));
    cx.on_action(|_: &SearchAllTasks, cx| crate::reminders_window::show_search(cx));
    cx.on_action(|_: &ReportBug, cx| cx.open_url(&crate::app_issues::bug_report()));
    cx.on_action(|_: &RequestFeature, cx| cx.open_url(&crate::app_issues::feature_request()));
    cx.on_action(|_: &Refresh, cx| {
        // Each list view has its own refresh action; only the focused one handles it.
        // This binding outranks the views' own when focus sits in a nested context (such
        // as a table), so it must reach the window the same deferred way.
        with_active_window(cx, |window, cx| {
            window.dispatch_action(Box::new(projects_ui::Refresh), cx);
            window.dispatch_action(Box::new(reminders_ui::Refresh), cx);
        });
    });
}

/// Run `f` on the active window.
///
/// Deferred: a key binding's action runs while GPUI is still updating the window it was
/// pressed in, and updating that window again from inside fails, so the action would be
/// silently dropped (the menu bar dispatches outside any window, so it worked from there).
fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        if let Err(err) = handle.update(cx, |_, window, cx| f(window, cx)) {
            log::warn!("window action skipped: {err:#}");
        }
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestEventExt as _;
    use gpui_kit::{
        AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
        TestAppContext, WindowOptions, div,
    };

    use super::*;

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn mod_w_closes_the_window_it_is_pressed_in(cx: &mut TestAppContext) {
        let handle = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Blank))
                .unwrap()
                .0
        });
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            window.key_down("secondary-w", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.update(|cx| cx.windows().is_empty()));
    }

    /// Stands in for a list view: refreshes on its own action, with focus in a nested
    /// context the way a focused table puts it.
    struct List {
        focus_handle: gpui_kit::FocusHandle,
        refreshed: bool,
    }

    impl Render for List {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .key_context("Reminders")
                .on_action(cx.listener(|this, _: &reminders_ui::Refresh, _, _| {
                    this.refreshed = true;
                }))
                .child(
                    div()
                        .key_context("DataTable")
                        .track_focus(&self.focus_handle),
                )
        }
    }

    #[gpui_kit::test]
    fn mod_r_refreshes_with_focus_in_a_nested_context(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            reminders_ui::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| List {
                    focus_handle: cx.focus_handle(),
                    refreshed: false,
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            let focus_handle = view.read(cx).focus_handle.clone();
            focus_handle.focus(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.key_down("secondary-r", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.update(|cx| view.read(cx).refreshed));
    }

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
