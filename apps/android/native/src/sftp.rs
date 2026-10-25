//! SFTP on the phone (UI spec 10, architecture 6.4): a session through the
//! shared session layer (`SessionManager::open_sftp`: the route, prompts and
//! host key questions of an SSH session), the requests of the remote panel
//! as JSON commands, and the transfer queue: one transfer at a time per
//! session, the others queued, each with its progress and end as events
//! (`{"kind": "transfer", ...}`) and a cancel.
//!
//! Transfers go between the server and a file in the app's cache, through
//! the SFTP code the desktop uses (its `.part` files and size checks); the
//! app copies that file to or from the document the Storage Access
//! Framework gave [decision, run 14]. No file content crosses JNI.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use shellpp_sessions::SessionHandle;
use shellpp_ssh::{EntryKind, RemoteEntry, Sftp, SftpError};
use tokio::sync::Semaphore;

use crate::events::EventQueue;

/// How often a running transfer reports its progress.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

struct Session {
    handle: SessionHandle,
    /// One transfer at a time on the session's channel.
    slot: Arc<Semaphore>,
}

/// What a transfer copies.
enum Job {
    Download { remote: String, local: PathBuf },
    Upload { local: PathBuf, remote: String },
}

struct Transfer {
    session: u64,
    task: tokio::task::JoinHandle<()>,
}

/// The SFTP sessions of the engine and their transfers.
pub struct SftpSessions {
    sessions: Mutex<BTreeMap<u64, Session>>,
    transfers: Arc<Mutex<BTreeMap<u64, Transfer>>>,
    next_transfer: AtomicU64,
    events: Arc<EventQueue>,
}

impl std::fmt::Debug for SftpSessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpSessions").finish_non_exhaustive()
    }
}

/// Why an SFTP command failed: a code for the app and the text to show.
#[derive(Debug)]
pub struct SftpFailure {
    pub code: &'static str,
    pub message: String,
}

