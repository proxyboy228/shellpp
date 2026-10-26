//! The command dispatcher on the computer (M1): the vault made at the
//! first run lasts across starts, hosts and groups with the desktop's
//! inheritance, the errors the screens show, and fingerprint unlock after a
//! lock (D218) with the in-memory key store standing in for the Keystore.

use std::sync::Arc;

use serde_json::{Value, json};
use shellpp_android::{Engine, Services};
use shellpp_core::platform::memory::{MemoryClipboard, MemorySecretStore, RecordingLauncher};

fn random(prefix: &str) -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("random");
    let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{prefix}-{suffix}")
}

fn services(store: &Arc<MemorySecretStore>) -> Services {
    Services {
        secret_store: store.clone(),
        clipboard: Arc::new(MemoryClipboard::new()),
        launcher: Arc::new(RecordingLauncher::new()),
        device_name: "Test phone".to_owned(),
    }
}

/// Runs `command` and returns its `ok` value; panics on an error.
fn ok(engine: &Engine, command: &str, input: &Value) -> Value {
    let answer: Value =
        serde_json::from_str(&engine.call(command, input.to_string().as_bytes())).expect("json");
    assert!(answer.get("error").is_none(), "{command}: {answer}");
    answer["ok"].clone()
}

/// Runs `command` and returns its error code; panics on success.
fn code(engine: &Engine, command: &str, input: &[u8]) -> String {
    let answer: Value = serde_json::from_str(&engine.call(command, input)).expect("json");
    answer["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("{command} did not fail: {answer}"))
        .to_owned()
}

#[test]
fn the_vault_lasts_across_starts_with_its_groups_and_hosts() {
    let files = tempfile::tempdir().expect("files");
    let store = Arc::new(MemorySecretStore::new());
    let password = random("pw");
    let (group_id, host_id) = {
        let engine = Engine::start(files.path(), services(&store)).expect("engine");
        assert_eq!(
            ok(&engine, "lock_status", &json!(null))["state"],
            "first_run"
        );
        assert_eq!(
            code(&engine, "records_query", b"{\"types\":[\"host\"]}"),
            "locked"
        );
        let status = ok(&engine, "first_run_setup", &json!({ "password": password }));
        assert_eq!(status["state"], "unlocked");
        assert_eq!(status["remember_device_enabled"], false);
        let vault_id = ok(&engine, "vaults", &json!(null))[0]["vault_id"].clone();
        let group = ok(
            &engine,
            "record_save",
            &json!({
                "vault_id": vault_id,
                "type": "group",
                "data": { "name": "Servers", "ssh": { "username": "deploy", "port": 2222 } },
            }),
        );
        let group_id = group["record_id"].clone();
        let host = ok(
            &engine,
            "record_save",
            &json!({
                "vault_id": vault_id,
                "type": "host",
                "data": { "address": "web.example.org", "label": "Web", "group_id": group_id },
                "secrets": { "ssh.password": random("host-pw") },
            }),
        );
        assert_eq!(host["inherited"]["username"]["value"], "deploy");
        assert_eq!(host["inherited"]["port"]["value"], 2222);
        assert_eq!(host["inherited"]["username"]["from_group_id"], group_id);
        assert_eq!(host["secrets_set"], json!(["ssh.password"]));
        assert!(host["data"]["ssh"].get("password").is_none());
        (group_id, host["record_id"].clone())
    };

    // The next start finds the vault locked, with everything in it.
    let engine = Engine::start(files.path(), services(&store)).expect("engine");
    assert_eq!(ok(&engine, "lock_status", &json!(null))["state"], "locked");
    let wrong = json!({ "password": random("wrong") }).to_string();
    assert_eq!(
        code(&engine, "unlock", wrong.as_bytes()),
        "invalid_password"
    );
    let status = ok(&engine, "unlock", &json!({ "password": password }));
    assert_eq!(status["state"], "unlocked");
    let top = ok(
        &engine,
        "records_query",
        &json!({ "types": ["host", "group"], "scope": { "kind": "top_level" } }),
    );
    assert_eq!(top.as_array().map(Vec::len), Some(1));
    assert_eq!(top[0]["type"], "group");
    assert_eq!(top[0]["host_count"], 1);
    let inside = ok(
        &engine,
        "records_query",
        &json!({ "types": ["host"], "scope": { "kind": "group", "group_id": group_id } }),
    );
    assert_eq!(inside[0]["record_id"], host_id);
    let found = ok(
        &engine,
        "records_query",
        &json!({ "types": ["host"], "text": "web.EXAMPLE" }),
    );
    assert_eq!(found[0]["record_id"], host_id);
}

#[test]
fn bad_input_and_unknown_commands_answer_their_codes() {
    let files = tempfile::tempdir().expect("files");
    let store = Arc::new(MemorySecretStore::new());
    let engine = Engine::start(files.path(), services(&store)).expect("engine");
    assert_eq!(code(&engine, "no_such_command", b""), "unknown_command");
    assert_eq!(code(&engine, "unlock", b"not json"), "bad_input");
    assert_eq!(code(&engine, "first_run_setup", b"{}"), "bad_input");
    // A short master password is refused by the core with its field.
    let answer: Value =
        serde_json::from_str(&engine.call("first_run_setup", br#"{"password":"short"}"#))
            .expect("json");
    assert_eq!(answer["error"]["code"], "validation_failed", "{answer}");
}

#[test]
fn fingerprint_unlock_opens_after_a_lock_and_not_without_the_prompt() {
    let files = tempfile::tempdir().expect("files");
    let store = Arc::new(MemorySecretStore::new());
    let engine = Engine::start(files.path(), services(&store)).expect("engine");
    ok(
        &engine,
        "first_run_setup",
        &json!({ "password": random("pw") }),
    );
    let status = ok(&engine, "remember_device_set", &json!({ "enabled": true }));
    assert_eq!(status["remember_device_enabled"], true);
    ok(&engine, "lock", &json!(null));
    assert_eq!(ok(&engine, "try_device_unlock", &json!(null)), true);
    assert_eq!(
        ok(&engine, "lock_status", &json!(null))["state"],
        "unlocked"
    );

    // Locked again, and the prompt not passed: no unlock, the option stays.
    ok(&engine, "lock", &json!(null));
    store.set_requires_authentication(true);
    assert_eq!(ok(&engine, "try_device_unlock", &json!(null)), false);
    let status = ok(&engine, "lock_status", &json!(null));
    assert_eq!(status["state"], "locked");
    assert_eq!(status["remember_device_enabled"], true);
    assert_eq!(store.accounts().len(), 1);
}

#[test]
fn the_sync_commands_answer_before_an_account_exists() {
    let files = tempfile::tempdir().expect("files");
    let store = Arc::new(MemorySecretStore::new());
    let engine = Engine::start(files.path(), services(&store)).expect("engine");
    ok(
        &engine,
        "first_run_setup",
        &json!({ "password": random("pw") }),
    );
    let status = ok(&engine, "sync_status", &json!(null));
    assert_eq!(status["bound"], false);
    assert_eq!(status["signed_in"], false);

    // Not a server address: the reason the wizard shows.
    let answer: Value = serde_json::from_str(
        &engine.call(
            "sync_server_check",
            json!({ "origin": "ftp://example.org" })
                .to_string()
                .as_bytes(),
        ),
    )
    .expect("json");
    assert_eq!(answer["error"]["code"], "sync_error", "{answer}");
    assert_eq!(
        answer["error"]["details"]["reason"], "invalid_origin",
        "{answer}"
    );

    // A trust choice that is not one, and a file that is not a certificate.
    let bad_pin = json!({ "origin": "https://sync.example.org", "choice": { "kind": "pin", "sha256": "abc" } });
    assert_eq!(
        code(
            &engine,
            "sync_server_trust_verified",
            bad_pin.to_string().as_bytes()
        ),
        "bad_input"
    );
    let not_a_cert = json!({ "file": "aGVsbG8=" });
    assert_eq!(
        code(&engine, "ca_file_info", not_a_cert.to_string().as_bytes()),
        "validation_failed"
    );
    assert_eq!(ok(&engine, "sync_weaker_kdf", &json!(null)), Value::Null);

    // The account screen's "Show the recovery key": a local vault has none
    // (D217); a bound one shows it for the master password.
    let show = json!({ "password": random("pw") });
    assert_eq!(
        code(&engine, "recovery_key_show", show.to_string().as_bytes()),
        "wrong_state"
    );
}

#[test]
fn identities_recent_connections_and_a_partial_save_through_the_binding() {
    let files = tempfile::tempdir().expect("files");
    let store = Arc::new(MemorySecretStore::new());
    let engine = Engine::start(files.path(), services(&store)).expect("engine");
    let password = random("pw");
    ok(&engine, "first_run_setup", &json!({ "password": password }));
    let vault = ok(&engine, "vaults", &json!(null))[0]["vault_id"].clone();

    // An identity, made with a password the answer never shows.
    let secret = random("identity-pw");
    let made = ok(
        &engine,
        "identity_save",
        &json!({"label": "deploy", "username": "deploy", "password": secret}),
    );
    assert_eq!(made["label"], "deploy");
    assert_eq!(made["password_set"], true);
    assert!(!made.to_string().contains(&secret));
    let list = ok(&engine, "identities_list", &json!(null));
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    // Cleared again; the label stays.
    let id = made["record_id"].clone();
    let cleared = ok(
        &engine,
        "identity_save",
        &json!({"record_id": id, "label": "deploy", "username": "deploy", "clear_password": true}),
    );
    assert_eq!(cleared["password_set"], false);

    // No session yet: no recent connection.
    assert_eq!(ok(&engine, "recent_connections", &json!(null)), json!([]));

    // The editor's partial save (D221): a change made meanwhile stays.
    let host = ok(
        &engine,
        "record_save",
        &json!({"vault_id": vault, "type": "host", "data": {"address": "web.example.org", "label": "web"}}),
    );
    let host_id = host["record_id"].clone();
    let base = json!({"revision": host["revision"], "data": host["data"]});
    ok(
        &engine,
        "record_save",
        &json!({"vault_id": vault, "record_id": host_id, "type": "host", "data": {"address": "web.example.org", "label": "web", "notes": "from elsewhere"}}),
    );
    let mut form = host["data"].clone();
    form["label"] = json!("edited");
    let saved = ok(
        &engine,
        "record_save",
        &json!({"vault_id": vault, "record_id": host_id, "type": "host", "data": form, "base": base}),
    );
    assert_eq!(saved["data"]["label"], "edited");
    assert_eq!(saved["data"]["notes"], "from elsewhere");

    // The identity dialog's partial save (D221, D223 item 2), as the
    // Keychain dialog sends it: the version it opened as `base`; a username
    // changed meanwhile stays.
    let opened = ok(&engine, "identities_list", &json!(null))[0].clone();
    let base = json!({"revision": opened["revision"], "data": {
        "label": opened["label"], "username": opened["username"], "key_id": opened["key_id"]}});
    ok(
        &engine,
        "identity_save",
        &json!({"record_id": id, "label": "deploy", "username": "from elsewhere"}),
    );
    let saved = ok(
        &engine,
        "identity_save",
        &json!({"record_id": id, "label": "deploy edited", "username": "deploy", "base": base}),
    );
    assert_eq!(saved["label"], "deploy edited");
    assert_eq!(saved["username"], "from elsewhere");
    assert!(saved["revision"].as_u64().is_some_and(|revision| revision > 0));
}
