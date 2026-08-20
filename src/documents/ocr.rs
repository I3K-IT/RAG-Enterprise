//! OCR for scanned PDFs: pdfium-render → Tesseract (ita+eng), both loaded at
//! runtime through libloading. Neither the build machine nor the target
//! machine needs a system install of either: the manifest downloads them like
//! every other component, and this file dlopens them from a path next to the
//! executable. Tessdata is looked up in {data_dir}/tessdata/ — the same
//! data_dir the manifest and bootstrap use — then in the system
//! TESSDATA_PREFIX.
//!
//! Tesseract's own build must set its RPATH/RUNPATH to `$ORIGIN` (Linux) so
//! it finds its sibling libleptonica next to itself with no LD_LIBRARY_PATH
//! and no dependency on the directory it happened to be built in — verified:
//! without it, the library still loads on the machine that built it (its
//! RUNPATH silently points at the build directory) and fails everywhere else.
//! Windows needs no equivalent: LoadLibrary already searches the directory of
//! the main executable first, where both DLLs are bundled.
//!
//! Pixels go from pdfium to Tesseract as raw RGBA (bitmap.as_rgba_bytes()),
//! not as a re-encoded PNG: Tesseract's C API takes raw pixels directly
//! (TessBaseAPISetImage), so encoding one just to decode it straight back
//! inside Tesseract was pure overhead — and pulled in the `image` crate, and
//! through it libpng/libjpeg/etc., for nothing.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

use anyhow::{Context, Result};

// ── libpdfium ────────────────────────────────────────────────────────────────

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

// ── libtesseract ─────────────────────────────────────────────────────────────

/// The file name libtesseract is bundled under. We build this ourselves (see
/// the module doc), so the name is our own choice, matching the CI build.
#[cfg(target_os = "windows")]
const TESSERACT_LIBRARY_NAME: &str = "libtesseract55.dll";
#[cfg(target_os = "macos")]
const TESSERACT_LIBRARY_NAME: &str = "libtesseract.5.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const TESSERACT_LIBRARY_NAME: &str = "libtesseract.so.5";

fn resolve_tesseract_library_path() -> std::path::PathBuf {
    // 1. Explicit override, mirroring PDFIUM_DYNAMIC_LIB_PATH.
    if let Ok(p) = std::env::var("TESSERACT_DYNAMIC_LIB_PATH") {
        return std::path::PathBuf::from(p);
    }

    // 2. Next to the executable, then in ./lib/ beside it.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(TESSERACT_LIBRARY_NAME);
            if candidate.is_file() {
                return candidate;
            }
            let candidate = dir.join("lib").join(TESSERACT_LIBRARY_NAME);
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    // 3. Fallback: the bare name → standard dlopen system search.
    std::path::PathBuf::from(TESSERACT_LIBRARY_NAME)
}

type TessHandle = *mut c_void;

/// The handful of Tesseract C API entry points OCR needs. Loaded once per
/// `ocr_pdf` call rather than cached process-wide: ingestion is not hot enough
/// for the dlopen cost to matter, and not caching means a corrupted or
/// missing library fails the one document being ingested, not the process.
struct TessApi {
    _lib: libloading::Library,
    create: unsafe extern "C" fn() -> TessHandle,
    init3: unsafe extern "C" fn(TessHandle, *const c_char, *const c_char, c_int) -> c_int,
    set_image: unsafe extern "C" fn(TessHandle, *const u8, c_int, c_int, c_int, c_int),
    get_utf8_text: unsafe extern "C" fn(TessHandle) -> *mut c_char,
    delete_text: unsafe extern "C" fn(*mut c_char),
    end: unsafe extern "C" fn(TessHandle),
    delete: unsafe extern "C" fn(TessHandle),
}

impl TessApi {
    fn load(path: &std::path::Path) -> Result<Self> {
        unsafe {
            let lib = libloading::Library::new(path).map_err(|e| {
                anyhow::anyhow!(
                    "libtesseract not found at {}: {e}\n  → place it next to the executable \
                     (or in ./lib/), or set TESSERACT_DYNAMIC_LIB_PATH",
                    path.display()
                )
            })?;
            // Deliberately NOT going through `*const ()` and `.cast()`: the
            // struct field's declared type below drives `lib.get::<T>()`'s
            // inference exactly as it would for a plain `let x: T = ...`, and
            // a manual raw-pointer cast to a function-pointer type here
            // produced a `Symbol` that crashed on the first call — verified
            // the hard way (SIGSEGV), not a style preference.
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get($name).with_context(|| {
                        format!(
                            "libtesseract: missing symbol {}",
                            String::from_utf8_lossy($name)
                        )
                    })?
                };
            }
            Ok(Self {
                create: sym!(b"TessBaseAPICreate\0"),
                init3: sym!(b"TessBaseAPIInit3\0"),
                set_image: sym!(b"TessBaseAPISetImage\0"),
                get_utf8_text: sym!(b"TessBaseAPIGetUTF8Text\0"),
                delete_text: sym!(b"TessDeleteText\0"),
                end: sym!(b"TessBaseAPIEnd\0"),
                delete: sym!(b"TessBaseAPIDelete\0"),
                _lib: lib,
            })
        }
    }
}

