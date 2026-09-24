//! Document text extraction: one reader per format, chosen by extension —
//! and, where one format is often found under another's name, by what the
//! file contains.
//!
//! - .txt/.md/.csv → read directly as UTF-8
//! - PDF → pdf_oxide (text_ratio > 0.5 AND len > 500), with an OCR fallback
//!   below that ratio (see ocr.rs)
//! - DOCX → docx-rs; DOC → msdoc.rs (or rtf.rs, or docx-rs, by content);
//!   ODT/ODP → odf.rs; RTF → rtf.rs
//! - XLSX/XLSM/XLSB/XLS/ODS → calamine
//! - PPTX → pptx.rs; PPT → ppt.rs (either, by content)
//! - HTML → html.rs; EPUB → epub.rs
//! - EML → eml.rs (mail-parser); MSG → msg.rs
//! - PNG/JPEG/TIFF/BMP/GIF/WebP → OCR (ocr.rs)
//!
//! Chunking happens in rag::chunker, NOT here.

use std::path::Path;
use anyhow::{Context, Result};

/// Bumped whenever extraction logic changes in a way that could alter the
/// resulting text or page spans for an unchanged input file — e.g. the
/// native/OCR acceptance threshold below, the OCR rasterisation DPI, or a
/// fix to how page spans are computed. Version 2: HTML keeps every visible
/// piece of text, where it used to keep only paragraphs, headings, list
/// items and table cells. Baked into every
/// `rag::chunker::provenance_id` (as `pv{VERSION}`), independently of
/// `rag::chunker::CHUNKING_CONFIG_VERSION`: an extraction change and a
/// chunking change are different pipeline stages with different change
/// cadences, so conflating them into one counter would make old ids less
/// diagnostic when something changes.
pub const EXTRACTION_CONFIG_VERSION: u32 = 2;

/// One page's byte-offset span `[start_byte, end_byte)` within
/// `ExtractedText::text`. `page` is 1-based. Only PDFs (native or OCR) and
/// pictures (one page, or one per page of a multi-page TIFF) have pages —
/// every other format leaves `ExtractedText::pages` empty, since none has a
/// page concept that survives extraction into flat text today. BYTE offsets, not char offsets — see
/// rag::chunker::Chunk for why that's named explicitly rather than left
/// implicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSpan {
    pub page: u32,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Result of extracting a document: the flat text chunker::split_text
/// operates on, the page count (None when the format has no pages), and —
/// PDF only — the byte-offset span of each page within `text`, so a later
/// chunk's `[start_byte, end_byte)` (see rag::chunker::Chunk) can be mapped
/// back to the page(s) it came from via `pages_for_range`.
#[derive(Debug, Clone, Default)]
pub struct ExtractedText {
    pub text: String,
    pub page_count: Option<u32>,
    pub pages: Vec<PageSpan>,
}

/// Extensions `extract_text` below knows how to read.
///
/// Exported so the upload handler can refuse an unsupported file BEFORE
/// streaming it to disk: an unsupported file used to be written out in full —
/// up to the configured limit — and only then rejected by the match in
/// extract_text. The two are pinned together by a test in this module, and
/// the web UI's file picker to this list by another; if you add an arm
/// below, add it here and there too or those tests fail.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "txt", "md", "csv", "pdf", "docx", "doc", "odt", "rtf", "xlsx", "xls", "xlsm", "xlsb", "ods", "pptx",
    "ppt", "odp", "epub", "html", "htm", "eml", "msg", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif",
    "webp",
];

/// Whether `ext` (lowercase, no dot) is one this parser can read.
pub fn is_supported_extension(ext: &str) -> bool {
    SUPPORTED_EXTENSIONS.contains(&ext)
}

/// Extracts the text (and, for PDFs and pictures, the per-page spans within
/// it).
///
/// `data_dir` is the data root (`Settings.data.data_path()`), needed only by
/// OCR — of scanned PDFs and of pictures — to find `{data_dir}/tessdata/`,
/// where the manifest downloads it.
pub fn extract_text(path: &Path, data_dir: &Path) -> Result<ExtractedText> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    guarded(&ext, || read_by_extension(path, data_dir, &ext))
}

