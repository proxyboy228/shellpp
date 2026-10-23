//! Settings > Terminal, "Script generation (beta)" (UI 9.5; run 8 decision
//! D144): off by default; the address of an OpenAI-compatible API (a
//! hosted service or a local model on this computer), the model, and the
//! API key, which goes into the system's credential store, never into a
//! setting. The snippet form's Generate row appears once it is on with a
//! model.
//!
//! The address and the model are saved as they are typed (UI 12.1); an
//! address that is not `https://` (or `http://` on this computer) shows the
//! reason and is not saved. Nothing here logs the key.

use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Entity, IntoElement, Render, Subscription, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex};
use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_core::service::dto::SettingsSetInput;

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::snippets::generate::{self, DEFAULT_URL, KEY_ENABLED, KEY_MODEL, KEY_URL};
use crate::state::AppState;
use crate::theme::metrics;

/// The block.
pub struct GenerationBlock {
    state: Entity<AppState>,
    enabled: bool,
    url: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    url_invalid: bool,
    /// What the last key save did.
    key_note: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for GenerationBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenerationBlock")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl GenerationBlock {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let values = state
            .read(cx)
            .core()
            .and_then(|core| core.settings_get().ok())
            .unwrap_or_default();
        let text = |key: &str| {
            values
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let enabled = values.get(KEY_ENABLED).and_then(Value::as_str) == Some("on");
        let url_value = text(KEY_URL);
        let model_value = text(KEY_MODEL);
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(DEFAULT_URL)
                .default_value(url_value)
        });
        let model = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(strings::GENERATION_MODEL_PLACEHOLDER)
                .default_value(model_value)
        });
        let key = cx.new(|cx| InputState::new(window, cx).masked(true));
        let subscriptions = vec![
            cx.subscribe(&url, |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let typed = input.read(cx).value().trim().to_owned();
                    this.url_typed(&typed, cx);
                }
            }),
            cx.subscribe(&model, |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let typed = input.read(cx).value().trim().to_owned();
                    let value = if typed.is_empty() {
                        Value::Null
                    } else {
                        json!(typed)
                    };
                    this.write(KEY_MODEL, value, cx);
                }
            }),
        ];
        Self {
            state,
            enabled,
            url,
            model,
            key,
            url_invalid: false,
            key_note: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn url_invalid(&self) -> bool {
        self.url_invalid
    }

    fn write(&mut self, key: &'static str, value: Value, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let mut values = std::collections::BTreeMap::new();
        values.insert(key.to_owned(), value);
        if let Err(error) = core.settings_set(SettingsSetInput {
            values,
            reset: Vec::new(),
        }) {
            tracing::warn!(code = error.code(), setting = key, "setting not saved");
        }
        cx.notify();
    }

    pub fn set_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.enabled = on;
        self.write(KEY_ENABLED, json!(if on { "on" } else { "off" }), cx);
    }

    /// An address is saved when it may be asked (empty: the default).
    fn url_typed(&mut self, typed: &str, cx: &mut Context<Self>) {
        if typed.is_empty() {
            self.url_invalid = false;
            self.write(KEY_URL, Value::Null, cx);
            return;
        }
        self.url_invalid = generate::destination_of(typed).is_err();
        if !self.url_invalid {
            self.write(KEY_URL, json!(typed), cx);
        }
        cx.notify();
    }

    /// Saves the typed API key for the address's host in the system's
    /// credential store, off the UI thread, and empties the field (an empty
    /// field removes the host's saved key).
    pub fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self.url.read(cx).value().trim().to_owned();
        let address = if address.is_empty() {
            DEFAULT_URL.to_owned()
        } else {
            address
        };
        let Ok((host, _)) = generate::destination_of(&address) else {
            self.key_note = Some(strings::GENERATION_KEY_ADDRESS);
            cx.notify();
            return;
        };
        let typed = self.key.read(cx).value().to_string();
        let key = (!typed.is_empty()).then(|| SecretString::from(typed));
        self.key
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        let adding = key.is_some();
        let work = cx.background_spawn(async move { generate::set_api_key(&host, key.as_ref()) });
        cx.spawn(async move |this, cx| {
            let saved = work.await;
            let _ = this.update(cx, |this, cx| {
                this.key_note = Some(match (saved, adding) {
                    (true, true) => strings::GENERATION_KEY_SAVED,
                    (true, false) => strings::GENERATION_KEY_REMOVED,
                    (false, _) => strings::GENERATION_KEY_FAILED,
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// What the last key save did (tests).
    pub fn key_note(&self) -> Option<&'static str> {
        self.key_note
    }

    fn field(
        label: &'static str,
        control: impl IntoElement,
        error: Option<&'static str>,
        cx: &App,
    ) -> impl IntoElement {
        div()
            .w(metrics::rpx(360.))
            .child(widgets::field(label, control, error, cx))
    }
}

impl Render for GenerationBlock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.enabled;
        v_flex()
            .id("settings-generation")
            .test_support()
            .gap(metrics::SPACE_3)
            .child(
                v_flex()
                    .gap(metrics::SPACE_1)
                    .child(
                        h_flex()
                            .justify_between()
                            .gap(metrics::SPACE_3)
                            .child(super::label(strings::GENERATION, cx))
                            .child(
                                Switch::new("settings-generation-on")
                                    .checked(enabled)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.set_enabled(*checked, cx)
                                    })),
                            ),
                    )
                    .child(widgets::caption(strings::GENERATION_HINT, cx)),
            )
            .when(enabled, |this| {
                this.child(Self::field(
                    strings::GENERATION_URL,
                    widgets::text_input(
                        &self.url,
                        "settings-generation-url",
                        self.url_invalid,
                        window,
                        cx,
                    ),
                    self.url_invalid.then_some(strings::GENERATION_URL_INVALID),
                    cx,
                ))
                .child(Self::field(
                    strings::GENERATION_MODEL,
                    widgets::text_input(
                        &self.model,
                        "settings-generation-model",
                        false,
                        window,
                        cx,
                    ),
                    None,
                    cx,
                ))
                .child(Self::field(
                    strings::GENERATION_KEY,
                    h_flex()
                        .w_full()
                        .gap(metrics::SPACE_2)
                        .child(div().flex_1().min_w_0().child(widgets::text_input(
                            &self.key,
                            "settings-generation-key",
                            false,
                            window,
                            cx,
                        )))
                        .child(
                            ControlButton::new(
                                "settings-generation-key-save",
                                ControlKind::Neutral,
                            )
                            .label(strings::GENERATION_KEY_SAVE)
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| this.save_key(window, cx),
                            )),
                        ),
                    None,
                    cx,
                ))
                .child(widgets::caption(
                    self.key_note.unwrap_or(strings::GENERATION_KEY_HINT),
                    cx,
                ))
            })
    }
}
