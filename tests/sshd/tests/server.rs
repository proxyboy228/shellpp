//! Integration tests of the library API with a russh client: start/stop,
//! every authentication method, the shell contract over SSH, exec answers,
//! agent forwarding, host key rotation, drops, freezes and the accepting
//! switch. Credentials and keys are generated per test run.

mod common;

use std::time::Duration;

use common::{
    ClientOptions, Ending, Terminal, TerminalOptions, connect, connect_with, eventually, exec,
    password_login, publickey_auth, random_secret, secret,
};
use russh::MethodKind;
use russh::client::KeyboardInteractiveAuthResponse;
use russh::keys::{PrivateKey, decode_secret_key};
use shellpp_test_sshd::shell::{AGENT_NOT_FORWARDED, SECRET_PROMPT, prompt};
use shellpp_test_sshd::{
    ExecAnswer, HostKeySource, KeyType, KiQuestion, ServerConfig, ServerEvent, TestServer,
    UserConfig, generate_key,
};
use tokio::io::AsyncReadExt;

fn auth_event(user: &str, method: &str, ok: bool) -> ServerEvent {
    ServerEvent::Auth {
        user: user.to_owned(),
        method: method.to_owned(),
        ok,
    }
}

async fn password_server(user: &str, password: &str) -> TestServer {
    TestServer::start(ServerConfig::new(vec![
        UserConfig::new(user).with_password(secret(password)),
    ]))
    .await
    .expect("server starts")
}

#[tokio::test]
async fn start_reports_address_and_host_key_and_shutdown_closes_the_port() {
    let server = password_server("alice", &random_secret()).await;
    let addr = server.addr();
    assert!(addr.ip().is_loopback());
    assert_ne!(server.port(), 0);
    assert_eq!(server.port(), addr.port());
    assert_eq!(server.host_key_type(), "ssh-ed25519");
    assert!(server.host_key_fingerprint().starts_with("SHA256:"));
    assert!(server.host_key_base64().starts_with("AAAA"));
    assert_eq!(
        server
            .host_public_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string(),
        server.host_key_fingerprint()
    );

    // The client sees the same host key.
    let (_handle, observed) = connect(server.port()).await;
    let seen = observed.server_key();
    assert_eq!(
        seen.fingerprint(russh::keys::HashAlg::Sha256).to_string(),
        server.host_key_fingerprint()
    );

    server.shutdown().await;
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "the port is closed after shutdown"
    );
}

#[tokio::test]
async fn refuses_a_non_loopback_bind_and_invalid_keys() {
    let mut config = ServerConfig::new(vec![UserConfig::new("alice")]);
    config.bind = "0.0.0.0:0".parse().expect("addr");
    let error = TestServer::start(config)
        .await
        .expect_err("non-loopback refused");
    assert!(matches!(error, shellpp_test_sshd::Error::Config(_)));

    let config = ServerConfig::new(vec![
        UserConfig::new("alice").with_authorized_key("ssh-ed25519 not-base64"),
    ]);
    let error = TestServer::start(config)
        .await
        .expect_err("bad key refused");
    assert!(matches!(error, shellpp_test_sshd::Error::Config(_)));
}

/// Reads the server identification line with a raw TCP connection.
async fn read_ident(addr: std::net::SocketAddr) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("tcp connect");
    let mut received = Vec::new();
    let mut byte = [0_u8; 1];
    while !received.ends_with(b"\r\n") {
        let count = tokio::time::timeout(common::WAIT, stream.read(&mut byte))
            .await
            .expect("ident in time")
            .expect("read");
        assert!(count > 0, "server closed before the identification line");
        received.push(byte[0]);
    }
    String::from_utf8(received).expect("ascii")
}

#[tokio::test]
async fn identification_string_carries_the_banner_comment() {
    let mut config = ServerConfig::new(vec![UserConfig::new("alice")]);
    config.banner_comment = Some("Ubuntu-3ubuntu13".to_owned());
    let server = TestServer::start(config).await.expect("server starts");
    assert_eq!(
        read_ident(server.addr()).await,
        "SSH-2.0-shellpp_test_sshd Ubuntu-3ubuntu13\r\n"
    );

    let plain = password_server("bob", &random_secret()).await;
    assert_eq!(
        read_ident(plain.addr()).await,
        "SSH-2.0-shellpp_test_sshd\r\n"
    );
}

