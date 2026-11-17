use jiff::Timestamp;
use serde::{Deserialize, Deserializer};

/// Which tasks to fetch from `GET /tasks`. Both fields are optional and combine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskQuery {
    /// Vikunja filter syntax, e.g. `project in 3,4 && done = false`.
    pub filter: Option<String>,
    /// Free-text search over titles and descriptions.
    pub search: Option<String>,
    /// Also match tasks where a filtered field is unset (`filter_include_nulls`).
    pub include_nulls: bool,
}

impl TaskQuery {
    /// All tasks, done or not, in the given projects (e.g. a project and its subprojects).
    #[must_use]
    pub fn projects(project_ids: &[i64]) -> Self {
        let ids: Vec<String> = project_ids.iter().map(ToString::to_string).collect();
        Self {
            filter: Some(format!("project in {}", ids.join(","))),
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
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    /// Rich text (HTML); see [`Task::plain_description`].
    #[serde(default)]
    pub description: String,
    /// `0` unset, then 1 (low) to 5 (do now).
    #[serde(default)]
    pub priority: i64,
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
    /// `null` when the task has none.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub labels: Vec<Label>,
    /// Seconds between repeats; `0` when the task does not repeat by interval.
    #[serde(default)]
    pub repeat_after: i64,
    /// `0` repeats `repeat_after` from the task's dates, `1` monthly, `2` from when it
    /// was marked done. See [`Task::repeat`].
    #[serde(default)]
    pub repeat_mode: i64,
    /// `rrggbb`, or empty for none; see [`Task::rgb`].
    #[serde(default)]
    pub hex_color: String,
}

impl Task {
    /// The task's colour as `0xRRGGBB`, or `None` when it has none or it is malformed.
    #[must_use]
    pub fn rgb(&self) -> Option<u32> {
        parse_rgb(&self.hex_color)
    }

    /// The description as a single line of plain text, with HTML tags removed.
    #[must_use]
    pub fn plain_description(&self) -> String {
        crate::project::html_to_plain_text(&self.description)
    }

    /// How the task (and so its reminders) repeats, or `None` if it does not.
    #[must_use]
    pub fn repeat(&self) -> Option<Repeat> {
        let every_secs = u64::try_from(self.repeat_after)
            .ok()
            .filter(|&secs| secs > 0);
        match self.repeat_mode {
            1 => Some(Repeat::Monthly),
            2 => every_secs.map(|every_secs| Repeat::Every {
                every_secs,
                from_done: true,
            }),
            _ => every_secs.map(|every_secs| Repeat::Every {
                every_secs,
                from_done: false,
            }),
        }
    }
}

/// A label on a task.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Label {
    pub id: i64,
    pub title: String,
    /// e.g. `e8e8e8`, with or without a leading `#`; may be empty.
    #[serde(default)]
    pub hex_color: String,
}

impl Label {
    /// The label's colour as `0xRRGGBB`, or `None` when it has none or it is malformed.
    #[must_use]
    pub fn rgb(&self) -> Option<u32> {
        parse_rgb(&self.hex_color)
    }
}

/// A Vikunja colour (`rrggbb`, with or without `#`) as `0xRRGGBB`; `None` when it is
/// empty or malformed.
pub(crate) fn parse_rgb(hex_color: &str) -> Option<u32> {
    let hex = hex_color.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// How a task repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// Every `every_secs`, counted from the task's dates or, if `from_done`, from when it
    /// was marked done.
    Every { every_secs: u64, from_done: bool },
    /// On the same day every month.
    Monthly,
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
            "description": "<p>Transfer &amp; confirm</p>", "priority": 3,
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
        assert_eq!(task.plain_description(), "Transfer & confirm");
        assert_eq!(task.priority, 3);
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
    fn null_reminders_and_labels_mean_none() {
        let task: Task =
            serde_json::from_str(r#"{"id":1,"title":"x","reminders":null,"labels":null}"#).unwrap();
        assert_eq!(task.reminders, []);
        assert_eq!(task.labels, []);
        assert_eq!(task.repeat(), None);
    }

    #[test]
    fn deserializes_labels_and_repeats() {
        let json = r#"{"id":1,"title":"x","repeat_after":604800,"repeat_mode":0,
            "labels":[{"id":2,"title":"Bills","hex_color":"ff0000"}]}"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(
            task.labels,
            [Label {
                id: 2,
                title: "Bills".into(),
                hex_color: "ff0000".into(),
            }]
        );
        assert_eq!(task.labels[0].rgb(), Some(0xff_00_00));
        assert_eq!(task.rgb(), None);
        let coloured = Task {
            hex_color: "#00ff7f".into(),
            ..Task::default()
        };
        assert_eq!(coloured.rgb(), Some(0x00_ff_7f));
        let color = |hex: &str| {
            Label {
                hex_color: hex.into(),
                ..Label::default()
            }
            .rgb()
        };
        assert_eq!(color("#1973ff"), Some(0x19_73_ff));
        assert_eq!(color(""), None);
        assert_eq!(color("fff"), None);
        assert_eq!(color("zzzzzz"), None);
        assert_eq!(color("+12345"), None);
        assert_eq!(
            task.repeat(),
            Some(Repeat::Every {
                every_secs: 604_800,
                from_done: false
            })
        );
        let repeat = |repeat_after, repeat_mode| {
            Task {
                repeat_after,
                repeat_mode,
                ..Task::default()
            }
            .repeat()
        };
        assert_eq!(repeat(0, 1), Some(Repeat::Monthly));
        assert_eq!(
            repeat(86_400, 2),
            Some(Repeat::Every {
                every_secs: 86_400,
                from_done: true
            })
        );
        assert_eq!(repeat(0, 2), None);
        assert_eq!(repeat(-5, 0), None);
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
    fn projects_query_lists_every_project() {
        assert_eq!(
            TaskQuery::projects(&[5]).filter.as_deref(),
            Some("project in 5")
        );
        assert_eq!(
            TaskQuery::projects(&[5, 8, 13]).filter.as_deref(),
            Some("project in 5,8,13")
        );
    }
}
