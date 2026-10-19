//! Helpers shared by the W2 integration tests (core flows, records, canary).
//!
//! Test data is fictional: documentation addresses only, passwords generated
//! at run time.

#![allow(dead_code)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};
use shellpp_core::platform::OsKind;
use shellpp_core::platform::memory::MemoryPlatform;
use shellpp_core::service::dto::{
    CoreEvent, CoreEventKind, GroupView, HostView, QueryScope, RecordSaveInput, RecordType,
    RecordView, RecordsQuery, SortMode,
};
use shellpp_core::{Core, CoreConfig, CoreError, ValidationReason};
use tempfile::TempDir;
use uuid::Uuid;

pub const APP_VERSION: &str = "0.1.0";

/// Random lowercase hex from the OS CSPRNG.
pub fn random_hex() -> String {
    let bytes = shellpp_core::crypto::random_array::<12>().expect("random");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A master password generated at run time (16+ characters).
pub fn new_password() -> SecretString {
    SecretString::from(format!("Pw-{}-x", random_hex()))
}

/// A temporary data directory and an in-memory platform.
pub struct Env {
    pub dir: TempDir,
    pub mem: MemoryPlatform,
}

impl Env {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
            mem: MemoryPlatform::new(),
        }
    }

    pub fn config(&self) -> CoreConfig {
        CoreConfig {
            data_dir: self.dir.path().join("data"),
            app_version: APP_VERSION.to_owned(),
        }
    }

    /// Opens (or reopens) the core on this data directory.
    pub fn open(&self) -> Core {
        Core::open(self.config(), self.mem.platform(OsKind::Windows, "TEST-PC"))
            .expect("core opens")
    }

    pub fn take_events(&self) -> Vec<CoreEvent> {
        self.mem.events.take()
    }
}

/// A core after the first run.
pub struct Unlocked {
    pub env: Env,
    pub core: Core,
    pub password: SecretString,
    pub recovery: String,
    pub vault_id: Uuid,
}

impl Unlocked {
    /// Drops the core and opens it again on the same data directory (locked).
    pub fn reopen(&mut self) {
        let fresh = self.env.open();
        self.core = fresh;
    }

    pub fn host(&self, data: Value) -> HostView {
        match save(&self.core, self.vault_id, None, RecordType::Host, data, &[])
            .expect("host saved")
        {
            RecordView::Host(view) => view,
            RecordView::Group(_) => panic!("expected a host"),
        }
    }

    pub fn host_with_password(&self, data: Value, password: &str) -> HostView {
        match save(
            &self.core,
            self.vault_id,
            None,
            RecordType::Host,
            data,
            &[("ssh.password", Some(password))],
        )
        .expect("host saved")
        {
            RecordView::Host(view) => view,
            RecordView::Group(_) => panic!("expected a host"),
        }
    }

    pub fn group(&self, data: Value) -> GroupView {
        match save(
            &self.core,
            self.vault_id,
            None,
            RecordType::Group,
            data,
            &[],
        )
        .expect("group saved")
        {
            RecordView::Group(view) => view,
            RecordView::Host(_) => panic!("expected a group"),
        }
    }

    /// Ids returned by a query.
    pub fn query(&self, types: &[RecordType], scope: QueryScope, sort: SortMode) -> Vec<Uuid> {
        ids(&self
            .core
            .records_query(RecordsQuery {
                vault_id: None,
                types: types.to_vec(),
                scope,
                text: None,
                tags: Vec::new(),
                sort,
            })
            .expect("query"))
    }
}

/// Runs the first run with a generated password.
pub fn first_run() -> Unlocked {
    let env = Env::new();
    let core = env.open();
    let password = new_password();
    let recovery = core
        .first_run_setup(password.clone())
        .expect("first run")
        .recovery_key
        .expose_secret()
        .to_owned();
    let vault_id = core.vaults().expect("vaults")[0].vault_id;
    env.take_events();
    Unlocked {
        env,
        core,
        password,
        recovery,
        vault_id,
    }
}

pub fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object")
}

/// `record_save` with plain secrets.
pub fn save(
    core: &Core,
    vault_id: Uuid,
    record_id: Option<Uuid>,
    record_type: RecordType,
    data: Value,
    secrets: &[(&str, Option<&str>)],
) -> Result<RecordView, CoreError> {
    let secrets: BTreeMap<String, Option<SecretString>> = secrets
        .iter()
        .map(|(path, value)| ((*path).to_owned(), value.map(SecretString::from)))
        .collect();
    core.record_save(RecordSaveInput {
        vault_id,
        record_id,
        record_type,
        data: map(data),
        secrets,
    })
}

/// The `(field, reason)` pairs of a validation error.
pub fn reasons(error: CoreError) -> Vec<(String, ValidationReason)> {
    match error {
        CoreError::Validation(errors) => errors.into_iter().map(|e| (e.field, e.reason)).collect(),
        other => panic!("expected validation_failed, got {}", other.code()),
    }
}

pub fn ids(views: &[RecordView]) -> Vec<Uuid> {
    views.iter().map(RecordView::record_id).collect()
}

/// Event names in order.
pub fn names(events: &[CoreEvent]) -> Vec<&'static str> {
    events.iter().map(CoreEvent::name).collect()
}

/// The record ids of the `records:changed` events.
pub fn changed_ids(events: &[CoreEvent]) -> Vec<Uuid> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            CoreEventKind::RecordsChanged { record_ids, .. } => Some(record_ids.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The host view of a record view.
pub fn as_host(view: RecordView) -> HostView {
    match view {
        RecordView::Host(view) => view,
        RecordView::Group(_) => panic!("expected a host"),
    }
}

pub fn as_group(view: RecordView) -> GroupView {
    match view {
        RecordView::Group(view) => view,
        RecordView::Host(_) => panic!("expected a group"),
    }
}