#[tokio::test]
async fn password_auth_accepts_the_right_password_and_keeps_offering_after_a_wrong_one() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let (mut handle, _) = connect(server.port()).await;

    let none = handle
        .authenticate_none("alice")
        .await
        .expect("none request");
    match none {
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => assert_eq!(remaining_methods.to_vec(), vec![MethodKind::Password]),
        russh::client::AuthResult::Success => panic!("none must be refused"),
    }

    let wrong = handle
        .authenticate_password("alice", format!("{password}x"))
        .await
        .expect("password request");
    match wrong {
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => assert!(remaining_methods.contains(&MethodKind::Password)),
        russh::client::AuthResult::Success => panic!("wrong password accepted"),
    }
    let unknown = handle
        .authenticate_password("mallory", password.as_str())
        .await
        .expect("password request");
    assert!(!unknown.success());
    let right = handle
        .authenticate_password("alice", password.as_str())
        .await
        .expect("password request");
    assert!(right.success());

    let events = server.events();
    assert_eq!(
        events,
        vec![
            auth_event("alice", "none", false),
            auth_event("alice", "password", false),
            auth_event("mallory", "password", false),
            auth_event("alice", "password", true),
        ]
    );
    // Events never carry the secret.
    let json = serde_json::to_string(&events).expect("json");
    assert!(!json.contains(&password));
}

#[tokio::test]
async fn publickey_auth_with_plain_encrypted_and_unauthorized_keys() {
    let passphrase = secret(&random_secret());
    let plain = generate_key(KeyType::Ed25519, "plain", None).expect("key");
    let encrypted = generate_key(KeyType::EcdsaP256, "enc", Some(&passphrase)).expect("key");
    let stranger = generate_key(KeyType::Ed25519, "stranger", None).expect("key");
    let server = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("alice")
            .with_authorized_key(&plain.public_openssh)
            .with_authorized_key(&encrypted.public_openssh),
    ]))
    .await
    .expect("server starts");

    let decode = |key: &shellpp_test_sshd::GeneratedKey, pass: Option<&str>| -> PrivateKey {
        use secrecy::ExposeSecret;
        decode_secret_key(key.private_openssh.expose_secret(), pass).expect("decodes")
    };

    let (mut handle, _) = connect(server.port()).await;
    assert!(!publickey_auth(&mut handle, "alice", decode(&stranger, None)).await);
    assert!(!publickey_auth(&mut handle, "bob", decode(&plain, None)).await);
    assert!(publickey_auth(&mut handle, "alice", decode(&plain, None)).await);

    let (mut handle, _) = connect(server.port()).await;
    {
        use secrecy::ExposeSecret;
        assert!(
            decode_secret_key(encrypted.private_openssh.expose_secret(), None).is_err(),
            "an encrypted key needs its passphrase"
        );
    }
    let key = {
        use secrecy::ExposeSecret;
        decode(&encrypted, Some(passphrase.expose_secret()))
    };
    assert!(publickey_auth(&mut handle, "alice", key).await);

    let events = server.events();
    assert_eq!(
        events,
        vec![
            auth_event("alice", "publickey", false),
            auth_event("bob", "publickey", false),
            auth_event("alice", "publickey", true),
            auth_event("alice", "publickey", true),
        ]
    );
}

#[tokio::test]
async fn publickey_auth_with_an_rsa_key_uses_sha2_signatures() {
    let key = generate_key(KeyType::Rsa3072, "rsa", None).expect("key");
    let server = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("alice").with_authorized_key(&key.public_openssh),
    ]))
    .await
    .expect("server starts");
    let (mut handle, _) = connect(server.port()).await;
    let private = {
        use secrecy::ExposeSecret;
        decode_secret_key(key.private_openssh.expose_secret(), None).expect("decodes")
    };
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .expect("query")
        .flatten();
    assert!(hash.is_some(), "the server offers rsa-sha2-256/512");
    assert!(publickey_auth(&mut handle, "alice", private).await);
}

