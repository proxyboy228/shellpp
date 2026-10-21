//! Headless tests of the Port Forwarding screen (GPUI `TestAppContext`, no
//! real window): the screen is the window's root view, over a temporary
//! profile (`TestProfile`: in-memory secret store). Rules are saved through
//! the form or the core; nothing connects (statuses are set by hand where a
//! test needs one).

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_core::service::snippets::RecordInput;
use shellpp_ssh::Bound;

use super::*;
use crate::state::AppState;
use crate::testing::TestProfile;

struct Screen {
    state: Entity<AppState>,
    screen: Entity<ForwardingScreen>,
    handle: AnyWindowHandle,
}

impl Screen {
    /// A 1280 px window with the screen over a temporary profile holding
    /// `hosts`.
    fn open(cx: &mut TestAppContext, hosts: &[Value]) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            cx.set_reduce_motion(true);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        state.update(cx, |state, cx| {
            for host in hosts {
                let data: Map<String, Value> = host.as_object().cloned().unwrap();
                state.save_host(None, data, cx).unwrap();
            }
        });
        cx.run_until_parked();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1280.), px(800.)),
            ))),
            ..Default::default()
        };
        let screen_state = state.clone();
        let (handle, screen) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| ForwardingScreen::new(screen_state, window, cx))
                })
            })
            .unwrap();
        let this = Self {
            state,
            screen,
            handle,
        };
        this.frame(cx, |_, _| {});
        this.frame(cx, |_, _| {});
        this
    }

    fn frame<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        result
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.frame(cx, |window, _| {
            window.try_find(id).is_some_and(|found| found.visible())
        })
    }

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    /// Clicks into the input `id`, empties it and types `text`.
    fn type_into(&self, cx: &mut TestAppContext, id: &'static str, text: &str) {
        self.frame(cx, |window, cx| {
            window.click(id, cx);
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            window.input(text, cx);
        });
    }

    /// Runs `f` on the screen with the window (commands that open forms).
    fn act(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut ForwardingScreen, &mut Window, &mut Context<ForwardingScreen>),
    ) {
        let screen = self.screen.clone();
        self.frame(cx, move |window, cx| {
            screen.update(cx, |screen, cx| f(screen, window, cx))
        });
    }

    fn form(&self, cx: &mut TestAppContext) -> Entity<RuleForm> {
        self.screen
            .read_with(cx, |screen, _| screen.form().cloned())
            .expect("the form is open")
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn rules(&self, cx: &mut TestAppContext) -> Vec<ForwardingRuleView> {
        self.core(cx).forwarding_rules_personal().unwrap()
    }

    fn host_id(&self, cx: &mut TestAppContext, address: &str) -> Uuid {
        self.state.read_with(cx, |state, _| {
            state
                .hosts()
                .iter()
                .find(|host| host.address == address)
                .unwrap()
                .record_id
        })
    }

    /// Saves a rule through the core and reloads the screen.
    fn save_rule(&self, cx: &mut TestAppContext, data: Value) -> ForwardingRuleView {
        let core = self.core(cx);
        let vault_id = self
            .state
            .read_with(cx, |state, _| state.vault_id().unwrap());
        let view = core
            .forwarding_rule_save(RecordInput {
                vault_id,
                record_id: None,
                data: data.as_object().cloned().unwrap(),
            })
            .unwrap();
        self.screen.update(cx, |screen, cx| screen.reload(cx));
        self.frame(cx, |_, _| {});
        view
    }

    fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
    }
}

