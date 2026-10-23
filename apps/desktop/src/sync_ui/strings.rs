//! Text of the sync interface (UI 12.2, 13.2, 13.5-13.7, 14).

pub const CONNECT_TITLE: &str = "Connect a sync server";
pub const SERVER_ADDRESS: &str = "Server address";
pub const SERVER_HINT: &str = "Your own Shell++ server, for example sync.example.com. Plain http:// works only for localhost.";
pub const CHECK: &str = "Continue";
pub const CHECKING: &str = "Checking…";
pub const CANCEL: &str = "Cancel";
pub const BACK: &str = "Back";
pub const CLOSE: &str = "Close";

pub const TRUST_TITLE: &str = "Check the server's certificate";
pub const TRUST_UNKNOWN: &str = "This certificate is not issued by an authority your system trusts. Trust it only if the fingerprint matches your server's certificate (server/README.md shows how to read it).";
pub const TRUST_CHANGED: &str = "The server shows another certificate than the one you trusted. Sync is stopped until you confirm the new one. If you did not renew the certificate, do not continue.";
pub const TRUST_REFUSED_EXPIRED: &str = "The certificate has expired or is not valid yet. Shell++ does not connect to it, even when trusted.";
pub const TRUST_REFUSED_NAME: &str = "The certificate is not issued for this address. Shell++ does not connect to it, even when trusted.";
pub const TRUST_SUBJECT: &str = "Issued to";
pub const TRUST_ISSUER: &str = "Issued by";
pub const TRUST_VALID: &str = "Valid";
pub const TRUST_FINGERPRINT: &str = "SHA-256 fingerprint";
pub const TRUST_PIN: &str = "Trust this certificate";

pub const MODE_SIGN_IN: &str = "Sign in";
pub const MODE_REGISTER: &str = "Create account";
pub const EMAIL: &str = "Email";
pub const PASSWORD: &str = "Master password";
pub const PASSWORD_HINT_REGISTER: &str = "The master password of this device. It stays the password of the account and of the local copy.";
pub const PASSWORD_HINT_SIGN_IN: &str = "The master password of the account.";
pub const INVITE_CODE: &str = "Invitation code";
pub const INVITE_HINT: &str = "This server registers by invitation: ask its owner for a code.";
pub const RUN_SIGN_IN: &str = "Sign in";
pub const RUN_REGISTER: &str = "Create account";
pub const RUNNING: &str = "Connecting…";
pub const MERGE_NOTE: &str = "Signing in on this device merges its local data into the account. A backup of the local data is written first.";

pub const TOTP_TITLE: &str = "Two-step sign-in";
pub const TOTP_CODE: &str = "6-digit code from your authenticator app";
pub const TOTP_RUN: &str = "Verify";

pub const DONE_TITLE: &str = "Connected";
pub const DONE_TEXT: &str = "This device now syncs with your server.";

pub const ACCOUNT: &str = "Account";
pub const STATUS_SIGNED_OUT: &str = "Signed out: sign in again to resume sync.";
pub const STATUS_SYNCED: &str = "Synced";
pub const STATUS_NEVER: &str = "Not synced yet";
pub const STATUS_PENDING: &str = "not synced yet";
pub const STATUS_ERROR: &str = "Last error";
pub const SYNC_NOW: &str = "Sync now";
pub const SIGN_IN_AGAIN: &str = "Sign in again…";
pub const DEVICES: &str = "Devices…";
pub const REVIEW: &str = "Review…";
pub const SIGN_OUT: &str = "Sign out…";
pub const DELETE_ACCOUNT: &str = "Delete account…";
pub const RECENTLY_DELETED: &str = "Recently deleted…";
pub const ROLLED_BACK: &str = "Server state rolled back: the server holds older keys than this device. Bring it up to date only if you restored it from a backup.";
pub const REPOST: &str = "Bring the server up to date…";
pub const NEWER_EPOCH: &str = "The account has newer keys: sign in again on this device.";
pub const RECOVERY_MISMATCH: &str = "The server's copy of the recovery wrap does not match this account's recovery key. Reissue the recovery key.";
pub const INTEGRITY: &str = "records from the server were refused as damaged or forged";

pub const TOGGLES: &str = "What syncs";
pub const TOGGLES_HINT: &str =
    "Per device. Turning a type off stops syncing it and deletes nothing.";

