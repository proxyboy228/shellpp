//! Every color token of docs/design-system.md (sections 2 and 3) for both
//! interface themes, and the colors that have no field in gpui-kit's
//! `ThemeColor`: tile colors (`--tag-*`), port forwarding badges, status
//! dots, the `*-subtle` tints, `--text-muted`, `--text-disabled`, shadows.
//!
//! This file and `themes/graphite.json` are the only places with raw color
//! values (design-system 7.1). Views take colors from `cx.theme()` (for the
//! tokens mapped into gpui-kit, see [`super::THEME_COLOR_MAPPING`]) or from
//! [`super::tokens`] / the functions below; never from a hex literal.
//!
//! A unit test checks that `graphite.json` and these values agree.

use gpui::{Hsla, Rgba};
use shellpp_core::service::dto::TagColor;

/// One color: `0xRRGGBB` and an alpha byte (`0xFF` = opaque).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub rgb: u32,
    pub alpha: u8,
}

/// An opaque color.
pub const fn opaque(rgb: u32) -> Token {
    Token { rgb, alpha: 0xFF }
}

/// A translucent color (drawn over the surface below it).
pub const fn translucent(rgb: u32, alpha: u8) -> Token {
    Token { rgb, alpha }
}

impl Token {
    /// The GPUI color.
    pub fn hsla(self) -> Hsla {
        Rgba {
            r: ((self.rgb >> 16) & 0xFF) as f32 / 255.,
            g: ((self.rgb >> 8) & 0xFF) as f32 / 255.,
            b: (self.rgb & 0xFF) as f32 / 255.,
            a: self.alpha as f32 / 255.,
        }
        .into()
    }

    /// `#RRGGBB`, or `#RRGGBBAA` when translucent (the theme file format).
    pub fn hex(self) -> String {
        if self.alpha == 0xFF {
            format!("#{:06X}", self.rgb)
        } else {
            format!("#{:06X}{:02X}", self.rgb, self.alpha)
        }
    }
}

impl From<Token> for Hsla {
    fn from(token: Token) -> Self {
        token.hsla()
    }
}

/// The interface tokens of one theme (design-system 2.1-2.5), plus the few
/// derived colors the theme file needs (marked "derived").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceTokens {
    // 2.1 Surfaces
    pub bg_base: Token,
    pub bg_sidebar: Token,
    pub bg_surface: Token,
    pub bg_raised: Token,
    pub bg_raised_hover: Token,
    pub bg_pressed: Token,
    pub bg_input: Token,
    pub bg_overlay: Token,
    pub bg_overlay_hover: Token,
    pub scrim: Token,
    // 2.2 Borders
    pub border_subtle: Token,
    pub border_strong: Token,
    // 2.3 Text
    pub text_primary: Token,
    pub text_secondary: Token,
    pub text_muted: Token,
    pub text_disabled: Token,
    // 2.4 Accent
    pub accent: Token,
    pub accent_hover: Token,
    pub accent_pressed: Token,
    pub accent_subtle: Token,
    pub accent_text: Token,
    pub text_on_accent: Token,
    pub focus_ring: Token,
    // 2.5 Semantic colors
    pub danger: Token,
    pub danger_hover: Token,
    pub danger_pressed: Token,
    pub danger_subtle: Token,
    pub text_on_danger: Token,
    pub warning: Token,
    pub warning_subtle: Token,
    pub success: Token,
    pub success_subtle: Token,
    pub info: Token,
    pub info_subtle: Token,
    // 2.6 Shadows
    pub shadow_overlay: Shadow,
    pub shadow_modal: Shadow,
    // Derived (not named in the design system): text selection in inputs is
    // the terminal theme's selection (`--accent` at 30 % dark, 20 % light;
    // design-system 4.1, 4.2; gpui-kit caps it at 30 %); the scrollbar thumb is `--text-secondary` at 35 %
    // (55 % on hover); `plum_subtle` is `--tag-plum` at the `*-subtle` alpha.
    pub selection: Token,
    pub thumb: Token,
    pub thumb_hover: Token,
    pub plum_subtle: Token,
}

/// A drop shadow: `0 <y>px <blur>px <color>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shadow {
    pub y: u8,
    pub blur: u8,
    pub color: Token,
}