impl From<SftpError> for SftpFailure {
    fn from(error: SftpError) -> Self {
        let code = match &error {
            SftpError::NotFound => "not_found",
            SftpError::PermissionDenied => "permission_denied",
            SftpError::Failure(_) => "failure",
            SftpError::ConnectionLost => "connection_lost",
            SftpError::Timeout => "timeout",
            SftpError::Local(_) => "local",
            SftpError::NotAFile => "not_a_file",
            SftpError::Changed => "changed",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

fn failure(code: &'static str, message: &str) -> SftpFailure {
    SftpFailure {
        code,
        message: message.to_owned(),
    }
}

#[derive(Deserialize)]
struct PathInput {
    path: String,
}

#[derive(Deserialize)]
struct RenameInput {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct RemoveInput {
    path: String,
    #[serde(default)]
    dir: bool,
}

fn parse<I: DeserializeOwned>(input: &[u8]) -> Result<I, SftpFailure> {
    serde_json::from_slice(input).map_err(|_| failure("bad_input", "bad input"))
}

/// A listed entry as the app shows it: folders first, then by name.
fn entry_json(entry: &RemoteEntry) -> Value {
    let (kind, to_dir) = match &entry.kind {
        EntryKind::File => ("file", false),
        EntryKind::Dir => ("dir", true),
        EntryKind::Symlink { to_dir, .. } => ("link", *to_dir),
        EntryKind::Other => ("other", false),
    };
    json!({
        "name": entry.name,
        "kind": kind,
        "dir": to_dir,
        "size": entry.size,
        "modified": entry.modified,
    })
}

impl SftpSessions {
    pub fn new(events: Arc<EventQueue>) -> Self {
        Self {
            sessions: Mutex::new(BTreeMap::new()),
            transfers: Arc::new(Mutex::new(BTreeMap::new())),
            next_transfer: AtomicU64::new(1),
            events,
        }
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Session>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn add(&self, id: u64, handle: SessionHandle) {
        self.sessions().insert(
            id,
            Session {
                handle,
                slot: Arc::new(Semaphore::new(1)),
            },
        );
    }

    pub fn contains(&self, id: u64) -> bool {
        self.sessions().contains_key(&id)
    }

    /// The session's handle (its prompts are answered through it).
    pub fn handle(&self, id: u64) -> Option<SessionHandle> {
        self.sessions()
            .get(&id)
            .map(|session| session.handle.clone())
    }

    pub fn ids(&self) -> Vec<u64> {
        self.sessions().keys().copied().collect()
    }

    /// Closes session `id`; its transfers are cancelled, each with its
    /// event, so the app cleans up after them.
    pub fn close(&self, id: u64) {
        let removed = self.sessions().remove(&id);
        let mut cancelled = Vec::new();
        let mut transfers = self
            .transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        transfers.retain(|transfer_id, transfer| {
            if transfer.session == id {
                transfer.task.abort();
                cancelled.push(*transfer_id);
                false
            } else {
                true
            }
        });
        drop(transfers);
        for transfer in cancelled {
            self.events.push(&json!({
                "kind": "transfer",
                "id": transfer,
                "session": id,
                "state": "cancelled",
            }));
        }
        if let Some(session) = removed {
            session.handle.close();
        }
    }

    /// The session's handle and its SFTP channel (once connected).
    fn channel(&self, id: u64) -> Result<(SessionHandle, Arc<Sftp>, Arc<Semaphore>), SftpFailure> {
        let sessions = self.sessions();
        let session = sessions
            .get(&id)
            .ok_or_else(|| failure("no_session", "no such session"))?;
        let sftp = session
            .handle
            .sftp()
            .ok_or_else(|| failure("not_connected", "not connected yet"))?;
        Ok((session.handle.clone(), sftp, session.slot.clone()))
    }

    /// Runs `work` on the session layer's runtime and waits for it (the
    /// caller is a JNI worker thread, never the main thread).
    fn wait<T: Send + 'static>(
        handle: &SessionHandle,
        work: impl std::future::Future<Output = Result<T, SftpError>> + Send + 'static,
    ) -> Result<T, SftpFailure> {
        match futures::executor::block_on(handle.spawn(work)) {
            Ok(result) => result.map_err(SftpFailure::from),
            Err(_) => Err(failure("connection_lost", "the request was stopped")),
        }
    }

    /// One request of the remote panel: `home`, `list` (`path`), `exists`
    /// (`path`), `mkdir` (`path`), `rename` (`from`, `to`), `remove`
    /// (`path`, `dir`). Answers `{"ok": ...}` or `{"error": {"code",
    /// "message"}}`.
    pub fn call(&self, id: u64, command: &str, input: &[u8]) -> String {
        let answer = match self.run(id, command, input) {
            Ok(value) => json!({ "ok": value }),
            Err(error) => {
                tracing::debug!(command, code = error.code, "sftp command failed");
                json!({ "error": { "code": error.code, "message": error.message } })
            }
        };
        answer.to_string()
    }

    fn run(&self, id: u64, command: &str, input: &[u8]) -> Result<Value, SftpFailure> {
        let (handle, sftp, _) = self.channel(id)?;
        match command {
            "home" => Ok(json!(Self::wait(&handle, async move {
                sftp.canonicalize(".").await
            })?)),
            "list" => {
                let input: PathInput = parse(input)?;
                let mut entries = Self::wait(&handle, async move { sftp.list(&input.path).await })?;
                entries.retain(|entry| entry.name != "." && entry.name != "..");
                let mut listed: Vec<Value> = entries.iter().map(entry_json).collect();
                listed.sort_by(|a, b| {
                    let dir = |value: &Value| !value["dir"].as_bool().unwrap_or(false);
                    let name = |value: &Value| value["name"].as_str().unwrap_or("").to_lowercase();
                    (dir(a), name(a)).cmp(&(dir(b), name(b)))
                });
                Ok(Value::Array(listed))
            }
            "exists" => {
                let input: PathInput = parse(input)?;
                let found = Self::wait(&handle, async move {
                    match sftp.stat(&input.path).await {
                        Ok(_) => Ok(true),
                        Err(SftpError::NotFound) => Ok(false),
                        Err(error) => Err(error),
                    }
                })?;
                Ok(json!(found))
            }
            "mkdir" => {
                let input: PathInput = parse(input)?;
                Self::wait(&handle, async move { sftp.create_dir(&input.path).await })?;
                Ok(Value::Null)
            }
            "rename" => {
                let input: RenameInput = parse(input)?;
                Self::wait(
                    &handle,
                    async move { sftp.rename(&input.from, &input.to).await },
                )?;
                Ok(Value::Null)
            }
            "remove" => {
                let input: RemoveInput = parse(input)?;
                Self::wait(&handle, async move {
                    if input.dir {
                        sftp.remove_dir(&input.path).await
                    } else {
                        sftp.remove_file(&input.path).await
                    }
                })?;
                Ok(Value::Null)
            }
            _ => Err(failure("unknown_command", "unknown command")),
        }
    }

    /// Queues a download of `remote` into the local file `local` (in the
    /// app's cache); returns the transfer's id.
    pub fn download(&self, id: u64, remote: String, local: PathBuf) -> Result<u64, SftpFailure> {
        self.queue(id, Job::Download { remote, local })
    }

    /// Queues an upload of the local file `local` to `remote`; returns the
    /// transfer's id.
    pub fn upload(&self, id: u64, local: PathBuf, remote: String) -> Result<u64, SftpFailure> {
        self.queue(id, Job::Upload { local, remote })
    }

    fn queue(&self, id: u64, job: Job) -> Result<u64, SftpFailure> {
        let (handle, _, slot) = self.channel(id)?;
        let transfer = self.next_transfer.fetch_add(1, Ordering::Relaxed);
        let events = self.events.clone();
        let (path, direction) = match &job {
            Job::Download { remote, .. } => (remote.clone(), "download"),
            Job::Upload { remote, .. } => (remote.clone(), "upload"),
        };
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let report = move |state: &str, done: u64, error: Option<String>| {
            events.push(&json!({
                "kind": "transfer",
                "id": transfer,
                "session": id,
                "name": name,
                "path": path,
                "direction": direction,
                "state": state,
                "done": done,
                "error": error,
            }));
        };
        report("queued", 0, None);
        let transfers = self.transfers.clone();
        // Held over the spawn: the task's own removal at its end waits until
        // the transfer is in the table.
        let mut table = self
            .transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let session_handle = handle.clone();
        let task = handle.spawn(async move {
            let Ok(_permit) = slot.acquire_owned().await else {
                return;
            };
            // The channel as it is now: a reconnect while this one waited
            // made a new one.
            let Some(sftp) = session_handle.sftp() else {
                report(
                    "failed",
                    0,
                    Some(SftpFailure::from(SftpError::ConnectionLost).message),
                );
                transfers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&transfer);
                return;
            };
            report("running", 0, None);
            let mut done = 0;
            let result = {
                let mut last = Instant::now();
                let mut progress = |bytes: u64| {
                    done = bytes;
                    if last.elapsed() >= PROGRESS_EVERY {
                        last = Instant::now();
                        report("running", bytes, None);
                    }
                };
                match &job {
                    Job::Download { remote, local } => {
                        sftp.download(remote, local, false, &mut progress).await
                    }
                    Job::Upload { local, remote } => {
                        sftp.upload(local, remote, false, &mut progress).await
                    }
                }
            };
            match result {
                Ok(bytes) => report("done", bytes, None),
                Err(error) => report("failed", done, Some(SftpFailure::from(error).message)),
            }
            transfers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&transfer);
        });
        table.insert(transfer, Transfer { session: id, task });
        drop(table);
        Ok(transfer)
    }

    /// Cancels transfer `id` (queued or running); its `.part` stays for no
    /// resume and the app deletes its cache file.
    pub fn cancel(&self, transfer: u64) {
        let removed = self
            .transfers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&transfer);
        if let Some(removed) = removed {
            removed.task.abort();
            self.events.push(&json!({
                "kind": "transfer",
                "id": transfer,
                "session": removed.session,
                "state": "cancelled",
            }));
        }
    }
}
