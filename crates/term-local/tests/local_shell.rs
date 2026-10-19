//! Local terminal tests against the real shells of this machine: detection,
//! and a PTY session that runs a trivial command, resizes, exits with a code
//! and can be killed. Robust to prompts and colors: escape sequences are
//! stripped and the tests look for random markers whose text appears only in
//! the command's output, never in the echo of the typed command.
//!
//! Test commands are kept out of the user's shell history (PowerShell starts
//! with history saving off; POSIX shells unset `HISTFILE` first).

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use shellpp_term_local::{
    ExitInfo, LocalError, LocalSession, PtyConfig, PtyWriter, ShellSpec, default_shell_id,
    detect_shells, resolve_shell, spawn,
};
use tokio::time::{Instant, timeout_at};

/// How long one step (a command's output, the exit, the end of file) may take.
const STEP: Duration = Duration::from_secs(20);
/// How often a command is sent when its output does not appear.
const ATTEMPTS: usize = 3;
const TERM: &str = "xterm-256color";

fn random_token() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

/// The command language of a shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialect {
    PowerShell,
    Cmd,
    Posix,
}

impl Dialect {
    fn of(spec: &ShellSpec) -> Self {
        let stem = spec
            .program
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match stem.as_str() {
            "powershell" | "pwsh" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ => Self::Posix,
        }
    }

    /// A command that prints `shellpp-<token>`; the typed text never holds
    /// that string in one piece.
    fn marker_command(self, token: &str) -> String {
        match self {
            Self::PowerShell => format!("Write-Output ('shellpp-' + '{token}')"),
            Self::Cmd => format!("echo shellpp-^{token}"),
            Self::Posix => format!("echo shellpp-''{token}"),
        }
    }

    /// A command that prints `term=<TERM>`.
    fn term_command(self) -> &'static str {
        match self {
            Self::PowerShell => "Write-Output ('term=' + $env:TERM)",
            Self::Cmd => "echo term=%TERM%",
            Self::Posix => "echo \"term=$TERM\"",
        }
    }

    /// A command that prints the terminal width and the text it prints for
    /// `cols` x `rows`; `None` when the dialect has no simple way.
    fn size_command(self, cols: u16, rows: u16) -> Option<(&'static str, String)> {
        match self {
            Self::PowerShell => Some((
                "Write-Output ('cols=' + [Console]::WindowWidth)",
                format!("cols={cols}"),
            )),
            Self::Posix => Some(("stty size", format!("{rows} {cols}"))),
            Self::Cmd => None,
        }
    }

    fn exit_command(self, code: u32) -> String {
        match self {
            Self::Cmd => format!("exit /b {code}"),
            Self::PowerShell | Self::Posix => format!("exit {code}"),
        }
    }
}

/// Keeps test commands out of the user's history.
fn hygienic(mut spec: ShellSpec) -> ShellSpec {
    if Dialect::of(&spec) == Dialect::PowerShell {
        spec.args = [
            "-NoLogo",
            "-NoProfile",
            "-NoExit",
            "-Command",
            "Set-PSReadLineOption -HistorySaveStyle SaveNothing",
        ]
        .map(str::to_owned)
        .to_vec();
    }
    spec
}

fn default_spec() -> ShellSpec {
    let shells = detect_shells();
    resolve_shell(None, None, &shells).expect("a default shell")
}