/// The dark theme (default).
pub const DARK: InterfaceTokens = InterfaceTokens {
    bg_base: opaque(0x121211),
    bg_sidebar: opaque(0x171716),
    bg_surface: opaque(0x1B1B19),
    bg_raised: opaque(0x242422),
    bg_raised_hover: opaque(0x2C2C29),
    bg_pressed: opaque(0x343431),
    bg_input: opaque(0x1E1E1C),
    bg_overlay: opaque(0x2A2A27),
    bg_overlay_hover: opaque(0x333330),
    // rgba(0,0,0,0.55)
    scrim: translucent(0x000000, 0x8C),
    border_subtle: opaque(0x2F2F2C),
    border_strong: opaque(0x3E3E3A),
    text_primary: opaque(0xEDEDE8),
    text_secondary: opaque(0xA6A59E),
    text_muted: opaque(0x76756F),
    text_disabled: opaque(0x55554F),
    accent: opaque(0x2EC4A6),
    accent_hover: opaque(0x4FD3B9),
    accent_pressed: opaque(0x23A68C),
    // rgba(46,196,166,0.14)
    accent_subtle: translucent(0x2EC4A6, 0x24),
    accent_text: opaque(0x5AD6BE),
    text_on_accent: opaque(0x08201B),
    focus_ring: opaque(0x2EC4A6),
    danger: opaque(0xF0665E),
    danger_hover: opaque(0xF57E77),
    danger_pressed: opaque(0xED4A41),
    // rgba(240,102,94,0.14)
    danger_subtle: translucent(0xF0665E, 0x24),
    text_on_danger: opaque(0x08201B),
    warning: opaque(0xE3B04B),
    warning_subtle: translucent(0xE3B04B, 0x24),
    success: opaque(0x7CC46F),
    success_subtle: translucent(0x7CC46F, 0x24),
    info: opaque(0x7AA9DB),
    info_subtle: translucent(0x7AA9DB, 0x24),
    // 0 8px 24px rgba(0,0,0,0.50); 0 16px 48px rgba(0,0,0,0.60)
    shadow_overlay: Shadow {
        y: 8,
        blur: 24,
        color: translucent(0x000000, 0x80),
    },
    shadow_modal: Shadow {
        y: 16,
        blur: 48,
        color: translucent(0x000000, 0x99),
    },
    selection: translucent(0x2EC4A6, 0x4D),
    thumb: translucent(0xA6A59E, 0x59),
    thumb_hover: translucent(0xA6A59E, 0x8C),
    plum_subtle: translucent(0x85549A, 0x24),
};

/// The light theme.
pub const LIGHT: InterfaceTokens = InterfaceTokens {
    bg_base: opaque(0xF2F1EC),
    bg_sidebar: opaque(0xECEBE5),
    bg_surface: opaque(0xF8F7F3),
    bg_raised: opaque(0xFFFFFF),
    bg_raised_hover: opaque(0xF0EFEA),
    bg_pressed: opaque(0xEDECE6),
    bg_input: opaque(0xF6F5F1),
    bg_overlay: opaque(0xFFFFFF),
    bg_overlay_hover: opaque(0xF0EFEA),
    // rgba(28,28,26,0.35)
    scrim: translucent(0x1C1C1A, 0x59),
    border_subtle: opaque(0xE2E0D9),
    border_strong: opaque(0xCBC8BF),
    text_primary: opaque(0x1C1C1A),
    text_secondary: opaque(0x5A5953),
    text_muted: opaque(0x88867F),
    text_disabled: opaque(0xB3B1A9),
    accent: opaque(0x13856E),
    accent_hover: opaque(0x0F7461),
    accent_pressed: opaque(0x0B6051),
    // rgba(19,133,110,0.10)
    accent_subtle: translucent(0x13856E, 0x1A),
    accent_text: opaque(0x0F7461),
    text_on_accent: opaque(0xFFFFFF),
    focus_ring: opaque(0x13856E),
    danger: opaque(0xC0392F),
    danger_hover: opaque(0xA33027),
    danger_pressed: opaque(0x852720),
    // rgba(192,57,47,0.10)
    danger_subtle: translucent(0xC0392F, 0x1A),
    text_on_danger: opaque(0xFFFFFF),
    warning: opaque(0x8D620B),
    warning_subtle: translucent(0x8D620B, 0x1A),
    success: opaque(0x36782F),
    success_subtle: translucent(0x36782F, 0x1A),
    info: opaque(0x2E6AA6),
    info_subtle: translucent(0x2E6AA6, 0x1A),
    // 0 8px 24px rgba(28,28,26,0.14); 0 16px 48px rgba(28,28,26,0.18)
    shadow_overlay: Shadow {
        y: 8,
        blur: 24,
        color: translucent(0x1C1C1A, 0x24),
    },
    shadow_modal: Shadow {
        y: 16,
        blur: 48,
        color: translucent(0x1C1C1A, 0x2E),
    },
    selection: translucent(0x13856E, 0x33),
    thumb: translucent(0x5A5953, 0x59),
    thumb_hover: translucent(0x5A5953, 0x8C),
    plum_subtle: translucent(0x85549A, 0x1A),
};

