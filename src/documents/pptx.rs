//! PowerPoint 2007 and later (.pptx): a ZIP archive with one XML part per
//! slide. Slides are read in the order the presentation lists them — not
//! in their file names' order, which moving a slide does not change — each
//! followed by the text of its SmartArt diagrams and its speaker notes.

use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

use super::html::{push_break, tidy};
use super::zipxml::{self, Archive};

/// Subtrees whose text is not the slide's own: fields — the slide number,
/// today's date — and the fallback copy of content that also has a
/// primary version, which would otherwise be read twice.
const SKIPPED: &[&[u8]] = &[b"a:fld", b"mc:Fallback"];

pub fn extract_text(path: &Path) -> Result<String> {
    let mut archive = zipxml::open(path, "presentation")?;
    let mut slides = Vec::new();
    for slide in slide_parts(&mut archive)? {
        let Some(xml) = zipxml::read_entry(&mut archive, &slide)? else {
            continue;
        };
        let mut parts = vec![drawing_text(&xml).with_context(|| format!("reading {slide}"))?];
        let related = relationships(&mut archive, &slide)?;
        for kind in ["diagramData", "notesSlide"] {
            for rel in related.iter().filter(|rel| rel.kind == kind) {
                if let Some(xml) = zipxml::read_entry(&mut archive, &rel.target)? {
                    parts.push(
                        drawing_text(&xml).with_context(|| format!("reading {}", rel.target))?,
                    );
                }
            }
        }
        parts.retain(|part| !part.is_empty());
        slides.push(parts.join("\n"));
    }
    slides.retain(|slide| !slide.is_empty());
    Ok(slides.join("\n\n"))
}

/// The slide parts, in presentation order.
fn slide_parts(archive: &mut Archive) -> Result<Vec<String>> {
    let presentation = relationships(archive, "")?
        .into_iter()
        .find(|rel| rel.kind == "officeDocument")
        .map_or_else(|| "ppt/presentation.xml".to_owned(), |rel| rel.target);
    let xml = zipxml::read_entry(archive, &presentation)?
        .with_context(|| format!("not a PowerPoint presentation: it has no {presentation}"))?;
    let related = relationships(archive, &presentation)?;
    let mut slides = Vec::new();
    let mut reader = Reader::from_str(&xml);
    loop {
        match reader
            .read_event()
            .with_context(|| format!("reading {presentation}"))?
        {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == b"p:sldId" => {
                // The relationship id is the prefixed `r:id`; the plain `id`
                // next to it is a number with no use here.
                let id = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref().ends_with(b":id"))
                    .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok());
                if let Some(rel) = id.and_then(|id| related.iter().find(|rel| rel.id == *id)) {
                    slides.push(rel.target.clone());
                }
            }
            _ => {}
        }
    }
    Ok(slides)
}

struct Relationship {
    id: String,
    /// The last segment of its type URI: `slide`, `notesSlide`…
    kind: String,
    /// The entry it points to.
    target: String,
}

/// The relationships of `part` (the package itself when empty) to other
/// entries of the archive; links to anything outside it are left out.
fn relationships(archive: &mut Archive, part: &str) -> Result<Vec<Relationship>> {
    let dir = zipxml::dir_of(part);
    let file = part.rsplit('/').next().unwrap_or_default();
    let rels = zipxml::resolve(dir, &format!("_rels/{file}.rels"));
    let Some(xml) = zipxml::read_entry(archive, &rels)? else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    let mut reader = Reader::from_str(&xml);
    loop {
        match reader
            .read_event()
            .with_context(|| format!("reading {rels}"))?
        {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                if zipxml::attr(&e, b"TargetMode").as_deref() == Some("External") {
                    continue;
                }
                let (Some(id), Some(kind), Some(target)) = (
                    zipxml::attr(&e, b"Id"),
                    zipxml::attr(&e, b"Type"),
                    zipxml::attr(&e, b"Target"),
                ) else {
                    continue;
                };
                found.push(Relationship {
                    id,
                    kind: kind.rsplit('/').next().unwrap_or_default().to_owned(),
                    target: zipxml::resolve(dir, &target),
                });
            }
            _ => {}
        }
    }
    Ok(found)
}

/// The text of a DrawingML part — a slide, its notes, a SmartArt diagram:
/// each paragraph on a line of its own, the cells of a table row separated
/// by tabs.
fn drawing_text(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    let mut skipping = 0usize;
    let mut in_run = false;
    let mut cells = 0usize;
    loop {
        let event = reader.read_event().with_context(|| {
            format!("not well-formed XML (at byte {})", reader.buffer_position())
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
                b"a:t" => in_run = true,
                b"a:tc" => cells += 1,
                _ => {}
            },
            Event::End(e) => match e.name().as_ref() {
                b"a:t" => in_run = false,
                // A line break usually carries its formatting, <a:br><a:rPr/></a:br>.
                b"a:p" | b"a:br" => push_break(&mut out, cells > 0),
                b"a:tc" => {
                    cells = cells.saturating_sub(1);
                    out.truncate(out.trim_end_matches(' ').len());
                    out.push('\t');
                }
                b"a:tr" => out.push('\n'),
                _ => {}
            },
            Event::Empty(e) => match e.name().as_ref() {
                b"a:br" => push_break(&mut out, cells > 0),
                b"a:tc" => out.push('\t'),
                _ => {}
            },
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if in_run => {
                zipxml::push_text(&mut out, &event);
            }
            _ => {}
        }
    }
    Ok(tidy(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/documents/testdata");

    #[test]
    fn slides_come_in_presentation_order_with_their_notes() {
        let text = extract_text(&Path::new(TESTDATA).join("deck.pptx")).unwrap();
        let slides: Vec<&str> = text.split("\n\n").collect();
        assert_eq!(slides.len(), 3, "{text}");
        assert_eq!(
            slides[0], "Quarterly review\nOpening slide, stored last",
            "slide3.xml is listed first in the presentation"
        );
        assert_eq!(
            slides[1],
            "Results\nRevenue grew in Città di Roma\nCosts fell by 3 €\nSpeaker note: mention the Q2 dip"
        );
        assert_eq!(slides[2], "Figures\nQuarter\tAmount\nQ1\t1250\nQ2\t980");
    }

    #[test]
    fn line_breaks_are_kept_and_fields_and_fallback_copies_are_not_read() {
        let xml = r#"<p:sld><p:cSld><p:spTree><p:sp><p:txBody>
            <a:p><a:r><a:t>Line &amp; one</a:t></a:r><a:br/><a:r><a:t>line two</a:t></a:r><a:br><a:rPr lang="en-US"/></a:br><a:r><a:t>three</a:t></a:r></a:p>
            <a:p><a:fld type="slidenum"><a:t>7</a:t></a:fld></a:p>
            </p:txBody></p:sp>
            <mc:AlternateContent><mc:Choice><a:p><a:r><a:t>Equation</a:t></a:r></a:p></mc:Choice>
            <mc:Fallback><a:p><a:r><a:t>Equation</a:t></a:r></a:p></mc:Fallback></mc:AlternateContent>
            </p:spTree></p:cSld></p:sld>"#;
        assert_eq!(
            drawing_text(xml).unwrap(),
            "Line & one\nline two\nthree\nEquation"
        );
    }
}
