//! Task reminders, independent of how they were fetched or how they are shown.
//!
//! Callers fetch tasks (from a project, a filter or a search), turn them into
//! [`Reminder`]s with [`from_tasks`], and hand those to a view. Every task is listed,
//! whether or not it has a reminder set.

use std::collections::BTreeSet;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use vikunja::{
    Client, Error, Label, Project, ProjectKind, RelativeReminder, RelativeTo, Repeat, Task,
    TaskQuery, Token,
};

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
    /// The task's colour as `0xRRGGBB`, or its project's (see
    /// [`fill_project_colours`]); `None` when neither has one.
    pub task_rgb: Option<u32>,
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
    pub id: i64,
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
    /// Every level, lowest first.
    pub const ALL: [Self; 5] = [
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Urgent,
        Self::DoNow,
    ];

    /// The level Vikunja stores, 1 (low) to 5 (do now).
    #[must_use]
    pub fn level(self) -> i64 {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::Urgent => 4,
            Self::DoNow => 5,
        }
    }

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
                        id: label.id,
                        title: label.title.clone(),
                        rgb: label.rgb(),
                    })
                    .collect(),
                task_rgb: task.rgb(),
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

/// `text` lowercased with everything but `a-z` and `0-9` removed, so "Pay-Rent!" and
/// "pay rent" both become "payrent". Accented and non-Latin letters are removed too.
#[must_use]
pub fn normalize(text: &str) -> String {
    text.chars()
        .map(|c| c.to_ascii_lowercase())
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// Search text split into its words and its labels: `*name` (as in Vikunja's quick add)
/// or `*"two words"` asks for tasks with a label starting with that name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchTerms {
    /// The words that aren't labels, separated by single spaces.
    pub text: String,
    /// The label names asked for, as typed (without `*` or quotes); none are empty.
    pub labels: Vec<String>,
}

impl SearchTerms {
    #[must_use]
    pub fn parse(query: &str) -> Self {
        let mut words: Vec<&str> = Vec::new();
        let mut labels = Vec::new();
        let mut rest = query.trim_start();
        while !rest.is_empty() {
            let (term, after) = if let Some(quoted) = rest.strip_prefix("*\"") {
                let end = quoted.find('"').unwrap_or(quoted.len());
                let after = quoted.get(end + 1..).unwrap_or("");
                (Some(&quoted[..end]), after)
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let (word, after) = rest.split_at(end);
                let label = word.strip_prefix('*');
                if label.is_none() {
                    words.push(word);
                }
                (label, after)
            };
            if let Some(label) = term.map(str::trim).filter(|label| !label.is_empty()) {
                labels.push(label.to_owned());
            }
            rest = after.trim_start();
        }
        Self {
            text: words.join(" "),
            labels,
        }
    }

    /// Whether `title`, a label's, is one these terms ask for.
    #[must_use]
    pub fn is_label_wanted(&self, title: &str) -> bool {
        let title = normalize(title);
        self.labels
            .iter()
            .any(|label| title.starts_with(&normalize(label)))
    }
}

/// A search over reminder titles, ignoring case, spaces and punctuation, and over their
/// task's labels with `*label` terms (see [`SearchTerms`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    /// [`normalize`]d words; empty matches every title.
    query: String,
    /// [`normalize`]d label names; a task needs a label starting with each.
    labels: Vec<String>,
}

impl Search {
    #[must_use]
    pub fn new(query: &str) -> Self {
        let terms = SearchTerms::parse(query);
        Self {
            query: normalize(&terms.text),
            labels: terms
                .labels
                .iter()
                .map(|label| normalize(label))
                .filter(|label| !label.is_empty())
                .collect(),
        }
    }

    /// Whether the query has anything left to match on; "" and "!?" do not.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.query.is_empty() && self.labels.is_empty()
    }

    /// Whether `labels`, a task's, include one starting with each label asked for.
    #[must_use]
    pub fn matches_labels(&self, labels: &[TaskLabel]) -> bool {
        self.labels.iter().all(|wanted| {
            labels
                .iter()
                .any(|label| normalize(&label.title).starts_with(wanted.as_str()))
        })
    }

    /// Whether `key`, already [`normalize`]d, contains the query.
    #[must_use]
    pub fn matches_normalized(&self, key: &str) -> bool {
        key.contains(&self.query)
    }

    /// Whether `title` contains the query, both normalized.
    #[must_use]
    pub fn matches(&self, title: &str) -> bool {
        self.is_empty() || self.matches_normalized(&normalize(title))
    }
}

