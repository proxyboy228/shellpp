//! Test: the contrast pairs of docs/design-system.md 7.3 in both interface
//! themes and every built-in terminal theme. WCAG 2.x contrast ratio,
//! translucent colors drawn over the surface below them, compared with the
//! threshold without rounding.
//!
//! The pair list mirrors the tables of 7.3; change both together.

use super::palette::{self, DARK, InterfaceTokens, LIGHT, Token};
use super::terminal::{self, TerminalTheme};

/// An opaque color as linear-free sRGB floats in 0..=255.
#[derive(Clone, Copy, Debug)]
struct Rgb(f64, f64, f64);

impl Rgb {
    fn opaque(token: Token) -> Self {
        assert_eq!(token.alpha, 0xFF, "{} must be opaque here", token.hex());
        Self::channels(token)
    }

    fn channels(token: Token) -> Self {
        Rgb(
            f64::from((token.rgb >> 16) & 0xFF),
            f64::from((token.rgb >> 8) & 0xFF),
            f64::from(token.rgb & 0xFF),
        )
    }

    /// `token` drawn over the opaque `below`.
    fn over(token: Token, below: Rgb) -> Self {
        let alpha = f64::from(token.alpha) / 255.;
        let top = Self::channels(token);
        Rgb(
            top.0 * alpha + below.0 * (1. - alpha),
            top.1 * alpha + below.1 * (1. - alpha),
            top.2 * alpha + below.2 * (1. - alpha),
        )
    }

    fn luminance(self) -> f64 {
        let channel = |value: f64| {
            let value = value / 255.;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.0) + 0.7152 * channel(self.1) + 0.0722 * channel(self.2)
    }
}

