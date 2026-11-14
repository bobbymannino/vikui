use serde::Deserialize;

/// A Vikunja project (`GET /projects`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Project {
    pub id: i64,
    pub title: String,
    /// Rich text (HTML); see [`Project::plain_description`].
    #[serde(default)]
    pub description: String,
    /// Short prefix used for task identifiers, e.g. `PRJ` in `PRJ-12`.
    #[serde(default)]
    pub identifier: String,
    /// `0` for top-level projects.
    #[serde(default)]
    pub parent_project_id: i64,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub is_favorite: bool,
}

impl Project {
    /// Vikunja's built-in pseudo projects (such as "Favorites") have negative ids
    /// and cannot hold tasks of their own.
    #[must_use]
    pub fn is_pseudo(&self) -> bool {
        self.id < 0
    }

    #[must_use]
    pub fn parent_id(&self) -> Option<i64> {
        (self.parent_project_id > 0).then_some(self.parent_project_id)
    }

    /// The description as a single line of plain text, with HTML tags removed.
    #[must_use]
    pub fn plain_description(&self) -> String {
        html_to_plain_text(&self.description)
    }
}

fn html_to_plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                // Tags usually separate words (`<p>a</p><p>b</p>`).
                text.push(' ');
            }
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }

    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_api_response() {
        let json = r#"[
            {"id":-1,"title":"Favorites","description":"","identifier":"","parent_project_id":0},
            {"id":3,"title":"Home","description":"<p>Chores &amp; errands</p>","identifier":"HOME",
             "hex_color":"e8e8e8","parent_project_id":1,"is_archived":false,"is_favorite":true,
             "position":65536,"created":"2024-01-01T00:00:00Z"}
        ]"#;
        let projects: Vec<Project> = serde_json::from_str(json).unwrap();
        assert!(projects[0].is_pseudo());
        let home = &projects[1];
        assert!(!home.is_pseudo());
        assert_eq!(home.parent_id(), Some(1));
        assert!(home.is_favorite);
        assert_eq!(home.plain_description(), "Chores & errands");
    }

    #[test]
    fn strips_html_and_collapses_whitespace() {
        assert_eq!(
            html_to_plain_text("<p>One</p><p>Two&nbsp; <b>three</b></p>\n"),
            "One Two three"
        );
        assert_eq!(html_to_plain_text("a &lt;b&gt; &amp;amp;"), "a <b> &amp;");
        assert_eq!(html_to_plain_text(""), "");
    }
}
