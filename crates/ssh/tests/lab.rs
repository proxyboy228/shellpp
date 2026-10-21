//! Stage 5 against real OpenSSH servers (`tests/openssh/lab.sh`, run with
//! `SHELLPP_TEST_SSHD=openssh`): a chain of two jump hosts in a network of
//! its own with a host key decision on every hop and failures on the second
//! one, port forwarding (Local to an HTTP server in the lab, Dynamic through
//! `curl --socks5-hostname`, Remote, rules coming back after their
//! connection dropped), refused environment variables and a command on six
//! hosts, one of them unreachable.
//!
//! One lab serves every scenario (they run in order in one test): starting
//! six containers per scenario would cost more than the scenarios.

mod support;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;
use shellpp_ssh::batch::{self, TargetOutcome};
use shellpp_ssh::{
    AuthPlan, Bound, ConnectSpec, Connection, FailureKind, FailurePlace, ForwardKind, ForwardSpec,
    HostForwarder, HostKeyDecision, JumpHost, RuleStatus, ShellRequest, connect,
};
use support::lab::Lab;
use support::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A spec for a server of the lab as `name` (a name inside the lab network,
/// resolved by the jump host before it), port 22.
fn lab_spec(name: &str, auth: AuthPlan, user: &str) -> ConnectSpec {
    ConnectSpec {
        address: name.to_owned(),
        port: 22,
        ..spec("127.0.0.1:22".parse().expect("address"), Some(user), auth)
    }
}

fn password_route(
    lab: &Lab,
    hops: &[&str],
    target: &str,
) -> (ConnectSpec, Vec<Arc<TestInteraction>>) {
    let (user, password) = &lab.password_user;
    let mut interactions = Vec::new();
    let mut jump_hosts = Vec::new();
    for (index, hop) in hops.iter().enumerate() {
        let interaction = TestInteraction::accepting();
        let spec = if index == 0 {
            spec(lab.jump1, Some(user), password_plan(password))
        } else {
            lab_spec(hop, password_plan(password), user)
        };
        jump_hosts.push(JumpHost {
            spec,
            interaction: interaction.as_dyn(),
        });
        interactions.push(interaction);
    }
    let spec = ConnectSpec {
        jump_hosts,
        ..lab_spec(target, password_plan(password), user)
    };
    (spec, interactions)
}

async fn run(connection: &Connection, command: &str) -> String {
    let output = connection
        .exec(command, SHORT, 64 * 1024)
        .await
        .expect("exec");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_stage_5_lab() {
    let Some(lab) = Lab::start() else {
        return;
    };
    two_jump_hosts_with_a_decision_on_each(&lab).await;
    failures_on_the_second_jump_host(&lab).await;
    local_and_dynamic_forwarding_reach_the_lab(&lab).await;
    remote_forwarding_reaches_back_here(&lab).await;
    rules_come_back_after_the_connection_drops(&lab).await;
    refused_environment_variables(&lab).await;
    a_command_on_six_hosts(&lab).await;
    the_built_in_agent_signs_for_the_next_hop(&lab).await;
    real_proxies_carry_the_first_connection(&lab).await;
}

async fn two_jump_hosts_with_a_decision_on_each(lab: &Lab) {
    let (spec, hops) = password_route(lab, &["jump1", "jump2"], "target");
    let target = TestInteraction::accepting();
    let connection = connect(spec, target.as_dyn()).await.expect("the route");
    for (interaction, server) in hops
        .iter()
        .chain([&target])
        .zip(["jump1", "jump2", "target"])
    {
        let keys = interaction.host_keys();
        assert_eq!(keys.len(), 1, "{server}: one decision");
        assert!(
            lab.fingerprints(server).contains(&keys[0].fingerprint),
            "{server} presented its own key"
        );
    }
    assert_eq!(run(&connection, "hostname").await.trim(), "target");
    // A shell on the target.
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    shell
        .writer()
        .write(b"echo shellpp-$((6*7))\r")
        .await
        .unwrap();
    read_until(&mut shell, "shellpp-42", SHORT).await;
    connection.disconnect().await;
}

async fn failures_on_the_second_jump_host(lab: &Lab) {
    // Wrong password on jump2 (and no prompt answer): its own failure.
    let (mut spec, hops) = password_route(lab, &["jump1", "jump2"], "target");
    spec.jump_hosts[1].spec.auth = password_plan("wrong-password");
    let target = TestInteraction::accepting();
    let error = connect(spec, target.as_dyn())
        .await
        .expect_err("jump2 refuses");
    assert_eq!(error.place, FailurePlace::JumpHost(1), "{error}");
    assert_eq!(
        error.kind,
        FailureKind::Cancelled,
        "the repeated prompt is cancelled"
    );
    assert!(
        !hops[1].prompts().is_empty(),
        "jump2 asked for its password"
    );
    assert!(target.host_keys().is_empty(), "the target was not reached");

    // jump2's key rejected.
    let (spec, hops) = password_route(lab, &["jump1", "jump2"], "target");
    hops[1].queue_decisions(&[HostKeyDecision::Reject]);
    let error = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect_err("rejected");
    assert_eq!(
        (error.kind, error.place),
        (FailureKind::HostKeyRejected, FailurePlace::JumpHost(1))
    );

    // A second hop jump1 cannot reach.
    let (spec, _) = password_route(lab, &["jump1", "no-such-host"], "target");
    let error = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect_err("unreachable");
    assert_eq!(
        (error.kind, error.place),
        (FailureKind::Refused, FailurePlace::JumpHost(1))
    );
}

async fn http_get(stream: &mut TcpStream, host: &str) -> String {
    stream
        .write_all(format!("GET / HTTP/1.0\r\nHost: {host}\r\n\r\n").as_bytes())
        .await
        .expect("request");
    let mut body = Vec::new();
    tokio::time::timeout(SHORT, stream.read_to_end(&mut body))
        .await
        .expect("an answer in time")
        .expect("read");
    String::from_utf8_lossy(&body).into_owned()
}

async fn jump1(lab: &Lab) -> Connection {
    let (user, password) = &lab.password_user;
    connect(
        spec(lab.jump1, Some(user), password_plan(password)),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("jump1")
}

fn local_addr(bound: Bound) -> SocketAddr {
    match bound {
        Bound::Local(addr) => addr,
        Bound::Remote(_) => panic!("a local listener"),
    }
}

async fn local_and_dynamic_forwarding_reach_the_lab(lab: &Lab) {
    let connection = jump1(lab).await;
    // Local: 127.0.0.1:<any> -> target:8080 as jump1 sees it.
    let forward = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Local {
                host: "target".to_owned(),
                port: 8080,
            },
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect("local rule");
    let addr = local_addr(forward.bound());
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let answer = http_get(&mut stream, "target").await;
    assert!(answer.contains(&lab.http_body), "{answer:?}");
    forward.stop().await;

    // Dynamic: curl resolves nothing itself (--socks5-hostname); jump1 does.
    let forward = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Dynamic,
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect("dynamic rule");
    let addr = local_addr(forward.bound());
    let proxy = addr.to_string();
    let curl = tokio::task::spawn_blocking(move || {
        std::process::Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--max-time",
                "10",
                "--socks5-hostname",
                &proxy,
                "http://target:8080/",
            ])
            .output()
    })
    .await
    .expect("curl task");
    match curl {
        Ok(output) => {
            assert!(output.status.success(), "curl: {output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                lab.http_body
            );
        }
        // No curl here: the same request through a SOCKS5 handshake of our own.
        Err(_) => {
            let mut socks = TcpStream::connect(addr).await.expect("connect");
            socks.write_all(&[5, 1, 0]).await.unwrap();
            let mut choice = [0u8; 2];
            socks.read_exact(&mut choice).await.unwrap();
            let mut request = vec![5, 1, 0, 3, 6];
            request.extend_from_slice(b"target");
            request.extend_from_slice(&8080u16.to_be_bytes());
            socks.write_all(&request).await.unwrap();
            let mut reply = [0u8; 10];
            socks.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply[1], 0, "the SOCKS request succeeded");
            assert!(
                http_get(&mut socks, "target")
                    .await
                    .contains(&lab.http_body)
            );
        }
    }
    forward.stop().await;

    // Through the chain: a Local rule on the target's connection (via jump1).
    let (spec, _) = password_route(lab, &["jump1"], "target");
    let target = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("target via jump1");
    let forward = target
        .forward(ForwardSpec {
            kind: ForwardKind::Local {
                host: "127.0.0.1".to_owned(),
                port: 8080,
            },
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect("local rule on the target");
    let mut stream = TcpStream::connect(local_addr(forward.bound()))
        .await
        .expect("connect");
    assert!(
        http_get(&mut stream, "localhost")
            .await
            .contains(&lab.http_body)
    );
    forward.stop().await;
    target.disconnect().await;
    connection.disconnect().await;
}

/// A TCP service here that answers with `echo:` and what it got.
async fn echo_here() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 256];
                if let Ok(n) = socket.read(&mut buf).await {
                    let mut reply = b"echo:".to_vec();
                    reply.extend_from_slice(&buf[..n]);
                    let _ = socket.write_all(&reply).await;
                }
            });
        }
    });
    (addr, task)
}

