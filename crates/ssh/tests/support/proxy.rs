//! Small proxies for the route tests, on loopback: an HTTP CONNECT proxy
//! with optional Basic authentication and a SOCKS5 proxy with optional
//! username/password authentication (RFC 1928, RFC 1929). Each records the
//! `host:port` it was asked to reach and connects only to loopback
//! destinations (`localhost` or a loopback address), so a test never
//! reaches out of the machine.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What both proxies share.
struct ProxyCore {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ProxyCore {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Connects to a loopback destination only.
async fn connect_loopback(host: &str, port: u16) -> Option<TcpStream> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let ip: std::net::IpAddr = if host.eq_ignore_ascii_case("localhost") {
        std::net::Ipv4Addr::LOCALHOST.into()
    } else {
        host.parse()
            .ok()
            .filter(|ip: &std::net::IpAddr| ip.is_loopback())?
    };
    TcpStream::connect((ip, port)).await.ok()
}

async fn relay(mut client: TcpStream, mut upstream: TcpStream) {
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

type Credentials = Option<(String, String)>;

/// An HTTP CONNECT proxy.
pub struct HttpProxy(ProxyCore);

impl HttpProxy {
    pub async fn start(credentials: Option<(&str, &str)>) -> HttpProxy {
        let credentials: Credentials =
            credentials.map(|(user, password)| (user.to_owned(), password.to_owned()));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let seen = Arc::clone(&seen);
                let credentials = credentials.clone();
                tokio::spawn(http_client(client, seen, credentials));
            }
        });
        HttpProxy(ProxyCore {
            addr,
            requests,
            task,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.0.addr
    }

    pub fn requests(&self) -> Vec<String> {
        self.0.requests.lock().unwrap().clone()
    }
}

async fn http_client(
    mut client: TcpStream,
    seen: Arc<Mutex<Vec<String>>>,
    credentials: Credentials,
) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 || client.read_exact(&mut byte).await.is_err() {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let request = lines.next().unwrap_or_default();
    let mut parts = request.split(' ');
    let (Some("CONNECT"), Some(target)) = (parts.next(), parts.next()) else {
        let _ = client
            .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
            .await;
        return;
    };
    if let Some((user, password)) = &credentials {
        let expected = format!(
            "Proxy-Authorization: Basic {}",
            data_encoding_base64(format!("{user}:{password}").as_bytes())
        );
        if !lines.any(|line| line == expected) {
            let _ = client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await;
            return;
        }
    }
    seen.lock().unwrap().push(target.to_owned());
    let Some((host, port)) = target.rsplit_once(':') else {
        let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    };
    let Some(upstream) = (match port.parse() {
        Ok(port) => connect_loopback(host, port).await,
        Err(_) => None,
    }) else {
        let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
        return;
    };
    if client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await
        .is_ok()
    {
        relay(client, upstream).await;
    }
}

/// Standard base64 with padding (the test crate has no base64 dependency).
fn data_encoding_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A SOCKS5 proxy.
pub struct SocksProxy(ProxyCore);

impl SocksProxy {
    pub async fn start(credentials: Option<(&str, &str)>) -> SocksProxy {
        let credentials: Credentials =
            credentials.map(|(user, password)| (user.to_owned(), password.to_owned()));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let seen = Arc::clone(&seen);
                let credentials = credentials.clone();
                tokio::spawn(async move {
                    let _ = socks_client(client, seen, credentials).await;
                });
            }
        });
        SocksProxy(ProxyCore {
            addr,
            requests,
            task,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.0.addr
    }

    pub fn requests(&self) -> Vec<String> {
        self.0.requests.lock().unwrap().clone()
    }
}

async fn socks_client(
    mut client: TcpStream,
    seen: Arc<Mutex<Vec<String>>>,
    credentials: Credentials,
) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    client.read_exact(&mut head).await?;
    let mut methods = vec![0u8; usize::from(head[1])];
    client.read_exact(&mut methods).await?;
    match &credentials {
        None if methods.contains(&0) => client.write_all(&[5, 0]).await?,
        Some((user, password)) if methods.contains(&2) => {
            client.write_all(&[5, 2]).await?;
            let mut version = [0u8; 2];
            client.read_exact(&mut version).await?;
            let mut name = vec![0u8; usize::from(version[1])];
            client.read_exact(&mut name).await?;
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut pass = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut pass).await?;
            if name != user.as_bytes() || pass != password.as_bytes() {
                client.write_all(&[1, 1]).await?;
                return Ok(());
            }
            client.write_all(&[1, 0]).await?;
        }
        _ => {
            client.write_all(&[5, 0xFF]).await?;
            return Ok(());
        }
    }
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    let host = match request[3] {
        1 => {
            let mut octets = [0u8; 4];
            client.read_exact(&mut octets).await?;
            std::net::Ipv4Addr::from(octets).to_string()
        }
        4 => {
            let mut octets = [0u8; 16];
            client.read_exact(&mut octets).await?;
            std::net::Ipv6Addr::from(octets).to_string()
        }
        _ => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    seen.lock().unwrap().push(format!("{host}:{port}"));
    match connect_loopback(&host, port).await {
        Some(upstream) => {
            client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            relay(client, upstream).await;
        }
        // Connection refused.
        None => client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?,
    }
    Ok(())
}
