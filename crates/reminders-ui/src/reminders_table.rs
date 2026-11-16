//! Table delegate listing reminders.

use std::cmp::Ordering;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, Context, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use reminders::{Priority, Reminder};

/// One row, formatted off the render path.
#[derive(Clone, Debug)]
pub struct ReminderRow {
    pub identifier: SharedString,
    pub task: SharedString,
    priority: Option<Priority>,
    pub priority_label: SharedString,
    due_at: Option<Timestamp>,
    pub due: SharedString,
    /// The labels as one line: the cell's accessible name and sort key.
    pub labels: SharedString,
    /// The labels as drawn, one tag each.
    label_tags: Vec<LabelTag>,
    /// The [`reminders::normalize`]d task title, for searching.
    pub search_key: String,
    /// Fired already, not scheduled, or its task is done: drawn muted (the details panel
    /// says which). Tasks without a reminder are not muted unless done.
    pub inactive: bool,
    /// A real reminder, not a task listed without one.
    pub is_set: bool,
    /// Index into the reminders the rows were made from; stable across sorting.
    pub order: usize,
}

impl ReminderRow {
    /// Format `reminders` for display in `zone`, judging past/upcoming against `now`.
    pub fn from_reminders(reminders: &[Reminder], now: Timestamp, zone: &TimeZone) -> Vec<Self> {
        reminders
            .iter()
            .enumerate()
            .map(|(order, reminder)| Self {
                identifier: reminder.task_reference().into(),
                task: reminder.task_title.clone().into(),
                priority: reminder.task_priority,
                priority_label: reminder.task_priority.map_or("", Priority::label).into(),
                due_at: reminder.task_due,
                due: reminder.due(zone).into(),
                labels: reminder.labels().into(),
                label_tags: reminder
                    .task_labels
                    .iter()
                    .map(|label| LabelTag {
                        title: label.title.clone().into(),
                        rgb: label.rgb,
                    })
                    .collect(),
                search_key: reminders::normalize(&reminder.task_title),
                inactive: reminder.status(now).is_inactive(),
                is_set: reminder.is_set,
                order,
            })
            .collect()
    }
}

/// One label, ready to draw as a tag.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LabelTag {
    title: SharedString,
    /// `0xRRGGBB`; `None` draws the theme's neutral tag.
    rgb: Option<u32>,
}

impl LabelTag {
    /// A faint fill of the label's colour with the colour as text, its lightness pushed
    /// away from the background so the text stays readable in light and dark themes.
    fn render(&self, cx: &App) -> Tag {
        let tag = match self.rgb {
            Some(rgb) => {
                let color: Hsla = gpui_kit::rgb(rgb).into();
                let fill = color.opacity(0.18);
                let text = Hsla {
                    l: if cx.theme().is_dark() {
                        color.l.max(0.72)
                    } else {
                        color.l.min(0.38)
                    },
                    ..color
                };
                Tag::custom(fill, text, fill)
            }
            None => Tag::secondary(),
        };
        tag.child(self.title.clone())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Col {
    Identifier,
    Task,
    Priority,
    Due,
    Labels,
}

const COLUMNS: [Col; 5] = [
    Col::Identifier,
    Col::Task,
    Col::Priority,
    Col::Due,
    Col::Labels,
];

impl Col {
    fn at(ix: usize) -> Option<Self> {
        COLUMNS.get(ix).copied()
    }

    fn text(self, row: &ReminderRow) -> &SharedString {
        match self {
            Self::Identifier => &row.identifier,
            Self::Task => &row.task,
            Self::Priority => &row.priority_label,
            Self::Due => &row.due,
            Self::Labels => &row.labels,
        }
    }
}

/// Called with the clicked row's index (in display order) on a single click; the table
/// reports double clicks itself, as `TableEvent::DoubleClickedRow`.
pub type OnRowClick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// What a row's context menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowCommand {
    OpenInWindow,
    OpenInBrowser,
}

/// Called with the row's index (in display order) and the command picked from its
/// context menu.
pub type OnRowCommand = Rc<dyn Fn(usize, RowCommand, &mut Window, &mut App)>;

pub struct RemindersTable {
    rows: Vec<ReminderRow>,
    on_row_click: Option<OnRowClick>,
    on_row_command: Option<OnRowCommand>,
    loading: bool,
    /// Shown when there are no rows.
    empty_message: SharedString,
    sort: Option<(Col, ColumnSort)>,
}

impl Default for RemindersTable {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            on_row_click: None,
            on_row_command: None,
            loading: false,
            empty_message: "No reminders.".into(),
            sort: None,
        }
    }
}

impl RemindersTable {
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_empty_message(&mut self, message: SharedString) {
        self.empty_message = message;
    }

    pub fn set_rows(&mut self, rows: Vec<ReminderRow>) {
        self.rows = rows;
        self.loading = false;
        self.apply_sort();
    }

    /// Runs on every left click on a row, in addition to the table selecting it.
    pub fn set_on_row_click(&mut self, on_row_click: OnRowClick) {
        self.on_row_click = Some(on_row_click);
    }

