//! Table delegate listing reminders.

use std::cmp::Ordering;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use reminders::Reminder;

/// One row, formatted off the render path.
#[derive(Clone, Debug)]
pub struct ReminderRow {
    at: Timestamp,
    pub when: SharedString,
    pub task: SharedString,
    pub identifier: SharedString,
    pub schedule: SharedString,
    pub status: SharedString,
    /// Fired already, or its task is done: drawn muted (and says so in `status`).
    pub inactive: bool,
    /// Input order, used to undo sorting.
    order: usize,
}

impl ReminderRow {
    /// Format `reminders` for display in `zone`, judging past/upcoming against `now`.
    pub fn from_reminders(reminders: &[Reminder], now: Timestamp, zone: &TimeZone) -> Vec<Self> {
        reminders
            .iter()
            .enumerate()
            .map(|(order, reminder)| {
                let past = reminder.is_past(now);
                let status = if reminder.task_done {
                    "Task done"
                } else if past {
                    "Past"
                } else {
                    "Upcoming"
                };
                Self {
                    at: reminder.at,
                    when: reminder
                        .at
                        .to_zoned(zone.clone())
                        .strftime("%a %-d %b %Y, %H:%M")
                        .to_string()
                        .into(),
                    task: reminder.task_title.clone().into(),
                    identifier: reminder.task_identifier.clone().into(),
                    schedule: reminder
                        .relative_description()
                        .unwrap_or_else(|| "Fixed time".to_owned())
                        .into(),
                    status: status.into(),
                    inactive: past || reminder.task_done,
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

#[derive(Default)]
pub struct RemindersTable {
    rows: Vec<ReminderRow>,
    loading: bool,
    sort: Option<(Col, ColumnSort)>,
}

impl RemindersTable {
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_rows(&mut self, rows: Vec<ReminderRow>) {
        self.rows = rows;
        self.loading = false;
        self.apply_sort();
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
        a.at.cmp(&b.at)
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
            .size_full()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child("No reminders.")
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
            at: at.parse().unwrap(),
            relative: None,
            task_id: 1,
            task_title: title.into(),
            task_identifier: String::new(),
            project_id: 1,
            task_done: done,
        }
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
