//! Task reminders, independent of how they were fetched or how they are shown.
//!
//! Callers fetch tasks (from a project, a filter or a search), turn them into
//! [`Reminder`]s with [`from_tasks`], and hand those to a view. Every task is listed,
//! whether or not it has a reminder set.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use vikunja::{Client, Error, RelativeReminder, RelativeTo, Repeat, Task, TaskQuery, Token};

/// One reminder, flattened out of its task, or a task that has no reminder set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    /// `None` for a task without a reminder, or for a relative reminder whose date
    /// (e.g. the due date) is unset; it will not fire until that date is set.
    pub at: Option<Timestamp>,
    pub relative: Option<RelativeReminder>,
    pub task_id: i64,
    pub task_title: String,
    /// e.g. `HOME-12`; may be empty.
    pub task_identifier: String,
    pub task_priority: Option<Priority>,
    pub task_due: Option<Timestamp>,
    /// The task's labels, in its order.
    pub task_labels: Vec<TaskLabel>,
    pub project_id: i64,
    pub task_done: bool,
    /// `false` for a task listed without any reminder set on it.
    pub is_set: bool,
}

impl Reminder {
    /// The task's identifier (`HOME-12`), or `#12` when it has none.
    #[must_use]
    pub fn task_reference(&self) -> String {
        if self.task_identifier.is_empty() {
            format!("#{}", self.task_id)
        } else {
            self.task_identifier.clone()
        }
    }

    /// The task's due date in `zone`, or empty when it has none.
    #[must_use]
    pub fn due(&self, zone: &TimeZone) -> String {
        self.task_due
            .map_or_else(String::new, |due| format_time(due, zone))
    }

    /// The task's labels as one line, e.g. "Bills, Home"; empty when it has none.
    #[must_use]
    pub fn labels(&self) -> String {
        self.task_labels
            .iter()
            .map(|label| label.title.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// "1 hour before due date", or `None` for reminders set at a fixed time.
    #[must_use]
    pub fn relative_description(&self) -> Option<String> {
        self.relative.map(describe_relative)
    }

    #[must_use]
    pub fn is_past(&self, now: Timestamp) -> bool {
        self.at.is_some_and(|at| at < now)
    }

    /// "Due date not set" for a relative reminder that has no time yet; "No reminder"
    /// for a task without one.
    #[must_use]
    pub fn unscheduled_reason(&self) -> Option<&'static str> {
        if self.at.is_some() {
            return None;
        }
        if !self.is_set {
            return Some("No reminder");
        }
        Some(match self.relative.map(|relative| relative.to) {
            Some(RelativeTo::Due) => "Due date not set",
            Some(RelativeTo::Start) => "Start date not set",
            Some(RelativeTo::End) => "End date not set",
            None => "No time set",
        })
    }
}

/// A label on a reminder's task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskLabel {
    pub title: String,
    /// `0xRRGGBB`, or `None` when the label has no colour.
    pub rgb: Option<u32>,
}

/// Vikunja's task priority levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
    DoNow,
}

impl Priority {
    /// `None` for `0` (unset) or levels Vikunja does not define.
    #[must_use]
    pub fn from_level(level: i64) -> Option<Self> {
        Some(match level {
            1 => Self::Low,
            2 => Self::Medium,
            3 => Self::High,
            4 => Self::Urgent,
            5 => Self::DoNow,
            _ => return None,
        })
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Urgent => "Urgent",
            Self::DoNow => "Do now",
        }
    }
}

/// "every 1 week", "every month", "every 2 days after it is done".
#[must_use]
pub fn describe_repeat(repeat: Repeat) -> String {
    match repeat {
        Repeat::Monthly => "every month".to_owned(),
        Repeat::Every {
            every_secs,
            from_done,
        } => {
            let mut text = format!("every {}", humanize(every_secs));
            if from_done {
                text.push_str(" after it is done");
            }
            text
        }
    }
}

