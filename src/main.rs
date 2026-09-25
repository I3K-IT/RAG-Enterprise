//! Thin launcher. All real logic lives in `src/lib.rs`, the shared I3K RAG
//! runtime, reusable as a library by other binaries.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    i3k_rag_engine::run().await
}
