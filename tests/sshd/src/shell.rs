//! The deterministic built-in shell (docs/architecture.md, section 9.3).
//! Its exact behavior is the contract of the stage 2 plan, section 4.1
//! ("Shell"): prompt, line discipline, echo control and the built-in
//! commands that Rust tests and e2e specs drive.
//!
//! [`Shell`] is a pure state machine: it turns input bytes into [`Action`]s
//! (output, exit, drop, timers). The server runs one per session channel and
//! performs the actions; unit tests drive it directly.
//!
//! Line discipline (server-side echo, like a cooked TTY):
//! - printable input (including UTF-8) is appended to the line and echoed;
//! - `\r` or `\n` (a `\n` right after `\r` is swallowed) echoes `\r\n` and
//!   runs the line; an empty line prints the prompt again;
//! - 0x7F or 0x08 erases the last character (`"\x08 \x08"`);
//! - Ctrl+C prints `^C\r\n` and a new prompt; Ctrl+D on an empty line exits
//!   with status 0;
//! - escape sequences (arrows, bracketed paste markers, OSC strings) are
//!   consumed and ignored, so pasted text between `ESC[200~` and `ESC[201~`
//!   is processed like typed text; other control characters are ignored.
//!
//! Commands (first word; output lines end with `\r\n`):
//! `echo <text>`, `exit [n]`, `read-secret`, `alt-screen`, `size`, `term`,
//! `env <NAME>`, `agent-keys`, `lines <n>`, `seq [<first>] <last>`, `bell`,
//! `url`, `title <text>`, `unicode`, `color`, `sleep <s>`,
//! `later <s> <text>`, `drop`; anything else prints
//! `<cmd>: command not found`.

use std::time::Duration;

/// The prompt printed before every command line: `<user>@shellpp-test:~$ `.
pub fn prompt(user: &str) -> String {
    format!("{user}@shellpp-test:~$ ")
}

/// Default MOTD printed once when the shell starts.
pub const DEFAULT_MOTD: &str = "Welcome to shellpp-test-sshd\r\n";

/// Prompt of `read-secret` (password prompt simulation with echo off).
pub const SECRET_PROMPT: &str = "[sudo] password for test: ";

/// What `read-secret` prints after the hidden line (then the prompt follows).
pub const SECRET_DONE: &str = "\r\nok\r\n";

/// Text shown by `alt-screen` on the alternate screen.
pub const ALT_SCREEN_TEXT: &str = "ALT SCREEN (press q)";

/// Enter the alternate screen (xterm private mode 1049).
pub const ALT_SCREEN_ON: &str = "\x1b[?1049h";

/// Leave the alternate screen.
pub const ALT_SCREEN_OFF: &str = "\x1b[?1049l";

/// Output of `url`.
pub const URL_TEXT: &str = "https://example.com/docs";

/// Output of `unicode`.
pub const UNICODE_TEXT: &str = "wide: 日本語 ✓ 👍";

/// Output of `agent-keys` when the client did not request agent forwarding.
pub const AGENT_NOT_FORWARDED: &str = "agent: not forwarded";

/// Output of `agent-keys` when the forwarded agent did not answer (channel
/// refused, protocol error or no answer within 10 seconds). On success it
/// prints `agent keys: <n>`.
pub const AGENT_ERROR: &str = "agent: error";

/// Largest count accepted by `lines`.
pub const MAX_LINES: u32 = 10_000;

/// Largest number accepted by `seq` (the terminal throughput benchmark of
/// the desktop app prints `seq 1 3000000`, about 23 MB).
pub const MAX_SEQ: u64 = 10_000_000;

/// `seq` output is sent in chunks of about this many bytes.
const SEQ_CHUNK: usize = 64 * 1024;

/// Longest delay accepted by `sleep` and `later`, in seconds.
pub const MAX_DELAY_SECS: f64 = 60.0;

/// Output of `color`: the 16 ANSI foreground colors, each labelled with its
/// SGR code, then a reset.
pub fn color_sample() -> String {
    let mut text = String::new();
    for code in (30..=37).chain(90..=97) {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&format!("\x1b[{code}m{code}"));
    }
    text.push_str("\x1b[0m");
    text
}

/// The PTY of a session: TERM and size (from the PTY request and window
/// changes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PtyInfo {
    pub term: String,
    pub cols: u32,
    pub rows: u32,
}

