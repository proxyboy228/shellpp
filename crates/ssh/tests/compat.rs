//! The server compatibility matrix (run 4, plan 9.1; docs/compatibility.md):
//! one server of the matrix per run, described by `server.json` of
//! `tests/openssh/start.sh` (`SHELLPP_TEST_SSHD=openssh` and
//! `SHELLPP_OPENSSH_SERVER_JSON`; skipped otherwise). The image is chosen
//! with `SHELLPP_OPENSSH_IMAGE` (latest OpenSSH, OpenSSH 7.4 on CentOS 7,
//! Dropbear) and the variant with `SHELLPP_OPENSSH_VARIANT` (legacy-sha1,
//! kbd-2fa, no-sftp, motd-banner). Each run checks password and key login,
//! the terminal, SFTP and a local forward, plus what its variant is about.

mod support;

use std::time::Duration;

use shellpp_ssh::{
    AlgorithmKind, AuthPlan, ConnectSpec, Connection, FailureKind, ForwardKind, ForwardSpec,
    KeyCredential, Prompt, ShellRequest, connect,
};
use support::openssh::{OpenSshServer, Provider, provider};
use support::*;
use tokio::io::AsyncReadExt;

/// The server's software, from its identification string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Software {
    OpenSsh,
    Dropbear,
}

fn software(ident: &str) -> Software {
    if ident.contains("dropbear") {
        Software::Dropbear
    } else {
        Software::OpenSsh
    }
}

/// The spec of the password user, with legacy algorithms when the variant
/// needs them.
fn password_spec(server: &OpenSshServer, legacy: bool) -> ConnectSpec {
    let mut spec = spec(
        server.addr,
        Some(&server.password_user.0),
        password_plan(&server.password_user.1),
    );
    spec.allow_legacy_algorithms = legacy;
    spec
}

async fn password_connection(server: &OpenSshServer, legacy: bool) -> (Connection, Vec<String>) {
    let interaction = TestInteraction::accepting();
    let connection = connect(password_spec(server, legacy), interaction.as_dyn())
        .await
        .expect("password login");
    (connection, interaction.banners())
}