fn ratio(a: Rgb, b: Rgb) -> f64 {
    let (a, b) = (a.luminance(), b.luminance());
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// Collects failures instead of stopping at the first one.
#[derive(Default)]
struct Report {
    checked: usize,
    failures: Vec<String>,
}

impl Report {
    fn check(&mut self, label: String, foreground: Rgb, background: Rgb, threshold: f64) {
        self.checked += 1;
        let value = ratio(foreground, background);
        if value < threshold {
            self.failures
                .push(format!("{label}: {value:.4}:1 < {threshold}:1"));
        }
    }

    fn finish(self, minimum: usize) {
        assert!(
            self.failures.is_empty(),
            "contrast pairs below threshold:\n{}",
            self.failures.join("\n")
        );
        assert!(
            self.checked >= minimum,
            "only {} pairs checked",
            self.checked
        );
    }
}

fn base_surfaces(t: &InterfaceTokens) -> [(&'static str, Token); 7] {
    [
        ("bg-base", t.bg_base),
        ("bg-sidebar", t.bg_sidebar),
        ("bg-surface", t.bg_surface),
        ("bg-raised", t.bg_raised),
        ("bg-raised-hover", t.bg_raised_hover),
        ("bg-input", t.bg_input),
        ("bg-overlay", t.bg_overlay),
    ]
}

fn state_surfaces(t: &InterfaceTokens) -> [(&'static str, Token); 2] {
    [
        ("bg-pressed", t.bg_pressed),
        ("bg-overlay-hover", t.bg_overlay_hover),
    ]
}

fn subtles(t: &InterfaceTokens) -> [(&'static str, Token, Token); 5] {
    // (name, tint, the color whose tint it is)
    [
        ("accent-subtle", t.accent_subtle, t.accent),
        ("danger-subtle", t.danger_subtle, t.danger),
        ("warning-subtle", t.warning_subtle, t.warning),
        ("success-subtle", t.success_subtle, t.success),
        ("info-subtle", t.info_subtle, t.info),
    ]
}

fn interface_pairs(mode: &str, t: &InterfaceTokens, report: &mut Report) {
    let base = base_surfaces(t);
    let all: Vec<(&str, Token)> = base.iter().copied().chain(state_surfaces(t)).collect();
    let mut pairs = |group: &str, fg: &[(&str, Token)], bg: &[(&str, Token)], threshold: f64| {
        for (fg_name, fg_token) in fg {
            for (bg_name, bg_token) in bg {
                report.check(
                    format!("{mode} {group}: --{fg_name} on --{bg_name}"),
                    Rgb::opaque(*fg_token),
                    Rgb::opaque(*bg_token),
                    threshold,
                );
            }
        }
    };

    // Text.
    let text = [
        ("text-primary", t.text_primary),
        ("text-secondary", t.text_secondary),
        ("accent-text", t.accent_text),
        ("danger", t.danger),
        ("danger-hover", t.danger_hover),
        ("warning", t.warning),
        ("success", t.success),
        ("info", t.info),
    ];
    pairs("text", &text, &base, 4.5);
    // Text on state surfaces (plain --danger is not placed there).
    let state_text: Vec<(&str, Token)> = text
        .iter()
        .copied()
        .filter(|(name, _)| *name != "danger")
        .collect();
    pairs(
        "text on state surfaces",
        &state_text,
        &state_surfaces(t),
        4.5,
    );
    // Buttons.
    pairs(
        "primary button",
        &[("text-on-accent", t.text_on_accent)],
        &[
            ("accent", t.accent),
            ("accent-hover", t.accent_hover),
            ("accent-pressed", t.accent_pressed),
        ],
        4.5,
    );
    pairs(
        "danger button",
        &[("text-on-danger", t.text_on_danger)],
        &[
            ("danger", t.danger),
            ("danger-hover", t.danger_hover),
            ("danger-pressed", t.danger_pressed),
        ],
        4.5,
    );
    // Badge letters, tile icons.
    let on_tag = [("text-on-tag", palette::TEXT_ON_TAG)];
    pairs(
        "badge letters",
        &on_tag,
        &[
            ("tag-jade", palette::TAG_JADE),
            ("tag-plum", palette::TAG_PLUM),
            ("tag-amber", palette::TAG_AMBER),
        ],
        4.5,
    );
    let tags: Vec<(&str, Token)> = palette::TAG_COLORS
        .iter()
        .map(|color| ("tag", palette::tag_color(*color)))
        .collect();
    pairs("tile icons", &on_tag, &tags, 3.);
    // Muted text and status dots.
    pairs("muted text", &[("text-muted", t.text_muted)], &base, 3.);
    pairs(
        "status dots",
        &[
            ("success", t.success),
            ("warning", t.warning),
            ("danger", t.danger),
            ("text-muted", t.text_muted),
        ],
        &base,
        3.,
    );
    // Focus ring and accent indicators.
    pairs(
        "focus ring and accent indicators",
        &[("focus-ring", t.focus_ring), ("accent", t.accent)],
        &all,
        3.,
    );
    // Progress.
    pairs(
        "progress",
        &[("accent", t.accent)],
        &[("border-subtle", t.border_subtle)],
        3.,
    );

    // Tinted backgrounds: translucent tints over each base surface.
    for (surface_name, surface) in base {
        let below = Rgb::opaque(surface);
        for (tint_name, tint, own) in subtles(t) {
            let background = Rgb::over(tint, below);
            report.check(
                format!(
                    "{mode} text on tint: --text-primary on --{tint_name} over --{surface_name}"
                ),
                Rgb::opaque(t.text_primary),
                background,
                4.5,
            );
            report.check(
                format!("{mode} icon on tint: own color on --{tint_name} over --{surface_name}"),
                Rgb::opaque(own),
                background,
                3.,
            );
        }
    }
    // Secondary text in a selection.
    for (surface_name, surface) in [
        ("bg-sidebar", t.bg_sidebar),
        ("bg-surface", t.bg_surface),
        ("bg-raised", t.bg_raised),
        ("bg-overlay", t.bg_overlay),
    ] {
        report.check(
            format!("{mode} selection: --text-secondary on --accent-subtle over --{surface_name}"),
            Rgb::opaque(t.text_secondary),
            Rgb::over(t.accent_subtle, Rgb::opaque(surface)),
            4.5,
        );
    }
}

fn terminal_pairs(theme: &TerminalTheme, report: &mut Report) {
    let name = &theme.name;
    let background = Rgb::opaque(theme.background);
    let foreground = Rgb::opaque(theme.foreground);
    report.check(format!("{name}: foreground"), foreground, background, 4.5);
    for index in (1..=6).chain(9..=14) {
        report.check(
            format!("{name}: ansi {}", terminal::AnsiColors::KEYS[index]),
            Rgb::opaque(theme.ansi.0[index]),
            background,
            4.5,
        );
    }
    report.check(
        format!("{name}: brightBlack"),
        Rgb::opaque(theme.ansi.0[8]),
        background,
        3.,
    );
    let selected = theme.selection_foreground.unwrap_or(theme.foreground);
    report.check(
        format!("{name}: selected text"),
        Rgb::opaque(selected),
        Rgb::over(theme.selection, background),
        4.5,
    );
    report.check(
        format!("{name}: text under the cursor"),
        Rgb::opaque(theme.cursor_text),
        Rgb::opaque(theme.cursor),
        4.5,
    );
    report.check(
        format!("{name}: cursor"),
        Rgb::opaque(theme.cursor),
        background,
        3.,
    );
    for (label, color) in [
        ("searchMatch", theme.search_match),
        ("searchMatchActive", theme.search_match_active),
    ] {
        report.check(
            format!("{name}: foreground on {label}"),
            foreground,
            Rgb::opaque(color),
            4.5,
        );
    }
    for (index, color) in palette::keyword_colors(theme.background)
        .all()
        .into_iter()
        .enumerate()
    {
        report.check(
            format!("{name}: keyword color {index}"),
            Rgb::opaque(color),
            background,
            4.5,
        );
    }
}

#[test]
fn interface_contrast_pairs_hold_in_both_themes() {
    let mut report = Report::default();
    interface_pairs("dark", &DARK, &mut report);
    interface_pairs("light", &LIGHT, &mut report);
    report.finish(2 * 200);
}

/// Only our own themes, Graphite and Paper (the first two entries of the
/// registry): the bundled open schemes of design-system 4.4 keep their
/// original colors, some of which are below these thresholds (Solarized
/// body text, for one); the "Adjust indistinguishable colors" setting
/// (UI 12.4, `crate::terminal::contrast`) helps there instead.
#[test]
fn terminal_contrast_pairs_hold_for_builtin_themes() {
    let mut report = Report::default();
    let ours = &terminal::registry()[..2];
    assert_eq!(
        ours.iter().map(|entry| entry.id()).collect::<Vec<_>>(),
        ["graphite", "paper"]
    );
    for entry in ours {
        terminal_pairs(&entry.theme, &mut report);
    }
    report.finish(2 * 25);
}

/// The ratios the design system quotes, to pin the formula.
#[test]
fn formula_matches_the_quoted_ratios() {
    let white = Rgb::opaque(palette::opaque(0xFFFFFF));
    // "white text on it is 4.4994:1" (old light accent #13866F).
    let old = ratio(white, Rgb::opaque(palette::opaque(0x13866F)));
    assert!((old - 4.4994).abs() < 1e-4, "{old}");
    // "On #13856E it is 4.56:1".
    let new = ratio(white, Rgb::opaque(LIGHT.accent));
    assert!((new - 4.56).abs() < 5e-3, "{new}");
}
