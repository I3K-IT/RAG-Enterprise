# BUILD — native dependencies and commands

How to build `i3k-rag-engine` from source.

Most users do not need this: the [releases](../../releases)
ship self-contained tarballs for Linux x86_64, Linux arm64 and Windows
x86_64, each bundling the compiled frontend and everything the binary needs
at runtime. Build from source if you want to modify the engine, add back
Candle's CUDA support (§2c — source-available, not published), or target a
platform we do not publish.

---

## 1. Rust toolchain

```sh
rustup default stable
rustc --version         # must be >= 1.75 (edition 2021, async fn in traits, sqlx 0.8)
```

If it is older:

```sh
rustup update stable
```

The nightly toolchain is not required.

---

## 2. System dependencies

A plain `cargo build` needs nothing but the Rust toolchain. CUDA (§2c) is the
only build-time system dependency left, and only for `--features cuda`.

### 2a. libtesseract + libleptonica (bundled, no system-wide install)

Like `pdfium-render` in §2b, OCR loads `libtesseract` **at runtime from an
explicit path**, through `libloading`, not by linking against it at compile
time. Nothing on the build machine is required, and — unlike the old
`leptess`-based approach — nothing on the *target* machine is either: the pair
travels inside the release tarball itself, next to the binary.

Unlike pdfium (§2b), Tesseract has no upstream prebuilt-binaries repo, so
there is nowhere to point a manifest.toml entry at — `bootstrap.rs` never
downloads it. Instead `ci.yml` builds both from source in every release job
(Tesseract 5.5.0 + Leptonica 1.85.0) and bundles the result directly into the
tarball: a distro's Tesseract links `libcurl` and `libarchive` and pulls in
roughly fifty shared libraries transitively, for URL-fetching and multi-page
TIFF support this engine never uses, since it only ever feeds Tesseract raw
pixels straight from pdfium. Built with `-DDISABLE_CURL=ON -DDISABLE_ARCHIVE=ON`
and Leptonica's own image-codec options off (no PNG/JPEG/TIFF/WebP — again,
unneeded when the input is already decoded pixels), the pair comes down to
`libtesseract` + `libleptonica` and nothing else beyond libc (on Windows:
statically-linked libgcc/libstdc++/libwinpthread instead, so the DLL pair
needs nothing beyond what a stock Windows install already has). The Linux
build sets `libtesseract`'s RPATH to `$ORIGIN` (via `patchelf`, in `ci.yml`)
so it finds its sibling `libleptonica` next to itself wherever the pair is
copied to — without it, the library only loads on the machine that happens to
still have the original build directory, which is not a fact about the
*library*, it just means the one test that would have caught it never ran
outside that machine's own leftover files.

Resolution order, mirroring `resolve_pdfium_library_path()`:

1. `TESSERACT_DYNAMIC_LIB_PATH` — explicit override, convenient during
   development.
2. `libtesseract.so.5` / `libtesseract55.dll` / `libtesseract.5.dylib` **next
   to the executable**, then in `./lib/` next to it — the layout the release
   tarballs use.
3. Fallback: the bare name → standard dlopen system search.

**Local development**: any working `libtesseract.so.5` + `libleptonica.so.6`
pair is fine for testing against — the OCR output does not depend on which
build produced them, only the pixel pipeline does, and that is exercised the
same way regardless. The distro's plain runtime packages (no `-dev`, no
headers or pkg-config needed since nothing compiles against them) are the
quickest way to get a pair locally:

```sh
# Ubuntu 22.04 / 24.04
sudo apt-get install -y libtesseract5 liblept5
export TESSERACT_DYNAMIC_LIB_PATH=/usr/lib/x86_64-linux-gnu/libtesseract.so.5
```

