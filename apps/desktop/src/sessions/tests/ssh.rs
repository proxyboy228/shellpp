//! Connecting: host key decisions, prompts, key files, settings that reach
//! the server, and the lock.

use secrecy::ExposeSecret as _;
use shellpp_core::service::dto::KnownHostAddInput;
use shellpp_test_sshd::{KeyType, KiQuestion, ServerEvent, generate_key};

use super::*;

fn password_user(name: &str, password: &str) -> UserConfig {
    UserConfig::new(name).with_password(secret(password))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tofu_accept_and_save_connects_and_streams_output() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);

    let mut session = harness.open(target.clone());
    assert_eq!(session.view.state, SessionState::Connecting);
    assert_eq!(session.view.title, "test host");
    let (id, kind) = session.prompt("host_key_unknown").await;
    let PromptKind::HostKeyUnknown { key } = kind else {
        unreachable!()
    };
    assert_eq!(key.fingerprint, server.host_key_fingerprint());
    assert_eq!(key.key_type, server.host_key_type());
    assert_eq!(key.port, server.port());
    // A changed-key answer does not fit a first connection.
    assert_eq!(
        session
            .handle
            .reply(id, PromptReply::HostKey(HostKeyChoice::Replace)),
        Err(ReplyError::Invalid)
    );
    session.reply(id, PromptReply::HostKey(HostKeyChoice::AcceptAndSave));
    let view = session.state(SessionState::Connected).await;
    assert_eq!(view.username.as_deref(), Some("alice"));
    assert!(view.prompt.is_none() && view.error.is_none());
    session.output("Welcome to shellpp-test-sshd").await;
    session.echo_round_trip("first-session").await;
    assert_eq!(
        session.handle.reply(id, PromptReply::Cancel),
        Err(ReplyError::Stale)
    );

    // Saved in known hosts: the next session asks nothing.
    let known = harness.core.known_hosts_list().expect("known hosts");
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].fingerprint, server.host_key_fingerprint());
    let mut second = harness.connect(target, false).await;
    assert!(second.prompts.is_empty(), "{:?}", second.prompts);
    second.echo_round_trip("second-session").await;
    assert!(
        server.events().contains(&ServerEvent::Pty {
            term: "xterm-256color".into(),
            cols: 80,
            rows: 24
        }),
        "pty request with the default TERM"
    );
    session.handle.close();
    second.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_once_trusts_the_key_for_this_session_only() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);

    let mut session = harness.open(target.clone());
    let (id, _) = session.prompt("host_key_unknown").await;
    session.reply(id, PromptReply::HostKey(HostKeyChoice::ConnectOnce));
    session.state(SessionState::Connected).await;
    assert!(harness.core.known_hosts_list().unwrap().is_empty());

    // A reconnect of the same session trusts it again without asking.
    session.type_line("drop");
    session.state(SessionState::Dropped).await;
    session.state(SessionState::Connected).await;
    assert_eq!(session.prompts, vec!["host_key_unknown"]);

    // Another session asks again.
    let mut other = harness.open(target);
    let (id, _) = other.prompt("host_key_unknown").await;
    other.reply(id, PromptReply::HostKey(HostKeyChoice::Cancel));
    let view = other.state(SessionState::Failed).await;
    assert_eq!(
        view.error,
        Some(SessionError::Ssh(FailureKind::HostKeyRejected))
    );
    assert!(view.message().is_some());
    session.handle.close();
    other.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_host_key_blocks_until_it_is_replaced() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let first = harness.connect(target.clone(), true).await;
    first.handle.close();
    let old = server.host_key_fingerprint();
    server.rotate_host_key().await.expect("rotate");
    let new = server.host_key_fingerprint();
    assert_ne!(old, new);

    let mut session = harness.open(target);
    let (id, kind) = session.prompt("host_key_changed").await;
    let PromptKind::HostKeyChanged {
        key,
        old_fingerprint,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(old_fingerprint, old);
    assert_eq!(key.fingerprint, new);
    // Only "Replace saved key" continues.
    assert_eq!(
        session
            .handle
            .reply(id, PromptReply::HostKey(HostKeyChoice::AcceptAndSave)),
        Err(ReplyError::Invalid)
    );
    session.reply(id, PromptReply::HostKey(HostKeyChoice::Cancel));
    let view = session.state(SessionState::Failed).await;
    assert_eq!(
        view.error,
        Some(SessionError::Ssh(FailureKind::HostKeyRejected))
    );
    assert_eq!(
        harness.core.known_hosts_list().unwrap()[0].fingerprint,
        old,
        "a cancel keeps the saved key"
    );

    // Retry: the warning again, then the replacement.
    session.handle.reconnect();
    let (id, _) = session.prompt("host_key_changed").await;
    session.reply(id, PromptReply::HostKey(HostKeyChoice::Replace));
    session.state(SessionState::Connected).await;
    let known = harness.core.known_hosts_list().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].fingerprint, new);
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_host_key_is_blocked_without_a_dialog() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    harness
        .core
        .known_host_add(KnownHostAddInput {
            host: "*".to_owned(),
            port: None,
            key_type: server.host_key_type(),
            public_key: server.host_key_base64(),
            revoked: true,
            comment: None,
        })
        .expect("revoked entry");
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.open(target);
    let view = session.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::HostKeyRevoked));
    assert_eq!(
        view.host_key.as_ref().map(|key| key.fingerprint.clone()),
        Some(server.host_key_fingerprint())
    );
    assert!(session.prompts.is_empty());
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_is_asked_again_and_save_stores_the_right_one() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.host(server.port(), json!({"ssh": {"username": "alice"}}), None);
    let mut session = harness.open(target.clone());
    session.accept_key().await;
    let kind = session
        .answer("password", &random_secret("wrong"), false)
        .await;
    assert_eq!(
        kind,
        PromptKind::Password {
            username: "alice".into(),
            error: false,
            save: SaveOption::Vault
        }
    );
    let (id, kind) = session.prompt("password").await;
    assert!(matches!(kind, PromptKind::Password { error: true, .. }));
    session.reply(
        id,
        PromptReply::Text {
            value: secret(&password),
            save: true,
        },
    );
    session.state(SessionState::Connected).await;
    let config = harness.core.session_config(&target).expect("config");
    assert_eq!(
        config
            .password
            .as_ref()
            .map(|p| p.expose_secret().to_owned()),
        Some(password.clone()),
        "Save stored the password in the host"
    );
    let second = harness.connect(target, false).await;
    assert!(second.prompts.is_empty());
    session.handle.close();
    second.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keyboard_interactive_questions_come_one_at_a_time() {
    let harness = Harness::new();
    let code = random_secret("code");
    let server = start_server(vec![UserConfig::new("kim").with_keyboard_interactive(
        "Server check",
        "Answer both",
        vec![
            KiQuestion::new("Code: ", false, secret(&code)),
            KiQuestion::new("Name: ", true, secret("kim")),
        ],
    )])
    .await;
    let target = harness.host(server.port(), json!({"ssh": {"username": "kim"}}), None);
    let mut session = harness.open(target);
    session.accept_key().await;
    let first = session.answer("keyboard_interactive", &code, false).await;
    match first {
        PromptKind::KeyboardInteractive {
            name,
            instruction,
            prompt,
            echo,
            index,
            count,
            error,
        } => {
            assert_eq!(name, "Server check");
            assert_eq!(instruction, "Answer both");
            assert_eq!(prompt, "Code: ");
            assert!(!echo, "the code is masked");
            assert_eq!((index, count, error), (0, 2, false));
        }
        other => panic!("unexpected {other:?}"),
    }
    session
        .until("the second question", |view| {
            matches!(
                view.prompt.as_ref().map(|p| &p.kind),
                Some(PromptKind::KeyboardInteractive { index: 1, .. })
            )
        })
        .await;
    let second = session.answer("keyboard_interactive", "kim", false).await;
    assert!(matches!(
        second,
        PromptKind::KeyboardInteractive {
            echo: true,
            index: 1,
            ..
        }
    ));
    assert!(!second.is_secret());
    session.state(SessionState::Connected).await;
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quick_connect_asks_for_the_username_and_never_offers_save() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let mut session = harness.open(SessionTarget::Quick {
        address: "127.0.0.1".to_owned(),
        port: Some(server.port()),
        username: None,
    });
    assert_eq!(session.view.title, "127.0.0.1");
    session.accept_key().await;
    session.answer("username", "alice", false).await;
    let kind = session.answer("password", &password, true).await;
    assert!(matches!(
        kind,
        PromptKind::Password {
            save: SaveOption::None,
            ..
        }
    ));
    let view = session.state(SessionState::Connected).await;
    assert_eq!(view.username.as_deref(), Some("alice"));
    assert!(view.host.is_none());
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quick_connect_targets_are_validated() {
    let harness = Harness::new();
    let error = harness
        .open_with(
            OpenRequest::Ssh(SessionTarget::Quick {
                address: "bad host name".to_owned(),
                port: Some(22),
                username: None,
            }),
            Arc::new(TestTerminal::default()),
        )
        .err();
    assert_eq!(error, Some(SessionError::InvalidTarget));
    let missing = harness.open_with(
        OpenRequest::Ssh(SessionTarget::Host {
            vault_id: harness.vault_id,
            record_id: Uuid::now_v7(),
        }),
        Arc::new(TestTerminal::default()),
    );
    assert_eq!(missing.err(), Some(SessionError::HostMissing));
}

/// A host behind a jump host connects through it (stage 5): the jump host's
/// key is decided first, on its own address, then the host's; each uses
/// its own saved credentials.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_behind_a_jump_host_connects_through_it() {
    let harness = Harness::new();
    let jump_password = random_secret("jump");
    let mut config = ServerConfig::new(vec![
        UserConfig::new("ops").with_password(secret(&jump_password)),
    ]);
    config.forwarding = true;
    let jump = start_with(config).await;
    let password = random_secret("pw");
    let server = start_server(vec![
        UserConfig::new("alice").with_password(secret(&password)),
    ])
    .await;
    let bastion = harness.host(
        jump.port(),
        json!({"label": "bastion", "ssh": {"username": "ops"}}),
        Some(&jump_password),
    );
    let SessionTarget::Host { record_id, .. } = bastion else {
        panic!("a saved host");
    };
    let target = harness.host(
        server.port(),
        json!({"ssh": {"username": "alice", "jump_chain": [record_id.to_string()]}}),
        Some(&password),
    );
    let mut session = harness.open(target);
    let jump_key = session.accept_key().await;
    assert_eq!(jump_key, jump.host_key_fingerprint(), "the jump host first");
    let target_key = session.accept_key().await;
    assert_eq!(target_key, server.host_key_fingerprint());
    let view = session.state(SessionState::Connected).await;
    assert_eq!(view.username.as_deref(), Some("alice"));
    assert!(jump.events().iter().any(|event| matches!(
        event,
        ServerEvent::DirectTcpip { port, .. } if *port == u32::from(server.port())
    )));
    session.handle.close();
    server.shutdown().await;
    jump.shutdown().await;
}

