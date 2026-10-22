//! Inline pickers of the panels: Vault, Parent Group (search, create on the
//! fly), Host Chaining (search hosts) and the tag suggestions. A picker opens
//! as a list under its trigger, filters by its search input and reports the
//! chosen value to the owning view.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, Context, Entity, IntoElement, SharedString, Window, div};
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use shellpp_core::model::text;
use uuid::Uuid;

use super::widgets;
use crate::theme::metrics;

/// What a picker row stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickValue {
    /// The personal vault (the only vault for now).
    PersonalVault,
    /// No parent group: the top level.
    TopLevel,
    Group(Uuid),
    Host(Uuid),
    /// An existing tag, with its stored spelling.
    Tag(String),
    /// Create a group (Parent Group) or a tag with the typed name.
    Create(String),
    /// A credential type of the "+" list (UI 3.1).
    Credential(CredentialKind),
    /// A saved identity or key to link.
    Identity(Uuid),
    Key(Uuid),
    /// "New identity": the inline creation form.
    NewIdentity,
    /// A charset label (`utf-8` is the default).
    Charset(String),
    /// A built-in terminal theme slug.
    TerminalTheme(String),
    /// The startup snippet; `None` is "None".
    Snippet(Option<Uuid>),
}

/// The credential types of the host panel's "+" (UI 3.1), in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CredentialKind {
    Identity,
    Key,
    Certificate,
    Fido2,
}

impl CredentialKind {
    /// Every type a host can have linked (stored data is always shown).
    pub const ALL: [Self; 4] = [Self::Identity, Self::Key, Self::Certificate, Self::Fido2];
    /// The types the "+" offers: a FIDO2 key joins with FIDO2 support (D5)
    /// and is hidden until then (UI 3.1, section 0).
    pub const OFFERED: [Self; 3] = [Self::Identity, Self::Key, Self::Certificate];

    /// The id part of elements about this type (`host-cred-key`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Key => "key",
            Self::Certificate => "certificate",
            Self::Fido2 => "fido2",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Identity => super::strings::CREDENTIAL_IDENTITY,
            Self::Key => super::strings::CREDENTIAL_KEY,
            Self::Certificate => super::strings::CREDENTIAL_CERTIFICATE,
            Self::Fido2 => super::strings::CREDENTIAL_FIDO2,
        }
    }
}

/// One row of a picker.
pub struct PickerRow {
    /// Unique within the picker; becomes the element id `{prefix}-{key}`.
    pub key: String,
    pub value: PickValue,
    pub title: SharedString,
    pub secondary: Option<SharedString>,
    pub checked: bool,
    pub leading: Option<AnyElement>,
}

impl std::fmt::Debug for PickerRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PickerRow")
            .field("key", &self.key)
            .field("checked", &self.checked)
            .finish_non_exhaustive()
    }
}

impl PickerRow {
    pub fn new(key: impl Into<String>, value: PickValue, title: impl Into<SharedString>) -> Self {
        Self {
            key: key.into(),
            value,
            title: title.into(),
            secondary: None,
            checked: false,
            leading: None,
        }
    }

    pub fn secondary(mut self, secondary: impl Into<SharedString>) -> Self {
        self.secondary = Some(secondary.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }
}

/// Case-insensitive "contains" used by every picker search.
pub fn matches(query: &str, text_value: &str) -> bool {
    let query = query.trim();
    query.is_empty() || text::fold(text_value).contains(&text::fold(query))
}

/// The handler a view gives a picker.
pub type OnPick<V> = Rc<dyn Fn(&mut V, PickValue, &mut Window, &mut Context<V>)>;

/// The list of a picker: an optional search field on top, then the rows,
/// then "Nothing found" when no row matches and nothing can be created.
pub fn render_list<V: 'static>(
    prefix: &str,
    search: Option<(&Entity<InputState>, &str)>,
    rows: Vec<PickerRow>,
    window: &Window,
    cx: &mut Context<V>,
    on_pick: OnPick<V>,
) -> gpui::Stateful<gpui::Div> {
    let empty = rows.is_empty();
    let mut items: Vec<AnyElement> = Vec::with_capacity(rows.len());
    for row in rows {
        let PickerRow {
            key,
            value,
            title,
            secondary,
            checked,
            leading,
        } = row;
        let on_pick = on_pick.clone();
        let create = matches!(value, PickValue::Create(_));
        let leading = if create {
            Some(
                Icon::new(IconName::Plus)
                    .small()
                    .text_color(cx.theme().muted_foreground)
                    .into_any_element(),
            )
        } else {
            leading
        };
        items.push(
            widgets::dropdown_item(
                SharedString::from(format!("{prefix}-{key}")),
                leading,
                title,
                secondary,
                checked,
                cx,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                on_pick(this, value.clone(), window, cx);
            }))
            .into_any_element(),
        );
    }
    let search_field = search.map(|(state, id)| {
        div().p(metrics::SPACE_1).child(widgets::text_input(
            state,
            SharedString::from(id.to_owned()),
            false,
            window,
            cx,
        ))
    });
    widgets::dropdown_list(SharedString::from(format!("{prefix}-list")), cx)
        .children(search_field)
        .child(
            gpui_kit::component::v_flex()
                .id(SharedString::from(format!("{prefix}-rows")))
                .w_full()
                .max_h(widgets::PICKER_MAX_HEIGHT)
                .overflow_y_scroll()
                .gap(metrics::rpx(2.))
                .children(items)
                .when(empty, |this| {
                    this.child(
                        div()
                            .px(metrics::SPACE_2)
                            .py(metrics::SPACE_2)
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(cx.theme().muted_foreground)
                            .child(super::strings::NO_MATCHES),
                    )
                }),
        )
}

/// Clears a picker's search text (on open and after a pick).
pub fn clear_search(state: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    state.update(cx, |input, cx| input.set_value("", window, cx));
}