/// Which tasks to list by whether they are done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusFilter {
    #[default]
    All,
    NotDone,
    Done,
    /// Not done and due before now.
    Overdue,
}

impl StatusFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::NotDone, Self::Done, Self::Overdue];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NotDone => "Not done",
            Self::Done => "Done",
            Self::Overdue => "Overdue",
        }
    }

    /// Whether `reminder`'s task has this status at `now`.
    #[must_use]
    pub fn matches(self, reminder: &Reminder, now: Timestamp) -> bool {
        match self {
            Self::All => true,
            Self::NotDone => !reminder.task_done,
            Self::Done => reminder.task_done,
            Self::Overdue => !reminder.task_done && reminder.task_due.is_some_and(|due| due < now),
        }
    }
}

/// Which labels a reminder's task must all have; none selected matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelFilter {
    selected: BTreeSet<i64>,
}

impl LabelFilter {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    #[must_use]
    pub fn is_selected(&self, label_id: i64) -> bool {
        self.selected.contains(&label_id)
    }

    /// Select `label_id` if it isn't, or deselect it if it is.
    pub fn toggle(&mut self, label_id: i64) {
        if !self.selected.remove(&label_id) {
            self.selected.insert(label_id);
        }
    }

    pub fn clear(&mut self) {
        self.selected.clear();
    }

    /// Keep only the selected labels in `labels`, e.g. after a reload removed some, so a
    /// label that is no longer listed can't hide everything.
    pub fn retain_in(&mut self, labels: &[TaskLabel]) {
        self.selected
            .retain(|id| labels.iter().any(|label| label.id == *id));
    }

    /// Whether `reminder`'s task has every selected label.
    #[must_use]
    pub fn matches(&self, reminder: &Reminder) -> bool {
        self.selected
            .iter()
            .all(|id| reminder.task_labels.iter().any(|label| label.id == *id))
    }
}

/// Every label on `reminders`' tasks, once each, sorted by title ignoring case.
#[must_use]
pub fn labels_in(reminders: &[Reminder]) -> Vec<TaskLabel> {
    let mut labels: Vec<TaskLabel> = Vec::new();
    for label in reminders.iter().flat_map(|reminder| &reminder.task_labels) {
        if !labels.iter().any(|seen| seen.id == label.id) {
            labels.push(label.clone());
        }
    }
    labels.sort_by_cached_key(|label| (label.title.to_lowercase(), label.id));
    labels
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
    /// Tasks in every project matching the text and `*label` terms (see
    /// [`SearchTerms`]); a blank search lists nothing.
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
            Self::Search(_) if self.is_blank_search() => "Search all tasks".to_owned(),
            Self::Search(text) => format!("Search – “{text}”"),
        }
    }

    /// Fetch the tasks this source covers. Blocks; run it on a background task.
    ///
    /// A search with `*label` terms (see [`SearchTerms`]) lists only tasks with a label
    /// starting with each, and none if no label does.
    pub fn tasks(&self, client: &Client, token: &Token) -> Result<Vec<Task>, Error> {
        if self.is_blank_search() {
            return Ok(Vec::new());
        }
        let query = match self.direct_query() {
            Some(query) => query,
            None => match self {
                Self::SavedFilter { filter_id, .. } => {
                    client.saved_filter(token, *filter_id)?.query()
                }
                Self::Search(text) => {
                    let terms = SearchTerms::parse(text);
                    let Some(query) = label_search(&terms, &client.labels(token)?) else {
                        return Ok(Vec::new());
                    };
                    let mut tasks = client.tasks(token, &query)?;
                    tasks.retain(|task| has_wanted_labels(&terms, task));
                    return Ok(tasks);
                }
                _ => TaskQuery::default(),
            },
        };
        client.tasks(token, &query)
    }

    /// A search with nothing to search for, which lists nothing (rather than every task).
    fn is_blank_search(&self) -> bool {
        match self {
            Self::Search(text) => {
                let terms = SearchTerms::parse(text);
                terms.text.trim().is_empty() && terms.labels.is_empty()
            }
            _ => false,
        }
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
            Self::Search(text) => {
                let terms = SearchTerms::parse(text);
                terms
                    .labels
                    .is_empty()
                    .then(|| TaskQuery::search(terms.text))
            }
        }
    }
}

