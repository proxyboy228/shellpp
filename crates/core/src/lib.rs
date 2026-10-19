//! Shell++ core: the data model, cryptography, storage, lock, in-memory index,
//! settings, notifications and platform traits (docs/architecture.md, section 2.4).
//!
//! The crate must stay mobile-ready: it never depends on Tauri or other
//! desktop-only crates, never looks up OS directories and never reads
//! user-chosen files by path (docs/architecture.md, section 1.2).
//!
//! The desktop host talks to the core only through [`service::Core`]; the JSON
//! shapes it exchanges with the webview are in [`service::dto`].
//!
//! Module owners for stage 1 (see the stage 1 plan): `ids`, `crypto`, `store`
//! (W1); `model`, `vault`, `lock`, `index`, `settings`, `notify`, `platform`,
//! `service` (W2); `error` and this file (lead).

#![forbid(unsafe_code)]

pub mod crypto;
pub mod error;
pub mod ids;
pub mod index;
pub mod lock;
pub mod model;
pub mod notify;
pub mod platform;
pub mod service;
pub mod settings;
pub mod store;
pub mod vault;

pub use error::{CoreError, FieldError, ValidationReason};
pub use service::{Core, CoreConfig};
