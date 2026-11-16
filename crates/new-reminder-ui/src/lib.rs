//! The new reminder window: a form for a task's title, notes, due date, priority,
//! repeat, labels and project.
//!
//! It never talks to the server. The app fills in the projects and labels with
//! [`NewReminderView::set_options`], then creates the reminder when the view reports
//! [`NewReminderEvent::Create`], answering with [`NewReminderView::set_error`] if that
//! fails. Cmd/Ctrl+Enter (or Enter in the title) creates; Escape cancels.

mod choice;
mod label_chip;

use std::collections::BTreeSet;
use std::rc::Rc;

use chrono::{Datelike as _, NaiveDate, NaiveDateTime, Timelike as _};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _, Toggle, ToggleVariants as _};
use gpui_kit::component::date_picker::{DatePicker, DatePickerState, DateTime as PickedDateTime};
use gpui_kit::component::form::{field, v_form};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{SearchableVec, Select, SelectState};
use gpui_kit::component::time_field::TimePrecision;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter,
    Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyBinding,
    KeyBindingContextPredicate, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, WindowOptions, div, px,
};
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use new_reminder::{Draft, Options, RepeatChoice};
use reminders::Priority;

use choice::Choice;
use label_chip::LabelChip;

const CONTEXT: &str = "NewReminder";

gpui_kit::actions!(new_reminder, [Create, Cancel]);