/// Runs one format's reader and turns a panic inside it into an error.
///
/// Every reader — ours or a third-party crate — parses bytes an uploader
/// chose, so a panic in one is a bug in that reader, but it must reach the
/// caller as "this file cannot be read", not unwind through it. calamine
/// 0.36.1 (the current release) panics on an .xlsb whose sheet relationship
/// it looks up without checking: the upload answered 500 "parse task
/// panicked" instead of 422, and `--bench` would simply have crashed. The
/// release profile unwinds (no `panic = "abort"`), so this does catch it.
fn guarded<T>(ext: &str, read: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)) {
        Ok(result) => result,
        Err(panic) => {
            let why = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            tracing::warn!(ext, panic = %why, "a document reader panicked");
            Err(anyhow::anyhow!("this .{ext} file could not be read: its reader failed on it"))
        }
    }
}

fn read_by_extension(path: &Path, data_dir: &Path, ext: &str) -> Result<ExtractedText> {
    if matches!(ext, "docx" | "xlsx" | "xlsm" | "xlsb" | "pptx") && is_encrypted_ooxml(path) {
        anyhow::bail!("this .{ext} is password-protected, so its text cannot be read");
    }
    match ext {
        "txt" | "md" | "csv" => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            Ok(ExtractedText { text, ..Default::default() })
        }
        "pdf" => extract_pdf(path, data_dir),
        "docx" => extract_docx(path).map(|text| ExtractedText { text, ..Default::default() }),
        "doc" => extract_doc(path).map(|text| ExtractedText { text, ..Default::default() }),
        "xlsx" | "xls" | "xlsm" | "xlsb" | "ods" => extract_xlsx(path).map(|text| ExtractedText { text, ..Default::default() }),
        "odt" | "odp" => super::odf::extract_text(path).map(|text| ExtractedText { text, ..Default::default() }),
        "pptx" | "ppt" => extract_presentation(path).map(|text| ExtractedText { text, ..Default::default() }),
        "epub" => super::epub::extract_text(path).map(|text| ExtractedText { text, ..Default::default() }),
        "rtf" => super::rtf::extract_text(path).map(|text| ExtractedText { text, ..Default::default() }),
        "eml" => super::eml::extract_text(path).map(|text| ExtractedText { text, ..Default::default() }),
        "msg" => super::msg::extract_text(path).map(|text| ExtractedText { text, ..Default::default() }),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" | "gif" | "webp" => super::ocr::ocr_picture(path, data_dir),
        "html" | "htm" => extract_html(path).map(|text| ExtractedText { text, ..Default::default() }),
        _ => Err(anyhow::anyhow!("unsupported format: .{ext}")),
    }
}

/// Given a chunk's byte-offset span `[start_byte, end_byte)` in the
/// extracted text and the page spans produced alongside it, returns the
/// inclusive `(first, last)` 1-based page range the chunk overlaps —
/// `None` when `pages` is empty (non-PDF, or a PDF whose extraction
/// produced no page spans). A chunk can legitimately straddle more than
/// one page (CHUNK_OVERLAP can bridge a page boundary), hence a range
/// rather than a single page.
pub fn pages_for_range(pages: &[PageSpan], start_byte: usize, end_byte: usize) -> Option<(u32, u32)> {
    let mut first: Option<u32> = None;
    let mut last: Option<u32> = None;
    for p in pages {
        // Half-open range overlap test: [p.start_byte, p.end_byte) intersects [start_byte, end_byte).
        if p.start_byte < end_byte && p.end_byte > start_byte {
            first = Some(first.map_or(p.page, |f| f.min(p.page)));
            last = Some(last.map_or(p.page, |l| l.max(p.page)));
        }
    }
    match (first, last) {
        (Some(f), Some(l)) => Some((f, l)),
        _ => None,
    }
}

