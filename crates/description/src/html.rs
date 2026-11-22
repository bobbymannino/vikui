//! HTML (as Vikunja's editor writes it) to markdown.

use std::fmt::Write as _;

/// How a line break inside a paragraph (`<br>`) is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineBreaks {
    /// A plain new line, for editing; [`crate::to_html`] turns it back into `<br>`.
    Newline,
    /// A markdown hard break (`\` before the new line), for rendering as markdown.
    Hard,
}

/// `html` as markdown: paragraphs, headings, emphasis, code, links, images, lists (task
/// lists included), quotes, rules and tables. Tags it doesn't know keep their text.
#[must_use]
pub fn to_markdown(html: &str, breaks: LineBreaks) -> String {
    let nodes = parse(html);
    Writer { breaks }.blocks(&nodes).join("\n\n")
}

#[derive(Debug)]
enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Default)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|(key, _)| key == name)
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }
}

/// Elements that never have children or a closing tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Elements whose content is never shown.
const HIDDEN: &[&str] = &["script", "style", "template", "head", "title"];

/// A forgiving parse into a tree: unclosed elements close with their parent, and stray
/// closing tags are ignored.
fn parse(html: &str) -> Vec<Node> {
    let mut stack = vec![Element::default()];
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        if let Some((tag, after)) = tag(rest) {
            rest = after;
            match tag {
                Tag::Close(name) => {
                    if let Some(depth) = stack.iter().rposition(|open| open.name == name)
                        && depth > 0
                    {
                        while stack.len() > depth {
                            close(&mut stack);
                        }
                    }
                }
                Tag::Open(element, self_closing) => {
                    if self_closing || VOID.contains(&element.name.as_str()) {
                        push(&mut stack, Node::Element(element));
                    } else {
                        stack.push(element);
                    }
                }
                Tag::Other => {}
            }
            continue;
        }
        // Text up to the next tag; a `<` that starts no tag is text too.
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let end = rest
            .get(first..)
            .and_then(|after| after.find('<'))
            .map_or(rest.len(), |next| next + first);
        push(&mut stack, Node::Text(decode(&rest[..end])));
        rest = &rest[end..];
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().map_or_else(Vec::new, |root| root.children)
}

fn push(stack: &mut [Element], node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    }
}

fn close(stack: &mut Vec<Element>) {
    if let Some(element) = stack.pop()
        && !HIDDEN.contains(&element.name.as_str())
    {
        push(stack, Node::Element(element));
    }
}

enum Tag {
    Open(Element, bool),
    Close(String),
    /// `<!doctype>`, `<?xml?>` and the like.
    Other,
}

/// The tag at the start of `text`, and the text after it; `None` if `text` doesn't start
/// with one.
fn tag(text: &str) -> Option<(Tag, &str)> {
    let inner = text.strip_prefix('<')?;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(inner) => (true, inner),
        None => (false, inner),
    };
    let first = inner.chars().next()?;
    if !(first.is_ascii_alphabetic() || (!closing && matches!(first, '!' | '?'))) {
        return None;
    }
    let end = tag_end(inner)?;
    let body = &inner[..end];
    let after = &inner[end + 1..];
    if matches!(first, '!' | '?') {
        return Some((Tag::Other, after));
    }
    let name_end = body
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(body.len());
    let name = body[..name_end].to_ascii_lowercase();
    if closing {
        return Some((Tag::Close(name), after));
    }
    let attrs_text = body[name_end..].trim_end();
    let self_closing = attrs_text.ends_with('/');
    let element = Element {
        name,
        attrs: attributes(attrs_text.trim_end_matches('/')),
        children: Vec::new(),
    };
    Some((Tag::Open(element, self_closing), after))
}

/// Where the tag starting `text` ends (its `>`), skipping `>` inside quoted values.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (ix, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), _) if c == open => quote = None,
            (None, '>') => return Some(ix),
            _ => {}
        }
    }
    None
}

fn attributes(mut text: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    loop {
        text = text.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if text.is_empty() {
            return attrs;
        }
        let name_end = text
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(text.len());
        let name = text[..name_end].to_ascii_lowercase();
        text = text[name_end..].trim_start();
        let mut value = String::new();
        if let Some(rest) = text.strip_prefix('=') {
            let rest = rest.trim_start();
            let quoted = rest
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\''))
                .and_then(|quote| Some((quote, rest.get(1..)?)));
            let (raw, after) = if let Some((quote, body)) = quoted {
                let end = body.find(quote).unwrap_or(body.len());
                (&body[..end], body.get(end + 1..).unwrap_or(""))
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..end], &rest[end..])
            };
            value = decode(raw);
            text = after;
        }
        attrs.push((name, value));
    }
}

