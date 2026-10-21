//! The key panel (UI 7.3) of a saved key, in the Keychain's details panel.
//!
//! Blocks: Vault; Label (required, saved as in UI 1.3: after the autosave
//! delay, an invalid value outlined with its reason and not saved); Private
//! key; Public key; Key export (Export to host, shown disabled, D30, D33).
//!
//! The mandatory security requirements of 7.3:
//!
//! - the private key is hidden by default: the panel shows its type, its
//!   SHA256 fingerprint and a note that it is hidden;
//! - only the explicit Show button reads it from the core; the text lives
//!   in this panel's [`SecretString`] while it is shown and is dropped (and
//!   wiped) on Hide, when the panel closes or shows another key, when the
//!   window leaves the Keychain and on lock ([`KeyPanel::hide`]);
//! - Copy private key goes through the core (`secret_copy`), which writes
//!   the OS clipboard as a sensitive value and clears it after 30 seconds if
//!   it still holds the key (docs/security.md, section 9.4); the text never
//!   passes through this panel;
//! - an unencrypted key carries the hint "Not protected by a passphrase".

use gpui::prelude::*;
use gpui::{ClipboardItem, Context, Entity, IntoElement, SharedString, Subscription, Window};
use gpui_kit::component::input::{InputEvent, InputState};
use secrecy::{ExposeSecret as _, SecretString};
use shellpp_core::service::dto::KeyView;
use shellpp_core::{CoreError, ValidationReason};
use uuid::Uuid;

use super::{KeychainEvent, strings};
use crate::panels::autosave::Autosave;
use crate::state::AppState;

/// Longest label, in characters (`string(256)`, docs/data-model.md 9.1).
const MAX_LABEL_CHARS: usize = 256;

/// The key panel of one key. A new panel is made for each key shown, so
/// nothing of one key stays around when another is shown.
pub struct KeyPanel {
    pub(super) state: Entity<AppState>,
    pub(super) view: KeyView,
    pub(super) label: Entity<InputState>,
    /// Why the typed label is not saved (`required`, `too_long`).
    pub(super) label_error: Option<ValidationReason>,
    autosave: Autosave,
    /// The one-line public key (read once; not a secret).
    pub(super) public_key: Option<String>,
    /// The private key text while it is shown (Show), else `None`.
    revealed: Option<SecretString>,
    /// The last Show could not read the key.
    pub(super) reveal_failed: bool,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for KeyPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPanel")
            .field("key", &self.view.record_id)
            .field("revealed", &self.revealed.is_some())
            .finish_non_exhaustive()
    }
}

