use tracing_subscriber::EnvFilter;

/// Idempotent on purpose (`try_init`, not `init`, which panics on a second
/// call): a Pro launcher needs tracing set up before its own composition
/// root runs (e.g. to log why a license failed to load), which is before
/// `run_with_extensions` gets a chance to call this itself — so this must
/// tolerate being called twice, silently keeping whichever subscriber was
/// installed first.
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init();
}