async fn remote_forwarding_reaches_back_here(lab: &Lab) {
    let (echo, task) = echo_here().await;
    let connection = jump1(lab).await;
    let forward = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Remote {
                host: "127.0.0.1".to_owned(),
                port: echo.port(),
            },
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect("remote rule");
    let Bound::Remote(port) = forward.bound() else {
        panic!("a remote listener");
    };
    assert_ne!(port, 0);
    // On jump1, connect to its forwarded port: the data comes back here.
    let answer = run(
        &connection,
        &format!("printf hello | busybox nc -w 3 127.0.0.1 {port}"),
    )
    .await;
    assert_eq!(answer, "echo:hello");
    forward.stop().await;
    connection.disconnect().await;
    task.abort();
}

async fn rules_come_back_after_the_connection_drops(lab: &Lab) {
    let (user, password) = lab.password_user.clone();
    let addr = lab.jump1;
    let connect_fn: shellpp_ssh::forward::ConnectFn = Arc::new(move || {
        let (user, password) = (user.clone(), password.clone());
        Box::pin(async move {
            connect(
                spec(addr, Some(&user), password_plan(&password)),
                TestInteraction::accepting().as_dyn(),
            )
            .await
            .map(Arc::new)
        })
    });
    let forwarder = HostForwarder::new(connect_fn);
    let port = support::net::closed_port().await.port();
    let status = forwarder
        .start(
            1,
            ForwardSpec {
                kind: ForwardKind::Local {
                    host: "target".to_owned(),
                    port: 8080,
                },
                bind_address: String::new(),
                bind_port: port,
            },
        )
        .await;
    assert!(matches!(status, RuleStatus::Running(_)), "{status:?}");
    let mut statuses = forwarder.subscribe();
    lab.drop_connections("jump1");
    let back = tokio::time::timeout(Duration::from_secs(20), async {
        // Down first, then running again.
        let _ = statuses
            .wait_for(|statuses| matches!(statuses.get(&1), Some(RuleStatus::Reconnecting { .. })))
            .await
            .map(|_| ());
        statuses
            .wait_for(|statuses| matches!(statuses.get(&1), Some(RuleStatus::Running(_))))
            .await
            .map(|_| ())
    })
    .await;
    assert!(matches!(back, Ok(Ok(()))), "the rule runs again");
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    assert!(
        http_get(&mut stream, "target")
            .await
            .contains(&lab.http_body)
    );
    forwarder.shutdown().await;
}

