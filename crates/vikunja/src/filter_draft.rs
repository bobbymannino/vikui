//! A saved filter's editable fields, to create a filter or save over an existing one.

use serde_json::Value;

/// What the user can set on a saved filter. See [`crate::Client::create_saved_filter`]
/// and [`crate::Client::update_saved_filter`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterDraft {
    pub title: String,
    /// Rich text (HTML).
    pub description: String,
    /// Vikunja's filter query, e.g. `done = false && due_date < now/w`.
    pub filter: String,
    /// Also list tasks that don't have the fields the query compares, e.g. no due date.
    pub include_nulls: bool,
}

impl FilterDraft {
    /// The body that creates the filter.
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({
            "title": self.title,
            "description": self.description,
            "filters": {
                "filter": self.filter,
                "filter_include_nulls": self.include_nulls,
            },
        })
    }

    /// Set the draft's fields on `filter`'s JSON, leaving every other field (the query's
    /// sorting, say) as it is. Vikunja's update replaces the whole filter, so it is sent
    /// back in full.
    pub(crate) fn apply(&self, filter: &mut Value) {
        let Some(fields) = filter.as_object_mut() else {
            return;
        };
        fields.insert("title".to_owned(), Value::String(self.title.clone()));
        fields.insert(
            "description".to_owned(),
            Value::String(self.description.clone()),
        );
        let query = fields
            .entry("filters")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !query.is_object() {
            *query = Value::Object(serde_json::Map::new());
        }
        if let Some(query) = query.as_object_mut() {
            query.insert("filter".to_owned(), Value::String(self.filter.clone()));
            query.insert(
                "filter_include_nulls".to_owned(),
                Value::Bool(self.include_nulls),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> FilterDraft {
        FilterDraft {
            title: "This week".into(),
            description: String::new(),
            filter: "done = false && due_date < now/w+1w".into(),
            include_nulls: true,
        }
    }

    #[test]
    fn creates_a_filter() {
        assert_eq!(
            draft().to_json(),
            serde_json::json!({
                "title": "This week", "description": "",
                "filters": {
                    "filter": "done = false && due_date < now/w+1w",
                    "filter_include_nulls": true
                }
            })
        );
    }

    #[test]
    fn apply_keeps_other_fields_and_the_query_sorting() {
        let mut filter = serde_json::json!({
            "id": 3, "title": "Old", "description": "<p>x</p>", "is_favorite": true,
            "filters": { "filter": "done = true", "filter_include_nulls": false,
                         "sort_by": ["due_date"], "order_by": ["asc"] }
        });
        draft().apply(&mut filter);
        assert_eq!(
            filter,
            serde_json::json!({
                "id": 3, "title": "This week", "description": "", "is_favorite": true,
                "filters": { "filter": "done = false && due_date < now/w+1w",
                             "filter_include_nulls": true,
                             "sort_by": ["due_date"], "order_by": ["asc"] }
            })
        );
    }

    #[test]
    fn apply_adds_a_missing_query() {
        let mut filter = serde_json::json!({ "id": 3, "title": "Old", "filters": null });
        draft().apply(&mut filter);
        assert_eq!(
            filter["filters"]["filter"],
            "done = false && due_date < now/w+1w"
        );
    }
}
