//! OCR per PDF scansionati: pdfium-render → leptess (Tesseract ita+eng).
//! Tessdata cercata in {I3K_DATA_DIR}/tessdata/, poi TESSDATA_PREFIX di sistema.
//! Attivazione: cargo build --features ocr
//! Dipendenze di sistema: libpdfium.so (in PATH o PDFIUM_DYNAMIC_LIB_PATH)
//!                        tessdata/ita.traineddata + tessdata/eng.traineddata

#[cfg(feature = "ocr")]
pub fn ocr_pdf(path: &std::path::Path, page_count: u32) -> anyhow::Result<String> {
    use anyhow::Context;
    use leptess::LepTess;
    use pdfium_render::prelude::*;

    let pdfium = Pdfium::new(
        Pdfium::bind_to_system_library().map_err(|e| {
            anyhow::anyhow!(
                "libpdfium non trovata: {e}\n  → installa libpdfium.so o imposta PDFIUM_DYNAMIC_LIB_PATH"
            )
        })?,
    );
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("pdfium: caricamento {:?}: {e}", path))?;

    // Cerca tessdata in {I3K_DATA_DIR}/tessdata/; se manca usa TESSDATA_PREFIX.
    let tessdata_path = std::env::var("I3K_DATA_DIR")
        .ok()
        .map(|d| std::path::PathBuf::from(d).join("tessdata"))
        .filter(|p| p.is_dir())
        .map(|p| p.display().to_string());

    let mut full_text = String::new();

    for i in 0..(page_count as usize) {
        let page = doc
            .pages()
            .get(i as u16)
            .with_context(|| format!("pdfium: pagina {i}"))?;

        // Rasterizza a ~200 dpi (A4: 1654×2339 px)
        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(1654)
                    .set_maximum_height(2339),
            )
            .with_context(|| format!("pdfium: render pagina {i}"))?;

        // set_image_from_mem si aspetta un formato immagine completo (PNG), NON raw bytes
        let img = bitmap.as_image();
        let mut png_buf: Vec<u8> = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_buf),
            image::ImageFormat::Png,
        )
        .with_context(|| format!("png encode pagina {i}"))?;

        let mut lt = LepTess::new(tessdata_path.as_deref(), "ita+eng").map_err(|e| {
            anyhow::anyhow!(
                "leptess init: {e}\n  → tessdata ita+eng in {:?}?",
                tessdata_path.as_deref().unwrap_or("TESSDATA_PREFIX")
            )
        })?;
        lt.set_image_from_mem(&png_buf)
            .map_err(|e| anyhow::anyhow!("leptess set_image pagina {i}: {e}"))?;
        let page_text = lt
            .get_utf8_text()
            .map_err(|e| anyhow::anyhow!("OCR pagina {i}: {e}"))?;

        tracing::debug!(page = i, chars = page_text.len(), "OCR pagina");
        full_text.push_str(&page_text);
        full_text.push('\n');
    }

    tracing::info!(
        file = %path.display(),
        pages = page_count,
        chars = full_text.len(),
        "OCR completato"
    );
    Ok(full_text)
}
