//! OS detection (UI 12.4 "OS detection"; docs/data-model.md, section 7.6;
//! docs/security.md, section 11.4).
//!
//! 1. From the server identification string (source `banner`), for example
//!    `SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13` -> linux / ubuntu, or
//!    `SSH-2.0-OpenSSH_for_Windows_9.5` -> windows.
//! 2. Otherwise, on the first successful connection to a saved host without
//!    `os`, the desktop host runs the script ([`DEFAULT_SCRIPT`] or the
//!    setting `terminal.os_detection_script`) in a separate exec channel (never
//!    in the interactive session, never recorded) with [`EXEC_TIMEOUT_SECS`]
//!    and [`MAX_OUTPUT_BYTES`], and parses its stdout (source `script`).
//!
//! The output is untrusted: it is mapped to the fixed set of
//! [`crate::service::dto::OsFamily`] values and a short lowercase distro id
//! (`[a-z0-9._-]{1,32}`); anything else gives `other` or nothing.
//!
//! Owner: S4. Signatures written by the lead (S4/S5 contract).

use crate::service::dto::OsFamily;

/// The built-in detection script (POSIX sh). Prints the kernel name, then
/// the os-release file.
pub const DEFAULT_SCRIPT: &str =
    "uname -s 2>/dev/null; cat /etc/os-release 2>/dev/null || cat /usr/lib/os-release 2>/dev/null";

/// Timeout of the exec channel.
pub const EXEC_TIMEOUT_SECS: u64 = 5;

/// Stdout beyond this is ignored.
pub const MAX_OUTPUT_BYTES: usize = 4096;

/// Longest `distro` and `version` (docs/data-model.md, section 7.6).
pub const MAX_FIELD_CHARS: usize = 32;

/// A detection result before it is stamped with the source and the time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedOs {
    pub family: OsFamily,
    pub distro: Option<String>,
    pub version: Option<String>,
}

impl DetectedOs {
    fn new(family: OsFamily, distro: Option<&str>) -> Self {
        Self {
            family,
            distro: distro.map(str::to_owned),
            version: None,
        }
    }

    /// The same result with `distro` and `version` dropped when they break
    /// the rules (a distro id `[a-z0-9._-]{1,32}`, a printable version of at
    /// most 32 characters). Applied to every value before it is stored.
    pub fn sanitized(self) -> Self {
        Self {
            family: self.family,
            distro: self.distro.as_deref().and_then(distro_id),
            version: self.version.as_deref().and_then(version_text),
        }
    }
}

/// A distro id: lowercased, `[a-z0-9._-]{1,32}`.
fn distro_id(raw: &str) -> Option<String> {
    let id = raw.trim().to_ascii_lowercase();
    let ok = (1..=MAX_FIELD_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b));
    ok.then_some(id)
}

/// A version: printable, 1 to 32 characters.
fn version_text(raw: &str) -> Option<String> {
    let version = raw.trim();
    let ok = !version.is_empty()
        && version.chars().count() <= MAX_FIELD_CHARS
        && version.chars().all(|c| !c.is_control());
    ok.then(|| version.to_owned())
}

/// The OS named by the server identification string, if any.
pub fn from_banner(ident: &str) -> Option<DetectedOs> {
    let ident = ident.trim();
    let rest = ident
        .strip_prefix("SSH-2.0-")
        .or_else(|| ident.strip_prefix("SSH-1.99-"))
        .unwrap_or(ident);
    let mut words = rest.split_whitespace();
    let software = words.next()?;
    if software.starts_with("OpenSSH_for_Windows") {
        return Some(DetectedOs::new(OsFamily::Windows, None));
    }
    for comment in words {
        let found = if comment.starts_with("Ubuntu-") {
            Some((OsFamily::Linux, "ubuntu"))
        } else if comment.starts_with("Debian-") {
            Some((OsFamily::Linux, "debian"))
        } else if comment.starts_with("Raspbian-") {
            Some((OsFamily::Linux, "raspbian"))
        } else if comment.starts_with("FreeBSD-") {
            Some((OsFamily::Bsd, "freebsd"))
        } else {
            None
        };
        if let Some((family, distro)) = found {
            return Some(DetectedOs::new(family, Some(distro)));
        }
    }
    None
}

