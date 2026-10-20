//! The provider-parameterized suite (stage 2 plan, section 4.2): the same
//! scenarios against the server chosen by `SHELLPP_TEST_SSHD`: unset or
//! `russh` = `shellpp-test-sshd` in process; `openssh` = the Docker OpenSSH
//! server of `tests/openssh` (skipped with a printed reason when Docker is
//! not available). Scenarios: host key presentation, password (saved and
//! prompted after a refusal), public key, keyboard-interactive with a masked
//! question, shell output, PTY size, TERM, env, exit status, exec, agent
//! forwarding, answered keepalives and Export to host (authorized_keys).

mod support;

use std::net::SocketAddr;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use shellpp_ssh::{
    AuthMethod, AuthPlan, Connection, ExportOutcome, KeyCredential, Prompt, SessionEnd,
    ShellRequest, SystemAgent, connect, export_public_key,
};
use shellpp_test_sshd::{ExecAnswer, KeyType, ServerConfig, TestServer, generate_key};
use support::agent::TestAgent;
use support::openssh::{OpenSshServer, Provider, provider};
use support::*;

/// Shell commands and their expected output per provider.
struct Commands {
    /// A command that prints a marker, and the marker as printed (it must
    /// not appear in the echoed command line).
    echo: &'static str,
    echo_output: &'static str,
    size: &'static str,
    size_output: &'static str,
    term: &'static str,
    env: &'static str,
    agent: &'static str,
}

const RUSSH_COMMANDS: Commands = Commands {
    echo: "echo shellpp-marker\r",
    echo_output: "\r\nshellpp-marker\r\n",
    size: "size\r",
    size_output: "100x40\r\n",
    term: "term\r",
    env: "env SHELLPP_TEST\r",
    agent: "agent-keys\r",
};

const OPENSSH_COMMANDS: Commands = Commands {
    // bash may wrap output in bracketed-paste mode switches; the computed
    // marker never appears in the echoed command line.
    echo: "echo shellpp-$((6*7))\r",
    echo_output: "shellpp-42\r\n",
    size: "stty size\r",
    size_output: "40 100\r\n",
    term: "echo \"$TERM\"\r",
    env: "echo \"$SHELLPP_TEST\"\r",
    agent: "ssh-add -l\r",
};

/// What the scenarios need from a provider.
struct Target {
    provider: Provider,
    addr: SocketAddr,
    /// Key type -> `SHA256:` fingerprint.
    host_keys: Vec<(String, String)>,
    password_user: (String, String),
    /// Name and private key text.
    key_user: (String, String),
    /// Address, name and the answer of the single masked question.
    ki_user: (SocketAddr, String, String),
    commands: &'static Commands,
}

/// Keeps the russh servers alive for the suite.
struct RusshServers {
    main: TestServer,
    ki: TestServer,
}

async fn russh_target() -> (Target, RusshServers) {
    let password = random_secret("pw");
    let key = generate_key(KeyType::Ed25519, "provider", None).expect("key");
    let mut config = ServerConfig::new(vec![
        password_user("pwuser", &password),
        key_user("keyuser", &key.public_openssh),
    ]);
    config.exec = Some(ExecAnswer {
        stdout: "Linux\n".to_owned(),
        exit_status: 0,
    });
    let main = start_with(config).await;
    let answer = random_secret("ki");
    let ki = start_server(vec![shellpp_test_sshd::UserConfig {
        keyboard_interactive: vec![question("Password: ", false, &answer)],
        ..user("kiuser")
    }])
    .await;
    let target = Target {
        provider: Provider::Russh,
        addr: main.addr(),
        host_keys: vec![(main.host_key_type(), main.host_key_fingerprint())],
        password_user: ("pwuser".to_owned(), password),
        key_user: (
            "keyuser".to_owned(),
            key.private_openssh.expose_secret().to_owned(),
        ),
        ki_user: (ki.addr(), "kiuser".to_owned(), answer),
        commands: &RUSSH_COMMANDS,
    };
    (target, RusshServers { main, ki })
}

