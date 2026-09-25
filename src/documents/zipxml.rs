//! Shared plumbing for the formats that are ZIP archives of XML —
//! OpenDocument, PowerPoint, EPUB: opening the archive behind the
//! decompression-bomb check, reading one entry, and turning XML text back
//! into text, including the entity and character references quick-xml
//! reports as events of their own.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};

pub type Archive = zip::ZipArchive<std::fs::File>;

/// Opens the archive at `path`, refusing it first if its entries declare
/// more uncompressed data than the parser allows.
pub fn open(path: &Path, kind: &str) -> Result<Archive> {
    super::parser::reject_decompression_bomb(path, kind)?;
    let file =
        std::fs::File::open(path).with_context(|| format!("opening {kind} {}", path.display()))?;
    zip::ZipArchive::new(file)
        .with_context(|| format!("{kind} is not a readable zip archive: {}", path.display()))
}

/// The entry `name` as text, or `None` when the archive has no such entry.
/// The read stops at the entry's declared size — which the bomb check has
/// already bounded — so a header that understates it cannot make it grow.
/// A UTF-16 byte-order mark is honoured; anything else is read as UTF-8.
pub fn read_entry(archive: &mut Archive, name: &str) -> Result<Option<String>> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {name}")),
    };
    let declared = entry.size();
    let mut bytes = Vec::new();
    entry
        .take(declared)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {name}"))?;
    // `decode` sniffs the byte-order mark before falling back to UTF-8.
    let (text, _, _) = encoding_rs::UTF_8.decode(&bytes);
    Ok(Some(text.into_owned()))
}

/// Appends the text a text-bearing event carries: plain text, CDATA, and
/// the references quick-xml reports separately — the five predefined
/// entities and numeric character references. Any other named entity would
/// need a DTD, which these formats do not use, so it is dropped.
pub fn push_text(out: &mut String, event: &Event<'_>) {
    match event {
        Event::Text(t) => {
            if let Ok(text) = t.decode() {
                out.push_str(&text);
            }
        }
        Event::CData(t) => {
            if let Ok(text) = t.decode() {
                out.push_str(&text);
            }
        }
        Event::GeneralRef(r) => {
            if let Ok(Some(c)) = r.resolve_char_ref() {
                out.push(c);
            } else if let Ok(name) = r.decode() {
                if let Some(text) = quick_xml::escape::resolve_predefined_entity(&name) {
                    out.push_str(text);
                }
            }
        }
        _ => {}
    }
}

/// The value of the attribute whose local name is `local`, whatever its
/// prefix.
pub fn attr(element: &BytesStart<'_>, local: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == local)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

/// Resolves `target` — relative to the directory `base` inside the
/// archive, or absolute from its root when it starts with `/` — into an
/// entry name. Percent-escapes are decoded, a `#fragment` is dropped, and
/// `..` cannot climb above the root.
pub fn resolve(base: &str, target: &str) -> String {
    let target = percent_decode(target.split('#').next().unwrap_or_default());
    let mut parts: Vec<String> = if target.starts_with('/') {
        Vec::new()
    } else {
        base.split('/')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect()
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s.to_owned()),
        }
    }
    parts.join("/")
}

/// The directory part of an entry name (`"a/b/c.xml"` → `"a/b"`).
pub fn dir_of(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::Reader;
    use std::io::Write;

    fn all_text(xml: &str) -> String {
        let mut reader = Reader::from_str(xml);
        let mut out = String::new();
        loop {
            match reader.read_event() {
                Ok(Event::Eof) | Err(_) => break,
                Ok(event) => push_text(&mut out, &event),
            }
        }
        out
    }

    #[test]
    fn references_become_the_characters_they_stand_for() {
        assert_eq!(
            all_text("<a>Tom &amp; Jerry &lt;3 &#233;t&#xE9;</a>"),
            "Tom & Jerry <3 été"
        );
        assert_eq!(all_text("<a><![CDATA[x < y]]></a>"), "x < y");
    }

    #[test]
    fn a_utf16_entry_is_read_by_its_byte_order_mark() {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?><p>città €</p>"#;
        let units = || xml.encode_utf16();
        let le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(units().flat_map(u16::to_le_bytes))
            .collect();
        let be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain(units().flat_map(u16::to_be_bytes))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("parts.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, bytes) in [("le.xml", &le), ("be.xml", &be)] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let mut archive = open(&path, "ZIP").unwrap();
        for name in ["le.xml", "be.xml"] {
            let text = read_entry(&mut archive, name).unwrap().unwrap();
            assert_eq!(text, xml, "{name}");
            assert_eq!(all_text(&text), "città €", "{name}");
        }
    }

    #[test]
    fn paths_resolve_inside_the_archive() {
        assert_eq!(resolve("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(
            resolve("ppt/slides", "../notesSlides/n1.xml"),
            "ppt/notesSlides/n1.xml"
        );
        assert_eq!(resolve("ppt", "/ppt/slides/s.xml"), "ppt/slides/s.xml");
        assert_eq!(
            resolve("OEBPS", "text/ch%201.xhtml#part"),
            "OEBPS/text/ch 1.xhtml"
        );
        assert_eq!(
            resolve("a", "../../../etc/passwd"),
            "etc/passwd",
            "cannot climb out"
        );
        assert_eq!(dir_of("OEBPS/content.opf"), "OEBPS");
        assert_eq!(dir_of("content.opf"), "");
    }
}
