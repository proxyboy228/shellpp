//! Headless tests of the recovery key and the password change (UI 13.4,
//! 12.2 "Lost or stolen", 13.5 step 4; D173): Show and Reissue with "I
//! saved the recovery key", the option "My old password may be
//! compromised" on by default, and what a refused change tells the user.
//! Temporary profiles and the in-memory secret store only; a sync account
//! is shown by setting the status by hand (the core stays in local mode, so
//! Show and Reissue run locally).

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, SharedString, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use secrecy::{ExposeSecret as _, SecretString};

use super::keys::{PasswordChangeDialog, PasswordStep, RecoveryKeyDialog, RecoveryStep};
use super::{ConnectStep, RecoveryKeyMode, strings};
use crate::app::{self, RootView};
use crate::state::AppState;
use crate::testing::TestProfile;

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    _root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext, opened: crate::state::OpenedCore) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
        });
        let state = cx.new(|cx| AppState::new(opened, cx));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1280.), px(800.)),
            ))),
            ..Default::default()
        };
        let (handle, root) = cx
            .update(|cx| app::open_window_with(state.clone(), options, cx))
            .expect("test window");
        let ui = Self {
            state,
            handle,
            _root: root,
        };
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
        })
        .unwrap()
    }

    fn click(&self, id: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.settle(cx);
    }

    fn type_text(&self, id: &str, text: &str, cx: &mut TestAppContext) {
        self.click(id, cx);
        let text = text.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.input(&text, cx))
            .unwrap();
        self.settle(cx);
    }

    /// Waits for a background step (Argon2id) to show `id`.
    fn wait_for(&self, id: &str, cx: &mut TestAppContext) -> bool {
        for _ in 0..100 {
            if self.visible(id, cx) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            self.settle(cx);
        }
        false
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn bind(&self, cx: &mut TestAppContext) {
        cx.update(|cx| {
            self.state.update(cx, |state, cx| {
                state.set_sync_status(
                    shellpp_core::service::SyncStatus {
                        bound: true,
                        signed_in: true,
                        server_url: Some("https://sync.example.com".into()),
                        email: Some("user@example.com".into()),
                        ..Default::default()
                    },
                    cx,
                )
            })
        });
        self.settle(cx);
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

fn open_recovery(
    ui: &Ui,
    mode: RecoveryKeyMode,
    cx: &mut TestAppContext,
) -> Entity<RecoveryKeyDialog> {
    let state = ui.state.clone();
    let dialog = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_recovery_key(state, mode, window, cx)
        })
        .unwrap();
    ui.settle(cx);
    dialog
}

fn open_password(
    ui: &Ui,
    lost: Option<uuid::Uuid>,
    cx: &mut TestAppContext,
) -> Entity<PasswordChangeDialog> {
    let state = ui.state.clone();
    let dialog = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_password_change(state, lost, window, cx)
        })
        .unwrap();
    ui.settle(cx);
    dialog
}

/// The key text of the open recovery key dialog.
fn shown_key(dialog: &Entity<RecoveryKeyDialog>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| match &dialog.read(cx).step {
        RecoveryStep::Key(key, _) => key.expose_secret().to_owned(),
        other => panic!("no key shown: {other:?}"),
    })
}

/// Run 8, agent C, finding 2 (REPORT-C, section 7): the recovery key was
/// never shown, while "Restore from sync" asks for it. With a sync account
/// Settings ▸ Security has Show and Reissue (UI 13.4); in local mode the
/// item is hidden (D29).
#[gpui::test]
fn security_offers_show_and_reissue_only_with_an_account(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.press("ctrl-,", cx);
    assert!(ui.visible("settings-security", cx));
    assert!(
        !ui.visible("settings-recovery-key", cx),
        "local mode never shows the key"
    );
    ui.bind(cx);
    // The section scrolls: down to the recovery key block.
    for _ in 0..20 {
        if ui.visible("settings-recovery-reissue", cx) {
            break;
        }
        cx.update_window(ui.handle, |_, window, cx| {
            window.scroll(
                "settings-section",
                gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-200.))),
                cx,
            )
        })
        .unwrap();
        ui.settle(cx);
    }
    assert!(ui.visible("settings-recovery-show", cx));
    assert!(ui.visible("settings-recovery-reissue", cx));
    ui.finish(cx);
}

