//! Terminal benchmarks (feature `dev-tools`; owner's stage 2a, step 4),
//! driven by `scripts/measure-terminal.ps1`. Each mode runs the real app on
//! a demo profile (300 hosts, temporary, in-memory secret store) against a
//! test server ([`super::test_server`]; never a real host):
//!
//! - `--bench-sessions <N>`: opens N SSH session tabs in the app's main
//!   window, waits until every one is connected and at its prompt and prints
//!   `bench: ready`; the app then stays idle until a `quit` line arrives on
//!   stdin or its window is closed (the script reads the RSS meanwhile).
//! - `--bench-output`: times `seq 1 <N>` (default [`SEQ_LINES`]) five ways,
//!   one `bench: output <kind> ...` line each ([`OutputKind`]), so that the
//!   cost of our terminal can be told apart from the PTY and the shell:
//!   1. `ssh`: typed into an SSH session; from Enter to the first painted
//!      frame that shows the last number and the prompt after it;
//!   2. `local-direct`: `seq` itself as the program of a local terminal (no
//!      shell; Windows: Git's `seq.exe`, skipped without it); from the start
//!      of the PTY to the first frame that shows the last number;
//!   3. `local-shell`: typed into the default local shell (Windows
//!      PowerShell on Windows: Git's `seq.exe` when present, else a
//!      PowerShell loop; [`LocalSeq`]) followed by an end marker
//!      ([`SHELL_DONE`]); from Enter to the first frame that shows the last
//!      number and the marker;
//!   4. `bare-direct` and 5. `bare-shell`: the same two through the same PTY
//!      layer (`shellpp-term-local`: ConPTY on Windows) with a bare reader
//!      that only counts the bytes and looks for the end, no terminal
//!      emulator and no window: the baseline of the PTY (and the shell).
//! - `--bench-latency`: 200 keystrokes each into a local terminal and an SSH
//!   session; for each, the time from the key event to the first presented
//!   frame that shows the echoed glyph; prints p50/p95 and quits.
//! - `--bench-splits` (owner's stage 2b, step 4): [`SPLIT_TABS`] session tabs
//!   in the main window, each split into [`SPLIT_PANES`] panes (2 x 2: the
//!   tab's pane, then a pane to its right, one below it and one below the
//!   right one; [`crate::panes::PaneGroup::split`] and `open_in`, what
//!   Ctrl+Shift+D / Ctrl+Shift+E and Enter in the new pane's host list do),
//!   every pane an SSH session to the test server: 16 sessions. Then
//!   `bench: ready` and the same idle as `--bench-sessions`.
//! - `--bench-flood` (stage 2b, step 4): one SSH session tab in the main
//!   window runs [`FloodShell::command`] (`yes | head -n 5000000` on
//!   OpenSSH; `seq 1 5000000` on the russh test server, whose shell has no
//!   `yes`) again and again for `--flood-seconds` (default
//!   [`FLOOD_SECONDS`], 10 minutes; a run that started goes on to its end).
//!   It prints `bench: flood sample <start|run|end> <seconds>` at the start,
//!   every `--sample-seconds` (default [`FLOOD_SAMPLE_SECONDS`]) and after
//!   the last run: the script reads the process memory at each of these
//!   lines (the app cannot read its own working set without `unsafe` on
//!   Windows). Then the number of runs, the terminal's scrollback line
//!   count against its cap ([`crate::terminal::DEFAULT_SCROLLBACK`]; more
//!   than the cap fails the run), `bench: ready` and the idle of
//!   `--bench-sessions` (the script reads the memory once more).
//!
//! Common options: `--test-server <server.json>` (OpenSSH from
//! `tests/openssh/start.sh`; default: the russh test server as a child
//! process), `--theme auto|light|dark`. Short smoke runs: `--seq-lines <N>`
//! for `--bench-output`, `--keys <N>` for `--bench-latency`,
//! `--flood-seconds <N>` and `--sample-seconds <N>` for `--bench-flood`.
//!
//! Output lines start with `bench: `; failures print `bench: failed <why>`
//! and exit with code 1. Nothing secret, no address and no terminal data is
//! printed or logged.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::theme::Appearance;

