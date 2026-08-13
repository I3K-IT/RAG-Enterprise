//! Document text extraction.
//!
//! Extraction chain, following ocr_service.py:
//! 1. .txt/.md/.csv  → read directly as UTF-8
//! 2. PDF            → pdf_oxide (text_ratio > 0.5 AND len > 500)
//!                     → optional OCR when the "ocr" feature is enabled
//! 3. DOCX           → docx-rs
//! 4. XLSX/XLS       → calamine
//! 5. HTML           → scraper
//!
//! OCR trigger: text_ratio below 30% of pages containing text, as in the
//! Python implementation. Chunking happens in rag::chunker, NOT here.

use std::path::Path;
use anyhow::{Context, Result};

/// Extracts the text and the page count (None when the format has no pages).
///
/// `data_dir` is the data root (`Settings.data.data_path()`), needed only by
/// the OCR branch to find `{data_dir}/tessdata/`, where the manifest
/// downloads it.
pub fn extract_text(path: &Path, data_dir: &Path) -> Result<(String, Option<u32>)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "txt" | "md" | "csv" => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            Ok((text, None))
        }
        "pdf" => extract_pdf(path, data_dir),
        "docx" | "doc" => extract_docx(path).map(|t| (t, None)),
        "xlsx" | "xls" => extract_xlsx(path).map(|t| (t, None)),
        "html" | "htm" => extract_html(path).map(|t| (t, None)),
        _ => Err(anyhow::anyhow!("unsupported format: .{ext}")),
    }
}

// ── PDF ───────────────────────────────────────────────────────────────────────

#[cfg_attr(not(feature = "ocr"), allow(unused_variables))]
fn extract_pdf(path: &Path, data_dir: &Path) -> Result<(String, Option<u32>)> {
    let doc = pdf_oxide::PdfDocument::open(path)
        .with_context(|| format!("pdf_oxide open {}", path.display()))?;
    let page_count = doc.page_count().context("pdf_oxide page_count")? as u32;

    let pages_to_check = (page_count as usize).min(10);
    let mut pages_with_text = 0usize;
    let mut full_text = String::new();

    for i in 0..(page_count as usize) {
        let page_text = doc.extract_text(i).unwrap_or_default();
        let has_text = !page_text.trim().is_empty();
        if has_text && i < pages_to_check {
            pages_with_text += 1;
        }
        if has_text {
            full_text.push_str(&page_text);
            full_text.push('\n');
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
        return Ok((full_text, Some(page_count)));
    }

    // Not enough text: fall back to OCR when available
    #[cfg(feature = "ocr")]
    {
        let ocr_text = super::ocr::ocr_pdf(path, page_count, data_dir)?;
        Ok((ocr_text, Some(page_count)))
    }

    #[cfg(not(feature = "ocr"))]
    {
        tracing::warn!(
            file = %path.display(),
            text_ratio = format!("{:.0}%", text_ratio * 100.0).as_str(),
            "PDF probabilmente scansionato — compila con --features ocr"
        );
        Ok((full_text, Some(page_count)))
    }
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
