//! The history capture on a real terminal model: the user's keys go through
//! [`TerminalHandle::send_typed`] (what the view does), and the server's
//! answer, echo included, is fed as bytes, the way an SSH channel delivers
//! it. The owner's sequences (stage 2b, step 2) run here end to end, first
//! against a recording sink, then through the real core into a temporary
//! profile.
//!
//! Every password and token below is fictional or built at run time.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use alacritty_terminal::vte::ansi::CursorShape;
use serde_json::json;
use shellpp_core::Core;
use shellpp_core::service::dto::{
    HistoryListInput, RecordRef, RecordSaveInput, RecordType, RecordView, SettingsSetInput,
};
use shellpp_core::service::host_api::HistoryRecordInput;
use uuid::Uuid;

use super::*;
use crate::terminal::colors::Palette;
use crate::terminal::model::{ModelOptions, TerminalSink, TerminalSize};
use crate::theme::terminal::builtin;

/// The candidates the capture produced.
#[derive(Default)]
struct Recorded(Mutex<Vec<HistoryCandidate>>);

impl HistorySink for Recorded {
    fn record(&self, candidate: HistoryCandidate) {
        self.0.lock().unwrap().push(candidate);
    }
}

/// What reached the session.
#[derive(Default)]
struct Sent(Mutex<Vec<u8>>);

impl TerminalSink for Sent {
    fn send(&self, bytes: Vec<u8>) {
        self.0.lock().unwrap().extend_from_slice(&bytes);
    }

    fn resize(&self, _size: TerminalSize) {}
}

/// A terminal with a server on the other side, driven by hand.
struct Shell {
    handle: TerminalHandle,
    history: Arc<Recorded>,
    sent: Arc<Sent>,
    _woken: async_channel::Receiver<()>,
}

const PROMPT: &str = "alice@web:~$ ";

impl Shell {
    fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let (wake, woken) = async_channel::bounded(1);
        let handle = TerminalHandle::spawn(
            ModelOptions {
                size: TerminalSize::cells(cols, rows),
                scrollback,
                cursor_shape: CursorShape::Block,
            },
            Palette::from_theme(&builtin()[0]),
            wake,
        );
        let sent = Arc::new(Sent::default());
        handle.set_sink(Some(sent.clone()));
        let history = Arc::new(Recorded::default());
        handle.set_history_sink(Some(history.clone()));
        Self {
            handle,
            history,
            sent,
            _woken: woken,
        }
    }

    /// A shell at its first prompt (80x24).
    fn at_prompt() -> Self {
        let shell = Self::new(80, 24, 1000);
        shell.output(PROMPT);
        shell
    }

    /// Bytes from the server, parsed before this returns.
    fn output(&self, text: &str) {
        self.handle.feed_blocking(text.as_bytes()).unwrap();
        self.handle.flush_blocking();
    }

    /// The user types `text`, one key per write, with no echo (yet).
    fn keys(&self, text: &str) {
        for c in text.chars() {
            self.handle.send_typed(c.to_string().into_bytes());
        }
    }

    /// The user types `text` and the server echoes every key at once.
    fn type_echoed(&self, text: &str) {
        for c in text.chars() {
            self.keys(&c.to_string());
            self.output(&c.to_string());
        }
    }

    fn enter(&self) {
        self.handle.send_typed(b"\r".to_vec());
    }

    /// A command line at a shell prompt: typed with echo, Enter, the line
    /// feed of the shell, the output, the next prompt.
    fn run(&self, command: &str, output: &str) {
        self.type_echoed(command);
        self.enter();
        self.output("\r\n");
        self.output(output);
        self.output(PROMPT);
    }

    fn commands(&self) -> Vec<String> {
        self.history
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|candidate| candidate.command.clone())
            .collect()
    }

    fn candidates(&self) -> Vec<HistoryCandidate> {
        self.history.0.lock().unwrap().clone()
    }

    /// Nothing the capture holds or produced contains `needle`.
    fn assert_never_seen(&self, needle: &str) {
        for candidate in self.candidates() {
            assert!(!candidate.command.contains(needle));
            assert!(!candidate.prompt_text.contains(needle));
        }
    }
}

// ----- basics ------------------------------------------------------------------

