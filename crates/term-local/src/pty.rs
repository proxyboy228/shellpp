//! A local shell in a pseudo-terminal (portable-pty). portable-pty is
//! blocking, so a reader thread forwards output to a tokio channel and a
//! waiter thread reports the exit status.
//!
//! Lifecycle:
//! - output arrives in chunks of at most 64 KiB on a bounded channel
//!   (capacity 64); the reader thread waits while the channel is full, so the
//!   consumer must keep draining [`LocalSession::output`];
//! - [`LocalSession::exit`] resolves as soon as the process exits; the
//!   trailing output may still be in (or arrive on) the output channel, which
//!   closes once the PTY reaches end of file. Read `output` to its end to get
//!   everything. On Unix a background job that keeps the terminal open can
//!   delay the end of file past the exit;
//! - after the exit, writes and resizes fail with [`LocalError::Closed`];
//! - [`PtyWriter::kill`] ends the process (the exit then reports no code);
//!   dropping every [`PtyWriter`] (the session holds one) does the same, so
//!   no shell outlives its session.
//!
//! On Windows the pseudo console (ConPTY) asks the terminal for the cursor
//! position (`ESC [ 6 n`) when it starts and waits for the answer before the
//! shell's output flows; the terminal emulator (xterm.js) answers it through
//! the normal input path.
//!
//! Nothing here logs paths, output or input.

use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::{mpsc, oneshot};

use crate::LocalError;
use crate::shells::ShellSpec;

/// Largest output chunk.
pub const MAX_CHUNK: usize = 64 * 1024;
/// Capacity of the output channel (chunks).
pub const OUTPUT_CAPACITY: usize = 64;
/// TERM when the configured value is empty or not printable.
const FALLBACK_TERM: &str = "xterm-256color";

/// How to start the shell.
#[derive(Clone, Debug)]
pub struct PtyConfig {
    pub shell: ShellSpec,
    pub cols: u16,
    pub rows: u16,
    /// TERM for the child (setting `terminal.term`).
    pub term: String,
    /// Working directory; `None` = the user's home directory.
    pub cwd: Option<PathBuf>,
}

/// How the shell ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    /// Exit code; `None` when the process was killed or the code is unknown.
    pub code: Option<u32>,
}

/// A running local shell.
pub struct LocalSession {
    /// Output chunks in order; closed when the PTY reaches end of file.
    pub output: tokio::sync::mpsc::Receiver<Vec<u8>>,
    /// Resolves once when the process exits.
    pub exit: tokio::sync::oneshot::Receiver<ExitInfo>,
    writer: PtyWriter,
}

impl std::fmt::Debug for LocalSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LocalSession { .. }")
    }
}

impl LocalSession {
    /// Input, resize and kill handle; cheap to clone.
    pub fn writer(&self) -> PtyWriter {
        self.writer.clone()
    }
}

/// Input side of a [`LocalSession`].
#[derive(Clone)]
pub struct PtyWriter {
    handle: Arc<Handle>,
}

impl std::fmt::Debug for PtyWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PtyWriter { .. }")
    }
}

impl PtyWriter {
    /// Writes input bytes (blocking for at most the PTY buffer; call from a
    /// blocking-friendly context or keep writes small).
    pub fn write(&self, data: &[u8]) -> Result<(), LocalError> {
        let shared = &self.handle.shared;
        if shared.exited.load(Ordering::SeqCst) {
            return Err(LocalError::Closed);
        }
        let mut writer = lock(&shared.writer);
        let Some(writer) = writer.as_mut() else {
            return Err(LocalError::Closed);
        };
        writer
            .write_all(data)
            .and_then(|()| writer.flush())
            .map_err(|err| {
                if shared.exited.load(Ordering::SeqCst) || err.kind() == ErrorKind::BrokenPipe {
                    LocalError::Closed
                } else {
                    LocalError::Io
                }
            })
    }

    /// Changes the terminal size (zero is raised to one).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), LocalError> {
        let shared = &self.handle.shared;
        if shared.exited.load(Ordering::SeqCst) {
            return Err(LocalError::Closed);
        }
        let master = lock(&shared.master);
        let Some(master) = master.as_ref() else {
            return Err(LocalError::Closed);
        };
        master
            .resize(pty_size(cols, rows))
            .map_err(|_| LocalError::Io)
    }

    /// Ends the process (closing a tab). No effect after the exit.
    pub fn kill(&self) {
        self.handle.shared.kill();
    }
}

/// State shared by the writers and the waiter thread.
struct Shared {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    /// Taken by the waiter thread when the process exits.
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    exited: AtomicBool,
    /// Set (under the `killer` lock) when `kill` signals the process.
    killed: AtomicBool,
}

impl Shared {
    fn kill(&self) {
        let mut killer = lock(&self.killer);
        if let Some(killer) = killer.as_mut() {
            self.killed.store(true, Ordering::SeqCst);
            // portable-pty's Windows killer reports success as an error;
            // the waiter thread observes the outcome either way.
            let _ = killer.kill();
        }
    }
}

