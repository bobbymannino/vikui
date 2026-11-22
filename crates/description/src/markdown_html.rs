//! Markdown to HTML, in the shape Vikunja's editor writes.

use std::fmt::Write as _;

use markdown::mdast::{List, ListItem, Node};
use markdown::{ParseOptions, to_mdast};

/// `text` (GitHub-flavoured markdown) as the HTML Vikunja stores. Raw HTML in the text
/// is kept as text, links with script URLs lose their link, and every new line inside a
/// paragraph becomes a `<br>`. Empty for blank text.
#[must_use]
pub fn to_html(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    if text.trim().is_empty() {
        return String::new();
    }
    let mut html = String::new();
    match to_mdast(&text, &ParseOptions::gfm()) {
        Ok(root) => blocks(&root, &mut html),
        // Only MDX syntax can fail to parse, and it is off; keep the text regardless.
        Err(_) => {
            for paragraph in text.split("\n\n").filter(|p| !p.trim().is_empty()) {
                html.push_str("<p>");
                html.push_str(&escape(paragraph.trim()).replace('\n', "<br>"));
                html.push_str("</p>");
            }
        }
    }
    html
}

fn blocks(parent: &Node, out: &mut String) {
    for child in parent.children().into_iter().flatten() {
        block(child, out);
    }
}

