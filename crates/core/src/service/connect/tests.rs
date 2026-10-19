//! In-crate tests of the stage 2 facade that need the private session:
//! lazy decryption of the `history` and `connection_log` classes (D10) and
//! the connection log rules at unlock (docs/data-model.md, section 13.3).

use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_sync_api::classes;
use tempfile::TempDir;
use uuid::Uuid;

use crate::model::types::RecordKind;
use crate::platform::OsKind;
use crate::platform::memory::MemoryPlatform;
use crate::service::dto::{HistoryCandidate, HistoryListInput};
use crate::service::host_api::{
    ConnectionEndReason, ConnectionLogStart, HistoryDecision, HistoryRecordInput,
};
use crate::service::records::parts;
use crate::service::{Core, CoreConfig};

struct Fixture {
    dir: TempDir,
    mem: MemoryPlatform,
    password: SecretString,
}

impl Fixture {
    fn new() -> (Self, Core) {
        let fixture = Self {
            dir: tempfile::tempdir().unwrap(),
            mem: MemoryPlatform::new(),
            password: SecretString::from(format!(
                "Pw-{}-x",
                crate::crypto::random_array::<12>()
                    .unwrap()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )),
        };
        let core = fixture.open();
        core.first_run_setup(fixture.password.clone()).unwrap();
        (fixture, core)
    }

    /// A new core on the same data directory (a new run of the app), locked.
    fn open(&self) -> Core {
        Core::open(
            CoreConfig {
                data_dir: self.dir.path().join("data"),
                app_version: "0.1.0".into(),
            },
            self.mem.platform(OsKind::Linux, "TEST"),
        )
        .unwrap()
    }
}

fn record(core: &Core, command: &str) -> HistoryDecision {
    core.history_record(HistoryRecordInput {
        host: None,
        session_id: Uuid::now_v7(),
        candidate: HistoryCandidate {
            command: command.into(),
            prompt_text: "$".into(),
            typed_printable: true,
            alt_screen: false,
        },
        injected: false,
        record_history: true,
    })
    .unwrap()
}

fn start(core: &Core) -> Uuid {
    core.connection_log_start(ConnectionLogStart {
        host: None,
        host_label: None,
        address: "192.0.2.80".into(),
        port: 22,
        username: None,
        os: None,
    })
    .unwrap()
}

/// (lazy classes loaded, rows of lazy classes decrypted, indexed records of `kind`).
fn lazy_state(core: &Core, kind: RecordKind) -> (Vec<u8>, usize, usize) {
    let inner = core.inner();
    let session = inner.session.as_ref().expect("unlocked");
    let mut loaded = session.lazy_loaded.clone();
    loaded.sort_unstable();
    (
        loaded,
        session.lazy_opened,
        session.index.of_kind(kind).count(),
    )
}

/// The data of an indexed connection log entry.
fn log_data(core: &Core, id: &Uuid) -> serde_json::Map<String, Value> {
    let inner = core.inner();
    let session = inner.session.as_ref().expect("unlocked");
    session
        .index
        .get_kind(id, RecordKind::ConnectionLogEntry)
        .expect("entry")
        .data()
        .clone()
}

