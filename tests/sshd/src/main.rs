//! `shellpp-test-sshd` binary for the UI e2e harness (stage 2 plan, section
//! 4.1). Commands:
//!
//! ```text
//! shellpp-test-sshd serve --config <file.json>
//!     <file.json> holds the ServerConfig fields, for example
//!       {"users":[{"name":"alice","password":"...","authorized_keys":["ssh-ed25519 AAAA..."],
//!         "keyboard_interactive":[{"prompt":"Verification code: ","echo":false,"answer":"..."}],
//!         "ki_name":"...","ki_instruction":"..."}],
//!        "host_key":{"generate":"ed25519"},"banner_comment":"Ubuntu-3ubuntu13",
//!        "exec":{"stdout":"Linux\nID=debian\nVERSION_ID=12\n","exit_status":0},
//!        "auth_banner":"...","motd":"...","bind":"127.0.0.1:0"}
//!     stdout, one JSON object per line:
//!       {"event":"ready","port":N,"host_key_type":"ssh-ed25519",
//!        "host_key_fingerprint":"SHA256:...","host_key_base64":"AAAA..."}
//!     stdin, one command per line, each answered by one JSON line:
//!       rotate-host-key  -> {"event":"rotated","host_key_type":...,"host_key_fingerprint":...,"host_key_base64":...}
//!       drop             -> {"event":"dropped","count":N}
//!       freeze           -> {"event":"frozen","count":N}   (extension: connections stop answering)
//!       accept off|on    -> {"event":"accepting","value":false|true}
//!       hold on|off      -> {"event":"holding","value":true|false}   (extension: new
//!                           connections are accepted and frozen before the banner)
//!       events           -> {"event":"events","items":[...ServerEvent...]}
//!       quit             -> {"event":"bye"} and exit 0
//!       anything else, or a failed command -> {"event":"error","message":"..."}
//!     Empty lines are ignored. The server exits (status 0) when stdin closes,
//!     so the harness never leaves it running.
//!
//! shellpp-test-sshd keygen --type ed25519|ecdsa-p256|rsa-3072 --out <path>
//!                          [--passphrase-env <VAR>] [--comment <text>]
//!     writes <path> (OpenSSH private key, encrypted when the variable is set
//!     and non-empty) and <path>.pub, then prints
//!       {"public_key":"ssh-ed25519 AAAA... comment","fingerprint":"SHA256:..."}
//!     The default comment is "shellpp-test-key".
//! ```
//!
//! Exit status: 0 on success, 1 on a runtime error, 2 on a usage error.
//! Logs (tracing, level from `SHELLPP_TEST_SSHD_LOG`, default `warn`) go to
//! stderr only and never contain secrets, user names or terminal data.
//!
//! Owner: S1.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use shellpp_test_sshd::{KeyType, ServerConfig, TestServer, generate_key};

const USAGE: &str = "usage:
  shellpp-test-sshd serve --config <file.json>
  shellpp-test-sshd keygen --type ed25519|ecdsa-p256|rsa-3072 --out <path> [--passphrase-env <VAR>] [--comment <text>]
";

const DEFAULT_COMMENT: &str = "shellpp-test-key";

/// A failure: usage errors exit with 2, runtime errors with 1.
enum Failure {
    Usage(String),
    Runtime(String),
}

impl From<shellpp_test_sshd::Error> for Failure {
    fn from(error: shellpp_test_sshd::Error) -> Self {
        Self::Runtime(error.to_string())
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::Runtime(format!("i/o error: {error}"))
    }
}

fn main() -> ExitCode {
    init_logging();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("serve") => serve(&args[1..]),
        Some("keygen") => keygen(&args[1..]),
        Some("help" | "--help" | "-h") => {
            let _ = std::io::stdout().write_all(USAGE.as_bytes());
            Ok(())
        }
        _ => Err(Failure::Usage("missing or unknown command".to_owned())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            let _ = writeln!(std::io::stderr(), "shellpp-test-sshd: {message}\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Runtime(message)) => {
            let _ = writeln!(std::io::stderr(), "shellpp-test-sshd: {message}");
            ExitCode::from(1)
        }
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("SHELLPP_TEST_SSHD_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .try_init();
}

/// Parses `--name value` pairs; every option takes a value.
fn options(args: &[String], allowed: &[&str]) -> Result<HashMap<String, String>, Failure> {
    let mut parsed = HashMap::new();
    let mut iter = args.iter();
    while let Some(name) = iter.next() {
        if !allowed.contains(&name.as_str()) {
            return Err(Failure::Usage(format!("unknown option {name}")));
        }
        let value = iter
            .next()
            .ok_or_else(|| Failure::Usage(format!("{name} needs a value")))?;
        parsed.insert(name.clone(), value.clone());
    }
    Ok(parsed)
}

fn required<'a>(options: &'a HashMap<String, String>, name: &str) -> Result<&'a str, Failure> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| Failure::Usage(format!("{name} is required")))
}

