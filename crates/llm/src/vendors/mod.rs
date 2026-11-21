//! Vendors behind the Providers page: the built-in catalog, model listing
//! and "latest per family", browser sign-ins, and remaining quota. Every
//! network call is blocking and cancellable; the app runs them off the UI
//! thread and owns persistence.

pub mod catalog;
pub mod endpoints;
pub mod family;
pub mod models;
pub mod oauth;
pub mod quota;
