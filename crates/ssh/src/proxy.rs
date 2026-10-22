//! The proxy of a route's first connection (docs/data-model.md, section 7.5;
//! stage 5): HTTP `CONNECT` with optional Basic authentication, and SOCKS5
//! (RFC 1928) with optional username/password authentication (RFC 1929).
//! The proxy resolves the target's name: SOCKS5 gets it as a domain name,
//! `CONNECT` names it in the request line.
//!
//! The proxy's replies are untrusted: only the status is read, nothing of
//! the reply text is kept or logged, and a reply longer than
//! [`MAX_HTTP_HEADER`] ends the attempt. Bytes the server sends after the
//! proxy's reply (the SSH identification line) stay in the stream: the HTTP
//! reply is read byte by byte up to its blank line, the SOCKS5 reply has a
//! known length.

use secrecy::{ExposeSecret, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::FailureKind;

/// The kind of proxy (`host.ssh.proxy.type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    /// HTTP `CONNECT`.
    Http,
    Socks5,
}

/// A proxy for the first connection of a route.
#[derive(Clone)]
pub struct ProxySpec {
    pub kind: ProxyKind,
    /// Host name or IP address of the proxy (IPv6 without brackets).
    pub address: String,
    pub port: u16,
    /// Basic authentication (HTTP) or username/password (SOCKS5); the
    /// password is used only with a username.
    pub username: Option<String>,
    pub password: Option<SecretString>,
}

impl std::fmt::Debug for ProxySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxySpec")
            .field("kind", &self.kind)
            .field("username", &self.username.is_some())
            .field("password", &self.password.is_some())
            .finish_non_exhaustive()
    }
}

/// Why the proxy did not open the tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProxyError {
    /// The proxy failed or refused: the failure is the proxy's.
    Proxy(FailureKind),
    /// The proxy reached for the target and reported that it failed
    /// (refused, unreachable): the failure is the target's.
    Target(FailureKind),
}

/// Most bytes of an HTTP `CONNECT` reply (status line and headers).
const MAX_HTTP_HEADER: usize = 8 * 1024;

/// Asks the proxy on `stream` for a tunnel to `host:port`.
pub(crate) async fn open<S>(
    stream: &mut S,
    proxy: &ProxySpec,
    host: &str,
    port: u16,
) -> Result<(), ProxyError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match proxy.kind {
        ProxyKind::Http => http_connect(stream, proxy, host, port).await,
        ProxyKind::Socks5 => socks5_connect(stream, proxy, host, port).await,
    }
}

/// `host:port` as the authority of a request line: an IPv6 literal in
/// brackets.
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn io_error(_: std::io::Error) -> ProxyError {
    ProxyError::Proxy(FailureKind::Network)
}

async fn http_connect<S>(
    stream: &mut S,
    proxy: &ProxySpec,
    host: &str,
    port: u16,
) -> Result<(), ProxyError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // A name with line breaks or spaces would split the request line.
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ProxyError::Target(FailureKind::NotFound));
    }
    let target = authority(host, port);
    // Every buffer that holds the credentials is wiped and allocated once
    // at its final size (no plain copy, no growth; docs/security.md 9.5).
    let encoded = proxy.username.as_deref().map(|username| {
        let password = proxy
            .password
            .as_ref()
            .map(|password| password.expose_secret())
            .unwrap_or_default();
        let mut credentials =
            zeroize::Zeroizing::new(String::with_capacity(username.len() + 1 + password.len()));
        credentials.push_str(username);
        credentials.push(':');
        credentials.push_str(password);
        zeroize::Zeroizing::new(data_encoding::BASE64.encode(credentials.as_bytes()))
    });
    let head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    const AUTH: &str = "Proxy-Authorization: Basic ";
    let extra = encoded
        .as_ref()
        .map_or(0, |encoded| AUTH.len() + encoded.len() + 2);
    let mut request = zeroize::Zeroizing::new(String::with_capacity(head.len() + extra + 2));
    request.push_str(&head);
    if let Some(encoded) = &encoded {
        request.push_str(AUTH);
        request.push_str(encoded);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(io_error)?;
    stream.flush().await.map_err(io_error)?;

    // The reply up to its blank line, one byte at a time: what follows is
    // the SSH server's and must stay in the stream.
    let mut reply = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !reply.ends_with(b"\r\n\r\n") && !reply.ends_with(b"\n\n") {
        if reply.len() >= MAX_HTTP_HEADER {
            return Err(ProxyError::Proxy(FailureKind::ProxyFailed));
        }
        match stream.read(&mut byte).await {
            Ok(0) => return Err(ProxyError::Proxy(FailureKind::ProxyFailed)),
            Ok(_) => reply.push(byte[0]),
            Err(error) => return Err(io_error(error)),
        }
    }
    match http_status(&reply) {
        Some(200..=299) => Ok(()),
        // 407 is the standard answer; tinyproxy answers wrong credentials
        // with 401 (and missing ones with 407).
        Some(401 | 407) => Err(ProxyError::Proxy(FailureKind::ProxyAuthFailed)),
        // Bad gateway, gateway timeout: the proxy could not reach the target.
        Some(502) => Err(ProxyError::Target(FailureKind::Refused)),
        Some(504) => Err(ProxyError::Target(FailureKind::Timeout)),
        _ => Err(ProxyError::Proxy(FailureKind::ProxyFailed)),
    }
}