/// Environment variables the server refuses (OpenSSH `AcceptEnv`) are named
/// once in the session, in the terminal; a reconnect does not repeat it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_environment_variables_are_named_once_per_session() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.accept_env = Some(vec!["LANG".to_owned()]);
    let server = start_with(config).await;
    let target = harness.host(
        server.port(),
        json!({"ssh": {"username": "alice", "env": [
            {"name": "LANG", "value": "C.UTF-8"},
            {"name": "APP_MODE", "value": "dev"}
        ]}}),
        Some(&password),
    );
    let mut session = harness.connect(target, true).await;
    session
        .output(
            "The server did not set APP_MODE: it accepts only the variables its AcceptEnv allows.",
        )
        .await;
    let notice = "The server did not set";
    session.type_line("drop");
    session.state(SessionState::Dropped).await;
    session.state(SessionState::Connected).await;
    session.type_line("echo after-reconnect");
    session.output("after-reconnect").await;
    let text = String::from_utf8_lossy(&session.terminal.bytes()).into_owned();
    assert_eq!(text.matches(notice).count(), 1, "{text:?}");
    session.handle.close();
    server.shutdown().await;
}

/// The startup snippet runs right after every login as injected input
/// (never recorded in history); one with variables waits for its form.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_startup_snippet_runs_after_login() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let snippet = harness
        .core
        .snippet_save(shellpp_core::service::snippets::RecordInput {
            vault_id: harness.vault_id,
            record_id: None,
            data: json!({"script": "echo started-$((6*7))\necho second"})
                .as_object()
                .cloned()
                .unwrap(),
        })
        .unwrap();
    let target = harness.host(
        server.port(),
        json!({"ssh": {"username": "alice",
            "startup_snippet_id": snippet.record.record_id.to_string()}}),
        Some(&password),
    );
    let mut session = harness.connect(target, true).await;
    session.output("second").await;
    assert_eq!(
        session.terminal.injected(),
        b"echo started-$((6*7))\recho second\r"
    );
    session.handle.close();

    // A snippet with variables is not run without its form.
    let with_vars = harness
        .core
        .snippet_save(shellpp_core::service::snippets::RecordInput {
            vault_id: harness.vault_id,
            record_id: None,
            data: json!({"script": "cd {{dir}}"})
                .as_object()
                .cloned()
                .unwrap(),
        })
        .unwrap();
    let target = harness.host(
        server.port(),
        json!({"label": "other", "ssh": {"username": "alice",
            "startup_snippet_id": with_vars.record.record_id.to_string()}}),
        Some(&password),
    );
    let session = harness.connect(target, false).await;
    assert!(session.terminal.injected().is_empty());
    // It waits for its values: the pane asks them (stage 5).
    let mut asked = None;
    for _ in 0..100 {
        asked = session.handle.view().startup_ask;
        if asked.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let asked = asked.expect("the startup snippet asks its values");
    assert_eq!(asked.variables, ["dir"]);
    assert_eq!(asked.login, 1);
    assert!(session.terminal.injected().is_empty());
    session.handle.close();
    server.shutdown().await;
}

