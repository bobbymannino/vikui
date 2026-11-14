//! Task reminders, independent of how they were fetched or how they are shown.
//!
//! Callers fetch tasks (from a project, a filter or a search), turn them into
//! [`Reminder`]s with [`from_tasks`], and hand those to a view.

use jiff::Timestamp;
use vikunja::{Client, Error, RelativeReminder, RelativeTo, Task, TaskQuery, Token};

/// One reminder, flattened out of its task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub at: Timestamp,
    pub relative: Option<RelativeReminder>,
    pub task_id: i64,
    pub task_title: String,
    /// e.g. `HOME-12`; may be empty.
    pub task_identifier: String,
    pub project_id: i64,
    pub task_done: bool,
}

impl Reminder {
    /// "1 hour before due date", or `None` for reminders set at a fixed time.
    #[must_use]
    pub fn relative_description(&self) -> Option<String> {
        self.relative.map(describe_relative)
    }

    #[must_use]
    pub fn is_past(&self, now: Timestamp) -> bool {
        self.at < now
    }
}

/// Every reminder on `tasks` that has a time, earliest first.
///
/// Relative reminders whose date is unset (so they never fire) are skipped.
#[must_use]
pub fn from_tasks(tasks: &[Task]) -> Vec<Reminder> {
    let mut reminders: Vec<Reminder> = tasks
        .iter()
        .flat_map(|task| {
            task.reminders.iter().filter_map(move |reminder| {
                Some(Reminder {
                    at: reminder.at?,
                    relative: reminder.relative,
                    task_id: task.id,
                    task_title: task.title.clone(),
                    task_identifier: task.identifier.clone(),
                    project_id: task.project_id,
                    task_done: task.done,
                })
            })
        })
        .collect();
    reminders.sort_by_key(|reminder| (reminder.at, reminder.task_id));
    reminders
}

/// Where a list of reminders came from; gives the window its title and the tasks query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Project {
        id: i64,
        title: String,
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
            Self::Project { id, .. } => Some(TaskQuery::project(*id)),
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
            identifier: String::new(),
            done: false,
            due_date: None,
            start_date: None,
            end_date: None,
            reminders,
        }
    }

    #[test]
    fn flattens_sorts_and_skips_unscheduled() {
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
                ("Two reminders", ts("2026-12-01T09:00:00Z")),
                ("Later", ts("2026-12-02T09:00:00Z")),
                ("Two reminders", ts("2026-12-03T09:00:00Z")),
            ]
        );
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
        };
        assert_eq!(project.title(), "Reminders – Home");
        assert_eq!(project.direct_query(), Some(TaskQuery::project(4)));
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
