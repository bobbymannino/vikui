//! A table's layout: the order of its columns, their widths and the column its rows are
//! sorted by. [`Columns`] keeps it while the table is shown; [`TableLayout`] is what is
//! saved between runs. No UI.
//!
//! Columns are saved by a stable key rather than by position, so a saved layout still
//! fits after a version adds, removes or reorders columns.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// Saved widths outside this range (in logical pixels) come from a damaged file and are
/// replaced by the column's default.
const PLAUSIBLE_WIDTHS: RangeInclusive<u32> = 20..=4000;

/// A table's layout as saved. Fields missing from the file take their defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TableLayout {
    /// The columns in display order. Columns it doesn't list keep their default place
    /// after these.
    pub columns: Vec<ColumnLayout>,
    /// The column the rows are sorted by; `None` keeps the rows in their own order.
    pub sort: Option<Sort>,
}

/// One saved column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnLayout {
    /// The column's [`Column::key`].
    pub key: String,
    /// In logical pixels.
    pub width: u32,
}

/// A saved sort.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    /// The sorted column's [`Column::key`].
    pub column: String,
    pub direction: SortDirection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// One of a table's columns.
pub trait Column: Copy + Eq + 'static {
    /// Identifies the column in a [`TableLayout`]; keep it the same across versions.
    fn key(self) -> &'static str;
    /// In logical pixels.
    fn default_width(self) -> u32;
}

/// The live layout of a table whose columns are `C`.
#[derive(Clone, Debug)]
pub struct Columns<C: 'static> {
    /// Every column, in default order.
    defaults: &'static [C],
    /// Every column with its width, in display order.
    shown: Vec<(C, u32)>,
    sort: Option<(C, SortDirection)>,
    revision: u64,
}