/// Something the server must do for the shell, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Write these bytes to the channel.
    Output(Vec<u8>),
    /// Send this exit status, EOF and close the channel. The shell is done.
    Exit(u32),
    /// Close the TCP connection without exit status. The shell is done.
    Drop,
    /// Wait, then call [`Shell::resume`] with `None`. Input received
    /// meanwhile is buffered by the shell.
    Sleep(Duration),
    /// Ask the forwarded agent for its identities, then call
    /// [`Shell::resume`] with the line to print.
    AgentKeys,
    /// `agent-sign`: asks the forwarded agent to sign with its first key.
    AgentSign,
    /// Cancel the pending `Sleep` or `AgentKeys` (Ctrl+C); a later
    /// [`Shell::resume`] is ignored.
    CancelBusy,
    /// After the delay, call [`Shell::later`] with the text.
    Later(Duration, String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Not started yet.
    Idle,
    /// Reading a command line with echo.
    Line,
    /// `read-secret`: reading one line without echo.
    Secret,
    /// `alt-screen`: waiting for `q`.
    AltScreen,
    /// `sleep` or `agent-keys` in progress: input is buffered.
    Busy,
    /// Exited or dropped.
    Ended,
}

/// Escape sequence parser state (input side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Esc {
    None,
    /// After ESC.
    Start,
    /// Control sequence: `ESC [` parameters, intermediates, final byte.
    Csi,
    /// `ESC O` + one byte.
    Ss3,
    /// String (OSC, DCS, APC, PM, SOS) until BEL or `ESC \`.
    Str,
    /// ESC inside a string (maybe the `ESC \` terminator).
    StrTerminator,
}

/// One input byte after escape filtering.
enum Key {
    Byte(u8),
    Ignored,
}

/// The shell of one session channel.
#[derive(Debug)]
pub struct Shell {
    user: String,
    motd: String,
    pty: Option<PtyInfo>,
    env: Vec<(String, String)>,
    mode: Mode,
    line: Vec<u8>,
    esc: Esc,
    last_cr: bool,
    pending: Vec<u8>,
    actions: Vec<Action>,
}