/// A `KEY=value` line of os-release with the value unquoted.
fn release_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let value = line.trim().strip_prefix(key)?.strip_prefix('=')?.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(value);
    Some(value)
}

/// Parses the stdout of the detection script: first line `Linux`, `Darwin`,
/// `FreeBSD`, `OpenBSD`, `NetBSD`, `MINGW*`/`MSYS*`/`CYGWIN*`; then `ID=` and
/// `VERSION_ID=` lines. `None` when nothing is recognized.
pub fn from_script_output(stdout: &str) -> Option<DetectedOs> {
    let stdout = if stdout.len() > MAX_OUTPUT_BYTES {
        let mut cut = MAX_OUTPUT_BYTES;
        while !stdout.is_char_boundary(cut) {
            cut -= 1;
        }
        &stdout[..cut]
    } else {
        stdout
    };
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or("").trim();
    let upper = first.to_ascii_uppercase();
    // A first line with `=` is os-release output without `uname`.
    let (family, uname_distro) = match first {
        "" => (None, None),
        "Linux" => (Some(OsFamily::Linux), None),
        "Darwin" => (Some(OsFamily::Macos), None),
        "FreeBSD" => (Some(OsFamily::Bsd), Some("freebsd")),
        "OpenBSD" => (Some(OsFamily::Bsd), Some("openbsd")),
        "NetBSD" => (Some(OsFamily::Bsd), Some("netbsd")),
        _ if upper.starts_with("MINGW")
            || upper.starts_with("MSYS")
            || upper.starts_with("CYGWIN") =>
        {
            (Some(OsFamily::Windows), None)
        }
        _ if first.contains('=') => (None, None),
        _ => (Some(OsFamily::Other), None),
    };
    let mut id = None;
    let mut version = None;
    for line in stdout.lines() {
        if id.is_none()
            && let Some(value) = release_value(line, "ID")
        {
            id = distro_id(value);
        }
        if version.is_none()
            && let Some(value) = release_value(line, "VERSION_ID")
        {
            version = version_text(value);
        }
    }
    let family = match family {
        Some(family) => family,
        // os-release without `uname`: a Linux system.
        None if id.is_some() => OsFamily::Linux,
        None => return None,
    };
    let distro = match uname_distro {
        Some(name) => Some(name.to_owned()),
        None if family == OsFamily::Macos || family == OsFamily::Windows => None,
        None => id,
    };
    Some(DetectedOs {
        family,
        distro,
        version: if family == OsFamily::Windows {
            None
        } else {
            version
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(family: OsFamily, distro: Option<&str>, version: Option<&str>) -> Option<DetectedOs> {
        Some(DetectedOs {
            family,
            distro: distro.map(str::to_owned),
            version: version.map(str::to_owned),
        })
    }

    #[test]
    fn banners() {
        assert_eq!(
            from_banner("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13"),
            os(OsFamily::Linux, Some("ubuntu"), None)
        );
        assert_eq!(
            from_banner("SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3\r\n"),
            os(OsFamily::Linux, Some("debian"), None)
        );
        assert_eq!(
            from_banner("SSH-2.0-OpenSSH_7.9p1 Raspbian-10+deb10u2"),
            os(OsFamily::Linux, Some("raspbian"), None)
        );
        assert_eq!(
            from_banner("SSH-2.0-OpenSSH_9.3 FreeBSD-20230719"),
            os(OsFamily::Bsd, Some("freebsd"), None)
        );
        assert_eq!(
            from_banner("SSH-2.0-OpenSSH_for_Windows_9.5"),
            os(OsFamily::Windows, None, None)
        );
        assert_eq!(
            from_banner("OpenSSH_for_Windows_8.1"),
            os(OsFamily::Windows, None, None)
        );
        assert_eq!(from_banner("SSH-2.0-OpenSSH_9.6"), None);
        assert_eq!(from_banner("SSH-2.0-shellpp_test_sshd"), None);
        assert_eq!(from_banner("SSH-2.0-dropbear_2022.83"), None);
        assert_eq!(from_banner("SSH-2.0-OpenSSH_9.6 ubuntu-3"), None);
        assert_eq!(from_banner(""), None);
    }

    #[test]
    fn script_output() {
        assert_eq!(
            from_script_output(
                "Linux\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nNAME=\"Debian GNU/Linux\"\nVERSION_ID=\"12\"\nID=debian\n"
            ),
            os(OsFamily::Linux, Some("debian"), Some("12"))
        );
        assert_eq!(
            from_script_output("Linux\nID=\"Ubuntu\"\nVERSION_ID='24.04'\n"),
            os(OsFamily::Linux, Some("ubuntu"), Some("24.04"))
        );
        assert_eq!(
            from_script_output("Linux\n"),
            os(OsFamily::Linux, None, None)
        );
        assert_eq!(
            from_script_output("Darwin\n"),
            os(OsFamily::Macos, None, None)
        );
        assert_eq!(
            from_script_output("FreeBSD\nNAME=FreeBSD\nVERSION_ID=14.1\nID=freebsd\n"),
            os(OsFamily::Bsd, Some("freebsd"), Some("14.1"))
        );
        assert_eq!(
            from_script_output("OpenBSD\n"),
            os(OsFamily::Bsd, Some("openbsd"), None)
        );
        assert_eq!(
            from_script_output("NetBSD\n"),
            os(OsFamily::Bsd, Some("netbsd"), None)
        );
        assert_eq!(
            from_script_output("MINGW64_NT-10.0-19045\n"),
            os(OsFamily::Windows, None, None)
        );
        assert_eq!(
            from_script_output("MSYS_NT-10.0\nID=msys2\n"),
            os(OsFamily::Windows, None, None)
        );
        assert_eq!(
            from_script_output("CYGWIN_NT-10.0\n"),
            os(OsFamily::Windows, None, None)
        );
        assert_eq!(
            from_script_output("SunOS\n"),
            os(OsFamily::Other, None, None)
        );
        // os-release without uname.
        assert_eq!(
            from_script_output("NAME=Alpine\nID=alpine\nVERSION_ID=3.20.1\n"),
            os(OsFamily::Linux, Some("alpine"), Some("3.20.1"))
        );
        assert_eq!(from_script_output(""), None);
        assert_eq!(from_script_output("\n\n"), None);
        assert_eq!(from_script_output("NAME=x\n"), None);
    }

    #[test]
    fn untrusted_values_are_constrained() {
        // An id with forbidden characters or too long is dropped.
        assert_eq!(
            from_script_output("Linux\nID=\"evil<script>\"\nVERSION_ID=\"1\u{1b}[31m\"\n"),
            os(OsFamily::Linux, None, None)
        );
        let long = format!(
            "Linux\nID={}\nVERSION_ID={}\n",
            "a".repeat(33),
            "9".repeat(33)
        );
        assert_eq!(from_script_output(&long), os(OsFamily::Linux, None, None));
        // Only the first ID line counts; ID_LIKE is not ID.
        assert_eq!(
            from_script_output("Linux\nID_LIKE=debian\nID=pop\nID=other\n"),
            os(OsFamily::Linux, Some("pop"), None)
        );
        // Output beyond the limit is ignored.
        let mut big = String::from("Linux\n");
        big.push_str(&"x".repeat(MAX_OUTPUT_BYTES));
        big.push_str("\nID=late\n");
        assert_eq!(from_script_output(&big), os(OsFamily::Linux, None, None));
        // Sanitizing applies the same rules to any value.
        let dirty = DetectedOs {
            family: OsFamily::Linux,
            distro: Some("Bad Id".into()),
            version: Some("ok".into()),
        };
        assert_eq!(
            dirty.sanitized(),
            DetectedOs {
                family: OsFamily::Linux,
                distro: None,
                version: Some("ok".into())
            }
        );
        assert_eq!(
            DetectedOs {
                family: OsFamily::Linux,
                distro: Some("Ubuntu".into()),
                version: None
            }
            .sanitized()
            .distro
            .as_deref(),
            Some("ubuntu")
        );
    }
}
