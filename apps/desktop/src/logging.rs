//! Diagnostic log (docs/architecture.md, section 3.8): `shellpp.log` in the
//! logs directory, rotated at 5 MiB with 5 files kept (`shellpp.log`,
//! `shellpp.log.1` ... `shellpp.log.4`). Level `info`, `debug` in debug
//! builds, for the project's crates (`warn` for third-party crates); debug
//! builds also write to stderr.
//!
//! Log lines carry event names, ids, states, counts and error codes only:
//! never secrets, host addresses, usernames, command text or paths
//! (docs/security.md, section 9.1). Call sites use explicit structured fields.
//!
//! [`close_file`] closes the log file for good (later lines go to stderr in
//! debug builds only); `--demo` calls it before deleting its temporary
//! profile, because Windows cannot delete an open file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use shellpp_core::ids;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Rotation threshold of one log file.
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Number of files kept, including the current one.
pub const KEPT_FILES: usize = 5;

/// A size-rotated log file. Rotation happens before a write that would push
/// the current file over the limit.
#[derive(Debug)]
pub struct RotatingFile {
    inner: Mutex<RotatingInner>,
}

#[derive(Debug)]
struct RotatingInner {
    path: PathBuf,
    max_bytes: u64,
    kept: usize,
    file: Option<File>,
    size: u64,
    /// Set by [`RotatingFile::close`]: writes are dropped from then on.
    closed: bool,
}

impl RotatingFile {
    /// Opens (appends to) `dir/<file_name>`.
    pub fn open(dir: &Path, file_name: &str, max_bytes: u64, kept: usize) -> io::Result<Self> {
        let path = dir.join(file_name);
        let file = open_append(&path)?;
        let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            inner: Mutex::new(RotatingInner {
                path,
                max_bytes: max_bytes.max(1),
                kept: kept.max(1),
                file: Some(file),
                size,
                closed: false,
            }),
        })
    }

    /// Closes the file; later writes are dropped (and do not reopen it).
    pub fn close(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(file) = inner.file.as_mut() {
            let _ = file.flush();
        }
        inner.file = None;
        inner.closed = true;
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn numbered(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

impl RotatingInner {
    fn rotate(&mut self) -> io::Result<()> {
        // Close the current file first (Windows cannot rename an open file).
        self.file = None;
        if self.kept > 1 {
            let _ = fs::remove_file(numbered(&self.path, self.kept - 1));
            for n in (1..self.kept - 1).rev() {
                let from = numbered(&self.path, n);
                if from.exists() {
                    let _ = fs::rename(&from, numbered(&self.path, n + 1));
                }
            }
            let _ = fs::rename(&self.path, numbered(&self.path, 1));
        } else {
            let _ = fs::remove_file(&self.path);
        }
        self.file = Some(open_append(&self.path)?);
        self.size = 0;
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.file = Some(open_append(&self.path)?);
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(buf)?;
            self.size += buf.len() as u64;
        }
        Ok(())
    }
}

/// One `MakeWriter` handle; every formatted event is written in one call.
#[derive(Clone, Debug)]
pub struct RotatingWriter(Arc<RotatingFile>);

impl RotatingWriter {
    pub fn new(file: RotatingFile) -> Self {
        Self(Arc::new(file))
    }

    /// The shared file behind this writer.
    pub fn file(&self) -> Arc<RotatingFile> {
        self.0.clone()
    }
}

/// The log file of the global subscriber, for [`close_file`].
static GLOBAL_FILE: OnceLock<Arc<RotatingFile>> = OnceLock::new();

/// Closes the log file of the global subscriber, if there is one.
pub fn close_file() {
    if let Some(file) = GLOBAL_FILE.get() {
        file.close();
    }
}

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self
            .0
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut inner = self
            .0
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match inner.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> MakeWriter<'a> for RotatingWriter {
    type Writer = RotatingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The default level filter of this build.
/// Our crates log at `info` (`debug` in debug builds); third-party crates
/// (GPUI, gpui-kit, russh, ...) only at `warn`, so their messages, which we
/// do not control, stay out of the file unless something goes wrong.
fn default_filter() -> EnvFilter {
    EnvFilter::new(if cfg!(debug_assertions) {
        "warn,shellpp_desktop=debug,shellpp=debug,shellpp_core=debug,shellpp_ssh=debug,shellpp_term_local=debug"
    } else {
        "warn,shellpp_desktop=info,shellpp=info,shellpp_core=info,shellpp_ssh=info,shellpp_term_local=info"
    })
}

/// Installs the global subscriber. A failure to open the log file keeps the
/// app running without a file log (debug builds still log to stderr).
pub fn init(logs_dir: &Path) {
    let file = RotatingFile::open(logs_dir, ids::LOG_FILE_NAME, MAX_FILE_BYTES, KEPT_FILES).ok();
    let file_layer = file.map(|file| {
        let writer = RotatingWriter::new(file);
        let _ = GLOBAL_FILE.set(writer.file());
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(true)
            .with_writer(writer)
            .with_filter(default_filter())
    });
    let stderr_layer = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(io::stderr)
            .with_filter(default_filter())
    });
    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init();
}