// ---------------------------------------------------------------------------
// 3.1 Host and group labels: the same in both themes.
// ---------------------------------------------------------------------------

pub const TAG_GRAPHITE: Token = opaque(0x5B5A55);
pub const TAG_JADE: Token = opaque(0x1D8571);
pub const TAG_AMBER: Token = opaque(0x9D6C1E);
pub const TAG_CORAL: Token = opaque(0xB8503F);
pub const TAG_PLUM: Token = opaque(0x85549A);
pub const TAG_MOSS: Token = opaque(0x5A7F35);
pub const TAG_OCEAN: Token = opaque(0x3A6890);
pub const TAG_ROSE: Token = opaque(0xA84B70);
/// Icons on tiles and letters on badges (`--text-on-tag`).
pub const TEXT_ON_TAG: Token = opaque(0xFFFFFF);

/// The eight tile colors in picker order (graphite first, the default).
pub const TAG_COLORS: [TagColor; 8] = [
    TagColor::Graphite,
    TagColor::Jade,
    TagColor::Amber,
    TagColor::Coral,
    TagColor::Plum,
    TagColor::Moss,
    TagColor::Ocean,
    TagColor::Rose,
];

/// The tile color of a host or group (`--tag-*`).
pub const fn tag_color(color: TagColor) -> Token {
    match color {
        TagColor::Graphite => TAG_GRAPHITE,
        TagColor::Jade => TAG_JADE,
        TagColor::Amber => TAG_AMBER,
        TagColor::Coral => TAG_CORAL,
        TagColor::Plum => TAG_PLUM,
        TagColor::Moss => TAG_MOSS,
        TagColor::Ocean => TAG_OCEAN,
        TagColor::Rose => TAG_ROSE,
    }
}

// ---------------------------------------------------------------------------
// 3.2 Port forwarding type badges
// ---------------------------------------------------------------------------

/// Port forwarding rule types (UI 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardingKind {
    Local,
    Remote,
    Dynamic,
}

/// Badge tile color: L jade, R plum, D amber (white letter on it).
pub const fn forwarding_badge(kind: ForwardingKind) -> Token {
    match kind {
        ForwardingKind::Local => TAG_JADE,
        ForwardingKind::Remote => TAG_PLUM,
        ForwardingKind::Dynamic => TAG_AMBER,
    }
}

/// The badge letter.
pub const fn forwarding_letter(kind: ForwardingKind) -> &'static str {
    match kind {
        ForwardingKind::Local => "L",
        ForwardingKind::Remote => "R",
        ForwardingKind::Dynamic => "D",
    }
}

// ---------------------------------------------------------------------------
// 3.3 Status indicators
// ---------------------------------------------------------------------------

/// Status dot meanings (8 px dot, `--size-status-dot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Connected or running: `--success`.
    Connected,
    /// Connecting: `--warning`.
    Connecting,
    /// Error or connection drop: `--danger`.
    Error,
    /// Inactive: `--text-muted`.
    Inactive,
}

/// The dot color of a status in a theme.
pub const fn status_color(status: Status, tokens: &InterfaceTokens) -> Token {
    match status {
        Status::Connected => tokens.success,
        Status::Connecting => tokens.warning,
        Status::Error => tokens.danger,
        Status::Inactive => tokens.text_muted,
    }
}

