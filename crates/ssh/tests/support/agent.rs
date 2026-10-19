//! A minimal SSH agent for tests (draft-miller-ssh-agent): it lists its
//! keys and signs with them (Ed25519 and ECDSA keys; RSA is not needed by
//! the tests), and refuses every other request. It listens on a Unix socket
//! in a temporary directory, or on Windows on a uniquely named pipe.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use russh::keys::PrivateKey;
use russh::keys::signature::Signer;
use russh::keys::ssh_encoding::Encode;
use russh::keys::ssh_key::Signature;
use shellpp_ssh::AgentLocation;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const FAILURE: u8 = 5;
const REQUEST_IDENTITIES: u8 = 11;
const IDENTITIES_ANSWER: u8 = 12;
const SIGN_REQUEST: u8 = 13;
const SIGN_RESPONSE: u8 = 14;

/// A running test agent; stops when dropped.
pub struct TestAgent {
    location: AgentLocation,
    requests: Arc<AtomicUsize>,
    signatures: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
    _dir: Option<tempfile::TempDir>,
}

impl Drop for TestAgent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestAgent {
    pub fn location(&self) -> AgentLocation {
        self.location.clone()
    }

    /// Requests served so far.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// Signatures made so far.
    pub fn signatures(&self) -> usize {
        self.signatures.load(Ordering::SeqCst)
    }

    #[cfg(unix)]
    pub async fn start(keys: Vec<PrivateKey>) -> TestAgent {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&path).expect("bind agent socket");
        let keys = Arc::new(keys);
        let requests = Arc::new(AtomicUsize::new(0));
        let signatures = Arc::new(AtomicUsize::new(0));
        let task = {
            let (keys, requests, signatures) = (keys.clone(), requests.clone(), signatures.clone());
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(serve(
                        stream,
                        keys.clone(),
                        requests.clone(),
                        signatures.clone(),
                    ));
                }
            })
        };
        TestAgent {
            location: AgentLocation::Path(path),
            requests,
            signatures,
            task,
            _dir: Some(dir),
        }
    }

    #[cfg(windows)]
    pub async fn start(keys: Vec<PrivateKey>) -> TestAgent {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = format!(
            r"\\.\pipe\shellpp-test-agent-{:016x}",
            rand::random::<u64>()
        );
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .expect("create agent pipe");
        let keys = Arc::new(keys);
        let requests = Arc::new(AtomicUsize::new(0));
        let signatures = Arc::new(AtomicUsize::new(0));
        let task = {
            let (keys, requests, signatures) = (keys.clone(), requests.clone(), signatures.clone());
            let name = name.clone();
            tokio::spawn(async move {
                loop {
                    if server.connect().await.is_err() {
                        return;
                    }
                    let connected = server;
                    server = match ServerOptions::new().create(&name) {
                        Ok(next) => next,
                        Err(_) => return,
                    };
                    tokio::spawn(serve(
                        connected,
                        keys.clone(),
                        requests.clone(),
                        signatures.clone(),
                    ));
                }
            })
        };
        TestAgent {
            location: AgentLocation::Path(std::path::PathBuf::from(name)),
            requests,
            signatures,
            task,
            _dir: None,
        }
    }
}

fn put_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_be_bytes());
}

fn put_string(buf: &mut Vec<u8>, value: &[u8]) {
    put_u32(buf, value.len() as u32);
    buf.extend_from_slice(value);
}

fn take_string<'a>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
    if input.len() < 4 {
        return None;
    }
    let len = u32::from_be_bytes([input[0], input[1], input[2], input[3]]) as usize;
    let rest = &input[4..];
    if rest.len() < len {
        return None;
    }
    let (value, tail) = rest.split_at(len);
    *input = tail;
    Some(value)
}

fn answer(keys: &[PrivateKey], request: &[u8], signatures: &AtomicUsize) -> Vec<u8> {
    let mut body = Vec::new();
    match request.first() {
        Some(&REQUEST_IDENTITIES) => {
            body.push(IDENTITIES_ANSWER);
            put_u32(&mut body, keys.len() as u32);
            for key in keys {
                put_string(&mut body, &key.public_key().to_bytes().expect("key blob"));
                put_string(&mut body, b"shellpp-test");
            }
        }
        Some(&SIGN_REQUEST) => {
            let mut rest = &request[1..];
            let signed = (|| {
                let blob = take_string(&mut rest)?;
                let data = take_string(&mut rest)?;
                let key = keys
                    .iter()
                    .find(|key| key.public_key().to_bytes().ok().as_deref() == Some(blob))?;
                let signature: Signature = key.try_sign(data).ok()?;
                let mut encoded = Vec::new();
                signature.encode(&mut encoded).ok()?;
                Some(encoded)
            })();
            match signed {
                Some(encoded) => {
                    signatures.fetch_add(1, Ordering::SeqCst);
                    body.push(SIGN_RESPONSE);
                    put_string(&mut body, &encoded);
                }
                None => body.push(FAILURE),
            }
        }
        _ => body.push(FAILURE),
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    put_string(&mut frame, &body);
    frame
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    keys: Arc<Vec<PrivateKey>>,
    requests: Arc<AtomicUsize>,
    signatures: Arc<AtomicUsize>,
) {
    loop {
        let mut len = [0u8; 4];
        if stream.read_exact(&mut len).await.is_err() {
            return;
        }
        let len = u32::from_be_bytes(len) as usize;
        if len == 0 || len > 256 * 1024 {
            return;
        }
        let mut request = vec![0u8; len];
        if stream.read_exact(&mut request).await.is_err() {
            return;
        }
        requests.fetch_add(1, Ordering::SeqCst);
        let reply = answer(&keys, &request, &signatures);
        if stream.write_all(&reply).await.is_err() || stream.flush().await.is_err() {
            return;
        }
    }
}