fn block(node: &Node, out: &mut String) {
    match node {
        Node::Paragraph(paragraph) => {
            out.push_str("<p>");
            inlines(&paragraph.children, out);
            out.push_str("</p>");
        }
        Node::Heading(heading) => {
            let level = heading.depth.clamp(1, 6);
            let _ = write!(out, "<h{level}>");
            inlines(&heading.children, out);
            let _ = write!(out, "</h{level}>");
        }
        Node::List(list) => self::list(list, out),
        Node::Blockquote(_) => {
            out.push_str("<blockquote>");
            blocks(node, out);
            out.push_str("</blockquote>");
        }
        Node::Code(code) => {
            match code.lang.as_deref().filter(|lang| !lang.is_empty()) {
                Some(lang) => {
                    let _ = write!(out, "<pre><code class=\"language-{}\">", escape_attr(lang));
                }
                None => out.push_str("<pre><code>"),
            }
            out.push_str(&escape(&code.value));
            out.push_str("\n</code></pre>");
        }
        Node::ThematicBreak(_) => out.push_str("<hr>"),
        Node::Table(table) => {
            out.push_str("<table><tbody>");
            for (ix, row) in table.children.iter().enumerate() {
                let cell = if ix == 0 { "th" } else { "td" };
                out.push_str("<tr>");
                for child in row.children().into_iter().flatten() {
                    let _ = write!(out, "<{cell}><p>");
                    if let Some(children) = child.children() {
                        inlines(children, out);
                    }
                    let _ = write!(out, "</p></{cell}>");
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody></table>");
        }
        Node::Html(html) => {
            out.push_str("<p>");
            out.push_str(&escape(&html.value).replace('\n', "<br>"));
            out.push_str("</p>");
        }
        // Link definitions are used by their references; they show nothing themselves.
        Node::Definition(_) => {}
        Node::FootnoteDefinition(_) | Node::Root(_) => blocks(node, out),
        // Anything else at block level (front matter, maths): its text, as a paragraph.
        _ => {
            let text = node.to_string();
            if !text.trim().is_empty() {
                out.push_str("<p>");
                out.push_str(&escape(&text));
                out.push_str("</p>");
            }
        }
    }
}

fn list(list: &List, out: &mut String) {
    let tasks = list.children.iter().any(|item| match item {
        Node::ListItem(item) => item.checked.is_some(),
        _ => false,
    });
    let tag = if list.ordered { "ol" } else { "ul" };
    if tasks && !list.ordered {
        out.push_str("<ul data-type=\"taskList\">");
    } else {
        match list.start.filter(|start| list.ordered && *start != 1) {
            Some(start) => {
                let _ = write!(out, "<ol start=\"{start}\">");
            }
            None => {
                let _ = write!(out, "<{tag}>");
            }
        }
    }
    for item in &list.children {
        match item {
            Node::ListItem(item) => list_item(item, tasks && !list.ordered, out),
            other => block(other, out),
        }
    }
    let _ = write!(out, "</{tag}>");
}

fn list_item(item: &ListItem, task: bool, out: &mut String) {
    if task {
        let checked = item.checked.unwrap_or(false);
        let _ = write!(
            out,
            "<li data-checked=\"{checked}\" data-type=\"taskItem\"><label><input type=\"checkbox\"{}><span></span></label><div>",
            if checked { " checked=\"checked\"" } else { "" }
        );
        item.children.iter().for_each(|child| block(child, out));
        out.push_str("</div></li>");
    } else {
        out.push_str("<li>");
        item.children.iter().for_each(|child| block(child, out));
        out.push_str("</li>");
    }
}

fn inlines(nodes: &[Node], out: &mut String) {
    for node in nodes {
        inline(node, out);
    }
}

fn inline(node: &Node, out: &mut String) {
    let wrapped = |tag: &str, children: &[Node], out: &mut String| {
        let _ = write!(out, "<{tag}>");
        inlines(children, out);
        let _ = write!(out, "</{tag}>");
    };
    match node {
        // A new line inside a paragraph (a soft break) is kept as a line break.
        Node::Text(text) => out.push_str(&escape(&text.value).replace('\n', "<br>")),
        Node::Break(_) => out.push_str("<br>"),
        Node::Strong(strong) => wrapped("strong", &strong.children, out),
        Node::Emphasis(emphasis) => wrapped("em", &emphasis.children, out),
        Node::Delete(delete) => wrapped("s", &delete.children, out),
        Node::InlineCode(code) => {
            out.push_str("<code>");
            out.push_str(&escape(&code.value));
            out.push_str("</code>");
        }
        Node::Link(link) if safe_url(&link.url) => {
            let _ = write!(
                out,
                "<a target=\"_blank\" rel=\"noopener noreferrer nofollow\" href=\"{}\">",
                escape_attr(&link.url)
            );
            inlines(&link.children, out);
            out.push_str("</a>");
        }
        Node::Link(link) => inlines(&link.children, out),
        Node::Image(image) if safe_url(&image.url) => {
            let _ = write!(
                out,
                "<img src=\"{}\" alt=\"{}\">",
                escape_attr(&image.url),
                escape_attr(&image.alt)
            );
        }
        Node::Image(image) => out.push_str(&escape(&image.alt)),
        Node::Html(html) => out.push_str(&escape(&html.value)),
        _ => match node.children() {
            Some(children) => inlines(children, out),
            None => out.push_str(&escape(&node.to_string())),
        },
    }
}

/// Whether `url` is safe to link: not a script or data URL.
fn safe_url(url: &str) -> bool {
    let scheme: String = url
        .trim()
        .chars()
        .take_while(|c| *c != ':' && *c != '/' && *c != '?' && *c != '#')
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    let has_scheme = url.trim().len() > scheme.len() && url.trim()[scheme.len()..].starts_with(':');
    !(has_scheme && matches!(scheme.as_str(), "javascript" | "vbscript" | "data"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LineBreaks, to_markdown};

    #[test]
    fn paragraphs_keep_their_lines() {
        assert_eq!(to_html(""), "");
        assert_eq!(to_html("  \n\n "), "");
        assert_eq!(to_html("a\nb\n\nc"), "<p>a<br>b</p><p>c</p>");
        assert_eq!(to_html("a\\\nb"), "<p>a<br>b</p>");
        assert_eq!(
            to_html("x & <b>y</b>"),
            "<p>x &amp; &lt;b&gt;y&lt;/b&gt;</p>"
        );
    }

    #[test]
    fn formatting() {
        assert_eq!(
            to_html("**Flat** 2, *soon*, ~~never~~, `a<b`"),
            "<p><strong>Flat</strong> 2, <em>soon</em>, <s>never</s>, <code>a&lt;b</code></p>"
        );
        assert_eq!(
            to_html("[docs](https://example.com/?a=1&b=\"2\")"),
            "<p><a target=\"_blank\" rel=\"noopener noreferrer nofollow\" href=\"https://example.com/?a=1&amp;b=&quot;2&quot;\">docs</a></p>"
        );
        assert_eq!(to_html("[x](javascript:alert(1))"), "<p>x</p>");
        assert_eq!(to_html("[x]( JavaScript:alert(1))"), "<p>x</p>");
    }

    #[test]
    fn blocks_and_lists() {
        assert_eq!(
            to_html("## Plan\n\n> quoted\n\n---\n\n```rust\nlet a = 1;\n```"),
            "<h2>Plan</h2><blockquote><p>quoted</p></blockquote><hr><pre><code class=\"language-rust\">let a = 1;\n</code></pre>"
        );
        assert_eq!(
            to_html("- one\n- two\n\n3. three"),
            "<ul><li><p>one</p></li><li><p>two</p></li></ul><ol start=\"3\"><li><p>three</p></li></ol>"
        );
        assert_eq!(
            to_html("- [x] done\n- [ ] to do"),
            "<ul data-type=\"taskList\"><li data-checked=\"true\" data-type=\"taskItem\"><label><input type=\"checkbox\" checked=\"checked\"><span></span></label><div><p>done</p></div></li><li data-checked=\"false\" data-type=\"taskItem\"><label><input type=\"checkbox\"><span></span></label><div><p>to do</p></div></li></ul>"
        );
    }

    /// What Vikunja's editor writes comes back the same after a trip through markdown.
    #[test]
    fn round_trips_vikunja_html() {
        for html in [
            "<p><strong>Flat</strong> 2<br>Ref A&amp;B</p><p>Call first</p>",
            "<h1>Title</h1><p>Text with <em>emphasis</em> and <code>code</code>.</p>",
            "<ul><li><p>one</p></li><li><p>two</p><ul><li><p>nested</p></li></ul></li></ul>",
            "<ol><li><p>first</p></li><li><p>second</p></li></ol>",
            "<blockquote><p>quote</p></blockquote><hr><p>2 * 3 [x]</p>",
            "<pre><code class=\"language-sh\">echo \"hi\" &amp;&amp; exit\n</code></pre>",
            "<ul data-type=\"taskList\"><li data-checked=\"true\" data-type=\"taskItem\"><label><input type=\"checkbox\" checked=\"checked\"><span></span></label><div><p>done</p></div></li></ul>",
            "<p><a target=\"_blank\" rel=\"noopener noreferrer nofollow\" href=\"https://example.com\">link</a></p>",
        ] {
            let markdown = to_markdown(html, LineBreaks::Newline);
            assert_eq!(to_html(&markdown), html, "via {markdown:?}");
        }
    }

    /// Markdown typed in Vikui stays the same after saving and editing again.
    #[test]
    fn round_trips_markdown() {
        for markdown in [
            "a\nb\n\nc",
            "**bold** and *em* and ~~gone~~ and `code`",
            "# Heading\n\n- one\n- two\n  - nested\n\n1. first\n2. second",
            "- [x] done\n- [ ] to do",
            "> quoted\n\n---\n\n```rust\nfn main() {}\n```",
            "snake_case, 2 \\* 3, \\[not a link\\]",
            "[docs](https://example.com)",
        ] {
            let html = to_html(markdown);
            assert_eq!(
                to_markdown(&html, LineBreaks::Newline),
                markdown,
                "via {html:?}"
            );
        }
    }
}
