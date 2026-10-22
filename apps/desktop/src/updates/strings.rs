//! Texts of the update check (UI 1.4, 12.7; D97).

use super::CheckError;

pub const OPEN_PAGE: &str = "Open Release Page";
pub const CHECKING: &str = "Checking for updates\u{2026}";

/// The toast and the status of Settings when an update is found.
pub fn available(version: &str) -> String {
    format!("Shell++ {version} is available")
}

/// The Help menu's item while an update is known.
pub fn menu_available(version: &str) -> String {
    format!("Shell++ {version} Is Available\u{2026}")
}

/// After a check that found nothing newer.
pub fn up_to_date(version: &str) -> String {
    format!("Shell++ {version} is the latest version")
}

/// After a check that failed.
pub fn failed(error: CheckError) -> String {
    let reason = match error {
        CheckError::NotHttps => "the update source is not an https:// address",
        CheckError::Unreachable => "the update source did not answer",
        CheckError::BadManifest => "the update source did not send an update manifest",
    };
    format!("The update check failed: {reason}")
}
