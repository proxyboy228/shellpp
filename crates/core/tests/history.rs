//! Command history through the facade (D16; docs/security.md, section 10;
//! stage 2 plan, section 4.4): drop reasons in order, the per-session
//! duplicate rule, listing, deletion and clearing. The rules themselves have
//! their table tests in `src/history/tests.rs`.

mod common;

use common::first_run;
use serde_json::json;
use shellpp_core::service::dto::{
    CoreEventKind, HistoryCandidate, HistoryClearInput, HistoryListInput, RecordRef, RecordsRef,
};
use shellpp_core::service::host_api::{HistoryDecision, HistoryDropReason, HistoryRecordInput};
use shellpp_core::{Core, CoreError};
use uuid::Uuid;

fn candidate(command: &str) -> HistoryCandidate {
    HistoryCandidate {
        command: command.to_owned(),
        prompt_text: "deploy@web:~$".to_owned(),
        typed_printable: true,
        alt_screen: false,
    }
}

fn record(core: &Core, host: Option<RecordRef>, session: Uuid, command: &str) -> HistoryDecision {
    core.history_record(HistoryRecordInput {
        host,
        session_id: session,
        candidate: candidate(command),
        injected: false,
        record_history: true,
    })
    .unwrap()
}

fn list(
    core: &Core,
    host: Option<RecordRef>,
    text: Option<&str>,
    limit: Option<u32>,
) -> Vec<String> {
    core.history_list(HistoryListInput {
        host,
        text: text.map(str::to_owned),
        limit,
    })
    .unwrap()
    .into_iter()
    .map(|entry| entry.command)
    .collect()
}

#[test]
fn drop_reasons_through_the_facade() {
    let u = first_run();
    let session = Uuid::now_v7();
    let base = |command: &str| HistoryRecordInput {
        host: None,
        session_id: session,
        candidate: candidate(command),
        injected: false,
        record_history: true,
    };
    let decide = |input: HistoryRecordInput| u.core.history_record(input).unwrap();
    use HistoryDropReason::*;
    assert_eq!(
        decide(HistoryRecordInput {
            record_history: false,
            ..base("ls")
        }),
        HistoryDecision::Dropped(Off)
    );
    assert_eq!(
        decide(HistoryRecordInput {
            injected: true,
            ..base("ls")
        }),
        HistoryDecision::Dropped(Injected)
    );
    let mut alt = base("vim notes");
    alt.candidate.alt_screen = true;
    assert_eq!(decide(alt), HistoryDecision::Dropped(AltScreen));
    assert_eq!(decide(base("****")), HistoryDecision::Dropped(EchoOff));
    let mut prompt = base("hunter2");
    prompt.candidate.prompt_text = "[sudo] password for test: ".into();
    assert_eq!(decide(prompt), HistoryDecision::Dropped(PasswordPrompt));
    let mut empty = base("");
    empty.candidate.typed_printable = false;
    assert_eq!(decide(empty), HistoryDecision::Dropped(Empty));
    assert_eq!(decide(base(" ls")), HistoryDecision::Dropped(LeadingSpace));
    assert_eq!(
        decide(base(&"y".repeat(4097))),
        HistoryDecision::Dropped(TooLong)
    );
    assert_eq!(
        decide(base("export GITHUB_TOKEN=abc")),
        HistoryDecision::Dropped(Secret)
    );
    // Nothing above was written.
    assert!(list(&u.core, None, None, None).is_empty());

    u.env.take_events();
    assert_eq!(decide(base("git status")), HistoryDecision::Recorded);
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![6]
    )));
    // The same command again in the same session: duplicate.
    assert_eq!(
        decide(base("git status")),
        HistoryDecision::Dropped(Duplicate)
    );
    // Bracketed paste markers are cleaned before the comparison.
    assert_eq!(
        decide(base("\u{1b}[200~git status\u{1b}[201~")),
        HistoryDecision::Dropped(Duplicate)
    );
    // Another session may record it.
    assert_eq!(
        record(&u.core, None, Uuid::now_v7(), "git status"),
        HistoryDecision::Recorded
    );
    // After another command, the first one is no longer the previous one.
    assert_eq!(decide(base("ls")), HistoryDecision::Recorded);
    assert_eq!(decide(base("git status")), HistoryDecision::Recorded);
    assert_eq!(list(&u.core, None, None, None).len(), 4);

    // Locked: dropped with `Locked`, not an error.
    u.core.lock().unwrap();
    assert_eq!(decide(base("pwd")), HistoryDecision::Dropped(Locked));
    assert!(matches!(
        u.core
            .history_list(HistoryListInput::default())
            .unwrap_err(),
        CoreError::Locked
    ));
    // The duplicate memory is wiped by the lock.
    u.core.unlock(u.password.clone()).unwrap();
    assert_eq!(decide(base("git status")), HistoryDecision::Recorded);
}

