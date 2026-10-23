//! Duplicates from sync (UI 13.7):
//!
//! - the hint of a screen ("2 hosts may be duplicates" with Review) while
//!   the core lists pairs of the screen's types. It is shown only where the
//!   user may change the vault; in this app that is always the personal
//!   vault, so it shows whenever there are pairs;
//! - the dialog with each pair side by side as in 13.6: both records as
//!   lists of their fields with the device and the time of their last
//!   change, the fields that differ highlighted, secrets masked by the
//!   core, and Keep left / Keep right (a merge into that one; the older
//!   one, on the left, has the focus) and Keep both, one group as wide as
//!   the widest (design-system 6). Two copies of a key have the one Merge
//!   of 7.2 instead of Keep left / Keep right: the key rule decides which
//!   one stays (data-model 18.3).

use std::rc::Rc;

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Rems, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use shellpp_core::CoreError;
use shellpp_core::service::{DuplicateItem, DuplicateSide};
use uuid::Uuid;

use super::{close_footer, error_line, error_text, label, strings, title, when};
use crate::hosts::controls::{self, ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::{AppState, AppStateEvent};
use crate::theme::{self, metrics};

/// Width of the dialog: two records side by side.
const DIALOG_WIDTH: f32 = 720.;
/// Width of a field's name in a record's list.
const FIELD_NAME_WIDTH: f32 = 104.;

/// The types of the Hosts screen's hint.
pub const HOSTS: &[&str] = &["host", "group"];
/// The types of the Known Hosts screen's hint.
pub const KNOWN_HOSTS: &[&str] = &["known_host"];
/// The types of the Snippets screen's hint.
pub const SNIPPETS: &[&str] = &["snippet", "snippet_package"];

/// The pairs a hint or a dialog covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicatesOf {
    /// The pairs of these types (a screen's hint).
    Types(&'static [&'static str]),
    /// One pair (a row of Review): `record_id`, `other_record_id`.
    Pair(Uuid, Uuid),
}

impl DuplicatesOf {
    fn covers(self, item: &DuplicateItem) -> bool {
        match self {
            Self::Types(types) => types.contains(&item.type_name.as_str()),
            Self::Pair(a, b) => item.record_id == a && item.other_record_id == b,
        }
    }
}

/// The pairs of `of`, as the core lists them (none while locked).
fn pairs(state: &Entity<AppState>, of: DuplicatesOf, cx: &App) -> Vec<DuplicateItem> {
    state
        .read(cx)
        .core()
        .and_then(|core| core.sync_duplicates().ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|item| of.covers(item))
        .collect()
}

// ---------------------------------------------------------------------------
// The hint of a screen
// ---------------------------------------------------------------------------

pub struct DuplicatesHint {
    state: Entity<AppState>,
    /// The element id of the hint; its Review button is `<id>-review`.
    id: &'static str,
    types: &'static [&'static str],
    /// The screen's horizontal padding (the hint lines up with its content).
    padding: Rems,
    /// The records in pairs, per type of `types`.
    counts: Vec<(&'static str, usize)>,
    /// The status's pair count the hint was read for.
    seen: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for DuplicatesHint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplicatesHint")
            .field("id", &self.id)
            .field("counts", &self.counts)
            .finish_non_exhaustive()
    }
}

