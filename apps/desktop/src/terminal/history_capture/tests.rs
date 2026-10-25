//! The owner's sequences (stage 2b, step 2) through the real core into a
//! temporary profile; the capture's own tests, against a recording sink,
//! are in `shellpp-terminal`.
//!
//! Every password below is fictional.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use shellpp_core::Core;
use shellpp_core::service::dto::{
    HistoryCandidate, HistoryListInput, RecordRef, RecordSaveInput, RecordType, RecordView,
    SettingsSetInput,
};
use shellpp_core::service::host_api::HistoryRecordInput;
use uuid::Uuid;

use super::HistorySink;
use crate::terminal::colors::{FromTheme as _, Palette};
use crate::terminal::model::{TerminalHandle, TerminalSink, TerminalSize};
use crate::terminal::paste;
use crate::theme::terminal::builtin;

const PROMPT: &str = "alice@web:~$ ";

#[derive(Default)]
struct Sent(Mutex<Vec<u8>>);

impl TerminalSink for Sent {
    fn send(&self, bytes: Vec<u8>) {
        self.0.lock().unwrap().extend_from_slice(&bytes);
    }

    fn resize(&self, _size: TerminalSize) {}
}

/// A terminal at a shell prompt, fed the way an SSH channel delivers.
struct Shell {
    handle: TerminalHandle,
    _woken: async_channel::Receiver<()>,
}

impl Shell {
    fn at_prompt() -> Self {
        let (handle, woken) = TerminalHandle::headless(80, 24, Palette::from_theme(&builtin()[0]));
        handle.set_sink(Some(Arc::new(Sent::default())));
        let shell = Self {
            handle,
            _woken: woken,
        };
        shell.output(PROMPT);
        shell
    }

    fn output(&self, text: &str) {
        self.handle.feed_blocking(text.as_bytes()).unwrap();
        self.handle.flush_blocking();
    }

    fn keys(&self, text: &str) {
        for c in text.chars() {
            self.handle.send_typed(c.to_string().into_bytes());
        }
    }

    fn type_echoed(&self, text: &str) {
        for c in text.chars() {
            self.keys(&c.to_string());
            self.output(&c.to_string());
        }
    }

    fn enter(&self) {
        self.handle.send_typed(b"\r".to_vec());
    }

    fn run(&self, command: &str, output: &str) {
        self.type_echoed(command);
        self.enter();
        self.output("\r\n");
        self.output(output);
        self.output(PROMPT);
    }
}

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