/// Removes CSI, OSC and other escape sequences.
fn strip_escapes(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        match bytes.get(i) {
            Some(b'[') => {
                i += 1;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i += 1;
            }
            Some(b']') => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            Some(b'(' | b')' | b'*' | b'+') => i += 2,
            Some(_) => i += 1,
            None => {}
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// A shell in a PTY, driven like a terminal emulator would.
struct Terminal {
    session: LocalSession,
    writer: PtyWriter,
    dialect: Dialect,
    raw: Vec<u8>,
    /// Cursor position reports sent (ConPTY asks once at start).
    answered: usize,
    exit: Option<ExitInfo>,
    eof: bool,
}

impl Terminal {
    fn start(spec: ShellSpec, cols: u16, rows: u16) -> Self {
        let dialect = Dialect::of(&spec);
        let session = spawn(PtyConfig {
            shell: hygienic(spec),
            cols,
            rows,
            term: TERM.to_owned(),
            cwd: None,
        })
        .expect("spawn the shell");
        let writer = session.writer();
        let terminal = Self {
            session,
            writer,
            dialect,
            raw: Vec::new(),
            answered: 0,
            exit: None,
            eof: false,
        };
        if dialect == Dialect::Posix {
            terminal.send(" unset HISTFILE");
        }
        terminal
    }

    fn send(&self, line: &str) {
        self.writer
            .write(format!("{line}\r").as_bytes())
            .expect("write to the shell");
    }

    /// Answers the cursor position requests seen so far.
    fn answer_requests(&mut self) {
        let requests = count(&self.raw, b"\x1b[6n");
        while self.answered < requests {
            // A closed session cannot answer; the caller sees the exit.
            let _ = self.writer.write(b"\x1b[1;1R");
            self.answered += 1;
        }
    }

    /// Receives output (and the exit) until `deadline`; false once nothing
    /// more can arrive or the deadline passed.
    async fn pump(&mut self, deadline: Instant) -> bool {
        if self.eof {
            return false;
        }
        let exit_pending = self.exit.is_none();
        let received = timeout_at(deadline, async {
            tokio::select! {
                chunk = self.session.output.recv() => Some(chunk),
                exit = &mut self.session.exit, if exit_pending => {
                    self.exit = Some(exit.unwrap_or(ExitInfo { code: None }));
                    None
                }
            }
        })
        .await;
        match received {
            Err(_) => false,
            Ok(None) => true,
            Ok(Some(None)) => {
                self.eof = true;
                false
            }
            Ok(Some(Some(chunk))) => {
                assert!(!chunk.is_empty(), "no empty chunks");
                assert!(chunk.len() <= shellpp_term_local::pty::MAX_CHUNK);
                self.raw.extend_from_slice(&chunk);
                self.answer_requests();
                true
            }
        }
    }

    /// Waits until `needle` appears in the output after byte `from`.
    async fn wait_for(&mut self, from: usize, needle: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            if strip_escapes(&self.raw[from..]).contains(needle) {
                return true;
            }
            if !self.pump(deadline).await {
                return strip_escapes(&self.raw[from..]).contains(needle);
            }
        }
    }

    /// Sends `line` (again, if needed) until `needle` appears.
    async fn run(&mut self, line: &str, needle: &str) -> bool {
        for _ in 0..ATTEMPTS {
            let from = self.raw.len();
            self.send(line);
            if self.wait_for(from, needle, STEP).await {
                return true;
            }
        }
        false
    }

    /// Waits for any output (the shell is up).
    async fn wait_for_output(&mut self) {
        let deadline = Instant::now() + STEP;
        while self.raw.is_empty() {
            assert!(self.pump(deadline).await, "the shell printed nothing");
        }
    }

    /// Drains output until the exit and the end of file.
    async fn finish(&mut self) -> (ExitInfo, bool) {
        let deadline = Instant::now() + STEP;
        while self.exit.is_none() || !self.eof {
            if !self.pump(deadline).await && !self.eof {
                break;
            }
        }
        if self.exit.is_none() && self.eof {
            // End of file came first: the exit follows on its own.
            let exit = timeout_at(Instant::now() + STEP, &mut self.session.exit)
                .await
                .expect("the exit follows the end of file");
            self.exit = Some(exit.unwrap_or(ExitInfo { code: None }));
        }
        (self.exit.expect("the shell exited"), self.eof)
    }
}

#[test]
fn detects_shells_with_a_default() {
    let shells = detect_shells();
    assert!(!shells.is_empty(), "at least one shell");
    let default = default_shell_id(&shells).expect("a default shell");
    assert!(shells.iter().any(|shell| shell.id == default));
    for (index, shell) in shells.iter().enumerate() {
        assert!(!shell.id.is_empty() && !shell.label.is_empty());
        assert!(
            shell.program.is_absolute() && shell.program.is_file(),
            "shell {index} has an existing absolute program"
        );
        assert!(
            shells[..index].iter().all(|other| other.id != shell.id),
            "ids are unique"
        );
    }
    if cfg!(windows) {
        assert_eq!(default, "powershell");
        let known = ["powershell", "pwsh", "cmd", "git-bash"];
        for shell in &shells {
            assert!(
                known.contains(&shell.id.as_str()) || shell.id.starts_with("wsl:"),
                "known Windows id"
            );
        }
        assert!(shells.iter().any(|shell| shell.id == "cmd"));
        let powershell = shells
            .iter()
            .find(|s| s.id == "powershell")
            .expect("powershell");
        assert_eq!(powershell.label, "PowerShell");
        assert_eq!(powershell.args, vec!["-NoLogo".to_owned()]);
    } else {
        for shell in &shells {
            assert_eq!(shell.program.to_str(), Some(shell.id.as_str()));
        }
    }
    let spec = resolve_shell(None, None, &shells).expect("resolved default");
    assert_eq!(spec.id.as_deref(), Some(default.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_shell_runs_a_command_resizes_and_exits_with_a_code() {
    let mut terminal = Terminal::start(default_spec(), 120, 40);
    let dialect = terminal.dialect;

    let token = random_token();
    let marker = format!("shellpp-{token}");
    assert!(
        terminal.run(&dialect.marker_command(&token), &marker).await,
        "the marker appears"
    );
    assert!(
        terminal
            .run(dialect.term_command(), &format!("term={TERM}"))
            .await,
        "TERM reaches the shell"
    );

    terminal.writer.resize(100, 30).expect("resize");
    if let Some((command, expected)) = dialect.size_command(100, 30) {
        assert!(
            terminal.run(command, &expected).await,
            "the new size reaches the shell"
        );
    }

    terminal.send(&dialect.exit_command(3));
    let (exit, eof) = terminal.finish().await;
    assert_eq!(exit, ExitInfo { code: Some(3) });
    assert!(eof, "the output closes after the exit");
    assert_eq!(terminal.writer.write(b"x"), Err(LocalError::Closed));
    assert_eq!(terminal.writer.resize(80, 24), Err(LocalError::Closed));
    // Killing an ended session has no effect.
    terminal.writer.kill();
}

/// Every detected Windows shell (Command Prompt always; PowerShell 7 and Git
/// Bash when installed) runs a command and exits with a code. WSL
/// distributions are left out: starting one boots the WSL virtual machine.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_detected_windows_shell_runs_a_command_and_exits_with_a_code() {
    let shells = detect_shells();
    let mut tested = Vec::new();
    for shell in shells.iter().filter(|shell| !shell.id.starts_with("wsl:")) {
        let spec = resolve_shell(Some(&shell.id), None, &shells).expect("resolved");
        let mut terminal = Terminal::start(spec, 100, 30);
        let dialect = terminal.dialect;
        let token = random_token();
        let marker = format!("shellpp-{token}");
        assert!(
            terminal.run(&dialect.marker_command(&token), &marker).await,
            "the marker appears in {}",
            shell.id
        );
        terminal.send(&dialect.exit_command(3));
        let (exit, eof) = terminal.finish().await;
        assert_eq!(
            exit,
            ExitInfo { code: Some(3) },
            "exit code of {}",
            shell.id
        );
        assert!(eof, "end of file of {}", shell.id);
        tested.push(shell.id.clone());
    }
    assert!(
        tested.iter().any(|id| id == "cmd"),
        "Command Prompt is tested"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_ends_the_session() {
    let mut terminal = Terminal::start(default_spec(), 80, 24);
    terminal.wait_for_output().await;
    terminal.writer.kill();
    let (exit, eof) = terminal.finish().await;
    assert_eq!(exit, ExitInfo { code: None }, "a killed shell has no code");
    assert!(eof);
    assert_eq!(terminal.writer.write(b"x"), Err(LocalError::Closed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_every_writer_ends_the_process() {
    let LocalSession {
        mut output,
        mut exit,
        ..
    } = spawn(PtyConfig {
        shell: hygienic(default_spec()),
        cols: 80,
        rows: 24,
        term: TERM.to_owned(),
        cwd: None,
    })
    .expect("spawn the shell");

    let deadline = Instant::now() + STEP;
    let mut exited = None;
    let mut eof = false;
    while exited.is_none() || !eof {
        let step = timeout_at(deadline, async {
            tokio::select! {
                chunk = output.recv(), if !eof => {
                    if chunk.is_none() {
                        eof = true;
                    }
                }
                code = &mut exit, if exited.is_none() => exited = Some(code),
            }
        })
        .await;
        assert!(step.is_ok(), "the process ends without its writers");
    }
    assert_eq!(exited, Some(Ok(ExitInfo { code: None })));
}

#[test]
fn spawn_rejects_a_missing_program() {
    let missing = std::env::temp_dir().join(format!("shellpp-missing-{}", random_token()));
    for program in [missing, "relative-shell".into()] {
        let result = spawn(PtyConfig {
            shell: ShellSpec {
                id: None,
                label: "missing".to_owned(),
                program,
                args: Vec::new(),
            },
            cols: 80,
            rows: 24,
            term: TERM.to_owned(),
            cwd: None,
        });
        assert_eq!(result.err(), Some(LocalError::Spawn));
    }
}
