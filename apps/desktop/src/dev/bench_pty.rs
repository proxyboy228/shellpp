//! The baseline runs of `--bench-output` (`bare-direct`, `bare-shell`, see
//! [`super::bench`]): the same PTY layer as the app's local terminals
//! (`shellpp-term-local`: ConPTY on Windows, a pty elsewhere; the same
//! reader thread and output channel), read by a bare reader that only counts
//! the bytes and looks for the end of the output. No terminal emulator, no
//! window, no frames: what is left is the cost of the program, the shell and
//! the PTY (on Windows, conhost turning the console output into VT
//! sequences). The difference to `local-direct` and `local-shell` is the
//! cost of our terminal.
//!
//! - Direct: `seq 1 <N>` is the PTY program; timed from the start of the PTY
//!   to the bytes `<N>\r\n` (the last number; the pseudo console writes the
//!   lines as they are).
//! - Shell: the default shell (the one a new local terminal starts: Windows
//!   PowerShell on Windows); once it waits at its prompt, the command of
//!   [`LocalSeq::command`] is typed, and timed from Enter to the bytes of
//!   [`SHELL_DONE`] that it prints after the numbers.
//!
//! The pseudo console asks for the cursor position (`ESC [ 6 n`) when it
//! starts and waits for the answer; the reader answers every request with
//! row 1, column 1, as a terminal would. Nothing here prints or logs the
//! output.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use shellpp_term_local::{LocalSession, PtyConfig, PtyWriter, ShellSpec};

use super::bench::{LocalSeq, SHELL_DONE, StreamMatch};

/// TERM of the bare runs (the app's default, `terminal.term`).
const TERM: &str = "xterm-256color";
/// The cursor position request of the pseudo console and the answer.
const CURSOR_REQUEST: &[u8] = b"\x1b[6n";
const CURSOR_REPORT: &[u8] = b"\x1b[1;1R";
/// No output for this long: the shell waits at its prompt (or the echo of
/// the typed command is complete).
const QUIET: Duration = Duration::from_millis(800);
/// The shell's start-up and its prompt.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// One bare run.
#[derive(Clone, Copy, Debug)]
pub struct BareRun {
    pub elapsed: Duration,
    /// Bytes read from the PTY in the timed part.
    pub bytes: u64,
}

/// The size of the pseudo console (the benchmark window's terminal grid).
#[derive(Clone, Copy, Debug)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

/// The output of a running PTY on a thread of its own, which also answers
/// the cursor position requests. Dropping it ends the process.
struct Reader {
    chunks: mpsc::Receiver<Vec<u8>>,
    writer: PtyWriter,
}

impl Reader {
    fn start(session: LocalSession) -> Result<Self, String> {
        let writer = session.writer();
        let answer = session.writer();
        let LocalSession { mut output, .. } = session;
        let (sender, chunks) = mpsc::channel();
        std::thread::Builder::new()
            .name("bench-bare-reader".to_owned())
            .spawn(move || {
                let mut requests = Requests::default();
                while let Some(chunk) = output.blocking_recv() {
                    for _ in 0..requests.count(&chunk) {
                        let _ = answer.write(CURSOR_REPORT);
                    }
                    if sender.send(chunk).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("reader thread: {error}"))?;
        Ok(Self { chunks, writer })
    }

    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        self.writer
            .write(bytes)
            .map_err(|error| format!("writing to the PTY: {error}"))
    }

    /// Reads until no output came for [`QUIET`] (after at least one chunk).
    fn wait_quiet(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut seen = false;
        loop {
            match self.chunks.recv_timeout(QUIET) {
                Ok(_) => seen = true,
                Err(RecvTimeoutError::Timeout) if seen => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("the program ended early".to_owned());
                }
            }
            if Instant::now() > deadline {
                return Err("timeout waiting for the prompt".to_owned());
            }
        }
    }