/// `seq 1 <SEQ_LINES>` of the output benchmark.
pub const SEQ_LINES: u64 = 3_000_000;
/// Keystrokes per terminal of the latency benchmark.
pub const LATENCY_KEYS: usize = 200;
/// The most sessions `--bench-sessions` opens.
pub const MAX_SESSIONS: usize = 50;
/// The most numbers `--seq-lines` takes.
pub const MAX_SEQ_LINES: u64 = 10_000_000;
/// The most keystrokes `--keys` takes.
pub const MAX_KEYS: usize = 2_000;
/// What the local shell prints after `seq` in the `local-shell` and
/// `bare-shell` runs: the end of the output in a byte stream, where no
/// prompt can be told apart. The typed command builds it from two parts,
/// so its echo never contains it.
pub const SHELL_DONE: &str = "seq-bench-done";
/// Session tabs of `--bench-splits`.
pub const SPLIT_TABS: usize = 4;
/// Panes per tab of `--bench-splits` (2 x 2).
pub const SPLIT_PANES: usize = 4;
/// Default length of `--bench-flood` (owner's step 4: 10 minutes).
pub const FLOOD_SECONDS: u64 = 600;
/// The longest `--flood-seconds` (2 hours).
pub const MAX_FLOOD_SECONDS: u64 = 7_200;
/// Default interval of the memory samples of `--bench-flood`.
pub const FLOOD_SAMPLE_SECONDS: u64 = 30;
/// Lines of one flood run (`yes | head -n 5000000`).
pub const FLOOD_LINES: u64 = 5_000_000;

/// What to measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BenchMode {
    Sessions(usize),
    /// [`SPLIT_TABS`] tabs x [`SPLIT_PANES`] panes.
    Splits,
    /// `seconds` of flood output, a memory sample every `sample_seconds`.
    Flood {
        seconds: u64,
        sample_seconds: u64,
    },
    Output {
        lines: u64,
    },
    Latency {
        keys: usize,
    },
    /// `--soak <minutes>`: the app under load (`super::soak`).
    Soak {
        minutes: u64,
    },
}

/// The command line of a benchmark run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchArgs {
    pub mode: BenchMode,
    pub server_json: Option<PathBuf>,
    pub appearance: Appearance,
}

/// The flags that select a benchmark.
const MODE_FLAGS: [&str; 6] = [
    "--bench-sessions",
    "--bench-splits",
    "--bench-flood",
    "--bench-output",
    "--bench-latency",
    "--soak",
];

/// The longest soak run, in minutes.
const MAX_SOAK_MINUTES: u64 = 600;

/// The usage of the benchmark modes (`shellpp --help` lists them too).
pub const USAGE: &str = "Benchmarks (dev-tools builds; scripts/measure-terminal.ps1):
  shellpp --bench-sessions <N> [--test-server <server.json>] [--theme <mode>]
  shellpp --bench-splits [--test-server <server.json>] [--theme <mode>]
  shellpp --bench-flood [--flood-seconds <N>] [--sample-seconds <N>] [--test-server <server.json>] [--theme <mode>]
  shellpp --bench-output [--seq-lines <N>] [--test-server <server.json>] [--theme <mode>]
  shellpp --bench-latency [--keys <N>] [--test-server <server.json>] [--theme <mode>]
Without --test-server the russh test server runs as a child process
(shellpp-test-sshd next to this binary, or SHELLPP_TEST_SSHD_EXE).
--seq-lines (default 3000000), --keys (default 200) and --flood-seconds
(default 600, samples every --sample-seconds, default 30) shorten smoke runs.
  shellpp --soak <minutes> --test-server <server.json> [--theme <mode>]";

/// The entry of `main`: `None` for a launch without a benchmark flag,
/// otherwise runs the benchmark (or refuses the arguments with exit code 2).
#[allow(clippy::print_stderr)]
pub fn main(args: &[String]) -> Option<std::process::ExitCode> {
    match parse(args)? {
        Ok(args) => Some(super::bench_run::run(args)),
        Err(error) => {
            eprintln!("shellpp: {error}\n{USAGE}");
            Some(std::process::ExitCode::from(2))
        }
    }
}

/// `None` when no benchmark flag is present (a normal launch); otherwise the
/// parsed arguments or why they were refused. `--demo` is implied and
/// accepted.
pub fn parse(args: &[String]) -> Option<Result<BenchArgs, String>> {
    if !args.iter().any(|arg| MODE_FLAGS.contains(&arg.as_str())) {
        return None;
    }
    Some(parse_bench(args))
}

