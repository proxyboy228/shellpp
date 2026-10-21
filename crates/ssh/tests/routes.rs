//! Routes (stage 5) against the russh test server: jump hosts over
//! `direct-tcpip` (each hop with its own credentials and host key decision,
//! failures named by their place) and the HTTP CONNECT and SOCKS5 proxies of
//! the first connection (small proxies in process, `support::proxy`).

mod support;

use std::net::SocketAddr;
use std::sync::Arc;

use shellpp_ssh::{
    ConnectSpec, FailureKind, FailurePlace, HostKeyDecision, JumpHost, Prompt, ProxyKind,
    ProxySpec, connect,
};
use shellpp_test_sshd::{ServerConfig, ServerEvent, TestServer};
use support::proxy::{HttpProxy, SocksProxy};
use support::*;

/// A russh server with one password user; `forwarding` lets it be a jump host.
async fn server(user: &str, password: &str, forwarding: bool) -> TestServer {
    let mut config = ServerConfig::new(vec![password_user(user, password)]);
    config.forwarding = forwarding;
    start_with(config).await
}

fn hop(
    addr: SocketAddr,
    user: &str,
    password: &str,
    interaction: &Arc<TestInteraction>,
) -> JumpHost {
    JumpHost {
        spec: spec(addr, Some(user), password_plan(password)),
        interaction: interaction.as_dyn(),
    }
}

/// Three servers: two jump hosts and the target, each with its own user.
struct Route {
    jump1: TestServer,
    jump2: TestServer,
    target: TestServer,
    passwords: [String; 3],
}

impl Route {
    async fn start() -> Route {
        let passwords = [
            random_secret("jump1"),
            random_secret("jump2"),
            random_secret("target"),
        ];
        Route {
            jump1: server("bastion", &passwords[0], true).await,
            jump2: server("inner", &passwords[1], true).await,
            target: server("app", &passwords[2], false).await,
            passwords,
        }
    }

    fn spec(&self, interactions: &[Arc<TestInteraction>; 3]) -> ConnectSpec {
        ConnectSpec {
            jump_hosts: vec![
                hop(
                    self.jump1.addr(),
                    "bastion",
                    &self.passwords[0],
                    &interactions[0],
                ),
                hop(
                    self.jump2.addr(),
                    "inner",
                    &self.passwords[1],
                    &interactions[1],
                ),
            ],
            ..spec(
                self.target.addr(),
                Some("app"),
                password_plan(&self.passwords[2]),
            )
        }
    }
}

fn accepting3() -> [Arc<TestInteraction>; 3] {
    [
        TestInteraction::accepting(),
        TestInteraction::accepting(),
        TestInteraction::accepting(),
    ]
}