async fn refused_environment_variables(lab: &Lab) {
    let connection = jump1(lab).await;
    // The lab's sshd_config: AcceptEnv LANG LC_* SHELLPP_*.
    let value = |text: &str| SecretString::from(text.to_owned());
    let mut shell = connection
        .open_shell(ShellRequest {
            env: vec![
                ("LANG".to_owned(), value("C.UTF-8")),
                ("APP_MODE".to_owned(), value("dev")),
                ("SHELLPP_TEST".to_owned(), value("lab")),
            ],
            ..shell_request()
        })
        .await
        .expect("shell");
    assert_eq!(shell.refused_env(), ["APP_MODE"]);
    shell
        .writer()
        .write(b"echo \"[$SHELLPP_TEST|$APP_MODE]\"\r")
        .await
        .unwrap();
    read_until(&mut shell, "[lab|]", SHORT).await;
    connection.disconnect().await;
}

async fn a_command_on_six_hosts(lab: &Lab) {
    let names = ["jump2", "target", "app1", "no-such-host", "app2", "app3"];
    let targets: Vec<batch::ConnectTarget> = names
        .iter()
        .map(|name| {
            let (spec, _) = password_route(lab, &["jump1"], name);
            batch::target(move || async move {
                connect(spec, TestInteraction::accepting().as_dyn()).await
            })
        })
        .collect();
    let results = batch::run(
        targets,
        "hostname; echo err >&2; exit 7".to_owned(),
        Duration::from_secs(30),
        4096,
        None,
    )
    .await;
    for (name, outcome) in names.iter().zip(&results) {
        if *name == "no-such-host" {
            assert!(
                matches!(outcome, TargetOutcome::ConnectFailed(error)
                    if error.kind == FailureKind::Refused && error.place == FailurePlace::Target),
                "{name}: {outcome:?}"
            );
            continue;
        }
        let TargetOutcome::Finished(output) = outcome else {
            panic!("{name}: {outcome:?}");
        };
        assert_eq!(output.exit_status, Some(7), "{name}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains(name) && text.contains("err"),
            "{name}: {text:?}"
        );
    }
}

