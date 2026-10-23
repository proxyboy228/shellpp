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
// Not "the newer change": a revoked known host key wins whatever its age
// (sync-protocol 6.7).
pub const CONFLICTS_HINT: &str =
    "Two devices changed the same item. One change is in place; the other one is kept here.";
// The comparison of a conflict (UI 13.6).
pub const CONFLICTS_TITLE: &str = "Sync conflicts";
pub const NO_CONFLICTS: &str = "No conflicts.";
pub const COMPARE_TITLE: &str = "Compare versions";
pub const COMPARE_HINT: &str =
    "Two devices changed this item at the same time. Keep one version; the other one is deleted.";
pub const CURRENT_VERSION: &str = "Current version";
pub const OTHER_VERSION: &str = "Other version";
pub const DELETED: &str = "Deleted";
pub const MISSING: &str = "Missing";
pub const KEEPS_REVOCATION: &str =
    "The revoked key stays revoked; the other fields come from the other version.";
pub const REVOKED_STAYS: &str =
    "A revoked key stays revoked: the other version's deletion is not applied.";
pub const TYPE_NOT_WRITTEN: &str = "This version of Shell++ cannot write this kind of item.";
pub const CURRENT_UNREADABLE: &str =
    "This device cannot read the current version, so the other one does not replace it.";
pub const ONLY_ORDER_DIFFERS: &str = "The versions differ only in their place in the list.";
pub const SAME_VALUES: &str = "The versions hold the same values.";
pub const VERSION_THIS_DEVICE: &str = "This device";
pub const DEVICE: &str = "Device";
pub const UNKNOWN_DEVICE: &str = "Unknown device";
pub const KEEP_THIS: &str = "Keep this";
pub const KEEP_OTHER: &str = "Keep other";
pub const CONFLICT_BADGE: &str = "Conflict";
pub const CONFLICT_TOOLTIP: &str = "Sync kept another version of this item. Click to compare.";
pub const DUPLICATES: &str = "Possible duplicates";
pub const DUPLICATES_HINT: &str = "Items created on two devices that look the same.";
pub const KEEP_BOTH: &str = "Keep both";
pub const COMPARE: &str = "Compare…";
pub const DUPLICATES_COMPARE_HINT: &str = "Keep the one on the left or on the right: its links move to it and the other goes to Recently deleted. Or keep both.";
pub const KEEP_LEFT: &str = "Keep left";
pub const KEEP_RIGHT: &str = "Keep right";
/// The one merge of two copies of a key (UI 13.7 and 7.2).
pub const MERGE_KEYS: &str = "Merge";
pub const KEYS_MERGE_NOTE: &str = "Merge keeps the older key, with the other's certificate and passphrase where it has none. Its links move to it.";
pub const CREATED: &str = "Created";
pub const CHANGED: &str = "Changed";
pub const HINT_REVIEW: &str = "Review";

/// A record type as a list names it: `known_host` is "Known host".
pub fn type_label(type_name: &str) -> String {
    let text = type_name.replace('_', " ");
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The hint of a screen (UI 13.7): "2 hosts may be duplicates", or
/// "2 hosts and 2 groups may be duplicates", from the records in pairs
/// per type.
pub fn may_be_duplicates(counts: &[(&str, usize)]) -> String {
    let parts: Vec<String> = counts
        .iter()
        .map(|(type_name, count)| {
            let noun = type_name.replace('_', " ");
            if *count == 1 {
                format!("{count} {noun}")
            } else {
                format!("{count} {noun}s")
            }
        })
        .collect();
    format!("{} may be duplicates", parts.join(" and "))
}
pub const BROUGHT_BACK: &str = "Brought back";
pub const BROUGHT_BACK_HINT: &str =
    "Deleted elsewhere long ago, but still here: they were restored to the server.";
pub const KEEP: &str = "Keep";
pub const DELETE_AGAIN: &str = "Delete again";
pub const NOTHING_TO_REVIEW: &str = "Nothing to review.";

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

pub const CHANGE_PASSWORD: &str = "Change master password…";
pub const CHANGE_PASSWORD_TITLE: &str = "Change master password";
pub const CHANGE_PASSWORD_RUN: &str = "Change password";
pub const CURRENT_PASSWORD: &str = "Current master password";
pub const NEW_PASSWORD: &str = "New master password";
pub const REPEAT_PASSWORD: &str = "Repeat the new password";
pub const COMPROMISED: &str = "My old password may be compromised";
pub const COMPROMISED_HINT: &str = "Also gives your personal vault a new key, so the old password stops opening current data on the server and in its backups.";
pub const PASSWORDS_DIFFER: &str = "The new passwords do not match.";
pub const PASSWORD_SHORT: &str = "Use at least 8 characters.";
pub const PASSWORD_CHANGED: &str =
    "The master password is changed. Other devices sign in again with it.";
pub const PASSWORD_CHANGED_ROTATED: &str = "The master password is changed and your personal vault has a new key. Other devices sign in again with the new password.";
pub const LOST_QUESTION: &str = "Was this device lost or stolen?";
pub const REVOKE_ONLY: &str = "Revoke only";
pub const LOST_OR_STOLEN: &str = "Lost or stolen";
pub const LOST_TEXT: &str = "The device is signed out. Nothing is wiped on it: if it was unlocked, change the passwords of your important hosts too. A new master password and a new vault key stop the old ones from opening your data.";

pub const RECOVERY_TEXT: &str = "Enter your account's recovery key and a new master password. The new password also becomes the account password; your other devices sign in again with it.";
pub const RECOVERY_KEY: &str = "Recovery key";
pub const RECOVERY_TOTP: &str = "Two-step sign-in code (only if it is on)";
pub const RECOVERY_RUN: &str = "Restore";
pub const RECOVERY_KEY_WRONG: &str = "This recovery key does not open this device's data.";
pub const RECOVERY_REFUSED: &str = "The server did not accept the recovery: the email may have changed on another device, or the key may be wrong.";
pub const RECOVER_LOCAL: &str = "Recover only this device";
pub const RECOVER_LOCAL_HINT: &str = "You can recover the data on this device only: the new password then opens the local copy, and this device is signed out of the account until you sign in with the account password or recover through the server.";
pub const RESTORE_FROM_SYNC_BOUND_HINT: &str = "With the recovery key of your account.";
