//! Record payloads and types for stage 1 (`vault_meta`, `recovery_key`,
//! `device_info`, `host`, `group`) and stage 2 (`key`, `known_host`,
//! `history_entry`, `connection_log_entry`), validation and normalization, the draft
//! rule, group tree and inheritance helpers, manual order keys and the hybrid
//! logical clock (docs/data-model.md, sections 1-8, 16, 18.1;
//! docs/sync-protocol.md, section 6.2).
//!
//! The draft rule (UI 1.3, D20): a new object is written only once every
//! required field holds a valid value. The core enforces it in
//! [`crate::service::Core::record_save`]: a save without `record_id` runs the
//! full validation of [`host::normalize`] or [`group::normalize`] and writes
//! nothing unless every field is valid ([`draft::creatable`] answers the same
//! question without writing).
//!
//! Owner: W2 (stage 1); S4 (stage 2 types).

pub mod connection_log;
pub mod draft;
pub mod fields;
pub mod group;
pub mod history_entry;
pub mod hlc;
pub mod host;
pub mod inherit;
pub mod key;
pub mod known_host;
pub mod merge;
pub mod order_key;
pub mod payload;
pub mod text;
pub mod time;
pub mod tree;
pub mod types;
pub mod wipe;

pub use hlc::{Hlc, HlcClock};
pub use payload::Payload;
pub use types::RecordKind;
