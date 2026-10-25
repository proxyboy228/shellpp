//! The test server and clients of the end-to-end tests (docs/architecture.md,
//! section 9.1; docs/sync-protocol.md, section 12): the real server on a
//! local port in this process, and cores of the real sync client on
//! temporary data directories. Passwords and secrets are generated at run
//! time; nothing touches a real profile.
//!
//! `SHELLPP_TEST_POSTGRES=docker` runs every server on PostgreSQL: one
//! `postgres:17-alpine` container per test process, stopped and removed when
//! the process ends, one database per server.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use shellpp_core::platform::OsKind;
use shellpp_core::platform::memory::MemoryPlatform;
use shellpp_core::service::dto::{
    QueryScope, RecordBase, RecordRef, RecordSaveInput, RecordType, RecordView, RecordsQuery,
    SortMode,
};
use shellpp_core::service::{SignInInput, SignInOutcome, SyncStatus};
use shellpp_core::{Core, CoreConfig};
use shellpp_server::config::{Cidr, Config};
use shellpp_server::db::Db;
use shellpp_server::ratelimit::RateLimits;
use shellpp_server::state::Shared;
use shellpp_sync_api::api::Registration;
use tempfile::TempDir;
use uuid::Uuid;

pub const APP_VERSION: &str = "0.7.0";

pub fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("random");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn new_password() -> SecretString {
    SecretString::from(format!("Pw-{}-x", random_hex(10)))
}

pub fn email() -> String {
    format!("user-{}@example.com", random_hex(4))
}

// ---------------------------------------------------------------------------
// PostgreSQL in Docker
// ---------------------------------------------------------------------------

/// The label every container of a test run carries besides its kind
/// (`shellpp-test=<kind>`): `shellpp-run=<SHELLPP_TEST_RUN>`, `local` when
/// the variable is unset, so a run or an agent finds and removes only its
/// own containers.
pub fn run_label() -> &'static str {
    static LABEL: OnceLock<String> = OnceLock::new();
    LABEL.get_or_init(|| {
        let run = std::env::var("SHELLPP_TEST_RUN")
            .ok()
            .filter(|run| !run.is_empty())
            .unwrap_or_else(|| "local".to_owned());
        format!("shellpp-run={run}")
    })
}

struct Postgres {
    port: u16,
    password: String,
    /// `docker start -a -i`, holding the container's stdin. The container
    /// stops when it reads the end of it, which comes when this process
    /// ends however it ends (the OS closes the pipe), and `--rm` then
    /// removes it with its volume.
    _attach: std::process::Child,
}

/// The last lines of a command's output, for a failure message.
fn last_lines(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let lines: Vec<&str> = text.trim().lines().collect();
    lines[lines.len().saturating_sub(30)..].join("\n")
}

