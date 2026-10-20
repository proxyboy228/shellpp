//! Unix: mode 0600, set when the file is opened and again on an existing
//! file before it is written; `O_NOFOLLOW` refuses a symbolic link that
//! appears after the check in [`crate::write`].

use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

pub fn prepare(options: &mut OpenOptions) {
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
}

pub fn restrict(file: &File) -> io::Result<()> {
    file.set_permissions(Permissions::from_mode(0o600))
}

pub fn is_private(path: &Path) -> io::Result<bool> {
    let mode = std::fs::metadata(path)?.permissions().mode();
    Ok(mode & 0o077 == 0)
}