pub const REVIEW_TITLE: &str = "Review sync";
pub const CONFLICTS: &str = "Conflicts";
pub const CONFLICTS_HINT: &str =
    "Two devices changed the same item. The newer change is in place; the other one is kept here.";
pub const KEEP_CURRENT: &str = "Keep current";
pub const KEEP_OTHER: &str = "Use other version";
pub const OTHER_DELETED: &str = "The other version deleted it.";
pub const DUPLICATES: &str = "Possible duplicates";
pub const DUPLICATES_HINT: &str = "Items created on two devices that look the same.";
pub const MERGE: &str = "Merge";
pub const KEEP_BOTH: &str = "Keep both";
pub const BROUGHT_BACK: &str = "Brought back";
pub const BROUGHT_BACK_HINT: &str =
    "Deleted elsewhere long ago, but still here: they were restored to the server.";
pub const KEEP: &str = "Keep";
pub const DELETE_AGAIN: &str = "Delete again";
pub const NOTHING_TO_REVIEW: &str = "Nothing to review.";

pub const TRASH_TITLE: &str = "Recently deleted";
pub const TRASH_HINT: &str = "Deleted items stay here for 90 days, on this device only, encrypted.";
pub const TRASH_EMPTY: &str = "Nothing deleted recently.";
pub const RESTORE: &str = "Restore";
pub const DELETE_FOREVER: &str = "Delete forever";
pub const EMPTY_TRASH: &str = "Empty";
pub const FROM_SYNC: &str = "deleted on another device";

pub const DEVICES_TITLE: &str = "Devices";
pub const THIS_DEVICE: &str = "this device";
pub const REVOKED: &str = "signed out";
pub const REVOKE: &str = "Sign out";
pub const LAST_SEEN: &str = "last seen";

pub const SIGN_OUT_TITLE: &str = "Sign out of the account";
pub const SIGN_OUT_TEXT: &str =
    "Sync stops on this device. The local copy stays and opens with the same password.";
pub const DELETE_LOCAL: &str = "Also delete the local copy on this device";
pub const SIGN_OUT_RUN: &str = "Sign out";

pub const DELETE_TITLE: &str = "Delete the account";
pub const DELETE_TEXT: &str = "The account and its data on the server are deleted. Every device keeps its local copy. Export your data first if you want a file of it.";
pub const EXPORT_FIRST: &str = "Export data…";
pub const DELETE_RUN: &str = "Delete account";

pub const REPOST_TITLE: &str = "Bring the server up to date";
pub const REPOST_TEXT: &str = "Sends the newer keys this device holds to the server, then the records the server lost. Do this only after restoring the server from a backup.";
pub const REPOST_RUN: &str = "Bring up to date";

pub const RESTORE_FROM_SYNC: &str = "Restore from sync…";

pub const ERROR_ADDRESS: &str = "Not a valid server address.";
pub const ERROR_HTTP: &str = "Plain http:// is allowed only for localhost.";
pub const ERROR_NETWORK: &str = "The server did not answer.";
pub const ERROR_CREDENTIALS: &str = "Wrong email or password.";
pub const ERROR_TOTP: &str = "Wrong code.";
pub const ERROR_TICKET: &str = "The sign-in expired. Start again.";
pub const ERROR_CLOSED: &str = "This server needs a valid invitation code.";
pub const ERROR_TAKEN: &str = "An account with this email exists: sign in instead.";
pub const ERROR_PASSWORD: &str = "This is not the master password of this device.";
pub const ERROR_KDF: &str = "The server sent unsafe key derivation parameters.";
pub const ERROR_PREVIOUS: &str = "This device was bound to another account before; connecting it to a new one is not supported yet.";
pub const ERROR_ROLLED_BACK: &str =
    "Server state rolled back: the server holds older keys than this device.";
pub const ERROR_INTEGRITY: &str =
    "The server sent data that does not open with this account's keys.";
pub const ERROR_OTHER: &str = "Something went wrong. Try again.";

pub const TOTP_SETUP: &str = "Two-step sign-in…";
pub const TOTP_INTRO: &str =
    "Every new sign-in to the account will also ask for a code from an authenticator app.";
pub const TOTP_SECRET_HINT: &str = "Add this key to your authenticator app (or open the link below on your phone), then enter the code it shows.";
pub const TOTP_ON: &str = "Two-step sign-in is on.";