#[tokio::test]
async fn keyboard_interactive_asks_all_questions_in_one_round_and_allows_three_rounds() {
    let code = random_secret();
    let pin = random_secret();
    // A password too, so that the connection survives the withdrawal of
    // keyboard-interactive (a client gives up when no method is left).
    let server = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("alice")
            .with_password(secret(&random_secret()))
            .with_keyboard_interactive(
                "Test server",
                "Answer both questions.",
                vec![
                    KiQuestion::new("Verification code: ", false, secret(&code)),
                    KiQuestion::new("Favourite color: ", true, secret(&pin)),
                ],
            ),
    ]))
    .await
    .expect("server starts");
    let (mut handle, _) = connect(server.port()).await;

    for round in 1..=3 {
        let response = handle
            .authenticate_keyboard_interactive_start("alice", None::<String>)
            .await
            .expect("ki start");
        let KeyboardInteractiveAuthResponse::InfoRequest {
            name,
            instructions,
            prompts,
        } = response
        else {
            panic!("expected questions in round {round}, got {response:?}");
        };
        assert_eq!(name, "Test server");
        assert_eq!(instructions, "Answer both questions.");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].prompt, "Verification code: ");
        assert!(!prompts[0].echo);
        assert_eq!(prompts[1].prompt, "Favourite color: ");
        assert!(prompts[1].echo);
        let answer = handle
            .authenticate_keyboard_interactive_respond(vec![code.clone(), "wrong".to_owned()])
            .await
            .expect("ki respond");
        let KeyboardInteractiveAuthResponse::Failure {
            remaining_methods, ..
        } = answer
        else {
            panic!("wrong answers must fail, got {answer:?}");
        };
        assert_eq!(
            remaining_methods.contains(&MethodKind::KeyboardInteractive),
            round < 3,
            "round {round}"
        );
        assert!(remaining_methods.contains(&MethodKind::Password));
    }
    // After three wrong rounds the method is withdrawn for this connection.
    let response = handle
        .authenticate_keyboard_interactive_start("alice", None::<String>)
        .await
        .expect("ki start");
    assert!(matches!(
        response,
        KeyboardInteractiveAuthResponse::Failure { .. }
    ));

    // A new connection answers right.
    let (mut handle, _) = connect(server.port()).await;
    let response = handle
        .authenticate_keyboard_interactive_start("alice", None::<String>)
        .await
        .expect("ki start");
    assert!(matches!(
        response,
        KeyboardInteractiveAuthResponse::InfoRequest { .. }
    ));
    let answer = handle
        .authenticate_keyboard_interactive_respond(vec![code.clone(), pin.clone()])
        .await
        .expect("ki respond");
    assert!(matches!(answer, KeyboardInteractiveAuthResponse::Success));

    let events = server.events();
    let ki: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, ServerEvent::Auth { method, .. } if method == "keyboard-interactive"))
        .cloned()
        .collect();
    assert_eq!(ki.len(), 5);
    assert_eq!(
        ki.last(),
        Some(&auth_event("alice", "keyboard-interactive", true))
    );
    let json = serde_json::to_string(&events).expect("json");
    assert!(!json.contains(&code) && !json.contains(&pin));
}

#[tokio::test]
async fn keyboard_interactive_only_user_fails_after_three_wrong_rounds() {
    let server = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("carol").with_keyboard_interactive(
            "",
            "",
            vec![KiQuestion::new("Code: ", false, secret(&random_secret()))],
        ),
    ]))
    .await
    .expect("server starts");
    let (mut handle, _) = connect(server.port()).await;
    let mut last_remaining = None;
    for _ in 0..3 {
        let response = handle
            .authenticate_keyboard_interactive_start("carol", None::<String>)
            .await
            .expect("ki start");
        assert!(matches!(
            response,
            KeyboardInteractiveAuthResponse::InfoRequest { .. }
        ));
        let answer = handle
            .authenticate_keyboard_interactive_respond(vec!["wrong".to_owned()])
            .await
            .expect("ki respond");
        if let KeyboardInteractiveAuthResponse::Failure {
            remaining_methods, ..
        } = answer
        {
            last_remaining = Some(remaining_methods);
        }
    }
    assert_eq!(last_remaining.map(|methods| methods.is_empty()), Some(true));
    eventually("the client gives up", || handle.is_closed()).await;
}