impl<C: Column> Columns<C> {
    /// `defaults` lists every column once, in the order shown before any is moved.
    pub fn new(defaults: &'static [C]) -> Self {
        Self {
            defaults,
            shown: defaults
                .iter()
                .map(|&col| (col, col.default_width()))
                .collect(),
            sort: None,
            revision: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.shown.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// The column shown at `ix`.
    pub fn get(&self, ix: usize) -> Option<C> {
        self.shown.get(ix).map(|&(col, _)| col)
    }

    /// The width of the column shown at `ix`, in logical pixels.
    pub fn width(&self, ix: usize) -> Option<u32> {
        self.shown.get(ix).map(|&(_, width)| width)
    }

    pub fn sort(&self) -> Option<(C, SortDirection)> {
        self.sort
    }

    /// Changes with every move, resize and sort, but not with [`Self::apply`], so a
    /// watcher can tell when the user changed the layout and it needs saving.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Move the column shown at `from` so it ends up at `to`.
    pub fn move_column(&mut self, from: usize, to: usize) {
        if from == to || from >= self.len() || to >= self.len() {
            return;
        }
        let col = self.shown.remove(from);
        self.shown.insert(to, col);
        self.revision += 1;
    }

    /// Set every column's width, in display order. Widths beyond the last column are
    /// ignored.
    pub fn set_widths(&mut self, widths: impl IntoIterator<Item = u32>) {
        let mut changed = false;
        for ((_, width), new) in self.shown.iter_mut().zip(widths) {
            if *width != new {
                *width = new;
                changed = true;
            }
        }
        if changed {
            self.revision += 1;
        }
    }

    /// Sort by a column, or `None` for the rows' own order.
    pub fn set_sort(&mut self, sort: Option<(C, SortDirection)>) {
        if self.sort != sort {
            self.sort = sort;
            self.revision += 1;
        }
    }

    /// The layout to save.
    pub fn layout(&self) -> TableLayout {
        TableLayout {
            columns: self
                .shown
                .iter()
                .map(|&(col, width)| ColumnLayout {
                    key: col.key().to_owned(),
                    width,
                })
                .collect(),
            sort: self.sort.map(|(col, direction)| Sort {
                column: col.key().to_owned(),
                direction,
            }),
        }
    }

    /// Show the columns as `layout` saved them. Columns it doesn't list (added since it
    /// was saved) follow in default order at their default width. Keys no column has,
    /// repeated keys and implausible widths are ignored.
    pub fn apply(&mut self, layout: &TableLayout) {
        let defaults = self.defaults;
        let find = |key: &str| defaults.iter().copied().find(|col| col.key() == key);
        let mut shown: Vec<(C, u32)> = Vec::with_capacity(defaults.len());
        for saved in &layout.columns {
            if let Some(col) = find(&saved.key)
                && !shown.iter().any(|&(seen, _)| seen == col)
            {
                let width = if PLAUSIBLE_WIDTHS.contains(&saved.width) {
                    saved.width
                } else {
                    col.default_width()
                };
                shown.push((col, width));
            }
        }
        for &col in defaults {
            if !shown.iter().any(|&(seen, _)| seen == col) {
                shown.push((col, col.default_width()));
            }
        }
        self.shown = shown;
        self.sort = layout
            .sort
            .as_ref()
            .and_then(|sort| Some((find(&sort.column)?, sort.direction)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Col {
        Task,
        Due,
        Labels,
    }

    const COLUMNS: [Col; 3] = [Col::Task, Col::Due, Col::Labels];

    impl Column for Col {
        fn key(self) -> &'static str {
            match self {
                Self::Task => "task",
                Self::Due => "due",
                Self::Labels => "labels",
            }
        }

        fn default_width(self) -> u32 {
            match self {
                Self::Task => 300,
                Self::Due => 200,
                Self::Labels => 100,
            }
        }
    }

    fn order(columns: &Columns<Col>) -> Vec<(Col, u32)> {
        (0..columns.len())
            .map_while(|ix| Some((columns.get(ix)?, columns.width(ix)?)))
            .collect()
    }

    fn saved(columns: &[(&str, u32)], sort: Option<(&str, SortDirection)>) -> TableLayout {
        TableLayout {
            columns: columns
                .iter()
                .map(|&(key, width)| ColumnLayout {
                    key: key.to_owned(),
                    width,
                })
                .collect(),
            sort: sort.map(|(column, direction)| Sort {
                column: column.to_owned(),
                direction,
            }),
        }
    }

    #[test]
    fn starts_in_default_order_and_width() {
        let columns = Columns::new(&COLUMNS);
        assert_eq!(
            order(&columns),
            [(Col::Task, 300), (Col::Due, 200), (Col::Labels, 100)]
        );
        assert_eq!(columns.sort(), None);
        assert_eq!(columns.get(3), None);
    }

    #[test]
    fn moves_resizes_and_sorts_bumping_the_revision() {
        let mut columns = Columns::new(&COLUMNS);
        columns.move_column(2, 0);
        assert_eq!(columns.revision(), 1);
        assert_eq!(
            order(&columns),
            [(Col::Labels, 100), (Col::Task, 300), (Col::Due, 200)]
        );

        columns.set_widths([120, 300, 250]);
        assert_eq!(columns.revision(), 2);
        assert_eq!(columns.width(2), Some(250));

        columns.set_sort(Some((Col::Due, SortDirection::Descending)));
        assert_eq!(columns.revision(), 3);
        assert_eq!(
            columns.layout(),
            saved(
                &[("labels", 120), ("task", 300), ("due", 250)],
                Some(("due", SortDirection::Descending))
            )
        );
    }

    #[test]
    fn no_op_changes_keep_the_revision() {
        let mut columns = Columns::new(&COLUMNS);
        columns.move_column(1, 1);
        columns.move_column(0, 9);
        columns.set_widths([300, 200, 100]);
        columns.set_sort(None);
        assert_eq!(columns.revision(), 0);
    }

    #[test]
    fn applies_a_saved_layout_without_bumping_the_revision() {
        let mut columns = Columns::new(&COLUMNS);
        let layout = saved(
            &[("due", 180), ("labels", 90), ("task", 400)],
            Some(("task", SortDirection::Ascending)),
        );
        columns.apply(&layout);
        assert_eq!(
            order(&columns),
            [(Col::Due, 180), (Col::Labels, 90), (Col::Task, 400)]
        );
        assert_eq!(columns.sort(), Some((Col::Task, SortDirection::Ascending)));
        assert_eq!(columns.revision(), 0);
        assert_eq!(columns.layout(), layout);
    }

    #[test]
    fn a_stale_or_damaged_layout_falls_back_to_defaults() {
        let mut columns = Columns::new(&COLUMNS);
        columns.apply(&saved(
            &[("labels", 0), ("gone", 50), ("labels", 70), ("due", 150)],
            Some(("gone", SortDirection::Ascending)),
        ));
        assert_eq!(
            order(&columns),
            [(Col::Labels, 100), (Col::Due, 150), (Col::Task, 300)],
            "unknown and repeated keys are skipped, a 0 width is reset, and the missing \
             column follows"
        );
        assert_eq!(columns.sort(), None);
    }

    #[test]
    fn layouts_round_trip_through_json() {
        let layout = saved(&[("task", 300)], Some(("task", SortDirection::Descending)));
        let json = serde_json::to_string(&layout).unwrap();
        assert_eq!(
            json,
            r#"{"columns":[{"key":"task","width":300}],"sort":{"column":"task","direction":"descending"}}"#
        );
        assert_eq!(serde_json::from_str::<TableLayout>(&json).unwrap(), layout);
        assert_eq!(
            serde_json::from_str::<TableLayout>("{}").unwrap(),
            TableLayout::default()
        );
    }
}
