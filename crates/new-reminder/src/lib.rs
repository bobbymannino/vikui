//! A new reminder: what the user fills in, its defaults, and the task it creates.
//!
//! A reminder is a Vikunja task with a reminder at its due date, so it fires when due.
//! No UI; the network calls block, so run them on a background task.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use reminders::Priority;
use vikunja::{
    Client, Error, Label, NewTask, Project, ProjectKind, RelativeReminder, RelativeTo, Repeat,
    Task, Token,
};

const DAY_SECS: u64 = 24 * 60 * 60;

/// How often a new reminder repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatChoice {
    #[default]
    Never,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl RepeatChoice {
    pub const ALL: [Self; 5] = [
        Self::Never,
        Self::Daily,
        Self::Weekly,
        Self::Monthly,
        Self::Yearly,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Never => "Doesn't repeat",
            Self::Daily => "Every day",
            Self::Weekly => "Every week",
            Self::Monthly => "Every month",
            Self::Yearly => "Every year",
        }
    }

    /// Vikunja's repeat. A year is 365 days, as Vikunja's own web app counts it.
    #[must_use]
    pub fn repeat(self) -> Option<Repeat> {
        let every = |days: u64| Repeat::Every {
            every_secs: days * DAY_SECS,
            from_done: false,
        };
        match self {
            Self::Never => None,
            Self::Daily => Some(every(1)),
            Self::Weekly => Some(every(7)),
            Self::Monthly => Some(Repeat::Monthly),
            Self::Yearly => Some(every(365)),
        }
    }
}

/// What the user has filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub title: String,
    /// Plain text; blank lines separate paragraphs.
    pub body: String,
    /// In the user's time zone; `None` for no due date (and so no reminder time).
    pub due: Option<DateTime>,
    pub priority: Priority,
    pub repeat: RepeatChoice,
    pub label_ids: Vec<i64>,
    pub project_id: Option<i64>,
}

impl Draft {
    /// The defaults: due now, low priority, not repeating, in `project_id` (the
    /// server's default project, see [`Options::default_project_id`]).
    #[must_use]
    pub fn new(now: DateTime, project_id: Option<i64>) -> Self {
        Self {
            title: String::new(),
            body: String::new(),
            due: Some(now),
            priority: Priority::Low,
            repeat: RepeatChoice::Never,
            label_ids: Vec::new(),
            project_id,
        }
    }

    /// The task to create and the project to create it in, with `due` read in `zone`.
    pub fn to_task(&self, zone: &TimeZone) -> Result<(i64, NewTask), Invalid> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(Invalid::MissingTitle);
        }
        let project_id = self.project_id.ok_or(Invalid::MissingProject)?;
        let due_date = self
            .due
            .map(|due| {
                due.to_zoned_datetime(zone.clone())
                    .map(|due| due.timestamp())
            })
            .transpose()
            .map_err(|_| Invalid::BadDueDate)?;
        let reminders = due_date
            .map(|_| RelativeReminder {
                period_secs: 0,
                to: RelativeTo::Due,
            })
            .into_iter()
            .collect();
        let task = NewTask {
            title: title.to_owned(),
            description: to_html(&self.body),
            due_date,
            priority: self.priority.level(),
            repeat: self.repeat.repeat(),
            reminders,
            label_ids: self.label_ids.clone(),
        };
        Ok((project_id, task))
    }
}

/// Why a draft can't be saved yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    MissingTitle,
    MissingProject,
    BadDueDate,
}

impl Invalid {
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::MissingTitle => "Enter a title.",
            Self::MissingProject => "Choose a project.",
            Self::BadDueDate => "That due date doesn't exist in your time zone.",
        }
    }
}

/// `text` as the HTML Vikunja stores: escaped, one `<p>` per paragraph, single line
/// breaks kept. Empty for blank text.
#[must_use]
pub fn to_html(text: &str) -> String {
    let escape = |line: &str| {
        line.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let normalized = text.replace("\r\n", "\n");
    let mut html = String::new();
    for paragraph in normalized.split("\n\n").map(str::trim) {
        if paragraph.is_empty() {
            continue;
        }
        let lines: Vec<String> = paragraph.lines().map(|line| escape(line.trim())).collect();
        html.push_str("<p>");
        html.push_str(&lines.join("<br>"));
        html.push_str("</p>");
    }
    html
}

/// A project a reminder can be created in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectChoice {
    pub id: i64,
    /// With its parents, e.g. "Home / Garden".
    pub title: String,
}

