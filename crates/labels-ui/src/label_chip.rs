//! A label drawn as a coloured tag, as in the reminders list.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::{App, Hsla, ParentElement as _, SharedString};
use vikunja::Label;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelChip {
    pub id: i64,
    pub title: SharedString,
    /// `0xRRGGBB`; `None` draws the theme's neutral tag.
    pub rgb: Option<u32>,
}

impl LabelChip {
    pub fn new(label: &Label) -> Self {
        Self {
            id: label.id,
            title: label.title.clone().into(),
            rgb: label.rgb(),
        }
    }

    /// A faint fill of the label's colour with the colour as text, its lightness pushed
    /// away from the background so the text stays readable in light and dark themes.
    pub fn render(&self, cx: &App) -> Tag {
        let tag = match self.rgb {
            Some(rgb) => {
                let color: Hsla = gpui_kit::rgb(rgb).into();
                let fill = color.opacity(0.18);
                let text = Hsla {
                    l: if cx.theme().is_dark() {
                        color.l.max(0.72)
                    } else {
                        color.l.min(0.38)
                    },
                    ..color
                };
                Tag::custom(fill, text, fill)
            }
            None => Tag::secondary(),
        };
        tag.child(self.title.clone())
    }
}