fn parse_bench(args: &[String]) -> Result<BenchArgs, String> {
    let mut mode = None;
    let mut server_json = None;
    let mut appearance = Appearance::Dark;
    let mut seq_lines = None;
    let mut keys = None;
    let mut flood_seconds = None;
    let mut sample_seconds = None;
    let mut iter = args.iter();
    let set = |mode: &mut Option<BenchMode>, value: BenchMode| {
        if mode.replace(value).is_some() {
            Err(format!("give one of {}", MODE_FLAGS.join(", ")))
        } else {
            Ok(())
        }
    };
    let number = |flag: &str, value: Option<&String>, range: std::ops::RangeInclusive<u64>| {
        value
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| range.contains(value))
            .ok_or_else(|| format!("{flag} takes {} to {}", range.start(), range.end()))
    };
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--demo" => {}
            "--bench-sessions" => {
                let value = iter.next().ok_or("--bench-sessions needs a count")?;
                let count: usize = value
                    .parse()
                    .ok()
                    .filter(|count| (1..=MAX_SESSIONS).contains(count))
                    .ok_or_else(|| format!("--bench-sessions takes 1 to {MAX_SESSIONS}"))?;
                set(&mut mode, BenchMode::Sessions(count))?;
            }
            "--bench-splits" => set(&mut mode, BenchMode::Splits)?,
            "--soak" => {
                let minutes = number(arg, iter.next(), 1..=MAX_SOAK_MINUTES)?;
                set(&mut mode, BenchMode::Soak { minutes })?;
            }
            "--bench-flood" => set(
                &mut mode,
                BenchMode::Flood {
                    seconds: FLOOD_SECONDS,
                    sample_seconds: FLOOD_SAMPLE_SECONDS,
                },
            )?,
            "--flood-seconds" => {
                flood_seconds = Some(number(arg, iter.next(), 1..=MAX_FLOOD_SECONDS)?);
            }
            "--sample-seconds" => {
                sample_seconds = Some(number(arg, iter.next(), 1..=MAX_FLOOD_SECONDS)?);
            }
            "--bench-output" => set(&mut mode, BenchMode::Output { lines: SEQ_LINES })?,
            "--bench-latency" => set(&mut mode, BenchMode::Latency { keys: LATENCY_KEYS })?,
            "--test-server" => {
                let value = iter
                    .next()
                    .ok_or("--test-server needs a server.json path")?;
                server_json = Some(PathBuf::from(value));
            }
            "--theme" => {
                let value = iter.next().ok_or("--theme needs a value")?;
                appearance = Appearance::parse(value)
                    .ok_or_else(|| format!("--theme takes auto, light or dark, not {value}"))?;
            }
            "--seq-lines" => {
                let value = iter.next().ok_or("--seq-lines needs a count")?;
                let lines: u64 = value
                    .parse()
                    .ok()
                    .filter(|lines| (1..=MAX_SEQ_LINES).contains(lines))
                    .ok_or_else(|| format!("--seq-lines takes 1 to {MAX_SEQ_LINES}"))?;
                seq_lines = Some(lines);
            }
            "--keys" => {
                let value = iter.next().ok_or("--keys needs a count")?;
                let count: usize = value
                    .parse()
                    .ok()
                    .filter(|count| (1..=MAX_KEYS).contains(count))
                    .ok_or_else(|| format!("--keys takes 1 to {MAX_KEYS}"))?;
                keys = Some(count);
            }
            other => return Err(format!("unknown option {other} for a benchmark run")),
        }
    }
    let mut mode = mode.ok_or("no benchmark selected")?;
    match &mut mode {
        BenchMode::Output { lines } => {
            if let Some(value) = seq_lines.take() {
                *lines = value;
            }
        }
        BenchMode::Latency { keys: count } => {
            if let Some(value) = keys.take() {
                *count = value;
            }
        }
        BenchMode::Flood {
            seconds,
            sample_seconds: every,
        } => {
            if let Some(value) = flood_seconds.take() {
                *seconds = value;
            }
            if let Some(value) = sample_seconds.take() {
                *every = value;
            }
        }
        BenchMode::Sessions(_) | BenchMode::Splits | BenchMode::Soak { .. } => {}
    }
    if seq_lines.is_some() {
        return Err("--seq-lines goes with --bench-output".into());
    }
    if keys.is_some() {
        return Err("--keys goes with --bench-latency".into());
    }
    if flood_seconds.is_some() || sample_seconds.is_some() {
        return Err("--flood-seconds and --sample-seconds go with --bench-flood".into());
    }
    Ok(BenchArgs {
        mode,
        server_json,
        appearance,
    })
}