/// Where a reminder stands, for its status label and whether to draw it muted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Its task is done.
    TaskDone,
    /// A task without any reminder that is not done.
    Open,
    /// A relative reminder whose date is unset.
    NotScheduled,
    Past,
    Upcoming,
}

impl Status {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::TaskDone => "Task done",
            Self::Open => "Open",
            Self::NotScheduled => "Not scheduled",
            Self::Past => "Past",
            Self::Upcoming => "Upcoming",
        }
    }

    /// Done, fired already or not scheduled; a plain open task is not inactive.
    #[must_use]
    pub fn is_inactive(self) -> bool {
        matches!(self, Self::TaskDone | Self::NotScheduled | Self::Past)
    }
}

impl Reminder {
    /// Its status at `now`.
    #[must_use]
    pub fn status(&self, now: Timestamp) -> Status {
        if self.task_done {
            Status::TaskDone
        } else if !self.is_set {
            Status::Open
        } else if self.at.is_none() {
            Status::NotScheduled
        } else if self.is_past(now) {
            Status::Past
        } else {
            Status::Upcoming
        }
    }

    /// "Fixed time", "1 hour before due date", or empty for a task without a reminder.
    #[must_use]
    pub fn schedule(&self) -> String {
        match self.relative_description() {
            Some(relative) => relative,
            None if self.is_set => "Fixed time".to_owned(),
            None => String::new(),
        }
    }

    /// The time it fires in `zone`, or why it has none (see
    /// [`Reminder::unscheduled_reason`]).
    #[must_use]
    pub fn when(&self, zone: &TimeZone) -> String {
        match self.at {
            Some(at) => format_time(at, zone),
            None => self.unscheduled_reason().unwrap_or_default().to_owned(),
        }
    }
}

/// "Tue 1 Dec 2026, 09:00" in `zone`.
#[must_use]
pub fn format_time(at: Timestamp, zone: &TimeZone) -> String {
    at.to_zoned_datetime(zone.clone())
        .strftime("%a %-d %b %Y, %H:%M")
        .to_string()
}

/// Every reminder on `tasks`, plus one entry per task that has none, earliest first;
/// entries without a time come last.
#[must_use]
pub fn from_tasks(tasks: &[Task]) -> Vec<Reminder> {
    let mut reminders: Vec<Reminder> = tasks
        .iter()
        .flat_map(|task| {
            let entry = |at, relative, is_set| Reminder {
                at,
                relative,
                task_id: task.id,
                task_title: task.title.clone(),
                task_identifier: task.identifier.clone(),
                task_priority: Priority::from_level(task.priority),
                task_due: task.due_date,
                task_labels: task
                    .labels
                    .iter()
                    .map(|label| TaskLabel {
                        title: label.title.clone(),
                        rgb: label.rgb(),
                    })
                    .collect(),
                project_id: task.project_id,
                task_done: task.done,
                is_set,
            };
            let set: Vec<Reminder> = task
                .reminders
                .iter()
                .map(|reminder| entry(reminder.at, reminder.relative, true))
                .collect();
            if set.is_empty() {
                vec![entry(None, None, false)]
            } else {
                set
            }
        })
        .collect();
    reminders.sort_by_key(|reminder| (reminder.at.is_none(), reminder.at, reminder.task_id));
    reminders
}

/// Where a list of reminders came from; gives the window its title and the tasks query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A project; `subproject_ids` (all levels below it) are included too.
    Project {
        id: i64,
        title: String,
        subproject_ids: Vec<i64>,
    },
    /// A saved filter; its stored query is fetched from the server.
    SavedFilter {
        filter_id: i64,
        title: String,
    },
    Filter(String),
    Search(String),
}

