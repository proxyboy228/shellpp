//! Owner's stage 1, steps 1.1 and 5.1: the tag filter of the Hosts screen
//! (UI 2.2, lead decision 5). The tag button opens a dropdown with every
//! tag, a search field and a checkbox per tag; several tags can be on (a
//! host must carry every tag that is on); "Clear" turns them all off. The
//! tags that are on show as chips next to the button, and the chips that do
//! not fit collapse into a "+N" chip that opens the dropdown. Nothing of it
//! overlaps the other controls of the action bar, also at the minimum window
//! size. Headless GPUI, a temporary profile per test.

mod common;

use common::{Ui, bounds, elements_with_prefix, visible};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{TestAppContext, Window};
use serde_json::json;
use shellpp_desktop::hosts::host_card_id;
use shellpp_desktop::hosts::tag_filter::tag_option_id;
use uuid::Uuid;

mod ids {
    pub const BUTTON: &str = "tag-filter";
    pub const MENU: &str = "tag-filter-menu";
    pub const SEARCH: &str = "tag-filter-search";
    pub const CLEAR: &str = "tag-filter-clear";
    pub const EMPTY: &str = "tag-filter-empty";
    pub const CHIPS: &str = "tag-chips";
    pub const MORE: &str = "tag-chip-more";
    /// A chip of a tag that is on: `tag-chip-<tag>`.
    pub const CHIP_PREFIX: &str = "tag-chip-";
    /// Other controls of the action bar.
    pub const SORT: &str = "sort";
    pub const VIEW_TOGGLE: &str = "view-toggle";
    pub const ACTION_BAR: &str = "action-bar";
}

/// The tags that are on, in the screen's filter.
fn active_tags(ui: &Ui, cx: &mut TestAppContext) -> Vec<String> {
    let shell = ui.shell(cx);
    cx.update(|cx| {
        shell
            .read(cx)
            .hosts_screen()
            .read(cx)
            .filter()
            .tags()
            .to_vec()
    })
}

fn open_menu(ui: &Ui, cx: &mut TestAppContext) {
    ui.frame(cx, |window, cx| {
        if !visible(window, ids::MENU) {
            window.click(ids::BUTTON, cx);
        }
    });
    ui.settle(cx);
    ui.frame(cx, |window, _| {
        assert!(visible(window, ids::MENU), "the dropdown is open")
    });
}

fn click_option(ui: &Ui, cx: &mut TestAppContext, tag: &str) {
    ui.frame(cx, |window, cx| window.click(tag_option_id(tag), cx));
    ui.settle(cx);
}