#[test]
fn statuses_read_as_words_and_a_busy_port_names_the_port() {
    assert_eq!(
        status_text(None),
        (StatusTone::Stopped, "Stopped".to_owned())
    );
    let (tone, text) = status_text(Some(&RuleStatus::Failed(ForwardError::PortBusy(8080))));
    assert_eq!(tone, StatusTone::Error);
    assert!(
        text.contains("8080") && text.contains("another port"),
        "{text}"
    );
    let (tone, text) = status_text(Some(&RuleStatus::Reconnecting { attempt: 2 }));
    assert_eq!(tone, StatusTone::Busy);
    assert!(text.contains('2'), "{text}");
    let running = RuleStatus::Running(Bound::Remote(9000));
    assert_eq!(status_text(Some(&running)).0, StatusTone::Running);
    let (tone, _) = status_text(Some(&RuleStatus::ConnectFailed(
        shellpp_ssh::FailureKind::AuthFailed,
    )));
    assert_eq!(tone, StatusTone::Error);
    assert!(is_started(Some(&running)));
    assert!(is_started(Some(&RuleStatus::Reconnecting { attempt: 1 })));
    assert!(!is_started(Some(&RuleStatus::Failed(
        ForwardError::Refused
    ))));
    assert!(!is_started(None));
}

#[test]
fn wide_binds_and_ports_are_read_from_the_fields() {
    for address in ["", "127.0.0.1", "localhost", "::1", "[::1]", " 127.0.0.2 "] {
        assert!(!form::is_wide(address), "{address}");
    }
    for address in ["0.0.0.0", "::", "[::]", "192.168.1.10"] {
        assert!(form::is_wide(address), "{address}");
    }
    assert_eq!(form::parse_port(" 8080 "), Some(8080));
    for text in ["", "0", "65536", "80a", "-1"] {
        assert_eq!(form::parse_port(text), None, "{text}");
    }
}

#[gpui_kit::test]
fn the_empty_screen_offers_a_new_rule(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com"})]);
    assert!(ui.visible(cx, "fwd-empty"));
    assert!(!ui.visible(cx, "fwd-panel"));
    ui.click(cx, "fwd-empty-new");
    assert!(ui.visible(cx, "fwd-panel"));
    let form = ui.form(cx);
    form.read_with(cx, |form, _| {
        assert_eq!(form.mode(), FormMode::New);
        assert_eq!(form.kind(), ForwardType::Local);
    });
    // The only host is chosen; bind defaults to 127.0.0.1.
    let web = ui.host_id(cx, "web.example.com");
    assert_eq!(form.read_with(cx, |form, _| form.host_id()), Some(web));
    assert_eq!(
        form.read_with(cx, |form, cx| form
            .bind_address_input()
            .read(cx)
            .value()
            .to_string()),
        "127.0.0.1"
    );
    for id in [
        "fwd-type-local",
        "fwd-host",
        "fwd-bind",
        "fwd-dest",
        "fwd-create",
        "fwd-create-start",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    assert!(!ui.visible(cx, "fwd-wide-warning"));
    assert!(!ui.visible(cx, "fwd-panel-status"));
    ui.click(cx, "panel-close");
    assert!(!ui.visible(cx, "fwd-panel"));
    ui.finish(cx);
}

#[gpui_kit::test]
fn a_local_rule_is_created_from_the_form(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com", "label": "Web"})]);
    ui.click(cx, "fwd-new");
    ui.type_into(cx, "fwd-label", "Database");
    ui.type_into(cx, "fwd-bind-port", "15432");
    ui.type_into(cx, "fwd-dest-address", "db.internal");
    ui.type_into(cx, "fwd-dest-port", "5432");
    ui.click(cx, "fwd-on-connect");
    ui.click(cx, "fwd-create");
    let rules = ui.rules(cx);
    assert_eq!(rules.len(), 1);
    let rule = &rules[0];
    assert_eq!(rule.kind, ForwardType::Local);
    assert_eq!(rule.label.as_deref(), Some("Database"));
    assert_eq!(
        (rule.bind_address.as_str(), rule.bind_port),
        ("127.0.0.1", 15432)
    );
    assert_eq!(rule.dest_address.as_deref(), Some("db.internal"));
    assert_eq!(rule.dest_port, Some(5432));
    assert!(rule.autostart_on_connect && !rule.autostart_on_launch);
    assert_eq!(rule.summary, "127.0.0.1:15432 → db.internal:5432 via Web");
    // The card is there; the panel now edits the saved rule.
    let id = rule.record.record_id;
    assert!(ui.visible(cx, card_id(id)));
    assert!(ui.visible(cx, status_id(id)));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(id)
    );
    assert_eq!(
        ui.form(cx).read_with(cx, |form, _| form.mode()),
        FormMode::Edit(id)
    );
    for id in [
        "fwd-panel-status",
        "fwd-panel-start",
        "fwd-save",
        "fwd-panel-remove",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }

    // Save keeps the record and takes the new values.
    ui.type_into(cx, "fwd-bind-port", "15433");
    ui.click(cx, "fwd-save");
    let rules = ui.rules(cx);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].record.record_id, id);
    assert_eq!(rules[0].bind_port, 15433);
    ui.finish(cx);
}