// ── PDF ───────────────────────────────────────────────────────────────────────

fn extract_pdf(path: &Path, data_dir: &Path) -> Result<ExtractedText> {
    let doc = pdf_oxide::PdfDocument::open(path)
        .with_context(|| format!("pdf_oxide open {}", path.display()))?;
    let page_count = doc.page_count().context("pdf_oxide page_count")? as u32;

    let pages_to_check = (page_count as usize).min(10);
    let mut pages_with_text = 0usize;
    let mut full_text = String::new();
    let mut pages: Vec<PageSpan> = Vec::new();

    for i in 0..(page_count as usize) {
        let page_text = doc.extract_text(i).unwrap_or_default();
        let has_text = !page_text.trim().is_empty();
        if has_text && i < pages_to_check {
            pages_with_text += 1;
        }
        if has_text {
            let start = full_text.len();
            full_text.push_str(&page_text);
            full_text.push('\n');
            pages.push(PageSpan { page: (i + 1) as u32, start_byte: start, end_byte: full_text.len() });
        }
    }

    let text_ratio = if pages_to_check > 0 {
        pages_with_text as f32 / pages_to_check as f32
    } else {
        0.0
    };

    if text_ratio > 0.5 && full_text.trim().len() > 500 {
        tracing::info!(
            file = %path.display(),
            pages = page_count,
            chars = full_text.len(),
            "pdf_oxide ok"
        );
        return Ok(ExtractedText { text: full_text, page_count: Some(page_count), pages });
    }

    // Not enough text: probably a scanned page, fall back to OCR.
    tracing::info!(
        file = %path.display(),
        text_ratio = format!("{:.0}%", text_ratio * 100.0).as_str(),
        "text ratio too low, falling back to OCR"
    );
    let ocr = super::ocr::ocr_pdf(path, page_count, data_dir)?;
    Ok(ExtractedText { text: ocr.text, page_count: Some(page_count), pages: ocr.pages })
}

// ── DOCX ──────────────────────────────────────────────────────────────────────

fn extract_docx(path: &Path) -> Result<String> {
    reject_decompression_bomb(path, "docx")?;
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading docx {}", path.display()))?;
    let docx = docx_rs::read_docx(&bytes)
        .map_err(|e| anyhow::anyhow!("docx parse: {e:?}"))?;
    Ok(collect_docx_text(&docx))
}

/// Word 6.0–2003 (.doc). What a file named .doc contains is checked first:
/// a .docx renamed to .doc is common, and Word itself saves RTF under that
/// extension when asked to.
fn extract_doc(path: &Path) -> Result<String> {
    let head = head(path)?;
    if head.starts_with(&OLE_MAGIC) {
        return super::msdoc::extract_text(path, MAX_UNCOMPRESSED_BYTES);
    }
    if is_zip_magic(&head) {
        return extract_docx(path);
    }
    if head.starts_with(b"{\\rtf") {
        return super::rtf::extract_text(path);
    }
    if head.starts_with(&[0xDB, 0xA5]) {
        anyhow::bail!("this .doc was saved by Word 2.0, which is not supported — save it as .docx");
    }
    anyhow::bail!("this .doc is neither a Word 6.0–2003 document nor a .docx")
}

/// PowerPoint, 97–2003 (.ppt) or 2007 and later (.pptx), told apart by
/// what the file contains rather than its name: either is often found
/// renamed to the other.
fn extract_presentation(path: &Path) -> Result<String> {
    let head = head(path)?;
    if head.starts_with(&OLE_MAGIC) {
        return super::ppt::extract_text(path);
    }
    if is_zip_magic(&head) {
        return super::pptx::extract_text(path);
    }
    anyhow::bail!("this is neither a PowerPoint 97–2003 presentation nor a .pptx")
}

