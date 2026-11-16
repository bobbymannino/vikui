//! Changes to an existing task, applied over the task as the server last sent it.

use jiff::Timestamp;
use serde_json::Value;

use crate::task::Repeat;

/// Whether to change a field that can also be unset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Change<T> {
    #[default]
    Keep,
    Set(T),
}

/// Fields to change on a task; `None` leaves a field as it is. See
/// [`crate::Client::update_task`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskUpdate {
    pub title: Option<String>,
    /// Rich text (HTML).
    pub description: Option<String>,
    /// `Set(None)` clears the due date.
    pub due_date: Change<Option<Timestamp>>,
    pub priority: Option<i64>,
    /// `Set(None)` stops the task repeating.
    pub repeat: Change<Option<Repeat>>,
    /// Moves the task to another project.
    pub project_id: Option<i64>,
    pub add_label_ids: Vec<i64>,
    pub remove_label_ids: Vec<i64>,
}

impl TaskUpdate {
    /// Whether nothing would change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether any field other than the labels changes.
    pub(crate) fn changes_fields(&self) -> bool {
        Self {
            add_label_ids: Vec::new(),
            remove_label_ids: Vec::new(),
            ..self.clone()
        } != Self::default()
    }

    /// Set the changed fields on `task`'s JSON, leaving every other field as it is.
    /// Vikunja's update replaces the whole task, so it is sent back in full.
    pub(crate) fn apply(&self, task: &mut Value) {
        let Some(fields) = task.as_object_mut() else {
            return;
        };
        let mut set = |name: &str, value: Value| {
            fields.insert(name.to_owned(), value);
        };
        if let Some(title) = &self.title {
            set("title", Value::String(title.clone()));
        }
        if let Some(description) = &self.description {
            set("description", Value::String(description.clone()));
        }
        if let Change::Set(due) = self.due_date {
            // Vikunja reads its zero time as "no due date".
            let due = due.map_or_else(|| "0001-01-01T00:00:00Z".to_owned(), |due| due.to_string());
            set("due_date", Value::String(due));
        }
        if let Some(priority) = self.priority {
            set("priority", priority.into());
        }
        if let Change::Set(repeat) = self.repeat {
            let (after, mode) = repeat_fields(repeat);
            set("repeat_after", after.into());
            set("repeat_mode", mode.into());
        }
        if let Some(project_id) = self.project_id {
            set("project_id", project_id.into());
        }
        // Labels are changed through their own endpoints; don't send a stale list.
        fields.remove("labels");
    }
}

/// `(repeat_after, repeat_mode)` as Vikunja stores them.
pub(crate) fn repeat_fields(repeat: Option<Repeat>) -> (i64, i64) {
    match repeat {
        None => (0, 0),
        Some(Repeat::Monthly) => (0, 1),
        Some(Repeat::Every {
            every_secs,
            from_done,
        }) => (
            i64::try_from(every_secs).unwrap_or(i64::MAX),
            if from_done { 2 } else { 0 },
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn task() -> Value {
        json!({
            "id": 7, "title": "Pay rent", "description": "<p>Flat 2</p>", "done": false,
            "due_date": "2026-12-01T09:00:00Z", "priority": 1, "repeat_after": 0,
            "repeat_mode": 0, "project_id": 1, "reminders": [{"relative_to": "due_date"}],
            "labels": [{"id": 3}], "percent_done": 0.5,
        })
    }

    #[test]
    fn changes_only_the_given_fields() {
        let mut body = task();
        TaskUpdate {
            title: Some("Pay the rent".into()),
            priority: Some(4),
            repeat: Change::Set(Some(Repeat::Every {
                every_secs: 604_800,
                from_done: true,
            })),
            project_id: Some(2),
            ..TaskUpdate::default()
        }
        .apply(&mut body);
        assert_eq!(body["title"], "Pay the rent");
        assert_eq!(body["priority"], 4);
        assert_eq!(
            (body["repeat_after"].clone(), body["repeat_mode"].clone()),
            (json!(604_800), json!(2))
        );
        assert_eq!(body["project_id"], 2);
        // Untouched fields, including ones Vikui doesn't know, are sent back as they were.
        assert_eq!(body["description"], "<p>Flat 2</p>");
        assert_eq!(body["due_date"], "2026-12-01T09:00:00Z");
        assert_eq!(body["percent_done"], 0.5);
        assert_eq!(body["reminders"], json!([{"relative_to": "due_date"}]));
        assert!(body.get("labels").is_none());
    }

    #[test]
    fn clears_the_due_date_and_repeat() {
        let mut body = task();
        TaskUpdate {
            due_date: Change::Set(None),
            repeat: Change::Set(None),
            ..TaskUpdate::default()
        }
        .apply(&mut body);
        assert_eq!(body["due_date"], "0001-01-01T00:00:00Z");
        assert_eq!(
            (body["repeat_after"].clone(), body["repeat_mode"].clone()),
            (json!(0), json!(0))
        );
    }

    #[test]
    fn knows_when_it_is_empty() {
        assert!(TaskUpdate::default().is_empty());
        let labels_only = TaskUpdate {
            add_label_ids: vec![3],
            ..TaskUpdate::default()
        };
        assert!(!labels_only.is_empty() && !labels_only.changes_fields());
    }
}