/// Decode character references: the named ones Vikunja writes, and numeric ones.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let c = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity.strip_prefix('#').and_then(|number| {
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => number.parse().ok(),
                    };
                    code.and_then(char::from_u32)
                }),
            };
            c.map(|c| (c, end))
        });
        if let Some((c, end)) = decoded {
            out.push(c);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

struct Writer {
    breaks: LineBreaks,
}

impl Writer {
    /// `nodes` as markdown blocks, to join with blank lines. Inline content between
    /// blocks becomes a paragraph.
    fn blocks(&self, nodes: &[Node]) -> Vec<String> {
        let mut blocks = Vec::new();
        let mut inline: Vec<&Node> = Vec::new();
        for node in nodes {
            match node {
                Node::Element(element) if is_block(&element.name) => {
                    self.paragraph(&inline, &mut blocks);
                    inline.clear();
                    self.block(element, &mut blocks);
                }
                _ => inline.push(node),
            }
        }
        self.paragraph(&inline, &mut blocks);
        blocks
    }

    fn paragraph(&self, nodes: &[&Node], blocks: &mut Vec<String>) {
        let mut text = String::new();
        for node in nodes {
            self.inline(node, &mut text);
        }
        let text = tidy(&text);
        if !text.is_empty() {
            blocks.push(escape_line_starts(&text));
        }
    }

    fn block(&self, element: &Element, blocks: &mut Vec<String>) {
        match element.name.as_str() {
            "p" => {
                let nodes: Vec<&Node> = element.children.iter().collect();
                self.paragraph(&nodes, blocks);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = element.name[1..].parse::<usize>().unwrap_or(1);
                let text = tidy(&self.inline_children(element)).replace('\n', " ");
                if !text.is_empty() {
                    blocks.push(format!("{} {text}", "#".repeat(level)));
                }
            }
            "ul" | "ol" => {
                let list = self.list(element);
                if !list.is_empty() {
                    blocks.push(list);
                }
            }
            "blockquote" => {
                let inner = self.blocks(&element.children).join("\n\n");
                if !inner.is_empty() {
                    let quoted: Vec<String> = inner
                        .lines()
                        .map(|line| {
                            if line.is_empty() {
                                ">".to_owned()
                            } else {
                                format!("> {line}")
                            }
                        })
                        .collect();
                    blocks.push(quoted.join("\n"));
                }
            }
            "pre" => blocks.push(code_block(element)),
            "hr" => blocks.push("---".to_owned()),
            "table" => {
                let table = self.table(element);
                if !table.is_empty() {
                    blocks.push(table);
                }
            }
            // Containers: their content, as blocks.
            _ => blocks.extend(self.blocks(&element.children)),
        }
    }

    fn list(&self, list: &Element) -> String {
        let ordered = list.name == "ol";
        let start: usize = list
            .attr("start")
            .and_then(|start| start.trim().parse().ok())
            .unwrap_or(1);
        let tasks = list.attr("data-type") == Some("taskList");
        let mut items = Vec::new();
        for (ix, item) in list.elements().filter(|item| item.name == "li").enumerate() {
            let checked = match item.attr("data-checked") {
                Some(checked) => Some(checked == "true"),
                None => checkbox(item).or(tasks.then_some(false)),
            };
            // A task's checkbox (`<label><input></label>`) becomes its `[x]`.
            let is_checkbox = |node: &&Node| {
                checked.is_some()
                    && matches!(node, Node::Element(element)
                        if matches!(element.name.as_str(), "label" | "input"))
            };
            let kept: Vec<&Node> = item
                .children
                .iter()
                .filter(|node| !is_checkbox(node))
                .collect();
            let content = self.item_blocks(&kept);
            let marker = if ordered {
                format!("{}. ", start + ix)
            } else {
                "- ".to_owned()
            };
            let indent = " ".repeat(marker.len());
            let task = match checked {
                Some(true) => "[x] ",
                Some(false) => "[ ] ",
                None => "",
            };
            let mut lines = content.lines();
            let first = lines.next().unwrap_or("");
            let mut text = format!("{marker}{task}{first}").trim_end().to_owned();
            for line in lines {
                text.push('\n');
                if !line.is_empty() {
                    text.push_str(&indent);
                    text.push_str(line);
                }
            }
            items.push(text);
        }
        items.join("\n")
    }

    /// A list item's content: its blocks on consecutive lines, as a tight list item.
    fn item_blocks(&self, nodes: &[&Node]) -> String {
        let mut blocks = Vec::new();
        let mut inline: Vec<&Node> = Vec::new();
        for node in nodes {
            match node {
                Node::Element(element) if is_block(&element.name) => {
                    self.paragraph(&inline, &mut blocks);
                    inline.clear();
                    self.block(element, &mut blocks);
                }
                _ => inline.push(node),
            }
        }
        self.paragraph(&inline, &mut blocks);
        blocks.join("\n")
    }

    fn table(&self, table: &Element) -> String {
        let mut rows: Vec<Vec<String>> = Vec::new();
        collect_rows(table, &mut |row| {
            let cells = row
                .elements()
                .filter(|cell| matches!(cell.name.as_str(), "td" | "th"))
                .map(|cell| {
                    tidy(&self.inline_children(cell))
                        .replace('\n', " ")
                        .replace('|', "\\|")
                })
                .collect();
            rows.push(cells);
        });
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return String::new();
        }
        let line = |cells: &[String]| {
            let mut cells = cells.to_vec();
            cells.resize(columns, String::new());
            format!("| {} |", cells.join(" | "))
        };
        let mut rows = rows.iter();
        let mut lines = Vec::new();
        if let Some(header) = rows.next() {
            lines.push(line(header));
            lines.push(line(&vec!["---".to_owned(); columns]));
        }
        lines.extend(rows.map(|row| line(row)));
        lines.join("\n")
    }

    fn inline_children(&self, element: &Element) -> String {
        let mut text = String::new();
        for child in &element.children {
            self.inline(child, &mut text);
        }
        text
    }

    fn inline(&self, node: &Node, out: &mut String) {
        let element = match node {
            Node::Text(text) => {
                push_text(text, out);
                return;
            }
            Node::Element(element) => element,
        };
        match element.name.as_str() {
            "strong" | "b" => wrap(out, "**", &self.inline_children(element)),
            "em" | "i" => wrap(out, "*", &self.inline_children(element)),
            "s" | "del" | "strike" => wrap(out, "~~", &self.inline_children(element)),
            "code" => out.push_str(&inline_code(&text_content(element))),
            "br" => out.push_str(match self.breaks {
                LineBreaks::Newline => "\n",
                LineBreaks::Hard => "\\\n",
            }),
            "a" => {
                let text = self.inline_children(element);
                match element.attr("href").map(str::trim) {
                    Some(href) if !href.is_empty() => {
                        let text = if text.trim().is_empty() {
                            escape(href)
                        } else {
                            text
                        };
                        let _ = write!(out, "[{}]({})", text.trim(), destination(href));
                    }
                    _ => out.push_str(&text),
                }
            }
            "img" => {
                if let Some(src) = element.attr("src").filter(|src| !src.trim().is_empty()) {
                    let alt = escape(element.attr("alt").unwrap_or(""));
                    let _ = write!(out, "![{alt}]({})", destination(src.trim()));
                }
            }
            "input" => {}
            // A block inside inline content (e.g. a list item's paragraph): its text.
            name if is_block(name) => {
                out.push_str(&self.inline_children(element));
                out.push('\n');
            }
            _ => out.push_str(&self.inline_children(element)),
        }
    }
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "figure"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "blockquote"
            | "pre"
            | "hr"
            | "table"
            | "body"
            | "html"
    )
}

