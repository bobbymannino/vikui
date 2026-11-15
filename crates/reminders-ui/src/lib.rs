//! A window listing reminders.
//!
//! The view does not fetch anything: the caller passes in the reminders (from one
//! project, a filter, or search results) and can replace them later.
//!
//! Clicking a row (or pressing Enter on it) toggles a details side panel. The view only
//! reserves the space and reports [`RemindersEvent::ShowDetails`]; the caller supplies
//! the panel with [`RemindersView::set_details_panel`] and fills it in.

mod reminders_table;

use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, Focusable as _,
    FontWeight, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    WindowOptions, div, prelude::FluentBuilder as _, px,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use reminders::Reminder;

use reminders_table::{ReminderRow, RemindersTable};

gpui_kit::actions!(reminders, [ToggleDetails]);

const CONTEXT: &str = "Reminders";
const DETAILS_WIDTH: f32 = 340.;

/// Register the reminders view's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("enter", ToggleDetails, Some(CONTEXT))]);
}

/// Events for the app to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemindersEvent {
    /// The details panel opened on, or moved to, this reminder; fill the panel in.
    ShowDetails(Reminder),
}

/// A titled, sortable list of reminders with an optional details panel.
pub struct RemindersView {
    title: SharedString,
    table: Entity<TableState<RemindersTable>>,
    summary: SharedString,
    error: Option<SharedString>,
    /// What the rows were made from; rows refer to it by `order`.
    reminders: Vec<Reminder>,
    details_panel: Option<AnyView>,
    /// `order` of the reminder the details panel is open on.
    details_for: Option<usize>,
    _table_events: Subscription,
}

impl RemindersView {
    /// Show `reminders` under `title`. Pass `None` to show a loading state until
    /// [`Self::set_reminders`] or [`Self::set_error`] is called.
    pub fn new(
        title: impl Into<SharedString>,
        reminders: Option<Vec<Reminder>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let table = cx.new(|cx| TableState::new(RemindersTable::default(), window, cx));
        table.read(cx).focus_handle(cx).focus(window, cx);

        let view = cx.entity().downgrade();
        table.update(cx, |table, _| {
            table
                .delegate_mut()
                .set_on_row_click(Rc::new(move |row_ix, _, cx| {
                    view.update(cx, |this, cx| this.toggle_row(row_ix, cx)).ok();
                }));
        });
        let table_events = cx.subscribe(&table, |this, _, event: &TableEvent, cx| match event {
            // Clicks are handled by `toggle_row`; this follows keyboard selection while the
            // panel is open.
            TableEvent::SelectRow(row_ix) if this.details_for.is_some() => {
                this.show_row(*row_ix, cx);
            }
            TableEvent::ClearSelection => this.hide_details(cx),
            _ => {}
        });

        let mut this = Self {
            title: title.into(),
            table,
            summary: SharedString::default(),
            error: None,
            reminders: Vec::new(),
            details_panel: None,
            details_for: None,
            _table_events: table_events,
        };
        match reminders {
            Some(reminders) => this.set_reminders(&reminders, None, cx),
            None => this.table.update(cx, |table, cx| {
                table.delegate_mut().set_loading(true);
                cx.notify();
            }),
        }
        this
    }

