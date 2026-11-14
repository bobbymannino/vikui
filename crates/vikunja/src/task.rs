use jiff::Timestamp;
use serde::{Deserialize, Deserializer};

/// Which tasks to fetch from `GET /tasks`. Both fields are optional and combine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskQuery {
    /// Vikunja filter syntax, e.g. `project = 3 && done = false`.
    pub filter: Option<String>,
    /// Free-text search over titles and descriptions.
    pub search: Option<String>,
    /// Also match tasks where a filtered field is unset (`filter_include_nulls`).
    pub include_nulls: bool,
}

impl TaskQuery {
    /// Open (not done) tasks in one project.
    #[must_use]
    pub fn project(project_id: i64) -> Self {
        Self {
            filter: Some(format!("project = {project_id} && done = false")),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn filter(filter: impl Into<String>) -> Self {
        Self {
            filter: Some(filter.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn search(text: impl Into<String>) -> Self {
        Self {
            search: Some(text.into()),
            ..Self::default()
        }
    }
}

/// A saved filter (`GET /filters/{id}`): a named, stored task query.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct SavedFilter {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub filters: SavedQuery,
}

/// The stored query of a [`SavedFilter`] (Vikunja's `TaskCollection`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct SavedQuery {
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub s: String,
    #[serde(default)]
    pub filter_include_nulls: bool,
}

impl SavedFilter {
    /// The tasks query this filter stands for.
    #[must_use]
    pub fn query(&self) -> TaskQuery {
        let non_empty = |value: &str| (!value.trim().is_empty()).then(|| value.to_owned());
        TaskQuery {
            filter: non_empty(&self.filters.filter),
            search: non_empty(&self.filters.s),
            include_nulls: self.filters.filter_include_nulls,
        }
    }
}

/// A Vikunja task, with the fields Vikui uses.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub project_id: i64,
    /// Human-readable reference such as `HOME-12`; may be empty.
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default, deserialize_with = "optional_time")]
    pub due_date: Option<Timestamp>,
    #[serde(default, deserialize_with = "optional_time")]
    pub start_date: Option<Timestamp>,
    #[serde(default, deserialize_with = "optional_time")]
    pub end_date: Option<Timestamp>,
    /// `null` when the task has none.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub reminders: Vec<TaskReminder>,
}

/// One reminder on a task.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "RawReminder")]
pub struct TaskReminder {
    /// When the reminder fires. For relative reminders the server computes this from
    /// the date it is relative to; `None` if that date is unset.
    pub at: Option<Timestamp>,
    pub relative: Option<RelativeReminder>,
}

/// A reminder defined as an offset from one of the task's dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelativeReminder {
    /// Seconds from the date; negative means before it.
    pub period_secs: i64,
    pub to: RelativeTo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The task date a relative reminder is measured from.
pub enum RelativeTo {
    Due,
    Start,
    End,
}

impl RelativeTo {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "due_date" => Some(Self::Due),
            "start_date" => Some(Self::Start),
            "end_date" => Some(Self::End),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct RawReminder {
    #[serde(default, deserialize_with = "optional_time")]
    reminder: Option<Timestamp>,
    #[serde(default)]
    relative_period: i64,
    #[serde(default)]
    relative_to: Option<String>,
}

impl From<RawReminder> for TaskReminder {
    fn from(raw: RawReminder) -> Self {
        let relative = raw
            .relative_to
            .as_deref()
            .and_then(RelativeTo::parse)
            .map(|to| RelativeReminder {
                period_secs: raw.relative_period,
                to,
            });
        Self {
            at: raw.reminder,
            relative,
        }
    }
}

/// Vikunja sends unset dates as `null` or as the zero time `0001-01-01T00:00:00Z`.
fn optional_time<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Timestamp>, D::Error> {
    let value: Option<String> = Option::deserialize(deserializer)?;
    Ok(value
        .filter(|value| !value.starts_with("0001-01-01"))
        .and_then(|value| value.parse().ok()))
}

fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_tasks_with_absolute_and_relative_reminders() {
        let json = r#"{
            "id": 7, "title": "Pay rent", "project_id": 3, "identifier": "HOME-7",
            "done": false,
            "due_date": "2026-12-01T09:00:00Z",
            "start_date": "0001-01-01T00:00:00Z",
            "end_date": null,
            "reminders": [
                {"reminder": "2026-11-30T09:00:00Z", "relative_period": 0, "relative_to": ""},
                {"reminder": "2026-12-01T08:00:00Z", "relative_period": -3600,
                 "relative_to": "due_date"}
            ]
        }"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.due_date, Some("2026-12-01T09:00:00Z".parse().unwrap()));
        assert_eq!(task.start_date, None);
        assert_eq!(task.end_date, None);
        assert_eq!(task.reminders.len(), 2);
        assert_eq!(task.reminders[0].relative, None);
        assert_eq!(
            task.reminders[1].relative,
            Some(RelativeReminder {
                period_secs: -3600,
                to: RelativeTo::Due
            })
        );
    }

    #[test]
    fn null_reminders_mean_none() {
        let task: Task = serde_json::from_str(r#"{"id":1,"title":"x","reminders":null}"#).unwrap();
        assert_eq!(task.reminders, []);
    }

    #[test]
    fn saved_filter_becomes_a_task_query() {
        let json = r#"{"id":3,"title":"Due this week","filters":{
            "filter":"due_date < now/w+1w && done = false","s":"",
            "filter_include_nulls":true,"sort_by":["due_date"]}}"#;
        let filter: SavedFilter = serde_json::from_str(json).unwrap();
        assert_eq!(
            filter.query(),
            TaskQuery {
                filter: Some("due_date < now/w+1w && done = false".into()),
                search: None,
                include_nulls: true,
            }
        );
    }

    #[test]
    fn project_query_filters_open_tasks() {
        assert_eq!(
            TaskQuery::project(5).filter.as_deref(),
            Some("project = 5 && done = false")
        );
    }
}
