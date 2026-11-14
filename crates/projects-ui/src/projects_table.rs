//! Table delegate listing the user's projects.

use std::cmp::Ordering;
use std::collections::HashMap;

use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use vikunja::Project;

/// One table row, prepared off the main thread so rendering only clones `SharedString`s.
#[derive(Clone, Debug)]
pub struct ProjectRow {
    pub title: SharedString,
    pub parent: SharedString,
    pub identifier: SharedString,
    pub description: SharedString,
    /// Position in the server's order, used to undo sorting.
    order: usize,
}

impl ProjectRow {
    /// Build rows from an API response, dropping pseudo projects and resolving parent names.
    pub fn from_projects(projects: &[Project]) -> Vec<Self> {
        let titles: HashMap<i64, &str> = projects
            .iter()
            .map(|project| (project.id, project.title.as_str()))
            .collect();
        projects
            .iter()
            .filter(|project| !project.is_pseudo())
            .enumerate()
            .map(|(order, project)| Self {
                title: project.title.clone().into(),
                parent: project
                    .parent_id()
                    .and_then(|id| titles.get(&id))
                    .map_or_else(SharedString::default, |title| (*title).to_owned().into()),
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
    Parent,
    Identifier,
    Description,
}

const COLUMNS: [Col; 4] = [Col::Title, Col::Parent, Col::Identifier, Col::Description];

impl Col {
    fn at(ix: usize) -> Option<Self> {
        COLUMNS.get(ix).copied()
    }

    fn text(self, row: &ProjectRow) -> &SharedString {
        match self {
            Self::Title => &row.title,
            Self::Parent => &row.parent,
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
            Some(Col::Parent) => Column::new("parent", "Parent").width(180.),
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
    fn rows_skip_pseudo_projects_and_name_parents() {
        let rows = ProjectRow::from_projects(&[
            project(-1, "Favorites", 0),
            project(1, "Work", 0),
            project(2, "Reports", 1),
            project(3, "Orphan", 99),
        ]);
        let titles: Vec<_> = rows.iter().map(|r| r.title.as_ref()).collect();
        assert_eq!(titles, ["Work", "Reports", "Orphan"]);
        assert_eq!(rows[0].parent, "");
        assert_eq!(rows[1].parent, "Work");
        assert_eq!(rows[2].parent, "");
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
        table.set_rows(ProjectRow::from_projects(&[
            project(1, "Work", 0),
            project(2, "Child", 1),
            project(3, "Home", 0),
        ]));
        table.sort = Some((Col::Parent, ColumnSort::Ascending));
        table.apply_sort();
        let titles: Vec<_> = table.rows().iter().map(|r| r.title.as_ref()).collect();
        assert_eq!(titles, ["Child", "Work", "Home"]);
    }
}