/// legacy-sha1: without the host setting the handshake fails and names
/// what the server offered; with it the login works.
async fn legacy_algorithms_need_the_setting(server: &OpenSshServer) {
    let error = connect(
        password_spec(server, false),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect_err("no common algorithm without the setting");
    let FailureKind::NoCommonAlgorithm(mismatch) = error.kind else {
        panic!("expected a mismatch, got {error:?}");
    };
    // The key exchange is negotiated first.
    assert_eq!(mismatch.kind(), AlgorithmKind::KeyExchange);
    assert!(
        mismatch.offered().contains(&"diffie-hellman-group14-sha1"),
        "{:?}",
        mismatch.offered()
    );
    assert!(mismatch.legacy_would_help());
}

async fn key_login(server: &OpenSshServer, legacy: bool) {
    let two_factor = server.variant == "kbd-2fa";
    let plan = AuthPlan {
        key: Some(KeyCredential {
            label: "compat key".to_owned(),
            private_key: secret(&server.key_user.1),
            passphrase: None,
        }),
        // kbd-2fa: after the key the server asks a keyboard-interactive
        // question (the account's password).
        keyboard_interactive: two_factor,
        ..AuthPlan::default()
    };
    let interaction = if two_factor {
        TestInteraction::answering(vec![text(&server.ki_user.1)])
    } else {
        TestInteraction::accepting()
    };
    let mut spec = spec(server.addr, Some(&server.key_user.0), plan);
    spec.allow_legacy_algorithms = legacy;
    let connection = connect(spec, interaction.as_dyn())
        .await
        .expect("key login");
    if two_factor {
        assert!(
            matches!(
                interaction.prompts().as_slice(),
                [Prompt::KeyboardInteractive { echo: false, .. }]
            ),
            "the second factor was asked: {:?}",
            interaction.prompts()
        );
    }
    connection.disconnect().await;
}

async fn keyboard_interactive_login(server: &OpenSshServer, legacy: bool) {
    let (name, answer) = &server.ki_user;
    let interaction = TestInteraction::answering(vec![text(answer)]);
    let plan = AuthPlan {
        keyboard_interactive: true,
        ..AuthPlan::default()
    };
    let mut spec = spec(server.addr, Some(name), plan);
    spec.allow_legacy_algorithms = legacy;
    let connection = connect(spec, interaction.as_dyn())
        .await
        .expect("keyboard-interactive login");
    connection.disconnect().await;
}

async fn terminal(connection: &Connection, motd: bool) {
    let request = ShellRequest {
        term: "xterm-256color".to_owned(),
        cols: 120,
        rows: 40,
        env: Vec::new(),
        agent_forwarding: false,
    };
    let mut shell = connection.open_shell(request).await.expect("shell");
    if motd {
        read_until(&mut shell, "Message of the day, line 120", SHORT).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let writer = shell.writer();
    // The marker is computed: it never appears in the echoed command line.
    writer
        .write(b"echo shellpp-$((6*7)) $(stty size)\r")
        .await
        .expect("write");
    read_until(&mut shell, "shellpp-42 40 120", SHORT).await;
    // UTF-8 both ways.
    writer
        .write("printf '%s\\n' \"при$((1+1))вет\"\r".as_bytes())
        .await
        .expect("write");
    read_until(&mut shell, "при2вет", SHORT).await;
    writer.write(b"exit\r").await.expect("write");
    let _ = read_to_end(&mut shell, SHORT).await;
}

async fn sftp(connection: &Connection, available: bool) {
    match connection.open_sftp().await {
        Ok(sftp) => {
            assert!(available, "the server has no SFTP subsystem");
            let home = sftp.canonicalize(".").await.expect("home");
            let entries = sftp.list(&home).await.expect("list home");
            // The home folder lists (".." and "." are left out).
            assert!(entries.iter().all(|entry| entry.name != "."));
        }
        Err(error) => {
            assert!(!available, "SFTP failed: {error:?}");
            assert_eq!(error.kind, FailureKind::ChannelFailed);
        }
    }
}

/// A local rule to the server's own SSH port: its greeting comes back.
async fn local_forward(connection: &Connection) {
    let forward = connection
        .forward(ForwardSpec {
            kind: ForwardKind::Local {
                host: "127.0.0.1".to_owned(),
                port: 22,
            },
            bind_address: String::new(),
            bind_port: 0,
        })
        .await
        .expect("local rule");
    let shellpp_ssh::Bound::Local(addr) = forward.bound() else {
        panic!("a local listener");
    };
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let mut greeting = [0u8; 8];
    tokio::time::timeout(SHORT, stream.read_exact(&mut greeting))
        .await
        .expect("in time")
        .expect("read");
    assert_eq!(&greeting, b"SSH-2.0-");
    forward.stop().await;
}

#[allow(clippy::print_stderr)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compatibility_matrix_entry() {
    if provider() != Provider::OpenSsh || std::env::var_os("SHELLPP_OPENSSH_SERVER_JSON").is_none()
    {
        eprintln!(
            "skipped: the compatibility matrix runs against tests/openssh/start.sh \
             (SHELLPP_TEST_SSHD=openssh, SHELLPP_OPENSSH_SERVER_JSON)"
        );
        return;
    }
    let Some(server) = OpenSshServer::start() else {
        return;
    };
    let legacy = server.variant == "legacy-sha1";
    if legacy {
        legacy_algorithms_need_the_setting(&server).await;
    }
    let (connection, banners) = password_connection(&server, legacy).await;
    let software = software(connection.server_ident());
    eprintln!(
        "compat: {} (variant {:?})",
        connection.server_ident(),
        server.variant
    );
    if server.variant == "motd-banner" {
        assert!(
            banners
                .iter()
                .any(|banner| banner.contains("Shell++ compatibility test banner.")),
            "{banners:?}"
        );
    }
    terminal(&connection, server.variant == "motd-banner").await;
    sftp(&connection, server.variant != "no-sftp").await;
    local_forward(&connection).await;
    connection.disconnect().await;
    key_login(&server, legacy).await;
    // Dropbear has no keyboard-interactive authentication.
    if software == Software::OpenSsh {
        keyboard_interactive_login(&server, legacy).await;
    }
}