impl Source {
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::Project { title, .. } | Self::SavedFilter { title, .. } => {
                format!("Reminders – {title}")
            }
            Self::Filter(filter) => format!("Reminders – filter “{filter}”"),
            Self::Search(text) => format!("Reminders – search “{text}”"),
        }
    }

    /// Fetch the tasks this source covers. Blocks; run it on a background task.
    pub fn tasks(&self, client: &Client, token: &Token) -> Result<Vec<Task>, Error> {
        let query = match self.direct_query() {
            Some(query) => query,
            None => match self {
                Self::SavedFilter { filter_id, .. } => {
                    client.saved_filter(token, *filter_id)?.query()
                }
                _ => TaskQuery::default(),
            },
        };
        client.tasks(token, &query)
    }

    /// The query, when it is known without asking the server.
    fn direct_query(&self) -> Option<TaskQuery> {
        match self {
            Self::Project {
                id, subproject_ids, ..
            } => {
                let ids: Vec<i64> = std::iter::once(*id)
                    .chain(subproject_ids.iter().copied())
                    .collect();
                Some(TaskQuery::projects(&ids))
            }
            Self::SavedFilter { .. } => None,
            Self::Filter(filter) => Some(TaskQuery::filter(filter.clone())),
            Self::Search(text) => Some(TaskQuery::search(text.clone())),
        }
    }
}

fn describe_relative(relative: RelativeReminder) -> String {
    let date = match relative.to {
        RelativeTo::Due => "due date",
        RelativeTo::Start => "start date",
        RelativeTo::End => "end date",
    };
    if relative.period_secs == 0 {
        return format!("At {date}");
    }
    let direction = if relative.period_secs < 0 {
        "before"
    } else {
        "after"
    };
    format!(
        "{} {direction} {date}",
        humanize(relative.period_secs.unsigned_abs())
    )
}

