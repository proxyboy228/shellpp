//! The full form of a forwarding rule (UI 8.4): type, host, label, where
//! it listens and where it leads, and the autostart flags. It is the panel
//! of the Port Forwarding screen for a new rule, a copy and an existing one.
//!
//! - The bind address defaults to 127.0.0.1. Any other address that is not
//!   loopback (0.0.0.0, ::, an address of a network) shows a warning; for
//!   Remote it names `GatewayPorts` (8.3, 8.4).
//! - Ports are checked here (1-65535); the core checks the rest and its
//!   errors appear under their fields.

use std::collections::BTreeMap;

use gpui::prelude::*;
use gpui::{App, ClickEvent, Context, Entity, IntoElement, Render, Subscription, Window, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use serde_json::{Map, Value, json};
use shellpp_core::error::{FieldError, ValidationReason};
use shellpp_core::service::snippets::{ForwardType, ForwardingRuleView};
use uuid::Uuid;

use super::{card_title, strings};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// The width of a port field.
const PORT_WIDTH: gpui::Pixels = px(96.);

/// Whether the form makes a new rule or edits a saved one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormMode {
    New,
    Edit(Uuid),
}

/// The data-model name of a rule type.
pub fn kind_name(kind: ForwardType) -> &'static str {
    match kind {
        ForwardType::Local => "local",
        ForwardType::Remote => "remote",
        ForwardType::Dynamic => "dynamic",
    }
}

/// Whether a typed bind address opens the port beyond loopback (empty is
/// 127.0.0.1).
pub fn is_wide(address: &str) -> bool {
    let address = address.trim();
    !address.is_empty()
        && shellpp_core::model::snippet::is_wide_bind(
            address.trim_start_matches('[').trim_end_matches(']'),
        )
}

/// A port typed in a field: 1-65535.
pub fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|port| *port > 0)
}

/// The text under a field the core refused.
fn reason_text(field: &str, reason: ValidationReason) -> &'static str {
    match (field, reason) {
        ("host_id", _) => strings::ERROR_HOST,
        ("bind_port" | "dest_port", _) => strings::ERROR_PORT,
        ("bind_address", _) => strings::ERROR_ADDRESS,
        ("dest_address", ValidationReason::Required) => strings::ERROR_DEST,
        (_, ValidationReason::TooLong) => strings::ERROR_TOO_LONG,
        _ => strings::ERROR_INVALID,
    }
}

pub struct RuleForm {
    mode: FormMode,
    kind: ForwardType,
    host_id: Option<Uuid>,
    host_open: bool,
    label: Entity<InputState>,
    bind_address: Entity<InputState>,
    bind_port: Entity<InputState>,
    dest_address: Entity<InputState>,
    dest_port: Entity<InputState>,
    on_connect: bool,
    on_launch: bool,
    /// The reasons under refused fields, by data-model member.
    errors: BTreeMap<&'static str, &'static str>,
    state: Entity<AppState>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for RuleForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleForm")
            .field("mode", &self.mode)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// The values a form starts with.
struct Start {
    mode: FormMode,
    kind: ForwardType,
    host_id: Option<Uuid>,
    label: String,
    bind_address: String,
    bind_port: String,
    dest_address: String,
    dest_port: String,
    on_connect: bool,
    on_launch: bool,
}

impl Start {
    fn of(rule: &ForwardingRuleView, mode: FormMode, label: String) -> Self {
        Self {
            mode,
            kind: rule.kind,
            host_id: Some(rule.host_id),
            label,
            bind_address: rule.bind_address.clone(),
            bind_port: rule.bind_port.to_string(),
            dest_address: rule.dest_address.clone().unwrap_or_default(),
            dest_port: rule
                .dest_port
                .map(|port| port.to_string())
                .unwrap_or_default(),
            on_connect: rule.autostart_on_connect,
            on_launch: rule.autostart_on_launch,
        }
    }
}

impl RuleForm {
    /// A new rule of `kind` (the host preselected when there is only one).
    pub fn new(
        kind: ForwardType,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let hosts = state.read(cx).hosts();
        let host_id = (hosts.len() == 1).then(|| hosts[0].record_id);
        let start = Start {
            mode: FormMode::New,
            kind,
            host_id,
            label: String::new(),
            bind_address: "127.0.0.1".into(),
            bind_port: if kind == ForwardType::Dynamic {
                "1080".into()
            } else {
                String::new()
            },
            dest_address: if kind == ForwardType::Remote {
                "127.0.0.1".into()
            } else {
                String::new()
            },
            dest_port: String::new(),
            on_connect: false,
            on_launch: false,
        };
        Self::build(start, state, window, cx)
    }

