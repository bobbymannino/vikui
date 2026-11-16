//! A task to create, and its request body.

use jiff::Timestamp;
use serde_json::{Value, json};

use crate::task::{RelativeReminder, RelativeTo, Repeat};

/// A task for [`crate::Client::create_task`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewTask {
    pub title: String,
    /// Rich text (HTML), as Vikunja stores descriptions.
    pub description: String,
    pub due_date: Option<Timestamp>,
    /// `0` unset, then 1 (low) to 5 (do now).
    pub priority: i64,
    pub repeat: Option<Repeat>,
    pub reminders: Vec<RelativeReminder>,
    /// Labels to attach once the task exists.
    pub label_ids: Vec<i64>,
}

impl NewTask {
    /// The body of `PUT /projects/{id}/tasks`; labels are attached separately.
    pub(crate) fn to_json(&self) -> Value {
        let (repeat_after, repeat_mode) = match self.repeat {
            None => (0, 0),
            Some(Repeat::Monthly) => (0, 1),
            Some(Repeat::Every {
                every_secs,
                from_done,
            }) => (
                i64::try_from(every_secs).unwrap_or(i64::MAX),
                if from_done { 2 } else { 0 },
            ),
        };
        let reminders: Vec<Value> = self
            .reminders
            .iter()
            .map(|reminder| {
                json!({
                    "relative_to": reminder.to.as_str(),
                    "relative_period": reminder.period_secs,
                })
            })
            .collect();
        let mut body = json!({
            "title": self.title,
            "description": self.description,
            "priority": self.priority,
            "repeat_after": repeat_after,
            "repeat_mode": repeat_mode,
            "reminders": reminders,
        });
        if let (Some(due), Some(fields)) = (self.due_date, body.as_object_mut()) {
            fields.insert("due_date".to_owned(), Value::String(due.to_string()));
        }
        body
    }
}

impl RelativeTo {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Due => "due_date",
            Self::Start => "start_date",
            Self::End => "end_date",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_create_body() {
        let task = NewTask {
            title: "Pay rent".into(),
            description: "<p>Flat 2</p>".into(),
            due_date: Some("2026-12-01T09:00:00Z".parse().unwrap()),
            priority: 1,
            repeat: Some(Repeat::Every {
                every_secs: 604_800,
                from_done: false,
            }),
            reminders: vec![RelativeReminder {
                period_secs: 0,
                to: RelativeTo::Due,
            }],
            label_ids: vec![3],
        };
        assert_eq!(
            task.to_json(),
            json!({
                "title": "Pay rent",
                "description": "<p>Flat 2</p>",
                "due_date": "2026-12-01T09:00:00Z",
                "priority": 1,
                "repeat_after": 604_800,
                "repeat_mode": 0,
                "reminders": [{"relative_to": "due_date", "relative_period": 0}],
            })
        );
    }

    #[test]
    fn maps_repeat_modes_and_omits_unset_due_date() {
        let monthly = NewTask {
            repeat: Some(Repeat::Monthly),
            ..NewTask::default()
        };
        let body = monthly.to_json();
        assert_eq!(
            (body["repeat_after"].clone(), body["repeat_mode"].clone()),
            (json!(0), json!(1))
        );
        assert!(body.get("due_date").is_none());

        let from_done = NewTask {
            repeat: Some(Repeat::Every {
                every_secs: 86_400,
                from_done: true,
            }),
            ..NewTask::default()
        };
        assert_eq!(from_done.to_json()["repeat_mode"], json!(2));
    }
}