**Traineddata (ita+eng)** is resolved at runtime from `{data_dir}/tessdata/` —
the same data root as `Settings.data.data_path()`, where the bootstrap
downloads every other model. It is pinned in `manifest.toml` as `tessdata-ita`
and `tessdata-eng` (source: [tessdata_best](https://github.com/tesseract-ocr/tessdata_best),
the highest-accuracy variant). On first run the bootstrap downloads and
verifies it on its own: no `apt-get install tesseract-ocr-ita` is needed in
production.

**Developing without the bootstrap** — for instance running `cargo test`
before the binary has ever started — either run the bootstrap once, or set
`TESSDATA_DIR_FOR_TEST` for the regression test below, or install the system
language packs and let Tesseract fall back to `TESSDATA_PREFIX` when
`{data_dir}/tessdata/` does not exist:

```sh
sudo apt-get install -y tesseract-ocr-ita tesseract-ocr-eng
find /usr/share/tesseract-ocr -name "*.traineddata"   # check where they landed
export TESSDATA_PREFIX=/usr/share/tesseract-ocr/5/    # adjust to the version found
```

**Regression test** — exercises OCR for real, not just symbol resolution (see
`src/documents/ocr.rs::smoke_test`):

```sh
export PDFIUM_LIB_FOR_TEST=/absolute/path/to/libpdfium.so     # downloaded as in §2b
export TESSERACT_LIB_FOR_TEST=/absolute/path/to/libtesseract.so.5
export TESSDATA_DIR_FOR_TEST=/path/to/folder/containing/tessdata/
cargo test bundled_libraries_path_resolution_and_ocr_roundtrip
```

Without those variables the test is skipped rather than failed: it needs real
shared libraries, which not every environment has.

---

### 2b. pdfium-render → libpdfium (bundled, no system-wide install)

`pdfium-render` loads `libpdfium` **at runtime from an explicit path**
(`Pdfium::bind_to_library`, see `resolve_pdfium_library_path()` in `ocr.rs`),
not by system library name. Resolution order:

1. `PDFIUM_DYNAMIC_LIB_PATH` — explicit override, convenient during development.
2. A `libpdfium.so` / `pdfium.dll` / `libpdfium.dylib` **next to the
   executable**, then in `./lib/` next to it. This is the layout of a
   distributed, bundled build, and it is what the release tarballs use.
3. Fallback: the standard system search (dlopen). Handy in development if you
   already installed it via apt/brew; **not required** for the shipped binary.

Prebuilt binaries for every platform (Linux x64/arm64, macOS x64/arm64/universal,
Windows x64/arm64/x86) come from
[bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases),
rebuilt weekly upstream — we compile nothing ourselves.

**Local development** (option 1 above):

```sh
wget https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz
tar -xzf pdfium-linux-x64.tgz          # extracts ./lib/libpdfium.so
export PDFIUM_DYNAMIC_LIB_PATH=$PWD/lib/libpdfium.so
cargo build
```

**Distributed build** (option 2 above): download the archive for the target
platform and copy `libpdfium.so` / `pdfium.dll` / `libpdfium.dylib` next to the
compiled binary, or into a `lib/` subfolder beside it, before packaging. The
end user installs nothing. The regression test for this path is the same one
listed at the end of §2a — it exercises pdfium and Tesseract together, since
that is how OCR is actually used.

---

### 2c. Candle CUDA → CUDA Toolkit (no cuDNN)

Candle uses **cudarc + cuBLAS**. It does **not** require cuDNN. It needs the
CUDA Toolkit for `libcublas.so`, `libcurand.so` and the `nvcc` compiler, since
some cudarc crates compile CUDA kernels.

**Check what is already installed:**

```sh
nvcc --version              # required — 12.x recommended
nvidia-smi                  # driver and GPU
ldconfig -p | grep cublas   # libcublas.so must appear
```

**If the dev headers are missing** — common on machines that only have the
runtime:

```sh
# Ubuntu — adjust the version suffix (e.g. 12-4, 12-6, 12-8)
sudo apt-get install -y \
    cuda-nvcc-12-8 \
    libcublas-dev-12-8 \
    libcurand-dev-12-8
```

> **RTX 5070 Ti (compute capability 12.0, Blackwell sm_120):** needs CUDA
> Toolkit **12.8 or newer** for full sm_120 support. With earlier versions
> Candle may compile but fail when JIT-compiling kernels.

If `libcudnn` happens to be installed it is simply ignored. Do not install it
on purpose.

---

## 3. Build commands

### CPU

```sh
cargo build --release
```

No native dependencies beyond the Rust toolchain. OCR is always compiled in —
see §2a, §2b for how it loads its two libraries at runtime rather than at
build time.

### CPU + GPU

```sh
cargo build --release --features cuda
```

Requires the CUDA Toolkit with libcublas-dev (§2c).

The binary lands in `target/release/i3k-rag-engine`.

### Windows (cross-compiled from Linux, CPU only)

No Windows machine needed — the release binary is cross-compiled from a
Linux host:

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt-get install -y --no-install-recommends \
  g++-mingw-w64-x86-64-posix gcc-mingw-w64-x86-64-posix
# posix, not the win32 variant: Rust's std::thread requires it.

# Tesseract's CMakeLists (built separately, see §2a) looks for -lWs2_32
# (uppercase, the MSVC/Windows convention); mingw only provides lowercase
# libws2_32.a. A case-sensitive Linux filesystem doesn't resolve one for
# the other on its own — Windows itself wouldn't have this problem.
sudo ln -sf libws2_32.a /usr/x86_64-w64-mingw32/lib/libWs2_32.a

cargo build --release --target x86_64-pc-windows-gnu
```

`--features cuda` does not exist for this target: `nvcc` cross-compiling
Windows CUDA code from a Linux host is not a combination cudarc/candle
support — confirmed by trying it, not assumed. eullm remains unaffected,
since it is a separate downloaded process, not compiled here: GPU chat
inference still works on Windows through `eullm-windows-x64-cuda` in
`manifest.toml`, only this binary's own embedding path is CPU-only. See
`config::IngestionEmbedding::Eullm` for routing document-ingestion
embedding through eullm instead — GPU-accelerated on Windows too, since
eullm does its own device management independent of this binary's CUDA
feature.

**`libstdc++-6.dll`**: unlike Tesseract/Leptonica (§2a, statically linked,
no runtime DLL dependency), the engine binary itself dynamically links
libstdc++ — esaxx-rs, a C++ dependency of `tokenizers`' Unigram algorithm,
pulls it in, and the usual `-static-libgcc -static-libstdc++` fix does not
apply here: this binary's final link goes through rustc's C driver (`gcc`,
not `g++`), which does not resolve the reference the way Tesseract's own
CMake/g++ build does — confirmed with several combinations (RUSTFLAGS,
`.cargo/config.toml`, `-C linker=g++`, `+crt-static`), not assumed. Bundle
the DLL instead, from the mingw sysroot, **next to the .exe** (not in
`lib/`: unlike pdfium/Tesseract, which this engine loads itself through
`libloading` with an explicit path, this is a load-time PE dependency
Windows resolves before `main()` runs, and the executable's own directory
is the first place it looks):

```sh
cp /usr/lib/gcc/x86_64-w64-mingw32/*-posix/libstdc++-6.dll ./dist/
```

The binary lands in `target/x86_64-pc-windows-gnu/release/i3k-rag-engine.exe`.
See `.github/workflows/ci.yml`'s `release-windows-x86_64` job for the full
release packaging (frontend, `libstdc++-6.dll`, Tesseract/Leptonica built
per §2a) — this section covers only the plain `cargo build` step.

### Tests

```sh
cargo test
```

The two OCR tests that need real shared libraries skip gracefully unless
`PDFIUM_LIB_FOR_TEST` / `TESSERACT_LIB_FOR_TEST` are set — see §2a, §2b.

---

## 4. Build errors and what they mean

| Error | Cause | Fix |
|---|---|---|
| `error: could not find CUDA` | `nvcc` or `libcublas-dev` missing | §2c |
| `ld: cannot find -lcublas` | `libcublas-dev` not installed | §2c |
| `CUDA compute capability sm_12X not supported` | CUDA Toolkit older than 12.8 on an RTX 5070 Ti | upgrade the toolkit |
| `libpdfium not found at …` (at runtime, from OCR) | pdfium not next to the executable | §2b |
| `libtesseract not found at …` (at runtime, from OCR) | Tesseract not next to the executable | §2a |
| `TessBaseAPIInit3 failed` (at runtime, from OCR) | tessdata missing or wrong language code | §2a — check `{data_dir}/tessdata/` or `TESSDATA_PREFIX` |

---

## 5. Runtime configuration

Configuration keys use `__` (double underscore) to separate hierarchy levels,
and can be set through the environment or a `.env` file:

```sh
SERVER__HOST=0.0.0.0
SERVER__PORT=8000
DATABASE__URL=sqlite://rag_users.db
AUTH__JWT_SECRET=change_this_secret
AUTH__ADMIN_DEFAULT_PASSWORD=change_this_password
QDRANT__URL=http://localhost:6333
QDRANT__COLLECTION=rag_documents
EULLM__URL=http://localhost:11434
EULLM__MODEL=qwen3:14b
EMBEDDINGS__MODEL_ID=BAAI/bge-m3
# true = refuse to start when CUDA is unavailable for embeddings, instead of
# silently degrading to CPU (ingestion takes minutes rather than seconds).
EMBEDDINGS__REQUIRE_GPU=false
RUST_LOG=info
```

`AUTH__JWT_SECRET` and `AUTH__ADMIN_DEFAULT_PASSWORD` have no safe defaults:
set them before exposing the service.

### Embedding via eullm

`EMBEDDINGS__INGESTION_EMBEDDING=eullm` routes both document-ingestion and
query embedding through eullm's own `POST /api/embed` instead of the
in-process Candle path (see `config::IngestionEmbedding` and
`CHANGELOG.md`'s 0.1.33 entry). eullm needs bge-m3 as its own GGUF for
this — not the Candle `.safetensors`/HuggingFace format this binary uses
for `Off`/`CandleGpu`.

eullm's own model resolution never downloads anything at request time: it
only looks in its model store, in a recognized mount point, or at an
explicit path if `EULLM_ALLOW_MODEL_PATHS=1` is set on the eullm process
(not recommended with the API exposed). So this binary provisions the GGUF
itself, once, via eullm's own `pull` — not a direct HTTP download of its
own — before eullm is ever started with `manage_subprocesses=true` (the
default): see `bootstrap::ensure_eullm_embedding_model`. Idempotent and
offline-safe, same as every other component: it checks whether the file is
already at `{data}/models/bge-m3-f16/bge-m3-f16.gguf` first and only
touches the network if it is missing, so a machine that already has it
(including from a previous run) never re-pulls it. sha256-verified after
the pull — `eullm pull <url>` does not verify a URL pull itself, this
project does it independently.

With `manage_subprocesses=false` (eullm run externally — Docker Compose,
systemd, whatever the deployment uses), this binary never touches eullm's
process or files at all, provisioning included: run the same pull
yourself before starting eullm, pointed at whatever `EULLM_MODELS_DIR`
(or store default, `~/.eullm/models`) that eullm instance actually reads:
```sh
EULLM_MODELS_DIR=/path/eullm/will/read eullm pull https://www.i3k.dev/models/bge-m3-gguf/bge-m3-f16.gguf
```

Either way, once provisioned, nothing else changes: ingestion and query
calls already reference the model by name
(`config::EULLM_EMBEDDING_MODEL = "bge-m3-f16"` — the name eullm itself
derives from that URL's filename, confirmed by actually running the pull,
not assumed), exactly like the chat model. No
`EULLM__RESERVE_EMBEDDING_MODEL` (`--embedding-model`) needed for this —
see that setting's own doc comment for why it is usually the wrong choice
on a VRAM-constrained card; plain on-demand `/api/embed` already does its
own coexist-or-evict.
