//! Default `device_info.name`: the OS computer name, else `<Platform> device`
//! (docs/data-model.md, `device_info`).
//!
//! The Unix branch calls `gethostname`; together with `hardening` this is the
//! only `unsafe` code of the crate.
#![cfg_attr(unix, allow(unsafe_code))]

use shellpp_core::platform::OsKind;

/// Longest name kept (characters).
const MAX_CHARS: usize = 64;

/// The computer name of this machine, cleaned up, or `<Platform> device`.
pub fn device_name(os: OsKind) -> String {
    raw_name()
        .and_then(|name| clean(&name))
        .unwrap_or_else(|| fallback(os))
}

/// The fallback name for a platform.
pub fn fallback(os: OsKind) -> String {
    let platform = match os {
        OsKind::Windows => "Windows",
        OsKind::Macos => "macOS",
        OsKind::Linux => "Linux",
        OsKind::Ios => "iOS",
        OsKind::Android => "Android",
    };
    format!("{platform} device")
}

/// Trims, drops control characters, limits the length; `None` when empty.
pub fn clean(name: &str) -> Option<String> {
    let cleaned: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_CHARS)
        .collect();
    let cleaned = cleaned.trim().to_owned();
    (!cleaned.is_empty()).then_some(cleaned)
}

#[cfg(windows)]
fn raw_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(unix)]
fn raw_name() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for `buf.len()` bytes for the duration of
    // the call; `gethostname` writes at most that many bytes.
    let result = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if result != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

#[cfg(not(any(windows, unix)))]
fn raw_name() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_names() {
        assert_eq!(clean("  build-box \n").as_deref(), Some("build-box"));
        assert_eq!(clean("a\u{0007}b").as_deref(), Some("ab"));
        assert_eq!(clean("   "), None);
        assert_eq!(clean(&"x".repeat(100)).map(|s| s.chars().count()), Some(64));
    }

    #[test]
    fn fallback_names() {
        assert_eq!(fallback(OsKind::Windows), "Windows device");
        assert_eq!(fallback(OsKind::Macos), "macOS device");
        assert_eq!(fallback(OsKind::Linux), "Linux device");
    }

    #[test]
    fn device_name_is_never_empty() {
        assert!(!device_name(OsKind::Windows).is_empty());
    }
}