#[test]
fn a_command_is_the_screen_text_after_the_prompt() {
    let shell = Shell::at_prompt();
    shell.run("ls -la", "total 0\r\n");
    let candidates = shell.candidates();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].command, "ls -la");
    assert_eq!(candidates[0].prompt_text, "alice@web:~$");
    assert!(candidates[0].typed_printable);
    assert!(!candidates[0].alt_screen);
    assert!(shell.handle.outbox().capture.is_idle());
    // The keys still reached the session.
    assert_eq!(&*shell.sent.0.lock().unwrap(), b"ls -la\r");
}

#[test]
fn the_echo_of_keys_typed_just_before_enter_is_waited_for() {
    let shell = Shell::at_prompt();
    // A slow link: the whole line and Enter are typed before any echo.
    shell.keys("uptime");
    shell.enter();
    shell.output("upt");
    shell.output("ime\r\n 10:00:00 up 1 day\r\n");
    shell.output(PROMPT);
    assert_eq!(shell.commands(), ["uptime"]);
}

#[test]
fn the_line_ends_when_the_cursor_moves_to_another_row() {
    let shell = Shell::at_prompt();
    shell.type_echoed("whoami");
    shell.enter();
    // A terminal renderer (ConPTY) moves the cursor instead of a line feed.
    shell.output("\x1b[3;1Halice\x1b[4;1H");
    assert_eq!(shell.commands(), ["whoami"]);
}

#[test]
fn a_wrapped_command_is_joined_and_follows_scrolling() {
    // 20 columns, 4 rows, a small and full scrollback: rows scroll away
    // while the command is typed and after it.
    let shell = Shell::new(20, 4, 10);
    for n in 0..30 {
        shell.output(&format!("old {n}\r\n"));
    }
    shell.output(PROMPT);
    let command = "echo one two three four five six seven";
    shell.run(command, "one two three four five six seven\r\n");
    assert_eq!(shell.commands(), [command]);
    // Ctrl+L redraws the prompt and the line at the top.
    shell.type_echoed("df");
    shell.keys("\u{c}");
    shell.output(&format!("\x1b[H\x1b[2J{PROMPT}df"));
    shell.type_echoed(" -h");
    shell.enter();
    shell.output("\r\nFilesystem\r\n");
    assert_eq!(shell.commands(), [command, "df -h"]);
}

#[test]
fn history_recall_and_tab_completion_record_the_final_line() {
    let shell = Shell::at_prompt();
    shell.run("cat notes.txt", "");
    // Up arrow: the shell shows the previous command.
    shell.handle.send_typed(b"\x1b[A".to_vec());
    shell.output("cat notes.txt");
    shell.enter();
    shell.output("\r\n");
    // Tab completion adds what was not typed.
    shell.output(PROMPT);
    shell.type_echoed("cd Doc");
    shell.handle.send_typed(b"\t".to_vec());
    shell.output("uments/");
    shell.enter();
    shell.output("\r\n");
    assert_eq!(
        shell.commands(),
        ["cat notes.txt", "cat notes.txt", "cd Documents/"]
    );
}

#[test]
fn ctrl_c_abandons_the_line() {
    let shell = Shell::at_prompt();
    shell.type_echoed("rm -rf build");
    shell.keys("\u{3}");
    shell.output("^C\r\n");
    shell.output(PROMPT);
    shell.run("ls", "");
    assert_eq!(shell.commands(), ["ls"]);
}

#[test]
fn nothing_is_captured_without_a_sink() {
    let shell = Shell::at_prompt();
    shell.handle.set_history_sink(None);
    shell.run("ls", "");
    assert!(shell.commands().is_empty());
    assert!(!shell.handle.outbox().capture.watching());
}

// ----- the owner's sequences (stage 2b, step 2) --------------------------------

#[test]
fn su_password_without_echo_is_not_recorded() {
    let shell = Shell::at_prompt();
    shell.type_echoed("su");
    shell.enter();
    shell.output("\r\n");
    shell.output("Password: ");
    // Echo is off: the server shows nothing while the password is typed.
    shell.keys("Tr0ub4dor-horse");
    shell.enter();
    shell.output("\r\nroot@web:/home/alice# ");
    assert_eq!(shell.commands(), ["su"]);
    shell.assert_never_seen("Tr0ub4dor");
    assert!(shell.handle.outbox().capture.is_idle());
    // The next command at the root prompt is recorded again.
    shell.type_echoed("id");
    shell.enter();
    shell.output("\r\nuid=0(root)\r\n");
    assert_eq!(shell.commands(), ["su", "id"]);
}

