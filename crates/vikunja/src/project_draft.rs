//! A project's editable fields, to create a project or save over an existing one.

use serde_json::Value;

/// What the user can set on a project. See [`crate::Client::create_project`] and
/// [`crate::Client::update_project`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectDraft {
    pub title: String,
    /// Rich text (HTML).
    pub description: String,
    /// The prefix of its tasks' identifiers, e.g. `HOME` in `HOME-12`; may be empty.
    pub identifier: String,
    /// `0xRRGGBB`, or `None` for no colour.
    pub rgb: Option<u32>,
    /// The project it sits under, or `None` for a top-level project.
    pub parent_id: Option<i64>,
}

impl ProjectDraft {
    fn fields(&self) -> [(&'static str, Value); 5] {
        [
            ("title", Value::String(self.title.clone())),
            ("description", Value::String(self.description.clone())),
            ("identifier", Value::String(self.identifier.clone())),
            (
                "hex_color",
                Value::String(crate::label_draft::hex_color(self.rgb)),
            ),
            (
                "parent_project_id",
                Value::from(self.parent_id.unwrap_or(0)),
            ),
        ]
    }

    /// The body that creates the project.
    pub(crate) fn to_json(&self) -> Value {
        Value::Object(
            self.fields()
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    /// Set the draft's fields on `project`'s JSON, leaving every other field as it is.
    /// Vikunja's update replaces the whole project, so it is sent back in full.
    pub(crate) fn apply(&self, project: &mut Value) {
        if let Some(fields) = project.as_object_mut() {
            for (name, value) in self.fields() {
                fields.insert(name.to_owned(), value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ProjectDraft {
        ProjectDraft {
            title: "Home".into(),
            description: "<p>Chores</p>".into(),
            identifier: "HOME".into(),
            rgb: Some(0x19_73_ff),
            parent_id: None,
        }
    }

    #[test]
    fn creates_a_top_level_project() {
        assert_eq!(
            draft().to_json(),
            serde_json::json!({
                "title": "Home", "description": "<p>Chores</p>", "identifier": "HOME",
                "hex_color": "1973ff", "parent_project_id": 0
            })
        );
    }

    #[test]
    fn apply_keeps_other_fields() {
        let mut project = serde_json::json!({
            "id": 4, "title": "Old", "description": "", "identifier": "", "hex_color": "",
            "parent_project_id": 0, "is_archived": false, "position": 3.5
        });
        ProjectDraft {
            parent_id: Some(2),
            rgb: None,
            ..draft()
        }
        .apply(&mut project);
        assert_eq!(
            project,
            serde_json::json!({
                "id": 4, "title": "Home", "description": "<p>Chores</p>",
                "identifier": "HOME", "hex_color": "", "parent_project_id": 2,
                "is_archived": false, "position": 3.5
            })
        );
    }
}
