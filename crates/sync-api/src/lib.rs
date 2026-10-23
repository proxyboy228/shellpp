//! Shell++ sync API: request and response types, error codes, and the record
//! envelope header codec with its size checks (docs/architecture.md, section 2.2).
//!
//! This crate has no cryptography and no I/O, so the AGPL server can validate
//! envelopes without linking client cryptography. Stage 1 contains only the
//! envelope header codec; stage 7 adds the API bodies (`api`).

#![forbid(unsafe_code)]

pub mod api;
pub mod classes;
pub mod envelope;
