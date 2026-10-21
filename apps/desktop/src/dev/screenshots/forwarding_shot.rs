//! The stage 5 states of the screenshot run (UI 8; plan of run 3, section
//! 8): the empty Port Forwarding screen, rule cards in each status, and the
//! full form of a Remote rule open on all interfaces (its warning names
//! `GatewayPorts`).
//!
//! The rules are saved in the demo profile for the run; nothing starts:
//! the statuses on the cards are set on the screen
//! ([`ForwardingScreen::show_statuses`]).

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Value, json};
use shellpp_core::service::snippets::{ForwardType, ForwardingRuleView, RecordInput};
use shellpp_ssh::{Bound, ForwardError, RuleStatus};

use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::forwarding::Step;
use crate::shell::{Section, TopTab};
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 10] = [
    ShotState::ForwardingEmpty,
    ShotState::ForwardingCards,
    ShotState::ForwardingForm,
    ShotState::ForwardingWizardType,
    ShotState::ForwardingWizardRemote,
    ShotState::ForwardingWizardDynamic,
    ShotState::ForwardingWizardHost,
    ShotState::ForwardingWizardListen,
    ShotState::ForwardingWizardDest,
    ShotState::ForwardingWizardSummary,
];

/// The wizard's type and step of a wizard state (UI 8.3).
fn wizard_of(shot: ShotState) -> Option<(ForwardType, Step)> {
    Some(match shot {
        ShotState::ForwardingWizardType => (ForwardType::Local, Step::Type),
        ShotState::ForwardingWizardRemote => (ForwardType::Remote, Step::Type),
        ShotState::ForwardingWizardDynamic => (ForwardType::Dynamic, Step::Type),
        ShotState::ForwardingWizardHost => (ForwardType::Local, Step::Host),
        ShotState::ForwardingWizardListen => (ForwardType::Local, Step::Listen),
        ShotState::ForwardingWizardDest => (ForwardType::Local, Step::Destination),
        ShotState::ForwardingWizardSummary => (ForwardType::Local, Step::Summary),
        _ => return None,
    })
}

fn not_reached(state: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(state, reason)
}

/// The demo rules, through the demo's first two hosts.
fn rules(first: uuid::Uuid, second: uuid::Uuid) -> [Value; 4] {
    [
        json!({"type": "local", "host_id": first, "label": "Database",
               "bind_port": 15432, "dest_address": "db.internal", "dest_port": 5432,
               "autostart_on_connect": true}),
        json!({"type": "local", "host_id": second, "label": "Grafana",
               "bind_port": 3000, "dest_address": "127.0.0.1", "dest_port": 3000}),
        json!({"type": "remote", "host_id": first, "label": "Webhook preview",
               "bind_address": "0.0.0.0", "bind_port": 8080,
               "dest_address": "127.0.0.1", "dest_port": 5173}),
        json!({"type": "dynamic", "host_id": second, "label": "Office SOCKS",
               "bind_port": 1080}),
    ]
}