#[test]
fn a_bracketed_paste_of_a_password_is_not_recorded_and_no_marker_is_kept() {
    let shell = Shell::at_prompt();
    // Bash turns bracketed paste on at its prompt and off at Enter.
    shell.output("\x1b[?2004h");
    shell.type_echoed("su");
    shell.enter();
    shell.output("\x1b[?2004l\r\r\n");
    shell.output("Password: ");
    // A program that left bracketed paste on: the paste arrives with the
    // markers, and nothing is echoed.
    shell.output("\x1b[?2004h");
    let bracketed = shell
        .handle
        .term()
        .lock()
        .mode()
        .contains(TermMode::BRACKETED_PASTE);
    assert!(bracketed);
    shell
        .handle
        .send_typed(paste::paste_bytes("Pasted-pw-5", bracketed));
    shell.enter();
    shell.output("\r\n");
    // A server that does not know bracketed paste echoes the markers in
    // caret notation, with the password, after a password prompt.
    shell.output("\x1b[?2004lPassword: ");
    shell
        .handle
        .send_typed(paste::paste_bytes("Pasted-pw-6", true));
    shell.output("^[[200~Pasted-pw-6^[[201~");
    shell.enter();
    shell.output("\r\n");
    shell.output(PROMPT);
    // A command pasted the same way is recorded without the markers.
    shell
        .handle
        .send_typed(paste::paste_bytes("git status", true));
    shell.output("^[[200~git status^[[201~");
    shell.enter();
    shell.output("\r\n");
    assert_eq!(shell.commands(), ["su", "git status"]);
    shell.assert_never_seen("Pasted-pw");
    for marker in [
        "\x1b[200~",
        "\x1b[201~",
        "^[[200~",
        "^[[201~",
        "200~",
        "\x1b",
    ] {
        shell.assert_never_seen(marker);
    }
    // The paste reached the session with its markers (that is the protocol);
    // the capture keeps nothing of it.
    assert!(shell.handle.outbox().capture.is_idle());
}

#[test]
fn sudo_s_with_its_password_echoed_is_not_recorded() {
    let shell = Shell::at_prompt();
    shell.type_echoed("sudo -S apt update");
    shell.enter();
    shell.output("\r\n[sudo] password for alice: ");
    // `sudo -S` reads standard input: the terminal echoes the password.
    shell.type_echoed("Sudo-pw-1");
    shell.enter();
    shell.output("\r\nHit:1 http://deb.example.com stable InRelease\r\n");
    shell.output(PROMPT);
    assert_eq!(shell.commands(), ["sudo -S apt update"]);
    shell.assert_never_seen("Sudo-pw-1");
}

#[test]
fn passwd_prompts_are_not_recorded() {
    let shell = Shell::at_prompt();
    shell.type_echoed("passwd");
    shell.enter();
    shell.output("\r\nChanging password for alice.\r\nCurrent password: ");
    shell.keys("Old-pw-1");
    shell.enter();
    shell.output("\r\nNew password: ");
    shell.keys("New-pw-2");
    shell.enter();
    shell.output("\r\nRetype new password: ");
    shell.keys("New-pw-2");
    shell.enter();
    shell.output("\r\npasswd: password updated successfully\r\n");
    shell.output(PROMPT);
    assert_eq!(shell.commands(), ["passwd"]);
    shell.assert_never_seen("-pw-");
}

#[test]
fn ssh_keygen_passphrase_prompts_are_not_recorded() {
    let shell = Shell::at_prompt();
    shell.type_echoed("ssh-keygen -t ed25519");
    shell.enter();
    shell.output(
        "\r\nGenerating public/private ed25519 key pair.\r\n\
         Enter file in which to save the key (/home/alice/.ssh/id_ed25519): ",
    );
    // Enter alone takes the default file.
    shell.enter();
    shell.output("\r\nEnter passphrase (empty for no passphrase): ");
    shell.keys("Key-phrase-1");
    shell.enter();
    shell.output("\r\nEnter same passphrase again: ");
    shell.keys("Key-phrase-1");
    shell.enter();
    shell.output("\r\nYour identification has been saved\r\n");
    shell.output(PROMPT);
    assert_eq!(shell.commands(), ["ssh-keygen -t ed25519"]);
    shell.assert_never_seen("Key-phrase");
}

