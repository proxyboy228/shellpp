//! The traffic diagram of the wizard's first step (UI 8.3): our own
//! drawing of the participants (this computer, the firewall, the SSH host,
//! the service or the internet), which way the forwarded connections go,
//! and where the port opens.
//!
//! - Local: this computer (the port) → firewall → SSH host → service.
//! - Remote: service ← this computer ← firewall ← SSH host (the port).
//! - Dynamic: this computer (SOCKS) → firewall → SSH host → internet.

use gpui::prelude::*;
use gpui::{AnyElement, App, IntoElement, SharedString, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use shellpp_core::service::snippets::ForwardType;

use super::strings;
use crate::theme::metrics;

/// A participant: its icon, its name, and the mark of the port it holds.
struct Node {
    icon: Lucide,
    name: &'static str,
    mark: Option<&'static str>,
}

fn node(node: Node, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let tokens = crate::theme::tokens(cx);
    v_flex()
        .flex_none()
        .w(px(64.))
        .items_center()
        .gap(metrics::SPACE_1)
        .child(
            div()
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(metrics::RADIUS_CONTROL)
                .border_1()
                .border_color(if node.mark.is_some() {
                    theme.primary
                } else {
                    theme.border
                })
                .bg(theme.secondary)
                .child(Icon::new(node.icon).size(px(18.))),
        )
        .child(
            div()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(theme.foreground)
                .text_center()
                .child(node.name),
        )
        .child(
            div()
                .h(px(14.))
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(tokens.accent_text.hsla())
                .children(node.mark),
        )
        .into_any_element()
}

/// The arrow between two participants: the way the connections go.
fn arrow(leftward: bool, cx: &App) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(12.))
        .pt(px(9.))
        .flex()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child(
            Icon::new(if leftward {
                Lucide::ArrowLeft
            } else {
                Lucide::ArrowRight
            })
            .size(px(16.)),
        )
        .into_any_element()
}

/// The diagram of `kind`; its element id names the type
/// (`fwd-diagram-local`, `-remote`, `-dynamic`).
pub fn diagram(kind: ForwardType, cx: &App) -> AnyElement {
    let computer = |mark| Node {
        icon: Lucide::Monitor,
        name: strings::NODE_COMPUTER,
        mark,
    };
    let firewall = Node {
        icon: Lucide::BrickWall,
        name: strings::NODE_FIREWALL,
        mark: None,
    };
    let host = |mark| Node {
        icon: Lucide::Server,
        name: strings::NODE_HOST,
        mark,
    };
    let service = Node {
        icon: Lucide::Database,
        name: strings::NODE_SERVICE,
        mark: None,
    };
    let (nodes, leftward, name) = match kind {
        ForwardType::Local => (
            [
                computer(Some(strings::PORT_MARK)),
                firewall,
                host(None),
                service,
            ],
            false,
            "local",
        ),
        ForwardType::Remote => (
            [
                service,
                computer(None),
                firewall,
                host(Some(strings::PORT_MARK)),
            ],
            true,
            "remote",
        ),
        ForwardType::Dynamic => (
            [
                computer(Some(strings::SOCKS_MARK)),
                firewall,
                host(None),
                Node {
                    icon: Lucide::Globe,
                    name: strings::NODE_INTERNET,
                    mark: None,
                },
            ],
            false,
            "dynamic",
        ),
    };
    let mut row = h_flex()
        .id(SharedString::from(format!("fwd-diagram-{name}")))
        .test_support()
        .w_full()
        .items_start()
        .py(metrics::SPACE_2);
    let count = nodes.len();
    for (index, participant) in nodes.into_iter().enumerate() {
        row = row.child(node(participant, cx));
        if index + 1 < count {
            row = row.child(arrow(leftward, cx));
        }
    }
    row.into_any_element()
}