/// The nearest-rank percentile `p` (0..=100) of `samples` (sorted here).
pub fn percentile(samples: &mut [Duration], p: f64) -> Option<Duration> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    let rank = ((p / 100.0) * samples.len() as f64).ceil() as usize;
    Some(samples[rank.clamp(1, samples.len()) - 1])
}

/// Milliseconds with two decimals, culture-free (`12.34`).
pub fn millis(duration: Duration) -> String {
    format!("{:.2}", duration.as_secs_f64() * 1000.0)
}

/// The five runs of `--bench-output`, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    /// Typed into an SSH session (our terminal).
    Ssh,
    /// `seq` as the program of a local terminal, no shell (our terminal).
    LocalDirect,
    /// Typed into the default local shell (our terminal).
    LocalShell,
    /// `seq` as the program of a bare PTY reader (no terminal).
    BareDirect,
    /// Typed into the default shell in a bare PTY reader (no terminal).
    BareShell,
}

impl OutputKind {
    /// The token of the output line (`bench: output <kind> ...`).
    pub fn token(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::LocalDirect => "local-direct",
            Self::LocalShell => "local-shell",
            Self::BareDirect => "bare-direct",
            Self::BareShell => "bare-shell",
        }
    }

    /// Whether the run goes through our terminal (window, emulator, frames).
    pub fn through_terminal(self) -> bool {
        matches!(self, Self::Ssh | Self::LocalDirect | Self::LocalShell)
    }
}

/// How the local runs print the numbers 1..=N.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalSeq {
    /// Git for Windows' `seq.exe` (coreutils): the direct runs start it as
    /// the PTY program, the shell runs type it into Windows PowerShell.
    GitSeq(PathBuf),
    /// No `seq.exe` (Windows without Git): PowerShell builds the text in a
    /// loop over `1..N` and writes it at once; there are no direct runs.
    PowerShellLoop,
    /// macOS and Linux: `seq` (its path on the PATH for the direct runs;
    /// `None` when it is not found there, then only the shell's `seq`).
    Seq(Option<PathBuf>),
}

impl LocalSeq {
    /// The program of the direct runs, if there is one.
    pub fn direct_program(&self) -> Option<&Path> {
        match self {
            Self::GitSeq(path) | Self::Seq(Some(path)) => Some(path),
            Self::PowerShellLoop | Self::Seq(None) => None,
        }
    }

    /// What the direct runs start, for their output lines.
    pub fn direct_label(&self) -> &'static str {
        match self {
            Self::GitSeq(_) => "Git seq.exe as the PTY program, no shell",
            Self::Seq(_) => "seq as the PTY program, no shell",
            Self::PowerShellLoop => "no seq program",
        }
    }

    /// What the shell runs type, for their output lines.
    pub fn shell_label(&self) -> &'static str {
        match self {
            Self::GitSeq(_) => "Git seq.exe typed into Windows PowerShell",
            Self::PowerShellLoop => "PowerShell loop typed into Windows PowerShell",
            Self::Seq(_) => "seq typed into the user's shell",
        }
    }

    /// The command line for the local shell (without the line ending): the
    /// numbers, then [`SHELL_DONE`] on a line of its own.
    pub fn command(&self, lines: u64) -> String {
        match self {
            Self::GitSeq(path) => {
                format!("& '{}' 1 {lines}; 'seq-bench-' + 'done'", path.display())
            }
            Self::PowerShellLoop => format!(
                "$b = [System.Text.StringBuilder]::new(); foreach ($i in 1..{lines}) {{ [void]$b.AppendLine($i) }}; [Console]::Out.Write($b.ToString()); 'seq-bench-' + 'done'"
            ),
            Self::Seq(_) => format!("seq 1 {lines}; printf 'seq-bench-%s\\n' done"),
        }
    }
}

/// The local `seq` of this machine: Git's `seq.exe` (found next to `git`)
/// or a PowerShell loop on Windows, `seq` elsewhere. Nothing is downloaded.
pub fn local_seq() -> LocalSeq {
    if cfg!(windows) {
        git_seq().map_or(LocalSeq::PowerShellLoop, LocalSeq::GitSeq)
    } else {
        LocalSeq::Seq(path_seq())
    }
}

fn git_seq() -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("--exec-path")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    // <git>/mingw64/libexec/git-core -> <git>/usr/bin/seq.exe
    let exec_path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let seq = exec_path
        .ancestors()
        .nth(3)?
        .join("usr")
        .join("bin")
        .join("seq.exe");
    seq.is_file().then_some(seq)
}

