//! OpenDocument text and presentations (.odt, .odp) — LibreOffice's own
//! formats: a ZIP archive whose `content.xml` holds the whole document.
//!
//! Elements are matched by the prefixes the specification uses (`text:`,
//! `table:`, `draw:`…), which is what every producer writes.

use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

use super::html::{push_break, push_collapsed, tidy};
use super::zipxml;

/// Subtrees whose text is not part of the document's content: the record
/// of deleted text kept by change tracking, comments, footnote markers,
/// and the alternative text of drawings.
const SKIPPED: &[&[u8]] = &[
    b"text:tracked-changes",
    b"office:annotation",
    b"text:note-citation",
    b"svg:title",
    b"svg:desc",
];

pub fn extract_text(path: &Path) -> Result<String> {
    let mut archive = zipxml::open(path, "OpenDocument file")?;
    if let Some(manifest) = zipxml::read_entry(&mut archive, "META-INF/manifest.xml")? {
        if manifest.contains("encryption-data") {
            anyhow::bail!("this OpenDocument file is password-protected");
        }
    }
    let content = zipxml::read_entry(&mut archive, "content.xml")?
        .context("not an OpenDocument file: it has no content.xml")?;
    text_of_content(&content)
}

/// The text of `content.xml`: paragraphs and headings on lines of their
/// own, the cells of a table row separated by tabs, and — in a
/// presentation — a blank line between slides, whose speaker notes follow
/// their text.
fn text_of_content(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut slides: Vec<String> = Vec::new();
    let mut out = String::new();
    let mut text = String::new();
    let mut skipping = 0usize;
    let mut paragraphs = 0usize;
    let mut cells = 0usize;
    loop {
        let event = reader.read_event().with_context(|| {
            format!(
                "content.xml is not well-formed XML (at byte {})",
                reader.buffer_position()
            )
        })?;
        if skipping > 0 {
            match event {
                Event::Start(_) => skipping += 1,
                Event::End(_) => skipping -= 1,
                _ => {}
            }
            continue;
        }
        match &event {
            Event::Eof => break,
            Event::Start(e) => match e.name().as_ref() {
                name if SKIPPED.contains(&name) => skipping = 1,
                b"text:p" | b"text:h" => {
                    // A paragraph inside another — a footnote, a text box —
                    // starts on its own.
                    if paragraphs > 0 {
                        push_break(&mut out, cells > 0);
                    }
                    paragraphs += 1;
                }
                b"table:table-cell" => cells += 1,
                _ => {}
            },
            Event::End(e) => match e.name().as_ref() {
                b"text:p" | b"text:h" => {
                    paragraphs = paragraphs.saturating_sub(1);
                    push_break(&mut out, cells > 0);
                }
                b"table:table-cell" => {
                    cells = cells.saturating_sub(1);
                    end_cell(&mut out);
                }
                b"table:table-row" => out.push('\n'),
                b"draw:page" => slides.push(tidy(&std::mem::take(&mut out))),
                _ => {}
            },
            Event::Empty(e) => match e.name().as_ref() {
                b"text:s" => out.push(' '),
                b"text:tab" => out.push('\t'),
                b"text:line-break" | b"text:p" | b"text:h" => push_break(&mut out, cells > 0),
                b"table:table-cell" => end_cell(&mut out),
                _ => {}
            },
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if paragraphs > 0 => {
                // Whitespace in a paragraph collapses as in HTML; the
                // spaces meant to be kept are written as <text:s/>.
                text.clear();
                zipxml::push_text(&mut text, &event);
                push_collapsed(&mut out, &text);
            }
            _ => {}
        }
    }
    slides.push(tidy(&out));
    slides.retain(|slide| !slide.is_empty());
    Ok(slides.join("\n\n"))
}

/// Ends a table cell with a tab, without the space its last paragraph
/// left behind.
fn end_cell(out: &mut String) {
    out.truncate(out.trim_end_matches(' ').len());
    out.push('\t');
}

#[cfg(test)]
mod tests {
    use super::*;

    const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/documents/testdata");

    #[test]
    fn a_text_document_is_read() {
        let text = extract_text(&Path::new(TESTDATA).join("book.odt")).unwrap();
        assert!(text.starts_with("OpenDocument fixture\n"), "{text}");
        for expected in [
            "First paragraph with accents: città, perché — and the euro sign €.",
            "Words separated by runs of spaces.",
            "Quarter\tAmount",
            "Q1\t1250",
            "Closing line of the document.",
        ] {
            assert!(
                text.contains(expected),
                "{expected:?} missing from:\n{text}"
            );
        }
    }

    #[test]
    fn a_presentation_is_read_slide_by_slide_with_its_notes() {
        let text = extract_text(&Path::new(TESTDATA).join("deck.odp")).unwrap();
        let slides: Vec<&str> = text.split("\n\n").collect();
        assert_eq!(slides.len(), 3, "{text}");
        assert!(slides[0].contains("Quarterly review"), "{text}");
        assert!(
            slides[1].contains("Revenue grew in Città di Roma"),
            "{text}"
        );
        assert!(
            slides[1].contains("Speaker note: mention the Q2 dip"),
            "{text}"
        );
        assert!(slides[2].contains("Q1\t1250"), "{text}");
    }

    #[test]
    fn what_is_not_content_is_left_out() {
        let xml = r#"<office:document-content><office:body><office:text>
            <text:tracked-changes><text:changed-region><text:deletion>
              <text:p>Deleted words</text:p></text:deletion></text:changed-region></text:tracked-changes>
            <text:h>Title</text:h>
            <text:p>One<text:s text:c="4000000000"/>two<text:tab/>three<text:line-break/>four
              <office:annotation><text:p>A reviewer's comment</text:p></office:annotation>
              five<text:note><text:note-citation>1</text:note-citation>
              <text:note-body><text:p>The footnote</text:p></text:note-body></text:note></text:p>
            <text:p/>
            <table:table><table:table-row><table:table-cell><text:p>a</text:p><text:p>b</text:p></table:table-cell>
              <table:table-cell/><table:table-cell><text:p>c &amp; d</text:p></table:table-cell></table:table-row></table:table>
            </office:text></office:body></office:document-content>"#;
        assert_eq!(
            text_of_content(xml).unwrap(),
            "Title\nOne two\tthree\nfour five\nThe footnote\na b\t\tc & d"
        );
    }

    #[test]
    fn broken_xml_is_an_error() {
        assert!(text_of_content("<office:text><text:p>unclosed</office:text>").is_err());
    }
}
