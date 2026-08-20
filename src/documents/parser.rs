//! Document text extraction.
//!
//! Extraction chain, following ocr_service.py:
//! 1. .txt/.md/.csv  → read directly as UTF-8
//! 2. PDF            → pdf_oxide (text_ratio > 0.5 AND len > 500)
//!                     → OCR fallback below that ratio (see ocr.rs)
//! 3. DOCX           → docx-rs
//! 4. XLSX/XLS       → calamine
//! 5. HTML           → scraper
//!
//! OCR trigger: text_ratio below 30% of pages containing text, as in the
//! Python implementation. Chunking happens in rag::chunker, NOT here.

use std::path::Path;
use anyhow::{Context, Result};

/// One page's byte-offset span `[start, end)` within `ExtractedText::text`.
/// `page` is 1-based. PDF-only (native or OCR) — every other format leaves
/// `ExtractedText::pages` empty, since none of txt/md/csv/docx/xlsx/html has
/// a page concept that survives extraction into flat text today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSpan {
    pub page: u32,
    pub start: usize,
    pub end: usize,
}

/// Result of extracting a document: the flat text chunker::split_text
/// operates on, the page count (None when the format has no pages), and —
/// PDF only — the byte-offset span of each page within `text`, so a later
/// chunk's `[start, end)` (see rag::chunker::Chunk) can be mapped back to
/// the page(s) it came from via `pages_for_range`.
#[derive(Debug, Clone, Default)]
pub struct ExtractedText {
    pub text: String,
    pub page_count: Option<u32>,
    pub pages: Vec<PageSpan>,
}

/// Extracts the text (and, for PDF, the per-page spans within it).
///
/// `data_dir` is the data root (`Settings.data.data_path()`), needed only by
/// the OCR branch to find `{data_dir}/tessdata/`, where the manifest
/// downloads it.
pub fn extract_text(path: &Path, data_dir: &Path) -> Result<ExtractedText> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "txt" | "md" | "csv" => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            Ok(ExtractedText { text, ..Default::default() })
        }
        "pdf" => extract_pdf(path, data_dir),
        "docx" | "doc" => extract_docx(path).map(|text| ExtractedText { text, ..Default::default() }),
        "xlsx" | "xls" => extract_xlsx(path).map(|text| ExtractedText { text, ..Default::default() }),
        "html" | "htm" => extract_html(path).map(|text| ExtractedText { text, ..Default::default() }),
        _ => Err(anyhow::anyhow!("unsupported format: .{ext}")),
    }
}

/// Given a chunk's byte-offset span `[start, end)` in the extracted text and
/// the page spans produced alongside it, returns the inclusive `(first,
/// last)` 1-based page range the chunk overlaps — `None` when `pages` is
/// empty (non-PDF, or a PDF whose extraction produced no page spans). A
/// chunk can legitimately straddle more than one page (CHUNK_OVERLAP can
/// bridge a page boundary), hence a range rather than a single page.
pub fn pages_for_range(pages: &[PageSpan], start: usize, end: usize) -> Option<(u32, u32)> {
    let mut first: Option<u32> = None;
    let mut last: Option<u32> = None;
    for p in pages {
        // Half-open range overlap test: [p.start, p.end) intersects [start, end).
        if p.start < end && p.end > start {
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
            pages.push(PageSpan { page: (i + 1) as u32, start, end: full_text.len() });
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
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading docx {}", path.display()))?;
    let docx = docx_rs::read_docx(&bytes)
        .map_err(|e| anyhow::anyhow!("docx parse: {e:?}"))?;
    Ok(collect_docx_text(&docx))
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

// ── HTML ──────────────────────────────────────────────────────────────────────

fn extract_html(path: &Path) -> Result<String> {
    let html = std::fs::read_to_string(path)
        .with_context(|| format!("reading html {}", path.display()))?;
    let document = scraper::Html::parse_document(&html);
    let selector =
        scraper::Selector::parse("p, h1, h2, h3, h4, h5, h6, li, td, th").unwrap();
    let text: Vec<String> = document
        .select(&selector)
        .map(|el| el.text().collect::<String>().trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(page: u32, start: usize, end: usize) -> PageSpan {
        PageSpan { page, start, end }
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
        assert_eq!(extracted.pages[0].start, 0);
        assert_eq!(extracted.pages[0].end, extracted.pages[1].start);
        assert_eq!(extracted.pages[1].end, extracted.text.len());

        // A chunk squarely inside page 1 maps to page 1 only.
        let inside_p1 = extracted.pages[0].start + 5..extracted.pages[0].end - 5;
        assert_eq!(pages_for_range(&extracted.pages, inside_p1.start, inside_p1.end), Some((1, 1)));

        // A chunk straddling the page boundary (as CHUNK_OVERLAP can produce) maps to (1, 2).
        let boundary = extracted.pages[0].end;
        assert_eq!(pages_for_range(&extracted.pages, boundary - 5, boundary + 5), Some((1, 2)));
    }
}