/// Kills the process when the last [`PtyWriter`] is dropped.
struct Handle {
    shared: Arc<Shared>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shared.kill();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// TERM must be a printable token; anything else falls back.
fn term_value(term: &str) -> &str {
    if !term.is_empty() && term.len() <= 64 && term.bytes().all(|b| b.is_ascii_graphic()) {
        term
    } else {
        FALLBACK_TERM
    }
}

fn command(config: &PtyConfig) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(&config.shell.program);
    cmd.args(&config.shell.args);
    cmd.env("TERM", term_value(&config.term));
    cmd.env("COLORTERM", "truecolor");
    if is_wsl(config) {
        // Pass TERM and COLORTERM into the WSL distribution.
        let mut wslenv = std::env::var("WSLENV").unwrap_or_default();
        for name in ["TERM", "COLORTERM"] {
            let listed = wslenv
                .split(':')
                .any(|entry| entry.split('/').next() == Some(name));
            if !listed {
                if !wslenv.is_empty() {
                    wslenv.push(':');
                }
                wslenv.push_str(name);
            }
        }
        cmd.env("WSLENV", wslenv);
    }
    if let Some(cwd) = config.cwd.clone().or_else(dirs::home_dir) {
        cmd.cwd(cwd);
    }
    cmd
}

fn is_wsl(config: &PtyConfig) -> bool {
    cfg!(windows)
        && config
            .shell
            .program
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("wsl.exe"))
}

/// Starts the shell in a new PTY.
pub fn spawn(config: PtyConfig) -> Result<LocalSession, LocalError> {
    // Check the program first: portable-pty reports a failed start with the
    // command line in its own error text, which never leaves this function.
    let program = &config.shell.program;
    if !program.is_absolute() || !program.is_file() {
        return Err(LocalError::Spawn);
    }

    let pty = portable_pty::native_pty_system()
        .openpty(pty_size(config.cols, config.rows))
        .map_err(|_| LocalError::Spawn)?;
    let reader = pty
        .master
        .try_clone_reader()
        .map_err(|_| LocalError::Spawn)?;
    let writer = pty.master.take_writer().map_err(|_| LocalError::Spawn)?;
    let child = pty
        .slave
        .spawn_command(command(&config))
        .map_err(|_| LocalError::Spawn)?;
    // Only the child keeps the terminal side open, so the end of file
    // follows its exit.
    drop(pty.slave);
    let killer = child.clone_killer();

    let shared = Arc::new(Shared {
        master: Mutex::new(Some(pty.master)),
        writer: Mutex::new(Some(writer)),
        killer: Mutex::new(Some(killer)),
        exited: AtomicBool::new(false),
        killed: AtomicBool::new(false),
    });
    let handle = Arc::new(Handle {
        shared: Arc::clone(&shared),
    });

    let (output_tx, output_rx) = mpsc::channel(OUTPUT_CAPACITY);
    let (exit_tx, exit_rx) = oneshot::channel();

    // From here on, dropping `handle` kills the child.
    std::thread::Builder::new()
        .name("shellpp-pty-reader".to_owned())
        .spawn(move || read_loop(reader, output_tx))
        .map_err(|_| LocalError::Spawn)?;
    let waiter_shared = Arc::clone(&shared);
    if std::thread::Builder::new()
        .name("shellpp-pty-waiter".to_owned())
        .spawn(move || wait_loop(child, waiter_shared, exit_tx))
        .is_err()
    {
        // Without a waiter nobody would close the PTY: kill the child and
        // release the PTY here.
        handle.shared.kill();
        drop(lock(&shared.master).take());
        drop(lock(&shared.writer).take());
        return Err(LocalError::Spawn);
    }

    tracing::debug!("local shell started");
    Ok(LocalSession {
        output: output_rx,
        exit: exit_rx,
        writer: PtyWriter { handle },
    })
}

/// Forwards PTY output until end of file or until the receiver is gone.
fn read_loop(mut reader: Box<dyn Read + Send>, output: mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; MAX_CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if output.blocking_send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            // A closed pipe (Windows) or any other failure ends the output.
            Err(_) => break,
        }
    }
}

/// Waits for the exit, reports it, then closes the PTY so the reader thread
/// reaches end of file.
fn wait_loop(
    mut child: Box<dyn Child + Send + Sync>,
    shared: Arc<Shared>,
    exit: oneshot::Sender<ExitInfo>,
) {
    let status = child.wait();
    let killed = {
        let mut killer = lock(&shared.killer);
        drop(killer.take());
        shared.exited.store(true, Ordering::SeqCst);
        shared.killed.load(Ordering::SeqCst)
    };
    let code = match status {
        Ok(status) if !killed && status.signal().is_none() => Some(status.exit_code()),
        _ => None,
    };
    tracing::debug!(killed, has_code = code.is_some(), "local shell exited");
    let _ = exit.send(ExitInfo { code });
    // Windows: dropping the master closes the pseudo console; a write that
    // is still blocked then fails, which releases the writer lock.
    drop(lock(&shared.master).take());
    drop(lock(&shared.writer).take());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_values() {
        assert_eq!(term_value("xterm-256color"), "xterm-256color");
        assert_eq!(term_value("vt100"), "vt100");
        assert_eq!(term_value(""), FALLBACK_TERM);
        assert_eq!(term_value("xterm 256"), FALLBACK_TERM);
        assert_eq!(term_value("bad\0term"), FALLBACK_TERM);
        assert_eq!(term_value("line\nbreak"), FALLBACK_TERM);
    }

    #[test]
    fn sizes_are_at_least_one() {
        let size = pty_size(0, 0);
        assert_eq!((size.cols, size.rows), (1, 1));
        let size = pty_size(120, 40);
        assert_eq!((size.cols, size.rows), (120, 40));
    }
}
