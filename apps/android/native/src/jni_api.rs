//! The JNI exports of `dev.shellpp.android.Native` (a Kotlin `object` with
//! `@JvmStatic external` functions). Each runs inside
//! `EnvUnowned::with_env`, which catches a panic, and turns an error into a
//! Java `RuntimeException` with the error's text (architecture 2). Sessions
//! are addressed by their handles, never by pointers.
//!
//! A platform module (CONTRIBUTING.md, "Unsafe code"): the exported symbols
//! (`no_mangle`) are its only unsafe code.
#![allow(unsafe_code)]

use std::sync::OnceLock;
use std::time::Duration;

use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jint, jlong};
use secrecy::SecretString;

use crate::engine::{Engine, EngineError};
use crate::input::{self, Cell, Key};
use crate::screen;

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// An error of a native call: JNI's own, or the engine's.
#[derive(Debug)]
enum NativeError {
    Jni(jni::errors::Error),
    Engine(EngineError),
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jni(error) => write!(f, "jni: {error}"),
            Self::Engine(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for NativeError {}

impl From<jni::errors::Error> for NativeError {
    fn from(error: jni::errors::Error) -> Self {
        Self::Jni(error)
    }
}

impl From<EngineError> for NativeError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

fn engine() -> Result<&'static Engine, NativeError> {
    ENGINE
        .get()
        .ok_or_else(|| NativeError::Engine(EngineError("the engine is not started".to_owned())))
}

fn id(handle: jlong) -> u64 {
    u64::try_from(handle).unwrap_or(0)
}

fn cell(col: jint, row: jint) -> Cell {
    Cell {
        col: u16::try_from(col).unwrap_or(0),
        row: u16::try_from(row).unwrap_or(0),
    }
}

/// `start(filesDir)`: opens the core and the session layer once per process.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_start<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    files: JString<'caller>,
) -> jboolean {
    unowned
        .with_env(|env| -> Result<jboolean, NativeError> {
            let files = files.try_to_string(env)?;
            if ENGINE.get().is_none() {
                let started = Engine::start(std::path::Path::new(&files))?;
                let _ = ENGINE.set(started);
            }
            Ok(true)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `keyDir()`: where the app copies a picked key file.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_keyDir<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    unowned
        .with_env(|env| -> Result<JString<'caller>, NativeError> {
            let dir = engine()?.key_dir().to_string_lossy().into_owned();
            Ok(JString::from_str(env, dir)?)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `open(address, port, username, cols, rows, dark)`: a quick connect;
/// returns the session's handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_open<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    address: JString<'caller>,
    port: jint,
    username: JString<'caller>,
    cols: jint,
    rows: jint,
    dark: jboolean,
) -> jlong {
    unowned
        .with_env(|env| -> Result<jlong, NativeError> {
            let address = address.try_to_string(env)?;
            let username = username.try_to_string(env)?;
            let port = u16::try_from(port).ok().filter(|port| *port > 0);
            let cols = u16::try_from(cols).unwrap_or(80).max(1);
            let rows = u16::try_from(rows).unwrap_or(24).max(1);
            let opened = engine()?.open(&address, port, Some(&username), cols, rows, dark)?;
            Ok(jlong::try_from(opened).unwrap_or(0))
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `nextEvent(timeoutMs)`: the next session event as JSON, or null.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_nextEvent<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    timeout_ms: jint,
) -> JString<'caller> {
    unowned
        .with_env(|env| -> Result<JString<'caller>, NativeError> {
            let timeout = Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0));
            match engine()?.next_event(timeout) {
                Some(event) => Ok(JString::from_str(env, event)?),
                None => Ok(JString::default()),
            }
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `reply(session, prompt, kind, text)`: answers a prompt; `text` is the
/// UTF-8 of a password or an answer, wiped on both sides after the call.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_reply<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
    prompt: jlong,
    kind: JString<'caller>,
    text: JByteArray<'caller>,
) {
    unowned
        .with_env(|env| -> Result<(), NativeError> {
            let kind = kind.try_to_string(env)?;
            let text = if text.is_null() {
                None
            } else {
                let mut bytes = env.convert_byte_array(&text)?;
                let value = String::from_utf8(bytes.clone())
                    .ok()
                    .map(SecretString::from);
                bytes.fill(0);
                value
            };
            engine()?.reply(id(session), id(prompt), &kind, text)?;
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `key(session, code, ctrl, alt)`: a key of the extra keys row
/// (`Native.KEY_*`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_key<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
    code: jint,
    ctrl: jboolean,
    alt: jboolean,
) {
    unowned
        .with_env(|_env| -> Result<(), NativeError> {
            let engine = engine()?;
            let Some(key) = Key::from_code(code) else {
                return Ok(());
            };
            let modes = screen::modes(&engine.terminal(id(session))?);
            engine.send(id(session), input::key_bytes(key, modes, ctrl, alt))?;
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `text(session, text, ctrl, alt)`: text from the soft keyboard.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_text<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
    text: JString<'caller>,
    ctrl: jboolean,
    alt: jboolean,
) {
    unowned
        .with_env(|env| -> Result<(), NativeError> {
            let text = text.try_to_string(env)?;
            engine()?.send(id(session), input::text_bytes(&text, ctrl, alt))?;
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `paste(session, text)`: a paste, bracketed when the program asked for it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_paste<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
    text: JString<'caller>,
) {
    unowned
        .with_env(|env| -> Result<(), NativeError> {
            let text = text.try_to_string(env)?;
            let engine = engine()?;
            let terminal = engine.terminal(id(session))?;
            let bracketed = terminal
                .term()
                .lock()
                .mode()
                .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
            let clean = shellpp_terminal::paste::sanitize(&text);
            engine.send(
                id(session),
                shellpp_terminal::paste::paste_bytes(&clean, bracketed),
            )?;
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `putCursor(session, col, row)`: puts the cursor on the tapped cell
/// (blocks for the walk; call it off the main thread). `true` when it got
/// there or a click was sent.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_putCursor<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
    col: jint,
    row: jint,
) -> jboolean {
    unowned
        .with_env(|_env| -> Result<jboolean, NativeError> {
            let wait = Duration::from_millis(400);
            Ok(engine()?.put_cursor(id(session), cell(col, row), wait)?)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `snapshot(session)`: the visible screen (`screen.rs` describes it).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_snapshot<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
) -> JByteArray<'caller> {
    unowned
        .with_env(|env| -> Result<JByteArray<'caller>, NativeError> {
            let bytes = screen::snapshot(&engine()?.terminal(id(session))?);
            Ok(env.byte_array_from_slice(&bytes)?)
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `awaitWake(session)`: blocks until the screen has something new;
/// `false` when the session's terminal is gone.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_awaitWake<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
) -> jboolean {
    unowned
        .with_env(|_env| -> Result<jboolean, NativeError> { Ok(engine()?.await_wake(id(session))) })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `resize(session, cols, rows)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_resize<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
    cols: jint,
    rows: jint,
) {
    unowned
        .with_env(|_env| -> Result<(), NativeError> {
            let cols = u16::try_from(cols).unwrap_or(1);
            let rows = u16::try_from(rows).unwrap_or(1);
            engine()?.resize(id(session), cols, rows)?;
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `scroll(session, lines)`: up into the scrollback when positive, 0 to
/// the bottom.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_scroll<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
    lines: jint,
) {
    unowned
        .with_env(|_env| -> Result<(), NativeError> {
            engine()?.scroll(id(session), lines)?;
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `close(session)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_close<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    session: jlong,
) {
    unowned
        .with_env(|_env| -> Result<(), NativeError> {
            engine()?.close(id(session));
            Ok(())
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

/// `openSessions()`: how many sessions are open (the service's
/// notification).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_shellpp_android_Native_openSessions<'caller>(
    mut unowned: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> jint {
    unowned
        .with_env(|_env| -> Result<jint, NativeError> {
            Ok(jint::try_from(engine()?.open_sessions().len()).unwrap_or(jint::MAX))
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}
