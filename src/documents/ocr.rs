//! OCR for scanned PDFs: pdfium-render → leptess (Tesseract ita+eng).
//! Tessdata is looked up in {data_dir}/tessdata/ — the same data_dir the
//! manifest and bootstrap use — then in the system TESSDATA_PREFIX. Enabled
//! with `cargo build --features ocr`.
//!
//! libpdfium is **bundlable**: loaded at runtime through dlopen, searching in
//! order PDFIUM_DYNAMIC_LIB_PATH → next to the executable → ./lib/ next to the
//! executable → the system search, which is convenient in development but not
//! required for a distributed build. Official binaries for every platform live
//! at github.com/bblanchon/pdfium-binaries.
//!
//! leptess/Tesseract, by contrast, links **at compile time** and has no
//! equivalent of bind_to_library: it still needs libtesseract and libleptonica
//! on the build system and — for a distributed build with no host dependency —
//! an rpath pointing at a bundled ./lib/. Not set up yet.

#[cfg(feature = "ocr")]
fn resolve_pdfium_library_path() -> std::path::PathBuf {
    use pdfium_render::prelude::Pdfium;

    // 1. Explicit override (local development, or a custom deploy layout).
    if let Ok(p) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        return std::path::PathBuf::from(p);
    }

    // 2. Next to the executable, then in ./lib/ beside it — the layout of a
    //    bundled distributed build, needing no system-wide install.
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

    // 3. Fallback: the bare name → standard dlopen system search. Handy in
    //    development with libpdfium installed via apt or brew; not the path
    //    intended for distributed builds.
    std::path::PathBuf::from(Pdfium::pdfium_platform_library_name())
}

/// `data_dir` is the same data root resolved by `Settings.data.data_path()`;
/// the bootstrap and manifest download the tessdata into
/// `{data_dir}/tessdata/`.
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
                "libpdfium not found: {e}\n  → place it next to the executable (or in ./lib/), or set PDFIUM_DYNAMIC_LIB_PATH"
            )
        })?,
    );
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| anyhow::anyhow!("pdfium: loading {:?}: {e}", path))?;

    // Look for tessdata in {data_dir}/tessdata/, where the manifest downloads
    // it; if absent (development without the bootstrap) fall back to the
    // system TESSDATA_PREFIX.
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

    /// The two tests below mutate PDFIUM_DYNAMIC_LIB_PATH, a process-wide
    /// environment variable. cargo test runs tests in parallel by default, so
    /// without serialisation one test could read the variable the other has
    /// just set or removed.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Builds a minimal PDF with no dependencies, containing one line of
    /// vector text, so the pdfium (rasterise) → leptess (OCR) pipeline has
    /// something real to read.
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

    /// Smoke test: checks that libpdfium is resolved and loaded from a
    /// NON-system path — next to the test executable, the "bundled" layout —
    /// and that the pdfium → PNG → leptess pipeline really does extract the
    /// expected text.
    ///
    /// Requires (locally only, not in CI):
    ///   - PDFIUM_LIB_FOR_TEST=/path/to/libpdfium.so (a copy downloaded from
    ///     github.com/bblanchon/pdfium-binaries; it must NOT be on a system path)
    ///   - tessdata ita+eng installed (see BUILD.md §2a)
    #[test]
    fn bundled_pdfium_path_resolution_and_ocr_roundtrip() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let Ok(scratch_lib) = std::env::var("PDFIUM_LIB_FOR_TEST") else {
            eprintln!("PDFIUM_LIB_FOR_TEST unset — skipping (see the test docs)");
            return;
        };

        // Simulate the release layout: copy libpdfium next to the current test
        // executable, then remove any explicit override so
        // resolve_pdfium_library_path() is forced down the "next to the
        // executable" branch.
        std::env::remove_var("PDFIUM_DYNAMIC_LIB_PATH");
        let exe = std::env::current_exe().expect("current_exe");
        let dir = exe.parent().expect("exe parent");
        let dest = dir.join(
            pdfium_render::prelude::Pdfium::pdfium_platform_library_name(),
        );
        std::fs::copy(&scratch_lib, &dest).expect("copying libpdfium next to the test binary");

        let pdf_bytes = minimal_pdf_with_text("i3k OCR bundling smoke test");
        let tmp_pdf = std::env::temp_dir().join("i3k_ocr_smoke.pdf");
        std::fs::write(&tmp_pdf, &pdf_bytes).expect("writing the test PDF");

        // An "empty" data_dir with no tessdata/ inside, so ocr_pdf falls back
        // to the system TESSDATA_PREFIX.
        let empty_data_dir = std::env::temp_dir().join("i3k_ocr_smoke_empty_data_dir");
        let _ = std::fs::create_dir_all(&empty_data_dir);

        let result = ocr_pdf(&tmp_pdf, 1, &empty_data_dir);

        // Clean up before the assertions, so nothing is left behind even if
        // the test fails.
        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(&tmp_pdf);
        let _ = std::fs::remove_dir_all(&empty_data_dir);

        let text = result.expect("ocr_pdf must succeed with libpdfium bundled next to the executable");
        assert!(
            text.to_lowercase().contains("bundling") || text.to_lowercase().contains("smoke"),
            "unexpected OCR text: {text:?}"
        );
    }

    /// Checks that tessdata is read from {data_dir}/tessdata/, the path the
    /// manifest downloads it to, and NOT from an environment variable
    /// disconnected from the rest of the system — an earlier bug read
    /// I3K_DATA_DIR, which nothing ever set.
    ///
    /// Requires TESSDATA_DIR_FOR_TEST=/path/to/dir/containing/tessdata/{ita,eng}.traineddata
    /// — the data root, mirroring {data} in the manifest, not the tessdata
    /// folder itself. Skipped gracefully when unset.
    #[test]
    fn tessdata_resolved_from_data_dir() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let (Ok(scratch_lib), Ok(data_dir)) = (
            std::env::var("PDFIUM_LIB_FOR_TEST"),
            std::env::var("TESSDATA_DIR_FOR_TEST"),
        ) else {
            eprintln!("PDFIUM_LIB_FOR_TEST/TESSDATA_DIR_FOR_TEST unset — skipping");
            return;
        };
        std::env::set_var("PDFIUM_DYNAMIC_LIB_PATH", &scratch_lib);

        let pdf_bytes = minimal_pdf_with_text("data dir tessdata roundtrip");
        let tmp_pdf = std::env::temp_dir().join("i3k_ocr_smoke_datadir.pdf");
        std::fs::write(&tmp_pdf, &pdf_bytes).expect("writing the test PDF");

        let result = ocr_pdf(&tmp_pdf, 1, std::path::Path::new(&data_dir));

        std::env::remove_var("PDFIUM_DYNAMIC_LIB_PATH");
        let _ = std::fs::remove_file(&tmp_pdf);

        let text = result.expect("ocr_pdf must succeed reading tessdata from {data_dir}/tessdata/");
        assert!(
            text.to_lowercase().contains("roundtrip") || text.to_lowercase().contains("data dir"),
            "unexpected OCR text: {text:?}"
        );
    }
}