/// Replaces the text of the dropdown's search field.
fn search_tags(ui: &Ui, cx: &mut TestAppContext, text: &str) {
    ui.frame(cx, |window, cx| {
        window.click(ids::SEARCH, cx);
        window.press("secondary-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
    });
    ui.settle(cx);
}

fn shown_hosts(ui: &Ui, cx: &mut TestAppContext, hosts: &[Uuid]) -> Vec<Uuid> {
    ui.frame(cx, |window, _| {
        hosts
            .iter()
            .copied()
            .filter(|id| visible(window, host_card_id(*id)))
            .collect()
    })
}

/// The chips of tags that are on (not "+N"), left to right.
fn chips(window: &Window) -> Vec<(String, gpui_kit::Bounds<gpui_kit::Pixels>)> {
    let mut chips: Vec<_> = elements_with_prefix(window, ids::CHIP_PREFIX)
        .into_iter()
        .filter(|(id, _)| id != ids::MORE)
        .map(|(id, snapshot)| (id, snapshot.bounds()))
        .collect();
    chips.sort_by(|a, b| {
        a.1.origin
            .x
            .partial_cmp(&b.1.origin.x)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    chips
}

/// Multi-select: checking tags narrows the hosts to those that carry every
/// checked tag; each checked tag shows as a chip; the button shows the
/// active state; a chip's click and "Clear" turn tags off.
#[gpui_kit::test]
fn checked_tags_filter_and_show_as_chips(cx: &mut TestAppContext) {
    let (ui, hosts) = Ui::open(
        cx,
        &[
            json!({"address": "a.example.com", "tags": ["prod", "linux"]}),
            json!({"address": "b.example.com", "tags": ["prod"]}),
            json!({"address": "c.example.com", "tags": ["linux"]}),
            json!({"address": "d.example.com", "tags": ["staging"]}),
            json!({"address": "e.example.com"}),
        ],
    );
    let [a, b, c, d, e] = hosts[..] else {
        unreachable!()
    };
    assert_eq!(shown_hosts(&ui, cx, &hosts), [a, b, c, d, e]);
    ui.frame(cx, |window, _| {
        assert!(!visible(window, ids::CHIPS), "no chips while no tag is on");
        assert_ne!(window.find(ids::BUTTON).selected(), Some(true));
    });

    open_menu(&ui, cx);
    ui.frame(cx, |window, _| {
        for tag in ["prod", "linux", "staging"] {
            assert!(visible(window, tag_option_id(tag)), "option {tag}");
        }
    });
    click_option(&ui, cx, "prod");
    assert_eq!(active_tags(&ui, cx), ["prod"]);
    assert_eq!(shown_hosts(&ui, cx, &hosts), [a, b]);
    // A second tag: hosts with both.
    open_menu(&ui, cx);
    click_option(&ui, cx, "linux");
    assert_eq!(active_tags(&ui, cx), ["prod", "linux"]);
    assert_eq!(shown_hosts(&ui, cx, &hosts), [a]);
    ui.frame(cx, |window, _| {
        if let Some(checked) = window.find(tag_option_id("prod")).checked() {
            assert!(checked, "the checkbox of a tag that is on is checked");
        }
    });

    // Chips next to the button, in the order the tags were turned on.
    ui.frame(cx, |window, cx| {
        window.press("escape", cx);
    });
    ui.settle(cx);
    ui.frame(cx, |window, _| {
        let chips = chips(window);
        let names: Vec<&str> = chips.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(names, ["tag-chip-prod", "tag-chip-linux"]);
        assert!(!visible(window, ids::MORE), "both fit");
        if let Some(selected) = window.find(ids::BUTTON).selected() {
            assert!(selected, "the button shows the active state");
        }
        let button = bounds(window, ids::BUTTON);
        let first = chips[0].1;
        assert!(
            first.origin.x >= button.origin.x + button.size.width,
            "the chips follow the button"
        );
    });

    // A chip's click turns its tag off.
    ui.frame(cx, |window, cx| window.click("tag-chip-prod", cx));
    ui.settle(cx);
    assert_eq!(active_tags(&ui, cx), ["linux"]);
    assert_eq!(shown_hosts(&ui, cx, &hosts), [a, c]);

    // Clear turns everything off.
    open_menu(&ui, cx);
    ui.frame(cx, |window, cx| window.click(ids::CLEAR, cx));
    ui.settle(cx);
    assert!(active_tags(&ui, cx).is_empty());
    assert_eq!(shown_hosts(&ui, cx, &hosts), [a, b, c, d, e]);
    ui.frame(cx, |window, _| assert!(!visible(window, ids::CHIPS)));
    ui.finish(cx);
}

/// The search field of the dropdown narrows the list of tags (case
/// insensitive); a search without matches says so; checking a tag found by
/// the search works like any other.
#[gpui_kit::test]
fn the_dropdown_search_narrows_the_tags(cx: &mut TestAppContext) {
    let (ui, hosts) = Ui::open(
        cx,
        &[
            json!({"address": "a.example.com", "tags": ["prod", "Production-EU"]}),
            json!({"address": "b.example.com", "tags": ["staging", "linux"]}),
            json!({"address": "c.example.com", "tags": ["windows"]}),
        ],
    );
    let [a, ..] = hosts[..] else { unreachable!() };
    open_menu(&ui, cx);
    search_tags(&ui, cx, "PROD");
    ui.frame(cx, |window, _| {
        assert!(visible(window, tag_option_id("prod")));
        assert!(visible(window, tag_option_id("production-eu")));
        for other in ["staging", "linux", "windows"] {
            assert!(
                !visible(window, tag_option_id(other)),
                "{other} is filtered out"
            );
        }
    });
    click_option(&ui, cx, "production-eu");
    assert_eq!(active_tags(&ui, cx), ["production-eu"]);
    assert_eq!(shown_hosts(&ui, cx, &hosts), [a]);

    search_tags(&ui, cx, "no-such-tag");
    ui.frame(cx, |window, _| {
        assert!(visible(window, ids::EMPTY), "no matching tag");
        assert!(!visible(window, tag_option_id("prod")));
    });
    ui.finish(cx);
}

/// Many tags on: the chips that fit are shown, the rest collapse into one
/// "+N" chip (shown chips + N = tags on) that opens the dropdown; the chips
/// stay inside the action bar and never overlap each other, the view toggle
/// or the sort button. Checked at 1280 x 800 and at the minimum window size
/// with and without the details panel.
#[gpui_kit::test]
fn chips_that_do_not_fit_collapse_into_plus_n(cx: &mut TestAppContext) {
    let tags: Vec<String> = (0..12)
        .map(|index| format!("environment-number-{index:02}"))
        .collect();
    let records = vec![
        json!({"address": "a.example.com", "tags": tags}),
        json!({"address": "b.example.com", "tags": ["other"]}),
    ];
    for (width, height, with_panel) in [
        (1280., 800., false),
        (960., 600., false),
        (960., 600., true),
    ] {
        let (ui, hosts) = Ui::open_sized(cx, width, height, &records);
        if with_panel {
            ui.frame(cx, |window, cx| window.click(host_card_id(hosts[0]), cx));
            ui.settle(cx);
        }
        // The list scrolls after about eight tags: each one is found
        // through the dropdown's search.
        for tag in &tags {
            open_menu(&ui, cx);
            search_tags(&ui, cx, tag);
            click_option(&ui, cx, tag);
        }
        ui.frame(cx, |window, cx| window.press("escape", cx));
        ui.settle(cx);
        assert_eq!(active_tags(&ui, cx).len(), tags.len());
        let case = format!("{width}x{height}, panel {with_panel}");
        // The screen's plan of the action bar: how many chips show; "+N"
        // holds the rest.
        let shell = ui.shell(cx);
        let plan = cx
            .update(|cx| shell.read(cx).hosts_screen().read(cx).bar_plan())
            .expect("the action bar was laid out");
        ui.frame(cx, |window, _| {
            let chips = chips(window);
            assert!(visible(window, ids::MORE), "{case}: the +N chip");
            assert!(chips.len() < tags.len(), "{case}: not every chip fits");
            assert_eq!(chips.len(), plan.chips, "{case}: the planned chips show");
            assert!(tags.len() - plan.chips > 0, "{case}: +N counts the rest");
            let more = window.find(ids::MORE);
            // Inside the bar, no overlaps.
            let bar = bounds(window, ids::ACTION_BAR);
            let mut boxes: Vec<(String, gpui_kit::Bounds<gpui_kit::Pixels>)> = chips.clone();
            boxes.push((ids::MORE.to_owned(), more.bounds()));
            boxes.push((ids::BUTTON.to_owned(), bounds(window, ids::BUTTON)));
            boxes.push((ids::SORT.to_owned(), bounds(window, ids::SORT)));
            boxes.push((
                ids::VIEW_TOGGLE.to_owned(),
                bounds(window, ids::VIEW_TOGGLE),
            ));
            // (left, top, right, bottom) in logical pixels.
            let edges = |rect: &gpui_kit::Bounds<gpui_kit::Pixels>| {
                (
                    rect.origin.x.as_f32(),
                    rect.origin.y.as_f32(),
                    (rect.origin.x + rect.size.width).as_f32(),
                    (rect.origin.y + rect.size.height).as_f32(),
                )
            };
            let (bar_left, _, bar_right, _) = edges(&bar);
            for (id, rect) in &boxes {
                let (left, _, right, _) = edges(rect);
                assert!(
                    left >= bar_left - 0.5 && right <= bar_right + 0.5,
                    "{case}: {id} leaves the action bar"
                );
            }
            for (index, (id, rect)) in boxes.iter().enumerate() {
                let (left, top, right, bottom) = edges(rect);
                for (other_id, other) in &boxes[index + 1..] {
                    let (other_left, other_top, other_right, other_bottom) = edges(other);
                    let overlap_x = left < other_right - 0.5 && other_left < right - 0.5;
                    let overlap_y = top < other_bottom - 0.5 && other_top < bottom - 0.5;
                    assert!(
                        !(overlap_x && overlap_y),
                        "{case}: {id} overlaps {other_id}"
                    );
                }
            }
        });
        // "+N" opens the dropdown.
        ui.frame(cx, |window, cx| window.click(ids::MORE, cx));
        ui.settle(cx);
        ui.frame(cx, |window, _| {
            assert!(visible(window, ids::MENU), "{case}: +N opens it")
        });
        ui.finish(cx);
    }
}