/// Installs the global subscriber without a file (the logs directory could
/// not be created); debug builds still log to stderr.
pub fn init_without_file() {
    let stderr_layer = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(io::stderr)
            .with_filter(default_filter())
    });
    let _ = tracing_subscriber::registry().with(stderr_layer).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    fn tempdir() -> crate::profile::TempProfile {
        crate::profile::TempProfile::create("shellpp-test-log").unwrap()
    }

    #[test]
    fn rotates_at_the_limit_and_keeps_n_files() {
        let temp = tempdir();
        let file = RotatingFile::open(temp.path(), "t.log", 10, 3).unwrap();
        let mut writer = RotatingWriter::new(file);
        for line in ["aaaaaa\n", "bbbbbb\n", "cccccc\n", "dddddd\n", "eeeeee\n"] {
            writer.write_all(line.as_bytes()).unwrap();
        }
        writer.flush().unwrap();
        let base = temp.path().join("t.log");
        assert_eq!(read(&base), "eeeeee\n");
        assert_eq!(read(&numbered(&base, 1)), "dddddd\n");
        assert_eq!(read(&numbered(&base, 2)), "cccccc\n");
        assert!(!numbered(&base, 3).exists());
        let count = fs::read_dir(temp.path()).unwrap().count();
        assert_eq!(count, 3);
    }

    #[test]
    fn appends_to_an_existing_file_and_counts_its_size() {
        let temp = tempdir();
        let base = temp.path().join("t.log");
        fs::write(&base, b"12345678").unwrap();
        let file = RotatingFile::open(temp.path(), "t.log", 10, 2).unwrap();
        let mut writer = RotatingWriter::new(file);
        writer.write_all(b"ab").unwrap();
        assert_eq!(read(&base), "12345678ab");
        writer.write_all(b"c").unwrap();
        assert_eq!(read(&base), "c");
        assert_eq!(read(&numbered(&base, 1)), "12345678ab");
    }

    #[test]
    fn a_single_oversized_write_goes_to_a_fresh_file() {
        let temp = tempdir();
        let file = RotatingFile::open(temp.path(), "t.log", 4, 2).unwrap();
        let mut writer = RotatingWriter::new(file);
        writer.write_all(b"0123456789").unwrap();
        writer.write_all(b"x").unwrap();
        let base = temp.path().join("t.log");
        assert_eq!(read(&base), "x");
        assert_eq!(read(&numbered(&base, 1)), "0123456789");
    }

    #[test]
    fn a_closed_file_drops_writes_and_can_be_deleted() {
        let temp = tempdir();
        let file = RotatingFile::open(temp.path(), "t.log", 100, 2).unwrap();
        let mut writer = RotatingWriter::new(file);
        writer
            .write_all(
                b"before
",
            )
            .unwrap();
        writer.file().close();
        writer
            .write_all(
                b"after
",
            )
            .unwrap();
        let base = temp.path().join("t.log");
        assert_eq!(
            read(&base),
            "before
"
        );
        fs::remove_file(&base).unwrap();
        assert!(!base.exists());
    }

    #[test]
    fn production_limits() {
        assert_eq!(MAX_FILE_BYTES, 5 * 1024 * 1024);
        assert_eq!(KEPT_FILES, 5);
    }
}