impl Shell {
    /// A shell for `user`; `motd` defaults to [`DEFAULT_MOTD`].
    pub fn new(user: &str, motd: Option<&str>, pty: Option<PtyInfo>) -> Self {
        Self {
            user: user.to_owned(),
            motd: motd.unwrap_or(DEFAULT_MOTD).to_owned(),
            pty,
            env: Vec::new(),
            mode: Mode::Idle,
            line: Vec::new(),
            esc: Esc::None,
            last_cr: false,
            pending: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// Records an env request (`env <NAME>` prints the last value).
    pub fn set_env(&mut self, name: &str, value: &str) {
        self.env.retain(|(existing, _)| existing != name);
        self.env.push((name.to_owned(), value.to_owned()));
    }

    /// Updates the PTY size after a window change (a session without a PTY
    /// request gets one with an empty TERM).
    pub fn resize(&mut self, cols: u32, rows: u32) {
        let pty = self.pty.get_or_insert_with(PtyInfo::default);
        pty.cols = cols;
        pty.rows = rows;
    }

    /// Whether the shell has exited or dropped the connection.
    pub fn is_ended(&self) -> bool {
        self.mode == Mode::Ended
    }

    /// Prints the MOTD and the first prompt.
    pub fn start(&mut self) -> Vec<Action> {
        if self.mode == Mode::Idle {
            self.mode = Mode::Line;
            let motd = std::mem::take(&mut self.motd);
            self.out(motd.as_bytes());
            self.print_prompt();
        }
        self.take()
    }

    /// Feeds input bytes from the client.
    pub fn input(&mut self, data: &[u8]) -> Vec<Action> {
        self.feed(data);
        self.take()
    }

    /// Ends a `Sleep` or `AgentKeys`: prints `line` (if any) and the prompt,
    /// then processes the input buffered meanwhile. Ignored when the shell is
    /// not busy (for example after Ctrl+C).
    pub fn resume(&mut self, line: Option<&str>) -> Vec<Action> {
        if self.mode == Mode::Busy {
            self.mode = Mode::Line;
            if let Some(line) = line {
                self.println(line);
            }
            self.print_prompt();
            let pending = std::mem::take(&mut self.pending);
            self.feed(&pending);
        }
        self.take()
    }

    /// A `later` timer fired: prints the text on its own line and redraws
    /// the prompt and the line being typed.
    pub fn later(&mut self, text: &str) -> Vec<Action> {
        match self.mode {
            Mode::Idle | Mode::Ended => {}
            Mode::Line => {
                self.out(b"\r\n");
                self.println(text);
                self.print_prompt();
                let line = self.line.clone();
                self.out(&line);
            }
            Mode::Secret => {
                self.out(b"\r\n");
                self.println(text);
                self.out(SECRET_PROMPT.as_bytes());
            }
            Mode::AltScreen | Mode::Busy => self.println(text),
        }
        self.take()
    }

    fn take(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    fn out(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(Action::Output(buffer)) = self.actions.last_mut() {
            buffer.extend_from_slice(bytes);
        } else {
            self.actions.push(Action::Output(bytes.to_vec()));
        }
    }

    fn println(&mut self, text: &str) {
        self.out(text.as_bytes());
        self.out(b"\r\n");
    }

    /// `seq`: the numbers `first..=last`, one per line like coreutils `seq`
    /// on a PTY, in separate output chunks of about [`SEQ_CHUNK`] bytes.
    fn seq(&mut self, first: u64, last: u64) {
        use std::io::Write as _;
        let mut chunk = Vec::with_capacity(SEQ_CHUNK + 32);
        for number in first..=last {
            let _ = write!(chunk, "{number}\r\n");
            if chunk.len() >= SEQ_CHUNK {
                self.actions.push(Action::Output(std::mem::replace(
                    &mut chunk,
                    Vec::with_capacity(SEQ_CHUNK + 32),
                )));
            }
        }
        if !chunk.is_empty() {
            self.actions.push(Action::Output(chunk));
        }
    }

    fn print_prompt(&mut self) {
        let prompt = prompt(&self.user);
        self.out(prompt.as_bytes());
    }

    fn feed(&mut self, data: &[u8]) {
        let mut index = 0;
        while index < data.len() {
            match self.mode {
                Mode::Idle | Mode::Ended => return,
                Mode::Busy => {
                    let rest = &data[index..];
                    match rest.iter().position(|&byte| byte == 0x03) {
                        Some(position) => {
                            // Ctrl+C interrupts `sleep` / `agent-keys`.
                            self.pending.clear();
                            self.actions.push(Action::CancelBusy);
                            self.mode = Mode::Line;
                            self.esc = Esc::None;
                            self.out(b"^C\r\n");
                            self.print_prompt();
                            index += position + 1;
                        }
                        None => {
                            self.pending.extend_from_slice(rest);
                            return;
                        }
                    }
                }
                Mode::Line | Mode::Secret | Mode::AltScreen => {
                    let byte = data[index];
                    index += 1;
                    if let Key::Byte(byte) = self.filter(byte) {
                        self.key(byte);
                    }
                }
            }
        }
    }

    /// Escape sequence filter: returns the bytes that are not part of an
    /// escape sequence.
    fn filter(&mut self, byte: u8) -> Key {
        match self.esc {
            Esc::None => {
                if byte == 0x1b {
                    self.esc = Esc::Start;
                    Key::Ignored
                } else {
                    Key::Byte(byte)
                }
            }
            Esc::Start => {
                self.esc = match byte {
                    b'[' => Esc::Csi,
                    b'O' => Esc::Ss3,
                    b']' | b'P' | b'_' | b'^' | b'X' => Esc::Str,
                    0x1b => Esc::Start,
                    _ => Esc::None,
                };
                Key::Ignored
            }
            Esc::Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    self.esc = Esc::None;
                } else if byte == 0x1b {
                    self.esc = Esc::Start;
                } else if !(0x20..=0x3f).contains(&byte) {
                    // A malformed sequence: drop it, keep the byte.
                    self.esc = Esc::None;
                    return Key::Byte(byte);
                }
                Key::Ignored
            }
            Esc::Ss3 => {
                self.esc = Esc::None;
                Key::Ignored
            }
            Esc::Str => {
                match byte {
                    0x07 => self.esc = Esc::None,
                    0x1b => self.esc = Esc::StrTerminator,
                    _ => {}
                }
                Key::Ignored
            }
            Esc::StrTerminator => {
                self.esc = if byte == b'\\' {
                    Esc::None
                } else if byte == 0x1b {
                    Esc::StrTerminator
                } else {
                    Esc::Str
                };
                Key::Ignored
            }
        }
    }

    fn key(&mut self, byte: u8) {
        let after_cr = std::mem::replace(&mut self.last_cr, byte == b'\r');
        if self.mode == Mode::AltScreen {
            if byte == b'q' {
                self.out(ALT_SCREEN_OFF.as_bytes());
                self.mode = Mode::Line;
                self.print_prompt();
            }
            return;
        }
        let echo = self.mode == Mode::Line;
        match byte {
            b'\n' if after_cr => {}
            b'\r' | b'\n' => {
                if echo {
                    self.out(b"\r\n");
                }
                let line = std::mem::take(&mut self.line);
                if self.mode == Mode::Secret {
                    self.mode = Mode::Line;
                    self.out(SECRET_DONE.as_bytes());
                    self.print_prompt();
                } else {
                    self.run(&String::from_utf8_lossy(&line));
                }
            }
            0x7f | 0x08 => {
                if self.erase_char() && echo {
                    self.out(b"\x08 \x08");
                }
            }
            0x03 => {
                self.line.clear();
                self.mode = Mode::Line;
                self.out(b"^C\r\n");
                self.print_prompt();
            }
            0x04 => {
                if self.mode == Mode::Line && self.line.is_empty() {
                    self.exit(0);
                }
            }
            0x00..=0x1f => {}
            _ => {
                self.line.push(byte);
                if echo {
                    self.out(&[byte]);
                }
            }
        }
    }

    /// Removes the last UTF-8 character of the line; `false` when empty.
    fn erase_char(&mut self) -> bool {
        let Some(mut last) = self.line.pop() else {
            return false;
        };
        while last & 0xc0 == 0x80 {
            match self.line.pop() {
                Some(byte) => last = byte,
                None => break,
            }
        }
        true
    }

    fn exit(&mut self, status: u32) {
        self.mode = Mode::Ended;
        self.actions.push(Action::Exit(status));
    }

    fn run(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            self.print_prompt();
            return;
        }
        let (command, rest) = match line.split_once(char::is_whitespace) {
            Some((command, rest)) => (command, rest.trim()),
            None => (line, ""),
        };
        match command {
            "echo" => self.println(rest),
            "exit" => {
                if rest.is_empty() {
                    self.exit(0);
                } else if let Ok(status) = rest.parse::<u32>() {
                    self.exit(status);
                } else {
                    self.println("exit: numeric argument required");
                    self.exit(2);
                }
                return;
            }
            "read-secret" => {
                self.out(SECRET_PROMPT.as_bytes());
                self.mode = Mode::Secret;
                return;
            }
            "alt-screen" => {
                self.out(ALT_SCREEN_ON.as_bytes());
                self.out(ALT_SCREEN_TEXT.as_bytes());
                self.mode = Mode::AltScreen;
                return;
            }
            "size" => {
                let text = self
                    .pty
                    .as_ref()
                    .map(|pty| format!("{}x{}", pty.cols, pty.rows))
                    .unwrap_or_else(|| "0x0".to_owned());
                self.println(&text);
            }
            "term" => {
                let text = self
                    .pty
                    .as_ref()
                    .map(|pty| pty.term.clone())
                    .unwrap_or_default();
                self.println(&text);
            }
            "env" => {
                let value = self
                    .env
                    .iter()
                    .find(|(name, _)| name == rest)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default();
                self.println(&value);
            }
            "agent-keys" => {
                self.mode = Mode::Busy;
                self.actions.push(Action::AgentKeys);
                return;
            }
            "agent-sign" => {
                self.mode = Mode::Busy;
                self.actions.push(Action::AgentSign);
                return;
            }
            "lines" => match rest.parse::<u32>() {
                Ok(count) if count <= MAX_LINES => {
                    for number in 1..=count {
                        self.println(&format!("line {number}"));
                    }
                }
                _ => self.println("lines: invalid count"),
            },
            "seq" => match parse_seq(rest) {
                Some((first, last)) => self.seq(first, last),
                None => self.println("seq: invalid range"),
            },
            "bell" => self.out(b"\x07"),
            "url" => self.println(URL_TEXT),
            "title" => self.out(format!("\x1b]0;{rest}\x07").as_bytes()),
            "unicode" => self.println(UNICODE_TEXT),
            "color" => self.println(&color_sample()),
            "sleep" => match parse_delay(rest) {
                Some(delay) => {
                    self.mode = Mode::Busy;
                    self.actions.push(Action::Sleep(delay));
                    return;
                }
                None => self.println("sleep: invalid time"),
            },
            "later" => {
                let (delay, text) = match rest.split_once(char::is_whitespace) {
                    Some((delay, text)) => (delay, text.trim()),
                    None => (rest, ""),
                };
                match parse_delay(delay) {
                    Some(delay) => self.actions.push(Action::Later(delay, text.to_owned())),
                    None => self.println("later: invalid time"),
                }
            }
            "drop" => {
                self.mode = Mode::Ended;
                self.actions.push(Action::Drop);
                return;
            }
            other => self.println(&format!("{other}: command not found")),
        }
        self.print_prompt();
    }
}

