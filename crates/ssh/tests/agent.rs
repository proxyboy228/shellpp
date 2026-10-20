//! System agent tests with a test agent on a temporary socket (Unix) or a
//! uniquely named pipe (Windows): identities, unreachable agents, discovery
//! that never hangs, and agent forwarding through the russh test server's
//! `agent-keys` command. No test probes the machine's real agent
//! (`SystemAgent::discover` itself is never called here).

mod support;

use std::path::PathBuf;
use std::time::Duration;

use secrecy::ExposeSecret;
use shellpp_ssh::{AgentLocation, ShellRequest, SystemAgent, connect};
use shellpp_test_sshd::{KeyType, ServerEvent, generate_key};
use support::agent::TestAgent;
use support::*;

const PROMPT: &str = "alice@shellpp-test:~$ ";

fn agent_keys(count: usize) -> (Vec<russh::keys::PrivateKey>, Vec<String>) {
    let mut keys = Vec::new();
    let mut publics = Vec::new();
    for index in 0..count {
        let generated =
            generate_key(KeyType::Ed25519, &format!("agent-{index}"), None).expect("key");
        keys.push(
            russh::keys::decode_secret_key(generated.private_openssh.expose_secret(), None)
                .expect("decodes"),
        );
        publics.push(generated.public_openssh);
    }
    (keys, publics)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identities_lists_the_agent_keys_as_openssh_lines() {
    let (keys, publics) = agent_keys(2);
    let agent = TestAgent::start(keys).await;
    let lines = SystemAgent::at(agent.location())
        .identities()
        .await
        .expect("identities");
    assert_eq!(lines.len(), 2);
    for (line, public) in lines.iter().zip(&publics) {
        let blob = |text: &str| text.split_whitespace().nth(1).map(str::to_owned);
        assert!(line.starts_with("ssh-ed25519 "));
        assert_eq!(blob(line), blob(public));
    }
}

/// A socket or pipe path where no agent listens.
fn missing_agent_path(dir: &tempfile::TempDir) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(
            r"\\.\pipe\shellpp-no-agent-{:016x}",
            rand::random::<u64>()
        ))
    } else {
        dir.path()
            .join(format!("missing-{:016x}.sock", rand::random::<u64>()))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_agent_is_an_error() {
    let dir = tempfile::tempdir().expect("dir");
    let result = SystemAgent::at(AgentLocation::Path(missing_agent_path(&dir)))
        .identities()
        .await;
    assert!(result.is_err());
}

/// Discovery skips locations where no agent answers, in bounded time, and
/// returns the first that does. Only the test's own agent and missing paths
/// are probed, never the machine's real agent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovery_skips_unreachable_candidates_in_bounded_time() {
    let (keys, _) = agent_keys(1);
    let agent = TestAgent::start(keys).await;
    let dir = tempfile::tempdir().expect("dir");
    let candidates = vec![
        AgentLocation::Path(missing_agent_path(&dir)),
        AgentLocation::Path(missing_agent_path(&dir)),
        agent.location(),
    ];
    let found = tokio::time::timeout(
        Duration::from_secs(20),
        SystemAgent::discover_from(candidates),
    )
    .await
    .expect("discovery finishes")
    .expect("the test agent is found");
    assert_eq!(found.location(), &agent.location());
    assert_eq!(found.identities().await.expect("identities").len(), 1);

    let none = tokio::time::timeout(
        Duration::from_secs(20),
        SystemAgent::discover_from(vec![AgentLocation::Path(missing_agent_path(&dir))]),
    )
    .await
    .expect("discovery finishes");
    assert!(none.is_none());
    assert!(
        SystemAgent::discover_from(Vec::new()).await.is_none(),
        "no candidates, no agent"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarding_lets_the_server_list_the_agent_keys() {
    let (keys, _) = agent_keys(2);
    let agent = TestAgent::start(keys).await;
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let mut spec = spec(server.addr(), Some("alice"), password_plan(&password));
    spec.agent = Some(SystemAgent::at(agent.location()));
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let mut shell = connection
        .open_shell(ShellRequest {
            agent_forwarding: true,
            ..shell_request()
        })
        .await
        .expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(b"agent-keys\r").await.expect("write");
    read_until(&mut shell, "agent keys: 2", SHORT).await;
    assert!(server.events().contains(&ServerEvent::AgentForward));
    assert!(agent.requests() >= 1);
    assert_eq!(agent.signatures(), 0);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarding_is_off_unless_requested() {
    let (keys, _) = agent_keys(1);
    let agent = TestAgent::start(keys).await;
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let mut spec = spec(server.addr(), Some("alice"), password_plan(&password));
    spec.agent = Some(SystemAgent::at(agent.location()));
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(b"agent-keys\r").await.expect("write");
    read_until(&mut shell, "agent: not forwarded", SHORT).await;
    assert!(!server.events().contains(&ServerEvent::AgentForward));
    assert_eq!(agent.requests(), 0);
    connection.disconnect().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarding_needs_an_agent() {
    let password = random_secret("pw");
    let server = start_server(vec![password_user("alice", &password)]).await;
    let connection = connect(
        spec(server.addr(), Some("alice"), password_plan(&password)),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("connects");
    let mut shell = connection
        .open_shell(ShellRequest {
            agent_forwarding: true,
            ..shell_request()
        })
        .await
        .expect("shell");
    read_until(&mut shell, PROMPT, SHORT).await;
    shell.writer().write(b"agent-keys\r").await.expect("write");
    read_until(&mut shell, "agent: not forwarded", SHORT).await;
    assert!(!server.events().contains(&ServerEvent::AgentForward));
    connection.disconnect().await;
    server.shutdown().await;
}
