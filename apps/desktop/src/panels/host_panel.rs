//! The host panel (UI 3): view, create and edit a host, saved automatically.
//!
//! Skeleton status: Address and Label only, with the draft rule and
//! autosave. TODO(host-panel): the blocks of UI 3.1 in order (Vault, Address,
//! General: Label, Parent Group, Tags, Color, Backspace, Notes; Share, SSH,
//! Add Telnet, Connect at the bottom), the "…" menu (3.3), secrets.

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, v_flex};
use serde_json::{Map, Value};
use shellpp_core::CoreError;
use uuid::Uuid;

use super::autosave::Autosave;
use super::{PanelEvent, field, header};
use crate::i18n;
use crate::state::AppState;
use crate::theme::metrics;

pub struct HostPanel {
    state: Entity<AppState>,
    /// `None` while the new host is a draft.
    record_id: Option<Uuid>,
    address: Entity<InputState>,
    label: Entity<InputState>,
    address_error: Option<SharedString>,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for HostPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostPanel").finish_non_exhaustive()
    }
}

impl EventEmitter<PanelEvent> for HostPanel {}

impl HostPanel {
    /// A panel for an existing host, or an empty one (`None`) with the focus
    /// in Address.
    pub fn new(
        state: Entity<AppState>,
        record_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = record_id.and_then(|id| state.read(cx).host(id).cloned());
        let record_id = host.as_ref().map(|host| host.record_id);
        let address_value = host.as_ref().map(|host| host.address.clone());
        let label_value = host.as_ref().and_then(|host| host.label.clone());
        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::panels::ADDRESS_PLACEHOLDER)
                .default_value(address_value.unwrap_or_default())
        });
        let label =
            cx.new(|cx| InputState::new(window, cx).default_value(label_value.unwrap_or_default()));
        let mut subscriptions = Vec::new();
        for input in [&address, &label] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        this.autosave.schedule(cx, |this, cx| this.save_now(cx));
                    }
                },
            ));
        }
        if record_id.is_none() {
            address.update(cx, |input, cx| input.focus(window, cx));
        }
        Self {
            state,
            record_id,
            address,
            label,
            address_error: None,
            autosave: Autosave::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The host shown, once it exists.
    pub fn record_id(&self) -> Option<Uuid> {
        self.record_id
    }

    /// Whether an edit is waiting for the autosave delay.
    pub fn has_pending_save(&self) -> bool {
        self.autosave.is_pending()
    }

    /// Saves a pending edit now (panel closing, another host opening).
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if self.autosave.is_pending() {
            self.save_now(cx);
        }
        self.autosave.cancel();
    }

    /// Applies the fields to the stored data and saves (creates the host once
    /// the address is filled in).
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.autosave.take_pending();
        let address = self.address.read(cx).value().trim().to_owned();
        let label = self.label.read(cx).value().trim().to_owned();
        if self.record_id.is_none() && address.is_empty() {
            // Still a draft (UI 1.3): nothing is written.
            return;
        }
        let mut data: Map<String, Value> = self
            .record_id
            .and_then(|id| self.state.read(cx).host(id).map(|host| host.data_map()))
            .unwrap_or_default();
        data.insert("address".to_owned(), Value::from(address));
        if label.is_empty() {
            data.remove("label");
        } else {
            data.insert("label".to_owned(), Value::from(label));
        }
        let record_id = self.record_id;
        let result = self
            .state
            .update(cx, |state, cx| state.save_host(record_id, data, cx));
        match result {
            Ok(host) => {
                self.record_id = Some(host.record_id);
                self.address_error = None;
            }
            Err(CoreError::Validation(fields)) => {
                tracing::info!(fields = fields.len(), "host not saved: validation");
                self.address_error = Some(i18n::panels::INVALID_VALUE.into());
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "host not saved");
                cx.emit(PanelEvent::Toast(i18n::lock::FAILED.into()));
            }
        }
        cx.notify();
    }
}

impl Render for HostPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = if self.record_id.is_some() {
            i18n::panels::HOST_DETAILS
        } else {
            i18n::panels::NEW_HOST
        };
        let close =
            Button::new("panel-close").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.flush(cx);
                cx.emit(PanelEvent::Close);
            }));
        v_flex()
            .id("host-panel")
            .test_support()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(header(title, close, cx))
            .child(
                v_flex()
                    .id("host-panel-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(metrics::SPACE_4)
                    .gap(metrics::SPACE_4)
                    .child(field(
                        i18n::panels::ADDRESS,
                        Input::new(&self.address).id("host-address"),
                        self.address_error.clone(),
                        cx,
                    ))
                    .child(field(
                        i18n::panels::LABEL,
                        Input::new(&self.label).id("host-label"),
                        None,
                        cx,
                    )),
            )
            .child(
                div().p(metrics::SPACE_4).child(
                    // TODO(host-panel): connect once sessions exist; the one
                    // accent-filled button of the screen while the panel is open.
                    Button::new("host-connect")
                        .primary()
                        .w_full()
                        .label(i18n::panels::CONNECT)
                        .disabled(self.record_id.is_none()),
                ),
            )
    }
}