/// Whether a list item holds a checkbox, and if so whether it is checked.
fn checkbox(item: &Element) -> Option<bool> {
    fn find(element: &Element) -> Option<bool> {
        if element.name == "input" && element.attr("type") == Some("checkbox") {
            return Some(element.has_attr("checked"));
        }
        element
            .elements()
            .filter(|child| matches!(child.name.as_str(), "label" | "input"))
            .find_map(find)
    }
    item.elements()
        .filter(|child| matches!(child.name.as_str(), "label" | "input"))
        .find_map(find)
}

fn collect_rows(element: &Element, f: &mut impl FnMut(&Element)) {
    for child in element.elements() {
        match child.name.as_str() {
            "tr" => f(child),
            "thead" | "tbody" | "tfoot" => collect_rows(child, f),
            _ => {}
        }
    }
}

/// All the text in `element`, as written (for code).
fn text_content(element: &Element) -> String {
    fn walk(node: &Node, out: &mut String) {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Element(element) if element.name == "br" => out.push('\n'),
            Node::Element(element) => element.children.iter().for_each(|child| walk(child, out)),
        }
    }
    let mut out = String::new();
    element
        .children
        .iter()
        .for_each(|child| walk(child, &mut out));
    out
}

fn code_block(pre: &Element) -> String {
    let code = pre.elements().find(|child| child.name == "code");
    let language = code
        .and_then(|code| code.attr("class"))
        .and_then(|class| {
            class
                .split_whitespace()
                .find_map(|name| name.strip_prefix("language-"))
        })
        .unwrap_or("");
    let text = text_content(pre);
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
    format!("{fence}{language}\n{text}\n{fence}")
}

