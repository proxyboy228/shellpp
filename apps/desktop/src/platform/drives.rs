//! The roots of the local file panel's drive selection (D48, D55): on
//! Windows the drive letters the system has, read with `GetLogicalDrives`
//! without touching any drive (an empty card reader or a disconnected
//! network drive is listed and fails only when it is opened); elsewhere `/`.
//!
//! A platform module (CONTRIBUTING.md, "Unsafe code"): the one
//! `GetLogicalDrives` call is its only `unsafe` code.
#![cfg_attr(windows, allow(unsafe_code))]

use std::path::PathBuf;

/// The roots of the drive selection, in letter order.
pub fn roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        roots_of(logical_drives())
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

/// The roots a `GetLogicalDrives` mask names: bit 0 is `A:`, bit 25 `Z:`;
/// higher bits are ignored.
#[cfg(any(windows, test))]
fn roots_of(mask: u32) -> Vec<PathBuf> {
    (b'A'..=b'Z')
        .enumerate()
        .filter(|(bit, _)| mask & (1 << bit) != 0)
        .map(|(_, letter)| PathBuf::from(format!("{}:\\", char::from(letter))))
        .collect()
}

#[cfg(windows)]
fn logical_drives() -> u32 {
    // SAFETY: GetLogicalDrives takes no arguments and returns a bit mask (0
    // on failure); it reads and writes no memory of ours.
    unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mask_names_its_letters_in_order() {
        assert!(roots_of(0).is_empty());
        assert_eq!(
            roots_of(0b101),
            [PathBuf::from("A:\\"), PathBuf::from("C:\\")]
        );
        assert_eq!(roots_of(1 << 25), [PathBuf::from("Z:\\")]);
        assert_eq!(roots_of(u32::MAX).len(), 26, "bits past Z: are ignored");
    }

    /// The `unsafe` call itself: the system drive is among the letters.
    #[cfg(windows)]
    #[test]
    fn the_system_drive_is_listed() {
        let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
        let roots = roots();
        assert!(
            roots.contains(&PathBuf::from(format!("{system}\\"))),
            "{roots:?}"
        );
        assert_eq!(roots, roots_of(logical_drives()));
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_the_root_is_the_only_drive() {
        assert_eq!(roots(), [PathBuf::from("/")]);
    }
}