/// Agent forwarding offers the Keychain's keys through the built-in agent
/// (D60): the server's agent channel lists them; a key with a passphrase
/// that is not saved is left out (it cannot be opened without asking).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_forwarding_offers_the_keychain_keys() {
    let harness = Harness::new();
    for (comment, passphrase, save) in [
        ("plain", None, false),
        ("saved", Some("pp-1"), true),
        ("asked", Some("pp-2"), false),
    ] {
        let key =
            generate_key(KeyType::Ed25519, comment, passphrase.map(secret).as_ref()).expect("key");
        harness
            .core
            .key_import(shellpp_core::service::dto::KeyImportInput {
                vault_id: harness.vault_id,
                label: None,
                private_key: key.private_openssh.clone(),
                passphrase: passphrase.map(secret),
                save_passphrase: save,
            })
            .expect("imported");
    }
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.host(
        server.port(),
        json!({"ssh": {"username": "alice", "agent_forwarding": true}}),
        Some(&password),
    );
    let mut session = harness.connect(target, true).await;
    session.type_line("agent-keys");
    session.output("agent keys: 2").await;
    assert!(server.events().contains(&ServerEvent::AgentForward));
    // A lock wipes the keys; the unlock loads them again (the review of
    // stage 5: they stayed gone).
    harness.manager.set_unlocked(false);
    harness.manager.set_unlocked(true);
    let mut back = false;
    for _ in 0..40 {
        let from = session.terminal.bytes().len();
        session.type_line("agent-keys");
        session.output_from(from, "agent keys: ").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let text = String::from_utf8_lossy(&session.terminal.bytes()[from..]).into_owned();
        if text.contains("agent keys: 2") {
            back = true;
            break;
        }
    }
    assert!(back, "the keys are offered again after the unlock");
    session.handle.close();
    server.shutdown().await;
}

