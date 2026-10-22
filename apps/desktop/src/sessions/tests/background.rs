//! Connections without a tab (stage 5): forwarding rules on a host's shared
//! connection, autostart when a session to the host connects, and a script
//! on several hosts. Only known host keys and saved credentials are used.

use std::time::Duration;

use shellpp_core::service::dto::{KnownHostAddInput, RecordRef};
use shellpp_core::service::snippets::RecordInput;
use shellpp_ssh::batch::TargetOutcome;
use shellpp_ssh::{Bound, FailureKind, RuleStatus};
use shellpp_test_sshd::ExecAnswer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

fn password_user(name: &str, password: &str) -> UserConfig {
    UserConfig::new(name).with_password(secret(password))
}

fn trust(harness: &Harness, server: &TestServer) {
    harness
        .core
        .known_host_add(KnownHostAddInput {
            host: "127.0.0.1".to_owned(),
            port: Some(server.port()),
            key_type: server.host_key_type(),
            public_key: server.host_key_base64(),
            revoked: false,
            comment: None,
        })
        .expect("known host");
}

fn record(target: &SessionTarget) -> RecordRef {
    match target {
        SessionTarget::Host {
            vault_id,
            record_id,
        } => RecordRef {
            vault_id: *vault_id,
            record_id: *record_id,
        },
        SessionTarget::Quick { .. } => panic!("a saved host"),
    }
}

/// A TCP service that answers with `echo:` and what it got.
async fn echo() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                if let Ok(n) = socket.read(&mut buf).await {
                    let mut reply = b"echo:".to_vec();
                    reply.extend_from_slice(&buf[..n]);
                    let _ = socket.write_all(&reply).await;
                }
            });
        }
    });
    (port, task)
}

fn rule(
    harness: &Harness,
    host: RecordRef,
    dest_port: u16,
    autostart: bool,
) -> shellpp_core::service::snippets::ForwardingRuleView {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    harness
        .core
        .forwarding_rule_save(RecordInput {
            vault_id: harness.vault_id,
            record_id: None,
            data: json!({"type": "local", "host_id": host.record_id.to_string(),
                "bind_port": port, "dest_address": "127.0.0.1", "dest_port": dest_port,
                "autostart_on_connect": autostart})
            .as_object()
            .cloned()
            .unwrap(),
        })
        .expect("rule")
}

async fn ask(port: u16) -> String {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut answer = vec![0u8; 9];
    tokio::time::timeout(WAIT, stream.read_exact(&mut answer))
        .await
        .expect("an answer")
        .unwrap();
    String::from_utf8(answer).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rule_runs_on_a_known_host_and_not_on_an_unknown_one() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.forwarding = true;
    let server = start_with(config).await;
    let (dest, task) = echo().await;
    let target = harness.password_host(&server, "alice", &password);
    let host = record(&target);
    let forward = rule(&harness, host, dest, false);

    // Unknown host key: no dialog to ask, so no connection.
    let status = harness.manager.forward_start(&forward).await;
    assert_eq!(
        status,
        RuleStatus::ConnectFailed(FailureKind::HostKeyRejected)
    );

    trust(&harness, &server);
    let status = harness.manager.forward_start(&forward).await;
    let RuleStatus::Running(Bound::Local(addr)) = status else {
        panic!("{status:?}");
    };
    assert_eq!(addr.port(), forward.bind_port);
    assert_eq!(ask(forward.bind_port).await, "echo:ping");
    harness.manager.forward_stop(forward.record.record_id).await;
    assert!(
        super::eventually_async(|| async {
            tokio::net::TcpStream::connect(("127.0.0.1", forward.bind_port))
                .await
                .is_err()
        })
        .await
    );
    task.abort();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rules_marked_for_it_start_when_a_session_to_their_host_connects() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.forwarding = true;
    let server = start_with(config).await;
    let (dest, task) = echo().await;
    let target = harness.password_host(&server, "alice", &password);
    let host = record(&target);
    let auto = rule(&harness, host, dest, true);
    let manual = rule(&harness, host, dest, false);
    // The session's TOFU saves the key the rule's connection then trusts.
    let session = harness.connect(target, true).await;
    let started = super::eventually_async(|| async {
        matches!(
            harness.manager.forward_status(auto.record.record_id),
            Some(RuleStatus::Running(_))
        )
    })
    .await;
    assert!(started, "the autostart rule runs");
    assert_eq!(ask(auto.bind_port).await, "echo:ping");
    assert_eq!(
        harness.manager.forward_status(manual.record.record_id),
        None
    );
    harness.manager.forward_stop(auto.record.record_id).await;
    session.handle.close();
    task.abort();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_script_runs_on_several_hosts() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let mut hosts = Vec::new();
    let mut servers = Vec::new();
    for index in 0..3 {
        let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
        config.exec = Some(ExecAnswer {
            stdout: format!("host {index}\n"),
            exit_status: index,
        });
        let server = start_with(config).await;
        let target = harness.host(
            server.port(),
            json!({"label": format!("h{index}"), "ssh": {"username": "alice"}}),
            Some(&password),
        );
        // The last one is not trusted yet.
        if index < 2 {
            trust(&harness, &server);
        }
        hosts.push(record(&target));
        servers.push(server);
    }
    let results = harness
        .manager
        .run_on_hosts("uptime".to_owned(), hosts, Duration::from_secs(10), None)
        .await;
    assert!(matches!(&results[0], TargetOutcome::Finished(output)
        if output.stdout == b"host 0\n" && output.exit_status == Some(0)));
    assert!(matches!(&results[1], TargetOutcome::Finished(output)
        if output.exit_status == Some(1)));
    assert!(matches!(&results[2], TargetOutcome::ConnectFailed(error)
        if error.kind == FailureKind::HostKeyRejected));
    for server in servers {
        server.shutdown().await;
    }
}

