//! The sidebar of the Vaults tab (UI 1.2, design-system 2.1, 6): six
//! sections, 220 px (`--width-sidebar`), `--bg-sidebar` with a
//! `--border-subtle` line on the right, no vault header (the vault is chosen
//! on the Vaults tab). Items are 36 px, radius 6: `--text-secondary`, hover
//! `--bg-raised-hover`, pressed `--bg-pressed`; the active one has the
//! `--accent-subtle` background and `--text-primary`. Keyboard focus shows
//! the design-system focus ring. Sections without a screen yet open the
//! "In development" placeholder.
//!
//! Narrow windows (owner's step 0.5): when the main area next to the full
//! sidebar would be narrower than 600 px (the details panel open in a
//! narrow window), the sidebar collapses into a 56 px strip of the same
//! items as icons, each with its name as a tooltip and the same active
//! state; it expands again as soon as the window is wide enough
//! ([`is_collapsed`]).

use gpui::prelude::*;
use gpui::{App, ClickEvent, FocusHandle, IntoElement, Pixels, SharedString, Window, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};

use crate::i18n;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

/// Width of the collapsed sidebar: the strip of icons (owner's step 0.5).
pub const WIDTH_COLLAPSED: Pixels = px(56.);
/// The narrowest main area next to the full sidebar; below it the sidebar
/// collapses (owner's step 0.5).
pub const MAIN_AREA_COLLAPSE: Pixels = px(600.);

/// Whether the sidebar is the strip of icons in a `window` px wide window
/// with the details panel open at its chosen width `details` (`None`:
/// closed): the main area next to the full sidebar would be narrower than
/// [`MAIN_AREA_COLLAPSE`]. Measured against the full sidebar, so the
/// collapse itself never flips the decision back, and the sidebar expands
/// as soon as the full one leaves the main area 600 px.
pub fn is_collapsed(window: Pixels, details: Option<Pixels>) -> bool {
    let panel = details.map_or(Pixels::ZERO, |width| {
        super::details::fitted_width(width, window, metrics::WIDTH_SIDEBAR)
    });
    window - metrics::WIDTH_SIDEBAR - panel < MAIN_AREA_COLLAPSE
}

/// The sidebar's width: full or collapsed.
pub fn width(collapsed: bool) -> Pixels {
    if collapsed {
        WIDTH_COLLAPSED
    } else {
        metrics::WIDTH_SIDEBAR
    }
}

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

    /// Stable element id of the item's tooltip in the collapsed sidebar
    /// (tests).
    pub fn tooltip_id(self) -> SharedString {
        format!("{}-tooltip", self.element_id()).into()
    }

    /// Whether the section has a screen yet (the others show the placeholder).
    pub fn is_built(self) -> bool {
        matches!(self, Section::Hosts)
    }
}

/// The sidebar with `active` highlighted; `focus` holds the focus handles of
/// the items in [`Section::ALL`] order; `on_select` runs on a click (or
/// Enter / Space on a focused item). `collapsed`: the strip of icons, the
/// names in tooltips ([`is_collapsed`]).
pub fn sidebar(
    active: Section,
    focus: &[FocusHandle; 6],
    collapsed: bool,
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
            .flex_none()
            .px(metrics::SPACE_3)
            .gap(metrics::SPACE_2)
            .items_center()
            .when(collapsed, |item| {
                let label = section.label();
                let id = section.tooltip_id();
                item.justify_center().tooltip(move |window, cx| {
                    let id = id.clone();
                    Tooltip::element(move |_, _| div().id(id.clone()).test_support().child(label))
                        .build(window, cx)
                })
            })
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
            .when(!collapsed, |item| {
                item.child(div().flex_1().min_w_0().truncate().child(section.label()))
            })
            .on_click(move |_: &ClickEvent, window, cx| on_select(section, window, cx))
    });
    v_flex()
        .id("sidebar")
        .test_support()
        .flex_none()
        .w(width(collapsed))
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

    #[test]
    fn the_sidebar_collapses_below_a_600_px_main_area() {
        // Without the details panel the minimum window (960 px) leaves 740.
        assert!(!is_collapsed(px(960.), None));
        // The 380 px panel at 960 px leaves 360: the strip of icons.
        assert!(is_collapsed(px(960.), Some(metrics::WIDTH_DETAILS)));
        // It expands once the full sidebar leaves the main area 600 px:
        // 220 + 380 + 600 = 1200.
        assert!(is_collapsed(px(1199.), Some(metrics::WIDTH_DETAILS)));
        assert!(!is_collapsed(px(1200.), Some(metrics::WIDTH_DETAILS)));
        // A wider panel collapses it in a wider window.
        assert!(is_collapsed(px(1300.), Some(metrics::WIDTH_DETAILS_MAX)));
        assert!(!is_collapsed(px(1380.), Some(metrics::WIDTH_DETAILS_MAX)));
        assert_eq!(width(true), px(56.));
        assert_eq!(width(false), metrics::WIDTH_SIDEBAR);
    }
}