    /// Replace the listed reminders.
    ///
    /// `matched_tasks` is how many tasks the reminders came from, if known; tasks without
    /// a reminder are listed too, so an empty list means no tasks matched.
    pub fn set_reminders(
        &mut self,
        reminders: &[Reminder],
        matched_tasks: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let rows = ReminderRow::from_reminders(reminders, Timestamp::now(), &TimeZone::system());
        let set = rows.iter().filter(|row| row.is_set).count();
        let upcoming = rows
            .iter()
            .filter(|row| row.is_set && !row.inactive)
            .count();
        self.summary = summary(set, upcoming, matched_tasks);
        let empty_message = empty_message(matched_tasks);
        self.error = None;
        self.reminders = reminders.to_vec();
        self.details_for = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_empty_message(empty_message);
            table.delegate_mut().set_rows(rows);
            cx.notify();
        });
        cx.notify();
    }

    /// The view shown in the details panel; it stays hidden until a row is opened.
    pub fn set_details_panel(&mut self, panel: AnyView, cx: &mut Context<Self>) {
        self.details_panel = Some(panel);
        cx.notify();
    }

    /// Close the details panel and return focus to the list.
    pub fn close_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_details(cx);
        self.table.read(cx).focus_handle(cx).focus(window, cx);
    }

    fn hide_details(&mut self, cx: &mut Context<Self>) {
        if self.details_for.take().is_some() {
            cx.notify();
        }
    }

    fn order_of(&self, row_ix: usize, cx: &App) -> Option<usize> {
        self.table
            .read(cx)
            .delegate()
            .row(row_ix)
            .map(|row| row.order)
    }

    /// Open the panel on the row, or close it if it is already open there.
    fn toggle_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let order = self.order_of(row_ix, cx);
        if order.is_some() && order == self.details_for {
            self.hide_details(cx);
        } else {
            self.show_row(row_ix, cx);
        }
    }

    fn show_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        let Some(order) = self.order_of(row_ix, cx) else {
            return;
        };
        if self.details_for == Some(order) {
            return;
        }
        let Some(reminder) = self.reminders.get(order).cloned() else {
            return;
        };
        self.details_for = Some(order);
        cx.emit(RemindersEvent::ShowDetails(reminder));
        cx.notify();
    }

    fn on_toggle_details(&mut self, _: &ToggleDetails, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(row_ix) = self.table.read(cx).selected_row() {
            self.toggle_row(row_ix, cx);
        }
    }

    /// Show why the reminders could not be loaded.
    pub fn set_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_loading(false);
            cx.notify();
        });
        cx.notify();
    }
}

fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}

/// "2 reminders on 5 tasks, 1 upcoming".
fn summary(reminders: usize, upcoming: usize, matched_tasks: Option<usize>) -> SharedString {
    let tasks = matched_tasks.map_or_else(String::new, |tasks| {
        format!(" on {}", plural(tasks, "task"))
    });
    format!(
        "{}{tasks}, {upcoming} upcoming",
        plural(reminders, "reminder")
    )
    .into()
}

fn empty_message(matched_tasks: Option<usize>) -> SharedString {
    match matched_tasks {
        None => "No reminders.".into(),
        Some(_) => "No tasks match.".into(),
    }
}

impl EventEmitter<RemindersEvent> for RemindersView {}

impl Render for RemindersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let details = self
            .details_panel
            .clone()
            .filter(|_| self.details_for.is_some());
        v_flex()
            .id("reminders")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_toggle_details))
            .size_full()
            .gap_4()
            .p_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(self.summary.clone()),
                    ),
            )
            .when_some(self.error.clone(), |this, message| {
                this.child(
                    // `Alert` has no accessible name of its own; announce the message.
                    div()
                        .id("reminders-error")
                        .role(Role::Alert)
                        .aria_label(format!("Couldn't load reminders: {message}"))
                        .child(
                            Alert::error("reminders-error-alert", message)
                                .title("Couldn't load reminders"),
                        )
                        .test_support(),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(
                        div().flex_1().min_w_0().h_full().child(
                            DataTable::new(&self.table)
                                .stripe(true)
                                .accessibility_label("Reminders"),
                        ),
                    )
                    .when_some(details, |this, panel| {
                        this.child(
                            div()
                                .w(px(DETAILS_WIDTH))
                                .flex_none()
                                .h_full()
                                .ml_4()
                                .border_l_1()
                                .border_color(theme.border)
                                .child(panel),
                        )
                    }),
            )
    }
}

