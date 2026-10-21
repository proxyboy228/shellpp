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
//! `service` (W2); `error` and this file (lead). Stage 2 (stage 2 plan,
//! section 9): `keys`, `known_hosts`, `history`, `os_detect`, `connlog` and the
//! stage 2 parts of `model`, `index`, `settings`, `store`, `service` (S4);
//! the stage 2 section of `service/dto.rs`, `service/host_api.rs` and the
//! facade signatures are the contract of the lead.

#![forbid(unsafe_code)]

pub mod connlog;
pub mod crypto;
pub mod error;
pub mod history;
pub mod ids;
pub mod import;
pub mod index;
pub mod keys;
pub mod known_hosts;
pub mod lock;
pub mod model;
pub mod notify;
pub mod os_detect;
pub mod platform;
pub mod service;
pub mod settings;
pub mod snippet;
pub mod store;
pub mod vault;

pub use error::{CoreError, FieldError, SshReason, ValidationReason};
pub use service::{Core, CoreConfig};
