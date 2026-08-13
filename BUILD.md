# BUILD — native dependencies and commands

How to build `i3k-rag-engine` from source.

Most users do not need this: the [releases](../../releases)
ship self-contained tarballs for Linux x86_64 and arm64, with and without CUDA,
each bundling the compiled frontend and everything the binary needs at runtime.
Build from source if you want to modify the engine or target a platform we do
not publish.

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

A plain `cargo build` needs nothing but the Rust toolchain. The dependencies
below are only required by the optional `ocr` and `cuda` features.

### 2a. leptess → libtesseract + libleptonica (dev libs) + traineddata

`leptess` links **at compile time** against the system `libtesseract` /
`libleptonica`, so the dev libraries must be present on the build machine:

```sh
# Ubuntu 22.04 / 24.04
sudo apt-get install -y libtesseract-dev libleptonica-dev
```

**Traineddata (ita+eng)** is resolved at runtime from `{data_dir}/tessdata/` —
the same data root as `Settings.data.data_path()`, where the bootstrap
downloads every other model. It is pinned in `manifest.toml` as `tessdata-ita`
and `tessdata-eng` (source: [tessdata_best](https://github.com/tesseract-ocr/tessdata_best),
the highest-accuracy variant). On first run the bootstrap downloads and
verifies it on its own: no `apt-get install tesseract-ocr-ita` is needed in
production.

**Developing without the bootstrap** — for instance running
`cargo test --features ocr` before the binary has ever started — either run the
bootstrap once, or install the system language packs as a fallback. `leptess`
falls back to `TESSDATA_PREFIX` when `{data_dir}/tessdata/` does not exist:

```sh
sudo apt-get install -y tesseract-ocr-ita tesseract-ocr-eng
find /usr/share/tesseract-ocr -name "*.traineddata"   # check where they landed
export TESSDATA_PREFIX=/usr/share/tesseract-ocr/5/    # adjust to the version found
```

**Regression test** — exercises OCR for real, not just linking (see
`src/documents/ocr.rs::smoke_test`):

```sh
export PDFIUM_LIB_FOR_TEST=/absolute/path/to/libpdfium.so     # downloaded as in §2b
export TESSDATA_DIR_FOR_TEST=/path/to/folder/containing/tessdata/
cargo test --features ocr documents::ocr::smoke_test
```

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
cargo build --features ocr
```

**Distributed build** (option 2 above): download the archive for the target
platform and copy `libpdfium.so` / `pdfium.dll` / `libpdfium.dylib` next to the
compiled binary, or into a `lib/` subfolder beside it, before packaging. The
end user installs nothing.

**Regression test** — verifies the bundled load path for real:

```sh
export PDFIUM_LIB_FOR_TEST=/absolute/path/to/libpdfium.so
cargo test --features ocr bundled_pdfium_path_resolution_and_ocr_roundtrip
```

Without that variable the test is skipped rather than failed: it needs a real
shared library, which not every environment has.

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

### CPU only, no OCR

```sh
cargo build --release
```

No native dependencies beyond the Rust toolchain.

### With OCR, CPU only

```sh
cargo build --release --features ocr
```

Requires libtesseract-dev, libleptonica-dev and libpdfium (§2a, §2b).

### Full build — OCR + GPU

```sh
cargo build --release --features ocr,cuda
```

Requires all of the above plus the CUDA Toolkit with libcublas-dev (§2c).

The binary lands in `target/release/i3k-rag-engine`.

### Tests

```sh
cargo test
cargo test --features ocr    # adds the OCR tests; see §2a/§2b for the env vars
```

---

## 4. Build errors and what they mean

| Error | Cause | Fix |
|---|---|---|
| `Package lept was not found` | `libtesseract-dev` / `libleptonica-dev` missing | §2a |
| `libpdfium.so: cannot open` | pdfium not reachable | §2b |
| `error: could not find CUDA` | `nvcc` or `libcublas-dev` missing | §2c |
| `ld: cannot find -lcublas` | `libcublas-dev` not installed | §2c |
| `CUDA compute capability sm_12X not supported` | CUDA Toolkit older than 12.8 on an RTX 5070 Ti | upgrade the toolkit |

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
