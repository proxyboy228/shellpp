//! Shell++ for Android: the JNI layer over the shared Rust core
//! (`docs/mobile/architecture.md`, D213).
//!
//! The core on the user's vault with the app's platform services (the
//! Keystore, the clipboard), the command dispatcher of its service API, SSH
//! sessions through the shared session layer (`shellpp-sessions`) into the
//! shared terminal model (`shellpp-terminal`), the screen as a binary
//! snapshot for the Kotlin view, and the keys and taps of the view as bytes
//! for the session.
//!
//! Everything but [`jni_api`] and [`android_platform`] is plain Rust, built
//! and tested on the computer too.

pub mod android_platform;
pub mod cursor;
pub mod dispatch;
pub mod engine;
pub mod events;
pub mod input;
pub mod jni_api;
pub mod screen;
pub mod theme;

pub use engine::{Engine, EngineError, Services};
