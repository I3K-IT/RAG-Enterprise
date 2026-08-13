//! OCR per PDF scansionati: pdfium-render → leptess (Tesseract ita+eng).
//! Tessdata cercata in {data_dir}/tessdata/ (stesso data_dir del manifest/bootstrap),
//! poi TESSDATA_PREFIX di sistema. Attivazione: cargo build --features ocr
//!
//! libpdfium è **bundlabile**: caricata a runtime (dlopen) cercando, in ordine,
//! PDFIUM_DYNAMIC_LIB_PATH → accanto all'eseguibile → ./lib/ accanto all'eseguibile
//! → ricerca di sistema (comoda in sviluppo, non richiesta in una build distribuita).
//! I binari ufficiali per ogni piattaforma sono su github.com/bblanchon/pdfium-binaries.
//!
//! leptess/Tesseract invece si linka **a compile-time** (non ha un equivalente di
//! bind_to_library): richiede ancora libtesseract + libleptonica sul sistema di build
//! e — per una build distribuita "no dipendenza host" — un rpath verso una ./lib/
//! bundlata. Non ancora impostato: vedi discussione bundling cross-platform.

#[cfg(feature = "ocr")]
fn resolve_pdfium_library_path() -> std::path::PathBuf {
    use pdfium_render::prelude::Pdfium;

    // 1. Override esplicito (sviluppo locale o layout di deploy custom).
    if let Ok(p) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        return std::path::PathBuf::from(p);
    }

    // 2. Accanto all'eseguibile, poi in ./lib/ accanto all'eseguibile — il layout
    //    di una build distribuita bundlata (no install system-wide richiesta).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = Pdfium::pdfium_platform_library_name_at_path(dir);
            if candidate.is_file() {
                return candidate;
            }
            let candidate = Pdfium::pdfium_platform_library_name_at_path(&dir.join("lib"));
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    // 3. Fallback: nome "nudo" → ricerca di sistema (dlopen standard). Comodo in
    //    sviluppo con libpdfium installata via apt/brew; non è il path previsto
    //    per le build distribuite.
    std::path::PathBuf::from(Pdfium::pdfium_platform_library_name())
}

/// `data_dir` è la stessa radice dati risolta da `Settings.data.data_path()`
/// (bootstrap/manifest scaricano la tessdata in `{data_dir}/tessdata/`).
#[cfg(feature = "ocr")]
pub fn ocr_pdf(
    path: &std::path::Path,
    page_count: u32,
    data_dir: &std::path::Path,
) -> anyhow::Result<String> {
    use anyhow::Context;
    use leptess::LepTess;
    use pdfium_render::prelude::*;

    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(resolve_pdfium_library_path()).map_err(|e| {
            anyhow::anyhow!(
                "libpdfium non trovata: {e}\n  → posizionala accanto all'eseguibile (o in ./lib/), oppure imposta PDFIUM_DYNAMIC_LIB_PATH"
            )
        })?,
    );
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("pdfium: caricamento {:?}: {e}", path))?;

    // Cerca tessdata in {data_dir}/tessdata/ (dove il manifest la scarica);
    // se manca (dev senza bootstrap) usa TESSDATA_PREFIX di sistema.
    let tessdata_path = Some(data_dir.join("tessdata"))
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

#[cfg(all(test, feature = "ocr"))]
mod smoke_test {
    use super::*;