#[test]
fn mysql_password_prompt_and_password_argument() {
    let shell = Shell::at_prompt();
    shell.type_echoed("mysql -u root -p");
    shell.enter();
    shell.output("\r\nEnter password: ");
    shell.keys("Db-pw-1");
    shell.enter();
    shell.output("\r\nWelcome to the MySQL monitor.\r\nmysql> ");
    shell.type_echoed("show databases;");
    shell.enter();
    shell.output("\r\n+----------+\r\nmysql> ");
    shell.type_echoed("exit");
    shell.enter();
    shell.output("\r\nBye\r\n");
    shell.output(PROMPT);
    // `-p<value>` on the command line (docs/security.md, S1).
    shell.run(
        "mysql -u root -pDb-pw-2 app",
        "mysql: [Warning] Using a password\r\n",
    );
    assert_eq!(
        shell.commands(),
        ["mysql -u root -p", "show databases;", "exit"]
    );
    shell.assert_never_seen("Db-pw");
}

#[test]
fn a_password_typed_before_the_prompt_appears_is_not_recorded() {
    let shell = Shell::at_prompt();
    // The password is typed while the command's own line feed is on its way:
    // neither the command (its line had not ended on screen) nor the
    // password (its boundary is unknown) is recorded.
    shell.type_echoed("sudo apt update");
    shell.enter();
    shell.keys("Ahead-pw-1");
    shell.enter();
    shell.output("\r\n[sudo] password for alice: ");
    shell.output("\r\n");
    shell.output(PROMPT);
    // The line feed came first, the prompt after the first key: the prompt
    // is inside the captured line.
    shell.type_echoed("sudo -v");
    shell.enter();
    shell.output("\r\n");
    shell.keys("Ahead-pw-2");
    shell.output("[sudo] password for alice: ");
    shell.enter();
    shell.output("\r\n");
    shell.output(PROMPT);
    assert_eq!(shell.commands(), ["sudo -v"]);
    shell.assert_never_seen("Ahead-pw");
    shell.assert_never_seen("[sudo]");
}

#[test]
fn a_password_typed_before_su_shows_its_prompt_is_not_recorded() {
    let shell = Shell::at_prompt();
    // The password is typed after `su`'s line feed, before its prompt:
    // `Password:` follows the echo on the same row, with no word boundary
    // between them.
    shell.type_echoed("su -");
    shell.enter();
    shell.output("\r\n");
    shell.type_echoed("Ahead-pw-3");
    shell.output("Password: ");
    shell.enter();
    shell.output("\r\n");
    assert_eq!(shell.commands(), ["su -"]);
    shell.assert_never_seen("Ahead-pw");

    // The whole password and its Enter before `su` turned echo off (a slow
    // PAM): the password is echoed on a row of its own, and `Password:`
    // comes on the next row, after that line ended.
    shell.output(PROMPT);
    shell.type_echoed("su -");
    shell.enter();
    shell.output("\r\n");
    shell.type_echoed("Ahead-pw-4");
    shell.enter();
    shell.output("\r\nPassword: ");
    shell.keys("x");
    shell.enter();
    shell.output("\r\nsu: Authentication failure\r\n");
    shell.output(PROMPT);
    // The same, with the prompt in a later piece of output.
    shell.type_echoed("su -");
    shell.enter();
    shell.output("\r\n");
    shell.type_echoed("Ahead-pw-5");
    shell.enter();
    shell.output("\r\n");
    shell.output("\x1b[0mPass");
    shell.output("word: ");
    shell.keys("y");
    shell.enter();
    shell.output("\r\nsu: Authentication failure\r\n");
    shell.output(PROMPT);
    shell.run("id", "uid=1000(alice)\r\n");
    assert_eq!(shell.commands(), ["su -", "su -", "su -", "id"]);
    shell.assert_never_seen("Ahead-pw");
    assert!(shell.handle.outbox().capture.is_idle());
}

