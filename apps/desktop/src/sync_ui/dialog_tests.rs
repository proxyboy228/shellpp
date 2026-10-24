//! Headless tests of the sync dialogs (run 10):
//!
//! - "Also delete the local copy" (UI 12.2; D197): the first "Sign out" only
//!   asks again and lists the set-aside copies and backups the server does
//!   not have; nothing is deleted before the second click, and Cancel
//!   deletes nothing;
//! - the field of a step has the focus when the step opens, and Enter in it
//!   runs the step (UI 13.5).

use gpui::{AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, TestAppContext, px, size};
use gpui::{WindowBounds, WindowOptions};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use shellpp_core::service::dto::LockState;

use super::SignOutDialog;
use crate::app::{self, RootView};
use crate::state::AppState;
use crate::testing::TestProfile;

const SET_ASIDE: &str = "shellpp.db.20261020T101500Z.old";

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    _root: Entity<RootView>,
}

impl Ui {
    fn open(cx: &mut TestAppContext, profile: TestProfile) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
        });
        let state = cx.new(|cx| AppState::new(profile.into_unlocked(), cx));
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

    fn visible(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
        })
        .unwrap()
    }

    fn click(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn focused(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> Option<bool> {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.find(id.clone()).focused()
        })
        .unwrap()
    }

    /// Types `text` into the focused field and presses Enter.
    fn type_and_enter(&self, text: &str, cx: &mut TestAppContext) {
        let text = text.to_owned();
        cx.update_window(self.handle, move |_, window, cx| {
            window.input(&text, cx);
            window.press("enter", cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn state_of_core(&self, cx: &mut TestAppContext) -> LockState {
        self.state.read_with(cx, |state, _| {
            state.core().cloned().unwrap().lock_status().state
        })
    }

    fn open_sign_out(&self, cx: &mut TestAppContext) -> Entity<SignOutDialog> {
        let state = self.state.clone();
        let dialog = cx
            .update_window(self.handle, move |_, window, cx| {
                super::open_sign_out(state, window, cx)
            })
            .unwrap();
        self.settle(cx);
        dialog
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

#[gpui::test]
fn deleting_the_local_copy_asks_twice_and_lists_what_the_server_lacks(cx: &mut TestAppContext) {
    let profile = TestProfile::new();
    let data = crate::dirs::AppDirs::under(profile.path().to_path_buf()).data;
    let ui = Ui::open(cx, profile);
    std::fs::write(data.join(SET_ASIDE), b"an older vault").expect("a set-aside copy");

    // The first click asks again; nothing is deleted.
    let dialog = ui.open_sign_out(cx);
    ui.click("sync-delete-local", cx);
    assert!(dialog.read_with(cx, |dialog, _| dialog.delete_local));
    ui.click("sync-sign-out-run", cx);
    assert!(ui.visible("sync-delete-local-confirm", cx));
    assert!(ui.visible("sync-delete-local-files", cx));
    let listed = dialog.read_with(cx, |dialog, _| dialog.confirming.clone().unwrap());
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, SET_ASIDE);
    assert_eq!(ui.state_of_core(cx), LockState::Unlocked);
    assert!(data.join("shellpp.db").is_file() && data.join(SET_ASIDE).is_file());

    // Cancel at the second step deletes nothing.
    ui.click("sync-sign-out-cancel", cx);
    assert!(!ui.visible("sync-delete-local-confirm", cx));
    assert_eq!(ui.state_of_core(cx), LockState::Unlocked);
    assert!(data.join("shellpp.db").is_file() && data.join(SET_ASIDE).is_file());

    // A double-click on "Sign out" does not confirm (review of run 10).
    ui.open_sign_out(cx);
    ui.click("sync-delete-local", cx);
    cx.update_window(ui.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click("sync-sign-out-run", cx);
    })
    .unwrap();
    ui.settle(cx);
    assert!(ui.visible("sync-delete-local-confirm", cx));
    assert_eq!(ui.state_of_core(cx), LockState::Unlocked);
    assert!(data.join("shellpp.db").is_file() && data.join(SET_ASIDE).is_file());

    // Confirmed: the local copy goes, the set-aside copy with it.
    ui.click("sync-delete-local-run", cx);
    assert_eq!(ui.state_of_core(cx), LockState::FirstRun);
    assert!(!data.join(SET_ASIDE).exists());
    ui.finish(cx);
}

#[gpui::test]
fn the_wizard_focuses_each_step_and_enter_runs_it(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new());
    let state = ui.state.clone();
    let wizard = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_connect(state, false, window, cx)
        })
        .unwrap();
    ui.settle(cx);
    assert_eq!(ui.focused("sync-server-address", cx), Some(true));
    // Enter checks the address: plain HTTP to another machine is refused
    // before anything is sent.
    ui.type_and_enter("http://sync.example.com", cx);
    assert_eq!(
        wizard.read_with(cx, |wizard, _| wizard.error),
        Some(super::strings::ERROR_HTTP)
    );
    // The account step: the email first.
    wizard.update(cx, |wizard, cx| {
        wizard.origin = Some("https://sync.example.com".to_owned());
        wizard.step = super::ConnectStep::Account;
        wizard.error = None;
        cx.notify();
    });
    ui.settle(cx);
    assert_eq!(ui.focused("sync-email", cx), Some(true));
    ui.finish(cx);
}

#[gpui::test]
fn the_password_dialogs_focus_their_field_and_enter_runs_them(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, TestProfile::new());
    // Enabling TOTP: the password first; Enter runs the step (refused: this
    // device has no account).
    let state = ui.state.clone();
    let totp = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_totp(state, window, cx)
        })
        .unwrap();
    ui.settle(cx);
    assert_eq!(ui.focused("sync-totp-password", cx), Some(true));
    ui.type_and_enter("not the password", cx);
    assert!(totp.read_with(cx, |dialog, _| dialog.error.is_some()));
    cx.update_window(ui.handle, |_, window, cx| window.close_dialog(cx))
        .unwrap();
    ui.settle(cx);

    // The recovery: the key first; Enter checks the fields.
    let state = ui.state.clone();
    let recovery = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_recovery(state, window, cx)
        })
        .unwrap();
    ui.settle(cx);
    assert_eq!(ui.focused("sync-recovery-key", cx), Some(true));
    ui.type_and_enter("x", cx);
    assert!(recovery.read_with(cx, |dialog, _| dialog.error.is_some()));
    cx.update_window(ui.handle, |_, window, cx| window.close_dialog(cx))
        .unwrap();
    ui.settle(cx);

    // Delete the account: the password first.
    let state = ui.state.clone();
    let delete = cx
        .update_window(ui.handle, move |_, window, cx| {
            super::open_password_action(state, super::PasswordAction::DeleteAccount, window, cx)
        })
        .unwrap();
    ui.settle(cx);
    assert_eq!(ui.focused("sync-action-password", cx), Some(true));
    ui.type_and_enter("not the password", cx);
    assert!(delete.read_with(cx, |dialog, _| dialog.error.is_some()));
    ui.finish(cx);
}