/// The status code of an HTTP reply's status line (`HTTP/1.x NNN ...`).
fn http_status(reply: &[u8]) -> Option<u16> {
    let line = reply.split(|&b| b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let mut parts = line.split(' ');
    let version = parts.next()?;
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let code = parts.next()?;
    if code.len() != 3 {
        return None;
    }
    code.parse().ok()
}

// SOCKS5 constants (RFC 1928, RFC 1929).
const SOCKS_VERSION: u8 = 5;
const METHOD_NONE: u8 = 0x00;
const METHOD_PASSWORD: u8 = 0x02;
const METHOD_REFUSED: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const AUTH_VERSION: u8 = 0x01;

async fn socks5_connect<S>(
    stream: &mut S,
    proxy: &ProxySpec,
    host: &str,
    port: u16,
) -> Result<(), ProxyError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let credentials = proxy.username.as_deref().map(|username| {
        let password = proxy
            .password
            .as_ref()
            .map(|password| zeroize::Zeroizing::new(password.expose_secret().as_bytes().to_vec()))
            .unwrap_or_default();
        (username.as_bytes(), password)
    });
    if let Some((username, password)) = &credentials
        && (username.is_empty() || username.len() > 255 || password.len() > 255)
    {
        return Err(ProxyError::Proxy(FailureKind::ProxyAuthFailed));
    }
    let greeting: &[u8] = if credentials.is_some() {
        &[SOCKS_VERSION, 2, METHOD_NONE, METHOD_PASSWORD]
    } else {
        &[SOCKS_VERSION, 1, METHOD_NONE]
    };
    stream.write_all(greeting).await.map_err(io_error)?;
    stream.flush().await.map_err(io_error)?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await.map_err(io_error)?;
    if choice[0] != SOCKS_VERSION {
        return Err(ProxyError::Proxy(FailureKind::ProxyFailed));
    }
    match (choice[1], &credentials) {
        (METHOD_NONE, _) => {}
        (METHOD_PASSWORD, Some((username, password))) => {
            let mut auth =
                zeroize::Zeroizing::new(Vec::with_capacity(3 + username.len() + password.len()));
            auth.push(AUTH_VERSION);
            auth.push(username.len() as u8);
            auth.extend_from_slice(username);
            auth.push(password.len() as u8);
            auth.extend_from_slice(password);
            stream.write_all(&auth).await.map_err(io_error)?;
            stream.flush().await.map_err(io_error)?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await.map_err(io_error)?;
            if status[1] != 0 {
                return Err(ProxyError::Proxy(FailureKind::ProxyAuthFailed));
            }
        }
        // No acceptable method: the proxy wants credentials we lack (or
        // refuses ours).
        (METHOD_REFUSED, _) | (METHOD_PASSWORD, None) => {
            return Err(ProxyError::Proxy(FailureKind::ProxyAuthFailed));
        }
        _ => return Err(ProxyError::Proxy(FailureKind::ProxyFailed)),
    }

    let mut request = vec![SOCKS_VERSION, CMD_CONNECT, 0];
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            request.push(ATYP_IPV4);
            request.extend_from_slice(&ip.octets());
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            request.push(ATYP_IPV6);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            // The proxy resolves the name.
            if host.is_empty() || host.len() > 255 {
                return Err(ProxyError::Target(FailureKind::NotFound));
            }
            request.push(ATYP_DOMAIN);
            request.push(host.len() as u8);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await.map_err(io_error)?;
    stream.flush().await.map_err(io_error)?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(io_error)?;
    if head[0] != SOCKS_VERSION {
        return Err(ProxyError::Proxy(FailureKind::ProxyFailed));
    }
    // The bound address that ends the reply.
    let address_len = match head[3] {
        ATYP_IPV4 => 4,
        ATYP_IPV6 => 16,
        ATYP_DOMAIN => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await.map_err(io_error)?;
            usize::from(len[0])
        }
        _ => return Err(ProxyError::Proxy(FailureKind::ProxyFailed)),
    };
    let mut rest = vec![0u8; address_len + 2];
    stream.read_exact(&mut rest).await.map_err(io_error)?;
    socks5_reply(head[1])
}