fn postgres() -> &'static Postgres {
    static PG: OnceLock<Postgres> = OnceLock::new();
    PG.get_or_init(|| {
        let password = random_hex(12);
        // The shell is the container's first process: when `cat` sees the
        // end of stdin it exits, and the container stops with it. The
        // timeout is only a backstop for a stdin that never closes.
        let output = Command::new("docker")
            .args([
                "create",
                "-i",
                "--rm",
                "--label",
                "shellpp-test=postgres",
                "--label",
                run_label(),
                "-e",
                &format!("POSTGRES_PASSWORD={password}"),
                "-p",
                "127.0.0.1::5432",
                "--entrypoint",
                "sh",
                "postgres:17-alpine",
                "-c",
                "docker-entrypoint.sh postgres & timeout 7200 cat > /dev/null",
            ])
            .output()
            .expect("docker runs");
        assert!(
            output.status.success(),
            "postgres container was not created: {}",
            last_lines(&output.stderr)
        );
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        // The container's own output (and docker's errors) go to a file in
        // the target folder, for the message when it does not start.
        let log_path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("postgres-{}.log", &id[..id.len().min(12)]));
        let log = std::fs::File::create(&log_path).expect("the container's log file");
        let log_out = log.try_clone().expect("the container's log file");
        let attach = Command::new("docker")
            .args(["start", "-a", "-i", &id])
            .stdin(std::process::Stdio::piped())
            .stdout(log_out)
            .stderr(log)
            .spawn()
            .expect("docker start runs");
        let failed = |what: &str| -> ! {
            panic!(
                "{what}; docker and the container said:\n{}",
                last_lines(&std::fs::read(&log_path).unwrap_or_default())
            )
        };
        let mut port = None;
        for _ in 0..120 {
            let port_out = Command::new("docker")
                .args(["port", &id, "5432/tcp"])
                .output()
                .expect("docker port");
            let text = String::from_utf8_lossy(&port_out.stdout);
            port = text
                .lines()
                .next()
                .and_then(|line| line.rsplit(':').next())
                .and_then(|port| port.trim().parse::<u16>().ok());
            if port.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let port = port.unwrap_or_else(|| failed("postgres has no mapped port"));
        // Ready when a connection and a query work.
        let url = format!("postgres://postgres:{password}@127.0.0.1:{port}/postgres");
        let mut ready = false;
        for _ in 0..120 {
            if let Ok(mut client) = postgres::Client::connect(&url, postgres::NoTls)
                && client.simple_query("SELECT 1").is_ok()
            {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        if !ready {
            failed("postgres did not become ready");
        }
        Postgres {
            port,
            password,
            _attach: attach,
        }
    })
}

/// The database URL of a new server: SQLite in `dir`, or a new database
/// on the test PostgreSQL.
pub fn database_url(dir: &Path) -> String {
    if std::env::var("SHELLPP_TEST_POSTGRES").as_deref() == Ok("docker") {
        let pg = postgres();
        let name = format!("t_{}", random_hex(6));
        let admin = format!(
            "postgres://postgres:{}@127.0.0.1:{}/postgres",
            pg.password, pg.port
        );
        let mut client = postgres::Client::connect(&admin, postgres::NoTls).expect("connect");
        client
            .simple_query(&format!("CREATE DATABASE {name}"))
            .expect("create database");
        format!(
            "postgres://postgres:{}@127.0.0.1:{}/{name}",
            pg.password, pg.port
        )
    } else {
        format!("sqlite:{}", dir.join("server.db").display())
    }
}

pub fn on_postgres() -> bool {
    std::env::var("SHELLPP_TEST_POSTGRES").as_deref() == Ok("docker")
}

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

pub struct Server {
    pub origin: String,
    pub db: Db,
    pub database_url: String,
    pub dir: TempDir,
    runtime: tokio::runtime::Runtime,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    pub config: Config,
}

pub struct Options {
    pub registration: Registration,
    pub quota_max_records: Option<u64>,
    pub tombstone_retention_days: u64,
    /// `SHELLPP_TRUSTED_PROXIES` (the tests connect from 127.0.0.1).
    pub trusted_proxies: Vec<Cidr>,
    pub rate_limits: RateLimits,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            registration: Registration::Open,
            quota_max_records: None,
            tombstone_retention_days: 90,
            trusted_proxies: Vec::new(),
            rate_limits: RateLimits::generous(),
        }
    }
}

impl Server {
    pub fn start() -> Server {
        Self::with(Options::default())
    }

    pub fn with(options: Options) -> Server {
        let dir = tempfile::tempdir().expect("temp dir");
        let database_url = database_url(dir.path());
        let db = Db::open(&database_url).expect("db opens");
        shellpp_server::migrate::migrate(&db).expect("migrated");
        let mut secret = vec![0u8; 32];
        getrandom::fill(&mut secret).expect("random");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind");
        let addr: SocketAddr = listener.local_addr().expect("addr");
        let origin = format!("http://127.0.0.1:{}", addr.port());
        let config = Config {
            database_url: database_url.clone(),
            bind: addr,
            public_url: origin.clone(),
            server_secret: secret.into(),
            registration: options.registration,
            quota_max_records: options.quota_max_records,
            quota_max_bytes: None,
            trusted_proxies: options.trusted_proxies,
            tombstone_retention_days: options.tombstone_retention_days,
            rate_limits: options.rate_limits,
            warnings: Vec::new(),
        };
        let mut server = Server {
            origin,
            db,
            database_url,
            dir,
            runtime,
            shutdown: None,
            task: None,
            config,
        };
        server.listen(listener);
        server
    }

    fn listen(&mut self, listener: tokio::net::TcpListener) {
        let db = Db::open(&self.database_url).expect("db opens");
        let shared: Arc<Shared> = Shared::new(db, self.config.clone());
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let task = self
            .runtime
            .spawn(shellpp_server::serve_on(listener, shared, async move {
                let _ = rx.await;
            }));
        self.shutdown = Some(tx);
        self.task = Some(task);
    }