/// Register the window's key bindings. Call once at startup.
pub fn init(cx: &mut App) {
    // Enter creates from the title field (see `NewReminderView::new`), not everywhere:
    // the notes field needs it for new lines.
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))]);
    let context = KeyBindingContextPredicate::parse(CONTEXT).ok().map(Rc::new);
    match KeyBinding::load(
        "secondary-enter",
        Box::new(Create),
        context,
        false,
        None,
        cx,
    ) {
        Ok(binding) => cx.bind_keys([binding]),
        Err(err) => log::error!("invalid create reminder key binding: {err}"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewReminderEvent {
    /// The user asked to create this reminder; it has a title and a project.
    Create(Draft),
    /// The user closed the form without creating anything.
    Cancel,
}

type ChoiceSelect = SelectState<SearchableVec<Choice>>;

/// The form. Projects and labels stay empty, and creating disabled, until
/// [`Self::set_options`] is called.
pub struct NewReminderView {
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    due: Entity<DatePickerState>,
    priority: Entity<ChoiceSelect>,
    repeat: Entity<ChoiceSelect>,
    project: Entity<ChoiceSelect>,
    labels: Vec<LabelChip>,
    selected_labels: BTreeSet<i64>,
    /// The projects and labels have arrived.
    loaded: bool,
    saving: bool,
    error: Option<SharedString>,
    _title_events: Subscription,
}

impl EventEmitter<NewReminderEvent> for NewReminderView {}

impl NewReminderView {
    /// An empty form due at `now` (in the user's time zone), with low priority.
    pub fn new(now: DateTime, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let defaults = Draft::new(now, None);
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("What to be reminded of"));
        let body = cx.new(|cx| TextareaState::new(window, cx).placeholder("Notes"));
        let due = cx.new(|cx| {
            let mut state = DatePickerState::new(window, cx).time_precision(TimePrecision::Minute);
            if let Some(due) = defaults.due.and_then(to_naive) {
                state.set_date_time(due, window, cx);
            }
            state
        });
        let priorities: Vec<Choice> = Priority::ALL
            .iter()
            .map(|priority| Choice::new(priority.level(), priority.label()))
            .collect();
        let low = Priority::ALL.iter().position(|p| *p == defaults.priority);
        let priority = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(priorities),
                low.map(IndexPath::new),
                window,
                cx,
            )
        });
        let repeats: Vec<Choice> = RepeatChoice::ALL
            .iter()
            .enumerate()
            .map(|(ix, repeat)| Choice::new(index_id(ix), repeat.label()))
            .collect();
        let never = RepeatChoice::ALL.iter().position(|r| *r == defaults.repeat);
        let repeat = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(repeats),
                never.map(IndexPath::new),
                window,
                cx,
            )
        });
        let project = cx.new(|cx| {
            SelectState::new(SearchableVec::new(Vec::new()), None, window, cx).searchable(true)
        });
        title.read(cx).focus_handle(cx).focus(window, cx);
        let title_events = cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.create(cx);
            }
        });
        Self {
            title,
            body,
            due,
            priority,
            repeat,
            project,
            labels: Vec::new(),
            selected_labels: BTreeSet::new(),
            loaded: false,
            saving: false,
            error: None,
            _title_events: title_events,
        }
    }

    /// Offer `options`' projects and labels, selecting the default project.
    pub fn set_options(&mut self, options: &Options, window: &mut Window, cx: &mut Context<Self>) {
        let projects: Vec<Choice> = options
            .projects
            .iter()
            .map(|project| Choice::new(project.id, project.title.clone()))
            .collect();
        self.project.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(projects), window, cx);
            if let Some(id) = options.default_project_id {
                select.set_selected_value(&id, window, cx);
            }
        });
        self.labels = options
            .labels
            .iter()
            .map(|label| LabelChip {
                id: label.id,
                title: label.title.clone().into(),
                rgb: label.rgb(),
            })
            .collect();
        self.selected_labels
            .retain(|id| options.labels.iter().any(|label| label.id == *id));
        self.loaded = true;
        if options.projects.is_empty() {
            self.error = Some("Create a project in Vikunja first.".into());
        }
        cx.notify();
    }

    /// Show why the projects and labels couldn't be loaded.
    pub fn set_load_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        cx.notify();
    }

    /// Show why creating failed; the form keeps what was entered.
    pub fn set_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(message.into());
        cx.notify();
    }

    /// What the form holds now.
    pub fn draft(&self, cx: &App) -> Draft {
        let repeat = self
            .repeat
            .read(cx)
            .selected_value()
            .and_then(|&ix| usize::try_from(ix).ok())
            .and_then(|ix| RepeatChoice::ALL.get(ix).copied())
            .unwrap_or_default();
        let priority = self
            .priority
            .read(cx)
            .selected_value()
            .and_then(|&level| Priority::from_level(level))
            .unwrap_or(Priority::Low);
        let due = match self.due.read(cx).date_time() {
            PickedDateTime::Single(due) => due.and_then(from_naive),
            PickedDateTime::Range(..) => None,
        };
        Draft {
            title: self.title.read(cx).value().to_string(),
            body: self.body.read(cx).value().to_string(),
            due,
            priority,
            repeat,
            label_ids: self.selected_labels.iter().copied().collect(),
            project_id: self.project.read(cx).selected_value().copied(),
        }
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        if self.saving || !self.loaded {
            return;
        }
        let draft = self.draft(cx);
        if let Err(invalid) = draft.to_task(&TimeZone::system()) {
            self.error = Some(invalid.message().into());
            cx.notify();
            return;
        }
        self.saving = true;
        self.error = None;
        cx.notify();
        cx.emit(NewReminderEvent::Create(draft));
    }

    fn on_create(&mut self, _: &Create, _: &mut Window, cx: &mut Context<Self>) {
        self.create(cx);
    }

    /// Close without creating, unless a reminder is being created: closing then would
    /// hide whether it worked.
    fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.saving {
            cx.emit(NewReminderEvent::Cancel);
        }
    }

    fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel(cx);
    }

    fn toggle_label(&mut self, label_id: i64, cx: &mut Context<Self>) {
        if !self.selected_labels.remove(&label_id) {
            self.selected_labels.insert(label_id);
        }
        cx.notify();
    }

    fn render_labels(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.labels.is_empty() {
            let text = if self.loaded {
                "No labels yet."
            } else {
                "Loading…"
            };
            return div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text)
                .into_any_element();
        }
        h_flex()
            .id("new-reminder-labels")
            .role(Role::Group)
            .aria_label("Labels")
            .flex_wrap()
            .gap_1()
            .children(self.labels.iter().enumerate().map(|(ix, label)| {
                let id = label.id;
                let checked = self.selected_labels.contains(&id);
                Toggle::new(("new-reminder-label", ix))
                    .ghost()
                    .small()
                    .px_0p5()
                    .checked(checked)
                    .tooltip(format!("Label {}", label.title))
                    .child(label.render(checked, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_label(id, cx)))
            }))
            .test_support()
            .into_any_element()
    }
}

/// A select's id for the item at `ix`.
fn index_id(ix: usize) -> i64 {
    i64::try_from(ix).unwrap_or(i64::MAX)
}