impl DuplicatesHint {
    pub fn new(
        state: Entity<AppState>,
        id: &'static str,
        types: &'static [&'static str],
        padding: Rems,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&state, |this: &mut Self, _, event, cx| match event {
                AppStateEvent::RecordsChanged | AppStateEvent::LockChanged(_) => this.reload(cx),
                AppStateEvent::SelectionChanged
                | AppStateEvent::LocationChanged(_)
                | AppStateEvent::AutoLockDue => {}
            }),
            // A sync or "Keep both" changes the pairs without a record
            // change: the status's count tells.
            cx.observe(&state, |this: &mut Self, state, cx| {
                let seen = state.read(cx).sync_status().map(|status| status.duplicates);
                if seen != this.seen {
                    this.reload(cx);
                }
            }),
        ];
        let mut hint = Self {
            state,
            id,
            types,
            padding,
            counts: Vec::new(),
            seen: None,
            _subscriptions: subscriptions,
        };
        hint.reload(cx);
        hint
    }

    /// The records in pairs (the hint's number; 0 hides it).
    pub fn count(&self) -> usize {
        self.counts.iter().map(|(_, count)| count).sum()
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.seen = self
            .state
            .read(cx)
            .sync_status()
            .map(|status| status.duplicates);
        let items = pairs(&self.state, DuplicatesOf::Types(self.types), cx);
        self.counts = self
            .types
            .iter()
            .map(|type_name| {
                let mut ids: Vec<Uuid> = items
                    .iter()
                    .filter(|item| item.type_name == *type_name)
                    .flat_map(|item| [item.record_id, item.other_record_id])
                    .collect();
                ids.sort();
                ids.dedup();
                (*type_name, ids.len())
            })
            .filter(|(_, count)| *count > 0)
            .collect();
        cx.notify();
    }
}

impl Render for DuplicatesHint {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.counts.is_empty() {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let state = self.state.clone();
        let types = self.types;
        // An info notice (design-system 2.5): `--info-subtle`, the color on
        // the icon only, `--text-primary` text.
        div()
            .flex_none()
            .w_full()
            .px(self.padding)
            .pb(metrics::SPACE_3)
            .child(
                h_flex()
                    .id(self.id)
                    .test_support()
                    .w_full()
                    .items_center()
                    .gap(metrics::SPACE_2)
                    .px(metrics::SPACE_3)
                    .py(metrics::SPACE_1)
                    .rounded(metrics::RADIUS_CONTROL)
                    .bg(tokens.info_subtle.hsla())
                    .child(
                        div()
                            .flex_none()
                            .child(Icon::new(IconName::Info).small().text_color(theme.info)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(metrics::FONT_SIZE_DENSE)
                            .text_color(theme.foreground)
                            .child(strings::may_be_duplicates(&self.counts)),
                    )
                    .child(
                        widgets::text_button(
                            SharedString::from(format!("{}-review", self.id)),
                            strings::HINT_REVIEW,
                            true,
                            cx,
                        )
                        .on_click(move |_, window, cx| {
                            open_duplicates(state.clone(), DuplicatesOf::Types(types), window, cx);
                        }),
                    ),
            )
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// The dialog
// ---------------------------------------------------------------------------

pub struct DuplicatesDialog {
    state: Entity<AppState>,
    of: DuplicatesOf,
    pub items: Vec<DuplicateItem>,
    /// The first pair's Keep left (Merge for two copies of a key): the
    /// older record has the focus.
    focus: FocusHandle,
    /// Runs after each choice (Review reads its lists again).
    on_change: Option<OnChange>,
    error: Option<&'static str>,
}

impl std::fmt::Debug for DuplicatesDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplicatesDialog")
            .field("of", &self.of)
            .field("pairs", &self.items.len())
            .finish_non_exhaustive()
    }
}

/// What runs after each choice of the dialog.
type OnChange = Rc<dyn Fn(&mut App)>;

/// Which record of a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

impl Side {
    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

impl DuplicatesDialog {
    pub fn new(state: Entity<AppState>, of: DuplicatesOf, cx: &mut Context<Self>) -> Self {
        let items = pairs(&state, of, cx);
        Self {
            state,
            of,
            items,
            focus: cx.focus_handle().tab_stop(true),
            on_change: None,
            error: None,
        }
    }

