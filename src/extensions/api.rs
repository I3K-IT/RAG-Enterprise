//! API extension point — I3K_RAG_Pro_Open_Core_Architecture.md
//! (rag-enterprise-pro, private repo) section 6.7.
//!
//! No new type needed here: `api::router` takes an
//! `Option<axum::Router<crate::state::AppState>>` as its second parameter.
//! A Pro launcher builds its own router (`/license`, `/pro/...`,
//! `/structured/...`, `/evidence/...`, ...) against the same `AppState` and
//! passes it in; Community's own launcher (`lib::run`) passes `None`. This
//! keeps the merge (`community_router.merge(pro_router)`) exactly where the
//! router is actually built, instead of threading a router type through
//! `ExtensionRegistry` and `AppState` for the whole app's lifetime when it
//! is only ever needed once, at startup.