/// One OCR session: a TessBaseAPI handle plus the vtable that operates on it.
/// `Drop` calls End then Delete unconditionally — Init3 failing before a
/// caller gets a `TessSession` never happens, since `new()` checks the
/// return code itself and never hands back a handle that failed to init.
struct TessSession<'a> {
    api: &'a TessApi,
    handle: TessHandle,
}

impl<'a> TessSession<'a> {
    fn new(api: &'a TessApi, tessdata_path: Option<&str>, languages: &str) -> Result<Self> {
        unsafe {
            let handle = (api.create)();
            if handle.is_null() {
                anyhow::bail!("TessBaseAPICreate returned null");
            }
            let session = Self { api, handle };

            let dir = tessdata_path.map(CString::new).transpose()?;
            let dir_ptr = dir.as_deref().map_or(std::ptr::null(), CStr::as_ptr);
            let langs = CString::new(languages)?;
            // OEM_DEFAULT = 3: whichever of the legacy/LSTM engines the
            // installed tessdata provides — the .traineddata the manifest
            // downloads only ships the LSTM data, so this resolves to LSTM.
            let rc = (session.api.init3)(session.handle, dir_ptr, langs.as_ptr(), 3);
            if rc != 0 {
                anyhow::bail!(
                    "TessBaseAPIInit3 failed (rc={rc})\n  → tessdata {languages} in {:?}?",
                    tessdata_path.unwrap_or("TESSDATA_PREFIX")
                );
            }
            Ok(session)
        }
    }

    /// `rgba` must be `width * height * 4` bytes, tightly packed (no row
    /// padding) — exactly what `PdfBitmap::as_rgba_bytes()` returns. Verified
    /// against 3-byte RGB too, but RGBA is what pdfium already hands us, so
    /// there is no conversion step to get wrong.
    fn set_image_rgba(&self, rgba: &[u8], width: i32, height: i32) {
        unsafe {
            (self.api.set_image)(
                self.handle,
                rgba.as_ptr(),
                width,
                height,
                4,
                width * 4,
            );
        }
    }

    fn get_text(&self) -> Result<String> {
        unsafe {
            let raw = (self.api.get_utf8_text)(self.handle);
            if raw.is_null() {
                anyhow::bail!("TessBaseAPIGetUTF8Text returned null");
            }
            let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
            (self.api.delete_text)(raw);
            Ok(text)
        }
    }
}

impl Drop for TessSession<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.api.end)(self.handle);
            (self.api.delete)(self.handle);
        }
    }
}

/// `data_dir` is the same data root resolved by `Settings.data.data_path()`;
/// the bootstrap and manifest download the tessdata into
/// `{data_dir}/tessdata/`.
pub fn ocr_pdf(
    path: &std::path::Path,
    page_count: u32,
    data_dir: &std::path::Path,
) -> Result<super::parser::ExtractedText> {
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

    let tess = TessApi::load(&resolve_tesseract_library_path())?;

    // Look for tessdata in {data_dir}/tessdata/, where the manifest downloads
    // it; if absent (development without the bootstrap) fall back to the
    // system TESSDATA_PREFIX.
    let tessdata_path = Some(data_dir.join("tessdata"))
        .filter(|p| p.is_dir())
        .map(|p| p.display().to_string());

    let mut full_text = String::new();
    let mut pages: Vec<super::parser::PageSpan> = Vec::new();

    for i in 0..(page_count as usize) {
        let page = doc
            .pages()
            .get(i as u16)
            .with_context(|| format!("pdfium: page {i}"))?;

        // Rasterise at ~200 dpi (A4: 1654×2339 px)
        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(1654)
                    .set_maximum_height(2339),
            )
            .with_context(|| format!("pdfium: render page {i}"))?;

        let rgba = bitmap.as_rgba_bytes();
        let (width, height) = (bitmap.width() as i32, bitmap.height() as i32);

        let session = TessSession::new(&tess, tessdata_path.as_deref(), "ita+eng")?;
        session.set_image_rgba(&rgba, width, height);
        let page_text = session.get_text().with_context(|| format!("OCR page {i}"))?;

        tracing::debug!(page = i, chars = page_text.len(), "OCR page");
        let start = full_text.len();
        full_text.push_str(&page_text);
        full_text.push('\n');
        pages.push(super::parser::PageSpan { page: (i + 1) as u32, start, end: full_text.len() });
    }

    tracing::info!(
        file = %path.display(),
        pages = page_count,
        chars = full_text.len(),
        "OCR complete"
    );
    Ok(super::parser::ExtractedText { text: full_text, page_count: Some(page_count), pages })
}

