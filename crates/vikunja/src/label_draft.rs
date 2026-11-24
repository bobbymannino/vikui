//! A label's title and colour, to create a label or save over an existing one.

use serde_json::Value;

/// What the user can set on a label. See [`crate::Client::create_label`] and
/// [`crate::Client::update_label`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelDraft {
    pub title: String,
    /// `0xRRGGBB`, or `None` for no colour.
    pub rgb: Option<u32>,
}

impl LabelDraft {
    /// The body that creates the label.
    pub(crate) fn to_json(&self) -> Value {
        serde_json::json!({ "title": self.title, "hex_color": hex_color(self.rgb) })
    }

    /// Set the title and colour on `label`'s JSON, leaving every other field as it is.
    /// Vikunja's update replaces the whole label, so it is sent back in full.
    pub(crate) fn apply(&self, label: &mut Value) {
        if let Some(fields) = label.as_object_mut() {
            fields.insert("title".to_owned(), Value::String(self.title.clone()));
            fields.insert("hex_color".to_owned(), Value::String(hex_color(self.rgb)));
        }
    }
}

/// A colour as Vikunja stores it: `rrggbb`, or empty for none.
pub(crate) fn hex_color(rgb: Option<u32>) -> String {
    rgb.map_or_else(String::new, |rgb| format!("{:06x}", rgb & 0x00ff_ffff))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_colour_as_vikunja_stores_it() {
        let draft = LabelDraft {
            title: "Bills".into(),
            rgb: Some(0x00_73_ff),
        };
        assert_eq!(
            draft.to_json(),
            serde_json::json!({ "title": "Bills", "hex_color": "0073ff" })
        );
        let plain = LabelDraft {
            title: "Home".into(),
            rgb: None,
        };
        assert_eq!(
            plain.to_json(),
            serde_json::json!({ "title": "Home", "hex_color": "" })
        );
    }

    #[test]
    fn apply_keeps_other_fields() {
        let mut label = serde_json::json!({
            "id": 3, "title": "Old", "hex_color": "e8e8e8", "description": "x",
            "created_by": {"id": 1}
        });
        LabelDraft {
            title: "New".into(),
            rgb: Some(0xff_00_00),
        }
        .apply(&mut label);
        assert_eq!(
            label,
            serde_json::json!({
                "id": 3, "title": "New", "hex_color": "ff0000", "description": "x",
                "created_by": {"id": 1}
            })
        );
    }
}