/// `seq` on the PATH (absolute: the PTY layer starts absolute paths only).
fn path_seq() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("seq"))
        .find(|candidate| candidate.is_file())
}

/// Whether the screen rows show the end of `seq 1 <last>`: a row that is
/// exactly the last number, followed by a non-empty row (the prompt).
pub fn seq_finished(rows: &[String], last: u64) -> bool {
    let last = last.to_string();
    rows.iter()
        .position(|row| row.trim_end() == last)
        .is_some_and(|index| rows[index + 1..].iter().any(|row| !row.trim().is_empty()))
}

/// The end of a direct run: a row that is exactly the last number (`seq`
/// is the program; no prompt follows).
pub fn seq_direct_finished(rows: &[String], last: u64) -> bool {
    let last = last.to_string();
    rows.iter().any(|row| row.trim_end() == last)
}

/// The end of a shell run: a row that is exactly the last number, followed
/// by a row that is exactly [`SHELL_DONE`].
pub fn seq_shell_finished(rows: &[String], last: u64) -> bool {
    let last = last.to_string();
    rows.iter()
        .position(|row| row.trim_end() == last)
        .is_some_and(|index| {
            rows[index + 1..]
                .iter()
                .any(|row| row.trim_end() == SHELL_DONE)
        })
}

/// The shell of `--bench-flood`'s session, by test server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloodShell {
    /// bash on OpenSSH: `yes | head -n 5000000`, then the run's marker
    /// line ([`flood_marker`]) from the same command line.
    Bash,
    /// The russh test server's own shell: one command per line and no
    /// `yes`: `seq 1 5000000`, then `echo` of the marker as a command of its
    /// own ([`FloodShell::mark_command`]).
    Russh,
}

impl FloodShell {
    /// The command line of run `run` (1, 2, ...), without the line ending.
    /// Its echo never shows the marker as a row of its own.
    pub fn command(self, run: u64) -> String {
        match self {
            Self::Bash => {
                format!("yes | head -n {FLOOD_LINES}; printf 'flood-%s-%s\\n' done {run}")
            }
            Self::Russh => format!("seq 1 {FLOOD_LINES}"),
        }
    }

    /// The command that prints run `run`'s marker after its output, when
    /// the run's own command line cannot (russh).
    pub fn mark_command(self, run: u64) -> Option<String> {
        match self {
            Self::Bash => None,
            Self::Russh => Some(format!("echo {}", flood_marker(run))),
        }
    }

    /// Whether the screen shows the end of run `run`'s output. On russh the
    /// previous run's marker must have scrolled away first, so the end of
    /// the previous run (the same last number) is never taken for this one.
    pub fn output_finished(self, rows: &[String], run: u64) -> bool {
        match self {
            Self::Bash => shows_marker(rows, run),
            Self::Russh => {
                (run <= 1 || !shows_marker(rows, run - 1)) && seq_finished(rows, FLOOD_LINES)
            }
        }
    }

    /// What one run prints, for the output lines.
    pub fn label(self) -> String {
        match self {
            Self::Bash => format!("yes | head -n {FLOOD_LINES}"),
            Self::Russh => {
                format!("seq 1 {FLOOD_LINES} (the russh test server's shell has no yes)")
            }
        }
    }
}

/// The line that ends flood run `run`.
pub fn flood_marker(run: u64) -> String {
    format!("flood-done-{run}")
}

/// Whether a row is exactly run `run`'s marker.
pub fn shows_marker(rows: &[String], run: u64) -> bool {
    let marker = flood_marker(run);
    rows.iter().any(|row| row.trim_end() == marker)
}

/// What the scrollback of the flood's terminal says about its cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbackCheck {
    /// Exactly the cap: full and bounded.
    AtCap,
    /// Fewer lines than the cap (too little output to fill it).
    Below,
    /// More lines than the cap: the scrollback is not bounded.
    Over,
}

impl ScrollbackCheck {
    pub fn of(lines: usize, cap: usize) -> Self {
        match lines.cmp(&cap) {
            std::cmp::Ordering::Equal => Self::AtCap,
            std::cmp::Ordering::Less => Self::Below,
            std::cmp::Ordering::Greater => Self::Over,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AtCap => "at the cap",
            Self::Below => "below the cap",
            Self::Over => "over the cap",
        }
    }
}

/// Finds `needle` in a byte stream read in chunks: keeps the tail of the
/// previous chunk, so a match across two chunks is found too.
#[derive(Debug)]
pub struct StreamMatch {
    needle: Vec<u8>,
    tail: Vec<u8>,
    found: bool,
}

