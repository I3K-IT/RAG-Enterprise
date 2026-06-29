//! Document text extraction.
//!
//! Catena fedele al Python (MAPPA §9 — ocr_service.py):
//! 1. .txt/.md/.csv  → lettura diretta UTF-8
//! 2. PDF            → pdf_oxide se text_ratio > 0.5 E chars > 500
//! 3. Scansioni      → pdfium-render (rasterizza) → leptess (Tesseract ita+eng)
//! 4. DOCX/PPTX      → docx-rs / quick-xml
//! 5. XLSX           → calamine
//! 6. HTML           → scraper
//! 7. MD             → pulldown-cmark
//! 8. CSV            → csv crate
//!
//! Trigger OCR: text_ratio < 0.5 (come il Python is_scanned = text_ratio < 0.3,
//! ma la soglia di accettazione è > 0.5 per PyMuPDF).
//! Il chunking NON avviene qui — avviene in rag::chunker.
//!
//! VIETATO: lopdf (produce spazzatura per i PDF testuali).

use std::path::Path;
use anyhow::Result;

pub fn extract_text(path: &Path) -> Result<String> {
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "txt" | "md" | "csv" => read_utf8(path),
        "pdf" => extract_pdf(path),
        "docx" | "doc" => extract_docx(path),
        "xlsx" | "xls" => extract_xlsx(path),
        "html" | "htm" => extract_html(path),
        _ => Err(anyhow::anyhow!("unsupported format: .{ext}")),
    }
}

fn read_utf8(path: &Path) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}

fn extract_pdf(_path: &Path) -> Result<String> {
    // TODO Fase 1:
    // 1. pdf_oxide → get text + page_count → compute text_ratio
    // 2. if text_ratio > 0.5 AND len > 500 → return text
    // 3. else → pdfium-render rasterize → leptess OCR (ita+eng)
    todo!("pdf extraction — Fase 1")
}

fn extract_docx(_path: &Path) -> Result<String> {
    // TODO Fase 1: docx-rs
    todo!("docx extraction — Fase 1")
}

fn extract_xlsx(_path: &Path) -> Result<String> {
    // TODO Fase 1: calamine
    todo!("xlsx extraction — Fase 1")
}

fn extract_html(_path: &Path) -> Result<String> {
    // TODO Fase 1: scraper
    todo!("html extraction — Fase 1")
}
