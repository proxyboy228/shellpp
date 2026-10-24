//! Configuration through the environment (docs/architecture.md, section
//! 10.2). Values are never logged.

use std::net::{IpAddr, SocketAddr};

use shellpp_sync_api::api::Registration;
use zeroize::Zeroizing;

use crate::ratelimit::RateLimits;

/// A CIDR block of trusted proxies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl Cidr {
    pub fn parse(text: &str) -> Option<Cidr> {
        let (addr, prefix) = match text.split_once('/') {
            Some((addr, prefix)) => (addr, prefix.parse::<u8>().ok()?),
            None => {
                let addr: IpAddr = text.parse().ok()?;
                let prefix = if addr.is_ipv4() { 32 } else { 128 };
                return Some(Cidr { addr, prefix });
            }
        };
        let addr: IpAddr = addr.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        (prefix <= max).then_some(Cidr { addr, prefix })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// The server's configuration.
#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: SocketAddr,
    /// The https origin the clients use (normalized).
    pub public_url: String,
    /// At least 32 bytes: the root of the server-only keys (wiped on drop).
    pub server_secret: Zeroizing<Vec<u8>>,
    pub registration: Registration,
    pub quota_max_records: Option<u64>,
    pub quota_max_bytes: Option<u64>,
    pub trusted_proxies: Vec<Cidr>,
    pub tombstone_retention_days: u64,
    pub rate_limits: RateLimits,
    /// What the operator should change, logged by `serve` (never a value).
    pub warnings: Vec<&'static str>,
}

/// The warning of a server secret in the environment (D177).
pub const SECRET_IN_ENVIRONMENT: &str = "SHELLPP_SERVER_SECRET is read from the environment, \
     where `docker inspect` and the process list of this user show it; put the same value in a \
     file and name it with SHELLPP_SERVER_SECRET_FILE (Docker secrets, server README)";

/// The longest server secret file read (a base64 secret of 32 to a few
/// hundred bytes, and a line break).
const SECRET_FILE_MAX: u64 = 4096;

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind", &self.bind)
            .field("registration", &self.registration)
            .finish_non_exhaustive()
    }
}

/// Why the configuration was refused (names the variable, never its value).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

impl Config {
    /// Reads the configuration from `get` (the environment in `main`).
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let required = |name: &str| {
            get(name)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| ConfigError(format!("{name} is required")))
        };
        let database_url = required("SHELLPP_DATABASE_URL")?;
        let bind = match get("SHELLPP_BIND") {
            Some(value) => value
                .parse()
                .map_err(|_| ConfigError("SHELLPP_BIND is not an address and port".to_owned()))?,
            None => SocketAddr::from(([127, 0, 0, 1], 8080)),
        };
        let public_url = normalize_origin(&required("SHELLPP_PUBLIC_URL")?)
            .ok_or_else(|| ConfigError("SHELLPP_PUBLIC_URL is not an http(s) origin".to_owned()))?;
        // The secret from a file (Docker secrets), or for compatibility from
        // the environment with a warning; both at once is refused, so no one
        // has to guess which of two secrets derives the keys (D177).
        let mut warnings = Vec::new();
        let in_env = get("SHELLPP_SERVER_SECRET").map(Zeroizing::new);
        let in_env = in_env.filter(|value| !value.trim().is_empty());
        let file = get("SHELLPP_SERVER_SECRET_FILE").filter(|value| !value.trim().is_empty());
        let (name, secret) = match (in_env, file) {
            (Some(_), Some(_)) => {
                return Err(ConfigError(
                    "SHELLPP_SERVER_SECRET and SHELLPP_SERVER_SECRET_FILE are both set; keep only \
                     SHELLPP_SERVER_SECRET_FILE"
                        .to_owned(),
                ));
            }
            (None, Some(path)) => ("SHELLPP_SERVER_SECRET_FILE", read_secret_file(path.trim())?),
            (Some(secret), None) => {
                warnings.push(SECRET_IN_ENVIRONMENT);
                ("SHELLPP_SERVER_SECRET", secret)
            }
            (None, None) => {
                return Err(ConfigError(
                    "SHELLPP_SERVER_SECRET_FILE (or SHELLPP_SERVER_SECRET) is required".to_owned(),
                ));
            }
        };
        let server_secret = decode_secret(secret.trim())
            .ok_or_else(|| ConfigError(format!("{name} is not base64")))?;
        if server_secret.len() < 32 {
            return Err(ConfigError(format!("{name} must hold at least 32 bytes")));
        }
        let registration = match get("SHELLPP_REGISTRATION").as_deref().map(str::trim) {
            None | Some("") | Some("invite") => Registration::Invite,
            Some("open") => Registration::Open,
            Some(_) => {
                return Err(ConfigError(
                    "SHELLPP_REGISTRATION is open or invite".to_owned(),
                ));
            }
        };
        let number = |name: &str| -> Result<Option<u64>, ConfigError> {
            match get(name).filter(|value| !value.trim().is_empty()) {
                None => Ok(None),
                Some(value) => value
                    .trim()
                    .parse::<u64>()
                    .map(Some)
                    .map_err(|_| ConfigError(format!("{name} is not a number"))),
            }
        };
        let trusted_proxies = match get("SHELLPP_TRUSTED_PROXIES") {
            None => Vec::new(),
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| {
                    Cidr::parse(item).ok_or_else(|| {
                        ConfigError("SHELLPP_TRUSTED_PROXIES holds an invalid CIDR".to_owned())
                    })
                })
                .collect::<Result<_, _>>()?,
        };
        Ok(Config {
            database_url,
            bind,
            public_url,
            server_secret,
            registration,
            quota_max_records: number("SHELLPP_QUOTA_MAX_RECORDS")?,
            quota_max_bytes: number("SHELLPP_QUOTA_MAX_BYTES")?,
            trusted_proxies,
            tombstone_retention_days: number("SHELLPP_TOMBSTONE_RETENTION_DAYS")?.unwrap_or(90),
            rate_limits: RateLimits::from_env(&get)?,
            warnings,
        })
    }
}

