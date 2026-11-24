//! Projects in tree order, each under its parent, for the list and the parent select.

use std::collections::{HashMap, HashSet};

use gpui_kit::SharedString;
use vikunja::{Project, ProjectKind};

/// One project in tree order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub id: i64,
    pub title: SharedString,
    /// Its title after its parents', e.g. "Home › Chores", to tell same-named apart.
    pub path: SharedString,
    /// `0` for a top-level project.
    pub depth: usize,
    pub rgb: Option<u32>,
}

/// Real projects (not Favorites or saved filters) in tree order: each after its parent,
/// siblings by title. A project whose parent isn't listed counts as top level.
pub fn rows(projects: &[Project]) -> Vec<TreeRow> {
    let projects: Vec<&Project> = projects
        .iter()
        .filter(|project| project.kind() == ProjectKind::Project)
        .collect();
    let ids: HashSet<i64> = projects.iter().map(|project| project.id).collect();
    let mut children: HashMap<Option<i64>, Vec<&Project>> = HashMap::new();
    for &project in &projects {
        let parent = project.parent_id().filter(|parent| ids.contains(parent));
        children.entry(parent).or_default().push(project);
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|project| (project.title.to_lowercase(), project.id));
    }
    let mut rows = Vec::with_capacity(projects.len());
    // Depth-first from the top level; `seen` guards against a parent loop on the server.
    let mut seen = HashSet::new();
    let mut stack: Vec<(&Project, usize, String)> = children
        .get(&None)
        .into_iter()
        .flatten()
        .rev()
        .map(|&project| (project, 0, String::new()))
        .collect();
    while let Some((project, depth, parents)) = stack.pop() {
        if !seen.insert(project.id) {
            continue;
        }
        let path = if parents.is_empty() {
            project.title.clone()
        } else {
            format!("{parents} › {}", project.title)
        };
        for &child in children.get(&Some(project.id)).into_iter().flatten().rev() {
            stack.push((child, depth + 1, path.clone()));
        }
        rows.push(TreeRow {
            id: project.id,
            title: project.title.clone().into(),
            path: path.into(),
            depth,
            rgb: project.rgb(),
        });
    }
    rows
}

/// `project_id` and every project under it: none of them can be its parent.
pub fn self_and_descendants(rows: &[TreeRow], project_id: i64) -> HashSet<i64> {
    let mut found = HashSet::new();
    // In tree order a project's descendants follow it, deeper than it, until the next
    // project at its depth or shallower.
    let mut rest = rows.iter().skip_while(|row| row.id != project_id);
    if let Some(start) = rest.next() {
        found.insert(project_id);
        found.extend(
            rest.take_while(|row| row.depth > start.depth)
                .map(|row| row.id),
        );
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: i64, title: &str, parent: i64) -> Project {
        Project {
            id,
            title: title.into(),
            parent_project_id: parent,
            ..Project::default()
        }
    }

    fn paths(rows: &[TreeRow]) -> Vec<(&str, usize)> {
        rows.iter()
            .map(|row| (row.path.as_ref(), row.depth))
            .collect()
    }

    #[test]
    fn lists_each_project_under_its_parent() {
        let rows = rows(&[
            project(3, "chores", 1),
            project(1, "Home", 0),
            project(2, "Work", 0),
            project(4, "Garden", 3),
            project(5, "Bills", 1),
            // Favorites and a saved filter aren't projects.
            project(-1, "Favorites", 0),
            project(-3, "This week", 0),
            // Its parent isn't listed (archived, say).
            project(6, "Orphan", 99),
        ]);
        assert_eq!(
            paths(&rows),
            [
                ("Home", 0),
                ("Home › Bills", 1),
                ("Home › chores", 1),
                ("Home › chores › Garden", 2),
                ("Orphan", 0),
                ("Work", 0),
            ]
        );
        let ids: HashSet<i64> = [3, 4].into();
        assert_eq!(self_and_descendants(&rows, 3), ids);
        assert_eq!(self_and_descendants(&rows, 2), [2].into());
        assert!(self_and_descendants(&rows, 42).is_empty());
    }

    #[test]
    fn survives_a_parent_loop() {
        let rows = rows(&[project(1, "A", 2), project(2, "B", 1), project(3, "C", 0)]);
        assert_eq!(paths(&rows), [("C", 0)]);
    }
}