/// A jump host logs in with the user's key files like a direct connection
/// to it would (the review of stage 5: hops had no key sources).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_jump_host_logs_in_with_the_key_files() {
    let harness = Harness::new();
    let key = generate_key(KeyType::Ed25519, "jump key", None).expect("key");
    std::fs::write(
        harness.key_dir.path().join("id_ed25519"),
        key.private_openssh.expose_secret(),
    )
    .unwrap();
    let mut config = ServerConfig::new(vec![
        UserConfig::new("ops").with_authorized_key(&key.public_openssh),
    ]);
    config.forwarding = true;
    let jump = start_with(config).await;
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let bastion = harness.host(
        jump.port(),
        json!({"label": "bastion", "ssh": {"username": "ops"}}),
        None,
    );
    let SessionTarget::Host { record_id, .. } = bastion else {
        panic!("a saved host");
    };
    let target = harness.host(
        server.port(),
        json!({"ssh": {"username": "alice", "jump_chain": [record_id.to_string()]}}),
        Some(&password),
    );
    let mut session = harness.open(target);
    session.accept_key().await;
    session.accept_key().await;
    session.state(SessionState::Connected).await;
    session.handle.close();
    server.shutdown().await;
    jump.shutdown().await;
}

/// A host whose chain names a host that does not exist is refused, never
/// connected directly instead (data-model 8.3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_with_a_missing_jump_host_is_not_connected_directly() {
    let harness = Harness::new();
    let target = harness.host(
        22,
        json!({"ssh": {"jump_chain": [Uuid::now_v7().to_string()]}}),
        None,
    );
    let error = harness
        .open_with(OpenRequest::Ssh(target), Arc::new(TestTerminal::default()))
        .err();
    assert_eq!(error, Some(SessionError::RouteInvalid));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_file_passphrase_is_asked_once_and_remembered_for_the_session() {
    let harness = Harness::new();
    let passphrase = random_secret("pass");
    let key = generate_key(KeyType::Ed25519, "test key", Some(&secret(&passphrase))).expect("key");
    std::fs::write(
        harness.key_dir.path().join("id_ed25519"),
        key.private_openssh.expose_secret(),
    )
    .unwrap();
    std::fs::write(
        harness.key_dir.path().join("id_ed25519.pub"),
        &key.public_openssh,
    )
    .unwrap();
    let server = start_server(vec![
        UserConfig::new("bob").with_authorized_key(&key.public_openssh),
    ])
    .await;
    let target = harness.host(server.port(), json!({"ssh": {"username": "bob"}}), None);

    let mut session = harness.open(target.clone());
    session.accept_key().await;
    let (id, kind) = session.prompt("passphrase").await;
    assert_eq!(
        kind,
        PromptKind::Passphrase {
            key_label: "id_ed25519".into(),
            error: false,
            save: SaveOption::ThisSession
        }
    );
    session.reply(
        id,
        PromptReply::Text {
            value: secret(&random_secret("wrong")),
            save: true,
        },
    );
    let (id, kind) = session.prompt("passphrase").await;
    assert!(matches!(kind, PromptKind::Passphrase { error: true, .. }));
    session.reply(
        id,
        PromptReply::Text {
            value: secret(&passphrase),
            save: true,
        },
    );
    session.state(SessionState::Connected).await;
    assert_eq!(harness.manager.remembered_passphrases(), 1);

    // Remembered: the next session asks nothing.
    let second = harness.connect(target.clone(), false).await;
    assert!(second.prompts.is_empty(), "{:?}", second.prompts);
    second.handle.close();

    // A lock forgets it.
    harness.manager.set_unlocked(false);
    assert_eq!(harness.manager.remembered_passphrases(), 0);
    harness.manager.set_unlocked(true);
    let mut third = harness.open(target);
    third.answer("passphrase", &passphrase, false).await;
    third.state(SessionState::Connected).await;
    assert_eq!(harness.manager.remembered_passphrases(), 0);
    session.handle.close();
    third.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn term_environment_charset_and_resize_reach_the_server() {
    let harness = Harness::new();
    harness.set("terminal.term", json!("vt100"));
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.host(
        server.port(),
        json!({
            "charset": "windows-1251",
            "ssh": {
                "username": "alice",
                "env": [{"name": "SHELLPP_TEST_VAR", "value": "seven"}]
            }
        }),
        Some(&password),
    );
    let mut session = harness.connect(target, true).await;
    let events = server.events();
    assert!(events.contains(&ServerEvent::Pty {
        term: "vt100".into(),
        cols: 80,
        rows: 24
    }));
    assert!(events.contains(&ServerEvent::Env {
        name: "SHELLPP_TEST_VAR".into()
    }));
    session
        .echo_round_trip("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}")
        .await;

    session.handle.resize(100, 30);
    let from = session.terminal.len();
    session.type_line("size");
    session.output_from(from, "100x30").await;
    assert!(server.events().contains(&ServerEvent::WindowChange {
        cols: 100,
        rows: 30
    }));
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lock_cancels_prompts_and_a_dropped_session_waits_for_unlock() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;

    // A prompt open at the lock: the first attempt stops.
    let asking = harness.host(server.port(), json!({"ssh": {"username": "alice"}}), None);
    let mut prompted = harness.open(asking);
    prompted.accept_key().await;
    prompted.prompt("password").await;
    harness.manager.set_unlocked(false);
    let view = prompted.state(SessionState::Failed).await;
    assert!(view.prompt.is_none());
    assert_eq!(view.error, Some(SessionError::Locked));
    // Nothing opens while locked.
    assert_eq!(
        harness
            .open_with(
                OpenRequest::Local { shell_id: None },
                Arc::new(TestTerminal::default())
            )
            .err(),
        Some(SessionError::Locked)
    );
    harness.manager.set_unlocked(true);

    // A drop while locked: automatic attempts wait for unlock.
    let target = harness.password_host(&server, "alice", &password);
    let mut session = harness.connect(target, false).await;
    harness.manager.set_unlocked(false);
    session.type_line("drop");
    let view = session
        .until("waiting for unlock", |view| {
            view.reconnect.is_some_and(|r| r.waiting_for_unlock)
        })
        .await;
    assert_eq!(view.state, SessionState::Dropped);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    session.drain();
    assert_eq!(
        session.view.state,
        SessionState::Dropped,
        "no attempt while locked"
    );
    harness.manager.set_unlocked(true);
    session.state(SessionState::Connected).await;
    prompted.handle.close();
    session.handle.close();
    server.shutdown().await;
}

/// UI 4.6, 13.3: the core locks after an automatic attempt's countdown but
/// before the session layer hears of the lock, so the attempt's settings
/// read is refused as locked. The attempt waits for unlock like one the
/// lock aborted; automatic attempts do not stop with "Shell++ is locked".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lock_racing_an_automatic_attempt_waits_for_unlock() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    // The first connection to this server: its key is accepted first.
    let mut session = harness.connect(target, true).await;

    harness
        .calls
        .config_locked
        .store(true, std::sync::atomic::Ordering::SeqCst);
    session.type_line("drop");
    let deadline = std::time::Instant::now() + WAIT;
    while harness
        .calls
        .config_refusals
        .load(std::sync::atomic::Ordering::SeqCst)
        == 0
    {
        assert!(std::time::Instant::now() < deadline, "no automatic attempt");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The lock state follows the core.
    harness.manager.set_unlocked(false);
    let view = session
        .until("waiting for unlock", |view| {
            view.reconnect.is_some_and(|r| r.waiting_for_unlock)
        })
        .await;
    assert_eq!(view.state, SessionState::Dropped);
    assert_ne!(view.error, Some(SessionError::Locked));

    harness
        .calls
        .config_locked
        .store(false, std::sync::atomic::Ordering::SeqCst);
    harness.manager.set_unlocked(true);
    session.state(SessionState::Connected).await;
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_and_close_while_connecting() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    server.set_holding(true);
    let target = harness.password_host(&server, "alice", &password);

    let mut cancelled = harness.open(target.clone());
    cancelled
        .until("the handshake", |view| {
            view.step == Some(ConnectStep::Connecting)
        })
        .await;
    cancelled.handle.cancel();
    let view = cancelled.state(SessionState::Failed).await;
    assert_eq!(view.error, Some(SessionError::Cancelled));
    assert_eq!(
        view.message().as_deref(),
        Some(strings::error_message(SessionError::Cancelled))
    );

    let mut closed = harness.open(target);
    closed
        .until("the handshake", |view| {
            view.step == Some(ConnectStep::Connecting)
        })
        .await;
    closed.handle.close();
    closed.state(SessionState::Closed).await;
    assert!(harness.manager.handle(closed.handle.id()).is_none());
    assert!(
        closed.terminal.attached().is_some(),
        "the terminal got the handle at open"
    );
    cancelled.handle.close();
    server.set_holding(false);
    server.drop_connections();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_agent_does_not_stop_other_methods() {
    let harness = Harness::with(|options| {
        options.keys.agent = AgentSource::At(shellpp_ssh::AgentLocation::Path(
            std::env::temp_dir().join(format!("shellpp-no-agent-{}", random_secret("a"))),
        ));
    });
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let target = harness.password_host(&server, "alice", &password);
    let session = harness.connect(target, true).await;
    session.handle.close();
    server.shutdown().await;
}