fn to_naive(at: DateTime) -> Option<NaiveDateTime> {
    NaiveDate::from_ymd_opt(
        at.year().into(),
        at.month().unsigned_abs().into(),
        at.day().unsigned_abs().into(),
    )?
    .and_hms_opt(
        at.hour().unsigned_abs().into(),
        at.minute().unsigned_abs().into(),
        0,
    )
}

fn from_naive(at: NaiveDateTime) -> Option<DateTime> {
    DateTime::new(
        i16::try_from(at.year()).ok()?,
        i8::try_from(at.month()).ok()?,
        i8::try_from(at.day()).ok()?,
        i8::try_from(at.hour()).ok()?,
        i8::try_from(at.minute()).ok()?,
        0,
        0,
    )
    .ok()
}

impl Render for NewReminderView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels = self.render_labels(cx);
        let theme = cx.theme();
        let error = self.error.clone().map(|message| {
            // `Alert` has no accessible name of its own; announce the message.
            div()
                .id("new-reminder-error")
                .role(Role::Alert)
                .aria_label(message.clone())
                .child(Alert::error("new-reminder-error-alert", message))
                .test_support()
        });
        v_flex()
            .id("new-reminder")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_create))
            .on_action(cx.listener(Self::on_cancel))
            .size_full()
            .gap_4()
            .p_6()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("New Reminder"),
            )
            .children(error)
            .child(
                div()
                    .id("new-reminder-fields")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_form()
                            .child(
                                field().label("Title").required(true).child(
                                    Input::new(&self.title)
                                        .id("new-reminder-title")
                                        .aria_label("Title"),
                                ),
                            )
                            .child(
                                field().label("Notes").child(
                                    Textarea::new(&self.body).h(px(88.)).aria_label("Notes"),
                                ),
                            )
                            .child(
                                field().label("Due").child(
                                    // The picker has no accessible name of its own.
                                    div()
                                        .id("new-reminder-due")
                                        .role(Role::Group)
                                        .aria_label("Due date")
                                        .child(
                                            DatePicker::new(&self.due)
                                                .placeholder("No due date")
                                                .cleanable(true),
                                        )
                                        .test_support(),
                                ),
                            )
                            .child(
                                field().label("Priority").child(
                                    Select::new(&self.priority)
                                        .id("new-reminder-priority")
                                        .accessibility_label("Priority"),
                                ),
                            )
                            .child(
                                field().label("Repeat").child(
                                    Select::new(&self.repeat)
                                        .id("new-reminder-repeat")
                                        .accessibility_label("Repeat"),
                                ),
                            )
                            .child(
                                field().label("Project").required(true).child(
                                    Select::new(&self.project)
                                        .id("new-reminder-project")
                                        .accessibility_label("Project")
                                        .placeholder(if self.loaded {
                                            "Choose a project"
                                        } else {
                                            "Loading…"
                                        })
                                        .disabled(!self.loaded),
                                ),
                            )
                            .child(field().label("Labels").child(labels)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("cancel-new-reminder")
                            .outline()
                            .label("Cancel")
                            .accessibility_label("Cancel")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .child(
                        Button::new("create-reminder")
                            .primary()
                            .label(if self.saving {
                                "Creating…"
                            } else {
                                "Create Reminder"
                            })
                            .accessibility_label("Create reminder")
                            .tooltip_with_action("Create reminder", &Create, Some(CONTEXT))
                            .loading(self.saving)
                            .disabled(self.saving || !self.loaded)
                            .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                    ),
            )
            .test_support()
    }
}