/// Recording follows the host's override and the setting as they are when
/// the command runs, not the copy a session took when it opened (security.md
/// 10.1 rule 4).
#[test]
fn recording_follows_the_current_host_and_setting() {
    use shellpp_core::service::dto::{RecordType, SettingsSetInput};
    let u = first_run();
    let host = u.host(json!({"address": "rec.example.com"}));
    let link = RecordRef {
        vault_id: u.vault_id,
        record_id: host.record_id,
    };
    let session = Uuid::now_v7();
    assert!(u.core.record_history_now(Some(link)).unwrap());
    assert_eq!(
        record(&u.core, Some(link), session, "uptime"),
        HistoryDecision::Recorded
    );
    // The host is set to "Off" while the session runs (its copy still says on).
    common::save(
        &u.core,
        u.vault_id,
        Some(host.record_id),
        RecordType::Host,
        json!({"address": "rec.example.com", "record_history": "off"}),
        &[],
    )
    .unwrap();
    assert!(!u.core.record_history_now(Some(link)).unwrap());
    assert_eq!(
        record(&u.core, Some(link), session, "whoami"),
        HistoryDecision::Dropped(HistoryDropReason::Off)
    );
    // Sessions without a host follow the setting.
    assert!(u.core.record_history_now(None).unwrap());
    let set = |value: bool| {
        let mut values = std::collections::BTreeMap::new();
        values.insert("terminal.record_history".to_owned(), json!(value));
        u.core
            .settings_set(SettingsSetInput {
                values,
                reset: Vec::new(),
            })
            .unwrap();
    };
    set(false);
    assert!(!u.core.record_history_now(None).unwrap());
    assert_eq!(
        record(&u.core, None, Uuid::now_v7(), "hostname"),
        HistoryDecision::Dropped(HistoryDropReason::Off)
    );
    // A host override "on" wins over the setting.
    common::save(
        &u.core,
        u.vault_id,
        Some(host.record_id),
        RecordType::Host,
        json!({"address": "rec.example.com", "record_history": "on"}),
        &[],
    )
    .unwrap();
    assert!(u.core.record_history_now(Some(link)).unwrap());
    assert_eq!(
        record(&u.core, Some(link), session, "date"),
        HistoryDecision::Recorded
    );
    set(true);
    assert_eq!(
        record(&u.core, None, Uuid::now_v7(), "hostname"),
        HistoryDecision::Recorded
    );
    u.core.lock().unwrap();
    assert!(matches!(
        u.core.record_history_now(None).unwrap_err(),
        CoreError::Locked
    ));
}

/// A session whose saved host was removed while it runs records nothing
/// more, whatever the global setting says: its entries would show in no
/// History tab (stage 2b decision: history only for saved hosts).
#[test]
fn a_removed_host_records_nothing() {
    let u = first_run();
    let host = u.host(json!({"address": "gone.example.com", "record_history": "off"}));
    let link = RecordRef {
        vault_id: u.vault_id,
        record_id: host.record_id,
    };
    assert!(!u.core.record_history_now(Some(link)).unwrap());
    u.core
        .record_delete(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![host.record_id],
        })
        .unwrap();
    // The global setting is on (default), the host is gone.
    assert!(u.core.record_history_now(None).unwrap());
    assert!(!u.core.record_history_now(Some(link)).unwrap());
    assert_eq!(
        record(&u.core, Some(link), Uuid::now_v7(), "uptime"),
        HistoryDecision::Dropped(HistoryDropReason::Off)
    );
    assert!(list(&u.core, Some(link), None, None).is_empty());
    assert!(list(&u.core, None, None, None).is_empty());
}

