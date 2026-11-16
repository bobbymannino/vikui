//! A side panel showing one reminder and its task.
//!
//! The view is given [`ReminderDetails`] to show and reports what the user asks for through
//! [`ReminderDetailsEvent`]: closing the panel or marking the task done. The host decides
//! where the panel lives and does the saving.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    App, Context, ElementId, EventEmitter, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use reminder_details::ReminderDetails;

/// Events for the host to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReminderDetailsEvent {
    /// The user asked to close the panel.
    Close,
    /// The user asked to mark the task done (or not done). Save it, then call
    /// [`ReminderDetailsView::set_details`] with the result or
    /// [`ReminderDetailsView::set_error`].
    SetDone { task_id: i64, done: bool },
    /// The user asked to open the task in a window of its own.
    OpenInWindow { task_id: i64 },
}

/// One labelled value, e.g. "Due" / "Tue 1 Dec 2026, 09:00".
#[derive(Clone, Debug, PartialEq, Eq)]
struct Field {
    label: SharedString,
    value: SharedString,
}

impl Field {
    fn new(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// Everything the panel shows, formatted off the render path.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Content {
    title: SharedString,
    identifier: SharedString,
    reminder: Vec<Field>,
    task: Vec<Field>,
    description: SharedString,
    /// One line per reminder on the task: "Tue 1 Dec 2026, 08:00 · 1 hour before due date".
    reminders: Vec<SharedString>,
}

impl Content {
    fn new(details: &ReminderDetails, now: Timestamp, zone: &TimeZone) -> Self {
        let reminder = &details.reminder;
        let reminder_fields = if reminder.is_set {
            vec![
                Field::new("When", reminder.when(zone)),
                Field::new("Schedule", reminder.schedule()),
                Field::new("Status", reminder.status(now).label()),
            ]
        } else {
            vec![Field::new("When", "No reminder set")]
        };

        let mut task = Vec::new();
        if let Some(project) = &details.project {
            task.push(Field::new("Project", project.clone()));
        }
        task.push(Field::new(
            "Status",
            if reminder.task_done { "Done" } else { "Open" },
        ));
        if let Some(priority) = details.priority {
            task.push(Field::new("Priority", priority.label()));
        }
        // On the task, not the reminder: a task repeats whether or not it has reminders.
        if let Some(repeat) = details.repeat {
            task.push(Field::new("Repeating", reminders::describe_repeat(repeat)));
        }
        task.extend(
            details
                .dates()
                .into_iter()
                .map(|(label, at)| Field::new(label, reminders::format_time(at, zone))),
        );

        Self {
            title: reminder.task_title.clone().into(),
            identifier: reminder.task_identifier.clone().into(),
            reminder: reminder_fields,
            task,
            description: details.description.clone().into(),
            reminders: details
                .reminders
                .iter()
                .map(|other| {
                    let schedule = other.schedule();
                    format!("{} · {schedule}", other.when(zone)).into()
                })
                .collect(),
        }
    }
}

/// The task shown, and whether it is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShownTask {
    id: i64,
    done: bool,
}

/// The details panel; empty until [`Self::set_details`] is called.
#[derive(Default)]
pub struct ReminderDetailsView {
    content: Option<Content>,
    task: Option<ShownTask>,
    /// A done/not-done change is being saved.
    saving: bool,
    error: Option<SharedString>,
    /// Shown as a window's whole content, not a side panel: no close or open-in-window
    /// buttons (the window has its own close).
    in_window: bool,
}

impl ReminderDetailsView {
    /// A side panel, with buttons to close it and to open the task in its own window.
    pub fn new() -> Self {
        Self::default()
    }

    /// The content of a window showing one task.
    pub fn new_window() -> Self {
        Self {
            in_window: true,
            ..Self::default()
        }
    }

