//! Community RAG pipeline (base — vettoriale + LLM).
//!
//! Corrisponde a Python `RAGPipeline` (rag_pipeline.py, MAPPA §2).
//! Flusso Fase 1:
//!   embed(query) → qdrant.search(top_k=15, threshold=0.30) → build_context
//!   → build_prompt → eullm.invoke → format_sources
//!
//! Ritorna (answer, sources, chunks) — 3 valori come il Python (MAPPA §5 step 15).

// TODO Fase 1: implementazione completa
