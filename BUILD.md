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

### 2a. leptess → libtesseract + libleptonica + traineddata

```sh
# Ubuntu 22.04 / 24.04
sudo apt-get install -y \
    libtesseract-dev \
    libleptonica-dev \
    tesseract-ocr \
    tesseract-ocr-ita \
    tesseract-ocr-eng
```

**Dove vanno i traineddata:**
Il pacchetto `tesseract-ocr-ita` installa `/usr/share/tesseract-ocr/*/tessdata/ita.traineddata`
(versione 4 o 5 secondo la distro). Verifica con:

```sh
find /usr/share/tesseract-ocr -name "*.traineddata" | head -5
```

**TESSDATA_PREFIX** (necessario solo se leptess non trova i file):

```sh
# Se tessdata è in /usr/share/tesseract-ocr/5/tessdata/
export TESSDATA_PREFIX=/usr/share/tesseract-ocr/5/
# o versione 4:
export TESSDATA_PREFIX=/usr/share/tesseract-ocr/4.00/
```

Aggiungilo a `.env` / `~/.bashrc` se necessario.
In Fase 1 il percorso traineddata sarà configurabile via `Settings`.

---

### 2b. pdfium-render → libpdfium

`pdfium-render` richiede `libpdfium.so` — **non è nei repository apt**.
La fonte ufficiale dei binari precompilati è [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases).

```sh
# Scarica l'ultimo release linux-x64
# Sostituisci V con il numero di versione corrente sulla pagina releases
wget https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz
tar -xzf pdfium-linux-x64.tgz          # estrae ./lib/libpdfium.so (e header)
sudo cp lib/libpdfium.so /usr/local/lib/
sudo ldconfig
```

**Verifica:**

```sh
ldconfig -p | grep pdfium              # deve comparire /usr/local/lib/libpdfium.so
```

**In alternativa** (senza installare system-wide), puoi usare la variabile d'ambiente
al momento del build e del run:

```sh
export PDFIUM_DYNAMIC_LIB_PATH=/path/assoluto/a/libpdfium.so
cargo build --features ocr
# e poi anche per il run:
LD_LIBRARY_PATH=/usr/local/lib ./target/release/i3k-rag-engine
```

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