    /// I due test qui sotto mutano PDFIUM_DYNAMIC_LIB_PATH (env var di processo).
    /// cargo test esegue i test in parallelo di default: senza serializzazione,
    /// un test potrebbe leggere l'env var che l'altro ha appena settato/rimosso.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Costruisce un PDF minimo (senza dipendenze) con una riga di testo vettoriale,
    /// cosi' la pipeline pdfium(rasterizza)->leptess(OCR) ha qualcosa di reale da leggere.
    fn minimal_pdf_with_text(text: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            {
                let stream = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET");
                format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len())
            },
        ];
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

    /// Golden-smoke-test: verifica che libpdfium venga risolta e caricata da un path
    /// NON di sistema (accanto all'eseguibile di test, layout "bundlato"), e che la
    /// pipeline pdfium->PNG->leptess estragga davvero il testo atteso.
    ///
    /// Richiede (solo in locale, non in CI):
    ///   - PDFIUM_LIB_FOR_TEST=/path/a/libpdfium.so   (una copia scaricata da
    ///     github.com/bblanchon/pdfium-binaries; NON deve essere in un path di sistema)
    ///   - tessdata ita+eng installate (vedi BUILD.md §2a)
    #[test]
    fn bundled_pdfium_path_resolution_and_ocr_roundtrip() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let Ok(scratch_lib) = std::env::var("PDFIUM_LIB_FOR_TEST") else {
            eprintln!("PDFIUM_LIB_FOR_TEST non impostata — skip (vedi doc del test)");
            return;
        };

        // Simula il layout di release: copia libpdfium accanto all'eseguibile di
        // test corrente, poi rimuove qualunque override esplicito così la
        // resolve_pdfium_library_path() e' costretta a passare dal ramo
        // "accanto all'eseguibile" (il ramo nuovo, non ancora testato prima d'ora).
        std::env::remove_var("PDFIUM_DYNAMIC_LIB_PATH");
        let exe = std::env::current_exe().expect("current_exe");
        let dir = exe.parent().expect("exe parent");
        let dest = dir.join(
            pdfium_render::prelude::Pdfium::pdfium_platform_library_name(),
        );
        std::fs::copy(&scratch_lib, &dest).expect("copia libpdfium accanto al binario di test");

        let pdf_bytes = minimal_pdf_with_text("i3k OCR bundling smoke test");
        let tmp_pdf = std::env::temp_dir().join("i3k_ocr_smoke.pdf");
        std::fs::write(&tmp_pdf, &pdf_bytes).expect("scrittura PDF di test");

        // data_dir "vuota": nessun tessdata/ dentro → ocr_pdf ricade sul
        // TESSDATA_PREFIX di sistema, esattamente come prima di questo test.
        let empty_data_dir = std::env::temp_dir().join("i3k_ocr_smoke_empty_data_dir");
        let _ = std::fs::create_dir_all(&empty_data_dir);

        let result = ocr_pdf(&tmp_pdf, 1, &empty_data_dir);

        // Pulizia prima degli assert, per non lasciare file anche se il test fallisce.
        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(&tmp_pdf);
        let _ = std::fs::remove_dir_all(&empty_data_dir);

        let text = result.expect("ocr_pdf deve riuscire con libpdfium bundlata accanto all'eseguibile");
        assert!(
            text.to_lowercase().contains("bundling") || text.to_lowercase().contains("smoke"),
            "testo OCR inatteso: {text:?}"
        );
    }

    /// Verifica che la tessdata venga letta da {data_dir}/tessdata/ (il path dove il
    /// manifest la scarica), NON da una variabile d'ambiente slegata dal resto del
    /// sistema (bug precedente: leggeva I3K_DATA_DIR, mai impostata da nessuno).
    ///
    /// Richiede TESSDATA_DIR_FOR_TEST=/path/a/cartella/con/tessdata/{ita,eng}.traineddata
    /// (root "dati", stessa struttura di {data} nel manifest — non la cartella
    /// tessdata stessa). Skip gracioso se non impostata.
    #[test]
    fn tessdata_resolved_from_data_dir() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let (Ok(scratch_lib), Ok(data_dir)) = (
            std::env::var("PDFIUM_LIB_FOR_TEST"),
            std::env::var("TESSDATA_DIR_FOR_TEST"),
        ) else {
            eprintln!("PDFIUM_LIB_FOR_TEST/TESSDATA_DIR_FOR_TEST non impostate — skip");
            return;
        };
        std::env::set_var("PDFIUM_DYNAMIC_LIB_PATH", &scratch_lib);

        let pdf_bytes = minimal_pdf_with_text("data dir tessdata roundtrip");
        let tmp_pdf = std::env::temp_dir().join("i3k_ocr_smoke_datadir.pdf");
        std::fs::write(&tmp_pdf, &pdf_bytes).expect("scrittura PDF di test");

        let result = ocr_pdf(&tmp_pdf, 1, std::path::Path::new(&data_dir));

        std::env::remove_var("PDFIUM_DYNAMIC_LIB_PATH");
        let _ = std::fs::remove_file(&tmp_pdf);

        let text = result.expect("ocr_pdf deve riuscire leggendo tessdata da {data_dir}/tessdata/");
        assert!(
            text.to_lowercase().contains("roundtrip") || text.to_lowercase().contains("data dir"),
            "testo OCR inatteso: {text:?}"
        );
    }
}
