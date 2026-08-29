# Third-party licences

`i3k-rag-engine` is released under the **GNU Affero General Public License
v3.0 only** (see `LICENSE`), and separately under a commercial licence from
i3k (see `NOTICE`).

This document inventories the third-party components — models and binaries —
that the product **downloads at runtime** or **ships inside the release
tarballs**. They are not covered by this project's licence: each keeps its own.

Components and their sha256 digests are pinned in `manifest.toml`.

## Why the "downloaded" vs "bundled" distinction matters

Licence obligations attach to **distribution**. For components the binary
fetches from their upstream source on first run, we are not the distributor.
For those we serve from `www.i3k.dev` or package inside the release tarballs,
**we are** — and the attribution obligations fall on this project, which is
what this file discharges.

## Components we redistribute

Served from `www.i3k.dev`, therefore redistributed by i3k.

| Component | Licence | Source |
|---|---|---|
| **bge-m3** (weights, tokenizer, config) — embedding model | MIT | [BAAI/bge-m3](https://huggingface.co/BAAI/bge-m3) |
| **Qwen3-14B** (GGUF Q4_K_M) — generative model | Apache-2.0 | [Qwen/Qwen3-14B](https://huggingface.co/Qwen/Qwen3-14B) |
| **Qwen3-8B** (GGUF Q4_K_M) — generative model | Apache-2.0 | [Qwen/Qwen3-8B](https://huggingface.co/Qwen/Qwen3-8B) |
| **tessdata** `ita` + `eng` — Tesseract OCR data | Apache-2.0 | [tesseract-ocr/tessdata_best](https://github.com/tesseract-ocr/tessdata_best) |
| **qdrant** (x86_64 musl; aarch64 custom build) — vector database | Apache-2.0 | [qdrant/qdrant](https://github.com/qdrant/qdrant) |

The Qwen GGUF files are derivative works (quantisations) of the original
models. Apache-2.0 permits this, provided copyright notices are retained.

The aarch64 `qdrant` build is recompiled from source at the same upstream
version with `JEMALLOC_SYS_WITH_LG_PAGE=16`, for boards with a 64K page size.
That is a build-configuration change; no source code was modified.

## Components bundled in the release tarballs

| Component | Licence | Where |
|---|---|---|
| **qdrant** | Apache-2.0 | `bin/qdrant` in the `linux-arm64*` tarballs (see `ci.yml`) |
| **Tesseract** (libtesseract) | Apache-2.0 | `lib/libtesseract.so.5` — every release tarball; built from source by `ci.yml` |
| **Leptonica** (libleptonica) | BSD-2-Clause | `lib/libleptonica.so.6` — every release tarball; built from source by `ci.yml` |

Tesseract and Leptonica are compiled from unmodified upstream sources at
pinned versions (5.5.0 and 1.85.0), with build options limited to disabling
components this project does not use. The shared objects are then relinked
with `RPATH=$ORIGIN` so the bundled pair resolve each other rather than
whatever the host has installed — a link-time change, not a source change.

## Components downloaded from upstream

Fetched by the binary on first run, straight from the upstream releases. We do
not redistribute these; they are listed for completeness.

| Component | Licence | Source |
|---|---|---|
| **eullm** — LLM inference engine | Apache-2.0 | [eullm/eullm](https://github.com/eullm/eullm) |
| **pdfium** — PDF rasterisation for OCR | MIT (build scripts) over [PDFium](https://pdfium.googlesource.com/pdfium/), BSD-3-Clause | [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) |

A note on pdfium: the `pdfium-binaries` repository licenses its own packaging
work under MIT, but the library it ships is **PDFium** from the Chromium
project, which is **BSD-3-Clause**. Anyone redistributing the binary must
honour the latter.

## Rust dependencies

Dependencies compiled into the binary are listed in `Cargo.toml` and locked in
`Cargo.lock`, with their licences in each crate's metadata. All 635 are
permissive (MIT / Apache-2.0 / BSD / ISC / Zlib / Unicode-3.0) apart from four
under MPL-2.0, whose section 3.3 explicitly permits combination under the
(A)GPL. Two crates offer LGPL-2.1-or-later only as one arm of an `OR` with
MIT/Apache-2.0, so the permissive arm applies. There are no GPL-2.0-only
dependencies, which are the ones that would genuinely conflict.

The policy of preferring permissive dependencies still stands, and matters
more than the project's own licence might suggest: a copyleft dependency
would restrict the **commercial** licence i3k grants alongside the AGPL, even
though it would sit perfectly well with the AGPL itself.

To regenerate the full list:

```
cargo install cargo-about && cargo about generate about.hbs
```

## System libraries

None required for OCR. `src/documents/ocr.rs` opens **libtesseract** at
runtime with `libloading`, rather than linking it at build time, and the
release tarballs ship that library and **libleptonica** alongside the binary —
see "Components bundled in the release tarballs" above, which is where their
attribution now lives.

> Corrected: this section previously stated that these two libraries came from
> the host system and were "not redistributed by this project", and gave
> Apache-2.0 for both. Neither still held. The build stopped using `leptess`
> and the `ocr` feature flag when OCR moved to runtime loading, and CI now
> compiles and ships both libraries — so this project *is* their distributor
> and owes the attribution. Leptonica is also **BSD-2-Clause**, not
> Apache-2.0.
