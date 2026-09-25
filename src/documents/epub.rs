//! EPUB e-books: a ZIP archive of XHTML chapters. `META-INF/container.xml`
//! names the package document, whose spine lists the chapters in reading
//! order.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

use super::zipxml::{self, Archive};

pub fn extract_text(path: &Path) -> Result<String> {
    let mut archive = zipxml::open(path, "EPUB")?;
    let container = zipxml::read_entry(&mut archive, "META-INF/container.xml")?
        .context("not an EPUB: it has no META-INF/container.xml")?;
    let package = package_path(&container)?
        .context("not an EPUB: its container names no package document")?;
    let opf = zipxml::read_entry(&mut archive, &package)?
        .with_context(|| format!("not an EPUB: it has no {package}"))?;
    let protected = encrypted(&mut archive)?;
    let mut chapters = Vec::new();
    for chapter in reading_order(&opf, zipxml::dir_of(&package))? {
        if protected.contains(&chapter) {
            anyhow::bail!("this EPUB is protected by DRM");
        }
        if let Some(xhtml) = zipxml::read_entry(&mut archive, &chapter)? {
            chapters.push(super::html::xhtml_to_text(&xhtml));
        }
    }
    chapters.retain(|chapter| !chapter.is_empty());
    Ok(chapters.join("\n\n"))
}

/// The entry name of the package document: the first `rootfile` of its
/// media type.
fn package_path(container: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(container);
    loop {
        match reader
            .read_event()
            .context("reading META-INF/container.xml")?
        {
            Event::Eof => return Ok(None),
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"rootfile" => {
                let media_type = zipxml::attr(&e, b"media-type");
                if matches!(
                    media_type.as_deref(),
                    None | Some("application/oebps-package+xml")
                ) {
                    if let Some(path) = zipxml::attr(&e, b"full-path") {
                        return Ok(Some(zipxml::resolve("", &path)));
                    }
                }
            }
            _ => {}
        }
    }
}

/// The chapters' entry names in reading order: the spine's (X)HTML items,
/// except the navigation document — a table of contents repeating the
/// chapter titles.
fn reading_order(opf: &str, base: &str) -> Result<Vec<String>> {
    let mut manifest: HashMap<String, Option<String>> = HashMap::new();
    let mut spine = Vec::new();
    let mut reader = Reader::from_str(opf);
    loop {
        match reader
            .read_event()
            .context("reading the EPUB package document")?
        {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => match e.local_name().as_ref() {
                b"item" => {
                    let (Some(id), Some(href)) =
                        (zipxml::attr(&e, b"id"), zipxml::attr(&e, b"href"))
                    else {
                        continue;
                    };
                    let html = matches!(
                        zipxml::attr(&e, b"media-type").as_deref(),
                        Some("application/xhtml+xml" | "text/html")
                    );
                    let nav = zipxml::attr(&e, b"properties")
                        .is_some_and(|p| p.split_ascii_whitespace().any(|p| p == "nav"));
                    manifest.insert(id, (html && !nav).then(|| zipxml::resolve(base, &href)));
                }
                b"itemref" => spine.extend(zipxml::attr(&e, b"idref")),
                _ => {}
            },
            _ => {}
        }
    }
    Ok(spine
        .iter()
        .filter_map(|id| manifest.get(id).cloned().flatten())
        .collect())
}

/// The entries `META-INF/encryption.xml` declares encrypted. Fonts often
/// are — obfuscated, not protected, and irrelevant here — but a chapter
/// that is means the book is under DRM.
fn encrypted(archive: &mut Archive) -> Result<HashSet<String>> {
    let Some(xml) = zipxml::read_entry(archive, "META-INF/encryption.xml")? else {
        return Ok(HashSet::new());
    };
    let mut found = HashSet::new();
    let mut reader = Reader::from_str(&xml);
    loop {
        match reader
            .read_event()
            .context("reading META-INF/encryption.xml")?
        {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"CipherReference" => {
                found.extend(zipxml::attr(&e, b"URI").map(|uri| zipxml::resolve("", &uri)));
            }
            _ => {}
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/documents/testdata");

    const CONTAINER: &str = r#"<?xml version="1.0"?>
        <container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
          <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
        </container>"#;

    const PACKAGE: &str = r#"<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf" version="3.0">
          <manifest>
            <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
            <item id="one" href="text/one.xhtml" media-type="application/xhtml+xml"/>
            <item id="two" href="text/chapter%20two.xhtml" media-type="application/xhtml+xml"/>
            <item id="cover" href="cover.jpg" media-type="image/jpeg"/>
          </manifest>
          <spine><itemref idref="nav"/><itemref idref="two"/><itemref idref="cover"/><itemref idref="one"/></spine>
        </package>"#;

    fn chapter(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
             <html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title/></head><body>{body}</body></html>"
        )
    }

    fn epub(entries: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.epub");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, content) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        (dir, path)
    }

    #[test]
    fn a_real_epub_is_read() {
        let text = extract_text(&Path::new(TESTDATA).join("book.epub")).unwrap();
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
    fn chapters_follow_the_spine_not_the_manifest() {
        let one = chapter("<h1>Chapter one</h1><p>First.</p>");
        let two = chapter("<h1>Chapter two</h1><p>Second.</p>");
        let nav = chapter("<nav><ol><li>Chapter one</li><li>Chapter two</li></ol></nav>");
        let (_dir, path) = epub(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", PACKAGE),
            ("OEBPS/nav.xhtml", &nav),
            ("OEBPS/text/one.xhtml", &one),
            ("OEBPS/text/chapter two.xhtml", &two),
        ]);
        assert_eq!(
            extract_text(&path).unwrap(),
            "Chapter two\nSecond.\n\nChapter one\nFirst."
        );
    }

    #[test]
    fn a_book_under_drm_is_refused() {
        let encryption = r#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"
            xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
            <enc:EncryptedData><enc:CipherData><enc:CipherReference URI="OEBPS/text/one.xhtml"/></enc:CipherData></enc:EncryptedData>
            </encryption>"#;
        let (_dir, path) = epub(&[
            ("META-INF/container.xml", CONTAINER),
            ("META-INF/encryption.xml", encryption),
            ("OEBPS/content.opf", PACKAGE),
            ("OEBPS/text/one.xhtml", "\u{1}\u{2}ciphertext"),
            ("OEBPS/text/chapter two.xhtml", &chapter("<p>Readable</p>")),
        ]);
        let err = extract_text(&path).unwrap_err().to_string();
        assert!(err.contains("DRM"), "{err}");
    }

    #[test]
    fn a_zip_that_is_not_an_epub_says_so() {
        let (_dir, path) = epub(&[("word/document.xml", "<w:document/>")]);
        let err = extract_text(&path).unwrap_err().to_string();
        assert!(err.contains("not an EPUB"), "{err}");
    }
}