#[test]
fn history_and_connection_log_rows_are_not_decrypted_at_unlock() {
    let (fixture, core) = Fixture::new();
    // First run: nothing to load, both classes count as loaded.
    assert_eq!(
        lazy_state(&core, RecordKind::HistoryEntry).0,
        vec![classes::HISTORY, classes::CONNECTION_LOG]
    );
    for n in 0..40 {
        assert_eq!(
            record(&core, &format!("echo {n}")),
            HistoryDecision::Recorded
        );
    }
    let log = start(&core);
    core.connection_log_end(log, ConnectionEndReason::Closed, None);

    core.lock().unwrap();
    core.unlock(fixture.password.clone()).unwrap();
    assert_eq!(lazy_state(&core, RecordKind::HistoryEntry), (vec![], 0, 0));
    assert_eq!(lazy_state(&core, RecordKind::ConnectionLogEntry).2, 0);
    // Recording a command writes without decrypting the class.
    assert_eq!(record(&core, "echo new"), HistoryDecision::Recorded);
    assert_eq!(lazy_state(&core, RecordKind::HistoryEntry).1, 0);

    // The History tab decrypts the class on first use.
    let listed = core
        .history_list(HistoryListInput {
            limit: Some(5000),
            ..HistoryListInput::default()
        })
        .unwrap();
    assert_eq!(listed.len(), 41);
    assert_eq!(listed[0].command, "echo new");
    let (loaded, opened, indexed) = lazy_state(&core, RecordKind::HistoryEntry);
    assert_eq!(loaded, vec![classes::HISTORY]);
    assert_eq!(opened, 41);
    assert_eq!(indexed, 41);
    // A second use does not decrypt again.
    core.history_list(HistoryListInput::default()).unwrap();
    assert_eq!(lazy_state(&core, RecordKind::HistoryEntry).1, 41);

    // Recent connections load the connection log.
    assert_eq!(core.recent_connections().unwrap().len(), 1);
    let (loaded, opened, indexed) = lazy_state(&core, RecordKind::ConnectionLogEntry);
    assert_eq!(loaded, vec![classes::HISTORY, classes::CONNECTION_LOG]);
    assert_eq!(opened, 42);
    assert_eq!(indexed, 1);

    // A new run of the app: the same.
    drop(core);
    let core = fixture.open();
    core.unlock(fixture.password.clone()).unwrap();
    assert_eq!(lazy_state(&core, RecordKind::HistoryEntry), (vec![], 0, 0));
}

#[test]
fn entries_left_open_by_an_earlier_run_get_unknown() {
    let (fixture, core) = Fixture::new();
    let crashed = start(&core);
    let ended = start(&core);
    core.connection_log_end(ended, ConnectionEndReason::ExitStatus, Some(2));
    // An entry of another device that is still open: not this device's to close.
    let foreign = start(&core);
    {
        let now = core.now();
        let mut guard = core.inner();
        let mut parts = parts(&mut guard).unwrap();
        let mut data = parts
            .session
            .index
            .get_kind(&foreign, RecordKind::ConnectionLogEntry)
            .unwrap()
            .data()
            .clone();
        data.insert("device_id".into(), json!(Uuid::now_v7()));
        core.write_record(
            &mut parts,
            RecordKind::ConnectionLogEntry,
            foreign,
            data,
            now,
        )
        .unwrap();
        parts.connlog.ended(&foreign);
    }
    // The app stops without ending `crashed`.
    core.on_exit();
    drop(core);

    let core = fixture.open();
    core.unlock(fixture.password.clone()).unwrap();
    // Nothing pending: the class stays lazy until it is used.
    assert!(
        lazy_state(&core, RecordKind::ConnectionLogEntry)
            .0
            .is_empty()
    );
    let still_running = start(&core);
    core.recent_connections().unwrap();
    let data = log_data(&core, &crashed);
    assert_eq!(data["end_reason"], json!("unknown"));
    assert!(!data.contains_key("ended_at"));
    let data = log_data(&core, &ended);
    assert_eq!(data["end_reason"], json!("exit_status"));
    assert_eq!(data["exit_status"], json!(2));
    let data = log_data(&core, &foreign);
    assert!(!data.contains_key("end_reason"));
    let data = log_data(&core, &still_running);
    assert!(!data.contains_key("end_reason"));

    // Entries of this run survive a lock while their session runs.
    core.lock().unwrap();
    core.unlock(fixture.password.clone()).unwrap();
    core.recent_connections().unwrap();
    assert!(!log_data(&core, &still_running).contains_key("end_reason"));
    core.connection_log_end(still_running, ConnectionEndReason::Closed, None);
    let data = log_data(&core, &still_running);
    assert_eq!(data["end_reason"], json!("closed"));
    assert!(data.contains_key("ended_at"));
}

#[test]
fn ends_held_while_locked_are_written_at_unlock() {
    let (fixture, core) = Fixture::new();
    let log = start(&core);
    core.lock().unwrap();
    core.connection_log_end(log, ConnectionEndReason::Dropped, None);
    assert!(core.inner().connlog.has_pending());
    core.unlock(fixture.password.clone()).unwrap();
    // Pending ends load the class at once.
    assert!(
        lazy_state(&core, RecordKind::ConnectionLogEntry)
            .0
            .contains(&classes::CONNECTION_LOG)
    );
    assert!(!core.inner().connlog.has_pending());
    let data = log_data(&core, &log);
    assert_eq!(data["end_reason"], json!("dropped"));
    assert!(data.contains_key("ended_at"));
    assert!(!data.contains_key("exit_status"));
}