#[tokio::test]
async fn methods_offered_are_those_configured_and_the_auth_banner_is_sent() {
    let key = generate_key(KeyType::Ed25519, "k", None).expect("key");
    let mut config = ServerConfig::new(vec![
        UserConfig::new("a").with_password(secret(&random_secret())),
        UserConfig::new("b").with_authorized_key(&key.public_openssh),
        UserConfig::new("c").with_keyboard_interactive(
            "",
            "",
            vec![KiQuestion::new("Code: ", false, secret(&random_secret()))],
        ),
    ]);
    config.auth_banner = Some("Authorized use only\r\n".to_owned());
    let server = TestServer::start(config).await.expect("server starts");
    let (mut handle, observed) = connect(server.port()).await;
    let result = handle.authenticate_none("a").await.expect("none");
    let russh::client::AuthResult::Failure {
        remaining_methods, ..
    } = result
    else {
        panic!("none must be refused");
    };
    assert_eq!(
        remaining_methods.to_vec(),
        vec![
            MethodKind::PublicKey,
            MethodKind::Password,
            MethodKind::KeyboardInteractive
        ]
    );
    eventually("auth banner received", || observed.banner().is_some()).await;
    assert_eq!(
        observed.banner().as_deref(),
        Some("Authorized use only\r\n")
    );

    let only_keys = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("b").with_authorized_key(&key.public_openssh),
    ]))
    .await
    .expect("server starts");
    let (mut handle, observed) = connect(only_keys.port()).await;
    let result = handle.authenticate_none("b").await.expect("none");
    let russh::client::AuthResult::Failure {
        remaining_methods, ..
    } = result
    else {
        panic!("none must be refused");
    };
    assert_eq!(remaining_methods.to_vec(), vec![MethodKind::PublicKey]);
    assert_eq!(observed.banner(), None);
}

#[tokio::test]
async fn shell_prints_motd_and_prompt_and_runs_echo_and_exit_zero() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    let start = terminal.wait_for(&prompt("alice")).await;
    assert_eq!(
        start,
        "Welcome to shellpp-test-sshd\r\nalice@shellpp-test:~$ "
    );
    terminal.send("echo hello world\r").await;
    let output = terminal.wait_for(&prompt("alice")).await;
    assert_eq!(
        output,
        "echo hello world\r\nhello world\r\nalice@shellpp-test:~$ "
    );
    terminal.send("exit\r").await;
    assert_eq!(terminal.wait_end().await, Ending::ExitStatus(0));
    eventually("closed event", || {
        server.events().contains(&ServerEvent::Closed {
            exit_status: Some(0),
        })
    })
    .await;
    let events = server.events();
    assert!(events.contains(&ServerEvent::Pty {
        term: "xterm-256color".to_owned(),
        cols: 80,
        rows: 24
    }));
    assert!(events.contains(&ServerEvent::Shell));
}