/// Open a reminders window; see [`RemindersView::new`] for `reminders`.
pub fn open(
    options: WindowOptions,
    title: impl Into<SharedString>,
    reminders: Option<Vec<Reminder>>,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<RemindersView>)> {
    let title = title.into();
    let (handle, view) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| RemindersView::new(title, reminders, window, cx))
    })?;
    Ok((handle, view))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{Bounds, Point, TestAppContext, WindowBounds, px, size};

    use super::*;

    fn reminder(at: Timestamp, title: &str) -> Reminder {
        Reminder {
            at: Some(at),
            relative: Some(vikunja::RelativeReminder {
                period_secs: -3600,
                to: vikunja::RelativeTo::Due,
            }),
            task_id: 1,
            task_title: title.into(),
            task_identifier: "HOME-1".into(),
            project_id: 1,
            task_done: false,
            is_set: true,
        }
    }

    fn options() -> WindowOptions {
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(960.), px(640.)),
            })),
            ..Default::default()
        }
    }

    #[gpui_kit::test]
    fn lists_the_reminders_it_is_given(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let handle = cx.update(|cx| {
            gpui_kit::init(cx);
            let reminders = vec![reminder(soon, "Pay rent")];
            open(options(), "Reminders – Home", Some(reminders), cx)
                .unwrap()
                .0
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Reminders");
            window.find_by_label("Pay rent");
            window.find_by_label("1 hour before due date");
            window.find_by_label("Upcoming");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn loads_later_or_shows_an_error(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(options(), "Reminders – search “rent”", None, cx).unwrap()
        });
        cx.update(|cx| view.update(cx, |view, cx| view.set_error("Server unavailable", cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Couldn't load reminders: Server unavailable");
        })
        .unwrap();

        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(1);
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.set_reminders(&[reminder(soon, "Call landlord")], Some(1), cx);
            });
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Call landlord");
            assert!(
                window
                    .try_find_by_label("Couldn't load reminders: Server unavailable")
                    .is_none()
            );
        })
        .unwrap();
    }

    struct TestPanel;

    impl Render for TestPanel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("test-panel")
                .aria_label("Test panel")
                .child("Details")
                .test_support()
        }
    }

    #[gpui_kit::test]
    fn clicking_a_row_toggles_the_details_panel(cx: &mut TestAppContext) {
        let soon = Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let later = soon + jiff::SignedDuration::from_hours(24);
        let second = Reminder {
            task_id: 2,
            ..reminder(later, "Call landlord")
        };
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let reminders = vec![reminder(soon, "Pay rent"), second.clone()];
            open(options(), "Reminders – Home", Some(reminders), cx).unwrap()
        });
        let shown = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let panel = cx.new(|_| TestPanel);
            view.update(cx, |view, cx| view.set_details_panel(panel.into(), cx));
            let shown = Rc::clone(&shown);
            cx.subscribe(&view, move |_, RemindersEvent::ShowDetails(reminder), _| {
                shown.borrow_mut().push(reminder.task_title.clone());
            })
            .detach();
        });
        let panel_visible = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label("Test panel").is_some()
            })
            .unwrap()
        };
        assert!(!panel_visible(cx));

        let click = |cx: &mut TestAppContext, row_ix: usize| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(("row", row_ix), cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        click(cx, 0);
        assert!(panel_visible(cx));
        click(cx, 1);
        assert!(panel_visible(cx));
        click(cx, 1);
        assert!(!panel_visible(cx));
        assert_eq!(*shown.borrow(), ["Pay rent", "Call landlord"]);

        // Keyboard: Enter opens it on the selected row, Up follows, Escape closes.
        let key = |cx: &mut TestAppContext, key: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.key_down(key, false, cx);
                window.key_up(key, cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        key(cx, "enter");
        assert!(panel_visible(cx));
        key(cx, "up");
        assert_eq!(
            *shown.borrow(),
            ["Pay rent", "Call landlord", "Call landlord", "Pay rent"]
        );
        key(cx, "escape");
        assert!(!panel_visible(cx));
    }

    #[test]
    fn summary_and_empty_message_mention_matched_tasks() {
        assert_eq!(summary(2, 1, Some(5)), "2 reminders on 5 tasks, 1 upcoming");
        assert_eq!(summary(1, 0, None), "1 reminder, 0 upcoming");
        assert_eq!(summary(0, 0, Some(1)), "0 reminders on 1 task, 0 upcoming");
        assert_eq!(empty_message(None), "No reminders.");
        assert_eq!(empty_message(Some(0)), "No tasks match.");
    }

    #[gpui_kit::test]
    fn lists_tasks_without_reminders(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            open(options(), "Reminders – Today", None, cx).unwrap()
        });
        let plain = Reminder {
            at: None,
            relative: None,
            is_set: false,
            ..reminder(Timestamp::now(), "Buy milk")
        };
        cx.update(|cx| view.update(cx, |view, cx| view.set_reminders(&[plain], Some(1), cx)));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Buy milk");
            window.find_by_label("No reminder");
        })
        .unwrap();
    }
}
