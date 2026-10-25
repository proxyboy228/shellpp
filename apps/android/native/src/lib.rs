//! Shell++ for Android: the JNI layer over the shared Rust core
//! (`docs/mobile/architecture.md`, D213).
//!
//! The run 13 prototype: a quick connect over SSH through the shared session
//! layer (`shellpp-sessions`) into the shared terminal model
//! (`shellpp-terminal`), the screen as a binary snapshot for the Kotlin view,
//! and the keys and taps of the view as bytes for the session. There is no
//! vault yet: the core runs on a throwaway vault in the app's files with a
//! random password, made again at every start, so known hosts last one run.
//!
//! Everything but [`jni_api`] is plain Rust, built and tested on the
//! computer too.

pub mod cursor;
pub mod engine;
pub mod input;
pub mod jni_api;
pub mod screen;
pub mod theme;

pub use engine::{Engine, EngineError};