#[gpui_kit::test]
fn the_form_names_the_missing_fields(cx: &mut TestAppContext) {
    let ui = Screen::open(
        cx,
        &[
            json!({"address": "web.example.com"}),
            json!({"address": "db.example.com"}),
        ],
    );
    ui.click(cx, "fwd-new");
    // Two hosts: none is chosen for the user.
    let form = ui.form(cx);
    assert_eq!(form.read_with(cx, |form, _| form.host_id()), None);
    ui.click(cx, "fwd-create");
    let errors = form.read_with(cx, |form, _| form.errors().clone());
    assert_eq!(
        errors.keys().copied().collect::<Vec<_>>(),
        ["bind_port", "dest_address", "dest_port", "host_id"]
    );
    assert_eq!(errors["host_id"], strings::ERROR_HOST);
    assert!(ui.rules(cx).is_empty());

    // A typed field drops its error; a bad address comes back from the core.
    ui.type_into(cx, "fwd-bind-port", "70000");
    assert!(!form.read_with(cx, |form, _| form.errors().contains_key("bind_port")));
    ui.click(cx, "fwd-host");
    let db = ui.host_id(cx, "db.example.com");
    ui.click(cx, format!("fwd-host-{db}"));
    assert_eq!(form.read_with(cx, |form, _| form.host_id()), Some(db));
    ui.type_into(cx, "fwd-bind-address", "not an address");
    ui.type_into(cx, "fwd-dest-address", "db.internal");
    ui.type_into(cx, "fwd-dest-port", "5432");
    ui.click(cx, "fwd-create");
    // 70000 is not a port: the form says so before the core sees it.
    assert_eq!(
        form.read_with(cx, |form, _| form
            .errors()
            .keys()
            .copied()
            .collect::<Vec<_>>()),
        ["bind_port"]
    );
    ui.type_into(cx, "fwd-bind-port", "8080");
    ui.click(cx, "fwd-create");
    assert_eq!(
        form.read_with(cx, |form, _| form.errors().get("bind_address").copied()),
        Some(strings::ERROR_ADDRESS)
    );
    assert!(ui.rules(cx).is_empty());
    ui.finish(cx);
}

#[gpui_kit::test]
fn a_wide_bind_warns_and_remote_names_gateway_ports(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com"})]);
    ui.click(cx, "fwd-new");
    ui.type_into(cx, "fwd-bind-address", "0.0.0.0");
    assert!(ui.visible(cx, "fwd-wide-warning"));
    let form = ui.form(cx);
    assert!(form.read_with(cx, |form, cx| form.wide_bind(cx)));
    ui.click(cx, "fwd-type-remote");
    assert_eq!(
        form.read_with(cx, |form, _| form.kind()),
        ForwardType::Remote
    );
    assert!(ui.visible(cx, "fwd-wide-warning"));
    assert!(strings::WIDE_REMOTE.contains("GatewayPorts"));
    // Remote suggests 127.0.0.1 as the destination on this computer.
    assert_eq!(
        form.read_with(cx, |form, cx| form
            .dest_address_input()
            .read(cx)
            .value()
            .to_string()),
        "127.0.0.1"
    );
    ui.type_into(cx, "fwd-bind-address", "127.0.0.1");
    assert!(!ui.visible(cx, "fwd-wide-warning"));
    ui.finish(cx);
}

