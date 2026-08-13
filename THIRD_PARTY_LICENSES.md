# Licenze di terze parti

`i3k-rag-engine` è rilasciato sotto **Apache License 2.0** (vedi `LICENSE`).

Questo documento inventaria i componenti di terze parti — modelli e binari —
che il prodotto **scarica a runtime** o **include nei tarball di release**.
Non sono coperti dalla licenza di questo progetto: ognuno mantiene la propria.

I componenti e i loro sha256 sono pinnati in `manifest.toml`.

## Perché la distinzione "scaricato" vs "incluso" conta

Gli obblighi di licenza scattano sulla **distribuzione**. Per i componenti che
il binario scarica dalla fonte upstream al primo avvio non siamo noi a
distribuirli. Per quelli che serviamo da `www.i3k.dev` o che impacchettiamo
dentro i tarball di release, invece, **il distributore siamo noi** e gli
obblighi di attribuzione ricadono su questo progetto — da cui questo file.

## Componenti ridistribuiti da noi

Serviti da `www.i3k.dev`, quindi ridistribuiti da i3k.

| Componente | Licenza | Origine |
|---|---|---|
| **bge-m3** (pesi, tokenizer, config) — modello di embedding | MIT | [BAAI/bge-m3](https://huggingface.co/BAAI/bge-m3) |
| **Qwen3-14B** (GGUF Q4_K_M) — modello generativo | Apache-2.0 | [Qwen/Qwen3-14B](https://huggingface.co/Qwen/Qwen3-14B) |
| **Qwen3-8B** (GGUF Q4_K_M) — modello generativo | Apache-2.0 | [Qwen/Qwen3-8B](https://huggingface.co/Qwen/Qwen3-8B) |
| **tessdata** `ita` + `eng` — dati OCR Tesseract | Apache-2.0 | [tesseract-ocr/tessdata_best](https://github.com/tesseract-ocr/tessdata_best) |
| **qdrant** (x86_64 musl; aarch64 build custom) — database vettoriale | Apache-2.0 | [qdrant/qdrant](https://github.com/qdrant/qdrant) |

I GGUF Qwen sono opere derivate (quantizzazioni) dei modelli originali: la
Apache-2.0 lo consente, con conservazione delle note di copyright.

La build `qdrant` per aarch64 è ricompilata da sorgente alla stessa versione
upstream, con `JEMALLOC_SYS_WITH_LG_PAGE=16` per le board a page size 64K —
modifica di configurazione di build, nessuna modifica al codice.

## Componenti inclusi nei tarball di release

| Componente | Licenza | Dove |
|---|---|---|
| **qdrant** | Apache-2.0 | `bin/qdrant` nei tarball `linux-arm64*` (vedi `ci.yml`) |

## Componenti scaricati dalla fonte upstream

Scaricati dal binario al primo avvio direttamente dalle release upstream: non
li ridistribuiamo, ma sono elencati per completezza.

| Componente | Licenza | Origine |
|---|---|---|
| **eullm** — motore di inferenza LLM | Apache-2.0 | [eullm/eullm](https://github.com/eullm/eullm) |
| **pdfium** — rasterizzazione PDF per l'OCR | MIT (script di build) su [PDFium](https://pdfium.googlesource.com/pdfium/), BSD-3-Clause | [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) |

Nota su pdfium: il repo `pdfium-binaries` distribuisce sotto MIT il proprio
lavoro di packaging, ma la libreria contenuta è **PDFium** del progetto
Chromium, **BSD-3-Clause**. Chi ridistribuisce il binario deve rispettare
quest'ultima.

## Dipendenze Rust

Le dipendenze compilate nel binario sono elencate in `Cargo.toml` e bloccate
in `Cargo.lock`, con le rispettive licenze nei metadati dei crate. Sono tutte
permissive (MIT / Apache-2.0 / BSD); il progetto **non** accetta dipendenze
copyleft (GPL, AGPL, LGPL statica).

Per rigenerare l'elenco completo:

```
cargo install cargo-about && cargo about generate about.hbs
```

## Librerie di sistema

`leptess` (feature `ocr`) si collega a **libtesseract** e **libleptonica**,
entrambe Apache-2.0, attese sul sistema host o installate dai pacchetti della
distribuzione. Non sono ridistribuite da questo progetto.