/// Show: after the master password, the key the vault holds, with Copy.
#[gpui::test]
fn show_gives_the_key_after_the_password(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let password = profile.password().clone();
    let ui = Ui::open(cx, opened);
    ui.bind(cx);
    let dialog = open_recovery(&ui, RecoveryKeyMode::Show, cx);
    assert!(
        !ui.visible("sync-key-text", cx),
        "nothing before the password"
    );
    ui.type_text("sync-key-password", password.expose_secret(), cx);
    ui.click("sync-key-next", cx);
    assert!(ui.wait_for("sync-key-text", cx));
    let held = ui
        .core(cx)
        .recovery_key_show(password.clone())
        .expect("held");
    let shown = shown_key(&dialog, cx);
    assert_eq!(shown, held.recovery_key.expose_secret());
    ui.click("sync-key-copy", cx);
    let copied = profile
        .memory()
        .and_then(|memory| memory.clipboard.text())
        .expect("copied");
    assert_eq!(copied, shown, "Copy puts the key on the clipboard");
    assert!(ui.visible("sync-key-close", cx));
    ui.finish(cx);
}

/// Reissue (SP 5.3, step 4): the new key is shown first; "Reissue" stays
/// off until "I saved the recovery key" is ticked, and only then does the
/// vault take the new key.
#[gpui::test]
fn reissue_shows_the_new_key_and_needs_i_saved_it(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let password = profile.password().clone();
    let ui = Ui::open(cx, opened);
    ui.bind(cx);
    let core = ui.core(cx);
    let before = core.recovery_key_show(password.clone()).expect("held");
    let dialog = open_recovery(&ui, RecoveryKeyMode::Reissue, cx);
    ui.type_text("sync-key-password", password.expose_secret(), cx);
    ui.click("sync-key-next", cx);
    assert!(ui.wait_for("sync-key-saved", cx));
    let shown = shown_key(&dialog, cx);
    assert_ne!(shown, before.recovery_key.expose_secret());
    let still = core.recovery_key_show(password.clone()).expect("held");
    assert_eq!(
        still.recovery_key.expose_secret(),
        before.recovery_key.expose_secret(),
        "nothing changes before the confirmation"
    );
    ui.click("sync-key-reissue", cx);
    assert!(
        !ui.visible("sync-key-close", cx),
        "Reissue does nothing until the key is saved"
    );
    ui.click("sync-key-saved", cx);
    ui.click("sync-key-reissue", cx);
    assert!(ui.wait_for("sync-key-close", cx));
    let after = core.recovery_key_show(password).expect("held");
    assert_eq!(after.recovery_key.expose_secret(), shown);
    ui.finish(cx);
}

/// SP 5.3, step 7: a reissue whose prepared key is no longer the pending
/// one (another window prepared a newer one) is refused, and the dialog
/// says the key shown here is not valid.
#[gpui::test]
fn a_refused_reissue_says_the_key_is_not_valid(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let password = profile.password().clone();
    let ui = Ui::open(cx, opened);
    ui.bind(cx);
    let core = ui.core(cx);
    let before = core.recovery_key_show(password.clone()).expect("held");
    let dialog = open_recovery(&ui, RecoveryKeyMode::Reissue, cx);
    ui.type_text("sync-key-password", password.expose_secret(), cx);
    ui.click("sync-key-next", cx);
    assert!(ui.wait_for("sync-key-saved", cx));
    // Another window prepares a newer change, which replaces this one.
    let other = core
        .recovery_key_reissue_prepare(password.clone())
        .expect("other");
    ui.click("sync-key-saved", cx);
    ui.click("sync-key-reissue", cx);
    assert!(ui.wait_for("sync-key-failed", cx));
    let text = cx.update(|cx| {
        matches!(dialog.read(cx).step, RecoveryStep::Failed(text) if text == strings::KEY_NOT_ACCEPTED)
    });
    assert!(text, "the user is told the key shown is not valid");
    core.key_change_cancel(other.id);
    let after = core.recovery_key_show(password).expect("held");
    assert_eq!(
        after.recovery_key.expose_secret(),
        before.recovery_key.expose_secret()
    );
    ui.finish(cx);
}

/// D173: "My old password may be compromised" is on by default; in local
/// mode the change rotates and replaces the recovery key without showing
/// it, and the new password opens the vault.
#[gpui::test]
fn the_compromised_option_is_on_by_default(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let old = profile.password().clone();
    let ui = Ui::open(cx, opened);
    let core = ui.core(cx);
    let before = core.recovery_key_show(old.clone()).expect("held");
    let dialog = open_password(&ui, None, cx);
    let checked = cx.update(|cx| {
        let dialog = dialog.read(cx);
        dialog.compromised && !dialog.fixed
    });
    assert!(checked, "on by default, and it can be turned off");
    let new = format!("{}-new", old.expose_secret());
    ui.type_text("sync-current-password", old.expose_secret(), cx);
    ui.type_text("sync-new-password", &new, cx);
    ui.type_text("sync-repeat-password", &new, cx);
    ui.click("sync-password-run", cx);
    assert!(ui.wait_for("sync-password-close", cx));
    assert!(
        !ui.visible("sync-key-text", cx),
        "local mode shows no recovery key"
    );
    let new = SecretString::from(new);
    let after = core.recovery_key_show(new.clone()).expect("held");
    assert_ne!(
        after.recovery_key.expose_secret(),
        before.recovery_key.expose_secret(),
        "the record holds a new key"
    );
    core.lock().unwrap();
    assert!(core.unlock(old).is_err());
    core.unlock(new).expect("the new password opens");
    ui.finish(cx);
}