/// Leaves the demo with no rules (`empty`) or with the four demo rules.
fn prepare(
    state: &Entity<AppState>,
    empty: bool,
    cx: &mut gpui::App,
) -> Option<Vec<ForwardingRuleView>> {
    let core = state.read(cx).core()?.clone();
    let vault_id = state.read(cx).vault_id()?;
    let existing = core.forwarding_rules_personal().ok()?;
    if empty {
        if !existing.is_empty() {
            let ids = existing.iter().map(|rule| rule.record.record_id).collect();
            state
                .update(cx, |state, cx| state.delete_records(ids, cx))
                .ok()?;
        }
        return Some(Vec::new());
    }
    if !existing.is_empty() {
        return Some(existing);
    }
    let hosts = state.read(cx).hosts();
    let first = hosts.first()?.record_id;
    let second = hosts.get(1).map_or(first, |host| host.record_id);
    for data in rules(first, second) {
        core.forwarding_rule_save(RecordInput {
            vault_id,
            record_id: None,
            data: data.as_object()?.clone(),
        })
        .ok()?;
    }
    core.forwarding_rules_personal().ok()
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, shot)?;
    let rules = cx
        .update(|cx| prepare(state, shot == ShotState::ForwardingEmpty, cx))
        .ok_or_else(|| not_reached(shot, "the demo rules could not be saved"))?;
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.set_top_tab(TopTab::Vaults, cx);
            shell.set_section(Section::PortForwarding, cx);
        })
    });
    let screen = cx.update(|cx| shell.read(cx).forwarding_screen().clone());
    cx.update(|cx| screen.update(cx, |screen, cx| screen.reload(cx)));
    settle(cx, handle).await?;
    if shot == ShotState::ForwardingEmpty {
        cx.update_window(handle, |_, window, cx| {
            screen.update(cx, |screen, cx| screen.close_panel(cx));
            window.refresh();
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
        return settle(cx, handle).await;
    }
    let by_label = |label: &str| {
        rules
            .iter()
            .find(|rule| rule.label.as_deref() == Some(label))
            .map(|rule| rule.record.record_id)
    };
    let (Some(database), Some(webhook), Some(socks)) = (
        by_label("Database"),
        by_label("Webhook preview"),
        by_label("Office SOCKS"),
    ) else {
        return Err(not_reached(shot, "the demo rules are missing"));
    };
    // Running, an error, reconnecting; Grafana stays stopped.
    let statuses = vec![
        (
            database,
            RuleStatus::Running(Bound::Local(([127, 0, 0, 1], 15432).into())),
        ),
        (webhook, RuleStatus::Failed(ForwardError::Refused)),
        (socks, RuleStatus::Reconnecting { attempt: 2 }),
    ];
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.show_statuses(statuses, cx);
            if shot == ShotState::ForwardingForm {
                screen.select(Some(webhook), window, cx);
            } else if wizard_of(shot).is_some() {
                screen.new_wizard(window, cx);
            } else {
                screen.close_panel(cx);
            }
        });
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    if let Some((kind, step)) = wizard_of(shot) {
        settle(cx, handle).await?;
        let host = rules
            .iter()
            .find(|rule| rule.record.record_id == database)
            .map(|rule| rule.host_id);
        cx.update_window(handle, |_, window, cx| {
            let Some(form) = screen.read(cx).form().cloned() else {
                return;
            };
            form.update(cx, |form, cx| {
                form.set_kind(kind, window, cx);
                if step == Step::Type {
                    return;
                }
                if let Some(host) = host {
                    form.set_host(host, cx);
                }
                for (input, value) in [
                    (form.bind_port_input().clone(), "15432"),
                    (form.dest_address_input().clone(), "db.internal"),
                    (form.dest_port_input().clone(), "5432"),
                    (form.label_input().clone(), "Database"),
                ] {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
                while form.step() != Some(step) && form.next(cx) {}
            });
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    }
    settle(cx, handle).await
}

/// Checks that the window shows `shot`.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let wanted: &[&str] = match shot {
        ShotState::ForwardingEmpty => &["fwd-empty"],
        ShotState::ForwardingCards => &["fwd-grid"],
        ShotState::ForwardingForm => &["fwd-panel", "fwd-wide-warning", "fwd-panel-status"],
        ShotState::ForwardingWizardType => &["fwd-wizard-type", "fwd-diagram-local"],
        ShotState::ForwardingWizardRemote => &["fwd-wizard-type", "fwd-diagram-remote"],
        ShotState::ForwardingWizardDynamic => &["fwd-wizard-type", "fwd-diagram-dynamic"],
        ShotState::ForwardingWizardHost => &["fwd-wizard-host"],
        ShotState::ForwardingWizardListen => &["fwd-wizard-listen"],
        ShotState::ForwardingWizardDest => &["fwd-wizard-dest"],
        ShotState::ForwardingWizardSummary => &["fwd-wizard-summary", "fwd-wizard-line"],
        _ => return Err(not_reached(shot, "not a stage 5 state")),
    };
    for id in wanted {
        let shown = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(*id).is_some_and(|found| found.visible())
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
        if !shown {
            return Err(not_reached(shot, &format!("{id} is not shown")));
        }
    }
    Ok(())
}