fn direct_tcpip(server: &TestServer) -> Vec<(String, u32)> {
    server
        .events()
        .into_iter()
        .filter_map(|event| match event {
            ServerEvent::DirectTcpip { host, port } => Some((host, port)),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_jump_hosts_reach_the_target_and_each_hop_decides_its_host_key() {
    let route = Route::start().await;
    let interactions = accepting3();
    let connection = connect(route.spec(&interactions), interactions[2].as_dyn())
        .await
        .expect("the route connects");
    assert_eq!(connection.jump_hosts(), 2);

    // Each connection presented its own host key to its own interaction
    // (TOFU on every hop).
    for (interaction, server) in
        interactions
            .iter()
            .zip([&route.jump1, &route.jump2, &route.target])
    {
        let keys = interaction.host_keys();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].fingerprint, server.host_key_fingerprint());
    }
    // jump1 opened the channel to jump2, jump2 the one to the target.
    assert_eq!(
        direct_tcpip(&route.jump1),
        vec![("127.0.0.1".to_owned(), u32::from(route.jump2.port()))]
    );
    assert_eq!(
        direct_tcpip(&route.jump2),
        vec![("127.0.0.1".to_owned(), u32::from(route.target.port()))]
    );
    assert!(direct_tcpip(&route.target).is_empty());

    // The shell is the target's.
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    let output = read_until(&mut shell, "app@", SHORT).await;
    assert!(output.contains("app@"), "{output:?}");

    // Disconnecting closes the whole route.
    connection.disconnect().await;
    drop(shell);
    drop(connection);
    for server in [&route.jump1, &route.jump2, &route.target] {
        assert!(
            eventually(SHORT, || server.connection_count() == 0).await,
            "every connection of the route closes"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failure_on_the_second_jump_host_names_that_hop() {
    // Wrong password on jump2.
    let route = Route::start().await;
    let interactions = accepting3();
    let mut spec = route.spec(&interactions);
    spec.jump_hosts[1].spec.auth = password_plan("wrong");
    let error = connect(spec, interactions[2].as_dyn())
        .await
        .expect_err("jump2 refuses");
    // jump2's own interaction asked for its password again; the scripted
    // answer cancels it.
    assert_eq!(
        interactions[1].prompts(),
        vec![Prompt::Password {
            username: "inner".to_owned(),
            error: true,
            can_save: false
        }]
    );
    assert!(interactions[0].prompts().is_empty());
    assert_eq!(error.kind, FailureKind::Cancelled);
    assert_eq!(error.place, FailurePlace::JumpHost(1));
    assert_eq!(error.to_string(), "ssh failure: cancelled (jump host 2)");
    // The target was never reached.
    assert!(interactions[2].host_keys().is_empty());
    assert!(direct_tcpip(&route.jump2).is_empty());

    // jump2's host key rejected: nothing is sent to it, the target is not reached.
    let interactions = accepting3();
    interactions[1].queue_decisions(&[HostKeyDecision::Reject]);
    let seen_before = route.jump2.events().len();
    let error = connect(route.spec(&interactions), interactions[2].as_dyn())
        .await
        .expect_err("rejected");
    assert_eq!(error.kind, FailureKind::HostKeyRejected);
    assert_eq!(error.place, FailurePlace::JumpHost(1));
    assert!(interactions[2].host_keys().is_empty());
    assert!(
        !route.jump2.events()[seen_before..]
            .iter()
            .any(|event| matches!(
                event,
                ServerEvent::Auth { user, .. } if user == "inner"
            )),
        "no authentication after a rejected key"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_jump_host_without_forwarding_refuses_the_route() {
    let password = random_secret("pw");
    let jump = server("bastion", &password, false).await;
    let target = server("app", &password, false).await;
    let jump_interaction = TestInteraction::accepting();
    let route = ConnectSpec {
        jump_hosts: vec![hop(jump.addr(), "bastion", &password, &jump_interaction)],
        ..spec(target.addr(), Some("app"), password_plan(&password))
    };
    let error = connect(route, TestInteraction::accepting().as_dyn())
        .await
        .expect_err("no forwarding");
    assert_eq!(error.kind, FailureKind::ForwardingRefused);
    assert_eq!(error.place, FailurePlace::JumpHost(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_target_the_jump_host_cannot_reach_is_refused_at_the_target() {
    let password = random_secret("pw");
    let jump = server("bastion", &password, true).await;
    let closed = support::net::closed_port().await;
    let jump_interaction = TestInteraction::accepting();
    let route = ConnectSpec {
        jump_hosts: vec![hop(jump.addr(), "bastion", &password, &jump_interaction)],
        ..spec(closed, Some("app"), password_plan(&password))
    };
    let error = connect(route, TestInteraction::accepting().as_dyn())
        .await
        .expect_err("closed port");
    assert_eq!(error.kind, FailureKind::Refused);
    assert_eq!(error.place, FailurePlace::Target);
}

fn proxy_spec(kind: ProxyKind, addr: SocketAddr, credentials: Option<(&str, &str)>) -> ProxySpec {
    ProxySpec {
        kind,
        address: addr.ip().to_string(),
        port: addr.port(),
        username: credentials.map(|(user, _)| user.to_owned()),
        password: credentials.map(|(_, password)| secret(password)),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_and_socks5_proxies_carry_the_first_connection() {
    let password = random_secret("pw");
    let target = server("app", &password, false).await;
    let proxy_password = random_secret("proxy");
    let http = HttpProxy::start(Some(("proxyuser", &proxy_password))).await;
    let socks = SocksProxy::start(Some(("proxyuser", &proxy_password))).await;
    for (kind, addr) in [
        (ProxyKind::Http, http.addr()),
        (ProxyKind::Socks5, socks.addr()),
    ] {
        let route = ConnectSpec {
            // The proxy resolves the name.
            address: "localhost".to_owned(),
            proxy: Some(proxy_spec(kind, addr, Some(("proxyuser", &proxy_password)))),
            ..spec(target.addr(), Some("app"), password_plan(&password))
        };
        let connection = connect(route, TestInteraction::accepting().as_dyn())
            .await
            .unwrap_or_else(|error| panic!("{kind:?}: {error}"));
        let mut shell = connection.open_shell(shell_request()).await.expect("shell");
        read_until(&mut shell, "app@", SHORT).await;
        connection.disconnect().await;
    }
    assert_eq!(
        http.requests(),
        vec![format!("localhost:{}", target.port())]
    );
    assert_eq!(
        socks.requests(),
        vec![format!("localhost:{}", target.port())]
    );

    // Wrong proxy credentials: the failure is the proxy's.
    for (kind, addr) in [
        (ProxyKind::Http, http.addr()),
        (ProxyKind::Socks5, socks.addr()),
    ] {
        let route = ConnectSpec {
            proxy: Some(proxy_spec(kind, addr, Some(("proxyuser", "wrong")))),
            ..spec(target.addr(), Some("app"), password_plan(&password))
        };
        let error = connect(route, TestInteraction::accepting().as_dyn())
            .await
            .expect_err("wrong proxy password");
        assert_eq!(error.kind, FailureKind::ProxyAuthFailed, "{kind:?}");
        assert_eq!(error.place, FailurePlace::Proxy, "{kind:?}");
    }

    // A proxy that is not running.
    let closed = support::net::closed_port().await;
    let route = ConnectSpec {
        proxy: Some(proxy_spec(ProxyKind::Socks5, closed, None)),
        ..spec(target.addr(), Some("app"), password_plan(&password))
    };
    let error = connect(route, TestInteraction::accepting().as_dyn())
        .await
        .expect_err("no proxy");
    assert_eq!(error.kind, FailureKind::Refused);
    assert_eq!(error.place, FailurePlace::Proxy);

    // The proxy cannot reach the target: the failure is the target's.
    let route = ConnectSpec {
        proxy: Some(proxy_spec(
            ProxyKind::Socks5,
            socks.addr(),
            Some(("proxyuser", &proxy_password)),
        )),
        ..spec(closed, Some("app"), password_plan(&password))
    };
    let error = connect(route, TestInteraction::accepting().as_dyn())
        .await
        .expect_err("closed target");
    assert_eq!(error.kind, FailureKind::Refused);
    assert_eq!(error.place, FailurePlace::Target);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_proxy_carries_the_first_jump_host_only() {
    let password = random_secret("pw");
    let jump = server("bastion", &password, true).await;
    let target = server("app", &password, false).await;
    let socks = SocksProxy::start(None).await;
    let jump_interaction = TestInteraction::accepting();
    let route = ConnectSpec {
        jump_hosts: vec![hop(jump.addr(), "bastion", &password, &jump_interaction)],
        proxy: Some(proxy_spec(ProxyKind::Socks5, socks.addr(), None)),
        ..spec(target.addr(), Some("app"), password_plan(&password))
    };
    let connection = connect(route, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    // The proxy saw the jump host only; the jump host opened the target.
    assert_eq!(
        socks.requests(),
        vec![format!("{}:{}", jump.addr().ip(), jump.port())]
    );
    assert_eq!(
        direct_tcpip(&jump),
        vec![("127.0.0.1".to_owned(), u32::from(target.port()))]
    );
    connection.disconnect().await;
}