/// "Lost or stolen" (UI 12.2): the option is on and fixed, and the dialog
/// says that saving revokes the device and Cancel only revokes it.
#[gpui::test]
fn lost_or_stolen_fixes_the_option(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    ui.bind(cx);
    let dialog = open_password(&ui, Some(uuid::Uuid::now_v7()), cx);
    assert!(ui.visible("sync-compromised", cx));
    assert!(
        ui.visible("sync-lost-text", cx),
        "the dialog says what it does"
    );
    let fixed = cx.update(|cx| {
        let dialog = dialog.read(cx);
        dialog.compromised && dialog.fixed && matches!(dialog.step, PasswordStep::Form)
    });
    assert!(fixed);
    ui.click("sync-compromised", cx);
    let still = cx.update(|cx| {
        let dialog = dialog.read(cx);
        dialog.compromised || dialog.fixed
    });
    assert!(still, "the option cannot be turned off");
    ui.finish(cx);
}

/// UI 13.5, step 4: after registration the wizard shows the account's key;
/// "Finish" stays off until "I saved the recovery key" is ticked.
#[gpui::test]
fn the_wizard_shows_the_key_after_registration(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new().into_unlocked());
    let state = ui.state.clone();
    let key = generated_key();
    let dialog = cx
        .update_window(ui.handle, move |_, window, cx| {
            let dialog = super::open_connect(state, false, window, cx);
            dialog.update(cx, |dialog, cx| {
                dialog.recovery_key = Some(key);
                dialog.step = ConnectStep::RecoveryKey;
                cx.notify();
            });
            dialog
        })
        .unwrap();
    ui.settle(cx);
    assert!(ui.visible("sync-registered-key", cx));
    assert!(ui.visible("sync-key-text", cx));
    ui.click("sync-finish", cx);
    assert!(
        ui.visible("sync-registered-key", cx),
        "Finish does nothing until the key is saved"
    );
    ui.click("sync-key-saved", cx);
    assert!(cx.update(|cx| dialog.read(cx).key_saved));
    ui.click("sync-finish", cx);
    assert!(!ui.visible("sync-registered-key", cx), "the wizard closed");
    ui.finish(cx);
}

fn generated_key() -> SecretString {
    let key = shellpp_core::crypto::Key32::generate().expect("random");
    shellpp_core::crypto::encode_recovery_key(&key)
}

/// Closing the Reissue dialog on its key step (here as a lock closes it)
/// drops the prepared key: it can no longer be confirmed or copied.
#[gpui::test]
fn closing_the_dialog_drops_the_prepared_key(cx: &mut TestAppContext) {
    let mut profile = TestProfile::new();
    let opened = profile.open_unlocked();
    let password = profile.password().clone();
    let ui = Ui::open(cx, opened);
    ui.bind(cx);
    let dialog = open_recovery(&ui, RecoveryKeyMode::Reissue, cx);
    ui.type_text("sync-key-password", password.expose_secret(), cx);
    ui.click("sync-key-next", cx);
    assert!(ui.wait_for("sync-key-saved", cx));
    let id = cx.update(|cx| match dialog.read(cx).step {
        RecoveryStep::Key(_, Some(id)) => id,
        _ => panic!("no prepared key"),
    });
    drop(dialog);
    // As a lock closes it (`close_overlays`).
    cx.update_window(ui.handle, |_, window, cx| window.close_all_dialogs(cx))
        .unwrap();
    ui.settle(cx);
    assert!(!ui.visible("sync-recovery-key", cx), "closed");
    let core = ui.core(cx);
    assert!(
        core.secret_copy(shellpp_core::service::dto::SecretCopyTarget::PendingRecoveryKey { id })
            .is_err()
    );
    assert!(core.key_change_confirm(id).is_err(), "nothing to confirm");
    ui.finish(cx);
}

/// The key breaks only between its groups: `RK1` and six groups, then the
/// other five; a group is never split across lines, and the lines put
/// together are the key.
#[test]
fn the_key_breaks_only_between_its_groups() {
    let key = "RK1-K2C5C-68EHX-R5K16-9JKG8-FQBXA-9ATVA-HEDD1-4YANQ-7ZG0A-YA8AM-QH2ER";
    let lines = super::keys::key_lines(key);
    assert_eq!(
        lines,
        vec![
            "RK1-K2C5C-68EHX-R5K16-9JKG8-FQBXA-9ATVA-",
            "HEDD1-4YANQ-7ZG0A-YA8AM-QH2ER",
        ]
    );
    assert_eq!(lines.concat(), key);
}
