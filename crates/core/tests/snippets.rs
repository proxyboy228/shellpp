//! Snippets, packages and forwarding rules in the core (docs/data-model.md,
//! sections 10 and 11; stage 5): save, list, references, removal of a
//! package, rule summaries, and the records after a restart.

mod common;

use common::{first_run, reasons};
use serde_json::json;
use shellpp_core::ValidationReason;
use shellpp_core::service::dto::RecordsRef;
use shellpp_core::service::snippets::{ForwardType, RecordInput};
use uuid::Uuid;

fn input(u: &common::Unlocked, record_id: Option<Uuid>, data: serde_json::Value) -> RecordInput {
    RecordInput {
        vault_id: u.vault_id,
        record_id,
        data: common::map(data),
    }
}

#[test]
fn snippets_with_variables_packages_and_targets() {
    let u = first_run();
    let web = u.host(json!({"address": "web.example"}));
    let package = u
        .core
        .snippet_package_save(input(&u, None, json!({"name": "Deploy"})))
        .unwrap();
    let snippet = u
        .core
        .snippet_save(input(
            &u,
            None,
            json!({"script": "cd {{ dir }} && git pull && systemctl restart {{unit}} # {{dir}}",
                "label": "Update", "package_id": package.record.record_id.to_string(),
                "tags": ["deploy"], "target_host_ids": [web.record_id.to_string()]}),
        ))
        .unwrap();
    assert_eq!(snippet.variables, ["dir", "unit"]);
    assert_eq!(snippet.package_id, Some(package.record.record_id));
    assert_eq!(snippet.target_host_ids, [web.record_id]);
    assert_eq!(snippet.tags, ["deploy"]);

    // A missing package or host is refused.
    let error = u
        .core
        .snippet_save(input(
            &u,
            None,
            json!({"script": "x", "package_id": Uuid::now_v7().to_string(),
                "target_host_ids": [Uuid::now_v7().to_string()]}),
        ))
        .unwrap_err();
    assert_eq!(
        reasons(error),
        [
            ("package_id".to_owned(), ValidationReason::MissingRef),
            ("target_host_ids".to_owned(), ValidationReason::MissingRef)
        ]
    );
    // A save replaces the known members (the input is the whole record, as
    // for hosts) and keeps members of later versions (data-model 1.2).
    let edited = u
        .core
        .snippet_save(input(
            &u,
            Some(snippet.record.record_id),
            json!({"script": "uptime", "label": "Update",
                "package_id": package.record.record_id.to_string(), "x_later": 1}),
        ))
        .unwrap();
    assert!(edited.variables.is_empty());
    assert!(
        edited.target_host_ids.is_empty(),
        "the targets were not named"
    );

    // A removed package moves its snippets out of it.
    u.core
        .record_delete(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![package.record.record_id],
        })
        .unwrap();
    assert!(u.core.snippet_packages_list(u.vault_id).unwrap().is_empty());
    let list = u.core.snippets_list(u.vault_id).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].package_id, None);
    assert_eq!(list[0].label.as_deref(), Some("Update"));
}

#[test]
fn forwarding_rules_and_their_summaries() {
    let u = first_run();
    let db = u.host(json!({"address": "db.example", "label": "db"}));
    let local = u
        .core
        .forwarding_rule_save(input(
            &u,
            None,
            json!({"type": "local", "host_id": db.record_id.to_string(), "bind_port": 15432,
                "dest_address": "127.0.0.1", "dest_port": 5432, "autostart_on_connect": true}),
        ))
        .unwrap();
    assert_eq!(local.kind, ForwardType::Local);
    assert_eq!(local.summary, "127.0.0.1:15432 → 127.0.0.1:5432 via db");
    assert!(local.autostart_on_connect && !local.autostart_on_launch);
    assert!(!local.wide_bind);
    let dynamic = u
        .core
        .forwarding_rule_save(input(
            &u,
            None,
            json!({"type": "dynamic", "host_id": db.record_id.to_string(), "bind_port": 1080,
                "bind_address": "0.0.0.0", "label": "SOCKS"}),
        ))
        .unwrap();
    assert_eq!(dynamic.summary, "SOCKS 0.0.0.0:1080 via db");
    assert!(dynamic.wide_bind, "the UI warns about all interfaces");
    let error = u
        .core
        .forwarding_rule_save(input(
            &u,
            None,
            json!({"type": "remote", "host_id": Uuid::now_v7().to_string(), "bind_port": 8080,
                "dest_address": "localhost", "dest_port": 3000}),
        ))
        .unwrap_err();
    assert_eq!(
        reasons(error),
        [("host_id".to_owned(), ValidationReason::MissingRef)]
    );
    let list = u.core.forwarding_rules_list(u.vault_id).unwrap();
    // By label, else summary: "127.0.0.1:…" sorts before "SOCKS".
    assert_eq!(
        list.iter()
            .map(|rule| rule.record.record_id)
            .collect::<Vec<_>>(),
        [local.record.record_id, dynamic.record.record_id]
    );
    // Removal and restore.
    u.core
        .record_delete(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![dynamic.record.record_id],
        })
        .unwrap();
    assert_eq!(u.core.forwarding_rules_list(u.vault_id).unwrap().len(), 1);
    u.core
        .record_restore(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![dynamic.record.record_id],
        })
        .unwrap();
    assert_eq!(u.core.forwarding_rules_list(u.vault_id).unwrap().len(), 2);
}

#[test]
fn the_records_survive_a_restart() {
    let mut u = first_run();
    let host = u.host(json!({"address": "app.example"}));
    u.core
        .snippet_save(input(&u, None, json!({"script": "df -h"})))
        .unwrap();
    u.core
        .forwarding_rule_save(input(
            &u,
            None,
            json!({"type": "dynamic", "host_id": host.record_id.to_string(), "bind_port": 1080}),
        ))
        .unwrap();
    u.reopen();
    u.core.unlock(u.password.clone()).expect("unlock");
    assert_eq!(u.core.snippets_list(u.vault_id).unwrap()[0].script, "df -h");
    assert_eq!(u.core.forwarding_rules_list(u.vault_id).unwrap().len(), 1);
}