#[cfg(test)]
mod smoke_test {
    use super::*;

    /// The tests below mutate process-wide environment variables. cargo test
    /// runs tests in parallel by default, so without serialisation one test
    /// could read the variable the other has just set or removed.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Builds a minimal PDF with no dependencies, containing one line of
    /// vector text, so the pdfium (rasterise) → Tesseract (OCR) pipeline has
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

    /// Smoke test: checks that both libpdfium and libtesseract are resolved
    /// and loaded from a NON-system path — next to the test executable, the
    /// "bundled" layout — and that the pdfium → raw RGBA → Tesseract pipeline
    /// really does extract the expected text.
    ///
    /// Requires (locally only, not in CI):
    ///   - PDFIUM_LIB_FOR_TEST=/path/to/libpdfium.so (downloaded from
    ///     github.com/bblanchon/pdfium-binaries; must NOT be on a system path)
    ///   - TESSERACT_LIB_FOR_TEST=/path/to/libtesseract.so.5, built with
    ///     RPATH=$ORIGIN and a sibling libleptonica in the same directory
    ///   - tessdata ita+eng — set TESSDATA_DIR_FOR_TEST, or install to the
    ///     system TESSDATA_PREFIX
    #[test]
    fn bundled_libraries_path_resolution_and_ocr_roundtrip() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let (Ok(pdfium_scratch), Ok(tesseract_scratch)) = (
            std::env::var("PDFIUM_LIB_FOR_TEST"),
            std::env::var("TESSERACT_LIB_FOR_TEST"),
        ) else {
            eprintln!("PDFIUM_LIB_FOR_TEST/TESSERACT_LIB_FOR_TEST unset — skipping (see the test docs)");
            return;
        };

        // Simulate the release layout: copy both libraries (and libtesseract's
        // own sibling libleptonica, if TESSERACT_LIB_FOR_TEST's directory has
        // one) next to the current test executable, then clear any explicit
        // override so both resolve_*_library_path() functions are forced down
        // the "next to the executable" branch.
        std::env::remove_var("PDFIUM_DYNAMIC_LIB_PATH");
        std::env::remove_var("TESSERACT_DYNAMIC_LIB_PATH");
        let exe = std::env::current_exe().expect("current_exe");
        let dir = exe.parent().expect("exe parent");

        let pdfium_dest = dir.join(pdfium_render::prelude::Pdfium::pdfium_platform_library_name());
        std::fs::copy(&pdfium_scratch, &pdfium_dest).expect("copying libpdfium next to the test binary");

        let tesseract_dest = dir.join(TESSERACT_LIBRARY_NAME);
        std::fs::copy(&tesseract_scratch, &tesseract_dest).expect("copying libtesseract next to the test binary");
        let mut copied_siblings = vec![pdfium_dest.clone(), tesseract_dest.clone()];
        if let Some(src_dir) = std::path::Path::new(&tesseract_scratch).parent() {
            if let Ok(entries) = std::fs::read_dir(src_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.contains("leptonica") {
                        let dest = dir.join(&*name);
                        if std::fs::copy(entry.path(), &dest).is_ok() {
                            copied_siblings.push(dest);
                        }
                    }
                }
            }
        }

        let pdf_bytes = minimal_pdf_with_text("i3k OCR bundling smoke test");
        let tmp_pdf = std::env::temp_dir().join("i3k_ocr_smoke.pdf");
        std::fs::write(&tmp_pdf, &pdf_bytes).expect("writing the test PDF");

        let data_dir = std::env::var("TESSDATA_DIR_FOR_TEST")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("i3k_ocr_smoke_empty_data_dir"));
        let _ = std::fs::create_dir_all(&data_dir);

        let result = ocr_pdf(&tmp_pdf, 1, &data_dir);

        // Clean up before the assertions, so nothing is left behind even if
        // the test fails.
        for f in &copied_siblings {
            let _ = std::fs::remove_file(f);
        }
        let _ = std::fs::remove_file(&tmp_pdf);

        let extracted = result.expect("ocr_pdf must succeed with both libraries bundled next to the executable");
        assert!(
            extracted.text.to_lowercase().contains("bundling")
                || extracted.text.to_lowercase().contains("smoke"),
            "unexpected OCR text: {:?}",
            extracted.text
        );

        // The page span must cover the whole (single-page) text, 1-based.
        assert_eq!(extracted.pages.len(), 1, "expected exactly one page span");
        assert_eq!(extracted.pages[0].page, 1);
        assert_eq!(extracted.pages[0].start, 0);
        assert_eq!(extracted.pages[0].end, extracted.text.len());
    }
}