#[test]
fn list_filters_order_and_limits() {
    let u = first_run();
    let web = u.host(json!({"address": "web.example.com"}));
    let db = u.host(json!({"address": "db.example.com"}));
    let web_ref = Some(RecordRef {
        vault_id: u.vault_id,
        record_id: web.record_id,
    });
    let db_ref = Some(RecordRef {
        vault_id: u.vault_id,
        record_id: db.record_id,
    });
    let session = Uuid::now_v7();
    for command in ["uptime", "df -h", "Docker PS", "tail -f /var/log/syslog"] {
        assert_eq!(
            record(&u.core, web_ref, session, command),
            HistoryDecision::Recorded
        );
    }
    record(&u.core, db_ref, Uuid::now_v7(), "psql");
    record(&u.core, None, Uuid::now_v7(), "dir");

    // Newest first, per host link; entries without a host are their own list.
    assert_eq!(
        list(&u.core, web_ref, None, None),
        vec!["tail -f /var/log/syslog", "Docker PS", "df -h", "uptime"]
    );
    assert_eq!(list(&u.core, db_ref, None, None), vec!["psql"]);
    assert_eq!(list(&u.core, None, None, None), vec!["dir"]);
    // Case-insensitive substring filter; blank filter = no filter.
    assert_eq!(
        list(&u.core, web_ref, Some("docker"), None),
        vec!["Docker PS"]
    );
    assert_eq!(list(&u.core, web_ref, Some("  "), None).len(), 4);
    assert!(list(&u.core, web_ref, Some("psql"), None).is_empty());
    // Limits: as given, at most 5000.
    assert_eq!(list(&u.core, web_ref, None, Some(2)).len(), 2);
    assert_eq!(list(&u.core, web_ref, None, Some(100_000)).len(), 4);
    assert!(list(&u.core, web_ref, None, Some(0)).is_empty());

    let entries = u
        .core
        .history_list(HistoryListInput {
            host: web_ref,
            ..HistoryListInput::default()
        })
        .unwrap();
    assert!(entries.iter().all(|entry| entry.vault_id == u.vault_id));
    assert!(
        entries
            .windows(2)
            .all(|pair| pair[0].executed_at >= pair[1].executed_at)
    );
}

#[test]
fn many_entries_list_newest_first_with_the_default_limit() {
    let u = first_run();
    let session = Uuid::now_v7();
    for n in 0..520 {
        assert_eq!(
            record(&u.core, None, session, &format!("echo {n}")),
            HistoryDecision::Recorded
        );
    }
    let all = list(&u.core, None, None, None);
    assert_eq!(all.len(), 500);
    assert_eq!(all[0], "echo 519");
    assert_eq!(all[499], "echo 20");
    assert_eq!(list(&u.core, None, None, Some(5000)).len(), 520);
}

#[test]
fn delete_and_clear() {
    let mut u = first_run();
    let web = u.host(json!({"address": "web.example.com"}));
    let web_ref = Some(RecordRef {
        vault_id: u.vault_id,
        record_id: web.record_id,
    });
    for command in ["a1", "a2", "a3"] {
        record(&u.core, web_ref, Uuid::now_v7(), command);
    }
    record(&u.core, None, Uuid::now_v7(), "local1");
    let entries = u
        .core
        .history_list(HistoryListInput {
            host: web_ref,
            ..HistoryListInput::default()
        })
        .unwrap();
    // Ids that are not history entries.
    assert!(matches!(
        u.core
            .history_delete(RecordsRef {
                vault_id: u.vault_id,
                record_ids: vec![entries[0].record_id, web.record_id],
            })
            .unwrap_err(),
        CoreError::NotFound
    ));
    assert!(matches!(
        u.core
            .history_delete(RecordsRef {
                vault_id: Uuid::now_v7(),
                record_ids: vec![entries[0].record_id],
            })
            .unwrap_err(),
        CoreError::NotFound
    ));
    u.env.take_events();
    u.core
        .history_delete(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![entries[0].record_id],
        })
        .unwrap();
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![6]
    )));
    assert_eq!(list(&u.core, web_ref, None, None), vec!["a2", "a1"]);

    // Clear one host: the entries without a host stay.
    let cleared = u
        .core
        .history_clear(HistoryClearInput { host: web_ref })
        .unwrap();
    assert_eq!(cleared.removed, 2);
    assert!(list(&u.core, web_ref, None, None).is_empty());
    assert_eq!(list(&u.core, None, None, None), vec!["local1"]);
    assert_eq!(
        u.core
            .history_clear(HistoryClearInput { host: web_ref })
            .unwrap()
            .removed,
        0
    );

    // Survives a restart (the class is decrypted again on first use).
    u.reopen();
    u.core.unlock(u.password.clone()).unwrap();
    assert_eq!(list(&u.core, None, None, None), vec!["local1"]);
    assert!(list(&u.core, web_ref, None, None).is_empty());
    assert_eq!(
        u.core
            .history_clear(HistoryClearInput::default())
            .unwrap()
            .removed,
        1
    );
}