/// Writes one JSON line to stdout and flushes it.
fn emit(value: &Value) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{value}");
    let _ = stdout.flush();
}

fn host_key_fields(server: &TestServer, event: &str) -> Value {
    json!({
        "event": event,
        "port": server.port(),
        "host_key_type": server.host_key_type(),
        "host_key_fingerprint": server.host_key_fingerprint(),
        "host_key_base64": server.host_key_base64(),
    })
}

fn error_line(message: &str) -> Value {
    json!({"event": "error", "message": message})
}

fn serve(args: &[String]) -> Result<(), Failure> {
    let options = options(args, &["--config"])?;
    let path = required(&options, "--config")?;
    let text = std::fs::read_to_string(path)
        .map_err(|error| Failure::Runtime(format!("cannot read the config file: {error}")))?;
    let config = ServerConfig::from_json(&text)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(run_server(config))
}

async fn run_server(config: ServerConfig) -> Result<(), Failure> {
    let server = TestServer::start(config).await?;
    emit(&host_key_fields(&server, "ready"));

    // stdin is read on a plain thread: a blocked read must not keep the
    // runtime from shutting down.
    let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    while let Some(line) = commands.recv().await {
        let command = line.trim();
        let reply = match command {
            "" => continue,
            "rotate-host-key" => match server.rotate_host_key().await {
                Ok(()) => {
                    let mut fields = host_key_fields(&server, "rotated");
                    if let Some(object) = fields.as_object_mut() {
                        object.remove("port");
                    }
                    fields
                }
                Err(error) => error_line(&error.to_string()),
            },
            "drop" => json!({"event": "dropped", "count": server.drop_connections()}),
            "freeze" => json!({"event": "frozen", "count": server.freeze_connections()}),
            "accept off" | "accept on" => {
                let value = command == "accept on";
                match server.set_accepting(value).await {
                    Ok(()) => json!({"event": "accepting", "value": value}),
                    Err(error) => error_line(&error.to_string()),
                }
            }
            "hold on" | "hold off" => {
                let value = command == "hold on";
                server.set_holding(value);
                json!({"event": "holding", "value": value})
            }
            "events" => json!({"event": "events", "items": server.events()}),
            "quit" => {
                server.shutdown().await;
                emit(&json!({"event": "bye"}));
                return Ok(());
            }
            _ => error_line("unknown command"),
        };
        emit(&reply);
    }
    server.shutdown().await;
    Ok(())
}

fn keygen(args: &[String]) -> Result<(), Failure> {
    let options = options(args, &["--type", "--out", "--passphrase-env", "--comment"])?;
    let key_type = KeyType::parse(required(&options, "--type")?).ok_or_else(|| {
        Failure::Usage("--type must be ed25519, ecdsa-p256 or rsa-3072".to_owned())
    })?;
    let out = PathBuf::from(required(&options, "--out")?);
    let passphrase = options
        .get("--passphrase-env")
        .and_then(|variable| std::env::var(variable).ok())
        .filter(|value| !value.is_empty())
        .map(SecretString::from);
    let comment = options
        .get("--comment")
        .map(String::as_str)
        .unwrap_or(DEFAULT_COMMENT);
    let key = generate_key(key_type, comment, passphrase.as_ref())?;
    write_private_key(&out, key.private_openssh.expose_secret())?;
    std::fs::write(public_key_path(&out), format!("{}\n", key.public_openssh))?;
    emit(&json!({"public_key": key.public_openssh, "fingerprint": key.fingerprint}));
    Ok(())
}

fn public_key_path(private: &Path) -> PathBuf {
    let mut path = private.as_os_str().to_owned();
    path.push(".pub");
    PathBuf::from(path)
}

/// Writes the private key readable by the owner only (Unix mode 0600; on
/// Windows the file inherits the ACL of its directory, a temporary one).
fn write_private_key(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    file.flush()
}
