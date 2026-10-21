//! The History tab's "save as snippets" hint through the facade (UI 4.2;
//! docs/data-model.md, sections 13.1 and 13.2): the three most frequent
//! commands of a host without secret-looking ones, "Close" remembered per
//! host across restarts, and the lock guard.

mod common;

use common::first_run;
use serde_json::json;
use shellpp_core::service::dto::{HistoryCandidate, HistoryClearInput, RecordRef};
use shellpp_core::service::host_api::{HistoryDecision, HistoryRecordInput};
use shellpp_core::{Core, CoreError};
use uuid::Uuid;

fn record(core: &Core, host: RecordRef, command: &str) -> HistoryDecision {
    core.history_record(HistoryRecordInput {
        host: Some(host),
        // A new session each time: the duplicate rule is per session.
        session_id: Uuid::now_v7(),
        candidate: HistoryCandidate {
            command: command.to_owned(),
            prompt_text: "deploy@web:~$".to_owned(),
            typed_printable: true,
            alt_screen: false,
        },
        injected: false,
        record_history: true,
    })
    .unwrap()
}

#[test]
fn the_hint_offers_the_most_frequent_commands_without_secrets() {
    let u = first_run();
    let web = u.host(json!({"address": "web.example.com"}));
    let db = u.host(json!({"address": "db.example.com"}));
    let web = RecordRef {
        vault_id: u.vault_id,
        record_id: web.record_id,
    };
    let db = RecordRef {
        vault_id: u.vault_id,
        record_id: db.record_id,
    };
    assert!(u.core.history_hint(web).unwrap().commands.is_empty());
    for (command, times) in [
        ("uptime", 1),
        ("df -h", 3),
        ("systemctl status nginx", 2),
        ("journalctl -u nginx -n 50", 2),
    ] {
        for _ in 0..times {
            assert_eq!(record(&u.core, web, command), HistoryDecision::Recorded);
        }
    }
    for _ in 0..5 {
        record(&u.core, db, "psql");
    }
    let hint = u.core.history_hint(web).unwrap();
    assert!(!hint.dismissed);
    assert_eq!(hint.commands[0], "df -h");
    assert_eq!(hint.commands.len(), 3);
    // Equal counts: the command used last comes first.
    assert_eq!(
        &hint.commands[1..],
        ["journalctl -u nginx -n 50", "systemctl status nginx"]
    );
    assert_eq!(u.core.history_hint(db).unwrap().commands, ["psql"]);
    // Clearing the host's history empties its hint.
    u.core
        .history_clear(HistoryClearInput { host: Some(db) })
        .unwrap();
    assert!(u.core.history_hint(db).unwrap().commands.is_empty());
}

#[test]
fn close_is_remembered_per_host_across_restarts() {
    let mut u = first_run();
    let web = u.host(json!({"address": "web.example.com"}));
    let db = u.host(json!({"address": "db.example.com"}));
    let web = RecordRef {
        vault_id: u.vault_id,
        record_id: web.record_id,
    };
    let db = RecordRef {
        vault_id: u.vault_id,
        record_id: db.record_id,
    };
    record(&u.core, web, "ls");
    u.core.history_hint_dismiss(web).unwrap();
    // Twice is fine and writes nothing new.
    u.core.history_hint_dismiss(web).unwrap();
    assert!(u.core.history_hint(web).unwrap().dismissed);
    assert!(!u.core.history_hint(db).unwrap().dismissed);
    // The commands are still computed (the History list keeps working).
    assert_eq!(u.core.history_hint(web).unwrap().commands, ["ls"]);

    u.reopen();
    assert!(matches!(
        u.core.history_hint(web).unwrap_err(),
        CoreError::Locked
    ));
    assert!(matches!(
        u.core.history_hint_dismiss(db).unwrap_err(),
        CoreError::Locked
    ));
    u.core.unlock(u.password.clone()).unwrap();
    assert!(u.core.history_hint(web).unwrap().dismissed);
    assert!(!u.core.history_hint(db).unwrap().dismissed);
}

/// "Save" makes snippets of the (edited) commands that target the host,
/// leaves out what looks like a secret, and the hint is not shown again
/// (stage 5). The Shell History search lists every host's entries, or
/// those of the hosts asked for, each with its host.
#[test]
fn save_makes_snippets_and_the_search_filters_by_host() {
    let u = first_run();
    let web = u.host(json!({"address": "web.example.com"}));
    let db = u.host(json!({"address": "db.example.com"}));
    let web = RecordRef {
        vault_id: u.vault_id,
        record_id: web.record_id,
    };
    let db = RecordRef {
        vault_id: u.vault_id,
        record_id: db.record_id,
    };
    record(&u.core, web, "df -h");
    record(&u.core, db, "psql");
    let saved = u
        .core
        .history_hint_save(
            web,
            &[
                "df -h /var".to_owned(),
                "export TOKEN=ghp_0123456789abcdefghijklmnopqrstuvwxyzAB".to_owned(),
                "  ".to_owned(),
            ],
        )
        .unwrap();
    assert_eq!(saved.len(), 1, "the secret and the blank line are left out");
    assert_eq!(saved[0].script, "df -h /var");
    assert_eq!(saved[0].target_host_ids, [web.record_id]);
    assert!(u.core.history_hint(web).unwrap().dismissed);

    use shellpp_core::service::dto::HistorySearchInput;
    let all = u
        .core
        .history_search(HistorySearchInput::default())
        .unwrap();
    assert_eq!(all.len(), 2);
    let only_db = u
        .core
        .history_search(HistorySearchInput {
            hosts: vec![db],
            ..HistorySearchInput::default()
        })
        .unwrap();
    assert_eq!(only_db.len(), 1);
    assert_eq!(only_db[0].command, "psql");
    assert_eq!(only_db[0].host, Some(db));
}