    /// Gives each row a context menu of [`RowCommand`]s.
    pub fn set_on_row_command(&mut self, on_row_command: OnRowCommand) {
        self.on_row_command = Some(on_row_command);
    }

    pub fn row(&self, row_ix: usize) -> Option<&ReminderRow> {
        self.rows.get(row_ix)
    }

    #[cfg(test)]
    fn rows(&self) -> &[ReminderRow] {
        &self.rows
    }

    fn cell(&self, row_ix: usize, col_ix: usize) -> Option<&SharedString> {
        Some(Col::at(col_ix)?.text(self.rows.get(row_ix)?))
    }

    fn apply_sort(&mut self) {
        match self.sort {
            Some((col, ColumnSort::Ascending)) => self.rows.sort_by(|a, b| compare(col, a, b)),
            Some((col, ColumnSort::Descending)) => self.rows.sort_by(|a, b| compare(col, b, a)),
            Some((_, ColumnSort::Default)) | None => self.rows.sort_by_key(|row| row.order),
        }
    }
}

fn compare(col: Col, a: &ReminderRow, b: &ReminderRow) -> Ordering {
    // Unset values after set ones; priority by level and due date chronologically,
    // not by the formatted text.
    let primary = match col {
        Col::Priority => {
            (a.priority.is_none(), a.priority).cmp(&(b.priority.is_none(), b.priority))
        }
        Col::Due => (a.due_at.is_none(), a.due_at).cmp(&(b.due_at.is_none(), b.due_at)),
        Col::Identifier | Col::Task | Col::Labels => {
            let (a_text, b_text) = (col.text(a), col.text(b));
            a_text
                .is_empty()
                .cmp(&b_text.is_empty())
                .then_with(|| a_text.to_lowercase().cmp(&b_text.to_lowercase()))
        }
    };
    primary.then_with(|| a.order.cmp(&b.order))
}

impl TableDelegate for RemindersTable {
    fn columns_count(&self, _: &App) -> usize {
        COLUMNS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let column = match Col::at(col_ix) {
            Some(Col::Identifier) => Column::new("identifier", "Task ID").width(110.),
            Some(Col::Task) => Column::new("task", "Task").width(340.),
            Some(Col::Priority) => Column::new("priority", "Priority").width(110.),
            Some(Col::Due) => Column::new("due", "Due").width(200.),
            Some(Col::Labels) => Column::new("labels", "Labels").width(240.),
            None => return Column::default(),
        };
        let sort = match self.sort {
            Some((col, sort)) if Col::at(col_ix) == Some(col) => sort,
            _ => ColumnSort::Default,
        };
        column.sort(sort)
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.sort = Col::at(col_ix).map(|col| (col, sort));
        self.apply_sort();
        cx.notify();
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div()
            .id(("row", row_ix))
            .when_some(self.on_row_click.clone(), |row, on_click| {
                row.on_click(move |event, window, cx| {
                    // A double click starts with a single one; later clicks don't toggle.
                    if event.click_count() == 1 {
                        on_click(row_ix, window, cx);
                    }
                })
            })
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let Some(on_command) = self.on_row_command.clone() else {
            return menu;
        };
        let item = |label: &'static str, icon: IconName, command: RowCommand| {
            let on_command = Rc::clone(&on_command);
            PopupMenuItem::new(label)
                .icon(icon)
                .on_click(move |_, window, cx| on_command(row_ix, command, window, cx))
        };
        menu.item(item(
            "Open in new window",
            IconName::ExternalLink,
            RowCommand::OpenInWindow,
        ))
        .item(item(
            "Open in browser",
            IconName::Globe,
            RowCommand::OpenInBrowser,
        ))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = self.rows.get(row_ix);
        let inactive = row.is_some_and(|row| row.inactive);
        if Col::at(col_ix) == Some(Col::Labels) {
            let tags = row.map_or(&[][..], |row| row.label_tags.as_slice());
            return h_flex()
                .w_full()
                .gap_1()
                .overflow_hidden()
                // Muted rows fade their tags rather than recolouring them.
                .when(inactive, |cell| cell.opacity(0.6))
                .children(tags.iter().map(|tag| tag.render(cx)))
                .into_any_element();
        }
        let text = self.cell(row_ix, col_ix).cloned().unwrap_or_default();
        div()
            .w_full()
            .truncate()
            .when(inactive, |cell| {
                cell.text_color(cx.theme().muted_foreground)
            })
            .child(text)
            .into_any_element()
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        self.cell(row_ix, col_ix).cloned()
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.cell(row_ix, col_ix)
            .map_or_else(String::new, ToString::to_string)
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        h_flex()
            .id("reminders-empty")
            .role(Role::Status)
            .aria_label(self.empty_message.clone())
            .size_full()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child(self.empty_message.clone())
            .test_support()
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reminder(at: &str, title: &str, done: bool) -> Reminder {
        Reminder {
            at: Some(at.parse().unwrap()),
            relative: None,
            task_id: 1,
            task_title: title.into(),
            task_identifier: String::new(),
            task_priority: None,
            task_due: None,
            task_labels: Vec::new(),
            project_id: 1,
            task_done: done,
            is_set: true,
        }
    }