#[test]
fn a_line_without_a_prompt_is_held_until_the_next_key() {
    // A shell with an empty prompt: nothing stands before the boundary.
    let shell = Shell::new(80, 24, 1000);
    shell.type_echoed("uptime");
    shell.enter();
    shell.output("\r\n 10:00 up 1 day\r\n");
    assert!(shell.commands().is_empty());
    assert!(shell.handle.outbox().capture.holds_a_line());
    assert!(shell.handle.outbox().capture.watching());
    // The next key lets it go.
    shell.type_echoed("date");
    assert_eq!(shell.commands(), ["uptime"]);
    shell.enter();
    shell.output("\r\nTue Oct 20\r\n");
    assert_eq!(shell.commands(), ["uptime"]);
    // Without a key, the first output after the timeout lets it go.
    {
        let mut state = shell.handle.outbox().capture.lock();
        let held = state.held.as_mut().expect("held");
        held.since -= PENDING_TIMEOUT + Duration::from_secs(1);
    }
    shell.output("\r\n");
    assert_eq!(shell.commands(), ["uptime", "date"]);
    assert!(shell.handle.outbox().capture.is_idle());
    assert!(!shell.handle.outbox().capture.watching());
    // A full-screen program after it drops it (fail closed).
    shell.type_echoed("top");
    shell.enter();
    shell.output("\r\n\x1b[?1049h");
    shell.output("top - 10:00:00\r\n");
    shell.keys("q");
    shell.output("\x1b[?1049l");
    assert_eq!(shell.commands(), ["uptime", "date"]);
    // Removing the sink forgets a held line.
    shell.type_echoed("df");
    shell.enter();
    shell.output("\r\nFilesystem\r\n");
    assert!(shell.handle.outbox().capture.holds_a_line());
    shell.handle.set_history_sink(None);
    assert!(shell.handle.outbox().capture.is_idle());
    assert_eq!(shell.commands(), ["uptime", "date"]);
}

#[test]
fn masked_echo_is_not_recorded() {
    let shell = Shell::at_prompt();
    shell.type_echoed("vault login");
    shell.enter();
    shell.output("\r\nToken (will be hidden): ");
    shell.keys("abc123");
    shell.output("******");
    shell.enter();
    shell.output("\r\n");
    shell.output("Enter code: ");
    shell.keys("123456");
    shell.output("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}");
    shell.enter();
    shell.output("\r\n");
    assert_eq!(shell.commands(), ["vault login"]);
}

#[test]
fn secret_looking_lines_are_never_recorded() {
    let shell = Shell::at_prompt();
    let token = format!("ghp_{}", "aB3dE6gH9jK2mN5pQ8sT1vW4yZ7bC0eF3hJ6");
    for command in [
        format!("export GITHUB_TOKEN={token}"),
        format!("gh auth login --with-token {token}"),
        "curl -H 'Authorization: Bearer abc.def' https://api.example.com/".to_owned(),
        "git clone https://bob:Clone-pw-1@git.example.com/repo.git".to_owned(),
        "echo q7Xv2LmR9tKp4WzN8sBeYc3Hd".to_owned(),
        "psql --password app".to_owned(),
    ] {
        shell.run(&command, "");
    }
    shell.run("git pull", "Already up to date.\r\n");
    assert_eq!(shell.commands(), ["git pull"]);
    shell.assert_never_seen("ghp_");
}

#[test]
fn the_alternate_screen_is_never_recorded() {
    let shell = Shell::at_prompt();
    shell.type_echoed("vim notes.txt");
    shell.enter();
    shell.output("\r\n\x1b[?1049h\x1b[H\x1b[2J~\r\n~\r\n");
    shell.type_echoed(":wq");
    shell.enter();
    shell.output("\x1b[?1049l");
    shell.output(PROMPT);
    // Typing started on the normal screen, a full-screen program took over
    // before Enter.
    shell.type_echoed("top");
    shell.output("\x1b[?1049h");
    shell.enter();
    shell.output("\x1b[?1049l\r\n");
    shell.output(PROMPT);
    assert_eq!(shell.commands(), ["vim notes.txt"]);
}

#[test]
fn a_leading_space_is_not_recorded() {
    let shell = Shell::at_prompt();
    shell.run(" history -c", "");
    shell.run("hostname", "web\r\n");
    assert_eq!(shell.commands(), ["hostname"]);
}