impl KeyPanel {
    pub fn new(
        state: Entity<AppState>,
        view: KeyView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(strings::FIELD_LABEL)
                .default_value(view.label.clone())
        });
        let subscription = cx.subscribe_in(
            &label,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.label_changed(cx),
                InputEvent::Blur => this.flush(cx),
                InputEvent::PressEnter { .. } | InputEvent::Focus => cx.notify(),
            },
        );
        let public_key = state
            .read(cx)
            .key_public_key(view.record_id)
            .map_err(|error| tracing::info!(code = error.code(), "public key not read"))
            .ok();
        Self {
            state,
            view,
            label,
            label_error: None,
            autosave: Autosave::default(),
            public_key,
            revealed: None,
            reveal_failed: false,
            _subscriptions: vec![subscription],
        }
    }

    // -- Reading ------------------------------------------------------------------

    pub fn key_id(&self) -> Uuid {
        self.view.record_id
    }

    /// Remove in the "…" menu (D50): asks first, naming the hosts, groups
    /// and identities that use the key; the screen removes it.
    pub fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key_id = self.view.record_id;
        let users = match self.state.read(cx).key_users(key_id) {
            Ok(users) => users,
            Err(error) => {
                tracing::warn!(code = error.code(), "users of a key not read");
                crate::shell::show_toast(strings::REMOVE_FAILED, window, cx);
                return;
            }
        };
        let panel = cx.entity().downgrade();
        super::view::open_remove_confirmation(
            &self.view.label,
            &users,
            window,
            cx,
            move |_, cx| {
                let _ = panel.update(cx, |_, cx| cx.emit(KeychainEvent::Remove { key_id }));
            },
        );
    }

    pub fn view(&self) -> &KeyView {
        &self.view
    }

    /// Whether the private key is shown.
    pub fn is_revealed(&self) -> bool {
        self.revealed.is_some()
    }

    pub fn label_error(&self) -> Option<ValidationReason> {
        self.label_error
    }

    pub fn public_key(&self) -> Option<&str> {
        self.public_key.as_deref()
    }

    /// Whether a label edit waits for the autosave.
    pub fn is_saving(&self) -> bool {
        self.autosave.is_pending()
    }

    // -- Commands ------------------------------------------------------------------

    /// The key's view after a reload (its label may have changed elsewhere;
    /// the typed label stays).
    pub fn set_view(&mut self, view: KeyView, cx: &mut Context<Self>) {
        if view.record_id == self.view.record_id {
            self.view = view;
            cx.notify();
        }
    }

    /// Show (UI 7.3): reads the private key from the core.
    pub fn reveal(&mut self, cx: &mut Context<Self>) {
        match self.state.read(cx).reveal_private_key(self.view.record_id) {
            Ok(text) => {
                self.revealed = Some(text);
                self.reveal_failed = false;
            }
            Err(error) => {
                tracing::info!(code = error.code(), "private key not revealed");
                self.revealed = None;
                self.reveal_failed = true;
            }
        }
        cx.notify();
    }

    /// Hide, and every way of leaving the panel: drops the shown text.
    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if self.revealed.take().is_some() {
            cx.notify();
        }
    }

    /// Copy private key (UI 7.3): the core copies it and clears the
    /// clipboard after 30 seconds.
    pub fn copy_private_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = match self.state.read(cx).copy_private_key(self.view.record_id) {
            Ok(()) => strings::PRIVATE_KEY_COPIED,
            Err(error) => {
                tracing::info!(code = error.code(), "private key not copied");
                strings::PRIVATE_KEY_NOT_COPIED
            }
        };
        crate::shell::show_toast(message, window, cx);
    }

    /// Copy public key: an ordinary copy (a public key is not a secret,
    /// docs/security.md, section 9.4).
    pub fn copy_public_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(line) = &self.public_key {
            cx.write_to_clipboard(ClipboardItem::new_string(line.clone()));
            crate::shell::show_toast(strings::PUBLIC_KEY_COPIED, window, cx);
        }
    }

    /// Checks the typed label and schedules its save (UI 1.3).
    fn label_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.label.read(cx).value().to_string();
        self.label_error = check_label(&text).err();
        if self.label_error.is_some() {
            // Not saved; the last valid label stays in effect.
            self.autosave.cancel();
        } else {
            self.autosave
                .schedule(cx, |panel: &mut Self, cx| panel.save_label(cx));
        }
        cx.notify();
    }

    /// Saves a label edit that still waits for the autosave.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if self.autosave.is_pending() {
            self.autosave.cancel();
            self.save_label(cx);
        }
    }

    fn save_label(&mut self, cx: &mut Context<Self>) {
        self.autosave.take_pending();
        let text = self.label.read(cx).value().to_string();
        if check_label(&text).is_err() || text.trim() == self.view.label {
            return;
        }
        let id = self.view.record_id;
        let saved = self
            .state
            .update(cx, |state, cx| state.rename_key(id, &text, cx));
        match saved {
            Ok(view) => {
                self.view = view;
                self.label_error = None;
            }
            Err(CoreError::Validation(fields)) => {
                self.label_error = fields
                    .into_iter()
                    .find(|error| error.field == "label")
                    .map(|error| error.reason);
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "key label not saved");
            }
        }
        cx.notify();
    }

    /// The lock: drops the shown key and a label edit that can no longer
    /// be saved.
    pub fn discard(&mut self, cx: &mut Context<Self>) {
        self.autosave.cancel();
        self.hide(cx);
    }

    /// Asks the screen to close the panel.
    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        self.flush(cx);
        self.hide(cx);
        cx.emit(KeychainEvent::Close);
    }

    /// The private key text as shown (one string per frame; the source stays
    /// in the panel's `SecretString`).
    pub(super) fn revealed_text(&self) -> Option<SharedString> {
        self.revealed
            .as_ref()
            .map(|text| SharedString::from(text.expose_secret().to_owned()))
    }
}

/// The label rule of a key (UI 7.3: required; at most 256 characters).
pub fn check_label(text: &str) -> Result<(), ValidationReason> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Err(ValidationReason::Required)
    } else if trimmed.chars().count() > MAX_LABEL_CHARS {
        Err(ValidationReason::TooLong)
    } else {
        Ok(())
    }
}

/// The panel's blocks are drawn in `view.rs` (`render_key_panel`).
impl Render for KeyPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_key_panel(window, cx)
    }
}