#[tokio::test]
async fn shell_exit_with_a_status_and_a_custom_motd() {
    let password = random_secret();
    let mut config = ServerConfig::new(vec![
        UserConfig::new("bob").with_password(secret(&password)),
    ]);
    config.motd = Some("Custom MOTD\r\n".to_owned());
    let server = TestServer::start(config).await.expect("server starts");
    let handle = password_login(server.port(), "bob", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    assert_eq!(
        terminal.wait_for(&prompt("bob")).await,
        "Custom MOTD\r\nbob@shellpp-test:~$ "
    );
    terminal.send("exit 3\r").await;
    assert_eq!(terminal.wait_end().await, Ending::ExitStatus(3));
    eventually("closed event", || {
        server.events().contains(&ServerEvent::Closed {
            exit_status: Some(3),
        })
    })
    .await;
}

#[tokio::test]
async fn read_secret_turns_echo_off_for_one_line() {
    let password = random_secret();
    let typed = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;
    terminal.send("read-secret\r").await;
    terminal.wait_for(SECRET_PROMPT).await;
    terminal.send(&format!("{typed}\r")).await;
    let after = terminal.wait_for(&prompt("alice")).await;
    assert_eq!(after, "\r\nok\r\nalice@shellpp-test:~$ ");
    assert!(
        !terminal.text().contains(&typed),
        "the secret is never echoed"
    );
    terminal.send("echo visible\r").await;
    assert_eq!(
        terminal.wait_for(&prompt("alice")).await,
        "echo visible\r\nvisible\r\nalice@shellpp-test:~$ "
    );
}

#[tokio::test]
async fn size_term_and_env_follow_the_requests() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open_with(
        &handle,
        TerminalOptions {
            term: "xterm-256color",
            cols: 132,
            rows: 43,
            env: &[("LANG", "C.UTF-8"), ("SHELLPP_TEST", "yes")],
            agent_forwarding: false,
        },
    )
    .await;
    terminal.wait_for(&prompt("alice")).await;
    terminal.send("size\r").await;
    assert!(
        terminal
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\n132x43\r\n")
    );
    terminal.resize(100, 30).await;
    terminal.send("size\r").await;
    assert!(
        terminal
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\n100x30\r\n")
    );
    terminal.send("term\r").await;
    assert!(
        terminal
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\nxterm-256color\r\n")
    );
    terminal.send("env LANG\r").await;
    assert!(
        terminal
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\nC.UTF-8\r\n")
    );
    terminal.send("env NOPE\r").await;
    assert_eq!(
        terminal.wait_for(&prompt("alice")).await,
        "env NOPE\r\n\r\nalice@shellpp-test:~$ "
    );

    let events = server.events();
    assert!(events.contains(&ServerEvent::Pty {
        term: "xterm-256color".to_owned(),
        cols: 132,
        rows: 43
    }));
    assert!(events.contains(&ServerEvent::WindowChange {
        cols: 100,
        rows: 30
    }));
    assert!(events.contains(&ServerEvent::Env {
        name: "LANG".to_owned()
    }));
    assert!(events.contains(&ServerEvent::Env {
        name: "SHELLPP_TEST".to_owned()
    }));
}

#[tokio::test]
async fn several_sessions_on_one_connection_are_independent() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    let mut first = Terminal::open(&handle).await;
    let mut second = Terminal::open(&handle).await;
    first.wait_for(&prompt("alice")).await;
    second.wait_for(&prompt("alice")).await;
    first.send("echo one\r").await;
    second.send("echo two\r").await;
    assert!(
        first
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\none\r\n")
    );
    assert!(
        second
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\ntwo\r\n")
    );
    assert!(!first.text().contains("two"));
    second.send("exit 5\r").await;
    assert_eq!(second.wait_end().await, Ending::ExitStatus(5));
    first.send("echo still\r").await;
    assert!(
        first
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\nstill\r\n")
    );
}

#[tokio::test]
async fn later_and_sleep_run_on_timers() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;
    terminal.send("later 0.3 marker-text\r").await;
    assert_eq!(
        terminal.wait_for(&prompt("alice")).await,
        "later 0.3 marker-text\r\nalice@shellpp-test:~$ "
    );
    assert_eq!(
        terminal.wait_for("marker-text\r\n").await,
        "\r\nmarker-text\r\n"
    );
    terminal.wait_for(&prompt("alice")).await;

    terminal.send("sleep 0.3\recho after\r").await;
    let output = terminal.wait_for("\r\nafter\r\n").await;
    assert_eq!(
        output,
        "sleep 0.3\r\nalice@shellpp-test:~$ echo after\r\nafter\r\n"
    );
}