/// Open a new reminder window due at `now`.
pub fn open(
    options: WindowOptions,
    now: DateTime,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<NewReminderView>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| NewReminderView::new(now, window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::TestAppContext;
    use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
    use new_reminder::ProjectChoice;
    use vikunja::Label;

    use super::*;

    fn now() -> DateTime {
        "2026-11-16T09:30".parse().unwrap()
    }

    fn options() -> Options {
        Options {
            projects: vec![
                ProjectChoice {
                    id: 1,
                    title: "Home".into(),
                },
                ProjectChoice {
                    id: 4,
                    title: "Work".into(),
                },
            ],
            labels: vec![
                Label {
                    id: 3,
                    title: "Bills".into(),
                    hex_color: "ff0000".into(),
                },
                Label {
                    id: 5,
                    title: "Errands".into(),
                    hex_color: String::new(),
                },
            ],
            default_project_id: Some(4),
        }
    }

    #[gpui_kit::test]
    fn creates_a_reminder_with_defaults_and_chosen_labels(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), now(), cx).unwrap()
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = Rc::clone(&events);
            cx.subscribe(&view, move |_, event: &NewReminderEvent, _| {
                events.borrow_mut().push(event.clone());
            })
            .detach();
        });
        let has_label = |cx: &mut TestAppContext, label: &str| {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(label).is_some()
            })
            .unwrap()
        };

        // Defaults: due now, low priority, not repeating; no project until loaded.
        let draft = cx.update(|cx| view.read(cx).draft(cx));
        assert_eq!(draft.due, Some(now()));
        assert_eq!(draft.priority, Priority::Low);
        assert_eq!(draft.repeat, RepeatChoice::Never);
        assert_eq!(draft.project_id, None);
        assert!(has_label(cx, "Title") && has_label(cx, "Due date"));

        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| view.set_options(&options(), window, cx));
        })
        .unwrap();
        assert_eq!(cx.update(|cx| view.read(cx).draft(cx).project_id), Some(4));
        assert!(has_label(cx, "Label Bills") && has_label(cx, "Label Errands"));

        // Creating without a title says what's missing.
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("create-reminder", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(has_label(cx, "Enter a title."));
        assert!(events.borrow().is_empty());

        let title = SharedString::from("Pay rent");
        cx.update_window(handle, |_, window, cx| {
            let input = view.read(cx).title.clone();
            input.update(cx, |input, cx| input.set_value(title, window, cx));
            window.render_frame(cx);
            window.click(("new-reminder-label", 0_usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("secondary-enter", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        let events = events.borrow();
        let [NewReminderEvent::Create(draft)] = events.as_slice() else {
            panic!("expected one create, got {events:?}");
        };
        assert_eq!(draft.title, "Pay rent");
        assert_eq!(draft.label_ids, [3]);
        assert_eq!(draft.project_id, Some(4));
        assert_eq!(draft.due, Some(now()));
        drop(events);
        assert!(has_label(cx, "Create reminder"));
        assert!(cx.update(|cx| view.read(cx).saving));

        // A failed create shows why and allows another try.
        cx.update(|cx| view.update(cx, |view, cx| view.set_error("Server unavailable", cx)));
        assert!(has_label(cx, "Server unavailable"));
        assert!(!cx.update(|cx| view.read(cx).saving));
    }

    #[gpui_kit::test]
    fn escape_cancels(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), now(), cx).unwrap()
        });
        let cancelled = Rc::new(RefCell::new(false));
        cx.update(|cx| {
            let cancelled = Rc::clone(&cancelled);
            cx.subscribe(&view, move |_, event: &NewReminderEvent, _| {
                *cancelled.borrow_mut() = *event == NewReminderEvent::Cancel;
            })
            .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.key_down("escape", false, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(*cancelled.borrow());
    }

    #[gpui_kit::test]
    fn enter_in_notes_adds_a_line_instead_of_creating(cx: &mut TestAppContext) {
        let (handle, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            open(WindowOptions::default(), now(), cx).unwrap()
        });
        let created = Rc::new(RefCell::new(false));
        cx.update(|cx| {
            let created = Rc::clone(&created);
            cx.subscribe(&view, move |_, event: &NewReminderEvent, _| {
                *created.borrow_mut() |= matches!(event, NewReminderEvent::Create(_));
            })
            .detach();
        });
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.set_options(&options(), window, cx);
                view.title
                    .update(cx, |title, cx| title.set_value("Pay rent", window, cx));
            });
            let notes = view.read(cx).body.read(cx).focus_handle(cx);
            notes.focus(window, cx);
            window.render_frame(cx);
            window.input("a", cx);
            window.key_down("enter", false, cx);
            window.input("b", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!*created.borrow());
        assert_eq!(cx.update(|cx| view.read(cx).draft(cx).body), "a\nb");
    }

    #[test]
    fn converts_picker_times() {
        let naive = to_naive(now()).unwrap();
        assert_eq!(naive.to_string(), "2026-11-16 09:30:00");
        assert_eq!(from_naive(naive), Some(now()));
    }
}