/// What the form offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub projects: Vec<ProjectChoice>,
    /// Sorted by title, ignoring case.
    pub labels: Vec<Label>,
    /// The server's default project if the user set one and it's listed, otherwise the
    /// first project; `None` only when there are no projects.
    pub default_project_id: Option<i64>,
}

impl Options {
    /// Build from the server's answers. `default_project_id` is the user's setting.
    #[must_use]
    pub fn new(
        projects: &[Project],
        mut labels: Vec<Label>,
        default_project_id: Option<i64>,
    ) -> Self {
        let projects = project_choices(projects);
        let default_project_id = default_project_id
            .filter(|id| projects.iter().any(|project| project.id == *id))
            .or_else(|| projects.first().map(|project| project.id));
        labels.sort_by_cached_key(|label| (label.title.to_lowercase(), label.id));
        labels.dedup_by_key(|label| label.id);
        Self {
            projects,
            labels,
            default_project_id,
        }
    }

    /// Fetch the projects, labels and default project.
    pub fn load(client: &Client, token: &Token) -> Result<Self, Error> {
        let projects = client.projects(token)?;
        let labels = client.labels(token)?;
        let default_project_id = client.default_project_id(token)?;
        Ok(Self::new(&projects, labels, default_project_id))
    }
}

/// Real, unarchived projects, each titled with its parents and sorted by that title.
fn project_choices(projects: &[Project]) -> Vec<ProjectChoice> {
    let real: Vec<&Project> = projects
        .iter()
        .filter(|project| project.kind() == ProjectKind::Project && !project.is_archived)
        .collect();
    let title_of = |project: &Project| {
        let mut parts = vec![project.title.as_str()];
        let mut parent = project.parent_id();
        // Bounded, in case the server reports a cycle.
        for _ in 0..real.len() {
            let Some(found) = parent.and_then(|id| real.iter().find(|p| p.id == id)) else {
                break;
            };
            parts.push(found.title.as_str());
            parent = found.parent_id();
        }
        parts.reverse();
        parts.join(" / ")
    };
    let mut choices: Vec<ProjectChoice> = real
        .iter()
        .map(|project| ProjectChoice {
            id: project.id,
            title: title_of(project),
        })
        .collect();
    choices.sort_by_cached_key(|choice| choice.title.to_lowercase());
    choices
}

/// Labels asked for in `label_ids` that `created` didn't end up with.
#[must_use]
pub fn missing_labels(label_ids: &[i64], created: &Task) -> Vec<i64> {
    label_ids
        .iter()
        .copied()
        .filter(|id| !created.labels.iter().any(|label| label.id == *id))
        .collect()
}

/// Create the task for a draft that [`Draft::to_task`] accepted.
pub fn create(
    client: &Client,
    token: &Token,
    project_id: i64,
    task: &NewTask,
) -> Result<Task, Error> {
    client.create_task(token, project_id, task)
}

