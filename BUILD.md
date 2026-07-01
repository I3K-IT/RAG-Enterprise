# BUILD — dipendenze native e comandi

Guida per il primo `cargo build` sulla macchina di produzione.
Questo è il **scaffold (Fase 0)**: il binario prodotto parte e va in `todo!()` —
build verde = linking ok, nessuna logica implementata ancora.

---

## 1. Toolchain Rust

```sh
rustup default stable   # oppure usa stable già installato
rustc --version         # deve essere >= 1.75 (edition 2021, async fn in traits, sqlx 0.8)
```

Se la versione è più vecchia:

```sh
rustup update stable
```

Non è necessaria la toolchain nightly.

---

## 2. Dipendenze di sistema

### 2a. leptess → libtesseract + libleptonica (dev libs) + traineddata

`leptess` si linka **a compile-time** contro `libtesseract`/`libleptonica` di sistema —
servono le dev libs sulla macchina di build (bundling cross-platform: rimandato a
Fase 4, packaging — vedi CLAUDE.md):

```sh
# Ubuntu 22.04 / 24.04
sudo apt-get install -y libtesseract-dev libleptonica-dev
```

**Traineddata (ita+eng):** risolta a runtime da `{data_dir}/tessdata/` — la stessa
radice dati di `Settings.data.data_path()` (dove il bootstrap/manifest scarica tutti
gli altri modelli). È nel `manifest.toml` come `tessdata-ita`/`tessdata-eng`
(sorgente: [tessdata_best](https://github.com/tesseract-ocr/tessdata_best), massima
accuratezza) — al primo avvio il bootstrap la scarica e verifica da sola, nessun
`apt-get install tesseract-ocr-ita` richiesto in produzione.

**Sviluppo senza bootstrap** (es. `cargo test --features ocr` prima di aver mai
avviato il binario): o esegui il bootstrap una volta, oppure installa i language pack
di sistema come fallback — `leptess` ricade su `TESSDATA_PREFIX` se
`{data_dir}/tessdata/` non esiste:

```sh
sudo apt-get install -y tesseract-ocr-ita tesseract-ocr-eng
find /usr/share/tesseract-ocr -name "*.traineddata"   # verifica dove sono finite
export TESSDATA_PREFIX=/usr/share/tesseract-ocr/5/    # adatta alla versione trovata
```

**Test di non-regressione** (verifica vera, non solo compilazione — vedi
`src/documents/ocr.rs::smoke_test`):

```sh
export PDFIUM_LIB_FOR_TEST=/path/assoluto/a/libpdfium.so     # scaricata come in §2b
export TESSDATA_DIR_FOR_TEST=/path/a/cartella/con/tessdata/  # contiene tessdata/{ita,eng}.traineddata
cargo test --features ocr documents::ocr::smoke_test
```

---

### 2b. pdfium-render → libpdfium (bundlabile, no install system-wide richiesta)

`pdfium-render` carica `libpdfium` **a runtime da un path esplicito** (`Pdfium::bind_to_library`,
vedi `resolve_pdfium_library_path()` in `ocr.rs`), non da un nome di libreria "di sistema".
Ordine di risoluzione:

1. `PDFIUM_DYNAMIC_LIB_PATH` (override esplicito — comodo in sviluppo).
2. Un file `libpdfium.so`/`pdfium.dll`/`libpdfium.dylib` **accanto all'eseguibile**, poi in
   `./lib/` accanto all'eseguibile — questo è il layout di una build **distribuita e bundlata**.
3. Fallback: ricerca di sistema standard (dlopen) — comoda in sviluppo se già installata
   via apt/brew, **non richiesta** per il binario finale.

La fonte ufficiale dei binari precompilati, per **tutte** le piattaforme (Linux x64/arm64,
macOS x64/arm64/universal, Windows x64/arm64/x86) è
[bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases) — build
automatiche settimanali, nessuna compilazione necessaria da parte nostra.

**Sviluppo locale (rapido, opzione 1 sopra):**

```sh
wget https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz
tar -xzf pdfium-linux-x64.tgz          # estrae ./lib/libpdfium.so
export PDFIUM_DYNAMIC_LIB_PATH=$PWD/lib/libpdfium.so
cargo build --features ocr
```

**Build distribuita (opzione 2 sopra, quella pensata per il pacchetto finale):** scarica
l'archivio per la piattaforma target dalle release di pdfium-binaries e copia
`libpdfium.so` / `pdfium.dll` / `libpdfium.dylib` nella stessa cartella del binario
compilato (o in una sua sottocartella `lib/`) prima di distribuire il pacchetto. Nessun
passaggio di installazione richiesto sulla macchina dell'utente finale.

**Test di non-regressione:** `src/documents/ocr.rs` contiene un test che verifica
davvero il caricamento bundlato (non solo che compili). Per eseguirlo:

```sh
export PDFIUM_LIB_FOR_TEST=/path/assoluto/a/libpdfium.so   # una copia scaricata come sopra
cargo test --features ocr bundled_pdfium_path_resolution_and_ocr_roundtrip
```

Senza questa variabile il test viene saltato (nessun fallimento) — non è richiesto in CI
(che comunque non esiste ancora, vedi CLAUDE.md) né bloccante per gli altri test.

---

### 2c. Candle CUDA → CUDA Toolkit (NO cuDNN)

Candle usa **cudarc + cuBLAS**. **Non richiede cuDNN.**
Ha bisogno del CUDA Toolkit per `libcublas.so`, `libcurand.so` e il compilatore `nvcc`
(alcune crate di cudarc compilano kernel CUDA).

**Verifica cosa è già installato** (hai già eullm che usa CUDA):

```sh
nvcc --version              # deve esserci — versione 12.x consigliata
nvidia-smi                  # verifica driver e GPU
ldconfig -p | grep cublas   # libcublas.so deve comparire
```

**Se mancano le dev headers** (frequente su macchine che hanno solo il runtime):

```sh
# Ubuntu — adatta il numero di versione (es. 12-4, 12-6, 12-8)
sudo apt-get install -y \
    cuda-nvcc-12-8 \
    libcublas-dev-12-8 \
    libcurand-dev-12-8
```

> **Nota per RTX 5070 Ti (compute capability 12.0 / Blackwell sm_120):**
> Richiede CUDA Toolkit **12.8 o superiore** per supporto completo sm_120.
> Con versioni precedenti candle può compilare ma fallire in fase di JIT dei kernel.
> Se hai già CUDA 12.x installato per eullm, verifica con `nvcc --version`.

**Candle non usa cuDNN** — se hai `libcudnn` installato è ignorato. Non installarlo apposta.

---

## 3. Comandi di build

### Build CPU-only (senza GPU, senza OCR)

Utile per verificare il linking di tutte le altre dipendenze:

```sh
cargo build
```

Nessuna dipendenza nativa richiesta oltre alla toolchain Rust.

### Build con OCR, CPU-only

```sh
cargo build --features ocr
```

Richiede: libtesseract-dev, libleptonica-dev, libpdfium.so (§2a, §2b).

### Build completa — OCR + GPU (produzione)

```sh
cargo build --features ocr,cuda
```

Richiede: tutto il precedente + CUDA Toolkit con libcublas-dev (§2c).

### Build release (ottimizzata)

```sh
cargo build --release --features ocr,cuda
```

Il binario finale è in `target/release/i3k-rag-engine`.

---

## 4. Cosa aspettarsi

Il binario **si compila ma non fa nulla di utile**: quasi tutto il codice è
`todo!()` (stub di Fase 0). Se lo esegui:

```sh
./target/release/i3k-rag-engine
```

Otterrai un panic immediato con messaggio tipo:
`thread 'main' panicked at 'not yet implemented: axum router + server bind — Fase 1'`

Questo è **corretto** per lo scaffold. **Build verde = linking ok.**

### Errori di build attesi e come interpretarli

| Errore | Causa | Soluzione |
|---|---|---|
| `Package lept was not found` | `apt install libtesseract-dev libleptonica-dev` mancante | §2a |
| `libpdfium.so: cannot open` | pdfium non installata | §2b |
| `error: could not find CUDA` | nvcc o libcublas-dev assenti | §2c |
| `ld: cannot find -lcublas` | libcublas-dev non installato | §2c |
| `CUDA compute capability sm_12X not supported` | CUDA Toolkit < 12.8 su RTX 5070 Ti | aggiorna toolkit |

### Warning attesi

`cargo build` produce ~80 warning `unused import / dead_code`: sono tutti attesi —
i moduli sono stub. Nessun warning è un errore da correggere ora.

---

## 5. Variabili d'ambiente utili per il primo run (Fase 1)

Quando in Fase 1 il binario inizierà a fare cose reali, avrà bisogno di:

```sh
# Esempio .env (copiare e personalizzare)
SERVER__HOST=0.0.0.0
SERVER__PORT=8000
DATABASE__URL=sqlite://rag_users.db
AUTH__JWT_SECRET=cambia_questo_segreto
AUTH__ADMIN_DEFAULT_PASSWORD=cambia_questa_password
QDRANT__URL=http://localhost:6333
QDRANT__COLLECTION=rag_documents
EULLM__URL=http://localhost:11434
EULLM__MODEL=qwen3:14b
EMBEDDINGS__MODEL_ID=BAAI/bge-m3
TESSDATA_PREFIX=/usr/share/tesseract-ocr/5/
RUST_LOG=info
```

La config usa il separatore `__` (doppio underscore) per i livelli gerarchici.