#[gpui_kit::test]
fn a_dynamic_rule_is_a_socks_port_without_a_destination(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com", "label": "Web"})]);
    ui.act(cx, |screen, window, cx| {
        screen.new_rule(ForwardType::Dynamic, window, cx)
    });
    let form = ui.form(cx);
    assert_eq!(
        form.read_with(cx, |form, cx| form
            .bind_port_input()
            .read(cx)
            .value()
            .to_string()),
        "1080"
    );
    assert!(ui.visible(cx, "fwd-bind"));
    assert!(!ui.visible(cx, "fwd-dest"));
    ui.click(cx, "fwd-create");
    let rules = ui.rules(cx);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].kind, ForwardType::Dynamic);
    assert_eq!(rules[0].summary, "SOCKS 127.0.0.1:1080 via Web");
    assert_eq!(rules[0].dest_address, None);
    ui.finish(cx);
}

#[gpui_kit::test]
fn cards_show_statuses_search_duplicate_and_remove(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com", "label": "Web"})]);
    let web = ui.host_id(cx, "web.example.com");
    let db = ui.save_rule(
        cx,
        json!({"type": "local", "host_id": web, "label": "Database", "bind_port": 15432,
               "dest_address": "db.internal", "dest_port": 5432}),
    );
    let socks = ui.save_rule(
        cx,
        json!({"type": "dynamic", "host_id": web, "bind_port": 1080}),
    );
    let (db_id, socks_id) = (db.record.record_id, socks.record.record_id);
    assert!(ui.visible(cx, card_id(db_id)) && ui.visible(cx, card_id(socks_id)));
    assert_eq!(type_letter(db.kind), "L");
    assert_eq!(card_title(&socks), "SOCKS 127.0.0.1:1080 via Web");

    // A failed start names the busy port on the card.
    ui.screen.update(cx, |screen, cx| {
        screen
            .statuses
            .insert(db_id, RuleStatus::Failed(ForwardError::PortBusy(15432)));
        cx.notify();
    });
    assert_eq!(
        status_text(
            ui.screen
                .read_with(cx, |screen, _| screen.status(db_id).cloned())
                .as_ref()
        )
        .0,
        StatusTone::Error
    );
    assert!(ui.visible(cx, status_id(db_id)));

    // Search by label or summary.
    ui.type_into(cx, "fwd-search", "socks");
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.shown_ids()),
        [socks_id]
    );
    ui.type_into(cx, "fwd-search", "nothing like it");
    assert!(ui.visible(cx, "fwd-no-matches"));
    ui.type_into(cx, "fwd-search", "");
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.shown_ids())
            .len(),
        2
    );

    // Duplicate opens a new rule with the values and "<title> Copy".
    ui.act(cx, |screen, window, cx| screen.duplicate(db_id, window, cx));
    let form = ui.form(cx);
    assert_eq!(form.read_with(cx, |form, _| form.mode()), FormMode::New);
    assert_eq!(
        form.read_with(cx, |form, cx| form
            .label_input()
            .read(cx)
            .value()
            .to_string()),
        "Database Copy"
    );
    assert_eq!(ui.rules(cx).len(), 2);

    // Remove from the panel asks first; Undo brings the rule back.
    ui.click(cx, card_id(db_id));
    ui.click(cx, "fwd-panel-remove");
    assert!(ui.visible(cx, "fwd-remove-dialog"));
    ui.click(cx, "fwd-remove-confirm");
    assert_eq!(ui.rules(cx).len(), 1);
    assert!(!ui.visible(cx, card_id(db_id)));
    assert!(!ui.visible(cx, "fwd-panel"));
    assert!(ui.screen.read_with(cx, |screen, _| screen.can_undo()));
    ui.act(cx, |screen, window, cx| screen.undo_remove(window, cx));
    assert!(
        ui.rules(cx)
            .iter()
            .any(|rule| rule.record.record_id == db_id)
    );
    assert!(ui.visible(cx, card_id(db_id)));
    ui.finish(cx);
}