fn inline_code(text: &str) -> String {
    let text = text.replace('\n', " ");
    if text.is_empty() {
        return String::new();
    }
    let fence = "`".repeat(longest_run(&text, '`') + 1);
    // Padding keeps a backtick at either end from joining the fence.
    if text.starts_with('`') || text.ends_with('`') {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

fn longest_run(text: &str, c: char) -> usize {
    let (mut longest, mut run) = (0, 0);
    for ch in text.chars() {
        run = if ch == c { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

/// A link or image destination, in `<>` if it has characters that would end it early.
fn destination(url: &str) -> String {
    if url.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_owned()
    }
}

/// `text` (from HTML, so its runs of white space are one space) with markdown's
/// characters escaped.
fn push_text(text: &str, out: &mut String) {
    let mut last_space = out.ends_with([' ', '\n']) || out.is_empty();
    let mut collapsed = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_space {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }
    out.push_str(&escape(&collapsed));
}

/// Escape what would otherwise be markdown. Underscores inside words are left alone,
/// since they never start emphasis.
fn escape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (ix, &c) in chars.iter().enumerate() {
        let escape = match c {
            '\\' | '`' | '*' | '[' | ']' | '<' => true,
            '_' => {
                let word = |other: Option<&char>| other.is_some_and(|c| c.is_alphanumeric());
                !(word(ix.checked_sub(1).and_then(|prev| chars.get(prev)))
                    && word(chars.get(ix + 1)))
            }
            '~' => {
                chars.get(ix + 1) == Some(&'~')
                    || ix.checked_sub(1).and_then(|prev| chars.get(prev)) == Some(&'~')
            }
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape a paragraph line that would otherwise start a heading, quote, list or rule.
fn escape_line_starts(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            let digits = line.chars().take_while(char::is_ascii_digit).count();
            let after_digits = &line[digits..];
            let starts_block = line.starts_with(['#', '>', '='])
                || line.starts_with("- ")
                || line.starts_with("+ ")
                || line == "-"
                || line.starts_with("---")
                || (digits > 0
                    && (after_digits.starts_with(". ") || after_digits.starts_with(") ")));
            if !starts_block {
                line.to_owned()
            } else if digits > 0 {
                format!("{}\\{}", &line[..digits], after_digits)
            } else {
                format!("\\{line}")
            }
        })
        .collect();
    lines.join("\n")
}

/// Trim each line and the whole, keeping a hard break's `\` at a line's end.
fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    lines
        .join("\n")
        .trim()
        .trim_end_matches('\\')
        .trim_end()
        .to_owned()
}

/// `inner` between `marker`s, with white space at its ends moved outside them (where
/// markdown requires it).
fn wrap(out: &mut String, marker: &str, inner: &str) {
    let core = inner.trim();
    if core.is_empty() {
        out.push_str(inner);
        return;
    }
    let lead = &inner[..inner.len() - inner.trim_start().len()];
    let trail = &inner[inner.trim_end().len()..];
    out.push_str(lead);
    out.push_str(marker);
    out.push_str(core);
    out.push_str(marker);
    out.push_str(trail);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        to_markdown(html, LineBreaks::Newline)
    }

    #[test]
    fn paragraphs_and_line_breaks() {
        assert_eq!(md(""), "");
        assert_eq!(md("plain"), "plain");
        assert_eq!(md("<p>One<br>two</p><p>Three</p>"), "One\ntwo\n\nThree");
        assert_eq!(
            to_markdown("<p>One<br>two</p>", LineBreaks::Hard),
            "One\\\ntwo"
        );
        assert_eq!(md("<p>  spread \n out  </p>"), "spread out");
        assert_eq!(md("<p></p><p>Only</p><p> </p>"), "Only");
    }

    #[test]
    fn inline_formatting() {
        assert_eq!(
            md("<p><strong>Flat</strong> 2, <em>soon</em>, <s>never</s> and <code>a*b</code></p>"),
            "**Flat** 2, *soon*, ~~never~~ and `a*b`"
        );
        assert_eq!(md("<p><strong>bold </strong>text</p>"), "**bold** text");
        assert_eq!(
            md(
                r#"<p>See <a target="_blank" rel="noopener" href="https://example.com/a">the docs</a></p>"#
            ),
            "See [the docs](https://example.com/a)"
        );
        assert_eq!(
            md(r#"<p><img src="https://example.com/a b.png" alt="pic"></p>"#),
            "![pic](<https://example.com/a b.png>)"
        );
    }

    #[test]
    fn escapes_markdown_in_text() {
        assert_eq!(
            md("<p>2 * 3 &amp; [x] &lt;b&gt;</p>"),
            r"2 \* 3 & \[x\] \<b>"
        );
        assert_eq!(
            md("<p>snake_case but _this_</p>"),
            r"snake_case but \_this\_"
        );
        assert_eq!(
            md("<p># not a heading</p><p>1. not a list</p>"),
            "\\# not a heading\n\n1\\. not a list"
        );
    }

    #[test]
    fn blocks() {
        assert_eq!(
            md("<h2>Plan</h2><blockquote><p>Quoted</p><p>Twice</p></blockquote><hr>"),
            "## Plan\n\n> Quoted\n>\n> Twice\n\n---"
        );
        assert_eq!(
            md(r#"<pre><code class="language-rust">fn main() {
    let a = 1 &lt; 2;
}
</code></pre>"#),
            "```rust\nfn main() {\n    let a = 1 < 2;\n}\n```"
        );
        assert_eq!(
            md(
                "<table><tbody><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>x|y</td></tr></tbody></table>"
            ),
            "| A | B |\n| --- | --- |\n| 1 | x\\|y |"
        );
    }

    #[test]
    fn lists() {
        assert_eq!(
            md(
                "<ul><li><p>one</p></li><li><p>two</p><ol start=\"3\"><li><p>three</p></li></ol></li></ul><p>after</p>"
            ),
            "- one\n- two\n  3. three\n\nafter"
        );
        assert_eq!(
            md(
                r#"<ul data-type="taskList"><li data-checked="true" data-type="taskItem"><label><input type="checkbox" checked="checked"><span></span></label><div><p>Done</p></div></li><li data-checked="false" data-type="taskItem"><label><input type="checkbox"><span></span></label><div><p>To do</p></div></li></ul>"#
            ),
            "- [x] Done\n- [ ] To do"
        );
    }

    #[test]
    fn forgives_broken_html() {
        assert_eq!(md("<p>open <b>bold"), "open **bold**");
        assert_eq!(md("<p>a</span> b</p>"), "a b");
        assert_eq!(md("a < b and c > d"), r"a \< b and c > d");
        assert_eq!(md("<p>x<!-- note -->y</p><script>alert(1)</script>"), "xy");
        assert_eq!(md("&amp;amp; &#65;&#x42; &bogus;"), "&amp; AB &bogus;");
    }

    #[test]
    fn keeps_any_text() {
        assert_eq!(md("été <b>über</b> 日本"), "été **über** 日本");
        assert_eq!(md("<p>é<br>ü</p>"), "é\nü");
        for html in [
            "<",
            ">",
            "&",
            "<<>>",
            "</",
            "<a",
            "<a href=\"x",
            "<a href='x>y'>z</a>",
            "&#xFFFFFFFF; &#; &#x;",
            "<p>é",
            "é<",
            "<ul><li>",
            "<pre>",
            "<table><tr>",
            "<ol start=\"-1\"><li>x</li></ol>",
            "<h7>x</h7>",
            "<h>x</h>",
        ] {
            // Never panics, and always gives something to edit.
            assert!(md(html).len() <= html.len() * 4 + 16);
            assert!(to_markdown(html, LineBreaks::Hard).len() <= html.len() * 4 + 16);
        }
    }
}
