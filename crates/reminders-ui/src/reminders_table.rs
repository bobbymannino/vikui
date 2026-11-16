//! Table delegate listing reminders.

use std::cmp::Ordering;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use reminders::Reminder;

/// One row, formatted off the render path.
#[derive(Clone, Debug)]
pub struct ReminderRow {
    at: Option<Timestamp>,
    pub when: SharedString,
    pub task: SharedString,
    pub identifier: SharedString,
    pub schedule: SharedString,
    pub status: SharedString,
    /// Fired already, not scheduled, or its task is done: drawn muted (and says so in
    /// `status`). Tasks without a reminder are not muted unless done.
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
            .map(|(order, reminder)| {
                let status = reminder.status(now);
                Self {
                    at: reminder.at,
                    when: reminder.when(zone).into(),
                    task: reminder.task_title.clone().into(),
                    identifier: reminder.task_identifier.clone().into(),
                    schedule: reminder.schedule().into(),
                    status: status.label().into(),
                    inactive: status.is_inactive(),
                    is_set: reminder.is_set,
                    order,
                }
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Col {
    When,
    Task,
    Identifier,
    Schedule,
    Status,
}

const COLUMNS: [Col; 5] = [
    Col::When,
    Col::Task,
    Col::Identifier,
    Col::Schedule,
    Col::Status,
];

impl Col {
    fn at(ix: usize) -> Option<Self> {
        COLUMNS.get(ix).copied()
    }

    fn text(self, row: &ReminderRow) -> &SharedString {
        match self {
            Self::When => &row.when,
            Self::Task => &row.task,
            Self::Identifier => &row.identifier,
            Self::Schedule => &row.schedule,
            Self::Status => &row.status,
        }
    }
}

/// Called with the clicked row's index (in display order).
pub type OnRowClick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

pub struct RemindersTable {
    rows: Vec<ReminderRow>,
    on_row_click: Option<OnRowClick>,
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
    // Chronological, not by the formatted text.
    let primary = if col == Col::When {
        // Unscheduled reminders after scheduled ones.
        (a.at.is_none(), a.at).cmp(&(b.at.is_none(), b.at))
    } else {
        let (a_text, b_text) = (col.text(a), col.text(b));
        a_text
            .is_empty()
            .cmp(&b_text.is_empty())
            .then_with(|| a_text.to_lowercase().cmp(&b_text.to_lowercase()))
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
            Some(Col::When) => Column::new("when", "When").width(200.),
            Some(Col::Task) => Column::new("task", "Task").width(300.),
            Some(Col::Identifier) => Column::new("identifier", "Task ID").width(110.),
            Some(Col::Schedule) => Column::new("schedule", "Schedule").width(220.),
            Some(Col::Status) => Column::new("status", "Status").width(110.),
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
                row.on_click(move |_, window, cx| on_click(row_ix, window, cx))
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let text = self.cell(row_ix, col_ix).cloned().unwrap_or_default();
        let inactive = self.rows.get(row_ix).is_some_and(|row| row.inactive);
        div()
            .w_full()
            .truncate()
            .when(inactive, |cell| {
                cell.text_color(cx.theme().muted_foreground)
            })
            .child(text)
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
        assert_eq!(rows[0].when, "No reminder");
        assert_eq!(rows[0].schedule, "");
        assert_eq!(rows[0].status, "Open");
        assert!(!rows[0].inactive);
        assert!(!rows[0].is_set);
        assert_eq!(rows[1].status, "Task done");
        assert!(rows[1].inactive);
    }

    #[test]
    fn formats_time_status_and_schedule() {
        let now: Timestamp = "2026-11-14T12:00:00Z".parse().unwrap();
        let rows = ReminderRow::from_reminders(
            &[
                reminder("2026-11-13T09:30:00Z", "Past one", false),
                reminder("2026-12-01T09:05:00Z", "Future one", false),
                reminder("2026-12-02T09:00:00Z", "Finished", true),
            ],
            now,
            &TimeZone::UTC,
        );
        assert_eq!(rows[0].when, "Fri 13 Nov 2026, 09:30");
        assert_eq!(rows[0].status, "Past");
        assert!(rows[0].inactive);
        assert_eq!(rows[1].status, "Upcoming");
        assert!(!rows[1].inactive);
        assert_eq!(rows[1].schedule, "Fixed time");
        assert_eq!(rows[2].status, "Task done");
    }

    #[test]
    fn unscheduled_reminders_say_why_and_sort_last() {
        let now: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        let unscheduled = Reminder {
            at: None,
            relative: Some(vikunja::RelativeReminder {
                period_secs: -3600,
                to: vikunja::RelativeTo::Due,
            }),
            ..reminder("2026-01-02T00:00:00Z", "No due date", false)
        };
        let mut table = RemindersTable::default();
        table.set_rows(ReminderRow::from_reminders(
            &[
                unscheduled,
                reminder("2026-12-01T09:00:00Z", "Scheduled", false),
            ],
            now,
            &TimeZone::UTC,
        ));
        let row = &table.rows()[0];
        assert_eq!(row.when, "Due date not set");
        assert_eq!(row.status, "Not scheduled");
        assert!(row.inactive);

        table.sort = Some((Col::When, ColumnSort::Ascending));
        table.apply_sort();
        let titles: Vec<_> = table.rows().iter().map(|r| r.task.as_ref()).collect();
        assert_eq!(titles, ["Scheduled", "No due date"]);
    }

    #[test]
    fn when_sorts_chronologically() {
        let now: Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        let mut table = RemindersTable::default();
        // "Tue 1 Dec" sorts before "Wed 2 Sep" as text; chronologically it is later.
        table.set_rows(ReminderRow::from_reminders(
            &[
                reminder("2026-12-01T09:00:00Z", "December", false),
                reminder("2026-09-02T09:00:00Z", "September", false),
            ],
            now,
            &TimeZone::UTC,
        ));
        table.sort = Some((Col::When, ColumnSort::Ascending));
        table.apply_sort();
        let titles: Vec<_> = table.rows().iter().map(|r| r.task.as_ref()).collect();
        assert_eq!(titles, ["September", "December"]);
    }
}
