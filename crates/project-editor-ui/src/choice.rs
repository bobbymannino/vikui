//! One option in a select, identified by a number.

use gpui_kit::SharedString;
use gpui_kit::component::searchable_list::SearchableListItem;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: i64,
    pub title: SharedString,
}

impl Choice {
    pub fn new(id: i64, title: impl Into<SharedString>) -> Self {
        Self {
            id,
            title: title.into(),
        }
    }
}

impl SearchableListItem for Choice {
    type Value = i64;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &i64 {
        &self.id
    }
}