#[tokio::test]
async fn exec_answers_with_the_configured_output_and_status() {
    let password = random_secret();
    let mut config = ServerConfig::new(vec![
        UserConfig::new("alice").with_password(secret(&password)),
    ]);
    config.exec = Some(ExecAnswer {
        stdout: "Linux\nID=debian\nVERSION_ID=12\n".to_owned(),
        exit_status: 0,
    });
    let server = TestServer::start(config).await.expect("server starts");
    let handle = password_login(server.port(), "alice", &password).await;
    let result = exec(&handle, "uname -s; cat /etc/os-release").await;
    assert!(result.accepted);
    assert_eq!(result.stdout, "Linux\nID=debian\nVERSION_ID=12\n");
    assert_eq!(result.exit_status, Some(0));
    eventually("exec events", || {
        let events = server.events();
        events.contains(&ServerEvent::Exec {
            command: "uname -s; cat /etc/os-release".to_owned(),
        }) && events.contains(&ServerEvent::Closed {
            exit_status: Some(0),
        })
    })
    .await;
    // A shell on the same connection still works.
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;
}

#[tokio::test]
async fn exec_is_refused_without_a_configured_answer() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    let result = exec(&handle, "uname").await;
    assert!(!result.accepted);
    assert_eq!(result.exit_status, None);
    assert!(server.events().contains(&ServerEvent::Exec {
        command: "uname".to_owned()
    }));
}

#[tokio::test]
async fn agent_keys_asks_the_forwarded_agent() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let agent_keys = vec![
        generate_key(KeyType::Ed25519, "a", None).expect("key"),
        generate_key(KeyType::EcdsaP256, "b", None).expect("key"),
    ]
    .into_iter()
    .map(|key| russh::keys::PublicKey::from_openssh(&key.public_openssh).expect("public key"))
    .collect();
    let (mut handle, _) = connect_with(
        server.port(),
        ClientOptions {
            agent_keys: Some(agent_keys),
            keepalive: None,
        },
    )
    .await;
    assert!(
        handle
            .authenticate_password("alice", password.as_str())
            .await
            .expect("auth")
            .success()
    );

    let mut plain = Terminal::open(&handle).await;
    plain.wait_for(&prompt("alice")).await;
    plain.send("agent-keys\r").await;
    assert!(
        plain
            .wait_for(&prompt("alice"))
            .await
            .contains(&format!("\r\n{AGENT_NOT_FORWARDED}\r\n"))
    );

    let mut forwarded = Terminal::open_with(
        &handle,
        TerminalOptions {
            agent_forwarding: true,
            ..TerminalOptions::default()
        },
    )
    .await;
    forwarded.wait_for(&prompt("alice")).await;
    forwarded.send("agent-keys\r").await;
    assert!(
        forwarded
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\nagent keys: 2\r\n")
    );
    assert!(server.events().contains(&ServerEvent::AgentForward));
}

#[tokio::test]
async fn drop_command_closes_the_connection_without_exit_status() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;
    terminal.send("drop\r").await;
    assert_eq!(terminal.wait_end().await, Ending::WithoutStatus);
    eventually("client sees the closed connection", || handle.is_closed()).await;
    eventually("closed without status", || {
        server
            .events()
            .contains(&ServerEvent::Closed { exit_status: None })
    })
    .await;
    eventually("server forgets the connection", || {
        server.connection_count() == 0
    })
    .await;
}

#[tokio::test]
async fn drop_connections_closes_every_connection_without_exit_status() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let first = password_login(server.port(), "alice", &password).await;
    let second = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&first).await;
    terminal.wait_for(&prompt("alice")).await;
    assert_eq!(server.connection_count(), 2);

    assert_eq!(server.drop_connections(), 2);
    assert_eq!(terminal.wait_end().await, Ending::WithoutStatus);
    eventually("clients see the drop", || {
        first.is_closed() && second.is_closed()
    })
    .await;
    eventually("server forgets the connections", || {
        server.connection_count() == 0
    })
    .await;
    assert_eq!(server.drop_connections(), 0);

    // The server keeps accepting.
    let again = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&again).await;
    terminal.wait_for(&prompt("alice")).await;
}