    /// Show `details`, replacing whatever was shown, and end any pending save.
    pub fn set_details(&mut self, details: &ReminderDetails, cx: &mut Context<Self>) {
        let content = Content::new(details, Timestamp::now(), &TimeZone::system());
        let task = Some(ShownTask {
            id: details.reminder.task_id,
            done: details.reminder.task_done,
        });
        if self.content.as_ref() != Some(&content)
            || self.task != task
            || self.saving
            || self.error.is_some()
        {
            self.content = Some(content);
            self.task = task;
            self.saving = false;
            self.error = None;
            cx.notify();
        }
    }

    /// The id of the task shown, if any.
    pub fn task_id(&self) -> Option<i64> {
        self.task.map(|task| task.id)
    }

    /// Show why saving failed; the task is shown as it was.
    pub fn set_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(message.into());
        cx.notify();
    }

    fn toggle_done(&mut self, cx: &mut Context<Self>) {
        let Some(task) = self.task.filter(|_| !self.saving) else {
            return;
        };
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.emit(ReminderDetailsEvent::SetDone {
            task_id: task.id,
            done: !task.done,
        });
    }
}

impl EventEmitter<ReminderDetailsEvent> for ReminderDetailsView {}

fn section(
    id: &'static str,
    heading: &'static str,
    cx: &App,
    children: impl IntoIterator<Item = impl IntoElement>,
) -> impl IntoElement {
    v_flex()
        .id(id)
        .role(Role::Group)
        .aria_label(heading)
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(heading),
        )
        .children(children)
}

fn field(id: impl Into<ElementId>, field: &Field, cx: &App) -> impl IntoElement {
    h_flex()
        .id(id)
        .aria_label(format!("{}: {}", field.label, field.value))
        .gap_3()
        .items_start()
        .text_sm()
        .child(
            div()
                .w(px(80.))
                .flex_none()
                .text_color(cx.theme().muted_foreground)
                .child(field.label.clone()),
        )
        .child(div().flex_1().min_w_0().child(field.value.clone()))
        .test_support()
}

fn fields(prefix: &'static str, fields: &[Field], cx: &App) -> Vec<impl IntoElement> {
    fields
        .iter()
        .enumerate()
        .map(|(ix, item)| field((prefix, ix), item, cx))
        .collect()
}