    pub fn runtime(&self) -> &tokio::runtime::Runtime {
        &self.runtime
    }

    /// Stops serving (the port stays reserved for [`Server::resume`]).
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = self.runtime.block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(10), task).await
            });
        }
    }

    /// Serves again on the same port (after a restore of the database).
    pub fn resume(&mut self) {
        let addr = self.config.bind;
        let listener = self
            .runtime
            .block_on(async {
                for _ in 0..50 {
                    if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
                        return Ok(listener);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                tokio::net::TcpListener::bind(addr).await
            })
            .expect("rebind");
        self.db = Db::open(&self.database_url).expect("db opens");
        self.listen(listener);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Clients
// ---------------------------------------------------------------------------

pub struct Client {
    pub dir: TempDir,
    pub mem: MemoryPlatform,
    pub core: Core,
}

impl Client {
    fn open(dir: TempDir, mem: MemoryPlatform, name: &str) -> Client {
        let core = Core::open(
            CoreConfig {
                data_dir: dir.path().join("data"),
                app_version: APP_VERSION.to_owned(),
            },
            mem.platform(OsKind::Windows, name),
        )
        .expect("core opens");
        Client { dir, mem, core }
    }

    /// A device at first run (nothing local).
    pub fn empty(name: &str) -> Client {
        Self::open(
            tempfile::tempdir().expect("dir"),
            MemoryPlatform::new(),
            name,
        )
    }

    /// A device after its own first run with `password`.
    pub fn local(name: &str, password: &SecretString) -> Client {
        let client = Self::empty(name);
        client
            .core
            .first_run_setup(password.clone())
            .expect("first run");
        client
    }

    /// Opens the core again on the same data directory (locked).
    pub fn reopen(self, name: &str) -> Client {
        let Client { dir, mem, core } = self;
        drop(core);
        Self::open(dir, mem, name)
    }

    pub fn vault_id(&self) -> Uuid {
        self.core.vaults().expect("vaults")[0].vault_id
    }

    pub fn save(&self, record_id: Option<Uuid>, record_type: RecordType, data: Value) -> Uuid {
        let view = self
            .core
            .record_save(RecordSaveInput {
                base: None,
                vault_id: self.vault_id(),
                record_id,
                record_type,
                data: data.as_object().cloned().expect("object"),
                secrets: BTreeMap::new(),
            })
            .expect("saved");
        view.record_id()
    }

    /// The revision and data an editor starts from (`record_get`'s view).
    pub fn base(&self, record_id: Uuid) -> RecordBase {
        let view = self
            .core
            .record_get(RecordRef {
                vault_id: self.vault_id(),
                record_id,
            })
            .expect("record");
        let (revision, data) = match view {
            RecordView::Host(host) => (host.revision, host.data),
            RecordView::Group(group) => (group.revision, group.data),
        };
        RecordBase {
            revision,
            data: data.as_object().cloned().expect("object"),
        }
    }

    /// An editor's save (D220): `data` is the editor's form, and only the
    /// members it changes against `base` are written.
    pub fn save_from(
        &self,
        record_id: Uuid,
        record_type: RecordType,
        base: &RecordBase,
        data: Value,
    ) -> Result<RecordView, shellpp_core::CoreError> {
        self.core.record_save(RecordSaveInput {
            base: Some(base.clone()),
            vault_id: self.vault_id(),
            record_id: Some(record_id),
            record_type,
            data: data.as_object().cloned().expect("object"),
            secrets: BTreeMap::new(),
        })
    }

    pub fn host(&self, address: &str) -> Uuid {
        self.save(
            None,
            RecordType::Host,
            serde_json::json!({ "address": address }),
        )
    }

    pub fn delete(&self, record_id: Uuid) {
        self.core
            .record_delete(shellpp_core::service::dto::RecordsRef {
                vault_id: self.vault_id(),
                record_ids: vec![record_id],
            })
            .expect("deleted");
    }

    pub fn views(&self, types: &[RecordType]) -> Vec<RecordView> {
        self.core
            .records_query(RecordsQuery {
                vault_id: None,
                types: types.to_vec(),
                scope: QueryScope::All,
                text: None,
                tags: Vec::new(),
                sort: SortMode::Name,
            })
            .expect("query")
    }

    /// Addresses of the live hosts, sorted.
    pub fn addresses(&self) -> Vec<String> {
        let mut list: Vec<String> = self
            .views(&[RecordType::Host])
            .iter()
            .filter_map(|view| match view {
                RecordView::Host(host) => host
                    .data
                    .get("address")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                _ => None,
            })
            .collect();
        list.sort();
        list
    }

    pub fn data(&self, record_id: Uuid) -> Value {
        let view = self
            .core
            .record_get(shellpp_core::service::dto::RecordRef {
                vault_id: self.vault_id(),
                record_id,
            })
            .expect("record");
        match view {
            RecordView::Host(host) => host.data,
            RecordView::Group(group) => group.data,
        }
    }

    pub fn register(&self, server: &Server, email: &str, password: &SecretString) -> SyncStatus {
        self.core
            .sync_register(SignInInput {
                origin: server.origin.clone(),
                email: email.to_owned(),
                password: password.clone(),
                registration_code: None,
            })
            .expect("registered")
    }

    pub fn sign_in(&self, server: &Server, email: &str, password: &SecretString) -> SignInOutcome {
        self.core
            .sync_sign_in(SignInInput {
                origin: server.origin.clone(),
                email: email.to_owned(),
                password: password.clone(),
                registration_code: None,
            })
            .expect("signed in")
    }

    /// Signs in after the password changed on another device, as the user
    /// would: the device says so (SP 4.2, `kdf_changed`) before it derives
    /// anything, and the user types `password`, the current one, again.
    pub fn sign_in_after_change(
        &self,
        server: &Server,
        email: &str,
        password: &SecretString,
    ) -> SignInOutcome {
        let input = || SignInInput {
            origin: server.origin.clone(),
            email: email.to_owned(),
            password: password.clone(),
            registration_code: None,
        };
        match self.core.sync_sign_in(input()) {
            Err(shellpp_core::CoreError::Sync(shellpp_core::sync::HttpError::Refused(
                "kdf_changed",
            ))) => {}
            other => panic!("expected the changed settings, got {other:?}"),
        }
        self.core
            .sync_sign_in_after_kdf_change(input(), None)
            .expect("signed in with the current password")
    }

    pub fn sync(&self) -> shellpp_core::service::SyncReport {
        self.core.sync_now().expect("sync")
    }

    pub fn status(&self) -> SyncStatus {
        self.core.sync_status().expect("status")
    }
}

pub fn expose(secret: &SecretString) -> &str {
    secret.expose_secret()
}

// ---------------------------------------------------------------------------
// Raw HTTP
// ---------------------------------------------------------------------------

/// The answer of [`raw_http`]: the status, the body (`Value::Null` when it
/// is not JSON) and the seconds of a `Retry-After` header.
pub struct RawAnswer {
    pub status: u16,
    pub body: Value,
    pub retry_after: Option<u64>,
}

impl RawAnswer {
    /// `error.code` of a protocol error body (docs/sync-protocol.md, section 8).
    pub fn code(&self) -> Option<&str> {
        self.body["error"]["code"].as_str()
    }
}

/// One HTTP/1.1 exchange with the server, for requests the client's own
/// HTTP layer never sends (malformed paths and bodies, extra headers).
pub fn raw_http(
    server: &Server,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> RawAnswer {
    raw_http_at(
        server.origin.trim_start_matches("http://"),
        method,
        path,
        headers,
        body,
    )
    .expect("an HTTP answer from the test server")
}

/// [`raw_http`] to `addr` (`host:port`), for a server in a container;
/// `None` when nothing answers there.
pub fn raw_http_at(
    addr: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Option<RawAnswer> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(addr).ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .expect("timeout");
    let mut head = format!("{method} {path} HTTP/1.1\r\nhost: {addr}\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    ));
    stream.write_all(head.as_bytes()).ok()?;
    stream.write_all(body).ok()?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).ok()?;
    let text = String::from_utf8_lossy(&answer);
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())?;
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_ref(), ""));
    let retry_after = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("retry-after")
            .then(|| value.trim().parse().ok())?
    });
    let body = serde_json::from_str(body).unwrap_or(Value::Null);
    Some(RawAnswer {
        status,
        body,
        retry_after,
    })
}