#[tokio::test]
async fn rotate_host_key_drops_connections_and_keeps_the_port() {
    let password = random_secret();
    let mut config = ServerConfig::new(vec![
        UserConfig::new("alice").with_password(secret(&password)),
    ]);
    config.host_key = HostKeySource::Generate(KeyType::EcdsaP256);
    let server = TestServer::start(config).await.expect("server starts");
    let port = server.port();
    let old_fingerprint = server.host_key_fingerprint();
    let old_base64 = server.host_key_base64();
    assert_eq!(server.host_key_type(), "ecdsa-sha2-nistp256");

    let handle = password_login(port, "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;

    server.rotate_host_key().await.expect("rotation");
    assert_eq!(terminal.wait_end().await, Ending::WithoutStatus);
    assert_eq!(server.port(), port);
    assert_ne!(server.host_key_fingerprint(), old_fingerprint);
    assert_ne!(server.host_key_base64(), old_base64);
    assert_eq!(server.host_key_type(), "ecdsa-sha2-nistp256");

    let (mut handle, observed) = connect(port).await;
    assert_eq!(
        observed
            .server_key()
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string(),
        server.host_key_fingerprint()
    );
    assert!(
        handle
            .authenticate_password("alice", password.as_str())
            .await
            .expect("auth")
            .success()
    );
}

#[tokio::test]
async fn host_key_from_a_file_and_rotation_keeps_its_type() {
    let directory = tempfile::tempdir().expect("temp dir");
    let key = generate_key(KeyType::Ed25519, "", None).expect("key");
    let path = directory.path().join("host_key");
    {
        use secrecy::ExposeSecret;
        std::fs::write(&path, key.private_openssh.expose_secret()).expect("write key");
    }
    let mut config = ServerConfig::new(vec![UserConfig::new("alice")]);
    config.host_key = HostKeySource::File(path);
    let server = TestServer::start(config).await.expect("server starts");
    assert_eq!(server.host_key_fingerprint(), key.fingerprint);
    server.rotate_host_key().await.expect("rotation");
    assert_ne!(server.host_key_fingerprint(), key.fingerprint);
    assert_eq!(server.host_key_type(), "ssh-ed25519");

    let encrypted =
        generate_key(KeyType::Ed25519, "", Some(&secret(&random_secret()))).expect("key");
    let path = directory.path().join("encrypted");
    {
        use secrecy::ExposeSecret;
        std::fs::write(&path, encrypted.private_openssh.expose_secret()).expect("write key");
    }
    let mut config = ServerConfig::new(vec![UserConfig::new("alice")]);
    config.host_key = HostKeySource::File(path);
    assert!(
        TestServer::start(config).await.is_err(),
        "encrypted host keys are refused"
    );
}

#[tokio::test]
async fn set_accepting_closes_and_reopens_the_port_and_keeps_open_sessions() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let addr = server.addr();
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;

    server.set_accepting(false).await.expect("close listener");
    server.set_accepting(false).await.expect("idempotent");
    let refused = tokio::net::TcpStream::connect(addr).await;
    assert!(refused.is_err(), "connections are refused");

    // The open session keeps working.
    terminal.send("echo alive\r").await;
    assert!(
        terminal
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\nalive\r\n")
    );

    server.set_accepting(true).await.expect("reopen listener");
    server.set_accepting(true).await.expect("idempotent");
    assert_eq!(server.addr(), addr);
    let again = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&again).await;
    terminal.wait_for(&prompt("alice")).await;
}

#[tokio::test]
async fn set_accepting_reopens_after_dropped_connections() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let handle = password_login(server.port(), "alice", &password).await;
    server.set_accepting(false).await.expect("close listener");
    assert_eq!(server.drop_connections(), 1);
    eventually("client sees the drop", || handle.is_closed()).await;
    server.set_accepting(true).await.expect("reopen listener");
    let _again = password_login(server.port(), "alice", &password).await;
}

#[tokio::test]
async fn held_connections_never_see_the_banner() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    server.set_holding(true);
    let mut stream = tokio::net::TcpStream::connect(server.addr())
        .await
        .expect("accepted");
    let mut buf = [0u8; 16];
    let read = tokio::time::timeout(
        Duration::from_millis(500),
        tokio::io::AsyncReadExt::read(&mut stream, &mut buf),
    )
    .await;
    assert!(read.is_err(), "no identification while held");
    eventually("the held connection is counted", || {
        server.connection_count() == 1
    })
    .await;
    assert_eq!(server.drop_connections(), 1);
    server.set_holding(false);
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;
}