impl Render for ReminderDetailsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let header = h_flex()
            .gap_2()
            .items_start()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child({
                        let title = self
                            .content
                            .as_ref()
                            .map_or_else(SharedString::default, |c| c.title.clone());
                        div()
                            .id("details-title")
                            .role(Role::Heading)
                            .aria_label(title.clone())
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title)
                            .test_support()
                    })
                    .when_some(
                        self.content
                            .as_ref()
                            .map(|c| c.identifier.clone())
                            .filter(|id| !id.is_empty()),
                        |this, identifier| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(identifier),
                            )
                        },
                    ),
            )
            .when(!self.in_window, |header| {
                header
                    .child(
                        Button::new("open-task-window")
                            .icon(IconName::ExternalLink)
                            .ghost()
                            .xsmall()
                            .accessibility_label("Open in new window")
                            .tooltip("Open in new window")
                            .disabled(self.task.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(task) = this.task {
                                    cx.emit(ReminderDetailsEvent::OpenInWindow {
                                        task_id: task.id,
                                    });
                                }
                            })),
                    )
                    .child(
                        Button::new("close-reminder-details")
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .accessibility_label("Close details")
                            .tooltip("Close details")
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(ReminderDetailsEvent::Close)),
                            ),
                    )
            });

        let done_button = self.task.map(|task| {
            let label = if task.done {
                "Mark as not done"
            } else {
                "Mark as complete"
            };
            Button::new("toggle-task-done")
                .when(!task.done, |button| button.icon(IconName::Check))
                .label(if self.saving { "Saving…" } else { label })
                .accessibility_label(label)
                .outline()
                .small()
                .loading(self.saving)
                .disabled(self.saving)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_done(cx)))
        });
        let error = self.error.clone().map(|message| {
            // `Alert` has no accessible name of its own; announce the message.
            div()
                .id("task-done-error")
                .role(Role::Alert)
                .aria_label(format!("Couldn't update the task: {message}"))
                .child(
                    Alert::error("task-done-error-alert", message)
                        .title("Couldn't update the task"),
                )
                .test_support()
        });

        let body = self.content.as_ref().map(|content| {
            v_flex()
                .gap_5()
                .child(section(
                    "details-reminder",
                    "Reminder",
                    cx,
                    fields("reminder-field", &content.reminder, cx),
                ))
                .child(section(
                    "details-task",
                    "Task",
                    cx,
                    fields("task-field", &content.task, cx),
                ))
                .when(!content.description.is_empty(), |this| {
                    this.child(section(
                        "details-description",
                        "Description",
                        cx,
                        [div()
                            .id("description")
                            .aria_label(content.description.clone())
                            .text_sm()
                            .child(content.description.clone())
                            .test_support()],
                    ))
                })
                .when(!content.reminders.is_empty(), |this| {
                    this.child(section(
                        "details-all-reminders",
                        "All reminders on this task",
                        cx,
                        content.reminders.iter().enumerate().map(|(ix, line)| {
                            div()
                                .id(("task-reminder", ix))
                                .aria_label(line.clone())
                                .text_sm()
                                .child(line.clone())
                                .test_support()
                        }),
                    ))
                })
        });

        v_flex()
            .id("reminder-details")
            .role(Role::Region)
            .aria_label(if self.in_window {
                "Task details"
            } else {
                "Reminder details"
            })
            .size_full()
            .gap_4()
            .p_4()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(header)
            .children(done_button)
            .children(error)
            .child(
                div()
                    .id("reminder-details-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(body),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, size,
    };
    use vikunja::{RelativeReminder, RelativeTo, Task, TaskReminder};

    use super::*;

    fn ts(value: &str) -> Timestamp {
        value.parse().unwrap()
    }

    fn task() -> Task {
        Task {
            id: 7,
            title: "Pay rent".into(),
            identifier: "HOME-7".into(),
            description: "<p>Transfer &amp; confirm</p>".into(),
            priority: 4,
            due_date: Some(ts("2026-12-01T09:00:00Z")),
            reminders: vec![TaskReminder {
                at: Some(ts("2026-12-01T08:00:00Z")),
                relative: Some(RelativeReminder {
                    period_secs: -3600,
                    to: RelativeTo::Due,
                }),
            }],
            ..Task::default()
        }
    }

    fn details(task: &Task) -> ReminderDetails {
        let all = reminders::from_tasks(std::slice::from_ref(task));
        ReminderDetails::new(&all[0], task, Some("Home"))
    }

    #[test]
    fn formats_reminder_and_task_fields() {
        let now = ts("2026-11-15T00:00:00Z");
        let content = Content::new(&details(&task()), now, &TimeZone::UTC);
        assert_eq!(
            content.reminder,
            [
                Field::new("When", "Tue 1 Dec 2026, 08:00"),
                Field::new("Schedule", "1 hour before due date"),
                Field::new("Status", "Upcoming"),
            ]
        );
        assert_eq!(
            content.task,
            [
                Field::new("Project", "Home"),
                Field::new("Status", "Open"),
                Field::new("Priority", "Urgent"),
                Field::new("Due", "Tue 1 Dec 2026, 09:00"),
            ]
        );
        assert_eq!(content.description, "Transfer & confirm");
        assert_eq!(
            content.reminders,
            ["Tue 1 Dec 2026, 08:00 · 1 hour before due date"]
        );

        let weekly = Task {
            repeat_after: 7 * 86_400,
            ..task()
        };
        let content = Content::new(&details(&weekly), now, &TimeZone::UTC);
        assert!(
            content
                .task
                .contains(&Field::new("Repeating", "every 1 week"))
        );
        // Shown for a repeating task without any reminders too.
        let weekly_plain = Task {
            reminders: vec![],
            ..weekly
        };
        let content = Content::new(&details(&weekly_plain), now, &TimeZone::UTC);
        assert!(
            content
                .task
                .contains(&Field::new("Repeating", "every 1 week"))
        );

        let plain = Task {
            reminders: vec![],
            ..task()
        };
        let content = Content::new(&details(&plain), now, &TimeZone::UTC);
        assert_eq!(content.reminder, [Field::new("When", "No reminder set")]);
        assert_eq!(content.reminders, Vec::<SharedString>::new());
    }

    #[gpui_kit::test]
    fn shows_details_and_asks_to_close(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(340.), px(640.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| {
                    let mut view = ReminderDetailsView::new();
                    view.set_details(&details(&task()), cx);
                    view
                })
            })
            .unwrap()
        });
        let closed = std::rc::Rc::new(std::cell::Cell::new(false));
        cx.update(|cx| {
            let closed = std::rc::Rc::clone(&closed);
            cx.subscribe(&view, move |_, event, _| {
                assert_eq!(*event, ReminderDetailsEvent::Close);
                closed.set(true);
            })
            .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Reminder details");
            window.find_by_label("Priority: Urgent");
            window.find_by_label("Project: Home");
            window.find_by_label("Schedule: 1 hour before due date");
            window.find_by_label("Transfer & confirm");
            window.find_by_label("Close details");
            window.click("close-reminder-details", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(closed.get());
    }

    #[gpui_kit::test]
    fn marks_the_task_complete(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| {
                    let mut view = ReminderDetailsView::new();
                    view.set_details(&details(&task()), cx);
                    view
                })
            })
            .unwrap()
        });
        let requests = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let requests = std::rc::Rc::clone(&requests);
            cx.subscribe(&view, move |_, event, _| requests.borrow_mut().push(*event))
                .detach();
        });
        let click = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("toggle-task-done", cx);
            })
            .unwrap();
            cx.run_until_parked();
        };
        let has_label = |cx: &mut TestAppContext, label: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(label).is_some()
            })
            .unwrap()
        };

        assert!(has_label(cx, "Mark as complete"));
        click(cx);
        // A second click while saving is ignored.
        click(cx);
        assert_eq!(
            *requests.borrow(),
            [ReminderDetailsEvent::SetDone {
                task_id: 7,
                done: true
            }]
        );

        cx.update(|cx| view.update(cx, |view, cx| view.set_error("Server unavailable", cx)));
        assert!(has_label(
            cx,
            "Couldn't update the task: Server unavailable"
        ));

        let done = Task {
            done: true,
            ..task()
        };
        cx.update(|cx| view.update(cx, |view, cx| view.set_details(&details(&done), cx)));
        assert!(has_label(cx, "Mark as not done"));
        assert!(has_label(cx, "Status: Done"));
        assert!(!has_label(
            cx,
            "Couldn't update the task: Server unavailable"
        ));
    }

    #[gpui_kit::test]
    fn opens_the_task_in_a_window_of_its_own(cx: &mut TestAppContext) {
        let open_view = |cx: &mut TestAppContext, build: fn() -> ReminderDetailsView| {
            cx.update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                    cx.new(|cx| {
                        let mut view = build();
                        view.set_details(&details(&task()), cx);
                        view
                    })
                })
                .unwrap()
            })
        };
        let (handle, view) = open_view(cx, ReminderDetailsView::new);
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = std::rc::Rc::clone(&events);
            cx.subscribe(&view, move |_, event, _| events.borrow_mut().push(*event))
                .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Open in new window");
            window.click("open-task-window", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            *events.borrow(),
            [ReminderDetailsEvent::OpenInWindow { task_id: 7 }]
        );

        // In its own window the task has neither the panel's close nor open buttons.
        let (handle, _) = open_view(cx, ReminderDetailsView::new_window);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_by_label("Task details");
            window.find_by_label("Pay rent");
            assert!(window.try_find_by_label("Open in new window").is_none());
            assert!(window.try_find_by_label("Close details").is_none());
        })
        .unwrap();
    }
}