struct LabKeys(Vec<Arc<russh::keys::PrivateKey>>);

impl shellpp_ssh::AgentKeys for LabKeys {
    fn keys(&self) -> Vec<Arc<russh::keys::PrivateKey>> {
        self.0.clone()
    }
}

/// Agent forwarding with the built-in agent: jump1 sees the key
/// (`ssh-add -l`) and its own `ssh` logs in to app1 with it, so the
/// signature made here is accepted by a real OpenSSH server.
async fn the_built_in_agent_signs_for_the_next_hop(lab: &Lab) {
    let key = russh::keys::PrivateKey::from_openssh(lab.key_user.1.as_bytes()).expect("lab key");
    let fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let (user, password) = &lab.password_user;
    let connection = connect(
        ConnectSpec {
            forward_agent: Some(shellpp_ssh::ForwardedAgent {
                keys: Arc::new(LabKeys(vec![Arc::new(key)])),
                system: None,
                confirm: None,
            }),
            ..spec(lab.jump1, Some(user), password_plan(password))
        },
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("jump1");
    let mut shell = connection
        .open_shell(ShellRequest {
            agent_forwarding: true,
            ..shell_request()
        })
        .await
        .expect("shell");
    shell.writer().write(b"ssh-add -l\r").await.unwrap();
    read_until(&mut shell, &fingerprint, SHORT).await;
    let login = format!(
        "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o BatchMode=yes \
         -o LogLevel=ERROR {}@app1 'echo on-$(hostname)'\r",
        lab.key_user.0
    );
    shell.writer().write(login.as_bytes()).await.unwrap();
    read_until(&mut shell, "on-app1", Duration::from_secs(20)).await;
    connection.disconnect().await;
}

/// tinyproxy (HTTP CONNECT with Basic authentication) and microsocks
/// (SOCKS5 with username/password) carry the first connection of a route;
/// the proxy resolves the lab's names. Wrong proxy credentials are the
/// proxy's failure.
async fn real_proxies_carry_the_first_connection(lab: &Lab) {
    let (user, password) = &lab.password_user;
    let (proxy_user, proxy_password) = &lab.proxy_user;
    let proxy = |kind, addr: SocketAddr, secret: &str| shellpp_ssh::ProxySpec {
        kind,
        address: addr.ip().to_string(),
        port: addr.port(),
        username: Some(proxy_user.clone()),
        password: Some(SecretString::from(secret.to_owned())),
    };
    for (kind, addr) in [
        (shellpp_ssh::ProxyKind::Http, lab.http_proxy),
        (shellpp_ssh::ProxyKind::Socks5, lab.socks_proxy),
    ] {
        // Straight to the target (only the proxy and jump1 are published):
        // the proxy reaches it by its lab name.
        let connection = connect(
            ConnectSpec {
                proxy: Some(proxy(kind, addr, proxy_password)),
                ..lab_spec("target", password_plan(password), user)
            },
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .unwrap_or_else(|error| panic!("{kind:?}: {error}"));
        assert_eq!(
            run(&connection, "hostname").await.trim(),
            "target",
            "{kind:?}"
        );
        connection.disconnect().await;

        // The proxy, then a chain: proxy -> app1 -> jump2.
        let hop = TestInteraction::accepting();
        let connection = connect(
            ConnectSpec {
                proxy: Some(proxy(kind, addr, proxy_password)),
                jump_hosts: vec![JumpHost {
                    spec: lab_spec("app1", password_plan(password), user),
                    interaction: hop.as_dyn(),
                }],
                ..lab_spec("jump2", password_plan(password), user)
            },
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .unwrap_or_else(|error| panic!("{kind:?} with a hop: {error}"));
        assert_eq!(
            run(&connection, "hostname").await.trim(),
            "jump2",
            "{kind:?}"
        );
        assert!(
            lab.fingerprints("app1")
                .contains(&hop.host_keys()[0].fingerprint)
        );
        connection.disconnect().await;

        let error = connect(
            ConnectSpec {
                proxy: Some(proxy(kind, addr, "wrong-password")),
                ..lab_spec("target", password_plan(password), user)
            },
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .expect_err("wrong proxy password");
        assert_eq!(
            (error.kind, error.place),
            (FailureKind::ProxyAuthFailed, FailurePlace::Proxy),
            "{kind:?}"
        );
    }
}
