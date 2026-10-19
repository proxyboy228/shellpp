// Generates the app manifest: every command gets its own `allow-<command>`
// permission, and `capabilities/default.json` grants exactly the stage-1 list
// (docs/architecture.md, section 3.4; docs/security.md, section 13.2).
//
// Keep COMMANDS in the same order as the `generate_handler!` list in
// src/lib.rs and the permissions in capabilities/default.json.
// `e2e_mock` always gets a permission, but only `capabilities/e2e.json`
// grants it, and that capability is enabled only by the e2e build
// (scripts/e2e-build.mjs) together with the `e2e-mocks` feature.

const COMMANDS: &[&str] = &[
    // app
    "app_info",
    "state_snapshot",
    "window_open",
    "activity",
    "app_exit",
    "external_open_url",
    // lock
    "lock_status",
    "first_run_setup",
    "unlock",
    "lock",
    "recovery_unlock",
    "password_change",
    "recovery_key_show",
    "recovery_key_reissue",
    "remember_device_set",
    // records
    "records_query",
    "record_get",
    "record_save",
    "record_delete",
    "record_restore",
    "record_move",
    "record_duplicate",
    "secret_reveal",
    "secret_copy",
    // settings
    "settings_get",
    "settings_set",
    // notifications
    "notifications_list",
    "notifications_mark_read",
    // clipboard
    "clipboard_write_text",
    "clipboard_read_text",
    // e2e only (registered with the e2e-mocks feature)
    "e2e_mock",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