/// The meaning of a SOCKS5 reply code (RFC 1928, section 6).
fn socks5_reply(code: u8) -> Result<(), ProxyError> {
    match code {
        0x00 => Ok(()),
        // Connection not allowed by the ruleset.
        0x02 => Err(ProxyError::Proxy(FailureKind::ProxyFailed)),
        // Network unreachable, host unreachable.
        0x03 | 0x04 => Err(ProxyError::Target(FailureKind::Network)),
        0x05 => Err(ProxyError::Target(FailureKind::Refused)),
        0x06 => Err(ProxyError::Target(FailureKind::Timeout)),
        // General failure, command or address type not supported, unknown.
        _ => Err(ProxyError::Proxy(FailureKind::ProxyFailed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: ProxyKind, username: Option<&str>, password: Option<&str>) -> ProxySpec {
        ProxySpec {
            kind,
            address: "proxy.example".to_owned(),
            port: 3128,
            username: username.map(str::to_owned),
            password: password.map(|password| SecretString::from(password.to_owned())),
        }
    }

    /// Runs `open` against a scripted proxy: `reply` is sent once the
    /// client's request arrived; returns what the client sent and the result.
    async fn http_exchange(
        proxy: ProxySpec,
        host: &str,
        reply: &'static [u8],
    ) -> (String, Result<(), ProxyError>, Vec<u8>) {
        let (mut client, mut server) = tokio::io::duplex(16 * 1024);
        let script = tokio::spawn(async move {
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                server.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            server.write_all(reply).await.unwrap();
            // What the SSH server sends right after.
            server.write_all(b"SSH-2.0-test\r\n").await.unwrap();
            String::from_utf8(request).unwrap()
        });
        let result = open(&mut client, &proxy, host, 22).await;
        let request = script.await.unwrap();
        let mut after = vec![0u8; 14];
        if result.is_ok() {
            client.read_exact(&mut after).await.unwrap();
        }
        (request, result, after)
    }

    #[tokio::test]
    async fn http_connect_names_the_target_and_leaves_the_ssh_bytes() {
        let (request, result, after) = http_exchange(
            spec(ProxyKind::Http, None, None),
            "db.internal",
            b"HTTP/1.1 200 Connection established\r\nVia: x\r\n\r\n",
        )
        .await;
        assert_eq!(result, Ok(()));
        assert!(request.starts_with("CONNECT db.internal:22 HTTP/1.1\r\n"));
        assert!(!request.contains("Proxy-Authorization"));
        assert_eq!(after, b"SSH-2.0-test\r\n");
        let (request, result, _) = http_exchange(
            spec(ProxyKind::Http, None, None),
            "::1",
            b"HTTP/1.0 200 OK\r\n\r\n",
        )
        .await;
        assert_eq!(result, Ok(()));
        assert!(request.starts_with("CONNECT [::1]:22 HTTP/1.1\r\n"));
    }

    #[tokio::test]
    async fn http_basic_authentication_and_statuses() {
        let (request, result, _) = http_exchange(
            spec(ProxyKind::Http, Some("alice"), Some("s3cret")),
            "host",
            b"HTTP/1.1 200 OK\r\n\r\n",
        )
        .await;
        assert_eq!(result, Ok(()));
        // base64("alice:s3cret")
        assert!(request.contains("Proxy-Authorization: Basic YWxpY2U6czNjcmV0\r\n"));
        for (reply, expected) in [
            (
                &b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n"[..],
                ProxyError::Proxy(FailureKind::ProxyAuthFailed),
            ),
            (
                &b"HTTP/1.0 401 Unauthorized\r\n\r\n"[..],
                ProxyError::Proxy(FailureKind::ProxyAuthFailed),
            ),
            (
                &b"HTTP/1.1 403 Forbidden\r\n\r\n"[..],
                ProxyError::Proxy(FailureKind::ProxyFailed),
            ),
            (
                &b"HTTP/1.1 502 Bad Gateway\r\n\r\n"[..],
                ProxyError::Target(FailureKind::Refused),
            ),
            (
                &b"SSH-2.0-not-a-proxy\r\n\r\n"[..],
                ProxyError::Proxy(FailureKind::ProxyFailed),
            ),
        ] {
            let (_, result, _) =
                http_exchange(spec(ProxyKind::Http, None, None), "host", reply).await;
            assert_eq!(result, Err(expected));
        }
    }

    #[tokio::test]
    async fn http_names_that_would_split_the_request_are_refused() {
        let (mut client, _server) = tokio::io::duplex(1024);
        let proxy = spec(ProxyKind::Http, None, None);
        assert_eq!(
            open(&mut client, &proxy, "evil\r\nX: y", 22).await,
            Err(ProxyError::Target(FailureKind::NotFound))
        );
    }

    #[tokio::test]
    async fn an_endless_http_reply_ends_the_attempt() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let _ = server.read(&mut buf).await;
            let _ = server.write_all(&[b'a'; MAX_HTTP_HEADER + 10]).await;
            server
        });
        let proxy = spec(ProxyKind::Http, None, None);
        assert_eq!(
            open(&mut client, &proxy, "host", 22).await,
            Err(ProxyError::Proxy(FailureKind::ProxyFailed))
        );
        drop(task.await.unwrap());
    }

    /// A scripted SOCKS5 proxy: checks the greeting, the optional
    /// authentication and the request, then answers with `code`.
    async fn socks_exchange(
        proxy: ProxySpec,
        host: &'static str,
        method: u8,
        auth_ok: bool,
        code: u8,
    ) -> (Vec<u8>, Result<(), ProxyError>) {
        let (mut client, mut server) = tokio::io::duplex(16 * 1024);
        let script = tokio::spawn(async move {
            let mut seen = Vec::new();
            let mut head = [0u8; 2];
            server.read_exact(&mut head).await.unwrap();
            let mut methods = vec![0u8; usize::from(head[1])];
            server.read_exact(&mut methods).await.unwrap();
            seen.extend_from_slice(&head);
            seen.extend_from_slice(&methods);
            server.write_all(&[5, method]).await.unwrap();
            if method == METHOD_PASSWORD {
                let mut version = [0u8; 2];
                server.read_exact(&mut version).await.unwrap();
                let mut user = vec![0u8; usize::from(version[1])];
                server.read_exact(&mut user).await.unwrap();
                let mut len = [0u8; 1];
                server.read_exact(&mut len).await.unwrap();
                let mut pass = vec![0u8; usize::from(len[0])];
                server.read_exact(&mut pass).await.unwrap();
                seen.extend_from_slice(&user);
                seen.push(b':');
                seen.extend_from_slice(&pass);
                server
                    .write_all(&[1, if auth_ok { 0 } else { 1 }])
                    .await
                    .unwrap();
                if !auth_ok {
                    return seen;
                }
            }
            if method == METHOD_REFUSED {
                return seen;
            }
            let mut request = [0u8; 4];
            server.read_exact(&mut request).await.unwrap();
            seen.extend_from_slice(&request);
            let len = match request[3] {
                ATYP_IPV4 => 4,
                ATYP_IPV6 => 16,
                _ => {
                    let mut len = [0u8; 1];
                    server.read_exact(&mut len).await.unwrap();
                    seen.push(len[0]);
                    usize::from(len[0])
                }
            };
            let mut address = vec![0u8; len + 2];
            server.read_exact(&mut address).await.unwrap();
            seen.extend_from_slice(&address);
            // A domain as the bound address, to check its length is read.
            server
                .write_all(&[5, code, 0, ATYP_DOMAIN, 3, b'a', b'b', b'c', 0, 22])
                .await
                .unwrap();
            server.write_all(b"SSH-").await.unwrap();
            seen
        });
        let result = open(&mut client, &proxy, host, 2222).await;
        let seen = script.await.unwrap();
        if result.is_ok() {
            let mut after = [0u8; 4];
            client.read_exact(&mut after).await.unwrap();
            assert_eq!(&after, b"SSH-", "the reply is read to its end only");
        }
        (seen, result)
    }

    #[tokio::test]
    async fn socks5_sends_names_for_the_proxy_to_resolve() {
        let proxy = spec(ProxyKind::Socks5, None, None);
        let (seen, result) = socks_exchange(proxy, "db.internal", METHOD_NONE, true, 0).await;
        assert_eq!(result, Ok(()));
        let mut expected = vec![5, 1, METHOD_NONE, 5, CMD_CONNECT, 0, ATYP_DOMAIN, 11];
        expected.extend_from_slice(b"db.internal");
        expected.extend_from_slice(&2222u16.to_be_bytes());
        assert_eq!(seen, expected);
        // An IP literal goes as an address.
        let proxy = spec(ProxyKind::Socks5, None, None);
        let (seen, result) = socks_exchange(proxy, "10.1.2.3", METHOD_NONE, true, 0).await;
        assert_eq!(result, Ok(()));
        assert_eq!(&seen[6..11], &[ATYP_IPV4, 10, 1, 2, 3]);
    }

    #[tokio::test]
    async fn socks5_password_authentication() {
        let proxy = spec(ProxyKind::Socks5, Some("bob"), Some("pw"));
        let (seen, result) = socks_exchange(proxy, "host", METHOD_PASSWORD, true, 0).await;
        assert_eq!(result, Ok(()));
        assert_eq!(&seen[..4], &[5, 2, METHOD_NONE, METHOD_PASSWORD]);
        assert_eq!(&seen[4..10], b"bob:pw");
        let proxy = spec(ProxyKind::Socks5, Some("bob"), Some("wrong"));
        let (_, result) = socks_exchange(proxy, "host", METHOD_PASSWORD, false, 0).await;
        assert_eq!(result, Err(ProxyError::Proxy(FailureKind::ProxyAuthFailed)));
        // The proxy accepts none of our methods.
        let proxy = spec(ProxyKind::Socks5, None, None);
        let (_, result) = socks_exchange(proxy, "host", METHOD_REFUSED, true, 0).await;
        assert_eq!(result, Err(ProxyError::Proxy(FailureKind::ProxyAuthFailed)));
    }

    #[tokio::test]
    async fn socks5_reply_codes() {
        for (code, expected) in [
            (2, ProxyError::Proxy(FailureKind::ProxyFailed)),
            (4, ProxyError::Target(FailureKind::Network)),
            (5, ProxyError::Target(FailureKind::Refused)),
            (6, ProxyError::Target(FailureKind::Timeout)),
            (1, ProxyError::Proxy(FailureKind::ProxyFailed)),
        ] {
            let proxy = spec(ProxyKind::Socks5, None, None);
            let (_, result) = socks_exchange(proxy, "host", METHOD_NONE, true, code).await;
            assert_eq!(result, Err(expected), "code {code}");
        }
    }

    #[test]
    fn debug_output_has_no_proxy_data() {
        let proxy = spec(ProxyKind::Http, Some("alice"), Some("hunter2"));
        let text = format!("{proxy:?}");
        assert!(!text.contains("proxy.example"));
        assert!(!text.contains("alice"));
        assert!(!text.contains("hunter2"));
    }
}