#[test]
fn injected_input_is_never_recorded() {
    let shell = Shell::at_prompt();
    // A History insert: injected, echoed by the shell, run with a typed Enter.
    shell.handle.inject_input(b"git status".to_vec());
    shell.output("git status");
    shell.enter();
    shell.output("\r\nOn branch main\r\n");
    shell.output(PROMPT);
    // Typed text after an insert on the same line: still injected.
    shell.type_echoed("echo ");
    shell.handle.inject_input(b"hello".to_vec());
    shell.output("hello");
    shell.enter();
    shell.output("\r\nhello\r\n");
    shell.output(PROMPT);
    // An injected Enter (a snippet run, later stages).
    shell.type_echoed("make");
    shell.handle.inject_input(b"\r".to_vec());
    shell.output("\r\n");
    shell.output(PROMPT);
    shell.run("uptime", "");
    assert_eq!(shell.commands(), ["uptime"]);
    // The injected text reached the session.
    let sent = String::from_utf8(shell.sent.0.lock().unwrap().clone()).unwrap();
    assert!(sent.starts_with("git status\r"));
}

#[test]
fn type_ahead_and_multi_line_pastes_record_only_lines_with_a_known_boundary() {
    let shell = Shell::at_prompt();
    // Unbracketed paste of two lines: the first line had not ended on
    // screen when the second began, and the second line's boundary is
    // unknown; neither is recorded.
    shell.handle.send_typed(b"ls\rpwd\r".to_vec());
    shell.output("ls\r\nfile\r\n");
    shell.output(PROMPT);
    shell.output("pwd\r\n/home/alice\r\n");
    shell.output(PROMPT);
    // A bracketed paste with a line break keeps the shell's line open.
    shell.output("\x1b[?2004h");
    shell
        .handle
        .send_typed(paste::paste_bytes("echo a\necho b", true));
    shell.output("echo a\r\n> echo b");
    shell.enter();
    shell.output("\r\na\r\nb\r\n");
    shell.output(PROMPT);
    shell.run("date", "");
    // Keys typed quickly on a slow link: the next command begins before the
    // previous one's line feed arrived.
    shell.type_echoed("uptime");
    shell.enter();
    shell.keys("df");
    shell.output("\r\n 10:00 up 1 day\r\n");
    shell.output(PROMPT);
    shell.output("df");
    shell.enter();
    shell.output("\r\nFilesystem\r\n");
    shell.output(PROMPT);
    shell.run("whoami", "alice\r\n");
    assert_eq!(shell.commands(), ["date", "whoami"]);
}

#[test]
fn a_line_that_never_ends_on_screen_is_dropped() {
    let shell = Shell::at_prompt();
    // `read -s`: no echo and no line feed; the next prompt follows on the
    // same row, and the next command is typed there.
    shell.type_echoed("read -s value");
    shell.enter();
    shell.output("\r\n");
    shell.keys("Silent-1");
    shell.enter();
    shell.output(PROMPT);
    assert_eq!(shell.commands(), ["read -s value"]);
    assert!(shell.handle.outbox().capture.watching());
    // The first key of the next command drops the pending line (its text
    // would mix in the prompt and the new echo); the new line's boundary is
    // unknown, so it is dropped too.
    shell.run("ls", "");
    assert!(!shell.handle.outbox().capture.watching());
    shell.run("pwd", "/home/alice\r\n");
    assert_eq!(shell.commands(), ["read -s value", "pwd"]);
    shell.assert_never_seen("Silent");
    // No command took in the prompt that followed on the same row.
    assert!(
        shell
            .commands()
            .iter()
            .all(|command| !command.contains("alice@web"))
    );

    // Without any key, a pending line ends after the timeout: the next
    // output only expires it.
    shell.type_echoed("cat");
    shell.enter();
    assert!(shell.handle.outbox().capture.watching());
    {
        let mut state = shell.handle.outbox().capture.lock();
        let pending = state.pending.as_mut().expect("pending");
        pending.since -= PENDING_TIMEOUT + Duration::from_secs(1);
    }
    shell.output("\r\n");
    assert!(!shell.handle.outbox().capture.watching());
    assert_eq!(shell.commands(), ["read -s value", "pwd"]);
}

#[test]
fn a_key_typed_in_a_full_screen_program_does_not_hide_the_next_command() {
    let shell = Shell::at_prompt();
    shell.run("less notes.txt", "\x1b[?1049hnotes\r\n");
    // `q` quits the pager: no Enter, the line it started belongs to the
    // full-screen program.
    shell.keys("q");
    shell.output("\x1b[?1049l");
    shell.output(PROMPT);
    shell.run("ls", "");
    assert_eq!(shell.commands(), ["less notes.txt", "ls"]);
}

