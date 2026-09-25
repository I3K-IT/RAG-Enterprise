//! HTML to plain text: for .html pages, and for the HTML inside EPUB books
//! and e-mails.
//!
//! Keeps every piece of text a browser would show, in document order. A
//! block element — paragraph, heading, list item, `div`, table row… — goes
//! on a line of its own, the cells of a table row are separated by tabs,
//! and runs of whitespace collapse to one space as a browser collapses
//! them, except inside `pre`. What a browser never shows — `head`, scripts,
//! styles — is left out.

use scraper::{Html, Node};

/// Never shown.
const HIDDEN: &[&str] = &["head", "script", "style", "noscript", "template"];

/// Shown on lines of their own.
const BLOCKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "caption",
    "center",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "legend",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tr",
    "ul",
];

/// Keep their whitespace as written.
const PREFORMATTED: &[&str] = &["pre", "textarea", "listing", "plaintext", "xmp"];

/// Raw-text elements: HTML ignores the `/>` that closes them in XHTML, so
/// `<title/>` would swallow the rest of the page as its own (hidden) text.
const RAW_TEXT: &[&str] = &[
    "title", "script", "style", "textarea", "xmp", "iframe", "noembed", "noframes", "noscript",
];

pub fn to_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut out = String::new();
    let mut preformatted = 0usize;
    let mut cells = 0usize;
    // An explicit stack rather than recursion: a page nested thousands of
    // elements deep must not be able to overflow the thread's stack —
    // unlike a panic, nothing could catch that.
    let mut stack = vec![(*document.root_element(), false)];
    while let Some((node, closing)) = stack.pop() {
        let element = match node.value() {
            Node::Text(text) if preformatted > 0 => {
                out.push_str(text);
                continue;
            }
            Node::Text(text) => {
                push_collapsed(&mut out, text);
                continue;
            }
            Node::Element(element) => element,
            _ => continue,
        };
        let name = element.name();
        if closing {
            if PREFORMATTED.contains(&name) {
                preformatted -= 1;
            }
            match name {
                "td" | "th" => {
                    cells -= 1;
                    // Without the space a paragraph in the cell left behind.
                    out.truncate(out.trim_end_matches(' ').len());
                    out.push('\t');
                }
                _ if BLOCKS.contains(&name) => push_break(&mut out, cells > 0),
                _ => {}
            }
            continue;
        }
        if HIDDEN.contains(&name) {
            continue;
        }
        match name {
            "br" => push_break(&mut out, cells > 0),
            "td" | "th" => cells += 1,
            _ if BLOCKS.contains(&name) => push_break(&mut out, cells > 0),
            _ => {}
        }
        if PREFORMATTED.contains(&name) {
            preformatted += 1;
        }
        stack.push((node, true));
        stack.extend(node.children().rev().map(|child| (child, false)));
    }
    tidy(&out)
}

/// `to_text` for XHTML, as EPUB books are written: first gives each
/// self-closed raw-text element, such as `<title/>`, the end tag HTML
/// needs.
pub fn xhtml_to_text(xhtml: &str) -> String {
    let mut fixed = String::with_capacity(xhtml.len());
    let mut rest = xhtml;
    while let Some(start) = rest.find('<') {
        fixed.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..=end];
        fixed.push_str(tag);
        rest = &rest[end + 1..];
        if let Some(name) = self_closed_raw_text(tag) {
            fixed.push_str("</");
            fixed.push_str(name);
            fixed.push('>');
        }
    }
    fixed.push_str(rest);
    to_text(&fixed)
}

/// The element name, when `tag` is a raw-text element closed by `/>`.
fn self_closed_raw_text(tag: &str) -> Option<&str> {
    let inner = tag.strip_prefix('<')?.strip_suffix("/>")?;
    let name = inner.split(|c: char| c.is_ascii_whitespace()).next()?;
    RAW_TEXT
        .iter()
        .copied()
        .find(|raw| raw.eq_ignore_ascii_case(name))
}

/// Appends `text` with each run of whitespace collapsed to a single space,
/// and none at the start of a line.
pub(crate) fn push_collapsed(out: &mut String, text: &str) {
    for c in text.chars() {
        if !c.is_ascii_whitespace() {
            out.push(c);
        } else if !matches!(out.chars().next_back(), None | Some(' ' | '\n' | '\t')) {
            out.push(' ');
        }
    }
}

/// Ends the line — or, inside a table cell, whose row stays on one line,
/// leaves a single space instead.
pub(crate) fn push_break(out: &mut String, in_cell: bool) {
    if !in_cell {
        out.push('\n');
    } else if !matches!(out.chars().next_back(), None | Some(' ' | '\n' | '\t')) {
        out.push(' ');
    }
}

/// Trims every line and drops the empty ones.
pub(crate) fn tidy(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_outside_paragraphs_is_kept_once() {
        let html = "<html><head><title>Tab title</title><style>p { color: red }</style></head>\
            <body><div>In a div<br>after a break</div>\
            <ul><li><p>Item holding a paragraph</p></li></ul>\
            <blockquote>Quoted</blockquote>\
            <script>var hidden = 1;</script></body></html>";
        assert_eq!(
            to_text(html),
            "In a div\nafter a break\nItem holding a paragraph\nQuoted",
            "the old selector dropped the div and the blockquote and read the item twice"
        );
    }

    #[test]
    fn whitespace_collapses_except_in_pre() {
        let html = "<p>Words   separated\n   by <b>runs</b>  of spaces.</p><pre>keep   this\n  as is</pre>";
        assert_eq!(
            to_text(html),
            "Words separated by runs of spaces.\nkeep   this\nas is"
        );
    }

    #[test]
    fn a_table_row_stays_on_one_line() {
        let html = "<table><tr><th>Quarter</th><th>Amount</th></tr>\
            <tr><td><p>Q1</p></td><td><p>1250</p><p>€</p></td></tr></table>";
        assert_eq!(to_text(html), "Quarter\tAmount\nQ1\t1250 €");
    }

    #[test]
    fn a_self_closed_title_does_not_swallow_the_book() {
        let xhtml = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
            <html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title/>\
            <script src=\"a.js\" /></head><body><p>Chapter text</p><a id=\"p1\"/>After the anchor</body></html>";
        assert_eq!(xhtml_to_text(xhtml), "Chapter text\nAfter the anchor");
        assert_eq!(
            to_text(xhtml),
            "",
            "read as plain HTML, <title/> hides the whole body"
        );
    }

    #[test]
    fn nesting_deeper_than_any_stack_is_fine() {
        let html = format!("{}deep{}", "<div>".repeat(20_000), "</div>".repeat(20_000));
        assert_eq!(to_text(&html), "deep");
    }
}