/// Now in `zone`, to the minute, for [`Draft::new`].
#[must_use]
pub fn now_in(zone: &TimeZone) -> DateTime {
    let now = Timestamp::now().to_zoned_datetime(zone.clone()).datetime();
    now.with()
        .second(0)
        .subsec_nanosecond(0)
        .build()
        .unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().unwrap()
    }

    #[test]
    fn defaults_are_due_now_low_priority_in_the_default_project() {
        let now = at("2026-11-16T09:30");
        let draft = Draft::new(now, Some(4));
        assert_eq!(draft.due, Some(now));
        assert_eq!(draft.priority, Priority::Low);
        assert_eq!(draft.repeat, RepeatChoice::Never);
        assert_eq!(draft.project_id, Some(4));
    }

    #[test]
    fn builds_a_task_with_a_reminder_at_its_due_date() {
        let draft = Draft {
            title: "  Pay rent ".into(),
            body: "Flat 2\nRef <A&B>\n\nCall first".into(),
            priority: Priority::High,
            repeat: RepeatChoice::Weekly,
            label_ids: vec![3, 5],
            ..Draft::new(at("2026-12-01T09:00"), Some(4))
        };
        let (project, task) = draft.to_task(&TimeZone::UTC).unwrap();
        assert_eq!(project, 4);
        assert_eq!(task.title, "Pay rent");
        assert_eq!(
            task.description,
            "<p>Flat 2<br>Ref &lt;A&amp;B&gt;</p><p>Call first</p>"
        );
        assert_eq!(task.due_date, Some("2026-12-01T09:00:00Z".parse().unwrap()));
        assert_eq!(task.priority, 3);
        assert_eq!(
            task.repeat,
            Some(Repeat::Every {
                every_secs: 7 * DAY_SECS,
                from_done: false
            })
        );
        assert_eq!(
            task.reminders,
            [RelativeReminder {
                period_secs: 0,
                to: RelativeTo::Due
            }]
        );
        assert_eq!(task.label_ids, [3, 5]);
    }

    #[test]
    fn needs_a_title_and_project_and_skips_the_reminder_without_a_due_date() {
        let draft = Draft::new(at("2026-12-01T09:00"), Some(1));
        assert_eq!(draft.to_task(&TimeZone::UTC), Err(Invalid::MissingTitle));
        let draft = Draft {
            title: "x".into(),
            project_id: None,
            ..draft
        };
        assert_eq!(draft.to_task(&TimeZone::UTC), Err(Invalid::MissingProject));
        let draft = Draft {
            project_id: Some(1),
            due: None,
            ..draft
        };
        let (_, task) = draft.to_task(&TimeZone::UTC).unwrap();
        assert!(task.due_date.is_none() && task.reminders.is_empty());
        assert_eq!(task.description, "");
    }

    #[test]
    fn repeats_map_to_vikunja() {
        assert_eq!(RepeatChoice::Never.repeat(), None);
        assert_eq!(RepeatChoice::Monthly.repeat(), Some(Repeat::Monthly));
        assert_eq!(
            RepeatChoice::Yearly.repeat(),
            Some(Repeat::Every {
                every_secs: 365 * DAY_SECS,
                from_done: false
            })
        );
    }

    fn project(id: i64, title: &str, parent: i64) -> Project {
        Project {
            id,
            title: title.into(),
            description: String::new(),
            identifier: String::new(),
            parent_project_id: parent,
            is_archived: false,
            is_favorite: false,
        }
    }

    #[test]
    fn offers_real_projects_with_their_parents_and_picks_a_default() {
        let projects = [
            project(-1, "Favorites", 0),
            project(-3, "Saved filter", 0),
            project(1, "Home", 0),
            project(2, "Garden", 1),
            project(3, "Archived", 0),
            project(4, "Work", 0),
        ];
        let mut projects = projects.to_vec();
        projects[4].is_archived = true;
        let label = |id, title: &str| Label {
            id,
            title: title.into(),
            hex_color: String::new(),
        };
        let options = Options::new(
            &projects,
            vec![label(2, "work"), label(1, "Bills"), label(2, "work")],
            Some(4),
        );
        let titles: Vec<_> = options.projects.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Home", "Home / Garden", "Work"]);
        assert_eq!(options.default_project_id, Some(4));
        let labels: Vec<_> = options.labels.iter().map(|l| l.title.as_str()).collect();
        assert_eq!(labels, ["Bills", "work"]);

        // No (or an unlisted) default falls back to the first project.
        assert_eq!(
            Options::new(&projects, Vec::new(), None).default_project_id,
            Some(1)
        );
        assert_eq!(
            Options::new(&projects, Vec::new(), Some(3)).default_project_id,
            Some(1)
        );
        assert_eq!(
            Options::new(&[], Vec::new(), Some(3)).default_project_id,
            None
        );
    }

    #[test]
    fn reports_labels_that_were_not_attached() {
        let created = Task {
            labels: vec![Label {
                id: 3,
                ..Label::default()
            }],
            ..Task::default()
        };
        assert_eq!(missing_labels(&[3, 5], &created), [5]);
    }
}
