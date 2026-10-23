//! Imports from elsewhere that would cost too much or act on the user's
//! behalf (run 8, C5; D171, D172): an Import Data file of tiny values or of
//! endless broken records, an `~/.ssh/config` whose includes fan out or
//! read one file under two names, a config of more hosts than the budget,
//! a key "file" that is not a regular file, and a host from a file that
//! would forward the user's agent.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{first_run, reasons, save};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use shellpp_core::ValidationReason;
use shellpp_core::import::ssh_config;
use shellpp_core::service::dto::{
    DataExportInput, DataImportInput, KeyFileStatus, RecordType, SshConfigSource,
};

/// The limits of Import Data (service/export.rs, private there).
const MAX_IMPORT_RECORDS: usize = 500_000;
const MAX_IMPORT_VALUES: usize = 8_000_000;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A plain export of an empty profile with `records` in its vault.
fn export_with(records: &str) -> String {
    let u = first_run();
    let plain = u.core.data_export(DataExportInput::default()).unwrap();
    let mut document: Value = serde_json::from_str(plain.text.expose_secret()).unwrap();
    document["vaults"][0]["records"] = json!([]);
    document
        .to_string()
        .replace(r#""records":[]"#, &format!(r#""records":{records}"#))
}

#[test]
fn an_import_of_tiny_values_or_endless_broken_records_is_refused() {
    let target = first_run();
    let attempt = |text: String| {
        target
            .core
            .data_import(DataImportInput::new(SecretString::from(text), None))
    };
    // More values than the limit, counted before the file is parsed.
    let zeros = vec!["0"; MAX_IMPORT_VALUES + 1].join(",");
    let started = Instant::now();
    assert_eq!(
        reasons(attempt(export_with(&format!("[{zeros}]"))).unwrap_err()),
        vec![("text".to_owned(), ValidationReason::TooMany)]
    );
    // Generous: the refusal comes from the count, not from parsing.
    assert!(started.elapsed() < Duration::from_secs(60));
    // Broken records count against the record limit like good ones, so the
    // list of refusals stays bounded.
    let broken = vec!["{}"; MAX_IMPORT_RECORDS + 1].join(",");
    assert_eq!(
        reasons(attempt(export_with(&format!("[{broken}]"))).unwrap_err()),
        vec![("text".to_owned(), ValidationReason::TooMany)]
    );
    // A few broken records are still refused one by one.
    let summary = attempt(export_with("[{},{},0]")).unwrap();
    assert_eq!(summary.refused.len(), 3);
}

#[test]
fn a_host_from_a_file_does_not_forward_the_agent() {
    let source = first_run();
    let host = save(
        &source.core,
        source.vault_id,
        None,
        RecordType::Host,
        json!({"label": "jump", "address": "192.0.2.7",
               "ssh": {"agent_forwarding": true, "agent_forward_system": true}}),
        &[],
    )
    .unwrap();
    let plain = source.core.data_export(DataExportInput::default()).unwrap();
    let forwarding = |core: &shellpp_core::Core| {
        let export = core.data_export(DataExportInput::default()).unwrap();
        let document: Value = serde_json::from_str(export.text.expose_secret()).unwrap();
        document["vaults"][0]["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["record_id"] == host.record_id().to_string())
            .map(|record| record["data"]["ssh"]["agent_forwarding"].clone())
    };

    // Into another profile: the host comes without Agent Forwarding.
    let target = first_run();
    let summary = target
        .core
        .data_import(DataImportInput::new(plain.text.clone(), None))
        .unwrap();
    assert_eq!(summary.hosts_agent_forwarding_off, 1);
    assert_eq!(forwarding(&target.core), Some(json!(false)));

    // The same file back into the profile it came from: the host is the
    // same here and stays as the user set it.
    let again = source
        .core
        .data_import(DataImportInput::new(plain.text.clone(), None))
        .unwrap();
    assert_eq!(again.hosts_agent_forwarding_off, 0);
    assert_eq!(forwarding(&source.core), Some(json!(true)));

    // A full restore of the user's own backup keeps it.
    let restored = first_run();
    let summary = restored
        .core
        .data_import(DataImportInput {
            full_restore: true,
            ..DataImportInput::new(plain.text, None)
        })
        .unwrap();
    assert_eq!(summary.hosts_agent_forwarding_off, 0);
    assert_eq!(forwarding(&restored.core), Some(json!(true)));
}

/// A config in a temporary home folder: `~/.ssh/config` and other files.
fn config(files: &[(&str, &str)]) -> (tempfile::TempDir, SshConfigSource) {
    let home = tempfile::tempdir().unwrap();
    for (name, text) in files {
        write(&home.path().join(".ssh").join(name), text);
    }
    let source = SshConfigSource {
        path: home.path().join(".ssh").join("config"),
        home: home.path().to_path_buf(),
    };
    (home, source)
}

#[test]
fn includes_that_fan_out_are_bounded() {
    let u = first_run();
    // `config` names `b` a thousand times; `b` names two thousand missing
    // files. Each file is read once, the arguments and problems are capped.
    let many_b = vec!["b"; 1000].join(" ");
    let missing: Vec<String> = (0..2000).map(|n| format!("missing-{n}")).collect();
    let (_home, source) = config(&[
        (
            "config",
            &format!("Include {many_b}\nHost one\n  HostName 192.0.2.1\n"),
        ),
        ("b", &format!("Include {}\n", missing.join(" "))),
    ]);
    let started = Instant::now();
    let preview = u.core.ssh_config_preview(source).unwrap();
    assert!(started.elapsed() < Duration::from_secs(60));
    assert!(
        preview.problems.len() <= ssh_config::MAX_PROBLEMS + 1,
        "{}",
        preview.problems.len()
    );
    assert!(
        preview
            .problems
            .iter()
            .any(|problem| problem.contains("too large")),
        "{:?}",
        preview.problems.iter().take(5).collect::<Vec<_>>()
    );
}

#[test]
fn a_file_included_under_another_name_is_read_once() {
    let u = first_run();
    let (_home, source) = config(&[(
        "config",
        "Include ../.ssh/config\nHost one\n  HostName 192.0.2.1\n",
    )]);
    let preview = u.core.ssh_config_preview(source).unwrap();
    assert!(
        preview
            .problems
            .iter()
            .any(|problem| problem.contains("loops")),
        "{:?}",
        preview.problems
    );
    assert_eq!(preview.hosts.len(), 1, "the host once, not twice");
}

#[test]
fn a_config_of_too_many_hosts_is_cut_to_the_budget() {
    let u = first_run();
    let text: String = (0..20_000)
        .map(|n| format!("Host h{n}\n  HostName 192.0.2.{}\n", n % 250 + 1))
        .collect();
    let (_home, source) = config(&[("config", &text)]);
    let started = Instant::now();
    let preview = u.core.ssh_config_preview(source).unwrap();
    // Generous: without the limits this takes hours.
    assert!(started.elapsed() < Duration::from_secs(60));
    // The blocks stop at their limit; the hosts at the matching budget.
    assert_eq!(
        preview.hosts.len(),
        ssh_config::MAX_MATCH_WORK / ssh_config::MAX_BLOCKS
    );
    assert!(
        preview
            .problems
            .iter()
            .any(|problem| problem.contains("too large"))
    );
}

#[test]
fn a_key_file_that_is_not_a_regular_file_is_not_read() {
    let u = first_run();
    let (home, source) = config(&[(
        "config",
        "Host one\n  HostName 192.0.2.1\n  IdentityFile ~/.ssh/keys\n",
    )]);
    std::fs::create_dir_all(home.path().join(".ssh").join("keys")).unwrap();
    let preview = u.core.ssh_config_preview(source).unwrap();
    assert_eq!(
        preview.keys[0].status,
        KeyFileStatus::Invalid("the file cannot be read".to_owned())
    );
}

/// A FIFO without a writer would block an open forever.
#[cfg(unix)]
#[test]
fn a_key_file_that_is_a_fifo_does_not_block() {
    let u = first_run();
    let (home, source) = config(&[(
        "config",
        "Host one\n  HostName 192.0.2.1\n  IdentityFile ~/.ssh/fifo\n",
    )]);
    let fifo = home.path().join(".ssh").join("fifo");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let preview = u.core.ssh_config_preview(source).unwrap();
    assert_eq!(
        preview.keys[0].status,
        KeyFileStatus::Invalid("the file cannot be read".to_owned())
    );
}