    /// A saved rule.
    pub fn edit(
        rule: &ForwardingRuleView,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let start = Start::of(
            rule,
            FormMode::Edit(rule.record.record_id),
            rule.label.clone().unwrap_or_default(),
        );
        Self::build(start, state, window, cx)
    }

    /// Duplicate: a new rule with the values of `rule`, "<title> Copy".
    pub fn copy_of(
        rule: &ForwardingRuleView,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label = format!("{} {}", card_title(rule), strings::COPY_SUFFIX);
        let start = Start::of(rule, FormMode::New, label);
        Self::build(start, state, window, cx)
    }

    fn build(
        start: Start,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let mut input = |value: String, placeholder: &'static str, member: &'static str| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder(placeholder)
            });
            // A change clears the field's error and redraws the warnings.
            subscriptions.push(cx.subscribe(
                &input,
                move |form: &mut Self, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        form.errors.remove(member);
                        cx.notify();
                    }
                },
            ));
            input
        };
        let label = input(start.label, strings::LABEL_PLACEHOLDER, "label");
        let bind_address = input(start.bind_address, "127.0.0.1", "bind_address");
        let bind_port = input(start.bind_port, "8080", "bind_port");
        let dest_address = input(start.dest_address, "db.internal", "dest_address");
        let dest_port = input(start.dest_port, "5432", "dest_port");
        Self {
            mode: start.mode,
            kind: start.kind,
            host_id: start.host_id,
            host_open: false,
            label,
            bind_address,
            bind_port,
            dest_address,
            dest_port,
            on_connect: start.on_connect,
            on_launch: start.on_launch,
            errors: BTreeMap::new(),
            state,
            _subscriptions: subscriptions,
        }
    }

    // -- Reading -----------------------------------------------------------------

    pub fn mode(&self) -> FormMode {
        self.mode
    }

    pub fn kind(&self) -> ForwardType {
        self.kind
    }

    pub fn host_id(&self) -> Option<Uuid> {
        self.host_id
    }

    pub fn errors(&self) -> &BTreeMap<&'static str, &'static str> {
        &self.errors
    }

    pub fn label_input(&self) -> &Entity<InputState> {
        &self.label
    }

    pub fn bind_address_input(&self) -> &Entity<InputState> {
        &self.bind_address
    }

    pub fn bind_port_input(&self) -> &Entity<InputState> {
        &self.bind_port
    }

    pub fn dest_address_input(&self) -> &Entity<InputState> {
        &self.dest_address
    }

    pub fn dest_port_input(&self) -> &Entity<InputState> {
        &self.dest_port
    }

    /// Whether the typed bind address opens the port beyond loopback.
    pub fn wide_bind(&self, cx: &App) -> bool {
        is_wide(&self.bind_address.read(cx).value())
    }

    /// The rule's data (data-model 11), or the reasons under the fields
    /// this form checks itself.
    pub fn data(
        &self,
        cx: &App,
    ) -> Result<Map<String, Value>, BTreeMap<&'static str, &'static str>> {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        let mut errors = BTreeMap::new();
        let mut data = Map::new();
        data.insert("type".into(), json!(kind_name(self.kind)));
        match self.host_id {
            Some(host) => {
                data.insert("host_id".into(), json!(host));
            }
            None => {
                errors.insert("host_id", strings::ERROR_HOST);
            }
        }
        let label = text(&self.label);
        if !label.is_empty() {
            data.insert("label".into(), json!(label));
        }
        // Empty is 127.0.0.1 (the core's default).
        data.insert("bind_address".into(), json!(text(&self.bind_address)));
        match parse_port(&text(&self.bind_port)) {
            Some(port) => {
                data.insert("bind_port".into(), json!(port));
            }
            None => {
                errors.insert("bind_port", strings::ERROR_PORT);
            }
        }
        if self.kind != ForwardType::Dynamic {
            let dest = text(&self.dest_address);
            if dest.is_empty() {
                errors.insert("dest_address", strings::ERROR_DEST);
            } else {
                data.insert("dest_address".into(), json!(dest));
            }
            match parse_port(&text(&self.dest_port)) {
                Some(port) => {
                    data.insert("dest_port".into(), json!(port));
                }
                None => {
                    errors.insert("dest_port", strings::ERROR_PORT);
                }
            }
        }
        data.insert("autostart_on_connect".into(), json!(self.on_connect));
        data.insert("autostart_on_launch".into(), json!(self.on_launch));
        if errors.is_empty() {
            Ok(data)
        } else {
            Err(errors)
        }
    }

    // -- Changes -------------------------------------------------------------------

    pub fn set_errors(
        &mut self,
        errors: BTreeMap<&'static str, &'static str>,
        cx: &mut Context<Self>,
    ) {
        self.errors = errors;
        cx.notify();
    }

    /// The core's refusal, under the fields it names.
    pub fn set_core_errors(&mut self, fields: &[FieldError], cx: &mut Context<Self>) {
        const MEMBERS: [&str; 6] = [
            "host_id",
            "label",
            "bind_address",
            "bind_port",
            "dest_address",
            "dest_port",
        ];
        self.errors = fields
            .iter()
            .map(|error| {
                let member = MEMBERS
                    .iter()
                    .copied()
                    .find(|member| *member == error.field)
                    .unwrap_or("type");
                (member, reason_text(member, error.reason))
            })
            .collect();
        cx.notify();
    }

    /// Picks the type; a new rule gets the defaults of 8.3 in the fields
    /// still empty (1080 for SOCKS, 127.0.0.1 as Remote's destination).
    pub fn set_kind(&mut self, kind: ForwardType, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        if self.mode == FormMode::New {
            let fill =
                |input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App| {
                    if input.read(cx).value().trim().is_empty() {
                        input.update(cx, |input, cx| input.set_value(value, window, cx));
                    }
                };
            match kind {
                ForwardType::Dynamic => fill(&self.bind_port, "1080", window, cx),
                ForwardType::Remote => fill(&self.dest_address, "127.0.0.1", window, cx),
                ForwardType::Local => {}
            }
        }
        self.errors.clear();
        cx.notify();
    }

    pub fn set_host(&mut self, host: Uuid, cx: &mut Context<Self>) {
        self.host_id = Some(host);
        self.host_open = false;
        self.errors.remove("host_id");
        cx.notify();
    }

    pub fn set_autostart(&mut self, on_connect: bool, on_launch: bool, cx: &mut Context<Self>) {
        self.on_connect = on_connect;
        self.on_launch = on_launch;
        cx.notify();
    }

    // -- Rendering -------------------------------------------------------------------

    fn render_type(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let segment =
            |id: &'static str, label: &'static str, kind: ForwardType, cx: &mut Context<Self>| {
                widgets::segment(id, label, self.kind == kind, cx)
                    .on_click(cx.listener(move |form, _: &ClickEvent, window, cx| {
                        form.set_kind(kind, window, cx)
                    }))
                    .into_any_element()
            };
        let explain = match self.kind {
            ForwardType::Local => strings::EXPLAIN_LOCAL,
            ForwardType::Remote => strings::EXPLAIN_REMOTE,
            ForwardType::Dynamic => strings::EXPLAIN_DYNAMIC,
        };
        widgets::block("fwd-form-type", None, cx)
            .child(widgets::field(
                strings::TYPE,
                widgets::segments([
                    segment(
                        "fwd-type-local",
                        strings::TYPE_LOCAL,
                        ForwardType::Local,
                        cx,
                    ),
                    segment(
                        "fwd-type-remote",
                        strings::TYPE_REMOTE,
                        ForwardType::Remote,
                        cx,
                    ),
                    segment(
                        "fwd-type-dynamic",
                        strings::TYPE_DYNAMIC,
                        ForwardType::Dynamic,
                        cx,
                    ),
                ]),
                None,
                cx,
            ))
            .child(
                widgets::caption(explain, cx)
                    .id("fwd-type-explain")
                    .test_support(),
            )
    }

    fn render_host(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hosts = self.state.read(cx).hosts();
        let chosen = self
            .host_id
            .and_then(|id| hosts.iter().find(|host| host.record_id == id));
        let control = if hosts.is_empty() {
            widgets::caption(strings::NO_HOSTS, cx)
                .id("fwd-no-hosts")
                .test_support()
                .into_any_element()
        } else {
            let trigger = widgets::select_trigger(
                "fwd-host",
                None,
                chosen.map_or(strings::HOST_NONE.to_owned(), |host| {
                    host.title().to_owned()
                }),
                self.host_open,
                cx,
            )
            .when(self.errors.contains_key("host_id"), |this| {
                this.border_color(cx.theme().danger)
            })
            .on_click(cx.listener(|form, _: &ClickEvent, _, cx| {
                form.host_open = !form.host_open;
                cx.notify();
            }));
            let list = self.host_open.then(|| {
                widgets::dropdown_list("fwd-host-list", cx)
                    .max_h(widgets::PICKER_MAX_HEIGHT)
                    .overflow_y_scroll()
                    .children(hosts.iter().map(|host| {
                        let id = host.record_id;
                        widgets::dropdown_item(
                            format!("fwd-host-{id}"),
                            None,
                            host.title().to_owned(),
                            Some(host.address.clone().into()),
                            self.host_id == Some(id),
                            cx,
                        )
                        .on_click(
                            cx.listener(move |form, _: &ClickEvent, _, cx| form.set_host(id, cx)),
                        )
                    }))
            });
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(trigger)
                .children(list)
                .into_any_element()
        };
        widgets::block("fwd-form-host", None, cx)
            .child(widgets::field(
                strings::HOST,
                control,
                self.errors.get("host_id").copied(),
                cx,
            ))
            .child(widgets::field(
                strings::LABEL,
                widgets::text_input(
                    &self.label,
                    "fwd-label",
                    self.errors.contains_key("label"),
                    window,
                    cx,
                ),
                self.errors.get("label").copied(),
                cx,
            ))
    }

    /// Address and port side by side, each with its reason below.
    fn endpoint(
        &self,
        id: &'static str,
        address: (&Entity<InputState>, &'static str, &'static str),
        port: (&Entity<InputState>, &'static str, &'static str),
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let (address, address_id, address_member) = address;
        let (port, port_id, port_member) = port;
        h_flex()
            .id(id)
            .test_support()
            .w_full()
            .items_start()
            .gap(metrics::SPACE_2)
            .child(div().flex_1().min_w_0().child(widgets::field(
                strings::ADDRESS,
                widgets::text_input(
                    address,
                    address_id,
                    self.errors.contains_key(address_member),
                    window,
                    cx,
                ),
                self.errors.get(address_member).copied(),
                cx,
            )))
            .child(div().flex_none().w(PORT_WIDTH).child(widgets::field(
                strings::PORT,
                widgets::text_input(
                    port,
                    port_id,
                    self.errors.contains_key(port_member),
                    window,
                    cx,
                ),
                self.errors.get(port_member).copied(),
                cx,
            )))
    }

    fn render_endpoints(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (listen, dest) = match self.kind {
            ForwardType::Local => (strings::LISTEN_HERE, Some(strings::DEST_THERE)),
            ForwardType::Remote => (strings::LISTEN_THERE, Some(strings::DEST_HERE)),
            ForwardType::Dynamic => (strings::SOCKS_HERE, None),
        };
        let warning = self.wide_bind(cx).then(|| {
            div()
                .id("fwd-wide-warning")
                .test_support()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(cx.theme().warning)
                .child(if self.kind == ForwardType::Remote {
                    strings::WIDE_REMOTE
                } else {
                    strings::WIDE_LOCAL
                })
        });
        widgets::block("fwd-form-listen", Some(listen), cx)
            .child(self.endpoint(
                "fwd-bind",
                (&self.bind_address, "fwd-bind-address", "bind_address"),
                (&self.bind_port, "fwd-bind-port", "bind_port"),
                window,
                cx,
            ))
            .children(warning)
            .when_some(dest, |this, dest| {
                this.child(widgets::divider(cx)).child(
                    v_flex()
                        .w_full()
                        .gap(metrics::SPACE_2)
                        .child(
                            div()
                                .text_size(metrics::FONT_SIZE_BODY)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                .child(dest),
                        )
                        .child(self.endpoint(
                            "fwd-dest",
                            (&self.dest_address, "fwd-dest-address", "dest_address"),
                            (&self.dest_port, "fwd-dest-port", "dest_port"),
                            window,
                            cx,
                        )),
                )
            })
    }

    fn render_autostart(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let on_connect = Switch::new("fwd-on-connect")
            .checked(self.on_connect)
            .on_click(cx.listener(|form, checked: &bool, _, cx| {
                form.set_autostart(*checked, form.on_launch, cx)
            }));
        let on_launch = Switch::new("fwd-on-launch")
            .checked(self.on_launch)
            .on_click(cx.listener(|form, checked: &bool, _, cx| {
                form.set_autostart(form.on_connect, *checked, cx)
            }));
        widgets::block("fwd-form-autostart", Some(strings::AUTOSTART), cx)
            .child(widgets::inline_row(strings::ON_CONNECT, on_connect, cx))
            .child(widgets::inline_row(strings::ON_LAUNCH, on_launch, cx))
            .when(self.on_launch, |this| {
                this.child(widgets::caption(strings::ON_LAUNCH_NOTE, cx))
            })
    }
}

impl Render for RuleForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("fwd-form")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(self.render_type(cx))
            .child(self.render_host(window, cx))
            .child(self.render_endpoints(window, cx))
            .child(self.render_autostart(cx))
    }
}