fn collect_docx_text(docx: &docx_rs::Docx) -> String {
    let mut out = String::new();
    for child in &docx.document.children {
        match child {
            docx_rs::DocumentChild::Paragraph(para) => {
                push_paragraph_text(&mut out, para);
                out.push('\n');
            }
            docx_rs::DocumentChild::Table(table) => {
                for row_child in &table.rows {
                    let docx_rs::TableChild::TableRow(row) = row_child;
                    for cell_child in &row.cells {
                        let docx_rs::TableRowChild::TableCell(cell) = cell_child;
                        for cell_content in &cell.children {
                            if let docx_rs::TableCellContent::Paragraph(para) = cell_content {
                                push_paragraph_text(&mut out, para);
                                out.push('\t');
                            }
                        }
                    }
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
    out
}

fn push_paragraph_text(out: &mut String, para: &docx_rs::Paragraph) {
    for pc in para.children() {
        if let docx_rs::ParagraphChild::Run(run) = pc {
            for rc in &run.children {
                if let docx_rs::RunChild::Text(t) = rc {
                    out.push_str(&t.text);
                }
            }
        }
    }
}

// ── XLSX ──────────────────────────────────────────────────────────────────────

fn extract_xlsx(path: &Path) -> Result<String> {
    use calamine::{open_workbook_auto, Reader};
    // Only a ZIP container (.xlsx) can be a decompression bomb. A legacy
    // .xls is an OLE compound file that calamine reads natively: sent
    // through the ZIP check, every real one was refused as "not a readable
    // zip archive" from 0.1.41, when the check arrived, until 0.1.46.
    if is_zip(path)? {
        reject_decompression_bomb(path, "spreadsheet")?;
    }
    let mut wb = open_workbook_auto(path)
        .with_context(|| format!("calamine open {}", path.display()))?;
    let mut out = String::new();
    for sheet_name in wb.sheet_names().to_vec() {
        if let Ok(range) = wb.worksheet_range(&sheet_name) {
            for row in range.rows() {
                let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
        }
    }
    Ok(out)
}

// ── Decompression bombs ───────────────────────────────────────────────────────

/// Ceiling on the total uncompressed size of a DOCX or XLSX.
///
/// Both formats are zip archives, and both parsers inflate them into memory
/// in one go. The upload limit only bounds the COMPRESSED file, and a
/// thousand-to-one ratio is trivial to produce, so a few kilobytes on the
/// wire could become gigabytes of resident memory — fatal on the ARM64
/// boards this project ships builds for, where RAM is already shared with
/// the language and embedding models.
///
/// 512 MiB is far above any genuine office document and far below what would
/// hurt.
const MAX_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

/// `Err(total)` as soon as the running total passes the ceiling, `Ok(())`
/// otherwise.
///
/// A free function over an iterator of sizes so the arithmetic — including
/// the saturating add that stops a crafted set of sizes from wrapping the
/// total back to something small — can be tested without building a real
/// bomb on disk, in the same spirit as `validate_auth` and
/// `validate_new_password` elsewhere in this codebase.
fn total_within_limit(sizes: impl Iterator<Item = u64>) -> Result<u64, u64> {
    let mut total: u64 = 0;
    for size in sizes {
        total = total.saturating_add(size);
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(total);
        }
    }
    Ok(total)
}

/// Signature of an OLE compound file: .doc, .xls and the other Office
/// 97–2003 formats.
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Up to the first 8 bytes of `path`: enough to tell the containers apart.
fn head(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::with_capacity(8);
    std::fs::File::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .take(8)
        .read_to_end(&mut head)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(head)
}

/// Local-file, empty-archive and spanned-archive signatures all count.
fn is_zip_magic(head: &[u8]) -> bool {
    [b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"].iter().any(|m| head.starts_with(*m))
}

/// Whether `path` is an Office 2007+ document protected by a password: not
/// a ZIP archive then, but an OLE compound file holding the encrypted
/// package — which the ZIP readers would only call corrupt.
fn is_encrypted_ooxml(path: &Path) -> bool {
    head(path).is_ok_and(|head| head.starts_with(&OLE_MAGIC))
        && std::fs::File::open(path)
            .ok()
            .and_then(|file| cfb::CompoundFile::open(file).ok())
            .is_some_and(|ole| ole.is_stream("EncryptedPackage"))
}

/// Whether `path` holds a ZIP container, judged by its first bytes rather
/// than its extension — so an .xlsx renamed to .xls is still checked for
/// decompression bombs, and a genuine .xls is not mistaken for a broken
/// ZIP.
fn is_zip(path: &Path) -> Result<bool> {
    Ok(is_zip_magic(&head(path)?))
}

/// Refuses a zip-based document whose entries declare more uncompressed bytes
/// than `MAX_UNCOMPRESSED_BYTES`, before any parser inflates it.
///
/// Reads the central directory only — `ZipEntry::size()` is a header field,
/// so nothing is decompressed to run this check.
///
/// Known limit, stated rather than implied: the declared size is part of the
/// archive and therefore also attacker-controlled. This stops the ordinary
/// bomb, which declares its real (enormous) size because that is what makes
/// it inflate; it does not stop an archive that lies about its sizes.
/// Catching that needs the decompression itself to run through a capped
/// reader, which means either patching the parsers or decompressing twice —
/// worth doing, but not at this price.
pub(crate) fn reject_decompression_bomb(path: &Path, kind: &str) -> Result<()> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening {kind} {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("{kind} is not a readable zip archive: {}", path.display()))?;

    let sizes = (0..archive.len()).map(|i| archive.by_index_raw(i).map(|e| e.size()).unwrap_or(0));

    match total_within_limit(sizes) {
        Ok(total) => {
            tracing::debug!(file = %path.display(), uncompressed = total, "{kind} size check ok");
            Ok(())
        }
        Err(total) => anyhow::bail!(
            "{kind} refused: its entries declare at least {total} bytes uncompressed, \
             over the {MAX_UNCOMPRESSED_BYTES} byte limit"
        ),
    }
}

// ── HTML ──────────────────────────────────────────────────────────────────────

fn extract_html(path: &Path) -> Result<String> {
    let html = std::fs::read_to_string(path)
        .with_context(|| format!("reading html {}", path.display()))?;
    Ok(super::html::to_text(&html))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins SUPPORTED_EXTENSIONS to extract_text's own match. A list that
    /// drifts from the dispatch is worse than no list: the upload handler
    /// would accept a file the parser then refuses, or refuse one it can
    /// read. Each extension is exercised against an empty file — what
    /// matters is only that the answer is NOT "unsupported format", which
    /// is the one error the dispatch's catch-all arm produces.
    ///
    /// That proves dispatch, not parsing: an extension can be routed and
    /// still never succeed. .xls passed here for five releases while every
    /// real .xls was refused — see `a_real_xls_is_read_not_refused_as_a_zip`.
    #[test]
    fn supported_extensions_match_the_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        for ext in SUPPORTED_EXTENSIONS {
            assert!(is_supported_extension(ext), "{ext} missing from is_supported_extension");
            let f = dir.path().join(format!("probe.{ext}"));
            std::fs::write(&f, b"").unwrap();
            if let Err(e) = extract_text(&f, dir.path()) {
                assert!(
                    !e.to_string().contains("unsupported format"),
                    ".{ext} is in SUPPORTED_EXTENSIONS but extract_text does not dispatch it"
                );
            }
        }
    }

    /// The web UI's file picker offers exactly these extensions: the two
    /// lists are kept by hand, and had drifted apart before — `.pptx`
    /// offered and refused, `.md` and `.csv` read but not offered.
    #[test]
    fn the_upload_picker_offers_exactly_the_supported_extensions() {
        let app = include_str!("../../frontend/src/App.jsx");
        let accept = app
            .split("accept=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("App.jsx has an accept=\"…\" attribute");
        let mut offered: Vec<&str> = accept.split(',').map(|e| e.trim().trim_start_matches('.')).collect();
        let mut supported = SUPPORTED_EXTENSIONS.to_vec();
        offered.sort_unstable();
        supported.sort_unstable();
        assert_eq!(offered, supported, "frontend/src/App.jsx accept= and SUPPORTED_EXTENSIONS differ");
    }

    #[test]
    fn an_unlisted_extension_is_refused_by_both() {
        assert!(!is_supported_extension("pages"));
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("letter.pages");
        std::fs::write(&f, b"").unwrap();
        let e = extract_text(&f, dir.path()).unwrap_err();
        assert!(e.to_string().contains("unsupported format"), "unexpected: {e}");
    }

    /// A reader that panics must come back as an ordinary error. (The
    /// panic message this prints is expected.)
    #[test]
    fn a_panicking_reader_becomes_an_error() {
        let e = guarded::<()>("xlsb", || panic!("index out of range")).unwrap_err();
        assert!(e.to_string().contains("could not be read"), "{e:#}");
        assert_eq!(guarded("txt", || Ok::<_, anyhow::Error>(7)).unwrap(), 7);
    }

    /// A genuine Excel 97–2003 workbook: an OLE compound file, not a ZIP.
    /// Generated once with xlwt 1.3.0; one sheet, a title and a small table.
    const LEGACY_XLS: &[u8] = include_bytes!("testdata/legacy.xls");

    #[test]
    fn a_real_xls_is_read_not_refused_as_a_zip() {
        assert_eq!(
            &LEGACY_XLS[..4],
            b"\xD0\xCF\x11\xE0",
            "fixture is not an OLE file"
        );
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("legacy.xls");
        std::fs::write(&f, LEGACY_XLS).unwrap();

        let text = extract_text(&f, dir.path())
            .expect("a real .xls must parse")
            .text;

        assert!(
            text.contains("Legacy spreadsheet"),
            "title cell missing: {text:?}"
        );
        assert!(
            text.contains("Quarter") && text.contains("Q2"),
            "table missing: {text:?}"
        );
    }

    /// .xlsm (macro-enabled; the macros are never run, only cell values
    /// read) and .ods, both saved by LibreOffice 24.2 from the same small
    /// table. .xlsb goes through the same calamine reader; LibreOffice
    /// cannot write it, so it has no fixture here.
    #[test]
    fn other_spreadsheet_formats_are_read() {
        let fixtures: [(&str, &[u8]); 2] = [
            ("sheet.ods", include_bytes!("testdata/sheet.ods")),
            ("sheet.xlsm", include_bytes!("testdata/sheet.xlsm")),
        ];
        for (name, bytes) in fixtures {
            let dir = tempfile::tempdir().unwrap();
            let f = dir.path().join(name);
            std::fs::write(&f, bytes).unwrap();
            let text = extract_text(&f, dir.path()).unwrap_or_else(|e| panic!("{name}: {e:#}")).text;
            for expected in ["Quarter", "Città di Roma", "Perù €", "1250"] {
                assert!(text.contains(expected), "{name}: missing {expected:?} in {text:?}");
            }
        }
    }

    /// The check goes by content: a ZIP renamed to .xls is still inspected
    /// as one, rather than handed to the OLE reader unchecked.
    #[test]
    fn a_zip_named_xls_still_goes_through_the_bomb_check() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("renamed.xls");
        std::fs::write(&f, b"PK\x03\x04 not really an archive").unwrap();

        let e = extract_text(&f, dir.path()).unwrap_err();

        assert!(
            format!("{e:#}").contains("zip archive"),
            "bomb check skipped: {e:#}"
        );
    }

    /// A file named .doc is dispatched on what it contains.
    #[test]
    fn a_doc_is_read_by_what_it_contains_not_by_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let cases: [(&[u8], &str); 2] = [
            // a .docx renamed: goes to the .docx reader and its bomb check
            (b"PK\x03\x04 not really an archive", "zip archive"),
            // Word 2.0, from before the OLE container
            (&[0xDB, 0xA5, 0x2D, 0x00, 0x31, 0x40], "Word 2.0"),
        ];
        let f = dir.path().join("sample.doc");
        for (bytes, expected) in cases {
            std::fs::write(&f, bytes).unwrap();
            let e = extract_text(&f, dir.path()).unwrap_err();
            assert!(format!("{e:#}").contains(expected), "expected {expected:?}, got: {e:#}");
        }
        // RTF that Word saved under .doc
        std::fs::write(&f, b"{\\rtf1\\ansi hello}").unwrap();
        assert_eq!(extract_text(&f, dir.path()).unwrap().text, "hello");
    }

    fn span(page: u32, start: usize, end: usize) -> PageSpan {
        PageSpan { page, start_byte: start, end_byte: end }
    }

    #[test]
    fn ordinary_documents_are_under_the_limit() {
        // A few megabytes across a handful of parts: an unremarkable DOCX.
        let sizes = [12_000u64, 3_500_000, 48_000, 900_000];
        assert_eq!(
            total_within_limit(sizes.into_iter()),
            Ok(sizes.iter().sum::<u64>())
        );
    }

    #[test]
    fn the_limit_itself_is_allowed() {
        assert_eq!(
            total_within_limit(std::iter::once(MAX_UNCOMPRESSED_BYTES)),
            Ok(MAX_UNCOMPRESSED_BYTES)
        );
        assert!(total_within_limit(std::iter::once(MAX_UNCOMPRESSED_BYTES + 1)).is_err());
    }

    /// The classic shape: many small entries that add up to far too much.
    #[test]
    fn many_entries_that_sum_past_the_limit_are_refused() {
        let sizes = std::iter::repeat_n(64 * 1024 * 1024, 16); // 1 GiB total
        assert!(total_within_limit(sizes).is_err());
    }

    /// A crafted set of sizes must not wrap the running total back to a
    /// small number and slip through.
    #[test]
    fn absurd_sizes_cannot_overflow_the_total() {
        let sizes = [u64::MAX, u64::MAX, 1];
        assert!(total_within_limit(sizes.into_iter()).is_err());
    }

    #[test]
    fn empty_pages_means_no_page_info() {
        assert_eq!(pages_for_range(&[], 0, 100), None);
    }

    #[test]
    fn chunk_fully_inside_one_page() {
        let pages = [span(1, 0, 100), span(2, 100, 200)];
        assert_eq!(pages_for_range(&pages, 10, 50), Some((1, 1)));
        assert_eq!(pages_for_range(&pages, 120, 180), Some((2, 2)));
    }

    #[test]
    fn chunk_straddling_two_pages() {
        let pages = [span(1, 0, 100), span(2, 100, 200)];
        // CHUNK_OVERLAP can legitimately bridge a page boundary.
        assert_eq!(pages_for_range(&pages, 90, 110), Some((1, 2)));
    }

    #[test]
    fn chunk_touching_boundary_exactly_stays_on_one_side() {
        let pages = [span(1, 0, 100), span(2, 100, 200)];
        // [90, 100) ends exactly at the boundary — half-open range, so it
        // must NOT pull in page 2 (whose span starts at 100, not before it).
        assert_eq!(pages_for_range(&pages, 90, 100), Some((1, 1)));
        // [100, 110) starts exactly at the boundary — page 2 only.
        assert_eq!(pages_for_range(&pages, 100, 110), Some((2, 2)));
    }

    #[test]
    fn chunk_spanning_three_short_pages_reports_first_and_last_not_middle() {
        let pages = [span(1, 0, 10), span(2, 10, 20), span(3, 20, 30)];
        assert_eq!(pages_for_range(&pages, 5, 25), Some((1, 3)));
    }

    #[test]
    fn chunk_outside_every_page_span_is_none() {
        let pages = [span(1, 0, 100)];
        assert_eq!(pages_for_range(&pages, 200, 300), None);
    }

    /// Builds a minimal, valid multi-page PDF from raw syntax — same
    /// technique as ocr.rs's `minimal_pdf_with_text`, extended to N pages —
    /// so `extract_pdf`'s NATIVE (non-OCR) branch can be exercised
    /// end-to-end with no external fixtures or bundled libraries required.
    /// Each page's text is padded well past the 500-char/50% native-text
    /// acceptance threshold in `extract_pdf`, so this genuinely stays on the
    /// pdf_oxide path and never falls through to OCR.
    fn minimal_multi_page_pdf(page_texts: &[&str]) -> Vec<u8> {
        let mut objects: Vec<String> = Vec::new();
        // 1: Catalog, 2: Pages, 3: Font, then N page objects, then N content objects.
        objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
        let n = page_texts.len();
        let page_obj_nums: Vec<usize> = (4..4 + n).collect();
        let content_obj_nums: Vec<usize> = (4 + n..4 + 2 * n).collect();
        let kids = page_obj_nums.iter().map(|o| format!("{o} 0 R")).collect::<Vec<_>>().join(" ");
        objects.push(format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>"));
        objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());
        for &content_obj in &content_obj_nums {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /Font << /F1 3 0 R >> >> /Contents {content_obj} 0 R >>"
            ));
        }
        for &text in page_texts {
            let stream = format!("BT /F1 10 Tf 20 700 Td ({text}) Tj ET");
            objects.push(format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()));
        }

        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(obj.as_bytes());
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref_offset = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF", objects.len() + 1)
                .as_bytes(),
        );
        out
    }

    /// Acceptance test 1 (native multi-page PDF: the retrieved chunk keeps
    /// the right page). No OCR libraries required — this fixture has enough
    /// real embedded text per page that pdf_oxide's own extraction clears
    /// `extract_pdf`'s native-text acceptance threshold, so it never falls
    /// through to the OCR branch.
    #[test]
    fn native_multi_page_pdf_preserves_page_spans() {
        let page1 = "Alpha marker sentence about the first page. ".repeat(8);
        let page2 = "Beta marker sentence about the second page. ".repeat(8);
        let pdf_bytes = minimal_multi_page_pdf(&[page1.trim(), page2.trim()]);

        let tmp = std::env::temp_dir().join("i3k_parser_native_multipage_test.pdf");
        std::fs::write(&tmp, &pdf_bytes).expect("writing the test PDF");
        let data_dir = std::env::temp_dir().join("i3k_parser_native_multipage_unused_data_dir");

        let result = extract_pdf(&tmp, &data_dir);
        let _ = std::fs::remove_file(&tmp);
        let extracted = result.expect("native pdf_oxide extraction must succeed on embedded text");

        assert!(extracted.text.contains("Alpha marker"), "page 1 text missing: {:?}", extracted.text);
        assert!(extracted.text.contains("Beta marker"), "page 2 text missing: {:?}", extracted.text);

        assert_eq!(extracted.pages.len(), 2, "expected one span per page, got {:?}", extracted.pages);
        assert_eq!(extracted.pages[0].page, 1);
        assert_eq!(extracted.pages[1].page, 2);
        // Spans are contiguous: every byte of `text` belongs to exactly one page.
        assert_eq!(extracted.pages[0].start_byte, 0);
        assert_eq!(extracted.pages[0].end_byte, extracted.pages[1].start_byte);
        assert_eq!(extracted.pages[1].end_byte, extracted.text.len());

        // A chunk squarely inside page 1 maps to page 1 only.
        let inside_p1 = extracted.pages[0].start_byte + 5..extracted.pages[0].end_byte - 5;
        assert_eq!(pages_for_range(&extracted.pages, inside_p1.start, inside_p1.end), Some((1, 1)));

        // A chunk straddling the page boundary (as CHUNK_OVERLAP can produce) maps to (1, 2).
        let boundary = extracted.pages[0].end_byte;
        assert_eq!(pages_for_range(&extracted.pages, boundary - 5, boundary + 5), Some((1, 2)));
    }
}