fn openssh_target(server: &OpenSshServer) -> Target {
    Target {
        provider: Provider::OpenSsh,
        addr: server.addr,
        host_keys: server.host_keys.clone(),
        password_user: server.password_user.clone(),
        key_user: server.key_user.clone(),
        ki_user: (
            server.addr,
            server.ki_user.0.clone(),
            server.ki_user.1.clone(),
        ),
        commands: &OPENSSH_COMMANDS,
    }
}

async fn key_connection(target: &Target, private_key: &str) -> Connection {
    let plan = AuthPlan {
        key: Some(KeyCredential {
            label: "provider key".to_owned(),
            private_key: secret(private_key),
            passphrase: None,
        }),
        ..AuthPlan::default()
    };
    connect(
        spec(target.addr, Some(&target.key_user.0), plan),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("key connection")
}

async fn password_connection(target: &Target) -> Connection {
    connect(
        spec(
            target.addr,
            Some(&target.password_user.0),
            password_plan(&target.password_user.1),
        ),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("password connection")
}

async fn host_key_is_presented(target: &Target) {
    let interaction = TestInteraction::accepting();
    let connection = connect(
        spec(
            target.addr,
            Some(&target.password_user.0),
            password_plan(&target.password_user.1),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects");
    let presented = interaction.host_keys();
    assert_eq!(presented.len(), 1);
    let expected = target
        .host_keys
        .iter()
        .find(|(key_type, _)| *key_type == presented[0].key_type)
        .map(|(_, fingerprint)| fingerprint.clone());
    assert_eq!(Some(presented[0].fingerprint.clone()), expected);
    assert!(connection.server_ident().starts_with("SSH-2.0-"));
    connection.disconnect().await;

    // Decision 21: with a key type saved for the host, a server that has
    // several host keys presents that one (RSA keys with rsa-sha2-*).
    for (key_type, fingerprint) in &target.host_keys {
        let interaction = TestInteraction::rejecting();
        let mut spec = spec(
            target.addr,
            Some(&target.password_user.0),
            password_plan(&target.password_user.1),
        );
        spec.preferred_host_key_types = vec![key_type.clone()];
        let _ = connect(spec, interaction.as_dyn()).await;
        let presented = interaction.host_keys();
        assert_eq!(presented.len(), 1, "{key_type}");
        assert_eq!(&presented[0].key_type, key_type);
        assert_eq!(&presented[0].fingerprint, fingerprint);
    }
}

async fn password_after_a_refusal(target: &Target) {
    let interaction = TestInteraction::answering(vec![text(&target.password_user.1)]);
    let connection = connect(
        spec(
            target.addr,
            Some(&target.password_user.0),
            password_plan(&random_secret("stale")),
        ),
        interaction.as_dyn(),
    )
    .await
    .expect("connects with the prompted password");
    assert_eq!(connection.auth().method, AuthMethod::Password);
    assert!(matches!(
        interaction.prompts().as_slice(),
        [Prompt::Password { error: true, .. }]
    ));
    connection.disconnect().await;
}

async fn public_key(target: &Target) {
    let plan = AuthPlan {
        key: Some(KeyCredential {
            label: "provider key".to_owned(),
            private_key: secret(&target.key_user.1),
            passphrase: None,
        }),
        ..AuthPlan::default()
    };
    let connection = connect(
        spec(target.addr, Some(&target.key_user.0), plan),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("public key connection");
    assert_eq!(connection.auth().method, AuthMethod::PublicKey);
    connection.disconnect().await;
}

async fn keyboard_interactive(target: &Target) {
    let (addr, name, answer) = &target.ki_user;
    let interaction = TestInteraction::answering(vec![text(answer)]);
    let plan = AuthPlan {
        keyboard_interactive: true,
        ..AuthPlan::default()
    };
    let connection = connect(spec(*addr, Some(name), plan), interaction.as_dyn())
        .await
        .expect("keyboard-interactive connection");
    assert_eq!(connection.auth().method, AuthMethod::KeyboardInteractive);
    match interaction.prompts().as_slice() {
        [
            Prompt::KeyboardInteractive {
                echo, error, count, ..
            },
        ] => {
            assert!(!echo, "the password question is masked");
            assert!(!error);
            assert_eq!(*count, 1);
        }
        other => panic!("unexpected prompts {other:?}"),
    }
    connection.disconnect().await;
}

async fn shell_round_trip(target: &Target) {
    let connection = password_connection(target).await;
    let request = ShellRequest {
        term: "xterm-256color".to_owned(),
        cols: 100,
        rows: 40,
        env: vec![(
            "SHELLPP_TEST".to_owned(),
            SecretString::from("value-1".to_owned()),
        )],
        agent_forwarding: false,
    };
    let mut shell = connection.open_shell(request).await.expect("shell");
    let writer = shell.writer();
    // Give an OS shell time to print its prompt.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let commands = target.commands;
    for (command, output) in [
        (commands.echo, commands.echo_output),
        (commands.size, commands.size_output),
        (commands.term, "xterm-256color\r\n"),
        (commands.env, "value-1\r\n"),
    ] {
        writer.write(command.as_bytes()).await.expect("write");
        read_until(&mut shell, output, SHORT).await;
    }
    writer.write(b"exit 3\r").await.expect("write");
    let output = read_to_end(&mut shell, SHORT).await;
    assert_eq!(output.end, Some(SessionEnd::ExitStatus(3)), "{output:?}");
    connection.disconnect().await;
}

async fn exec(target: &Target) {
    let connection = password_connection(target).await;
    let output = connection
        .exec("uname -s", Duration::from_secs(10), 4096)
        .await
        .expect("exec");
    assert!(output.stdout.starts_with(b"Linux"), "{output:?}");
    assert_eq!(output.exit_status, Some(0));
    connection.disconnect().await;
}

/// Export to host (UI 7.3, stage 3): the key is appended once, the modes
/// are 700 and 600, and the key then signs in. The russh server answers
/// every exec with exit 0, so there only the mapping of the answer runs.
async fn export_key(target: &Target) {
    let generated = generate_key(KeyType::Ed25519, "exported key", None).expect("key");
    // OpenSSH: the key user (whose authorized_keys already holds its own
    // key), so that the exported key can sign in too; the password user of
    // the test server accepts passwords only.
    let connection = match target.provider {
        Provider::OpenSsh => key_connection(target, &target.key_user.1).await,
        Provider::Russh => password_connection(target).await,
    };
    if target.provider == Provider::OpenSsh {
        // A file that does not end with a line break: the key still gets a
        // line of its own.
        let tail = connection
            .exec(
                "printf %s \"# no line break at the end\" >> \"$HOME/.ssh/authorized_keys\"",
                Duration::from_secs(10),
                4096,
            )
            .await
            .expect("printf");
        assert_eq!(tail.exit_status, Some(0));
    }
    let outcome = export_public_key(&connection, &generated.public_openssh)
        .await
        .expect("export");
    assert_eq!(outcome, ExportOutcome::Added);
    if target.provider == Provider::OpenSsh {
        let again = export_public_key(&connection, &generated.public_openssh)
            .await
            .expect("export again");
        assert_eq!(again, ExportOutcome::AlreadyPresent);
        let modes = connection
            .exec(
                "stat -c %a \"$HOME/.ssh\" \"$HOME/.ssh/authorized_keys\"",
                Duration::from_secs(10),
                4096,
            )
            .await
            .expect("stat");
        assert_eq!(String::from_utf8_lossy(&modes.stdout), "700\n600\n");
        let count = connection
            .exec(
                &format!(
                    "grep -cF {} \"$HOME/.ssh/authorized_keys\"",
                    generated.public_openssh.split_whitespace().nth(1).unwrap()
                ),
                Duration::from_secs(10),
                4096,
            )
            .await
            .expect("grep");
        assert_eq!(
            String::from_utf8_lossy(&count.stdout),
            "1\n",
            "no duplicate"
        );
        let own_line = connection
            .exec(
                &format!(
                    "grep -c \"^ssh-ed25519 {}\" \"$HOME/.ssh/authorized_keys\"",
                    generated.public_openssh.split_whitespace().nth(1).unwrap()
                ),
                Duration::from_secs(10),
                4096,
            )
            .await
            .expect("grep line start");
        assert_eq!(
            String::from_utf8_lossy(&own_line.stdout),
            "1\n",
            "on its own line"
        );
        connection.disconnect().await;
        let plan = AuthPlan {
            key: Some(KeyCredential {
                label: "exported".to_owned(),
                private_key: generated.private_openssh.clone(),
                passphrase: None,
            }),
            ..AuthPlan::default()
        };
        let signed_in = connect(
            spec(target.addr, Some(&target.key_user.0), plan),
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .expect("the exported key signs in");
        assert_eq!(signed_in.auth().method, AuthMethod::PublicKey);
        signed_in.disconnect().await;
    } else {
        connection.disconnect().await;
    }
}

async fn agent_forwarding(target: &Target) {
    let generated = generate_key(KeyType::Ed25519, "forwarded", None).expect("key");
    let key = russh::keys::decode_secret_key(generated.private_openssh.expose_secret(), None)
        .expect("decodes");
    let agent = TestAgent::start(vec![key]).await;
    let mut spec = spec(
        target.addr,
        Some(&target.password_user.0),
        password_plan(&target.password_user.1),
    );
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
    tokio::time::sleep(Duration::from_millis(300)).await;
    shell
        .writer()
        .write(target.commands.agent.as_bytes())
        .await
        .expect("write");
    let expected = match target.provider {
        Provider::Russh => "agent keys: 1".to_owned(),
        Provider::OpenSsh => generated.fingerprint.clone(),
    };
    read_until(&mut shell, &expected, SHORT).await;
    assert!(agent.requests() >= 1);
    connection.disconnect().await;
}

async fn keepalive_is_answered(target: &Target) {
    let mut spec = spec(
        target.addr,
        Some(&target.password_user.0),
        password_plan(&target.password_user.1),
    );
    spec.keepalive = Some(Duration::from_millis(250));
    let connection = connect(spec, TestInteraction::accepting().as_dyn())
        .await
        .expect("connects");
    let mut shell = connection.open_shell(shell_request()).await.expect("shell");
    // Idle for more than three keepalive intervals: answered keepalives
    // keep the connection.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!connection.is_closed());
    let commands = target.commands;
    shell
        .writer()
        .write(commands.echo.as_bytes())
        .await
        .expect("write");
    read_until(&mut shell, commands.echo_output, SHORT).await;
    connection.disconnect().await;
}

async fn run_suite(target: &Target) {
    host_key_is_presented(target).await;
    password_after_a_refusal(target).await;
    public_key(target).await;
    keyboard_interactive(target).await;
    shell_round_trip(target).await;
    exec(target).await;
    export_key(target).await;
    agent_forwarding(target).await;
    keepalive_is_answered(target).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_suite() {
    match provider() {
        Provider::Russh => {
            let (target, servers) = russh_target().await;
            run_suite(&target).await;
            servers.main.shutdown().await;
            servers.ki.shutdown().await;
        }
        Provider::OpenSsh => {
            let Some(server) = OpenSshServer::start() else {
                return;
            };
            let target = openssh_target(&server);
            run_suite(&target).await;
            drop(server);
        }
    }
}

/// The reader of `server.json` (OpenSSH provider).
#[test]
fn server_json_is_read() {
    let text = r#"{
  "host": "127.0.0.1",
  "port": 32768,
  "host_keys": {"ssh-ed25519":"SHA256:abc","ssh-rsa":"SHA256:d\/ef"},
  "users": {"password": {"name": "pwuser", "password": "x\"y"}},
  "container": "c", "list": [1, true, null]
}"#;
    let json = support::json::parse(text).expect("valid");
    assert_eq!(
        json.get("port").and_then(support::json::Json::number),
        Some(32768.0)
    );
    assert_eq!(
        json.path(&["users", "password", "password"])
            .and_then(support::json::Json::str),
        Some("x\"y")
    );
    assert_eq!(json.get("host_keys").map(|k| k.entries().len()), Some(2));
    assert_eq!(
        json.path(&["host_keys", "ssh-rsa"])
            .and_then(support::json::Json::str),
        Some("SHA256:d/ef")
    );
}
