//! Thin Community launcher. All real logic lives in `src/lib.rs`, the
//! shared I3K RAG runtime reusable by the Pro binary too — see
//! `I3K_RAG_Pro_Open_Core_Architecture.md` in the `rag-enterprise-pro`
//! repository.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    i3k_rag_engine::run().await
}
