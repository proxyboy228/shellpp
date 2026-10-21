//! Snippets on many hosts (UI 9.4) against the russh test server: six
//! targets, one of them unreachable; at most eight at a time; the timeout.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use shellpp_ssh::batch::{self, BatchEvent, MAX_PARALLEL, TargetOutcome};
use shellpp_ssh::{FailureKind, SshError, connect};
use shellpp_test_sshd::{ExecAnswer, ServerConfig, ServerEvent, TestServer};
use support::*;

async fn exec_server(password: &str, stdout: &str, exit_status: u32) -> TestServer {
    let mut config = ServerConfig::new(vec![password_user("alice", password)]);
    config.exec = Some(ExecAnswer {
        stdout: stdout.to_owned(),
        exit_status,
    });
    start_with(config).await
}

fn target_of(addr: std::net::SocketAddr, password: &str) -> batch::ConnectTarget {
    let password = password.to_owned();
    batch::target(move || async move {
        connect(
            spec(addr, Some("alice"), password_plan(&password)),
            TestInteraction::accepting().as_dyn(),
        )
        .await
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn six_hosts_one_of_them_unreachable() {
    let password = random_secret("pw");
    let mut servers = Vec::new();
    for index in 0..5u32 {
        // The last one fails with exit status 3.
        let status = if index == 4 { 3 } else { 0 };
        servers.push(exec_server(&password, &format!("host {index}\n"), status).await);
    }
    let closed = support::net::closed_port().await;
    let mut targets: Vec<batch::ConnectTarget> = servers
        .iter()
        .map(|server| target_of(server.addr(), &password))
        .collect();
    targets.insert(2, target_of(closed, &password));
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let results = batch::run(
        targets,
        "uname -a".to_owned(),
        SHORT,
        64 * 1024,
        Some(events),
    )
    .await;
    assert_eq!(results.len(), 6);
    let ran = [0usize, 1, 3, 4, 5];
    for (position, index) in ran.iter().enumerate() {
        let TargetOutcome::Finished(output) = &results[*index] else {
            panic!("target {index}: {:?}", results[*index]);
        };
        assert_eq!(output.stdout, format!("host {position}\n").into_bytes());
        assert!(!output.truncated && !output.timed_out);
    }
    assert!(results[0].succeeded());
    assert!(!results[5].succeeded(), "exit status 3");
    assert!(
        matches!(&results[5], TargetOutcome::Finished(output) if output.exit_status == Some(3))
    );
    assert_eq!(
        results[2],
        TargetOutcome::ConnectFailed(SshError::new(FailureKind::Refused))
    );
    // Every host ran the command in an exec channel, never a shell.
    for server in &servers {
        let events = server.events();
        assert!(events.contains(&ServerEvent::Exec {
            command: "uname -a".to_owned()
        }));
        assert!(!events.contains(&ServerEvent::Shell));
    }
    // Progress: a start and an end for each target.
    let mut started = 0;
    let mut finished = 0;
    while let Ok(event) = received.try_recv() {
        match event {
            BatchEvent::Started(_) => started += 1,
            BatchEvent::Finished(..) => finished += 1,
        }
    }
    assert_eq!((started, finished), (6, 6));
    // The connections are closed afterwards.
    for server in &servers {
        assert!(eventually(SHORT, || server.connection_count() == 0).await);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_most_eight_targets_run_at_once() {
    let running = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let targets: Vec<batch::ConnectTarget> = (0..20)
        .map(|_| {
            let running = Arc::clone(&running);
            let most = Arc::clone(&most);
            batch::target(move || async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                running.fetch_sub(1, Ordering::SeqCst);
                Err(SshError::new(FailureKind::Refused))
            })
        })
        .collect();
    let results = batch::run(targets, "true".to_owned(), SHORT, 1024, None).await;
    assert_eq!(results.len(), 20);
    assert_eq!(most.load(Ordering::SeqCst), MAX_PARALLEL);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_target_that_takes_too_long_times_out() {
    let silent = support::net::SilentServer::start().await;
    let password = random_secret("pw");
    let targets = vec![target_of(silent.addr(), &password)];
    let results = batch::run(
        targets,
        "true".to_owned(),
        Duration::from_millis(300),
        1024,
        None,
    )
    .await;
    assert!(
        matches!(
            &results[0],
            TargetOutcome::ConnectFailed(error) if error.kind == FailureKind::Timeout
        ),
        "{:?}",
        results[0]
    );
}