#[test]
fn typed_keys_are_classified_without_being_kept() {
    use Key::*;
    assert_eq!(keys(b"ab"), [Printable, Printable]);
    assert_eq!(keys("ж✓".as_bytes()), [Printable, Printable]);
    assert_eq!(keys(b"\r\n"), [Enter, Enter]);
    assert_eq!(keys(b"\x03\x7f\t\x15"), [Interrupt, Other, Other, Other]);
    assert_eq!(
        keys(b"\x1b[A\x1bOA\x1b[1;5C\x1b[3~\x1bb\x1b"),
        [Other, Other, Other, Other, Other, Other]
    );
    assert_eq!(
        keys(b"\x1b[200~pw\x1b[201~"),
        [PasteStart, Printable, Printable, PasteEnd]
    );
}

// ----- through the core ----------------------------------------------------------

/// Records through the core, like the session layer does (synchronously).
struct CoreSink {
    core: Arc<Core>,
    host: RecordRef,
    session: Uuid,
}

impl HistorySink for CoreSink {
    fn record(&self, candidate: HistoryCandidate) {
        self.core
            .history_record(HistoryRecordInput {
                host: Some(self.host),
                session_id: self.session,
                candidate,
                injected: false,
                record_history: true,
            })
            .unwrap();
    }
}

#[test]
fn the_owner_sequences_through_the_core() {
    let opened = crate::testing::TestProfile::new().into_unlocked();
    let core = opened.core.clone();
    let vault_id = core.vaults().unwrap()[0].vault_id;
    let RecordView::Host(host) = core
        .record_save(RecordSaveInput {
            vault_id,
            record_id: None,
            record_type: RecordType::Host,
            data: json!({"address": "web.example.com"})
                .as_object()
                .unwrap()
                .clone(),
            secrets: BTreeMap::new(),
        })
        .unwrap()
    else {
        panic!("a host");
    };
    let host = RecordRef {
        vault_id,
        record_id: host.record_id,
    };
    let shell = Shell::at_prompt();
    shell.handle.set_history_sink(Some(Arc::new(CoreSink {
        core: core.clone(),
        host,
        session: Uuid::now_v7(),
    })));

    shell.output("\x1b[?2004h");
    shell.type_echoed("su");
    shell.enter();
    shell.output("\x1b[?2004l\r\r\nPassword: ");
    shell.keys("Core-pw-1");
    shell.enter();
    shell.output("\r\nsu: Authentication failure\r\n");
    shell.output(PROMPT);
    shell.output("\x1b[?2004h");
    shell.handle.send_typed(paste::paste_bytes("ls -la", true));
    shell.output("^[[200~ls -la^[[201~");
    shell.enter();
    shell.output("\x1b[?2004l\r\r\ntotal 0\r\n");
    shell.output(PROMPT);
    shell.run("mysql -pCore-pw-2", "");
    shell.run(" ls", "");
    shell.handle.inject_input(b"whoami".to_vec());
    shell.output("whoami");
    shell.enter();
    shell.output("\r\nalice\r\n");
    shell.output(PROMPT);
    shell.run("vim x", "\x1b[?1049h");
    shell.type_echoed(":q");
    shell.enter();
    shell.output("\x1b[?1049l");
    shell.output(PROMPT);

    // "Record command history" off (Settings > Terminal).
    let mut values = BTreeMap::new();
    values.insert("terminal.record_history".to_owned(), json!(false));
    core.settings_set(SettingsSetInput {
        values,
        reset: Vec::new(),
    })
    .unwrap();
    shell.run("hostname", "web\r\n");

    let stored: Vec<String> = core
        .history_list(HistoryListInput {
            host: Some(host),
            text: None,
            limit: None,
        })
        .unwrap()
        .into_iter()
        .map(|entry| entry.command)
        .collect();
    // Newest first.
    assert_eq!(stored, ["vim x", "ls -la", "su"]);
    for command in &stored {
        assert!(!command.contains("Core-pw"));
        assert!(!command.contains('\u{1b}'));
        assert!(!command.contains("200~") && !command.contains("201~"));
    }
    // The hint never offers secret-looking entries (none were stored), nor
    // trivial ones such as `ls -la` (D64).
    let hint = core.history_hint(host).unwrap();
    assert_eq!(hint.commands, ["vim x", "su"]);
}
