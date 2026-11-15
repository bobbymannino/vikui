//! A window listing reminders.
//!
//! The view does not fetch anything: the caller passes in the reminders (from one
//! project, a filter, or search results) and can replace them later.

mod reminders_table;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::table::{DataTable, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, WindowOptions, div,
    prelude::FluentBuilder as _,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use reminders::Reminder;

use reminders_table::{ReminderRow, RemindersTable};

/// A titled, sortable list of reminders.
pub struct RemindersView {
    title: SharedString,
    table: Entity<TableState<RemindersTable>>,
    summary: SharedString,
    error: Option<SharedString>,
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
        let mut this = Self {
            title: title.into(),
            table,
            summary: SharedString::default(),
            error: None,
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
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_empty_message(empty_message);
            table.delegate_mut().set_rows(rows);
            cx.notify();
        });
        cx.notify();
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

impl Render for RemindersView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("reminders")
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
                div().flex_1().min_h_0().child(
                    DataTable::new(&self.table)
                        .stripe(true)
                        .accessibility_label("Reminders"),
                ),
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
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
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
