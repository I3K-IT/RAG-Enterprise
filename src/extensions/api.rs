//! API extension point.
//!
//! No new type needed here: `api::router` takes an
//! `Option<axum::Router<crate::state::AppState>>` as its second parameter.
//! A downstream launcher builds its own routes against the same `AppState`
//! and passes them in; this crate's own launcher (`lib::run`) passes `None`.
//! That keeps the merge exactly where the router is actually built, instead
//! of threading a router type through
//! `ExtensionRegistry` and `AppState` for the whole app's lifetime when it
//! is only ever needed once, at startup.