/// Looks a token up by its design-system name without the `--` prefix
/// (`"bg-raised"`, `"tag-jade"`, `"text-on-tag"`), plus the derived names
/// `selection`, `thumb`, `thumb-hover`, `plum-subtle` and `transparent`.
pub fn token_by_name(name: &str, tokens: &InterfaceTokens) -> Option<Token> {
    let token = match name {
        "bg-base" => tokens.bg_base,
        "bg-sidebar" => tokens.bg_sidebar,
        "bg-surface" => tokens.bg_surface,
        "bg-raised" => tokens.bg_raised,
        "bg-raised-hover" => tokens.bg_raised_hover,
        "bg-pressed" => tokens.bg_pressed,
        "bg-input" => tokens.bg_input,
        "bg-overlay" => tokens.bg_overlay,
        "bg-overlay-hover" => tokens.bg_overlay_hover,
        "scrim" => tokens.scrim,
        "border-subtle" => tokens.border_subtle,
        "border-strong" => tokens.border_strong,
        "text-primary" => tokens.text_primary,
        "text-secondary" => tokens.text_secondary,
        "text-muted" => tokens.text_muted,
        "text-disabled" => tokens.text_disabled,
        "accent" => tokens.accent,
        "accent-hover" => tokens.accent_hover,
        "accent-pressed" => tokens.accent_pressed,
        "accent-subtle" => tokens.accent_subtle,
        "accent-text" => tokens.accent_text,
        "text-on-accent" => tokens.text_on_accent,
        "focus-ring" => tokens.focus_ring,
        "danger" => tokens.danger,
        "danger-hover" => tokens.danger_hover,
        "danger-pressed" => tokens.danger_pressed,
        "danger-subtle" => tokens.danger_subtle,
        "text-on-danger" => tokens.text_on_danger,
        "warning" => tokens.warning,
        "warning-subtle" => tokens.warning_subtle,
        "success" => tokens.success,
        "success-subtle" => tokens.success_subtle,
        "info" => tokens.info,
        "info-subtle" => tokens.info_subtle,
        "selection" => tokens.selection,
        "thumb" => tokens.thumb,
        "thumb-hover" => tokens.thumb_hover,
        "plum-subtle" => tokens.plum_subtle,
        "transparent" => translucent(0x000000, 0x00),
        "tag-graphite" => TAG_GRAPHITE,
        "tag-jade" => TAG_JADE,
        "tag-amber" => TAG_AMBER,
        "tag-coral" => TAG_CORAL,
        "tag-plum" => TAG_PLUM,
        "tag-moss" => TAG_MOSS,
        "tag-ocean" => TAG_OCEAN,
        "tag-rose" => TAG_ROSE,
        "text-on-tag" => TEXT_ON_TAG,
        _ => return None,
    };
    Some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_format() {
        assert_eq!(DARK.bg_base.hex(), "#121211");
        assert_eq!(DARK.accent_subtle.hex(), "#2EC4A624");
        assert_eq!(LIGHT.accent_subtle.hex(), "#13856E1A");
    }

    #[test]
    fn hsla_conversion_keeps_alpha() {
        let color = DARK.accent_subtle.hsla();
        assert!((color.a - 0x24 as f32 / 255.).abs() < 1e-6);
        assert!((TEXT_ON_TAG.hsla().l - 1.).abs() < 1e-6);
    }

    #[test]
    fn graphite_is_the_default_tile_color() {
        assert_eq!(tag_color(TagColor::default()), TAG_GRAPHITE);
        assert_eq!(TAG_COLORS[0], TagColor::Graphite);
        assert_eq!(TAG_COLORS.len(), 8);
    }

    #[test]
    fn badges_and_statuses() {
        assert_eq!(forwarding_badge(ForwardingKind::Local), TAG_JADE);
        assert_eq!(forwarding_badge(ForwardingKind::Remote), TAG_PLUM);
        assert_eq!(forwarding_badge(ForwardingKind::Dynamic), TAG_AMBER);
        assert_eq!(forwarding_letter(ForwardingKind::Dynamic), "D");
        assert_eq!(status_color(Status::Connected, &DARK), DARK.success);
        assert_eq!(status_color(Status::Inactive, &LIGHT), LIGHT.text_muted);
    }
}