    fn titles(table: &RemindersTable) -> Vec<&str> {
        table.rows().iter().map(|row| row.task.as_ref()).collect()
    }

    #[test]
    fn formats_the_task_columns() {
        let now: Timestamp = "2026-11-14T12:00:00Z".parse().unwrap();
        let rows = ReminderRow::from_reminders(
            &[
                Reminder {
                    task_identifier: "HOME-7".into(),
                    task_priority: Some(Priority::Urgent),
                    task_due: Some("2026-12-01T09:00:00Z".parse().unwrap()),
                    task_labels: vec![
                        reminders::TaskLabel {
                            id: 1,
                            title: "Bills".into(),
                            rgb: Some(0xff_00_00),
                        },
                        reminders::TaskLabel {
                            id: 2,
                            title: "Home".into(),
                            rgb: None,
                        },
                    ],
                    ..reminder("2026-12-01T08:00:00Z", "Pay rent", false)
                },
                Reminder {
                    task_id: 9,
                    ..reminder("2026-12-01T08:00:00Z", "Plain", false)
                },
            ],
            now,
            &TimeZone::UTC,
        );
        let cells = |row: &ReminderRow| {
            COLUMNS
                .iter()
                .map(|col| col.text(row).to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            cells(&rows[0]),
            [
                "HOME-7",
                "Pay rent",
                "Urgent",
                "Tue 1 Dec 2026, 09:00",
                "Bills, Home"
            ]
        );
        assert_eq!(cells(&rows[1]), ["#9", "Plain", "", "", ""]);
        assert_eq!(
            rows[0].label_tags,
            [
                LabelTag {
                    title: "Bills".into(),
                    rgb: Some(0xff_00_00)
                },
                LabelTag {
                    title: "Home".into(),
                    rgb: None
                },
            ]
        );
    }

    #[test]
    fn tasks_without_reminders_are_listed_unmuted() {
        let now: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        let without = |done| Reminder {
            at: None,
            relative: None,
            is_set: false,
            ..reminder("2026-01-02T00:00:00Z", "Plain task", done)
        };
        let rows =
            ReminderRow::from_reminders(&[without(false), without(true)], now, &TimeZone::UTC);
        assert!(!rows[0].inactive);
        assert!(!rows[0].is_set);
        assert!(rows[1].inactive);
    }

    #[test]
    fn past_unscheduled_and_done_reminders_are_muted() {
        let now: Timestamp = "2026-11-14T12:00:00Z".parse().unwrap();
        let unscheduled = Reminder {
            at: None,
            relative: Some(vikunja::RelativeReminder {
                period_secs: -3600,
                to: vikunja::RelativeTo::Due,
            }),
            ..reminder("2026-01-02T00:00:00Z", "No due date", false)
        };
        let rows = ReminderRow::from_reminders(
            &[
                reminder("2026-11-13T09:30:00Z", "Past one", false),
                reminder("2026-12-01T09:05:00Z", "Future one", false),
                reminder("2026-12-02T09:00:00Z", "Finished", true),
                unscheduled,
            ],
            now,
            &TimeZone::UTC,
        );
        let inactive: Vec<_> = rows.iter().map(|row| row.inactive).collect();
        // Past, upcoming, task done, not scheduled.
        assert_eq!(inactive, [true, false, true, true]);
    }

    #[test]
    fn priority_and_due_sort_by_value_with_unset_last() {
        let now: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        let task = |title, priority, due: Option<&str>| Reminder {
            task_priority: priority,
            task_due: due.map(|due| due.parse().unwrap()),
            ..reminder("2026-12-01T09:00:00Z", title, false)
        };
        let mut table = RemindersTable::default();
        table.set_rows(ReminderRow::from_reminders(
            &[
                // "Tue 1 Dec" sorts before "Wed 2 Sep" as text; chronologically it is later.
                task(
                    "December",
                    Some(Priority::Low),
                    Some("2026-12-01T09:00:00Z"),
                ),
                task("Unset", None, None),
                task(
                    "September",
                    Some(Priority::DoNow),
                    Some("2026-09-02T09:00:00Z"),
                ),
            ],
            now,
            &TimeZone::UTC,
        ));

        table.sort = Some((Col::Due, ColumnSort::Ascending));
        table.apply_sort();
        assert_eq!(titles(&table), ["September", "December", "Unset"]);

        // "Do now" sorts before "Low" as text; by level it is higher.
        table.sort = Some((Col::Priority, ColumnSort::Ascending));
        table.apply_sort();
        assert_eq!(titles(&table), ["December", "September", "Unset"]);

        table.sort = None;
        table.apply_sort();
        assert_eq!(titles(&table), ["December", "Unset", "September"]);
    }
}