#[tokio::test]
async fn frozen_connections_stop_answering_keepalives() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let (mut handle, _) = connect_with(
        server.port(),
        ClientOptions {
            agent_keys: None,
            keepalive: Some(Duration::from_millis(300)),
        },
    )
    .await;
    assert!(
        handle
            .authenticate_password("alice", password.as_str())
            .await
            .expect("auth")
            .success()
    );
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;

    assert_eq!(server.freeze_connections(), 1);
    // The client's keepalives go unanswered: it gives up on its own.
    assert_eq!(terminal.wait_end().await, Ending::WithoutStatus);
    eventually("client detects the keepalive timeout", || {
        handle.is_closed()
    })
    .await;

    assert_eq!(server.drop_connections(), 1);
    eventually("server forgets the connection", || {
        server.connection_count() == 0
    })
    .await;
}

fn keepalives(server: &TestServer) -> usize {
    server
        .events()
        .iter()
        .filter(|event| **event == ServerEvent::Keepalive)
        .count()
}

#[tokio::test]
async fn keepalives_are_answered_and_recorded() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let (mut handle, _) = connect_with(
        server.port(),
        ClientOptions {
            agent_keys: None,
            keepalive: Some(Duration::from_millis(200)),
        },
    )
    .await;
    assert!(
        handle
            .authenticate_password("alice", password.as_str())
            .await
            .expect("auth")
            .success()
    );
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;
    eventually("three keepalives recorded", || keepalives(&server) >= 3).await;
    // Answered: the client (keepalive_max 2) is still connected.
    assert!(!handle.is_closed());
    terminal.send("echo still\r").await;
    assert!(
        terminal
            .wait_for(&prompt("alice"))
            .await
            .contains("\r\nstill\r\n")
    );
}

#[tokio::test]
async fn ordinary_traffic_is_not_taken_for_keepalives() {
    let password = random_secret();
    let mut config = ServerConfig::new(vec![
        UserConfig::new("alice").with_password(secret(&password)),
    ]);
    config.exec = Some(ExecAnswer {
        stdout: "Linux\n".to_owned(),
        exit_status: 0,
    });
    let server = TestServer::start(config).await.expect("server starts");
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open_with(
        &handle,
        TerminalOptions {
            env: &[("LANG", "C.UTF-8")],
            ..TerminalOptions::default()
        },
    )
    .await;
    terminal.wait_for(&prompt("alice")).await;
    for input in [
        "echo a\r",
        "lines 300\r",
        "size\r",
        "env LANG\r",
        "agent-keys\r",
        "color\r",
    ] {
        terminal.send(input).await;
        terminal.wait_for(&prompt("alice")).await;
    }
    for character in "echo typed one key at a time\r".chars() {
        terminal.send(&character.to_string()).await;
    }
    terminal.wait_for("\r\ntyped one key at a time\r\n").await;
    terminal.resize(90, 30).await;
    terminal.send("later 0.2 tick\rsleep 0.2\r").await;
    terminal.wait_for("tick\r\n").await;
    terminal.send("read-secret\r").await;
    terminal.wait_for(SECRET_PROMPT).await;
    terminal.send("x\r").await;
    terminal.wait_for("ok\r\n").await;
    let result = exec(&handle, "uname").await;
    assert_eq!(result.exit_status, Some(0));
    let mut second = Terminal::open(&handle).await;
    second.wait_for(&prompt("alice")).await;
    second.send("exit 1\r").await;
    assert_eq!(second.wait_end().await, Ending::ExitStatus(1));
    terminal.send("exit\r").await;
    assert_eq!(terminal.wait_end().await, Ending::ExitStatus(0));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(keepalives(&server), 0, "events: {:?}", server.events());
}

#[tokio::test]
async fn dropping_the_server_stops_everything() {
    let password = random_secret();
    let server = password_server("alice", &password).await;
    let addr = server.addr();
    let handle = password_login(server.port(), "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    terminal.wait_for(&prompt("alice")).await;
    drop(server);
    assert_eq!(terminal.wait_end().await, Ending::WithoutStatus);
    let deadline = tokio::time::Instant::now() + common::WAIT;
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "listener still open"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