/// The query for a search with labels, given every label there is: its words, and any of
/// the labels asked for. `None` when some label asked for matches no label, so no task
/// can match.
fn label_search(terms: &SearchTerms, labels: &[Label]) -> Option<TaskQuery> {
    let mut ids = BTreeSet::new();
    for wanted in &terms.labels {
        let one = SearchTerms {
            labels: vec![wanted.clone()],
            ..SearchTerms::default()
        };
        let matching: Vec<i64> = labels
            .iter()
            .filter(|label| one.is_label_wanted(&label.title))
            .map(|label| label.id)
            .collect();
        if matching.is_empty() {
            return None;
        }
        ids.extend(matching);
    }
    let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
    Some(TaskQuery {
        filter: Some(format!("labels in {}", ids.join(","))),
        search: Some(terms.text.clone()).filter(|text| !text.is_empty()),
        ..TaskQuery::default()
    })
}

/// Whether `task` has a label starting with each one `terms` asks for (the server's
/// `labels in` matches any of them).
fn has_wanted_labels(terms: &SearchTerms, task: &Task) -> bool {
    terms.labels.iter().all(|wanted| {
        let one = SearchTerms {
            labels: vec![wanted.clone()],
            ..SearchTerms::default()
        };
        task.labels
            .iter()
            .any(|label| one.is_label_wanted(&label.title))
    })
}

/// Give each of `reminders` without a colour of its own its project's, from `projects`.
pub fn fill_project_colours(reminders: &mut [Reminder], projects: &[Project]) {
    for reminder in reminders.iter_mut().filter(|r| r.task_rgb.is_none()) {
        reminder.task_rgb = projects
            .iter()
            .find(|project| project.id == reminder.project_id)
            .and_then(Project::rgb);
    }
}

/// A project or saved filter to go to, e.g. from the menu bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    /// The title, after its parents' for a subproject: `Home › Bills`.
    pub name: String,
    pub source: Source,
}

/// Where `projects` (from `GET /projects`) can be opened: the projects in the server's
/// order, each with its subprojects, then the saved filters. Favorites is left out.
#[must_use]
pub fn destinations(projects: &[Project]) -> Vec<Destination> {
    let parent_of = |id: i64| {
        projects
            .iter()
            .find(|project| project.id == id)
            .and_then(Project::parent_id)
    };
    let title_of = |id: i64| {
        projects
            .iter()
            .find(|project| project.id == id)
            .map(|project| project.title.as_str())
    };
    let path = |project: &Project| {
        let mut names = vec![project.title.as_str()];
        let mut parent = project.parent_id();
        // The length check stops a malformed parent cycle from looping forever.
        while let Some(id) = parent
            && names.len() <= projects.len()
        {
            names.extend(title_of(id));
            parent = parent_of(id);
        }
        names.reverse();
        names.join(" › ")
    };
    let mut filters = Vec::new();
    let mut found: Vec<Destination> = Vec::new();
    for project in projects {
        match project.kind() {
            ProjectKind::Project => found.push(Destination {
                name: path(project),
                source: Source::Project {
                    id: project.id,
                    title: project.title.clone(),
                    subproject_ids: subproject_ids(projects, project.id),
                },
            }),
            ProjectKind::SavedFilter { filter_id } => filters.push(Destination {
                name: project.title.clone(),
                source: Source::SavedFilter {
                    filter_id,
                    title: project.title.clone(),
                },
            }),
            ProjectKind::Favorites => {}
        }
    }
    found.extend(filters);
    found
}