    /// The focus of the first pair's Keep left.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// After a choice: the pairs again; the dialog closes after the last.
    fn act(&mut self, result: Result<(), CoreError>, window: &mut Window, cx: &mut Context<Self>) {
        self.error = result.err().map(|error| error_text(&error));
        self.items = pairs(&self.state, self.of, cx);
        self.state.update(cx, |state, cx| {
            state.refresh_sync_status(cx);
            state.kick_sync(cx);
        });
        if let Some(on_change) = self.on_change.clone() {
            on_change(cx);
        }
        if self.items.is_empty() && self.error.is_none() {
            window.close_dialog(cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Merges the pair into `keep` (UI 13.7): every link moves to it and
    /// the other record goes to Recently deleted.
    fn merge(&mut self, keep: Uuid, other: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let result = core.sync_duplicates_merge(keep, other);
        self.act(result, window, cx);
    }

    fn keep_both(&mut self, item: &DuplicateItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let result = core.sync_duplicates_keep(item.record_id, item.other_record_id);
        self.act(result, window, cx);
    }

    fn render_side(
        &self,
        index: usize,
        side: Side,
        item: &DuplicateItem,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let record: &DuplicateSide = match side {
            Side::Left => &item.left,
            Side::Right => &item.right,
        };
        let rows = item.fields.iter().map(|field| {
            let value = match side {
                Side::Left => &field.left,
                Side::Right => &field.right,
            };
            // A differing field carries `-differs` in its id (tests).
            let id = if field.differs {
                format!("dup-{index}-{}-{}-differs", side.name(), field.path)
            } else {
                format!("dup-{index}-{}-{}", side.name(), field.path)
            };
            h_flex()
                .id(SharedString::from(id))
                .test_support()
                .w_full()
                .gap(metrics::SPACE_2)
                .px(metrics::SPACE_2)
                .py(metrics::SPACE_1)
                .rounded(metrics::RADIUS_CONTROL)
                .when(field.differs, |row| row.bg(tokens.warning_subtle.hsla()))
                .child(
                    div()
                        .flex_none()
                        .w(metrics::rpx(FIELD_NAME_WIDTH))
                        .truncate()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(field.path.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(metrics::FONT_SIZE_DENSE)
                        .text_color(theme.foreground)
                        .child(value.clone()),
                )
        });
        v_flex()
            .id((
                match side {
                    Side::Left => "dup-left",
                    Side::Right => "dup-right",
                },
                index,
            ))
            .test_support()
            .flex_1()
            .min_w_0()
            .gap(metrics::SPACE_1)
            .p(metrics::SPACE_2)
            .rounded(metrics::RADIUS_CARD)
            .border_1()
            .border_color(tokens.border_subtle.hsla())
            .child(
                v_flex()
                    .px(metrics::SPACE_2)
                    .pb(metrics::SPACE_1)
                    .child(
                        div()
                            .truncate()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                            .text_color(theme.popover_foreground)
                            .child(record.title.clone()),
                    )
                    .child(widgets::caption(
                        format!("{} {}", strings::CREATED, when(&record.created_at)),
                        cx,
                    ))
                    .child(
                        widgets::caption(
                            format!(
                                "{} {} · {}",
                                strings::CHANGED,
                                when(&record.modified_at),
                                device_text(record)
                            ),
                            cx,
                        )
                        .id(SharedString::from(format!(
                            "dup-{index}-{}-device",
                            side.name()
                        )))
                        .test_support(),
                    ),
            )
            .children(rows)
    }

    fn render_pair(
        &self,
        index: usize,
        item: &DuplicateItem,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (left, right) = (item.left.record_id, item.right.record_id);
        let pair = item.clone();
        // Two copies of a key: the one Merge of 7.2 (the core keeps the
        // older whichever is named).
        let key = item.type_name == "key";
        let labels: &[&str] = if key {
            &[strings::MERGE_KEYS, strings::KEEP_BOTH]
        } else {
            &[strings::KEEP_LEFT, strings::KEEP_RIGHT, strings::KEEP_BOTH]
        };
        // One group: every button as wide as the widest (design-system 6).
        let width = controls::text_group_width(window, labels);
        let first = |id: &'static str, label: &'static str| {
            ControlButton::new((id, index), ControlKind::Neutral)
                .label(label)
                .min_w(width)
                .when(index == 0, |button| button.focus_handle(self.focus.clone()))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.merge(left, right, window, cx)
                }))
        };
        let buttons = h_flex()
            .w_full()
            .justify_end()
            .gap(metrics::SPACE_2)
            // Room for the focus ring inside the scrolling body.
            .pr(metrics::SPACE_1)
            .pb(metrics::SPACE_1);
        let buttons = if key {
            buttons.child(first("dup-merge", strings::MERGE_KEYS))
        } else {
            buttons
                .child(first("dup-keep-left", strings::KEEP_LEFT))
                .child(
                    ControlButton::new(("dup-keep-right", index), ControlKind::Neutral)
                        .label(strings::KEEP_RIGHT)
                        .min_w(width)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.merge(right, left, window, cx)
                        })),
                )
        };
        v_flex()
            .id(("dup-pair", index))
            .test_support()
            .w_full()
            .gap(metrics::SPACE_2)
            .child(label(
                format!("{} · {}", item.title, strings::type_label(&item.type_name)),
                cx,
            ))
            .when(key, |this| {
                this.child(widgets::caption(strings::KEYS_MERGE_NOTE, cx))
            })
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap(metrics::SPACE_3)
                    .child(self.render_side(index, Side::Left, item, cx))
                    .child(self.render_side(index, Side::Right, item, cx)),
            )
            .child(
                buttons.child(
                    ControlButton::new(("dup-keep-both", index), ControlKind::Neutral)
                        .label(strings::KEEP_BOTH)
                        .min_w(width)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.keep_both(&pair, window, cx)
                        })),
                ),
            )
    }
}

