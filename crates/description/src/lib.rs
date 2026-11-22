//! Vikunja stores task descriptions as HTML (from its rich text editor). Vikui shows and
//! edits them as markdown: [`to_markdown`] turns the HTML into markdown, and [`to_html`]
//! turns edited markdown back into HTML in the shape Vikunja's editor writes.
//!
//! Paragraph line breaks (`<br>`) become plain new lines for editing
//! ([`LineBreaks::Newline`]), and every new line in edited markdown becomes a `<br>`
//! again, so text typed line by line keeps its lines. For display,
//! [`LineBreaks::Hard`] writes them as markdown hard breaks instead. No UI.

mod html;
mod markdown_html;

pub use html::{LineBreaks, to_markdown};
pub use markdown_html::to_html;
