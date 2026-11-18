//! Table delegate listing the user's projects.

use std::cmp::Ordering;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, Context, IntoElement, ParentElement as _, Pixels, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use table_layout::{Columns, SortDirection, TableLayout};
use vikunja::{Project, ProjectKind};

/// One table row, prepared off the main thread so rendering only clones `SharedString`s.
#[derive(Clone, Debug)]
pub struct ProjectRow {
    pub id: i64,
    /// A real project or a saved filter; Favorites is never listed.
    pub kind: ProjectKind,
    pub title: SharedString,
    /// `0xRRGGBB`, drawn as a dot before the title; `None` for no colour.
    pub rgb: Option<u32>,
    pub kind_label: SharedString,
    /// To find a project's subprojects when it is opened.
    pub parent_id: Option<i64>,
    pub identifier: SharedString,
    pub description: SharedString,
    /// Position in the server's order, used to undo sorting.
    order: usize,
}

impl ProjectRow {
    /// Build rows from an API response: projects and saved filters. The built-in
    /// Favorites list is left out.
    pub fn from_projects(projects: &[Project]) -> Vec<Self> {
        projects
            .iter()
            .filter(|project| project.kind() != ProjectKind::Favorites)
            .enumerate()
            .map(|(order, project)| Self {
                id: project.id,
                kind: project.kind(),
                title: project.title.clone().into(),
                rgb: project.rgb(),
                kind_label: match project.kind() {
                    ProjectKind::SavedFilter { .. } => "Saved filter",
                    ProjectKind::Project | ProjectKind::Favorites => "Project",
                }
                .into(),
                parent_id: project.parent_id(),
                identifier: project.identifier.clone().into(),
                description: project.plain_description().into(),
                order,
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Col {
    Title,
    Kind,
    Identifier,
    Description,
}

const COLUMNS: [Col; 4] = [Col::Title, Col::Kind, Col::Identifier, Col::Description];

impl table_layout::Column for Col {
    fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Kind => "kind",
            Self::Identifier => "identifier",
            Self::Description => "description",
        }
    }

    fn default_width(self) -> u32 {
        match self {
            Self::Title => 260,
            Self::Kind => 120,
            Self::Identifier => 110,
            Self::Description => 420,
        }
    }
}

impl Col {
    fn name(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Kind => "Type",
            Self::Identifier => "Identifier",
            Self::Description => "Description",
        }
    }

    fn text(self, row: &ProjectRow) -> &SharedString {
        match self {
            Self::Title => &row.title,
            Self::Kind => &row.kind_label,
            Self::Identifier => &row.identifier,
            Self::Description => &row.description,
        }
    }
}

pub struct ProjectsTable {
    rows: Vec<ProjectRow>,
    loading: bool,
    /// Their order, widths and sort, as the user left them.
    columns: Columns<Col>,
}

impl Default for ProjectsTable {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            loading: false,
            columns: Columns::new(&COLUMNS),
        }
    }
}

impl ProjectsTable {
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_rows(&mut self, rows: Vec<ProjectRow>) {
        self.rows = rows;
        self.loading = false;
        self.apply_sort();
    }

    pub fn rows(&self) -> &[ProjectRow] {
        &self.rows
    }

    pub fn row(&self, row_ix: usize) -> Option<&ProjectRow> {
        self.rows.get(row_ix)
    }

    /// Every project below `project_id`, at any depth, in breadth-first order.
    pub fn subproject_ids(&self, project_id: i64) -> Vec<i64> {
        let mut found: Vec<i64> = Vec::new();
        let mut next = 0;
        let mut parent = Some(project_id);
        while let Some(current) = parent {
            for row in &self.rows {
                // The `contains` check stops a malformed parent cycle from looping forever.
                if row.parent_id == Some(current)
                    && row.id != project_id
                    && !found.contains(&row.id)
                {
                    found.push(row.id);
                }
            }
            parent = found.get(next).copied();
            next += 1;
        }
        found
    }

    /// The column order, widths and sort to save.
    pub fn layout(&self) -> TableLayout {
        self.columns.layout()
    }

    /// Changes whenever the user moves, resizes or sorts a column.
    pub fn layout_revision(&self) -> u64 {
        self.columns.revision()
    }

    /// Show the columns and sort as saved; refresh the table afterwards.
    pub fn set_layout(&mut self, layout: &TableLayout) {
        self.columns.apply(layout);
        self.apply_sort();
    }

