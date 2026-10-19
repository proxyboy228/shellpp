//! Network helpers: a TCP relay whose links can be frozen (no data in either
//! direction, connections stay open), a listener that never speaks SSH, a
//! port nobody listens on, and a fake server that captures the client's
//! KEXINIT name lists.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A relay from a local port to `target`.
pub struct FreezeProxy {
    addr: SocketAddr,
    frozen: Arc<AtomicBool>,
    /// Bytes relayed from the client to the server.
    upstream_bytes: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FreezeProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FreezeProxy {
    pub async fn start(target: SocketAddr) -> FreezeProxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
        let addr = listener.local_addr().expect("relay address");
        let frozen = Arc::new(AtomicBool::new(false));
        let upstream_bytes = Arc::new(AtomicU64::new(0));
        let task = {
            let frozen = frozen.clone();
            let upstream_bytes = upstream_bytes.clone();
            tokio::spawn(async move {
                while let Ok((client, _)) = listener.accept().await {
                    let Ok(server) = TcpStream::connect(target).await else {
                        continue;
                    };
                    let (client_read, client_write) = client.into_split();
                    let (server_read, server_write) = server.into_split();
                    tokio::spawn(pump(
                        client_read,
                        server_write,
                        frozen.clone(),
                        Some(upstream_bytes.clone()),
                    ));
                    tokio::spawn(pump(server_read, client_write, frozen.clone(), None));
                }
            })
        };
        FreezeProxy {
            addr,
            frozen,
            upstream_bytes,
            task,
        }
    }

    /// Bytes relayed from the client to the server so far.
    pub fn upstream_bytes(&self) -> u64 {
        self.upstream_bytes.load(Ordering::SeqCst)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// From now on nothing is relayed; the connections stay open.
    pub fn freeze(&self) {
        self.frozen.store(true, Ordering::SeqCst);
    }
}

async fn pump<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut from: R,
    mut to: W,
    frozen: Arc<AtomicBool>,
    counter: Option<Arc<AtomicU64>>,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if frozen.load(Ordering::SeqCst) {
            // Hold both sockets open without relaying anything.
            std::future::pending::<()>().await;
        }
        if let Some(counter) = &counter {
            counter.fetch_add(n as u64, Ordering::SeqCst);
        }
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

/// A listener that accepts connections and never sends anything.
pub struct SilentServer {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for SilentServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SilentServer {
    pub async fn start() -> SilentServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        SilentServer { addr, task }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// A loopback port with no listener (bound once, then released).
pub async fn closed_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    drop(listener);
    addr
}

/// The name lists of a KEXINIT message (RFC 4253, section 7.1).
#[derive(Debug, Default)]
pub struct KexInit {
    pub kex: Vec<String>,
    pub host_key: Vec<String>,
    pub cipher_c2s: Vec<String>,
    pub cipher_s2c: Vec<String>,
    pub mac_c2s: Vec<String>,
    pub mac_s2c: Vec<String>,
    pub compression_c2s: Vec<String>,
    pub compression_s2c: Vec<String>,
}

impl KexInit {
    pub fn all_names(&self) -> Vec<String> {
        [
            &self.kex,
            &self.host_key,
            &self.cipher_c2s,
            &self.cipher_s2c,
            &self.mac_c2s,
            &self.mac_s2c,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
    }
}

/// A fake server: sends an identification line, reads the client's
/// identification and its first binary packet (KEXINIT, unencrypted), and
/// returns the parsed name lists. The connection is then closed.
pub struct KexInitCapture {
    addr: SocketAddr,
    result: tokio::sync::oneshot::Receiver<Result<KexInit, String>>,
}

impl KexInitCapture {
    pub async fn start() -> KexInitCapture {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("address");
        let (sender, result) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let outcome = match listener.accept().await {
                Ok((stream, _)) => capture(stream).await,
                Err(error) => Err(error.to_string()),
            };
            let _ = sender.send(outcome);
        });
        KexInitCapture { addr, result }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn kexinit(self) -> KexInit {
        tokio::time::timeout(Duration::from_secs(10), self.result)
            .await
            .expect("client sent KEXINIT in time")
            .expect("capture task")
            .expect("valid KEXINIT")
    }
}

async fn capture(mut stream: TcpStream) -> Result<KexInit, String> {
    stream
        .write_all(b"SSH-2.0-shellpp_kexinit_capture\r\n")
        .await
        .map_err(|e| e.to_string())?;
    // The client's identification line.
    let mut line = Vec::new();
    loop {
        let byte = stream.read_u8().await.map_err(|e| e.to_string())?;
        line.push(byte);
        if byte == b'\n' {
            break;
        }
        if line.len() > 255 {
            return Err("identification line too long".to_owned());
        }
    }
    if !line.starts_with(b"SSH-2.0-") {
        return Err("not an SSH client".to_owned());
    }
    let packet_len = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
    if packet_len > 35_000 {
        return Err("packet too large".to_owned());
    }
    let mut packet = vec![0u8; packet_len];
    stream
        .read_exact(&mut packet)
        .await
        .map_err(|e| e.to_string())?;
    let padding = *packet.first().ok_or("empty packet")? as usize;
    let payload = packet
        .get(1..packet_len.saturating_sub(padding))
        .ok_or("bad padding")?;
    if payload.first() != Some(&20) {
        return Err("first packet is not KEXINIT".to_owned());
    }
    let mut rest = payload.get(17..).ok_or("short KEXINIT")?;
    let mut lists = Vec::new();
    for _ in 0..8 {
        if rest.len() < 4 {
            return Err("truncated name list".to_owned());
        }
        let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let text = rest.get(4..4 + len).ok_or("truncated name list")?;
        let names: Vec<String> = String::from_utf8_lossy(text)
            .split(',')
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .collect();
        lists.push(names);
        rest = &rest[4 + len..];
    }
    let mut lists = lists.into_iter();
    let mut next = || lists.next().unwrap_or_default();
    Ok(KexInit {
        kex: next(),
        host_key: next(),
        cipher_c2s: next(),
        cipher_s2c: next(),
        mac_c2s: next(),
        mac_s2c: next(),
        compression_c2s: next(),
        compression_s2c: next(),
    })
}