/// The text of the server secret file at `path`, in a buffer wiped on
/// drop. The error names the variable, never the path's contents.
fn read_secret_file(path: &str) -> Result<Zeroizing<String>, ConfigError> {
    use std::io::Read as _;
    let unreadable =
        |what: &str| ConfigError(format!("SHELLPP_SERVER_SECRET_FILE cannot be read: {what}"));
    // A regular file only: opening a FIFO would block the start for ever.
    let metadata = std::fs::metadata(path).map_err(|e| unreadable(&e.kind().to_string()))?;
    if !metadata.is_file() {
        return Err(unreadable("not a regular file"));
    }
    if metadata.len() > SECRET_FILE_MAX {
        return Err(unreadable("larger than 4 KiB"));
    }
    let mut file = std::fs::File::open(path).map_err(|e| unreadable(&e.kind().to_string()))?;
    // Allocated once at its largest size: a buffer that grows leaves
    // unwiped copies behind (security.md, section 9.5).
    let mut bytes = Zeroizing::new(Vec::with_capacity(SECRET_FILE_MAX as usize + 1));
    (&mut file)
        .take(SECRET_FILE_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| unreadable(&e.kind().to_string()))?;
    if bytes.len() as u64 > SECRET_FILE_MAX {
        return Err(unreadable("larger than 4 KiB"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| unreadable("not text"))?;
    // A byte order mark, as some editors and PowerShell 5 write one.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut secret = Zeroizing::new(String::with_capacity(text.len()));
    secret.push_str(text);
    Ok(secret)
}

/// The bytes of a base64 server secret (with or without padding), decoded
/// into a buffer that is wiped on drop, on failure too.
fn decode_secret(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    for encoding in [&data_encoding::BASE64, &data_encoding::BASE64_NOPAD] {
        let Ok(len) = encoding.decode_len(text.len()) else {
            continue;
        };
        let mut out = Zeroizing::new(vec![0u8; len]);
        if let Ok(written) = encoding.decode_mut(text.as_bytes(), &mut out) {
            out.truncate(written);
            return Some(out);
        }
    }
    None
}

/// Scheme and host in lowercase, the port only when not the default, no
/// path or trailing slash (docs/sync-protocol.md, section 2).
pub fn normalize_origin(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => 443,
        "http" => 80,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (host, after) = stripped.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port.parse::<u16>().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        (format!("[{}]", host.to_ascii_lowercase()), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_ascii_lowercase(), Some(port.parse::<u16>().ok()?)),
            None => (authority.to_ascii_lowercase(), None),
        }
    };
    if host.is_empty() {
        return None;
    }
    Some(match port {
        Some(port) if port != default_port => format!("{scheme}://{host}:{port}"),
        _ => format!("{scheme}://{host}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_normalized() {
        assert_eq!(
            normalize_origin("HTTPS://Sync.Example.com:443/").as_deref(),
            Some("https://sync.example.com")
        );
        assert_eq!(
            normalize_origin("https://sync.example.com:8443/v1").as_deref(),
            Some("https://sync.example.com:8443")
        );
        assert_eq!(
            normalize_origin("http://[::1]:8080").as_deref(),
            Some("http://[::1]:8080")
        );
        assert_eq!(normalize_origin("ftp://x"), None);
        assert_eq!(normalize_origin("https://user@x"), None);
    }

    #[test]
    fn cidr_matching() {
        let net = Cidr::parse("172.18.0.0/16").unwrap();
        assert!(net.contains("172.18.4.2".parse().unwrap()));
        assert!(!net.contains("172.19.0.1".parse().unwrap()));
        let one = Cidr::parse("10.0.0.5").unwrap();
        assert!(one.contains("10.0.0.5".parse().unwrap()));
        assert!(!one.contains("10.0.0.6".parse().unwrap()));
        assert!(Cidr::parse("10.0.0.0/33").is_none());
        let v6 = Cidr::parse("fd00::/8").unwrap();
        assert!(v6.contains("fd12::1".parse().unwrap()));
    }

    #[test]
    fn the_environment_is_checked() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        let secret = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
        let ok = Config::from_env(env(&[
            ("SHELLPP_DATABASE_URL", "sqlite::memory:"),
            ("SHELLPP_PUBLIC_URL", "https://Sync.Example.com/"),
            (
                "SHELLPP_SERVER_SECRET",
                "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
            ),
        ]))
        .unwrap();
        assert_eq!(ok.public_url, "https://sync.example.com");
        assert_eq!(ok.registration, Registration::Invite);
        assert_eq!(ok.quota_max_records, None);
        assert_eq!(ok.bind, SocketAddr::from(([127, 0, 0, 1], 8080)));
        let _ = secret;
        let short = Config::from_env(env(&[
            ("SHELLPP_DATABASE_URL", "sqlite::memory:"),
            ("SHELLPP_PUBLIC_URL", "https://x"),
            ("SHELLPP_SERVER_SECRET", "c2hvcnQ="),
        ]));
        assert!(short.is_err());
        let missing = Config::from_env(env(&[("SHELLPP_DATABASE_URL", "sqlite::memory:")]));
        assert!(missing.unwrap_err().0.contains("SHELLPP_PUBLIC_URL"));
    }

    /// D177: the secret is read from the file `SHELLPP_SERVER_SECRET_FILE`
    /// names (with or without a line break) without a warning; from the
    /// environment it works with a warning; both at once, an empty file, a
    /// missing or oversized one are refused, naming the variable only.
    #[test]
    fn the_server_secret_comes_from_a_file() {
        let bytes = crate::secrets::random_bytes::<40>();
        let text = data_encoding::BASE64.encode(&bytes);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server_secret");
        let path_text = path.display().to_string();
        let config_with = |pairs: Vec<(&'static str, String)>| {
            Config::from_env(move |name: &str| {
                let base = [
                    ("SHELLPP_DATABASE_URL", "sqlite::memory:".to_owned()),
                    ("SHELLPP_PUBLIC_URL", "https://sync.example.com".to_owned()),
                ];
                base.iter()
                    .chain(pairs.iter())
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.clone())
            })
        };
        for content in [
            text.clone(),
            format!("{text}\n"),
            format!("  {text}\r\n"),
            format!("\u{feff}{text}\r\n"),
        ] {
            std::fs::write(&path, content).unwrap();
            let config =
                config_with(vec![("SHELLPP_SERVER_SECRET_FILE", path_text.clone())]).unwrap();
            assert_eq!(config.server_secret.as_slice(), &bytes[..]);
            assert!(config.warnings.is_empty(), "no warning for a file");
        }
        // The environment: the same secret, with the warning.
        let config = config_with(vec![("SHELLPP_SERVER_SECRET", text.clone())]).unwrap();
        assert_eq!(config.server_secret.as_slice(), &bytes[..]);
        assert_eq!(config.warnings, vec![SECRET_IN_ENVIRONMENT]);
        // Both: refused, even when they agree.
        let both = config_with(vec![
            ("SHELLPP_SERVER_SECRET", text.clone()),
            ("SHELLPP_SERVER_SECRET_FILE", path_text.clone()),
        ])
        .unwrap_err();
        assert!(both.0.contains("both set"), "{}", both.0);
        // Neither.
        let neither = config_with(Vec::new()).unwrap_err();
        assert!(
            neither.0.contains("SHELLPP_SERVER_SECRET_FILE"),
            "{}",
            neither.0
        );
        // A missing file, a folder, an empty one, a short secret, an
        // oversized file.
        let folder = dir.path().display().to_string();
        let error = config_with(vec![("SHELLPP_SERVER_SECRET_FILE", folder)]).unwrap_err();
        assert!(error.0.contains("not a regular file"), "{}", error.0);
        let missing = dir.path().join("absent").display().to_string();
        let error = config_with(vec![("SHELLPP_SERVER_SECRET_FILE", missing)]).unwrap_err();
        assert!(
            error
                .0
                .starts_with("SHELLPP_SERVER_SECRET_FILE cannot be read"),
            "{}",
            error.0
        );
        std::fs::write(&path, "").unwrap();
        let error =
            config_with(vec![("SHELLPP_SERVER_SECRET_FILE", path_text.clone())]).unwrap_err();
        assert!(
            error.0.contains("SHELLPP_SERVER_SECRET_FILE"),
            "{}",
            error.0
        );
        std::fs::write(&path, "c2hvcnQ=").unwrap();
        let error =
            config_with(vec![("SHELLPP_SERVER_SECRET_FILE", path_text.clone())]).unwrap_err();
        assert!(error.0.contains("at least 32 bytes"), "{}", error.0);
        std::fs::write(&path, format!("{text}{}", " ".repeat(5000))).unwrap();
        let error =
            config_with(vec![("SHELLPP_SERVER_SECRET_FILE", path_text.clone())]).unwrap_err();
        assert!(error.0.contains("4 KiB"), "{}", error.0);
        // A broken secret in the file: the error never shows it.
        let broken = format!("{}!", data_encoding::BASE64_NOPAD.encode(&bytes));
        std::fs::write(&path, &broken).unwrap();
        let error = config_with(vec![("SHELLPP_SERVER_SECRET_FILE", path_text)]).unwrap_err();
        assert!(
            error.0.contains("SHELLPP_SERVER_SECRET_FILE is not base64"),
            "{}",
            error.0
        );
        assert!(!error.0.contains(&broken[..8]));
    }

    /// The server secret is decoded with and without padding into a buffer
    /// that is wiped on drop, and neither it nor the database password
    /// shows in `Debug` or in an error.
    #[test]
    fn the_server_secret_is_wiped_and_never_shown() {
        fn wiped_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
        let bytes = crate::secrets::random_bytes::<33>();
        let password = data_encoding::HEXLOWER.encode(&crate::secrets::random_bytes::<12>());
        let url = format!("postgres://shellpp:{password}@127.0.0.1:5432/shellpp");
        for text in [
            data_encoding::BASE64.encode(&bytes),
            data_encoding::BASE64_NOPAD.encode(&bytes),
        ] {
            let get = |name: &str| match name {
                "SHELLPP_DATABASE_URL" => Some(url.clone()),
                "SHELLPP_PUBLIC_URL" => Some("https://sync.example.com".to_owned()),
                "SHELLPP_SERVER_SECRET" => Some(text.clone()),
                _ => None,
            };
            let config = Config::from_env(get).unwrap();
            assert_eq!(config.server_secret.as_slice(), &bytes[..]);
            wiped_on_drop(&config.server_secret);
            let shown = format!("{config:?}");
            assert!(!shown.contains(&text), "{shown}");
            assert!(!shown.contains(&password), "{shown}");
            assert!(!shown.contains(&format!("{:?}", &bytes[..4])), "{shown}");
        }
        // A broken secret: the error names the variable, never the value.
        let broken = format!("{}!", data_encoding::BASE64_NOPAD.encode(&bytes));
        let error = Config::from_env(|name: &str| match name {
            "SHELLPP_DATABASE_URL" => Some(url.clone()),
            "SHELLPP_PUBLIC_URL" => Some("https://sync.example.com".to_owned()),
            "SHELLPP_SERVER_SECRET" => Some(broken.clone()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.0.contains("SHELLPP_SERVER_SECRET"));
        assert!(!error.0.contains(&broken[..8]), "{}", error.0);
        assert!(!format!("{error:?}").contains(&broken[..8]));
    }
}