/// `5400` → "1 hour 30 minutes"; at most two units, largest first.
fn humanize(secs: u64) -> String {
    const UNITS: [(u64, &str); 5] = [
        (7 * 86_400, "week"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
        (1, "second"),
    ];
    let mut rest = secs;
    let parts: Vec<String> = UNITS
        .iter()
        .filter_map(|&(size, name)| {
            let count = rest / size;
            rest %= size;
            (count > 0).then(|| format!("{count} {name}{}", if count == 1 { "" } else { "s" }))
        })
        .take(2)
        .collect();
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use vikunja::TaskReminder;

    use super::*;

    fn ts(value: &str) -> Timestamp {
        value.parse().unwrap()
    }

    fn task(id: i64, title: &str, reminders: Vec<TaskReminder>) -> Task {
        Task {
            id,
            title: title.into(),
            project_id: 1,
            reminders,
            ..Task::default()
        }
    }

    #[test]
    fn flattens_and_sorts_unscheduled_last() {
        let tasks = [
            task(
                1,
                "Later",
                vec![TaskReminder {
                    at: Some(ts("2026-12-02T09:00:00Z")),
                    relative: None,
                }],
            ),
            task(
                2,
                "Two reminders",
                vec![
                    TaskReminder {
                        at: Some(ts("2026-12-03T09:00:00Z")),
                        relative: None,
                    },
                    TaskReminder {
                        at: Some(ts("2026-12-01T09:00:00Z")),
                        relative: None,
                    },
                    TaskReminder {
                        at: None,
                        relative: Some(RelativeReminder {
                            period_secs: -60,
                            to: RelativeTo::Due,
                        }),
                    },
                ],
            ),
            task(3, "None", vec![]),
        ];
        let reminders = from_tasks(&tasks);
        let order: Vec<_> = reminders
            .iter()
            .map(|r| (r.task_title.as_str(), r.at))
            .collect();
        assert_eq!(
            order,
            [
                ("Two reminders", Some(ts("2026-12-01T09:00:00Z"))),
                ("Later", Some(ts("2026-12-02T09:00:00Z"))),
                ("Two reminders", Some(ts("2026-12-03T09:00:00Z"))),
                ("Two reminders", None),
                ("None", None),
            ]
        );
        let unscheduled = &reminders[3];
        assert_eq!(unscheduled.unscheduled_reason(), Some("Due date not set"));
        assert!(!unscheduled.is_past(ts("2030-01-01T00:00:00Z")));
        let without = &reminders[4];
        assert!(!without.is_set);
        assert_eq!(without.unscheduled_reason(), Some("No reminder"));

        let now = ts("2026-12-02T12:00:00Z");
        let statuses: Vec<_> = reminders.iter().map(|r| r.status(now)).collect();
        assert_eq!(
            statuses,
            [
                Status::Past,
                Status::Past,
                Status::Upcoming,
                Status::NotScheduled,
                Status::Open
            ]
        );
        assert!(!Status::Open.is_inactive());
        assert_eq!(reminders[0].when(&TimeZone::UTC), "Tue 1 Dec 2026, 09:00");
        assert_eq!(reminders[0].schedule(), "Fixed time");
        assert_eq!(without.when(&TimeZone::UTC), "No reminder");
        assert_eq!(without.schedule(), "");
    }

    #[test]
    fn carries_task_priority_labels_and_reference() {
        let tasks = [Task {
            priority: 4,
            labels: vec![
                vikunja::Label {
                    id: 1,
                    title: "Bills".into(),
                    hex_color: "ff0000".into(),
                },
                vikunja::Label {
                    id: 2,
                    title: "Home".into(),
                    hex_color: String::new(),
                },
            ],
            ..task(12, "Pay rent", vec![])
        }];
        let reminder = &from_tasks(&tasks)[0];
        assert_eq!(reminder.task_priority, Some(Priority::Urgent));
        assert_eq!(reminder.labels(), "Bills, Home");
        assert_eq!(reminder.task_labels[0].rgb, Some(0xff_00_00));
        assert_eq!(reminder.task_labels[1].rgb, None);
        assert_eq!(reminder.due(&TimeZone::UTC), "");
        let due = Reminder {
            task_due: Some(ts("2026-12-01T09:00:00Z")),
            ..reminder.clone()
        };
        assert_eq!(due.due(&TimeZone::UTC), "Tue 1 Dec 2026, 09:00");
        assert_eq!(reminder.task_reference(), "#12");
        let named = Reminder {
            task_identifier: "HOME-3".into(),
            ..reminder.clone()
        };
        assert_eq!(named.task_reference(), "HOME-3");
        assert_eq!(Priority::from_level(0), None);
    }

    #[test]
    fn describes_repeats() {
        let every = |every_secs, from_done| {
            describe_repeat(Repeat::Every {
                every_secs,
                from_done,
            })
        };
        assert_eq!(every(7 * 86_400, false), "every 1 week");
        assert_eq!(every(2 * 86_400, true), "every 2 days after it is done");
        assert_eq!(every(5_400, false), "every 1 hour 30 minutes");
        assert_eq!(describe_repeat(Repeat::Monthly), "every month");
    }

    #[test]
    fn describes_relative_reminders() {
        let describe = |period_secs, to| describe_relative(RelativeReminder { period_secs, to });
        assert_eq!(describe(0, RelativeTo::Due), "At due date");
        assert_eq!(describe(-3600, RelativeTo::Due), "1 hour before due date");
        assert_eq!(
            describe(5400, RelativeTo::Start),
            "1 hour 30 minutes after start date"
        );
        assert_eq!(
            describe(-2 * 86_400, RelativeTo::End),
            "2 days before end date"
        );
        assert_eq!(humanize(8 * 86_400 + 3_600 + 5), "1 week 1 day");
    }

    #[test]
    fn source_titles_and_queries() {
        let project = Source::Project {
            id: 4,
            title: "Home".into(),
            subproject_ids: vec![9, 12],
        };
        assert_eq!(project.title(), "Reminders – Home");
        assert_eq!(
            project.direct_query(),
            Some(TaskQuery::projects(&[4, 9, 12]))
        );
        assert_eq!(
            Source::Search("rent".into())
                .direct_query()
                .and_then(|query| query.search),
            Some("rent".to_owned())
        );
        let saved = Source::SavedFilter {
            filter_id: 2,
            title: "This week".into(),
        };
        assert_eq!(saved.title(), "Reminders – This week");
        assert_eq!(saved.direct_query(), None);
    }
}
