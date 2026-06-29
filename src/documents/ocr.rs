//! OCR for scanned PDFs: pdfium-render → leptess (Tesseract ita+eng).
//! Traineddata bundled — no host Tesseract dependency.
//! Triggered when text_ratio < 0.5 (see parser.rs).

// TODO Fase 1: implement rasterize + OCR pipeline
