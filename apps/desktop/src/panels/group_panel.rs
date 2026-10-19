//! The group panel (UI 3.4): view, create and edit a group, saved
//! automatically.
//!
//! Skeleton status: Name only, with the draft rule and autosave.
//! TODO(group-panel): Parent Group, Color, the inheritable SSH block, the
//! "…" menu and the non-empty group removal of UI 3.4.

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use serde_json::{Map, Value};
use shellpp_core::CoreError;
use uuid::Uuid;

use super::autosave::Autosave;
use super::{PanelEvent, field, header};
use crate::i18n;
use crate::state::AppState;
use crate::theme::metrics;

pub struct GroupPanel {
    state: Entity<AppState>,
    record_id: Option<Uuid>,
    name: Entity<InputState>,
    name_error: Option<SharedString>,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for GroupPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupPanel").finish_non_exhaustive()
    }
}

impl EventEmitter<PanelEvent> for GroupPanel {}

impl GroupPanel {
    /// A panel for an existing group, or an empty one (`None`) for a new
    /// group inside the current location, with the focus in Name.
    pub fn new(
        state: Entity<AppState>,
        record_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let group = record_id.and_then(|id| state.read(cx).group(id).cloned());
        let record_id = group.as_ref().map(|group| group.record_id);
        let name_value = group.map(|group| group.name).unwrap_or_default();
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name_value));
        let subscription = cx.subscribe_in(
            &name,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.autosave.schedule(cx, |this, cx| this.save_now(cx));
                }
            },
        );
        if record_id.is_none() {
            name.update(cx, |input, cx| input.focus(window, cx));
        }
        Self {
            state,
            record_id,
            name,
            name_error: None,
            autosave: Autosave::default(),
            _subscriptions: vec![subscription],
        }
    }

    pub fn record_id(&self) -> Option<Uuid> {
        self.record_id
    }

    /// Saves a pending edit now.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if self.autosave.is_pending() {
            self.save_now(cx);
        }
        self.autosave.cancel();
    }

    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.autosave.take_pending();
        let name = self.name.read(cx).value().trim().to_owned();
        if self.record_id.is_none() && name.is_empty() {
            return;
        }
        let state = self.state.read(cx);
        let mut data: Map<String, Value> = match self.record_id {
            Some(id) => state
                .group(id)
                .map(|group| group.data_map())
                .unwrap_or_default(),
            None => {
                // A new group goes into the group the screen shows.
                let mut data = Map::new();
                if let Some(parent) = state.location() {
                    data.insert("parent_id".to_owned(), Value::from(parent.to_string()));
                }
                data
            }
        };
        data.insert("name".to_owned(), Value::from(name));
        let record_id = self.record_id;
        let result = self
            .state
            .update(cx, |state, cx| state.save_group(record_id, data, cx));
        match result {
            Ok(group) => {
                self.record_id = Some(group.record_id);
                self.name_error = None;
            }
            Err(CoreError::Validation(_)) => {
                self.name_error = Some(i18n::panels::INVALID_VALUE.into());
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "group not saved");
                cx.emit(PanelEvent::Toast(i18n::lock::FAILED.into()));
            }
        }
        cx.notify();
    }
}

impl Render for GroupPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = if self.record_id.is_some() {
            i18n::panels::GROUP_DETAILS
        } else {
            i18n::panels::NEW_GROUP
        };
        let close =
            Button::new("panel-close").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.flush(cx);
                cx.emit(PanelEvent::Close);
            }));
        v_flex()
            .id("group-panel")
            .test_support()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(header(title, close, cx))
            .child(
                v_flex()
                    .p(metrics::SPACE_4)
                    .gap(metrics::SPACE_4)
                    .child(field(
                        i18n::panels::NAME,
                        Input::new(&self.name).id("group-name"),
                        self.name_error.clone(),
                        cx,
                    )),
            )
    }
}
