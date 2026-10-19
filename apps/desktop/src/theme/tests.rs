use std::collections::BTreeSet;

use gpui::{Hsla, TestAppContext};
use gpui_kit::component::{ActiveTheme as _, ThemeColor};
use serde_json::Value;

use super::palette::{DARK, InterfaceTokens, LIGHT, token_by_name};
use super::*;

fn theme_file() -> Value {
    serde_json::from_str(THEME_FILE).expect("graphite.json is JSON")
}

fn theme_colors<'a>(file: &'a Value, name: &str) -> &'a serde_json::Map<String, Value> {
    file["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .find(|theme| theme["name"] == name)
        .unwrap_or_else(|| panic!("{name} missing"))["colors"]
        .as_object()
        .expect("colors")
}

/// graphite.json, palette.rs and THEME_COLOR_MAPPING agree, in both modes.
#[test]
fn theme_file_matches_the_tokens() {
    let file = theme_file();
    for (name, mode, tokens) in [
        (DARK_THEME_NAME, "dark", &DARK),
        (LIGHT_THEME_NAME, "light", &LIGHT),
    ] {
        let theme = file["themes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|theme| theme["name"] == name)
            .unwrap();
        assert_eq!(theme["mode"], mode);
        let colors = theme_colors(&file, name);
        let keys: BTreeSet<&str> = colors.keys().map(String::as_str).collect();
        let mapped: BTreeSet<&str> = THEME_COLOR_MAPPING.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, mapped, "{name}: theme keys and mapping differ");
        for (key, token) in THEME_COLOR_MAPPING {
            let expected = token_by_name(token, tokens)
                .unwrap_or_else(|| panic!("unknown token {token}"))
                .hex();
            assert_eq!(
                colors[*key].as_str().map(str::to_ascii_uppercase),
                Some(expected),
                "{name}: {key} should be --{token}"
            );
        }
    }
}

/// Every `ThemeColor` field holds one of our colors once the theme is
/// installed: no gpui-kit default color leaks through a field the mapping
/// forgot. (The file has one key more than `ThemeColor` has fields:
/// `group_box.title.foreground` has no field of its own.)
#[gpui_kit::test]
fn every_theme_color_field_is_ours(cx: &mut TestAppContext) {
    let fields = serde_json::to_value(ThemeColor::default()).expect("ThemeColor serializes");
    assert_eq!(
        fields.as_object().expect("object").len() + 1,
        THEME_COLOR_MAPPING.len()
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        for (appearance, tokens) in [(Appearance::Dark, &DARK), (Appearance::Light, &LIGHT)] {
            install(appearance, cx);
            let ours: Vec<Hsla> = THEME_COLOR_MAPPING
                .iter()
                .filter_map(|(_, token)| token_by_name(token, tokens))
                .map(|token| token.hsla())
                .collect();
            let applied = serde_json::to_value(cx.theme().colors).expect("serializes");
            for (field, value) in applied.as_object().expect("object") {
                let color: Hsla = serde_json::from_value(value.clone()).expect("Hsla");
                assert!(
                    ours.iter().any(|candidate| close(*candidate, color)),
                    "{appearance:?}: {field} is not one of our tokens"
                );
            }
        }
    });
}

/// The accent trap: gpui-kit `primary` is our accent, gpui-kit `accent` is
/// the menu hover surface.
#[test]
fn our_accent_is_gpui_kit_primary() {
    let lookup = |key: &str| {
        THEME_COLOR_MAPPING
            .iter()
            .find(|(field, _)| *field == key)
            .map(|(_, token)| *token)
    };
    assert_eq!(lookup("primary.background"), Some("accent"));
    assert_eq!(lookup("accent.background"), Some("bg-overlay-hover"));
    assert_eq!(lookup("ring"), Some("focus-ring"));
}

fn close(a: Hsla, b: Hsla) -> bool {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    (a.r - b.r).abs() < 1e-3
        && (a.g - b.g).abs() < 1e-3
        && (a.b - b.b).abs() < 1e-3
        && (a.a - b.a).abs() < 1e-3
}

fn assert_applied(cx: &gpui::App, tokens: &InterfaceTokens) {
    let theme = cx.theme();
    assert!(close(theme.primary, tokens.accent.hsla()), "primary");
    assert!(close(
        theme.primary_foreground,
        tokens.text_on_accent.hsla()
    ));
    assert!(
        close(theme.accent, tokens.bg_overlay_hover.hsla()),
        "accent"
    );
    assert!(close(theme.background, tokens.bg_base.hsla()), "background");
    assert!(close(theme.sidebar, tokens.bg_sidebar.hsla()), "sidebar");
    assert!(close(theme.ring, tokens.focus_ring.hsla()), "ring");
    assert!(close(theme.input, tokens.border_strong.hsla()), "input");
    assert!(close(theme.popover, tokens.bg_overlay.hsla()), "popover");
}

/// The installed theme paints with our tokens and switches between modes.
#[gpui_kit::test]
fn install_applies_dark_and_light(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        install(Appearance::Dark, cx);
        assert!(cx.theme().is_dark());
        assert_eq!(tokens(cx), &DARK);
        assert_applied(cx, &DARK);

        set_appearance(Appearance::Light, None, cx);
        assert!(!cx.theme().is_dark());
        assert_eq!(tokens(cx), &LIGHT);
        assert_applied(cx, &LIGHT);
    });
}

#[test]
fn appearance_names() {
    assert_eq!(Appearance::parse("dark"), Some(Appearance::Dark));
    assert_eq!(Appearance::parse("light"), Some(Appearance::Light));
    assert_eq!(Appearance::parse("auto"), Some(Appearance::Auto));
    assert_eq!(Appearance::parse("blue"), None);
}

/// Auto is the default and follows the window appearance; an explicit
/// Light or Dark setting is not overridden by a system change.
#[gpui_kit::test]
fn auto_follows_the_system_explicit_modes_stay(cx: &mut TestAppContext) {
    use gpui::{AppContext as _, WindowOptions};
    cx.update(|cx| {
        gpui_kit::init(cx);
        install(Appearance::default(), cx);
        assert_eq!(appearance(cx), Appearance::Auto);
    });
    let window = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| gpui::EmptyView)
            })
        })
        .unwrap()
        .0;
    cx.update_window(window, |_, window, cx| {
        let system_dark = ThemeMode::from(window.appearance()).is_dark();
        // Auto: the theme matches the system.
        follow_system(window, cx);
        assert_eq!(cx.theme().is_dark(), system_dark);
        // Explicit: a system change does not override it.
        let explicit = if system_dark {
            Appearance::Light
        } else {
            Appearance::Dark
        };
        set_appearance(explicit, Some(window), cx);
        follow_system(window, cx);
        assert_eq!(cx.theme().is_dark(), !system_dark);
        assert_eq!(appearance(cx), explicit);
    })
    .unwrap();
}
