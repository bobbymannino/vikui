//! Table delegate listing the user's projects.

use std::cmp::Ordering;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use vikunja::{Project, ProjectKind};

/// One table row, prepared off the main thread so rendering only clones `SharedString`s.
#[derive(Clone, Debug)]
pub struct ProjectRow {
    pub id: i64,
    /// A real project or a saved filter; Favorites is never listed.
    pub kind: ProjectKind,
    pub title: SharedString,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Col {
    Title,
    Kind,
    Identifier,
    Description,
}

const COLUMNS: [Col; 4] = [Col::Title, Col::Kind, Col::Identifier, Col::Description];

impl Col {
    fn at(ix: usize) -> Option<Self> {
        COLUMNS.get(ix).copied()
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

#[derive(Default)]
pub struct ProjectsTable {
    rows: Vec<ProjectRow>,
    loading: bool,
    sort: Option<(Col, ColumnSort)>,
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

    fn row_text(&self, row_ix: usize, col_ix: usize) -> Option<&SharedString> {
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

/// Case-insensitive, with empty values last and server order breaking ties.
fn compare(col: Col, a: &ProjectRow, b: &ProjectRow) -> Ordering {
    let (a_text, b_text) = (col.text(a), col.text(b));
    a_text
        .is_empty()
        .cmp(&b_text.is_empty())
        .then_with(|| a_text.to_lowercase().cmp(&b_text.to_lowercase()))
        .then_with(|| a.order.cmp(&b.order))
}

impl TableDelegate for ProjectsTable {
    fn columns_count(&self, _: &App) -> usize {
        COLUMNS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let column = match Col::at(col_ix) {
            Some(Col::Title) => Column::new("title", "Title").width(260.),
            Some(Col::Kind) => Column::new("kind", "Type").width(120.),
            Some(Col::Identifier) => Column::new("identifier", "Identifier").width(110.),
            Some(Col::Description) => Column::new("description", "Description").width(420.),
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
        let text = self.row_text(row_ix, col_ix).cloned().unwrap_or_default();
        div()
            .w_full()
            .truncate()
            .when(Col::at(col_ix) == Some(Col::Description), |cell| {
                cell.text_color(cx.theme().muted_foreground)
            })
            .child(text)
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

        table.sort = Some((Col::Title, ColumnSort::Ascending));
        table.apply_sort();
        assert_eq!(titles(&table), ["Alpha", "beta", "gamma"]);

        table.sort = Some((Col::Title, ColumnSort::Descending));
        table.apply_sort();
        assert_eq!(titles(&table), ["gamma", "beta", "Alpha"]);

        table.sort = Some((Col::Title, ColumnSort::Default));
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
        table.sort = Some((Col::Identifier, ColumnSort::Ascending));
        table.apply_sort();
        let titles: Vec<_> = table.rows().iter().map(|r| r.title.as_ref()).collect();
        assert_eq!(titles, ["Home", "Work", "Bills"]);
    }
}