    /// Keep the widths the user resized the columns to, in display order.
    pub fn set_widths(&mut self, widths: &[Pixels]) {
        self.columns
            .set_widths(widths.iter().map(|&width| whole_pixels(width)));
    }

    fn row_text(&self, row_ix: usize, col_ix: usize) -> Option<&SharedString> {
        Some(self.columns.get(col_ix)?.text(self.rows.get(row_ix)?))
    }

    fn apply_sort(&mut self) {
        match self.columns.sort() {
            Some((col, SortDirection::Ascending)) => self.rows.sort_by(|a, b| compare(col, a, b)),
            Some((col, SortDirection::Descending)) => {
                self.rows.sort_by(|a, b| compare(col, b, a));
            }
            None => self.rows.sort_by_key(|row| row.order),
        }
    }
}

/// Case-insensitive, with empty values last and server order breaking ties.
fn compare(col: Col, a: &ProjectRow, b: &ProjectRow) -> Ordering {
    let (a_text, b_text) = (col.text(a), col.text(b));
    a_text
        .is_empty()
        .cmp(&b_text.is_empty())
        .then_with(|| a_text.to_lowercase().cmp(&b_text.to_lowercase()))
        .then_with(|| a.order.cmp(&b.order))
}

fn column_sort(direction: SortDirection) -> ColumnSort {
    match direction {
        SortDirection::Ascending => ColumnSort::Ascending,
        SortDirection::Descending => ColumnSort::Descending,
    }
}

fn width_pixels(width: u32) -> Pixels {
    // Saved widths are well below 2^24, so they convert to `f32` exactly.
    #[expect(clippy::cast_precision_loss, reason = "column widths are small")]
    gpui_kit::px(width as f32)
}

/// A column width rounded to whole logical pixels, as saved.
fn whole_pixels(width: Pixels) -> u32 {
    let width = f32::from(width).round().max(0.);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded, non-negative and far below u32::MAX"
    )]
    let width = width as u32;
    width
}

impl TableDelegate for ProjectsTable {
    fn columns_count(&self, _: &App) -> usize {
        COLUMNS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let (Some(col), Some(width)) = (self.columns.get(col_ix), self.columns.width(col_ix))
        else {
            return Column::default();
        };
        let sort = match self.columns.sort() {
            Some((sorted, direction)) if sorted == col => column_sort(direction),
            _ => ColumnSort::Default,
        };
        Column::new(table_layout::Column::key(col), col.name())
            .width(width_pixels(width))
            .sort(sort)
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let direction = match sort {
            ColumnSort::Ascending => Some(SortDirection::Ascending),
            ColumnSort::Descending => Some(SortDirection::Descending),
            ColumnSort::Default => None,
        };
        let sort = self.columns.get(col_ix).zip(direction);
        self.columns.set_sort(sort);
        self.apply_sort();
        cx.notify();
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.columns.move_column(col_ix, to_ix);
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let text = self.row_text(row_ix, col_ix).cloned().unwrap_or_default();
        let col = self.columns.get(col_ix);
        let swatch = (col == Some(Col::Title)).then(|| {
            // Decoration only: the colour is the project's own, and the title names it.
            let dot = div().flex_none().size_2p5().rounded_full();
            match self.row(row_ix).and_then(|row| row.rgb) {
                Some(rgb) => dot.bg(gpui_kit::rgb(rgb)),
                // Keeps titles aligned with those that have a colour.
                None => dot,
            }
        });
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .when(col == Some(Col::Description), |cell| {
                cell.text_color(cx.theme().muted_foreground)
            })
            .children(swatch)
            .child(div().min_w_0().flex_1().truncate().child(text))
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        self.row_text(row_ix, col_ix).cloned()
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.row_text(row_ix, col_ix)
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
            .child("No projects yet.")
    }

    fn loading(&self, _: &App) -> bool {
        self.loading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: i64, title: &str, parent: i64) -> Project {
        Project {
            id,
            title: title.into(),
            description: String::new(),
            identifier: String::new(),
            parent_project_id: parent,
            is_archived: false,
            is_favorite: false,
            hex_color: String::new(),
        }
    }