/// Every project below `project_id`, at any depth.
fn subproject_ids(projects: &[Project], project_id: i64) -> Vec<i64> {
    let mut found: Vec<i64> = Vec::new();
    let mut next = 0;
    let mut parent = Some(project_id);
    while let Some(current) = parent {
        for project in projects {
            // The `contains` check stops a malformed parent cycle from looping forever.
            if project.parent_id() == Some(current)
                && project.id != project_id
                && !found.contains(&project.id)
            {
                found.push(project.id);
            }
        }
        parent = found.get(next).copied();
        next += 1;
    }
    found
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
                Label {
                    id: 1,
                    title: "Bills".into(),
                    hex_color: "ff0000".into(),
                },
                Label {
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
    fn normalizes_to_lowercase_letters_and_digits() {
        assert_eq!(normalize("Pay-Rent! (Flat 2B)"), "payrentflat2b");
        assert_eq!(normalize("  "), "");
        assert_eq!(normalize("Café über"), "cafber");
    }

    #[test]
    fn searches_ignoring_case_spaces_and_punctuation() {
        let search = Search::new("pay rent");
        assert!(search.matches("Pay-Rent!"));
        assert!(search.matches("Remember to PAYRENT today"));
        assert!(!search.matches("Pay the bills"));
        assert!(Search::new("2b").matches("Flat 2-B"));

        let empty = Search::new("?!");
        assert!(empty.is_empty());
        assert!(!Search::new("*work").is_empty());
        assert!(empty.matches("anything"));
        assert!(Search::default().matches(""));
    }

    #[test]
    fn splits_labels_out_of_search_text() {
        let terms = SearchTerms::parse("  pay *Bills rent *\"Next week\" *  ");
        assert_eq!(terms.text, "pay rent");
        assert_eq!(terms.labels, ["Bills", "Next week"]);
        assert_eq!(SearchTerms::parse("*\"open").labels, ["open"]);
        assert_eq!(
            SearchTerms::parse("2*3"),
            SearchTerms {
                text: "2*3".into(),
                labels: Vec::new(),
            }
        );
        assert!(terms.is_label_wanted("bills-2026"));
        assert!(terms.is_label_wanted("NEXT WEEK"));
        assert!(!terms.is_label_wanted("Work"));
    }

    #[test]
    fn searches_by_label() {
        let label = |title: &str| TaskLabel {
            id: 1,
            title: title.into(),
            rgb: None,
        };
        let both = [label("Work"), label("Urgent!")];
        let search = Search::new("*work *urg");
        assert!(search.matches_labels(&both));
        assert!(!search.matches_labels(&both[..1]), "every label is needed");
        assert!(search.matches(""), "no words: any title");
        assert!(
            Search::new("rent").matches_labels(&[]),
            "no labels asked for"
        );
        assert!(!Search::new("*home").matches_labels(&both));
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
        let blank = Source::Search("  * ".into());
        assert!(blank.is_blank_search());
        assert_eq!(blank.title(), "Search all tasks");
        assert!(!Source::Search("*bills".into()).is_blank_search());
        assert_eq!(
            Source::Search("rent *bills".into()).direct_query(),
            None,
            "labels need the server's"
        );
        let saved = Source::SavedFilter {
            filter_id: 2,
            title: "This week".into(),
        };
        assert_eq!(saved.title(), "Reminders – This week");
        assert_eq!(saved.direct_query(), None);
    }

    #[test]
    fn searches_every_project_by_label() {
        let label = |id, title: &str| Label {
            id,
            title: title.into(),
            ..Label::default()
        };
        let labels = [label(1, "Bills"), label(2, "Bills 2026"), label(3, "Work")];
        let terms = SearchTerms::parse("rent *bills *work");
        let query = label_search(&terms, &labels).expect("every label matches");
        assert_eq!(query.filter.as_deref(), Some("labels in 1,2,3"));
        assert_eq!(query.search.as_deref(), Some("rent"));
        assert_eq!(
            label_search(&SearchTerms::parse("*work"), &labels).and_then(|q| q.search),
            None
        );
        assert_eq!(label_search(&SearchTerms::parse("*home"), &labels), None);

        let task = |ids: &[i64]| Task {
            labels: labels
                .iter()
                .filter(|l| ids.contains(&l.id))
                .cloned()
                .collect(),
            ..Task::default()
        };
        assert!(has_wanted_labels(&terms, &task(&[2, 3])));
        assert!(
            !has_wanted_labels(&terms, &task(&[1])),
            "needs a match for each"
        );
    }

    fn label(id: i64, title: &str) -> TaskLabel {
        TaskLabel {
            id,
            title: title.into(),
            rgb: None,
        }
    }

    fn labelled(labels: &[TaskLabel]) -> Reminder {
        let mut reminder = from_tasks(&[task(1, "Pay rent", Vec::new())]).remove(0);
        reminder.task_labels = labels.to_vec();
        reminder
    }

    #[test]
    fn tasks_without_a_colour_take_their_projects() {
        let coloured = Task {
            hex_color: "ff0000".into(),
            ..task(1, "Red", Vec::new())
        };
        let plain = task(2, "Plain", Vec::new());
        let elsewhere = Task {
            project_id: 9,
            ..task(3, "Elsewhere", Vec::new())
        };
        let mut reminders = from_tasks(&[coloured, plain, elsewhere]);
        let home = Project {
            id: 1,
            hex_color: "00ff00".into(),
            ..Project::default()
        };
        fill_project_colours(&mut reminders, &[home]);
        let colours: Vec<_> = reminders
            .iter()
            .map(|r| (r.task_title.as_str(), r.task_rgb))
            .collect();
        assert_eq!(
            colours,
            [
                ("Red", Some(0xff_00_00)),
                ("Plain", Some(0x00_ff_00)),
                ("Elsewhere", None)
            ]
        );
    }

    #[test]
    fn lists_projects_with_their_paths_then_saved_filters() {
        let project = |id: i64, title: &str, parent: i64| Project {
            id,
            title: title.into(),
            description: String::new(),
            identifier: String::new(),
            parent_project_id: parent,
            is_archived: false,
            is_favorite: false,
            hex_color: String::new(),
        };
        let projects = [
            project(-1, "Favorites", 0),
            project(-3, "This week", 0),
            project(1, "Home", 0),
            project(2, "Bills", 1),
            project(3, "Rent", 2),
            project(4, "Work", 0),
        ];
        let found = destinations(&projects);
        let names: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Home",
                "Home › Bills",
                "Home › Bills › Rent",
                "Work",
                "This week"
            ]
        );
        assert_eq!(
            found[0].source,
            Source::Project {
                id: 1,
                title: "Home".into(),
                subproject_ids: vec![2, 3],
            }
        );
        assert_eq!(
            found[4].source,
            Source::SavedFilter {
                filter_id: 2,
                title: "This week".into(),
            }
        );
    }

    #[test]
    fn status_filter_matches_done_and_overdue_tasks() {
        let now = ts("2026-12-01T12:00:00Z");
        let entry = |done: bool, due: Option<&str>| {
            let mut task = task(1, "Pay rent", Vec::new());
            task.done = done;
            task.due_date = due.map(ts);
            from_tasks(&[task]).remove(0)
        };
        let late = entry(false, Some("2026-12-01T11:59:00Z"));
        let later = entry(false, Some("2026-12-02T09:00:00Z"));
        let undated = entry(false, None);
        let done_late = entry(true, Some("2026-11-30T09:00:00Z"));
        let all = [&late, &later, &undated, &done_late];
        let matching = |filter: StatusFilter| -> Vec<bool> {
            all.iter().map(|r| filter.matches(r, now)).collect()
        };
        assert_eq!(StatusFilter::default(), StatusFilter::All);
        assert_eq!(matching(StatusFilter::All), [true, true, true, true]);
        assert_eq!(matching(StatusFilter::NotDone), [true, true, true, false]);
        assert_eq!(matching(StatusFilter::Done), [false, false, false, true]);
        assert_eq!(matching(StatusFilter::Overdue), [true, false, false, false]);
    }

    #[test]
    fn label_filter_needs_every_selected_label() {
        let (bills, home, work) = (label(1, "Bills"), label(2, "Home"), label(3, "Work"));
        let both = labelled(&[bills.clone(), home.clone()]);
        let only_bills = labelled(std::slice::from_ref(&bills));
        let none = labelled(&[]);

        let mut filter = LabelFilter::default();
        assert!(filter.is_empty());
        assert!(
            [&both, &only_bills, &none]
                .iter()
                .all(|r| filter.matches(r))
        );

        filter.toggle(bills.id);
        assert!(filter.is_selected(bills.id));
        assert!(filter.matches(&both) && filter.matches(&only_bills) && !filter.matches(&none));

        filter.toggle(home.id);
        assert!(filter.matches(&both));
        assert!(!filter.matches(&only_bills), "labels are ANDed");

        filter.toggle(home.id);
        assert!(!filter.is_selected(home.id), "toggling again deselects");
        assert!(filter.matches(&only_bills));

        filter.toggle(work.id);
        filter.retain_in(&[bills.clone(), home]);
        assert!(!filter.is_selected(work.id) && filter.is_selected(bills.id));
        filter.clear();
        assert!(filter.is_empty());
    }

    #[test]
    fn lists_each_label_once_by_title() {
        let reminders = [
            labelled(&[label(2, "home"), label(1, "Bills")]),
            labelled(&[label(1, "Bills"), label(3, "Work")]),
        ];
        let titles: Vec<_> = labels_in(&reminders)
            .into_iter()
            .map(|label| label.title)
            .collect();
        assert_eq!(titles, ["Bills", "home", "Work"]);
    }
}