impl StreamMatch {
    pub fn new(needle: &str) -> Self {
        Self {
            needle: needle.as_bytes().to_vec(),
            tail: Vec::new(),
            found: false,
        }
    }

    /// Feeds the next chunk; true once the needle was seen.
    pub fn feed(&mut self, chunk: &[u8]) -> bool {
        if self.found || self.needle.is_empty() {
            self.found = true;
            return true;
        }
        let keep = self.needle.len() - 1;
        // The previous tail joined with the start of this chunk covers every
        // match that crosses the boundary.
        let mut joined = std::mem::take(&mut self.tail);
        joined.extend_from_slice(&chunk[..chunk.len().min(keep)]);
        if find(&joined, &self.needle) || find(chunk, &self.needle) {
            self.found = true;
            return true;
        }
        // The last `keep` bytes of everything seen so far.
        let mut seen = joined;
        if chunk.len() > keep {
            seen.clear();
            seen.extend_from_slice(&chunk[chunk.len() - keep..]);
        }
        let from = seen.len().saturating_sub(keep);
        self.tail = seen.split_off(from);
        false
    }

    pub fn found(&self) -> bool {
        self.found
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// The keystrokes of the latency benchmark: lowercase letters, a fresh
/// line (Ctrl+U clears it) every 40 characters so the line never wraps.
pub fn latency_char(index: usize) -> char {
    (b'a' + (index % 26) as u8) as char
}

/// Whether `index` starts a fresh input line.
pub fn latency_line_start(index: usize) -> bool {
    index.is_multiple_of(40)
}

/// Path for messages: the file name only (no user directories in output).
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn parses_the_benchmark_flags() {
        assert_eq!(parse(&args(&["--demo"])), None);
        assert_eq!(parse(&args(&[])), None);
        assert_eq!(
            parse(&args(&[
                "--demo",
                "--bench-sessions",
                "10",
                "--theme",
                "light"
            ])),
            Some(Ok(BenchArgs {
                mode: BenchMode::Sessions(10),
                server_json: None,
                appearance: Appearance::Light,
            }))
        );
        assert_eq!(
            parse(&args(&["--bench-output", "--test-server", "x/server.json"])),
            Some(Ok(BenchArgs {
                mode: BenchMode::Output { lines: SEQ_LINES },
                server_json: Some(PathBuf::from("x/server.json")),
                appearance: Appearance::Dark,
            }))
        );
        assert!(matches!(
            parse(&args(&["--bench-latency"])),
            Some(Ok(BenchArgs {
                mode: BenchMode::Latency { keys: LATENCY_KEYS },
                ..
            }))
        ));
        assert_eq!(
            parse(&args(&["--bench-output", "--seq-lines", "1000"])),
            Some(Ok(BenchArgs {
                mode: BenchMode::Output { lines: 1000 },
                server_json: None,
                appearance: Appearance::Dark,
            }))
        );
        assert!(matches!(
            parse(&args(&["--keys", "20", "--bench-latency"])),
            Some(Ok(BenchArgs {
                mode: BenchMode::Latency { keys: 20 },
                ..
            }))
        ));
        for bad in [
            &["--bench-sessions"][..],
            &["--bench-sessions", "0"],
            &["--bench-sessions", "51"],
            &["--bench-output", "--bench-latency"],
            &["--bench-output", "--test-server"],
            &["--bench-output", "--bogus"],
            &["--bench-output", "--theme", "blue"],
            &["--bench-output", "--seq-lines", "0"],
            &["--bench-output", "--seq-lines", "x"],
            &["--bench-output", "--keys", "10"],
            &["--bench-latency", "--seq-lines", "10"],
            &["--bench-latency", "--keys", "0"],
            &["--bench-sessions", "2", "--keys", "10"],
            &["--bench-splits", "--bench-flood"],
            &["--bench-splits", "--flood-seconds", "10"],
            &["--bench-flood", "--flood-seconds", "0"],
            &["--bench-flood", "--flood-seconds", "7201"],
            &["--bench-flood", "--sample-seconds"],
            &["--bench-flood", "--seq-lines", "10"],
            &["--bench-output", "--sample-seconds", "5"],
        ] {
            assert!(matches!(parse(&args(bad)), Some(Err(_))), "{bad:?}");
        }
    }

    #[test]
    fn parses_the_stage_2b_benchmarks() {
        assert_eq!(
            parse(&args(&["--demo", "--bench-splits"])),
            Some(Ok(BenchArgs {
                mode: BenchMode::Splits,
                server_json: None,
                appearance: Appearance::Dark,
            }))
        );
        assert_eq!(
            parse(&args(&["--bench-flood"])).map(|parsed| parsed.map(|args| args.mode)),
            Some(Ok(BenchMode::Flood {
                seconds: FLOOD_SECONDS,
                sample_seconds: FLOOD_SAMPLE_SECONDS,
            }))
        );
        assert_eq!(
            parse(&args(&[
                "--sample-seconds",
                "5",
                "--bench-flood",
                "--flood-seconds",
                "20",
                "--test-server",
                "x/server.json",
            ])),
            Some(Ok(BenchArgs {
                mode: BenchMode::Flood {
                    seconds: 20,
                    sample_seconds: 5,
                },
                server_json: Some(PathBuf::from("x/server.json")),
                appearance: Appearance::Dark,
            }))
        );
        assert_eq!(FLOOD_SECONDS, 600, "owner's step 4: 10 minutes");
        assert_eq!(SPLIT_TABS * SPLIT_PANES, 16);
    }

    #[test]
    fn flood_runs_end_at_their_own_marker() {
        let rows =
            |list: &[&str]| -> Vec<String> { list.iter().map(|r| (*r).to_owned()).collect() };
        let bash = FloodShell::Bash;
        assert_eq!(
            bash.command(3),
            "yes | head -n 5000000; printf 'flood-%s-%s\\n' done 3"
        );
        assert_eq!(bash.mark_command(3), None);
        // The echo of the command line is not the marker.
        let echo = format!("pwuser@shellpp-openssh:~$ {}", bash.command(3));
        assert!(!bash.output_finished(&rows(&[&echo, "y", "y"]), 3));
        assert!(!bash.output_finished(&rows(&["y", "flood-done-2", "$"]), 3));
        assert!(bash.output_finished(&rows(&["y", "y", "flood-done-3", "$"]), 3));

        let russh = FloodShell::Russh;
        assert_eq!(russh.command(1), "seq 1 5000000");
        assert_eq!(russh.mark_command(2).as_deref(), Some("echo flood-done-2"));
        let done = rows(&["4999999", "5000000", "pwuser@shellpp-test:~$"]);
        assert!(russh.output_finished(&done, 1));
        // The previous run's end with its marker below: not this run yet.
        let previous = rows(&[
            "5000000",
            "pwuser@shellpp-test:~$ echo flood-done-1",
            "flood-done-1",
            "pwuser@shellpp-test:~$ seq 1 5000000",
        ]);
        assert!(!russh.output_finished(&previous, 2));
        assert!(russh.output_finished(&done, 2));
        assert!(!russh.output_finished(&rows(&["4999999", "5000000"]), 2));
        for shell in [bash, russh] {
            assert!(!shell.command(7).lines().any(|line| line == flood_marker(7)));
            assert!(shell.label().contains("5000000"));
        }

        assert_eq!(ScrollbackCheck::of(10_000, 10_000), ScrollbackCheck::AtCap);
        assert_eq!(ScrollbackCheck::of(9_000, 10_000), ScrollbackCheck::Below);
        assert_eq!(ScrollbackCheck::of(10_001, 10_000), ScrollbackCheck::Over);
        assert_eq!(ScrollbackCheck::Over.label(), "over the cap");
    }

    #[test]
    fn nearest_rank_percentiles() {
        let mut samples: Vec<Duration> = (1..=200).map(Duration::from_millis).collect();
        samples.reverse();
        assert_eq!(
            percentile(&mut samples, 50.0),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            percentile(&mut samples, 95.0),
            Some(Duration::from_millis(190))
        );
        assert_eq!(
            percentile(&mut samples, 100.0),
            Some(Duration::from_millis(200))
        );
        assert_eq!(percentile(&mut [], 50.0), None);
        assert_eq!(
            percentile(&mut [Duration::from_millis(7)], 95.0),
            Some(Duration::from_millis(7))
        );
        assert_eq!(millis(Duration::from_micros(12_345)), "12.35");
    }

    #[test]
    fn detects_the_end_of_seq_output() {
        let rows =
            |list: &[&str]| -> Vec<String> { list.iter().map(|r| (*r).to_owned()).collect() };
        assert!(!seq_finished(&rows(&["2999998", "2999999"]), 3_000_000));
        assert!(!seq_finished(&rows(&["2999999", "3000000", ""]), 3_000_000));
        assert!(seq_finished(
            &rows(&["2999999", "3000000", "pwuser@shellpp-test:~$ "]),
            3_000_000
        ));
        assert!(seq_finished(
            &rows(&["3000000   ", "PS C:\\Users\\x> "]),
            3_000_000
        ));
        // The echoed command line is not the last number.
        assert!(!seq_finished(&rows(&["$ seq 1 3000000", "1"]), 3_000_000));

        assert!(seq_direct_finished(
            &rows(&["2999999", "3000000"]),
            3_000_000
        ));
        assert!(!seq_direct_finished(
            &rows(&["299999", "300000"]),
            3_000_000
        ));

        let command = "PS> & 'seq.exe' 1 3000000; 'seq-bench-' + 'done'";
        assert!(!seq_shell_finished(&rows(&[command, "1"]), 3_000_000));
        assert!(!seq_shell_finished(
            &rows(&["2999999", "3000000", "PS> "]),
            3_000_000
        ));
        assert!(seq_shell_finished(
            &rows(&["3000000", "seq-bench-done  ", "PS> "]),
            3_000_000
        ));
    }

    #[test]
    fn finds_a_marker_across_chunks() {
        let mut matcher = StreamMatch::new("3000000");
        assert!(!matcher.feed(b"2999999\r\n300"));
        assert!(!matcher.found());
        assert!(matcher.feed(b"0000\r\n"));
        assert!(matcher.found());
        assert!(matcher.feed(b"anything"), "stays found");

        // One byte at a time.
        let mut matcher = StreamMatch::new(SHELL_DONE);
        let stream = b"\x1b[32m'seq-bench-' + 'done'\x1b[m\r\n1\r\nseq-bench-done\r\n";
        let mut hit = None;
        for (index, byte) in stream.iter().enumerate() {
            if matcher.feed(std::slice::from_ref(byte)) {
                hit = Some(index);
                break;
            }
        }
        assert_eq!(hit, Some(stream.len() - 3), "at the marker's last byte");

        // Chunks longer and shorter than the needle; the echo never matches.
        let mut matcher = StreamMatch::new(SHELL_DONE);
        assert!(!matcher.feed(b"'seq-bench-' + 'done'"));
        assert!(!matcher.feed(b"seq-bench"));
        assert!(!matcher.feed(b"-"));
        assert!(matcher.feed(b"done"));

        let mut matcher = StreamMatch::new("abcdef");
        assert!(!matcher.feed(b"xxxxxxxxabc"));
        assert!(!matcher.feed(b"d"));
        assert!(matcher.feed(b"efyyyyyyyyyyy"));
    }

    #[test]
    fn local_seq_commands() {
        let git = LocalSeq::GitSeq(PathBuf::from(r"C:\Git\usr\bin\seq.exe"));
        assert_eq!(
            git.command(5),
            r"& 'C:\Git\usr\bin\seq.exe' 1 5; 'seq-bench-' + 'done'"
        );
        assert_eq!(
            git.direct_program(),
            Some(Path::new(r"C:\Git\usr\bin\seq.exe"))
        );
        assert!(LocalSeq::PowerShellLoop.command(5).contains("1..5"));
        assert_eq!(LocalSeq::PowerShellLoop.direct_program(), None);
        assert_eq!(
            LocalSeq::Seq(None).command(5),
            "seq 1 5; printf 'seq-bench-%s\\n' done"
        );
        // Every command prints the marker, and its echo never contains it.
        for seq in [git.clone(), LocalSeq::PowerShellLoop, LocalSeq::Seq(None)] {
            assert!(!seq.command(5).contains(SHELL_DONE), "{seq:?}");
        }
        let detected = local_seq();
        if cfg!(windows) {
            assert!(matches!(
                detected,
                LocalSeq::GitSeq(_) | LocalSeq::PowerShellLoop
            ));
        } else {
            assert!(matches!(detected, LocalSeq::Seq(_)));
        }
        if let Some(program) = detected.direct_program() {
            assert!(program.is_absolute() && program.is_file());
        }
        assert_eq!(OutputKind::BareShell.token(), "bare-shell");
        assert!(OutputKind::LocalDirect.through_terminal());
        assert!(!OutputKind::BareDirect.through_terminal());
        assert_eq!(latency_char(0), 'a');
        assert_eq!(latency_char(27), 'b');
        assert!(latency_line_start(0) && latency_line_start(40) && !latency_line_start(39));
    }
}