/// `seq` arguments: `<last>` (from 1) or `<first> <last>`, both between 1
/// and [`MAX_SEQ`]; an empty range (`first > last`) prints nothing.
fn parse_seq(text: &str) -> Option<(u64, u64)> {
    let mut numbers = text.split_whitespace().map(str::parse::<u64>);
    let (first, last) = match (numbers.next(), numbers.next(), numbers.next()) {
        (Some(Ok(last)), None, None) => (1, last),
        (Some(Ok(first)), Some(Ok(last)), None) => (first, last),
        _ => return None,
    };
    let valid = 1..=MAX_SEQ;
    (valid.contains(&first) && valid.contains(&last)).then_some((first, last))
}

/// Seconds (decimal allowed) between 0 and [`MAX_DELAY_SECS`].
fn parse_delay(text: &str) -> Option<Duration> {
    let seconds: f64 = text.parse().ok()?;
    if seconds.is_finite() && (0.0..=MAX_DELAY_SECS).contains(&seconds) {
        Some(Duration::from_secs_f64(seconds))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(user: &str) -> Shell {
        let mut shell = Shell::new(
            user,
            None,
            Some(PtyInfo {
                term: "xterm-256color".to_owned(),
                cols: 80,
                rows: 24,
            }),
        );
        shell.start();
        shell
    }

    /// Concatenated output of the actions; other actions are returned apart.
    fn split(actions: Vec<Action>) -> (String, Vec<Action>) {
        let mut text = Vec::new();
        let mut rest = Vec::new();
        for action in actions {
            match action {
                Action::Output(bytes) => text.extend_from_slice(&bytes),
                other => rest.push(other),
            }
        }
        (String::from_utf8(text).expect("utf-8 output"), rest)
    }

    fn run(shell: &mut Shell, input: &str) -> (String, Vec<Action>) {
        split(shell.input(input.as_bytes()))
    }

    #[test]
    fn start_prints_motd_then_prompt() {
        let mut shell = Shell::new("alice", None, None);
        let (text, rest) = split(shell.start());
        assert_eq!(
            text,
            "Welcome to shellpp-test-sshd\r\nalice@shellpp-test:~$ "
        );
        assert!(rest.is_empty());
        // A second start does nothing.
        assert!(shell.start().is_empty());

        let mut custom = Shell::new("bob", Some("hello\r\n"), None);
        assert_eq!(split(custom.start()).0, "hello\r\nbob@shellpp-test:~$ ");
    }

    #[test]
    fn echo_command_echoes_input_and_prints_text() {
        let mut shell = started("alice");
        let (text, rest) = run(&mut shell, "echo hi there\r");
        assert_eq!(text, "echo hi there\r\nhi there\r\nalice@shellpp-test:~$ ");
        assert!(rest.is_empty());
    }

    #[test]
    fn crlf_runs_one_line_and_lf_alone_runs_too() {
        let mut shell = started("alice");
        let (text, _) = run(&mut shell, "echo a\r\n");
        assert_eq!(text, "echo a\r\na\r\nalice@shellpp-test:~$ ");
        let (text, _) = run(&mut shell, "echo b\n");
        assert_eq!(text, "echo b\r\nb\r\nalice@shellpp-test:~$ ");
    }

    #[test]
    fn empty_line_prints_the_prompt_again() {
        let mut shell = started("alice");
        let (text, _) = run(&mut shell, "\r");
        assert_eq!(text, "\r\nalice@shellpp-test:~$ ");
        let (text, _) = run(&mut shell, "   \r");
        assert_eq!(text, "   \r\nalice@shellpp-test:~$ ");
    }

    #[test]
    fn input_split_across_packets() {
        let mut shell = started("alice");
        let mut text = String::new();
        for chunk in ["ec", "ho ", "x", "y\r"] {
            text.push_str(&run(&mut shell, chunk).0);
        }
        assert_eq!(text, "echo xy\r\nxy\r\nalice@shellpp-test:~$ ");
    }

    #[test]
    fn utf8_input_is_echoed_and_erased_per_character() {
        let mut shell = started("alice");
        let input = "echo жё✓\x7f\r";
        let (text, _) = run(&mut shell, input);
        assert_eq!(text, "echo жё✓\x08 \x08\r\nжё\r\nalice@shellpp-test:~$ ");
        // A UTF-8 character split across two packets.
        let bytes = "echo ж\r".as_bytes();
        let (first, second) = bytes.split_at(6);
        let mut out = Vec::new();
        for action in shell.input(first).into_iter().chain(shell.input(second)) {
            if let Action::Output(chunk) = action {
                out.extend_from_slice(&chunk);
            }
        }
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "echo ж\r\nж\r\nalice@shellpp-test:~$ "
        );
    }

    #[test]
    fn backspace_on_empty_line_does_nothing() {
        let mut shell = started("alice");
        let (text, _) = run(&mut shell, "\x7f\x08");
        assert_eq!(text, "");
        let (text, _) = run(&mut shell, "ab\x08\x08\x08c\r");
        assert_eq!(
            text,
            "ab\x08 \x08\x08 \x08c\r\nc: command not found\r\nalice@shellpp-test:~$ "
        );
    }

    #[test]
    fn ctrl_c_cancels_the_line() {
        let mut shell = started("alice");
        let (text, _) = run(&mut shell, "echo no\x03echo yes\r");
        assert_eq!(
            text,
            "echo no^C\r\nalice@shellpp-test:~$ echo yes\r\nyes\r\nalice@shellpp-test:~$ "
        );
    }

    #[test]
    fn ctrl_d_exits_only_on_an_empty_line() {
        let mut shell = started("alice");
        let (text, rest) = run(&mut shell, "x\x04");
        assert_eq!(text, "x");
        assert!(rest.is_empty());
        let (_, rest) = run(&mut shell, "\x7f\x04");
        assert_eq!(rest, vec![Action::Exit(0)]);
        assert!(shell.is_ended());
        // Nothing happens after the end.
        assert!(shell.input(b"echo late\r").is_empty());
    }

    #[test]
    fn exit_with_and_without_status() {
        let mut shell = started("alice");
        let (text, rest) = run(&mut shell, "exit\r");
        assert_eq!(text, "exit\r\n");
        assert_eq!(rest, vec![Action::Exit(0)]);

        let mut shell = started("alice");
        let (_, rest) = run(&mut shell, "exit 3\r");
        assert_eq!(rest, vec![Action::Exit(3)]);

        let mut shell = started("alice");
        let (text, rest) = run(&mut shell, "exit x\r");
        assert!(text.ends_with("exit: numeric argument required\r\n"));
        assert_eq!(rest, vec![Action::Exit(2)]);
    }

    #[test]
    fn output_before_exit_keeps_its_order() {
        let mut shell = started("alice");
        let actions = shell.input(b"echo bye\rexit 4\rafter\r");
        assert_eq!(
            actions,
            vec![
                Action::Output(b"echo bye\r\nbye\r\nalice@shellpp-test:~$ exit 4\r\n".to_vec()),
                Action::Exit(4),
            ]
        );
    }

    #[test]
    fn escape_sequences_are_ignored() {
        let mut shell = started("alice");
        // Arrows (CSI and SS3), a focus report, an OSC string and bracketed paste.
        let input = "\x1b[A\x1bOB\x1b[I\x1b]0;x\x07\x1b[200~echo pasted\x1b[201~\r";
        let (text, _) = run(&mut shell, input);
        assert_eq!(text, "echo pasted\r\npasted\r\nalice@shellpp-test:~$ ");
        // An escape sequence split across packets.
        let (text, _) = run(&mut shell, "\x1b[1");
        assert_eq!(text, "");
        let (text, _) = run(&mut shell, ";5Decho z\r");
        assert_eq!(text, "echo z\r\nz\r\nalice@shellpp-test:~$ ");
        // OSC terminated by ESC backslash; other control characters dropped.
        let (text, _) = run(&mut shell, "\x1b]2;t\x1b\\\x01\x09echo k\r");
        assert_eq!(text, "echo k\r\nk\r\nalice@shellpp-test:~$ ");
    }

    #[test]
    fn bracketed_paste_with_several_lines_runs_each_line() {
        let mut shell = started("alice");
        let (text, _) = run(&mut shell, "\x1b[200~echo one\recho two\r\x1b[201~");
        assert_eq!(
            text,
            "echo one\r\none\r\nalice@shellpp-test:~$ echo two\r\ntwo\r\nalice@shellpp-test:~$ "
        );
    }

    #[test]
    fn read_secret_turns_echo_off_for_one_line() {
        let mut shell = started("alice");
        let (text, _) = run(&mut shell, "read-secret\r");
        assert_eq!(text, format!("read-secret\r\n{SECRET_PROMPT}"));
        let (text, _) = run(&mut shell, "hunter");
        assert_eq!(text, "");
        let (text, _) = run(&mut shell, "x\x7f2\r");
        assert_eq!(text, "\r\nok\r\nalice@shellpp-test:~$ ");
        // Echo is back on.
        let (text, _) = run(&mut shell, "echo v\r");
        assert_eq!(text, "echo v\r\nv\r\nalice@shellpp-test:~$ ");
    }

    #[test]
    fn read_secret_cancelled_with_ctrl_c() {
        let mut shell = started("alice");
        run(&mut shell, "read-secret\r");
        let (text, _) = run(&mut shell, "abc\x03");
        assert_eq!(text, "^C\r\nalice@shellpp-test:~$ ");
        let (text, _) = run(&mut shell, "x");
        assert_eq!(text, "x");
    }

    #[test]
    fn alt_screen_waits_for_q() {
        let mut shell = started("alice");
        let (text, _) = run(&mut shell, "alt-screen\r");
        assert_eq!(text, "alt-screen\r\n\x1b[?1049hALT SCREEN (press q)");
        let (text, _) = run(&mut shell, "echo x\r\x1b[A");
        assert_eq!(text, "");
        let (text, _) = run(&mut shell, "q");
        assert_eq!(text, "\x1b[?1049lalice@shellpp-test:~$ ");
    }

    #[test]
    fn size_and_term_follow_the_pty() {
        let mut shell = started("alice");
        assert_eq!(
            run(&mut shell, "size\r").0,
            "size\r\n80x24\r\nalice@shellpp-test:~$ "
        );
        assert_eq!(
            run(&mut shell, "term\r").0,
            "term\r\nxterm-256color\r\nalice@shellpp-test:~$ "
        );
        shell.resize(120, 40);
        assert_eq!(
            run(&mut shell, "size\r").0,
            "size\r\n120x40\r\nalice@shellpp-test:~$ "
        );

        let mut no_pty = Shell::new("bob", None, None);
        no_pty.start();
        assert_eq!(
            run(&mut no_pty, "size\r").0,
            "size\r\n0x0\r\nbob@shellpp-test:~$ "
        );
        assert_eq!(
            run(&mut no_pty, "term\r").0,
            "term\r\n\r\nbob@shellpp-test:~$ "
        );
    }

    #[test]
    fn env_prints_the_requested_value() {
        let mut shell = started("alice");
        shell.set_env("LANG", "C.UTF-8");
        shell.set_env("LANG", "en_US.UTF-8");
        assert_eq!(
            run(&mut shell, "env LANG\r").0,
            "env LANG\r\nen_US.UTF-8\r\nalice@shellpp-test:~$ "
        );
        assert_eq!(
            run(&mut shell, "env MISSING\r").0,
            "env MISSING\r\n\r\nalice@shellpp-test:~$ "
        );
    }

    #[test]
    fn seq_prints_numbers_in_chunks_with_a_limit() {
        let mut shell = started("alice");
        assert_eq!(
            run(&mut shell, "seq 3\r").0,
            "seq 3\r\n1\r\n2\r\n3\r\nalice@shellpp-test:~$ "
        );
        assert_eq!(
            run(&mut shell, "seq 9 11\r").0,
            "seq 9 11\r\n9\r\n10\r\n11\r\nalice@shellpp-test:~$ "
        );
        assert_eq!(
            run(&mut shell, "seq 5 4\r").0,
            "seq 5 4\r\nalice@shellpp-test:~$ "
        );
        for bad in [
            "seq\r",
            "seq 0\r",
            "seq x\r",
            "seq 1 2 3\r",
            "seq 1 10000001\r",
        ] {
            assert!(
                run(&mut shell, bad).0.contains("seq: invalid range\r\n"),
                "{bad:?}"
            );
        }
        // 300,000 numbers: every one in order, in chunks of about 64 KiB.
        let actions = shell.input(b"seq 1 300000\r");
        let chunks: Vec<&Vec<u8>> = actions
            .iter()
            .filter_map(|action| match action {
                Action::Output(bytes) => Some(bytes),
                _ => None,
            })
            .collect();
        assert!(chunks.len() > 20, "{} chunks", chunks.len());
        assert!(chunks.iter().all(|chunk| chunk.len() <= SEQ_CHUNK + 32));
        let text: String = chunks
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect();
        let numbers: Vec<&str> = text
            .trim_start_matches("seq 1 300000\r\n")
            .trim_end_matches("alice@shellpp-test:~$ ")
            .split_terminator("\r\n")
            .collect();
        assert_eq!(numbers.len(), 300_000);
        assert_eq!(numbers.first(), Some(&"1"));
        assert_eq!(numbers.last(), Some(&"300000"));
    }

    #[test]
    fn lines_prints_numbered_lines_with_a_limit() {
        let mut shell = started("alice");
        assert_eq!(
            run(&mut shell, "lines 3\r").0,
            "lines 3\r\nline 1\r\nline 2\r\nline 3\r\nalice@shellpp-test:~$ "
        );
        assert!(
            run(&mut shell, "lines 10001\r")
                .0
                .contains("lines: invalid count\r\n")
        );
        assert!(
            run(&mut shell, "lines x\r")
                .0
                .contains("lines: invalid count\r\n")
        );
        let (text, _) = run(&mut shell, "lines 10000\r");
        assert!(text.contains("\r\nline 10000\r\nalice@shellpp-test:~$ "));
        assert_eq!(text.matches("line ").count(), 10_000);
    }

    #[test]
    fn simple_output_commands() {
        let mut shell = started("alice");
        let prompt = "alice@shellpp-test:~$ ";
        assert_eq!(run(&mut shell, "bell\r").0, format!("bell\r\n\x07{prompt}"));
        assert_eq!(
            run(&mut shell, "url\r").0,
            format!("url\r\nhttps://example.com/docs\r\n{prompt}")
        );
        assert_eq!(
            run(&mut shell, "title My Tab\r").0,
            format!("title My Tab\r\n\x1b]0;My Tab\x07{prompt}")
        );
        assert_eq!(
            run(&mut shell, "unicode\r").0,
            format!("unicode\r\nwide: 日本語 ✓ 👍\r\n{prompt}")
        );
        let (text, _) = run(&mut shell, "color\r");
        assert!(text.starts_with("color\r\n\x1b[30m30 \x1b[31m31"));
        assert!(text.contains("\x1b[97m97\x1b[0m\r\n"));
        assert_eq!(text.matches("\x1b[").count(), 17);
        assert_eq!(
            run(&mut shell, "frobnicate --now\r").0,
            format!("frobnicate --now\r\nfrobnicate: command not found\r\n{prompt}")
        );
    }

    #[test]
    fn sleep_buffers_input_until_resumed() {
        let mut shell = started("alice");
        let (text, rest) = run(&mut shell, "sleep 1.5\r");
        assert_eq!(text, "sleep 1.5\r\n");
        assert_eq!(rest, vec![Action::Sleep(Duration::from_millis(1500))]);
        assert!(shell.input(b"echo queued\r").is_empty());
        let (text, rest) = split(shell.resume(None));
        assert_eq!(
            text,
            "alice@shellpp-test:~$ echo queued\r\nqueued\r\nalice@shellpp-test:~$ "
        );
        assert!(rest.is_empty());
        // A stale resume is ignored.
        assert!(shell.resume(None).is_empty());
        // Invalid delays.
        assert!(
            run(&mut shell, "sleep 61\r")
                .0
                .contains("sleep: invalid time\r\n")
        );
        assert!(
            run(&mut shell, "sleep -1\r")
                .0
                .contains("sleep: invalid time\r\n")
        );
        assert!(
            run(&mut shell, "sleep NaN\r")
                .0
                .contains("sleep: invalid time\r\n")
        );
    }

    #[test]
    fn ctrl_c_interrupts_sleep() {
        let mut shell = started("alice");
        run(&mut shell, "sleep 30\r");
        let (text, rest) = run(&mut shell, "lost\x03echo after\r");
        assert_eq!(
            text,
            "^C\r\nalice@shellpp-test:~$ echo after\r\nafter\r\nalice@shellpp-test:~$ "
        );
        assert_eq!(rest, vec![Action::CancelBusy]);
        assert!(shell.resume(None).is_empty());
    }

    #[test]
    fn later_prints_the_prompt_at_once_and_the_text_afterwards() {
        let mut shell = started("alice");
        let (text, rest) = run(&mut shell, "later 2 marker text\r");
        assert_eq!(text, "later 2 marker text\r\nalice@shellpp-test:~$ ");
        assert_eq!(
            rest,
            vec![Action::Later(
                Duration::from_secs(2),
                "marker text".to_owned()
            )]
        );
        run(&mut shell, "ec");
        let (text, _) = split(shell.later("marker text"));
        assert_eq!(text, "\r\nmarker text\r\nalice@shellpp-test:~$ ec");
        assert!(
            run(&mut shell, "\x03later x y\r")
                .0
                .contains("later: invalid time\r\n")
        );
    }

    #[test]
    fn agent_keys_waits_for_the_answer() {
        let mut shell = started("alice");
        let (text, rest) = run(&mut shell, "agent-keys\r");
        assert_eq!(text, "agent-keys\r\n");
        assert_eq!(rest, vec![Action::AgentKeys]);
        let (text, _) = split(shell.resume(Some("agent keys: 2")));
        assert_eq!(text, "agent keys: 2\r\nalice@shellpp-test:~$ ");
    }

    #[test]
    fn drop_ends_the_shell() {
        let mut shell = started("alice");
        let (_, rest) = run(&mut shell, "drop\r");
        assert_eq!(rest, vec![Action::Drop]);
        assert!(shell.is_ended());
        assert!(shell.later("x").is_empty());
    }
}
