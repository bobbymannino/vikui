//! Everything worth showing about one reminder: the reminder itself, its task and the
//! task's other reminders. Built from data the caller already has; no fetching, no UI.

use jiff::Timestamp;
pub use reminders::Priority;
use reminders::Reminder;
use vikunja::{Repeat, Task};

/// A reminder together with the task it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReminderDetails {
    /// The reminder that was picked (or the task's entry, if it has no reminders).
    pub reminder: Reminder,
    /// The title of the task's project, when known.
    pub project: Option<String>,
    /// How the task, and so its reminders, repeats.
    pub repeat: Option<Repeat>,
    /// Plain text; empty when the task has no description.
    pub description: String,
    pub priority: Option<Priority>,
    pub due: Option<Timestamp>,
    pub start: Option<Timestamp>,
    pub end: Option<Timestamp>,
    /// Every reminder set on the task, earliest first, including `reminder`.
    pub reminders: Vec<Reminder>,
}

impl ReminderDetails {
    /// Details for `reminder`, which must belong to `task`; `project` is the title of the
    /// task's project, if the caller knows it.
    #[must_use]
    pub fn new(reminder: &Reminder, task: &Task, project: Option<&str>) -> Self {
        let reminders = reminders::from_tasks(std::slice::from_ref(task))
            .into_iter()
            .filter(|reminder| reminder.is_set)
            .collect();
        Self {
            reminder: reminder.clone(),
            project: project.map(str::to_owned),
            repeat: task.repeat(),
            description: task.plain_description(),
            priority: Priority::from_level(task.priority),
            due: task.due_date,
            start: task.start_date,
            end: task.end_date,
            reminders,
        }
    }

    /// The task's dates that are set, labelled, in due/start/end order.
    #[must_use]
    pub fn dates(&self) -> Vec<(&'static str, Timestamp)> {
        [("Due", self.due), ("Start", self.start), ("End", self.end)]
            .into_iter()
            .filter_map(|(label, at)| Some((label, at?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use vikunja::{RelativeReminder, RelativeTo, TaskReminder};

    use super::*;

    fn ts(value: &str) -> Timestamp {
        value.parse().unwrap()
    }

    #[test]
    fn collects_the_task_and_its_reminders() {
        let task = Task {
            id: 7,
            title: "Pay rent".into(),
            description: "<p>Transfer &amp; confirm</p>".into(),
            priority: 3,
            due_date: Some(ts("2026-12-01T09:00:00Z")),
            end_date: Some(ts("2026-12-02T09:00:00Z")),
            reminders: vec![
                TaskReminder {
                    at: Some(ts("2026-12-01T08:00:00Z")),
                    relative: Some(RelativeReminder {
                        period_secs: -3600,
                        to: RelativeTo::Due,
                    }),
                },
                TaskReminder {
                    at: Some(ts("2026-11-30T09:00:00Z")),
                    relative: None,
                },
            ],
            ..Task::default()
        };
        let all = reminders::from_tasks(std::slice::from_ref(&task));
        let details = ReminderDetails::new(&all[1], &task, Some("Home"));
        assert_eq!(details.reminder.at, Some(ts("2026-12-01T08:00:00Z")));
        assert_eq!(details.description, "Transfer & confirm");
        assert_eq!(details.project.as_deref(), Some("Home"));
        assert_eq!(details.repeat, None);
        assert_eq!(details.priority, Some(Priority::High));
        assert_eq!(
            details.dates(),
            [
                ("Due", ts("2026-12-01T09:00:00Z")),
                ("End", ts("2026-12-02T09:00:00Z"))
            ]
        );
        assert_eq!(details.reminders, all);
    }

    #[test]
    fn task_without_reminders_lists_none() {
        let task = Task {
            id: 1,
            title: "Buy milk".into(),
            ..Task::default()
        };
        let all = reminders::from_tasks(std::slice::from_ref(&task));
        let details = ReminderDetails::new(&all[0], &task, None);
        assert!(!details.reminder.is_set);
        assert_eq!(details.reminders, []);
        assert_eq!(details.priority, None);
        assert_eq!(details.project, None);
        assert_eq!(details.dates(), []);
    }

    #[test]
    fn carries_the_task_repeat() {
        let task = Task {
            id: 1,
            repeat_after: 86_400,
            ..Task::default()
        };
        let all = reminders::from_tasks(std::slice::from_ref(&task));
        let details = ReminderDetails::new(&all[0], &task, None);
        assert_eq!(
            details.repeat,
            Some(Repeat::Every {
                every_secs: 86_400,
                from_done: false
            })
        );
    }
}
