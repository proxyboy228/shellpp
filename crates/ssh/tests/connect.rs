//! Connection tests against the in-process russh test server: steps, host
//! key presentation, accept and reject, a changed key, the identification
//! line, the authentication banner, transport failures, the offered
//! algorithms and the reconnect after the server gave up during the host
//! key decision.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use shellpp_ssh::{AuthMethod, AuthPlan, ConnectStep, FailureKind, HostKeyDecision, connect};
use shellpp_test_sshd::{ServerConfig, ServerEvent};
use support::net::{KexInitCapture, SilentServer, closed_port};
use support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_connection_presents_the_host_key_then_runs_the_shell() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");

    assert_eq!(
        interaction.steps(),
        vec![
            ConnectStep::Resolving,
            ConnectStep::Connecting,
            ConnectStep::HostKey,
            ConnectStep::Authenticating
        ]
    );
    let keys = interaction.host_keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key_type, server.host_key_type());
    assert_eq!(keys[0].public_key_base64, server.host_key_base64());
    assert_eq!(keys[0].fingerprint, server.host_key_fingerprint());
    assert!(keys[0].fingerprint.starts_with("SHA256:"));
    assert_eq!(connection.auth().method, AuthMethod::Password);
    assert_eq!(connection.auth().username, "alice");
    assert!(interaction.prompts().is_empty());

    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, "alice@shellpp-test:~$ ", SHORT).await;
    shell
        .writer()
        .write(b"echo hello-shellpp\r")
        .await
        .expect("write");
    read_until(&mut shell, "\r\nhello-shellpp\r\n", SHORT).await;
    assert!(!connection.is_closed());
    connection.disconnect().await;
    assert!(connection.is_closed());
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_host_key_ends_the_attempt_before_any_authentication() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::rejecting();
    let error = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect_err("rejected");
    assert_eq!(error.kind, FailureKind::HostKeyRejected);
    assert_eq!(interaction.host_keys().len(), 1);
    assert!(interaction.prompts().is_empty());
    assert!(!interaction.steps().contains(&ConnectStep::Authenticating));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let auth_events: Vec<ServerEvent> = server
        .events()
        .into_iter()
        .filter(|event| matches!(event, ServerEvent::Auth { .. }))
        .collect();
    assert!(
        auth_events.is_empty(),
        "no authentication after a reject: {auth_events:?}"
    );
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rotated_host_key_is_presented_as_a_different_key() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let first = TestInteraction::accepting();
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        first.as_dyn(),
    )
    .await
    .expect("first connection");
    let old = first.host_keys()[0].clone();
    connection.disconnect().await;

    server.rotate_host_key().await.expect("rotate");
    let second = TestInteraction::rejecting();
    let error = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        second.as_dyn(),
    )
    .await
    .expect_err("the host decides: blocked");
    assert_eq!(error.kind, FailureKind::HostKeyRejected);
    let new = second.host_keys()[0].clone();
    assert_ne!(new.fingerprint, old.fingerprint);
    assert_ne!(new.public_key_base64, old.public_key_base64);
    assert_eq!(new.key_type, old.key_type);
    assert_eq!(new.fingerprint, server.host_key_fingerprint());
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_identification_line_and_the_banner_reach_the_caller() {
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.banner_comment = Some("Ubuntu-3ubuntu13".to_owned());
    config.auth_banner = Some("Authorized use only\r\n".to_owned());
    let server = start_with(config).await;
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    assert_eq!(
        connection.server_ident(),
        "SSH-2.0-shellpp_test_sshd Ubuntu-3ubuntu13"
    );
    assert_eq!(
        interaction.banners(),
        vec!["Authorized use only\r\n".to_owned()]
    );
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_port_is_refused() {
    let addr = closed_port().await;
    let interaction = TestInteraction::accepting();
    let error = connect(
        spec(addr, Some("alice"), AuthPlan::default()),
        interaction.as_dyn(),
    )
    .await
    .expect_err("nothing listens");
    assert_eq!(error.kind, FailureKind::Refused);
    assert_eq!(
        interaction.steps(),
        vec![ConnectStep::Resolving, ConnectStep::Connecting]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_never_answers_times_out() {
    let silent = SilentServer::start().await;
    let interaction = TestInteraction::accepting();
    let mut spec = spec(silent.addr(), Some("alice"), AuthPlan::default());
    spec.connect_timeout = Duration::from_millis(500);
    let started = Instant::now();
    let error = connect(spec, interaction.as_dyn())
        .await
        .expect_err("no SSH server");
    assert_eq!(error.kind, FailureKind::Timeout);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(interaction.host_keys().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_routable_address_fails_within_the_timeout() {
    // An address whose TCP connect hangs. On Unix a local black hole
    // (`BlackHole`): 192.0.2.0/24 (TEST-NET-1) is never routed, but a
    // network may answer for it anyway (a dev server's did, with an SSH
    // server). On Windows TEST-NET-1: depending on the machine's routes the
    // connect hangs (timeout) or fails at once (network error).
    #[cfg(unix)]
    let hole = support::net::BlackHole::start().await;
    #[cfg(unix)]
    let address = hole.addr;
    #[cfg(not(unix))]
    let address = "192.0.2.1:22".parse().expect("address");
    let interaction = TestInteraction::accepting();
    let mut spec = spec(address, Some("alice"), AuthPlan::default());
    spec.connect_timeout = Duration::from_millis(500);
    let started = Instant::now();
    let error = connect(spec, interaction.as_dyn())
        .await
        .expect_err("not routable");
    assert!(
        matches!(error.kind, FailureKind::Timeout | FailureKind::Network),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_name_is_not_found() {
    let interaction = TestInteraction::accepting();
    let spec = shellpp_ssh::ConnectSpec {
        // An empty label fails in the resolver without a DNS query (a query for
        // a `.invalid` name can take seconds on some networks).
        address: "shellpp..invalid".to_owned(),
        ..spec(
            "127.0.0.1:22".parse().expect("address"),
            Some("alice"),
            AuthPlan::default(),
        )
    };
    let error = connect(spec, interaction.as_dyn())
        .await
        .expect_err("does not resolve");
    assert_eq!(error.kind, FailureKind::NotFound);
    assert_eq!(interaction.steps(), vec![ConnectStep::Resolving]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_sha1_algorithm_is_offered_and_saved_key_types_come_first() {
    let capture = KexInitCapture::start().await;
    let mut spec = spec(capture.addr(), Some("alice"), AuthPlan::default());
    spec.preferred_host_key_types = vec!["ecdsa-sha2-nistp256".to_owned()];
    let result = connect(spec, TestInteraction::accepting().as_dyn()).await;
    assert!(result.is_err(), "the fake server closes after KEXINIT");
    let kexinit = capture.kexinit().await;

    for name in kexinit.all_names() {
        assert!(!name.contains("sha1"), "{name} must not be offered");
        assert!(
            !name.contains("cert-v01"),
            "{name}: host certificates are not trusted"
        );
    }
    assert!(
        !kexinit.host_key.iter().any(|name| name == "ssh-rsa"),
        "ssh-rsa (SHA-1) signatures must not be offered: {:?}",
        kexinit.host_key
    );
    assert_eq!(kexinit.host_key[0], "ecdsa-sha2-nistp256");
    assert!(kexinit.host_key.contains(&"rsa-sha2-512".to_owned()));
    assert!(kexinit.kex.contains(&"curve25519-sha256".to_owned()));
    assert_eq!(kexinit.compression_c2s[0], "none");
    assert!(kexinit.mac_c2s.iter().all(|mac| mac.contains("sha2")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_closing_during_the_decision_reconnects_once_without_asking_again() {
    let password = random_secret("pw");
    let server = Arc::new(start_server(vec![password_user("alice", &password)]).await);
    let interaction = TestInteraction::accepting();
    {
        let server = Arc::clone(&server);
        interaction.on_first_verify(move || {
            server.drop_connections();
        });
    }
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect("reconnected after the accept");
    assert_eq!(interaction.host_keys().len(), 1, "the user is asked once");
    let host_key_steps = interaction
        .steps()
        .iter()
        .filter(|step| **step == ConnectStep::HostKey)
        .count();
    assert_eq!(host_key_steps, 2, "two handshakes");
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, "alice@shellpp-test:~$ ", SHORT).await;
    connection.disconnect().await;
    drop(shell);
    drop(connection);
    if let Ok(server) = Arc::try_unwrap(server) {
        server.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reject_after_the_server_closed_is_not_retried() {
    let password = random_secret("pw");
    let server = Arc::new(start_server(vec![password_user("alice", &password)]).await);
    let interaction = TestInteraction::rejecting();
    {
        let server = Arc::clone(&server);
        interaction.on_first_verify(move || {
            server.drop_connections();
        });
    }
    let error = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect_err("rejected");
    assert_eq!(error.kind, FailureKind::HostKeyRejected);
    assert_eq!(interaction.host_keys().len(), 1);
    if let Ok(server) = Arc::try_unwrap(server) {
        server.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decisions_are_per_handshake() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::accepting();
    interaction.queue_decisions(&[HostKeyDecision::Reject]);
    let error = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect_err("first decision rejects");
    assert_eq!(error.kind, FailureKind::HostKeyRejected);
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction.as_dyn(),
    )
    .await
    .expect("second decision accepts");
    assert_eq!(interaction.host_keys().len(), 2);
    connection.disconnect().await;
    server.shutdown().await;
}

/// An interaction whose host key decision never comes; records when the
/// pending decision future is dropped.
struct UndecidedInteraction {
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

struct SetOnDrop(Arc<std::sync::atomic::AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl shellpp_ssh::Interaction for UndecidedInteraction {
    fn step(&self, _step: ConnectStep) {}

    fn verify_host_key<'a>(
        &'a self,
        _key: &'a shellpp_ssh::HostKeyInfo,
    ) -> shellpp_ssh::BoxFuture<'a, HostKeyDecision> {
        let guard = SetOnDrop(Arc::clone(&self.dropped));
        Box::pin(async move {
            let _guard = guard;
            std::future::pending::<HostKeyDecision>().await
        })
    }

    fn prompt<'a>(
        &'a self,
        _prompt: shellpp_ssh::Prompt,
    ) -> shellpp_ssh::BoxFuture<'a, Option<shellpp_ssh::PromptAnswer>> {
        Box::pin(async { None })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoning_the_attempt_ends_a_pending_host_key_decision() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let interaction = Arc::new(UndecidedInteraction {
        dropped: Arc::clone(&dropped),
    });
    let attempt = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        interaction,
    );
    let still_waiting = tokio::time::timeout(Duration::from_millis(800), attempt).await;
    assert!(still_waiting.is_err(), "waiting for the user");
    assert!(
        eventually(SHORT, || dropped.load(std::sync::atomic::Ordering::SeqCst)).await,
        "the pending decision is dropped with the attempt"
    );
    let auth_events = server
        .events()
        .into_iter()
        .filter(|event| matches!(event, ServerEvent::Auth { .. }))
        .count();
    assert_eq!(auth_events, 0);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_decision_time_does_not_count_against_the_connect_timeout() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let interaction = TestInteraction::accepting();
    // The hook delays the decision past the connect timeout.
    interaction.on_first_verify(|| std::thread::sleep(Duration::from_millis(700)));
    let mut spec = spec(server.addr(), Some("alice"), password_plan(&password));
    spec.connect_timeout = Duration::from_millis(400);
    let connection = connect(spec, interaction.as_dyn())
        .await
        .expect("a slow decision is not a timeout");
    connection.disconnect().await;
    server.shutdown().await;
}
