//! RAG pipeline — vector search plus LLM.
//!
//! Flow:
//!   embed(query) → qdrant.search(top_k=15, threshold=0.30) → build_context
//!   → build_prompt → eullm.invoke → format_sources
//!
//! Returns (answer, sources, chunks).