/// Rules marked "on launch" start once per app run: at start when the app
/// is already unlocked ("remember on this device"), and a later unlock does
/// not start them again (the review of stage 5).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_rules_start_once_per_run() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.forwarding = true;
    let server = start_with(config).await;
    trust(&harness, &server);
    let (dest, task) = echo().await;
    let target = harness.password_host(&server, "alice", &password);
    let host = record(&target);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let launch = harness
        .core
        .forwarding_rule_save(RecordInput {
            vault_id: harness.vault_id,
            record_id: None,
            data: json!({"type": "local", "host_id": host.record_id.to_string(),
                "bind_port": port, "dest_address": "127.0.0.1", "dest_port": dest,
                "autostart_on_launch": true})
            .as_object()
            .cloned()
            .unwrap(),
        })
        .expect("rule");
    // This manager started before the rule existed: a later unlock does
    // not run the launch autostart again.
    harness.manager.set_unlocked(false);
    harness.manager.set_unlocked(true);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        harness.manager.forward_status(launch.record.record_id),
        None
    );
    // A new app run, already unlocked: the rule starts.
    let mut options = ManagerOptions::with_keys(KeySources {
        key_dir: Some(harness.key_dir.path().to_path_buf()),
        agent: AgentSource::None,
    });
    options.runtime = Some(tokio::runtime::Handle::current());
    let run = SessionManager::start(harness.calls.clone() as Arc<dyn SessionCore>, options)
        .expect("start");
    let started = super::eventually_async(|| async {
        matches!(
            run.forward_status(launch.record.record_id),
            Some(RuleStatus::Running(_))
        )
    })
    .await;
    assert!(started, "{:?}", run.forward_status(launch.record.record_id));
    assert_eq!(ask(port).await, "echo:ping");
    run.forward_stop(launch.record.record_id).await;
    task.abort();
    server.shutdown().await;
}

/// An "on launch" rule that cannot start (its port is taken) shows its
/// failure and leaves no connection behind: no rule runs on it, and it
/// would reconnect after every drop (section 12 review).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_launch_rule_leaves_no_connection() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.forwarding = true;
    let server = start_with(config).await;
    trust(&harness, &server);
    let target = harness.password_host(&server, "alice", &password);
    let host = record(&target);
    // The port is taken for the whole test.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let launch = harness
        .core
        .forwarding_rule_save(RecordInput {
            vault_id: harness.vault_id,
            record_id: None,
            data: json!({"type": "local", "host_id": host.record_id.to_string(),
                "bind_port": port, "dest_address": "127.0.0.1", "dest_port": 9,
                "autostart_on_launch": true})
            .as_object()
            .cloned()
            .unwrap(),
        })
        .expect("rule");
    let mut options = ManagerOptions::with_keys(KeySources {
        key_dir: Some(harness.key_dir.path().to_path_buf()),
        agent: AgentSource::None,
    });
    options.runtime = Some(tokio::runtime::Handle::current());
    let run = SessionManager::start(harness.calls.clone() as Arc<dyn SessionCore>, options)
        .expect("start");
    let failed = super::eventually_async(|| async {
        matches!(
            run.forward_status(launch.record.record_id),
            Some(RuleStatus::Failed(_))
        )
    })
    .await;
    assert!(failed, "{:?}", run.forward_status(launch.record.record_id));
    let closed = super::eventually_async(|| async { server.connection_count() == 0 }).await;
    assert!(closed, "{} connections left", server.connection_count());
    // Stopped by the user, the failure is gone too.
    run.forward_stop(launch.record.record_id).await;
    assert_eq!(run.forward_status(launch.record.record_id), None);
    drop(taken);
    server.shutdown().await;
}

/// A rule whose connection drops while the app is locked comes back after
/// the unlock (security.md 8: reconnects wait for unlock; the review of
/// stage 5: it ended with ConnectFailed).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rule_dropped_while_locked_comes_back_after_unlock() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let mut config = ServerConfig::new(vec![password_user("alice", &password)]);
    config.forwarding = true;
    let server = start_with(config).await;
    trust(&harness, &server);
    let (dest, task) = echo().await;
    let target = harness.password_host(&server, "alice", &password);
    let forward = rule(&harness, record(&target), dest, false);
    let status = harness.manager.forward_start(&forward).await;
    assert!(matches!(status, RuleStatus::Running(_)), "{status:?}");
    harness.manager.set_unlocked(false);
    assert_eq!(server.drop_connections(), 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        matches!(
            harness.manager.forward_status(forward.record.record_id),
            Some(RuleStatus::Reconnecting { .. })
        ),
        "{:?}",
        harness.manager.forward_status(forward.record.record_id)
    );
    harness.manager.set_unlocked(true);
    let back = super::eventually_async(|| async {
        matches!(
            harness.manager.forward_status(forward.record.record_id),
            Some(RuleStatus::Running(_))
        )
    })
    .await;
    assert!(
        back,
        "{:?}",
        harness.manager.forward_status(forward.record.record_id)
    );
    assert_eq!(ask(forward.bind_port).await, "echo:ping");
    harness.manager.forward_stop(forward.record.record_id).await;
    task.abort();
    server.shutdown().await;
}
