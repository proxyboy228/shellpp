//! Authentication tests against the in-process russh test server: password
//! (saved, prompted, wrong then right, the save flag), the username prompt,
//! public keys (plain, encrypted with prompted and saved passphrases,
//! unusable), the system agent, keyboard-interactive with masked questions
//! and refused rounds, cancellation and the final failure kinds.

mod support;

use std::time::Duration;

use secrecy::ExposeSecret;
use shellpp_ssh::{AuthMethod, AuthPlan, FailureKind, KeyCredential, Prompt, SystemAgent, connect};
use shellpp_test_sshd::{KeyType, ServerConfig, ServerEvent, UserConfig, generate_key};
use support::agent::TestAgent;
use support::*;

fn key_plan(private_openssh: &str, passphrase: Option<&str>) -> AuthPlan {
    AuthPlan {
        key: Some(KeyCredential {
            label: "Work key".to_owned(),
            private_key: secret(private_openssh),
            passphrase: passphrase.map(secret),
        }),
        ..AuthPlan::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_password_needs_no_prompt() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(connection.auth().method, AuthMethod::Password);
    assert!(connection.take_prompted_secrets().password.is_none());
    assert!(interaction.prompts().is_empty());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_prompted_password_is_returned_with_its_save_flag() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::answering(vec![saved(&password)]);
    let plan = AuthPlan {
        can_save_password: true,
        ..AuthPlan::default()
    };
    let connection = connect(
        spec(server.addr(), Some("alice"), plan),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(
        interaction.prompts(),
        vec![Prompt::Password {
            username: "alice".to_owned(),
            error: false,
            can_save: true
        }]
    );
    let prompted = connection
        .take_prompted_secrets()
        .password
        .expect("prompted password kept for reconnects");
    assert!(prompted.save);
    assert_eq!(prompted.value.expose_secret(), password);
    // Handed out once: the connection keeps no copy.
    let again = connection.take_prompted_secrets();
    assert!(again.password.is_none() && again.passphrase.is_none());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_is_asked_again_with_an_error() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let wrong = random_secret("wrong");
    let interaction = TestInteraction::answering(vec![text(&wrong), text(&password)]);
    let connection = connect(
        spec(
            server.addr(),
            Some("alice"),
            password_plan(&random_secret("stale")),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects on the third password");
    let prompts = interaction.prompts();
    assert_eq!(prompts.len(), 2);
    for prompt in prompts {
        assert_eq!(
            prompt,
            Prompt::Password {
                username: "alice".to_owned(),
                error: true,
                can_save: false
            }
        );
    }
    let prompted = connection
        .take_prompted_secrets()
        .password
        .expect("prompted");
    assert!(!prompted.save);
    let refused = server
        .events()
        .into_iter()
        .filter(|event| {
            matches!(event, ServerEvent::Auth { method, ok: false, .. } if method == "password")
        })
        .count();
    assert_eq!(refused, 2);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_prompt_cancels_the_attempt() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::answering(vec![Answer::Cancel]);
    let error = connect(
        spec(server.addr(), Some("alice"), AuthPlan::default()),
        interaction.as_dyn(),
    )
    .await
    .expect_err("cancelled");
    assert_eq!(error.kind, FailureKind::Cancelled);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_username_is_asked_first() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::answering(vec![text("alice"), text(&password)]);
    let connection = connect(
        spec(server.addr(), None, AuthPlan::default()),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    let prompts = interaction.prompts();
    assert_eq!(prompts[0], Prompt::Username);
    assert!(matches!(&prompts[1], Prompt::Password { username, .. } if username == "alice"));
    assert_eq!(connection.auth().username, "alice");
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_ed25519_key_authenticates() {
    let key = generate_key(KeyType::Ed25519, "shellpp-test", None).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(key.private_openssh.expose_secret(), None),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    assert!(connection.take_prompted_secrets().passphrase.is_none());
    assert!(interaction.prompts().is_empty());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ecdsa_key_authenticates() {
    let key = generate_key(KeyType::EcdsaP256, "shellpp-test", None).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let connection = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(key.private_openssh.expose_secret(), None),
        ),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    connection.disconnect().await;
    server.shutdown().await;
}

/// RSA keys sign with rsa-sha2-512/256, never ssh-rsa (decision 22): the
/// russh server negotiates from its `server-sig-algs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_rsa_key_signs_with_sha2() {
    let key = generate_key(KeyType::Rsa3072, "shellpp-test", None).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let connection = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(key.private_openssh.expose_secret(), None),
        ),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects with rsa-sha2-*");
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_encrypted_key_asks_for_the_passphrase_until_it_opens() {
    let passphrase = random_secret("pass");
    let key =
        generate_key(KeyType::Ed25519, "shellpp-test", Some(&secret(&passphrase))).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let interaction =
        TestInteraction::answering(vec![text(&random_secret("wrong")), saved(&passphrase)]);
    let mut plan = key_plan(key.private_openssh.expose_secret(), None);
    plan.can_save_passphrase = true;
    let connection = connect(spec(server.addr(), Some("bob"), plan), interaction.as_dyn())
        .await
        .expect("connects");
    assert_eq!(
        interaction.prompts(),
        vec![
            Prompt::Passphrase {
                key_label: "Work key".to_owned(),
                error: false,
                can_save: true
            },
            Prompt::Passphrase {
                key_label: "Work key".to_owned(),
                error: true,
                can_save: true
            },
        ]
    );
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    let prompted = connection
        .take_prompted_secrets()
        .passphrase
        .expect("prompted passphrase");
    assert!(prompted.save);
    assert_eq!(prompted.value.expose_secret(), passphrase);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_passphrase_opens_the_key_without_a_prompt() {
    let passphrase = random_secret("pass");
    let key =
        generate_key(KeyType::Ed25519, "shellpp-test", Some(&secret(&passphrase))).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(key.private_openssh.expose_secret(), Some(&passphrase)),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert!(interaction.prompts().is_empty());
    assert!(connection.take_prompted_secrets().passphrase.is_none());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_passphrase_prompt_cancels_the_attempt() {
    let passphrase = random_secret("pass");
    let key =
        generate_key(KeyType::Ed25519, "shellpp-test", Some(&secret(&passphrase))).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let interaction = TestInteraction::answering(vec![Answer::Cancel]);
    let error = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(key.private_openssh.expose_secret(), None),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect_err("cancelled");
    assert_eq!(error.kind, FailureKind::Cancelled);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unusable_key_falls_back_to_the_password() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let mut plan = key_plan("not a private key", None);
    plan.password = Some(secret(&password));
    let connection = connect(
        spec(server.addr(), Some("alice"), plan),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("password still works");
    assert_eq!(connection.auth().method, AuthMethod::Password);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unusable_key_is_reported_when_nothing_else_works() {
    let key = generate_key(KeyType::Ed25519, "shellpp-test", None).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let error = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
                None,
            ),
        ),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect_err("no usable credential");
    assert_eq!(error.kind, FailureKind::KeyUnusable);
    server.shutdown().await;
}

/// The OpenSSH text of an encrypted key with its bcrypt-pbkdf rounds field
/// set to `rounds` (a crafted key file; nothing is re-encrypted).
fn with_kdf_rounds(text: &str, rounds: u32) -> String {
    use data_encoding::BASE64;
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let mut bytes = BASE64.decode(body.as_bytes()).expect("base64 body");
    let read_u32 = |bytes: &[u8], at: usize| {
        u32::from_be_bytes(bytes[at..at + 4].try_into().expect("u32")) as usize
    };
    // "openssh-key-v1\0", string cipher, string kdf name, string kdf options
    // (string salt, uint32 rounds).
    let mut at = b"openssh-key-v1\0".len();
    at += 4 + read_u32(&bytes, at);
    at += 4 + read_u32(&bytes, at);
    at += 4;
    at += 4 + read_u32(&bytes, at);
    bytes[at..at + 4].copy_from_slice(&rounds.to_be_bytes());
    let encoded = BASE64.encode(&bytes);
    let mut out = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
    for chunk in encoded.as_bytes().chunks(70) {
        out.push_str(std::str::from_utf8(chunk).expect("ascii"));
        out.push('\n');
    }
    out.push_str("-----END OPENSSH PRIVATE KEY-----\n");
    out
}

/// A key file that declares an absurd bcrypt-pbkdf rounds count is never
/// decrypted (the KDF cannot be interrupted): the key is unusable at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_with_too_many_kdf_rounds_is_unusable() {
    let passphrase = random_secret("pass");
    let key =
        generate_key(KeyType::Ed25519, "shellpp-test", Some(&secret(&passphrase))).expect("key");
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let crafted = with_kdf_rounds(key.private_openssh.expose_secret(), u32::MAX);
    let started = std::time::Instant::now();
    let error = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(&crafted, Some(&passphrase)),
        ),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect_err("the key is refused");
    assert_eq!(error.kind, FailureKind::KeyUnusable);
    assert!(started.elapsed() < Duration::from_secs(10));
    // The same key with its normal rounds still opens.
    let normal = with_kdf_rounds(key.private_openssh.expose_secret(), 16);
    let connection = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(&normal, Some(&passphrase)),
        ),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unauthorized_key_fails_authentication() {
    let authorized = generate_key(KeyType::Ed25519, "authorized", None).expect("key");
    let other = generate_key(KeyType::Ed25519, "other", None).expect("key");
    let server = start_server(vec![key_user("bob", &authorized.public_openssh)]).await;
    let error = connect(
        spec(
            server.addr(),
            Some("bob"),
            key_plan(other.private_openssh.expose_secret(), None),
        ),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect_err("refused");
    assert_eq!(error.kind, FailureKind::AuthFailed);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_system_agent_signs_for_its_keys() {
    let key = generate_key(KeyType::Ed25519, "agent", None).expect("key");
    let private =
        russh::keys::decode_secret_key(key.private_openssh.expose_secret(), None).expect("decodes");
    let decoy = generate_key(KeyType::Ed25519, "decoy", None).expect("key");
    let decoy = russh::keys::decode_secret_key(decoy.private_openssh.expose_secret(), None)
        .expect("decodes");
    let agent = TestAgent::start(vec![decoy, private]).await;
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let mut spec = spec(
        server.addr(),
        Some("bob"),
        AuthPlan {
            use_agent: true,
            ..AuthPlan::default()
        },
    );
    spec.agent = Some(SystemAgent::at(agent.location()));
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("agent authentication");
    assert_eq!(connection.auth().method, AuthMethod::Agent);
    assert_eq!(agent.signatures(), 1);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_agent_is_not_used_unless_the_plan_says_so() {
    let key = generate_key(KeyType::Ed25519, "agent", None).expect("key");
    let private =
        russh::keys::decode_secret_key(key.private_openssh.expose_secret(), None).expect("decodes");
    let agent = TestAgent::start(vec![private]).await;
    let server = start_server(vec![key_user("bob", &key.public_openssh)]).await;
    let mut spec = spec(server.addr(), Some("bob"), AuthPlan::default());
    spec.agent = Some(SystemAgent::at(agent.location()));
    let error = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect_err("no method left");
    assert_eq!(error.kind, FailureKind::AuthFailed);
    assert_eq!(agent.requests(), 0);
    server.shutdown().await;
}

fn ki_user(name: &str, answers: [&str; 2]) -> UserConfig {
    UserConfig {
        keyboard_interactive: vec![
            question("Password: ", false, answers[0]),
            question("Verification code: ", true, answers[1]),
        ],
        ki_name: "shellpp-test".to_owned(),
        ki_instruction: "Answer both questions".to_owned(),
        ..user(name)
    }
}

fn ki_prompt(prompt: &str, echo: bool, index: u32, error: bool) -> Prompt {
    Prompt::KeyboardInteractive {
        name: "shellpp-test".to_owned(),
        instruction: "Answer both questions".to_owned(),
        prompt: prompt.to_owned(),
        echo,
        index,
        count: 2,
        error,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keyboard_interactive_asks_each_question_with_its_echo_flag() {
    let (first, second) = (random_secret("a1"), random_secret("a2"));
    let server = start_server(vec![ki_user("carol", [&first, &second])]).await;
    let interaction = TestInteraction::answering(vec![text(&first), text(&second)]);
    let plan = AuthPlan {
        keyboard_interactive: true,
        ..AuthPlan::default()
    };
    let connection = connect(
        spec(server.addr(), Some("carol"), plan),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(connection.auth().method, AuthMethod::KeyboardInteractive);
    assert_eq!(
        interaction.prompts(),
        vec![
            ki_prompt("Password: ", false, 0, false),
            ki_prompt("Verification code: ", true, 1, false),
        ]
    );
    connection.disconnect().await;
    server.shutdown().await;
}

/// Server texts of keyboard-interactive rounds reach the user without
/// control, bidi or invisible characters and cut to a bounded length; a
/// round with too many questions fails the attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keyboard_interactive_server_texts_are_cleaned_and_bounded() {
    let answer = random_secret("a");
    let server = start_server(vec![
        UserConfig {
            keyboard_interactive: vec![question("Code\u{1b}[2J\u{202e}: ", false, &answer)],
            ki_name: format!("n\u{7}{}", "x".repeat(400)),
            ki_instruction: "line one\nline\u{200b}two\r".to_owned(),
            ..user("dave")
        },
        UserConfig {
            keyboard_interactive: (0..33)
                .map(|n| question(&format!("Q{n}: "), true, &answer))
                .collect(),
            ..user("erin")
        },
    ])
    .await;
    let plan = AuthPlan {
        keyboard_interactive: true,
        ..AuthPlan::default()
    };
    let interaction = TestInteraction::answering(vec![text(&answer)]);
    let connection = connect(
        spec(server.addr(), Some("dave"), plan.clone()),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(
        interaction.prompts(),
        vec![Prompt::KeyboardInteractive {
            name: format!("n{}", "x".repeat(255)),
            instruction: "line one\nlinetwo".to_owned(),
            prompt: "Code[2J: ".to_owned(),
            echo: false,
            index: 0,
            count: 1,
            error: false,
        }]
    );
    connection.disconnect().await;

    let interaction = TestInteraction::answering(vec![text(&answer)]);
    let error = connect(
        spec(server.addr(), Some("erin"), plan),
        interaction.as_dyn(),
    )
    .await
    .expect_err("too many questions");
    assert_eq!(error.kind, FailureKind::Protocol);
    assert!(interaction.prompts().is_empty());
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_keyboard_interactive_round_is_asked_again_with_an_error() {
    let (first, second) = (random_secret("a1"), random_secret("a2"));
    let server = start_server(vec![ki_user("carol", [&first, &second])]).await;
    let interaction = TestInteraction::answering(vec![
        text("wrong-1"),
        text("wrong-2"),
        text(&first),
        text(&second),
    ]);
    let plan = AuthPlan {
        keyboard_interactive: true,
        ..AuthPlan::default()
    };
    let connection = connect(
        spec(server.addr(), Some("carol"), plan),
        interaction.as_dyn(),
    )
    .await
    .expect("connects in the second round");
    assert_eq!(
        interaction.prompts(),
        vec![
            ki_prompt("Password: ", false, 0, false),
            ki_prompt("Verification code: ", true, 1, false),
            ki_prompt("Password: ", false, 0, true),
            ki_prompt("Verification code: ", true, 1, true),
        ]
    );
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keyboard_interactive_is_not_used_unless_the_plan_says_so() {
    let server = start_server(vec![ki_user("carol", ["x", "y"])]).await;
    let interaction = TestInteraction::answering(vec![text("x"), text("y")]);
    let error = connect(
        spec(server.addr(), Some("carol"), AuthPlan::default()),
        interaction.as_dyn(),
    )
    .await
    .expect_err("nothing to try");
    assert_eq!(error.kind, FailureKind::AuthFailed);
    assert!(interaction.prompts().is_empty());
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_key_comes_before_the_password() {
    let key = generate_key(KeyType::Ed25519, "shellpp-test", None).expect("key");
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![UserConfig {
        password: Some(secret(&password)),
        authorized_keys: vec![key.public_openssh.clone()],
        ..user("erin")
    }]);
    config.motd = Some("hi\r\n".to_owned());
    let server = start_with(config).await;
    let mut plan = key_plan(key.private_openssh.expose_secret(), None);
    plan.password = Some(secret(&password));
    let connection = connect(
        spec(server.addr(), Some("erin"), plan),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    let password_attempts = server
        .events()
        .into_iter()
        .filter(|event| matches!(event, ServerEvent::Auth { method, .. } if method == "password"))
        .count();
    assert_eq!(password_attempts, 0);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answer_that_never_comes_can_be_bounded_by_the_caller() {
    // The crate waits for prompts as long as the user needs; the host bounds
    // or cancels attempts. Dropping the connect future cancels it cleanly.
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    struct Never;
    impl shellpp_ssh::Interaction for Never {
        fn step(&self, _step: shellpp_ssh::ConnectStep) {}
        fn verify_host_key<'a>(
            &'a self,
            _key: &'a shellpp_ssh::HostKeyInfo,
        ) -> shellpp_ssh::BoxFuture<'a, shellpp_ssh::HostKeyDecision> {
            Box::pin(async { shellpp_ssh::HostKeyDecision::Accept })
        }
        fn prompt<'a>(
            &'a self,
            _prompt: Prompt,
        ) -> shellpp_ssh::BoxFuture<'a, Option<shellpp_ssh::PromptAnswer>> {
            Box::pin(std::future::pending())
        }
    }
    let attempt = connect(
        spec(server.addr(), Some("alice"), AuthPlan::default()),
        std::sync::Arc::new(Never),
    );
    let result = tokio::time::timeout(Duration::from_millis(800), attempt).await;
    assert!(result.is_err(), "still waiting for the password");
    server.shutdown().await;
}
