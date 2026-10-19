//! The sidebar of the Vaults tab (UI 1.2, design-system 2.1, 6): six
//! sections, 220 px (`--width-sidebar`), `--bg-sidebar` with a
//! `--border-subtle` line on the right, no vault header (the vault is chosen
//! on the Vaults tab). Items are 36 px, radius 6: `--text-secondary`, hover
//! `--bg-raised-hover`, pressed `--bg-pressed`; the active one has the
//! `--accent-subtle` background and `--text-primary`. Keyboard focus shows
//! the design-system focus ring. Sections without a screen yet open the
//! "In development" placeholder.

use gpui::prelude::*;
use gpui::{App, ClickEvent, FocusHandle, IntoElement, SharedString, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};

use crate::i18n;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

/// A sidebar section, top to bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Section {
    #[default]
    Hosts,
    Keychain,
    PortForwarding,
    Snippets,
    KnownHosts,
    Logs,
}

impl Section {
    /// All sections in sidebar order.
    pub const ALL: [Section; 6] = [
        Section::Hosts,
        Section::Keychain,
        Section::PortForwarding,
        Section::Snippets,
        Section::KnownHosts,
        Section::Logs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Hosts => i18n::nav::HOSTS,
            Section::Keychain => i18n::nav::KEYCHAIN,
            Section::PortForwarding => i18n::nav::PORT_FORWARDING,
            Section::Snippets => i18n::nav::SNIPPETS,
            Section::KnownHosts => i18n::nav::KNOWN_HOSTS,
            Section::Logs => i18n::nav::LOGS,
        }
    }

    /// Icons by meaning (Lucide): server, key, arrow-left-right, braces,
    /// fingerprint (`fingerprint-pattern` in current Lucide), scroll-text.
    pub fn icon(self) -> Lucide {
        match self {
            Section::Hosts => Lucide::Server,
            Section::Keychain => Lucide::Key,
            Section::PortForwarding => Lucide::ArrowLeftRight,
            Section::Snippets => Lucide::Braces,
            Section::KnownHosts => Lucide::FingerprintPattern,
            Section::Logs => Lucide::ScrollText,
        }
    }

    /// Stable element id of the sidebar item (tests).
    pub fn element_id(self) -> SharedString {
        let id = match self {
            Section::Hosts => "nav-hosts",
            Section::Keychain => "nav-keychain",
            Section::PortForwarding => "nav-port-forwarding",
            Section::Snippets => "nav-snippets",
            Section::KnownHosts => "nav-known-hosts",
            Section::Logs => "nav-logs",
        };
        id.into()
    }

    /// Whether the section has a screen yet (the others show the placeholder).
    pub fn is_built(self) -> bool {
        matches!(self, Section::Hosts)
    }
}

/// The sidebar with `active` highlighted; `focus` holds the focus handles of
/// the items in [`Section::ALL`] order; `on_select` runs on a click (or
/// Enter / Space on a focused item).
pub fn sidebar(
    active: Section,
    focus: &[FocusHandle; 6],
    on_select: impl Fn(Section, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let surface = theme.sidebar;
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    let items = Section::ALL.into_iter().zip(focus).map(|(section, focus)| {
        let selected = section == active;
        let on_select = on_select.clone();
        h_flex()
            .id(section.element_id())
            .test_support()
            .h(metrics::HEIGHT_CONTROL)
            .px(metrics::SPACE_3)
            .gap(metrics::SPACE_2)
            .items_center()
            .rounded(metrics::RADIUS_CONTROL)
            .cursor_pointer()
            .track_focus(focus)
            .tab_index(0)
            .focus_ring_visible(surface, cx)
            .text_size(metrics::FONT_SIZE_BODY)
            .when(selected, |item| {
                item.bg(tokens.accent_subtle.hsla())
                    .text_color(theme.sidebar_accent_foreground)
                    .font_weight(metrics::FONT_WEIGHT_MEDIUM)
            })
            .when(!selected, |item| {
                item.text_color(theme.muted_foreground)
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            })
            .child(Icon::new(section.icon()).size(gpui::px(16.)))
            .child(div().flex_1().min_w_0().truncate().child(section.label()))
            .on_click(move |_: &ClickEvent, window, cx| on_select(section, window, cx))
    });
    v_flex()
        .id("sidebar")
        .test_support()
        .flex_none()
        .w(metrics::WIDTH_SIDEBAR)
        .h_full()
        .p(metrics::SPACE_2)
        .gap(metrics::SPACE_1)
        .bg(surface)
        .border_r_1()
        .border_color(theme.sidebar_border)
        .children(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_sections_in_spec_order_with_their_icons() {
        let labels: Vec<&str> = Section::ALL.iter().map(|section| section.label()).collect();
        assert_eq!(
            labels,
            [
                "Hosts",
                "Keychain",
                "Port Forwarding",
                "Snippets",
                "Known Hosts",
                "Logs"
            ]
        );
        let icons: Vec<Lucide> = Section::ALL.iter().map(|section| section.icon()).collect();
        assert_eq!(
            icons,
            [
                Lucide::Server,
                Lucide::Key,
                Lucide::ArrowLeftRight,
                Lucide::Braces,
                Lucide::FingerprintPattern,
                Lucide::ScrollText
            ]
        );
        assert!(Section::Hosts.is_built());
        assert!(Section::ALL[1..].iter().all(|section| !section.is_built()));
    }
}