    #[test]
    fn rows_include_saved_filters_but_not_favorites() {
        let rows = ProjectRow::from_projects(&[
            project(-1, "Favorites", 0),
            project(1, "Work", 0),
            project(-3, "Due this week", 0),
        ]);
        let listed: Vec<_> = rows
            .iter()
            .map(|r| (r.title.as_ref(), r.kind_label.as_ref(), r.kind))
            .collect();
        assert_eq!(
            listed,
            [
                ("Work", "Project", ProjectKind::Project),
                (
                    "Due this week",
                    "Saved filter",
                    ProjectKind::SavedFilter { filter_id: 2 }
                ),
            ]
        );
    }

    #[test]
    fn rows_skip_favorites_and_keep_parents() {
        let rows = ProjectRow::from_projects(&[
            project(-1, "Favorites", 0),
            project(1, "Work", 0),
            project(2, "Reports", 1),
            project(3, "Orphan", 99),
        ]);
        let titles: Vec<_> = rows.iter().map(|r| r.title.as_ref()).collect();
        assert_eq!(titles, ["Work", "Reports", "Orphan"]);
        let parents: Vec<_> = rows.iter().map(|r| r.parent_id).collect();
        assert_eq!(parents, [None, Some(1), Some(99)]);
    }

    #[test]
    fn rows_keep_project_colours() {
        let rows = ProjectRow::from_projects(&[
            Project {
                hex_color: "1973ff".into(),
                ..project(1, "Work", 0)
            },
            project(2, "Home", 0),
            Project {
                hex_color: "not a colour".into(),
                ..project(3, "Bills", 0)
            },
        ]);
        let colours: Vec<_> = rows.iter().map(|r| r.rgb).collect();
        assert_eq!(colours, [Some(0x19_73_ff), None, None]);
    }

    #[test]
    fn finds_subprojects_at_every_depth() {
        let mut table = ProjectsTable::default();
        table.set_rows(ProjectRow::from_projects(&[
            project(1, "Work", 0),
            project(2, "Reports", 1),
            project(3, "Q4", 2),
            project(4, "Home", 0),
            project(5, "Hiring", 1),
        ]));
        assert_eq!(table.subproject_ids(1), [2, 5, 3]);
        assert_eq!(table.subproject_ids(2), [3]);
        assert_eq!(table.subproject_ids(4), Vec::<i64>::new());
    }

    #[test]
    fn cells_follow_a_moved_column() {
        let mut table = ProjectsTable::default();
        table.set_rows(ProjectRow::from_projects(&[Project {
            identifier: "HOME".into(),
            ..project(1, "Home", 0)
        }]));
        // Drag Identifier (third) to the front.
        table.columns.move_column(2, 0);
        let cells: Vec<_> = (0..COLUMNS.len())
            .map(|col_ix| table.row_text(0, col_ix).map(ToString::to_string))
            .collect();
        assert_eq!(cells[0].as_deref(), Some("HOME"));
        assert_eq!(cells[1].as_deref(), Some("Home"));
    }

    #[test]
    fn sorting_cycles_back_to_server_order() {
        let mut table = ProjectsTable::default();
        table.set_rows(ProjectRow::from_projects(&[
            project(1, "beta", 0),
            project(2, "Alpha", 0),
            project(3, "gamma", 0),
        ]));
        let titles = |t: &ProjectsTable| -> Vec<String> {
            t.rows().iter().map(|r| r.title.to_string()).collect()
        };

        table
            .columns
            .set_sort(Some((Col::Title, SortDirection::Ascending)));
        table.apply_sort();
        assert_eq!(titles(&table), ["Alpha", "beta", "gamma"]);

        table
            .columns
            .set_sort(Some((Col::Title, SortDirection::Descending)));
        table.apply_sort();
        assert_eq!(titles(&table), ["gamma", "beta", "Alpha"]);

        table.columns.set_sort(None);
        table.apply_sort();
        assert_eq!(titles(&table), ["beta", "Alpha", "gamma"]);
    }

    #[test]
    fn empty_values_sort_last() {
        let mut table = ProjectsTable::default();
        let with_identifier = |id: i64, title: &str, identifier: &str| Project {
            identifier: identifier.into(),
            ..project(id, title, 0)
        };
        table.set_rows(ProjectRow::from_projects(&[
            with_identifier(1, "Work", ""),
            with_identifier(2, "Home", "HOME"),
            with_identifier(3, "Bills", ""),
        ]));
        table
            .columns
            .set_sort(Some((Col::Identifier, SortDirection::Ascending)));
        table.apply_sort();
        let titles: Vec<_> = table.rows().iter().map(|r| r.title.as_ref()).collect();
        assert_eq!(titles, ["Home", "Work", "Bills"]);
    }
}