    /// Reads until `end` is seen; returns the bytes read.
    fn read_until(&self, end: &mut StreamMatch, deadline: Instant) -> Result<u64, String> {
        let mut bytes = 0u64;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.chunks.recv_timeout(left) {
                Ok(chunk) => {
                    bytes += chunk.len() as u64;
                    if end.feed(&chunk) {
                        return Ok(bytes);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err("timeout waiting for the end of the output".to_owned());
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("the output ended before its last line".to_owned());
                }
            }
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // The reader thread keeps a writer: end the process explicitly, so
        // the PTY closes and the thread finishes.
        self.writer.kill();
    }
}

/// Counts cursor position requests in a stream read in chunks (a request
/// may cross two chunks).
#[derive(Default)]
struct Requests {
    tail: Vec<u8>,
}

impl Requests {
    fn count(&mut self, chunk: &[u8]) -> usize {
        let keep = CURSOR_REQUEST.len() - 1;
        let mut joined = std::mem::take(&mut self.tail);
        joined.extend_from_slice(chunk);
        let count = joined
            .windows(CURSOR_REQUEST.len())
            .filter(|window| *window == CURSOR_REQUEST)
            .count();
        // A request is longer than the tail, so none is counted twice.
        let from = joined.len().saturating_sub(keep);
        self.tail = joined.split_off(from);
        count
    }
}

fn spawn(spec: ShellSpec, size: GridSize) -> Result<Reader, String> {
    let session = shellpp_term_local::spawn(PtyConfig {
        shell: spec,
        cols: size.cols,
        rows: size.rows,
        term: TERM.to_owned(),
        cwd: None,
    })
    .map_err(|error| format!("the PTY did not start ({error})"))?;
    Reader::start(session)
}

/// `bare-direct`: `program 1 <lines>` as the PTY program.
pub fn run_direct(
    program: &Path,
    lines: u64,
    size: GridSize,
    timeout: Duration,
) -> Result<BareRun, String> {
    let spec = ShellSpec {
        id: None,
        label: "seq".to_owned(),
        program: program.to_path_buf(),
        args: vec!["1".to_owned(), lines.to_string()],
    };
    let started = Instant::now();
    let reader = spawn(spec, size)?;
    let mut end = StreamMatch::new(&format!("{lines}\r\n"));
    let bytes = reader.read_until(&mut end, started + timeout)?;
    Ok(BareRun {
        elapsed: started.elapsed(),
        bytes,
    })
}

/// `bare-shell`: the default shell; `seq`'s command typed at its prompt.
pub fn run_shell(
    seq: &LocalSeq,
    lines: u64,
    size: GridSize,
    timeout: Duration,
) -> Result<BareRun, String> {
    let shells = shellpp_term_local::detect_shells();
    let spec = shellpp_term_local::resolve_shell(None, None, &shells)
        .map_err(|error| format!("no default shell ({error})"))?;
    let reader = spawn(spec, size)?;
    reader.wait_quiet(START_TIMEOUT)?;
    // The command first; Enter once its echo is complete.
    reader.write(seq.command(lines).as_bytes())?;
    reader.wait_quiet(START_TIMEOUT)?;
    let mut end = StreamMatch::new(SHELL_DONE);
    let started = Instant::now();
    reader.write(b"\r")?;
    let bytes = reader.read_until(&mut end, started + timeout)?;
    let run = BareRun {
        elapsed: started.elapsed(),
        bytes,
    };
    // A normal end of the shell (Windows PowerShell saves its history then:
    // the scripts give it a temporary APPDATA).
    let _ = reader.write(b"exit\r");
    let _ = reader.wait_quiet(Duration::from_secs(2));
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_cursor_requests_across_chunks() {
        let mut requests = Requests::default();
        assert_eq!(requests.count(b"\x1b[6n\x1b[m"), 1);
        assert_eq!(requests.count(b"text\x1b["), 0);
        assert_eq!(requests.count(b"6nmore\x1b[6n"), 2);
        assert_eq!(requests.count(b"\x1b"), 0);
        assert_eq!(requests.count(b"["), 0);
        assert_eq!(requests.count(b"6"), 0);
        assert_eq!(requests.count(b"n"), 1);
        assert_eq!(requests.count(b"\x1b[6n"), 1, "not counted twice");
    }

    /// The direct baseline end to end with a short run, where the program
    /// exists (Git's seq.exe on Windows, `seq` elsewhere).
    #[test]
    fn direct_baseline_reads_to_the_last_number() {
        let seq = super::super::bench::local_seq();
        let Some(program) = seq.direct_program() else {
            return;
        };
        let run = run_direct(
            program,
            2_000,
            GridSize {
                cols: 120,
                rows: 40,
            },
            Duration::from_secs(60),
        )
        .expect("the bare run ends");
        assert!(run.bytes >= "2000\r\n".len() as u64, "{} bytes", run.bytes);
    }
}