/// The device of a side's last change: this device, its name, or the
/// start of its id (as the conflict comparison names a version's, UI 13.6).
fn device_text(side: &DuplicateSide) -> String {
    if side.this_device {
        return strings::VERSION_THIS_DEVICE.to_owned();
    }
    match &side.device_name {
        Some(name) => name.clone(),
        None => format!(
            "{} {}",
            strings::DEVICE,
            side.device_id.chars().take(8).collect::<String>()
        ),
    }
}

impl Render for DuplicatesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("sync-duplicates")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_4)
            .child(widgets::caption(strings::DUPLICATES_COMPARE_HINT, cx));
        if self.items.is_empty() {
            body = body.child(widgets::caption(strings::NOTHING_TO_REVIEW, cx));
        }
        for (index, item) in self.items.clone().iter().enumerate() {
            if index > 0 {
                body = body.child(widgets::divider(cx));
            }
            body = body.child(self.render_pair(index, item, window, cx));
        }
        body.children(error_line(self.error, cx))
    }
}

/// Opens the pairs of `of` side by side; Keep left of the first pair has
/// the focus.
pub fn open_duplicates(
    state: Entity<AppState>,
    of: DuplicatesOf,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DuplicatesDialog> {
    open_with(state, of, None, window, cx)
}

/// [`open_duplicates`] over another dialog: `on_change` runs after each
/// choice.
pub fn open_duplicates_then(
    state: Entity<AppState>,
    of: DuplicatesOf,
    on_change: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DuplicatesDialog> {
    open_with(state, of, Some(Rc::new(on_change)), window, cx)
}

fn open_with(
    state: Entity<AppState>,
    of: DuplicatesOf,
    on_change: Option<OnChange>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DuplicatesDialog> {
    let dialog = cx.new(|cx| {
        let mut dialog = DuplicatesDialog::new(state, of, cx);
        dialog.on_change = on_change;
        dialog
    });
    let view = dialog.clone();
    window.open_dialog(cx, move |modal, window, cx| {
        // Never wider than the window (a zoomed small window).
        let room = window.viewport_size().width - metrics::scaled(2. * 24., cx);
        let width = metrics::scaled(DIALOG_WIDTH, cx);
        modal
            .w(if width < room { width } else { room })
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::DUPLICATES))
            .child(view.clone())
            .footer(close_footer("sync-duplicates-close"))
    });
    // After the dialog took the focus: the older record's Keep left.
    if !dialog.read(cx).items.is_empty() {
        let focus = dialog.read(cx).focus.clone();
        focus.focus(window, cx);
    }
    dialog
}
