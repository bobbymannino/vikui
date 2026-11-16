//! A task label drawn as a coloured tag, in the table and the label filter.

use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::{App, Hsla, ParentElement as _, SharedString, prelude::FluentBuilder as _};
use reminders::TaskLabel;

/// One label, ready to draw as a tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelTag {
    pub id: i64,
    pub title: SharedString,
    /// `0xRRGGBB`; `None` draws the theme's neutral tag.
    rgb: Option<u32>,
}

impl LabelTag {
    pub fn new(label: &TaskLabel) -> Self {
        Self {
            id: label.id,
            title: label.title.clone().into(),
            rgb: label.rgb,
        }
    }

    /// A faint fill of the label's colour with the colour as text, its lightness pushed
    /// away from the background so the text stays readable in light and dark themes.
    /// `checked` adds a check mark, so a selected label isn't told apart by colour alone.
    pub fn render(&self, checked: bool, cx: &App) -> Tag {
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
        tag.when(checked, |tag| {
            tag.child(Icon::new(IconName::Check).xsmall())
        })
        .child(self.title.clone())
    }
}
