//! Key files (`~/.ssh/id_*`, [`shellpp_ssh::FileKey`]) against the in-process
//! russh test server: a key is offered by its public key and decrypted only
//! after the server accepted it, so a passphrase is asked only for a key the
//! server takes; wrong passphrases are asked again, a remembered one is used
//! without asking, a cancelled prompt cancels the attempt, refused keys fall
//! through to the password, and a key offered once is not offered again.
//! Keys are generated at run time; nothing is read from the real `~/.ssh`.

mod support;

use secrecy::ExposeSecret;
use shellpp_ssh::{AuthMethod, AuthPlan, FailureKind, FileKey, KeyCredential, Prompt, connect};
use shellpp_test_sshd::{GeneratedKey, KeyType, ServerEvent, UserConfig, generate_key};
use support::*;

fn key(passphrase: Option<&str>) -> GeneratedKey {
    generate_key(
        KeyType::Ed25519,
        "shellpp-test",
        passphrase.map(secret).as_ref(),
    )
    .expect("key")
}

fn file_key(label: &str, key: &GeneratedKey, remembered: Option<&str>) -> FileKey {
    FileKey {
        label: label.to_owned(),
        private_key: key.private_openssh.clone(),
        public_key: None,
        passphrase: remembered.map(secret),
    }
}

fn file_plan(keys: Vec<FileKey>) -> AuthPlan {
    AuthPlan {
        file_keys: keys,
        ..AuthPlan::default()
    }
}

fn publickey_failures(events: &[ServerEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(event, ServerEvent::Auth { method, ok: false, .. } if method == "publickey")
        })
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_key_file_connects_without_prompts() {
    let plain = key(None);
    let server = start_server(vec![key_user("bob", &plain.public_openssh)]).await;
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(
            server.addr(),
            Some("bob"),
            file_plan(vec![file_key("id_ed25519", &plain, None)]),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    assert!(interaction.prompts().is_empty());
    assert!(connection.take_prompted_secrets().file_passphrase.is_none());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accepted_encrypted_key_file_asks_for_its_passphrase_until_it_opens() {
    let passphrase = random_secret("pass");
    let other = key(None);
    let encrypted = key(Some(&passphrase));
    let server = start_server(vec![key_user("bob", &encrypted.public_openssh)]).await;
    let interaction =
        TestInteraction::answering(vec![text(&random_secret("wrong")), saved(&passphrase)]);
    // The first file is not authorized: it is refused without a prompt.
    let plan = file_plan(vec![
        file_key("id_other", &other, None),
        file_key("id_ed25519", &encrypted, None),
    ]);
    let connection = connect(spec(server.addr(), Some("bob"), plan), interaction.as_dyn())
        .await
        .expect("connects");
    assert_eq!(
        interaction.prompts(),
        vec![
            Prompt::FileKeyPassphrase {
                key_label: "id_ed25519".to_owned(),
                error: false
            },
            Prompt::FileKeyPassphrase {
                key_label: "id_ed25519".to_owned(),
                error: true
            },
        ]
    );
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    let (index, answer) = connection
        .take_prompted_secrets()
        .file_passphrase
        .expect("prompted passphrase");
    assert_eq!(index, 1);
    assert!(answer.save);
    assert_eq!(answer.value.expose_secret(), passphrase);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_encrypted_key_file_is_never_decrypted() {
    let password = random_secret("pw");
    let encrypted = key(Some(&random_secret("pass")));
    // Another user's key makes the server offer public keys.
    let server = start_server(vec![
        password_user("alice", &password),
        key_user("carol", &key(None).public_openssh),
    ])
    .await;
    let interaction = TestInteraction::answering(vec![text(&password)]);
    let connection = connect(
        spec(
            server.addr(),
            Some("alice"),
            file_plan(vec![file_key("id_ed25519", &encrypted, None)]),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects with the password");
    assert_eq!(connection.auth().method, AuthMethod::Password);
    assert_eq!(
        interaction.prompts(),
        vec![Prompt::Password {
            username: "alice".to_owned(),
            error: false,
            can_save: false
        }]
    );
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remembered_passphrase_opens_the_key_file_without_a_prompt() {
    let passphrase = random_secret("pass");
    let encrypted = key(Some(&passphrase));
    let server = start_server(vec![key_user("bob", &encrypted.public_openssh)]).await;
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(
            server.addr(),
            Some("bob"),
            file_plan(vec![file_key("id_ed25519", &encrypted, Some(&passphrase))]),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert!(interaction.prompts().is_empty());
    assert!(connection.take_prompted_secrets().file_passphrase.is_none());
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_key_file_prompt_cancels_the_attempt() {
    let encrypted = key(Some(&random_secret("pass")));
    let server = start_server(vec![key_user("bob", &encrypted.public_openssh)]).await;
    let interaction = TestInteraction::answering(vec![Answer::Cancel]);
    let error = connect(
        spec(
            server.addr(),
            Some("bob"),
            file_plan(vec![file_key("id_ed25519", &encrypted, None)]),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect_err("cancelled");
    assert_eq!(error.kind, FailureKind::Cancelled);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_is_offered_once_even_when_it_is_also_a_file() {
    let password = random_secret("pw");
    let unauthorized = key(None);
    let server = start_server(vec![
        UserConfig::new("alice").with_password(secret(&password)),
        key_user("carol", &key(None).public_openssh),
    ])
    .await;
    let interaction = TestInteraction::accepting();
    let plan = AuthPlan {
        key: Some(KeyCredential {
            label: "Work key".to_owned(),
            private_key: unauthorized.private_openssh.clone(),
            passphrase: None,
        }),
        password: Some(secret(&password)),
        file_keys: vec![
            file_key("id_ed25519", &unauthorized, None),
            file_key("id_copy", &unauthorized, None),
        ],
        ..AuthPlan::default()
    };
    let connection = connect(
        spec(server.addr(), Some("alice"), plan),
        interaction.as_dyn(),
    )
    .await
    .expect("connects with the password");
    assert_eq!(connection.auth().method, AuthMethod::Password);
    assert_eq!(publickey_failures(&server.events()), 1);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_public_key_file_stands_in_for_a_key_without_one() {
    // An unreadable private text with a matching `.pub` line: the key is
    // offered, accepted, and then cannot sign, so the attempt stops.
    let plain = key(None);
    let server = start_server(vec![key_user("bob", &plain.public_openssh)]).await;
    let interaction = TestInteraction::accepting();
    let broken = FileKey {
        label: "id_broken".to_owned(),
        private_key: secret("-----BEGIN RSA PRIVATE KEY-----\nnot a key\n"),
        public_key: Some(plain.public_openssh.clone()),
        passphrase: None,
    };
    let error = connect(
        spec(server.addr(), Some("bob"), file_plan(vec![broken])),
        interaction.as_dyn(),
    )
    .await
    .expect_err("the key cannot sign");
    assert_eq!(error.kind, FailureKind::KeyUnusable);
    assert!(interaction.prompts().is_empty());
    server.shutdown().await;
}
