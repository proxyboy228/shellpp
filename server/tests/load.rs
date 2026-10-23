//! The load test (plan run 7, section 4): 10 000 records, the first sync
//! and a repeated one, with time and memory for the report. Runs with
//! `SHELLPP_SYNC_LOAD=1` (best with `--release`); otherwise it returns at
//! once, like the SFTP measurement (`SHELLPP_SFTP_BENCH=1`).

mod support;

use std::time::Instant;

use serde_json::json;
use shellpp_core::service::SignInOutcome;
use shellpp_core::service::dto::RecordType;
use support::*;

const RECORDS: usize = 10_000;

/// The working set of this process in KiB (Windows `tasklist`), if known.
fn memory_kib() -> Option<u64> {
    let pid = std::process::id();
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let field = text.split("\",\"").last()?;
    let digits: String = field.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[test]
fn ten_thousand_records_first_and_repeated_sync() {
    if std::env::var("SHELLPP_SYNC_LOAD").as_deref() != Ok("1") {
        return;
    }
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    let started = Instant::now();
    for i in 0..RECORDS {
        a.save(
            None,
            RecordType::Host,
            json!({"address": format!("h{i}.example.com"), "label": format!("host {i}")}),
        );
    }
    let created = started.elapsed();

    let started = Instant::now();
    let status = a.register(&server, &email, &password);
    let first_push = started.elapsed();
    assert_eq!(status.pending, 0, "everything went up in the first cycle");

    let b = Client::empty("B");
    let started = Instant::now();
    assert!(matches!(
        b.sign_in(&server, &email, &password),
        SignInOutcome::Done(_)
    ));
    let first_pull = started.elapsed();
    assert_eq!(b.views(&[RecordType::Host]).len(), RECORDS);

    let started = Instant::now();
    let report = b.sync();
    let repeated = started.elapsed();
    // At most the echo of B's own `device_info` push.
    assert!(report.pulled <= 1, "nothing new: {}", report.pulled);

    let ids: Vec<_> = a
        .views(&[RecordType::Host])
        .iter()
        .take(100)
        .map(|view| view.record_id())
        .collect();
    for id in &ids {
        let mut data = a.data(*id);
        data["label"] = json!("changed");
        a.save(Some(*id), RecordType::Host, data);
    }
    let started = Instant::now();
    a.sync();
    let report = b.sync();
    let hundred = started.elapsed();
    assert!(report.pulled >= 100, "the hundred edits: {}", report.pulled);

    let memory = memory_kib();
    println!(
        "LOAD records={RECORDS} create={:.1}s first_push={:.1}s first_pull={:.1}s repeat={:.2}s hundred_edits={:.2}s memory={}",
        created.as_secs_f64(),
        first_push.as_secs_f64(),
        first_pull.as_secs_f64(),
        repeated.as_secs_f64(),
        hundred.as_secs_f64(),
        memory.map_or("unknown".to_owned(), |kib| format!("{} MiB", kib / 1024)),
    );
}
