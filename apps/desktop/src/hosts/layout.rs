//! Pure layout and text logic of the Hosts screen (unit-tested without a
//! window): grid columns, card width, the action bar at narrow widths,
//! keyboard navigation, the card subtitle and the tile icon.

use gpui_kit::assets::IconName;

use super::strings;
use crate::state::HostItem;
use crate::theme::metrics;

/// Number of card columns for a content width (owner's audit item 4):
/// the fewest columns that keep cards at most 360 px wide, but never so many
/// that a card gets narrower than 240 px; at least one.
pub fn columns_for(width: f32) -> usize {
    let gap = metrics::HOST_CARD_GAP.as_f32();
    let min = metrics::HOST_CARD_MIN_WIDTH.as_f32();
    let max = metrics::HOST_CARD_MAX_WIDTH.as_f32();
    let width = width.max(0.);
    let fewest = ((width + gap) / (max + gap)).ceil().max(1.) as usize;
    let most = ((width + gap) / (min + gap)).floor().max(1.) as usize;
    fewest.min(most).max(1)
}

/// The width of one card column: the content width shared by `columns`
/// columns and their 12 px gaps, never wider than 360 px.
pub fn card_width(width: f32, columns: usize) -> f32 {
    let columns = columns.max(1) as f32;
    let gap = metrics::HOST_CARD_GAP.as_f32();
    let card = (width - (columns - 1.) * gap) / columns;
    card.min(metrics::HOST_CARD_MAX_WIDTH.as_f32()).max(0.)
}

/// How the buttons on the left of the action bar show their labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarLabels {
    /// New host, Terminal and Serial with icon and label.
    Full,
    /// Terminal and Serial as icon buttons (their names in tooltips).
    LocalIcons,
    /// New host too (the narrowest).
    Icons,
}

impl BarLabels {
    /// Widest first.
    pub const ALL: [Self; 3] = [Self::Full, Self::LocalIcons, Self::Icons];

    fn index(self) -> usize {
        match self {
            Self::Full => 0,
            Self::LocalIcons => 1,
            Self::Icons => 2,
        }
    }
}

/// The widths the action bar is planned from (pixels).
#[derive(Clone, Debug, PartialEq)]
pub struct BarMeasure {
    /// The width of the bar.
    pub available: f32,
    /// The left group (split New host, Terminal, Serial) for each of
    /// [`BarLabels::ALL`].
    pub left: [f32; 3],
    /// The right group without chips: the view toggle, the tag button, the
    /// sort button and the gaps between them.
    pub right: f32,
    /// The chips of the tags that are on, in order.
    pub chips: Vec<f32>,
    /// The gap between the groups and before the chips.
    pub gap: f32,
    /// The gap between two chips.
    pub chip_gap: f32,
}

/// How the action bar is laid out (owner's step 1.4, lead decision 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarPlan {
    pub labels: BarLabels,
    /// The right group (view, tags, sort) moved to a second row.
    pub wrapped: bool,
    /// How many tag chips show; the rest are in the "+N" chip.
    pub chips: usize,
}

/// Plans the action bar so nothing overlaps or is clipped (owner's step 0.1):
/// one row with every label while it fits, the tag chips that do not fit
/// collapsing into "+N" but at least one chip staying; then Terminal and
/// Serial as icons, then New host as an icon too; when even that leaves no
/// room for one chip, the right group wraps to a second row (the left group
/// keeps the widest labels that fit its row) and shows the chips that fit
/// that row. "+N" stands alone only when not even one chip fits the second
/// row; the bar then stays one row if "+N" fits it. `more(n)` is the width
/// of the "+n" chip.
pub fn plan_action_bar(measure: &BarMeasure, more: impl Fn(usize) -> f32) -> BarPlan {
    use super::tag_filter::{chips_room, chips_that_fit};
    let BarMeasure {
        available,
        left,
        right,
        chips,
        gap,
        chip_gap,
    } = measure;
    let one_row = |least: f32| {
        BarLabels::ALL.into_iter().find_map(|labels| {
            let room = available - left[labels.index()] - gap - right;
            (room + 0.5 >= least).then(|| BarPlan {
                labels,
                wrapped: false,
                chips: chips_that_fit(chips, room, *gap, *chip_gap, &more),
            })
        })
    };
    // With a tag on, its first chip (and "+N" for the others) should fit.
    let one_chip = chips_room(chips, 1, *gap, *chip_gap, &more);
    if let Some(plan) = one_row(one_chip) {
        return plan;
    }
    let wrapped_chips = chips_that_fit(chips, available - right, *gap, *chip_gap, &more);
    if wrapped_chips == 0
        && !chips.is_empty()
        && let Some(plan) = one_row(chips_room(chips, 0, *gap, *chip_gap, &more))
    {
        return plan;
    }
    let labels = BarLabels::ALL
        .into_iter()
        .find(|labels| left[labels.index()] <= available + 0.5)
        .unwrap_or(BarLabels::Icons);
    BarPlan {
        labels,
        wrapped: true,
        chips: wrapped_chips,
    }
}

/// The search bar's hint that fits `available` px of text (UI 2.1, lead
/// decision 4: nothing is cut off at the minimum window): the full hint,
/// else the short one, else "Search". `width(text)` measures a hint.
pub fn search_placeholder(available: f32, width: impl Fn(&str) -> f32) -> &'static str {
    [
        strings::SEARCH_PLACEHOLDER,
        strings::SEARCH_PLACEHOLDER_SHORT,
    ]
    .into_iter()
    .find(|text| width(text) <= available + 0.5)
    .unwrap_or(strings::SEARCH_PLACEHOLDER_SHORTEST)
}

/// Case-insensitive match of the search text against the label, address,
/// username and tags of a host (UI 2.1). An empty query matches everything.
pub fn host_matches(host: &HostItem, query: &str) -> bool {
    crate::state::hosts_view::text_matches_host(host, query)
}

/// An arrow key on the grid (UI 2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

impl Arrow {
    /// The arrow of a GPUI key name (`left`, `right`, `up`, `down`).
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            _ => None,
        }
    }
}

/// The index the selection moves to: left and right step by one, up and
/// down by a row of `columns` (one in the list layout). Without a current
/// index the first item is selected. Stays inside `0..len`.
pub fn arrow_target(
    current: Option<usize>,
    len: usize,
    columns: usize,
    arrow: Arrow,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(current) = current.filter(|current| *current < len) else {
        return Some(0);
    };
    let columns = columns.max(1);
    let target = match arrow {
        Arrow::Left => current.checked_sub(1),
        Arrow::Right => Some(current + 1).filter(|next| *next < len),
        Arrow::Up => current.checked_sub(columns),
        Arrow::Down => Some(current + columns).filter(|next| *next < len),
    };
    Some(target.unwrap_or(current))
}

/// The card subtitle (UI 2.3): protocol(s) (SSH, then Telnet when the
/// host has a Telnet block), the username, then the group or, without a
/// group, the tags; comma-separated.
pub fn subtitle(host: &HostItem, group_name: Option<&str>) -> String {
    let mut parts: Vec<&str> = vec![strings::PROTOCOL_SSH];
    if host.has_telnet() {
        parts.push(strings::PROTOCOL_TELNET);
    }
    if let Some(user) = host.username.as_deref() {
        parts.push(user);
    }
    match group_name {
        Some(group) => parts.push(group),
        None => parts.extend(host.tags.iter().map(String::as_str)),
    }
    parts.join(", ")
}

/// What the last column of the list layout shows for a host (owner's step
/// 0.4): its tags, or, when it has none, its group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LastColumn {
    /// The tags, comma-separated.
    Tags(String),
    /// The name of the host's group (shown with the folder icon).
    Group(String),
    /// No tags and no group.
    Empty,
}

/// The last list column of `host`; `group_name` is its group's name.
pub fn last_column(host: &HostItem, group_name: Option<&str>) -> LastColumn {
    if !host.tags.is_empty() {
        LastColumn::Tags(host.tags.join(", "))
    } else if let Some(group) = group_name {
        LastColumn::Group(group.to_owned())
    } else {
        LastColumn::Empty
    }
}

/// The white monochrome tile icon (UI 2.3, design-system 3.1): an OS icon
/// once the OS is detected, else the neutral server icon. Never a letter.
///
/// The icons come from the app's Lucide subset (ISC license), which has no
/// distribution logos; the OS families get generic glyphs. Linux gets the
/// app's own penguin ([`crate::app::assets::LINUX_ICON`]): never the
/// terminal glyph, which is the session type icon of a tab (UI 1.1).
pub fn os_icon(os_family: Option<&str>) -> IconName {
    match os_family {
        Some("linux") => crate::app::assets::LINUX_ICON,
        Some("windows" | "macos") => IconName::Monitor,
        _ => IconName::Server,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use shellpp_core::service::dto::TagColor;
    use uuid::Uuid;

    #[test]
    fn columns_follow_the_card_width_limits() {
        // 1920 px window: 1920 - 196 sidebar - 2 x 24 padding.
        assert_eq!(columns_for(1676.), 5);
        // 1280 px window.
        assert_eq!(columns_for(1036.), 3);
        // With the 380 px panel open at 1920 px.
        assert_eq!(columns_for(1296.), 4);
        for width in [300., 500., 760., 1036., 1296., 1676., 2400.] {
            let cols = columns_for(width);
            let card = card_width(width, cols);
            assert!(card <= 360. || cols == 1, "{width}: {cols} x {card}");
            assert!(card >= 240. || cols == 1, "{width}: {cols} x {card}");
        }
        assert_eq!(columns_for(0.), 1);
        // One column never grows past 360 px.
        assert_eq!(card_width(300., 1), 300.);
        assert_eq!(card_width(1000., 1), 360.);
    }

    #[test]
    fn the_search_hint_shortens_instead_of_being_cut() {
        // 7 px per character.
        let width = |text: &str| text.chars().count() as f32 * 7.;
        assert_eq!(search_placeholder(400., width), strings::SEARCH_PLACEHOLDER);
        assert_eq!(
            search_placeholder(200., width),
            strings::SEARCH_PLACEHOLDER_SHORT
        );
        assert_eq!(
            search_placeholder(60., width),
            strings::SEARCH_PLACEHOLDER_SHORTEST
        );
    }

    #[test]
    fn the_action_bar_collapses_labels_then_wraps() {
        let measure = |available: f32, chips: Vec<f32>| BarMeasure {
            available,
            left: [355., 233., 161.],
            right: 164.,
            chips,
            gap: 8.,
            chip_gap: 4.,
        };
        let more = |_: usize| 40.;
        // 960 px window, no details panel (740 - 48): one row, full labels,
        // both chips (8 + 60 + 4 + 70 = 142 <= 692 - 355 - 8 - 164 = 165).
        assert_eq!(
            plan_action_bar(&measure(692., vec![60., 70.]), more),
            BarPlan {
                labels: BarLabels::Full,
                wrapped: false,
                chips: 2
            }
        );
        // Less room: the chips collapse into "+N" first, down to one chip.
        assert_eq!(
            plan_action_bar(&measure(640., vec![60., 70.]), more),
            BarPlan {
                labels: BarLabels::Full,
                wrapped: false,
                chips: 1
            }
        );
        // Then Terminal and Serial lose their labels before the last chip
        // goes: never "+N" alone while a chip fits (owner's step 0.1).
        assert_eq!(
            plan_action_bar(&measure(600., vec![60., 70.]), more),
            BarPlan {
                labels: BarLabels::LocalIcons,
                wrapped: false,
                chips: 2
            }
        );
        // The minimum window with the details panel (524 - 48 px): New host
        // too, then one chip and "+1" (it showed "+2" alone before).
        assert_eq!(
            plan_action_bar(&measure(476., vec![67., 80.]), more),
            BarPlan {
                labels: BarLabels::Icons,
                wrapped: false,
                chips: 1
            }
        );
        // A chip too wide even for a row of its own: "+N" alone, in one row
        // while it fits.
        assert_eq!(
            plan_action_bar(&measure(700., vec![600.]), more),
            BarPlan {
                labels: BarLabels::Full,
                wrapped: false,
                chips: 0
            }
        );
        assert_eq!(
            plan_action_bar(&measure(300., vec![400., 50.]), more),
            BarPlan {
                labels: BarLabels::LocalIcons,
                wrapped: true,
                chips: 0
            }
        );
        // Then Terminal and Serial lose their labels.
        assert_eq!(
            plan_action_bar(&measure(450., vec![]), more).labels,
            BarLabels::LocalIcons
        );
        assert_eq!(
            plan_action_bar(&measure(340., vec![]), more),
            BarPlan {
                labels: BarLabels::Icons,
                wrapped: false,
                chips: 0
            }
        );
        // 960 px window with the 380 px panel (360 - 48): two rows; the left
        // row keeps the widest labels that fit it.
        assert_eq!(
            plan_action_bar(&measure(312., vec![60., 70., 80.]), more),
            BarPlan {
                labels: BarLabels::LocalIcons,
                wrapped: true,
                chips: 1
            }
        );
        // Every planned row fits its width.
        for available in [200., 260., 312., 400., 500., 692., 1000.] {
            for chips in [vec![], vec![50.], vec![50., 60., 70., 80.]] {
                let m = measure(available, chips.clone());
                let plan = plan_action_bar(&m, more);
                let left = m.left[plan.labels.index()];
                let right = m.right
                    + crate::hosts::tag_filter::chips_room(&chips, plan.chips, 8., 4., more);
                if plan.wrapped {
                    assert!(left <= available.max(161.), "{available} {plan:?}");
                    assert!(right <= available.max(164. + 48.), "{available} {plan:?}");
                } else {
                    assert!(left + 8. + right <= available + 0.5, "{available} {plan:?}");
                }
            }
        }
    }

    #[test]
    fn arrows_move_across_the_grid() {
        assert_eq!(arrow_target(None, 0, 4, Arrow::Down), None);
        assert_eq!(arrow_target(None, 9, 4, Arrow::Down), Some(0));
        assert_eq!(arrow_target(Some(5), 9, 4, Arrow::Left), Some(4));
        assert_eq!(arrow_target(Some(5), 9, 4, Arrow::Right), Some(6));
        assert_eq!(arrow_target(Some(5), 9, 4, Arrow::Up), Some(1));
        assert_eq!(arrow_target(Some(5), 9, 4, Arrow::Down), Some(5));
        assert_eq!(arrow_target(Some(4), 9, 4, Arrow::Down), Some(8));
        assert_eq!(arrow_target(Some(0), 9, 4, Arrow::Left), Some(0));
        assert_eq!(arrow_target(Some(8), 9, 4, Arrow::Right), Some(8));
        assert_eq!(arrow_target(Some(2), 9, 1, Arrow::Down), Some(3));
        assert_eq!(Arrow::from_key("up"), Some(Arrow::Up));
        assert_eq!(Arrow::from_key("e"), None);
    }

    #[test]
    fn subtitles_and_icons() {
        let mut host = HostItem {
            record_id: Uuid::nil(),
            vault_id: Uuid::nil(),
            revision: 1,
            label: None,
            address: "db.lan".to_owned(),
            tags: vec!["prod".to_owned(), "db".to_owned()],
            color: TagColor::default(),
            username: Some("root".to_owned()),
            port: None,
            os_family: None,
            group_id: None,
            path: Vec::new(),
            created_at: String::new(),
            modified_at: String::new(),
            data: Value::Null,
            secrets_set: Vec::new(),
        };
        assert_eq!(subtitle(&host, None), "SSH, root, prod, db");
        assert_eq!(subtitle(&host, Some("Production")), "SSH, root, Production");
        // Add Telnet saves a `telnet` block: the card lists both protocols.
        host.data = serde_json::json!({"address": "db.lan", "telnet": {"port": 2323}});
        assert_eq!(
            subtitle(&host, Some("Production")),
            "SSH, Telnet, root, Production"
        );
        host.data = serde_json::json!({"address": "db.lan", "telnet": {}});
        assert_eq!(subtitle(&host, None), "SSH, Telnet, root, prod, db");
        host.data = Value::Null;
        host.username = None;
        host.tags.clear();
        assert_eq!(subtitle(&host, None), "SSH");
        assert_eq!(last_column(&host, None), LastColumn::Empty);
        assert_eq!(
            last_column(&host, Some("Production")),
            LastColumn::Group("Production".to_owned())
        );
        host.tags = vec!["prod".to_owned(), "db".to_owned()];
        assert_eq!(
            last_column(&host, Some("Production")),
            LastColumn::Tags("prod, db".to_owned())
        );
        assert_eq!(os_icon(None), IconName::Server);
        assert_eq!(os_icon(Some("linux")), crate::app::assets::LINUX_ICON);
        // UI 1.1: no OS icon is the terminal glyph of the type icon.
        for os in [
            None,
            Some("linux"),
            Some("windows"),
            Some("macos"),
            Some("bsd"),
        ] {
            assert_ne!(os_icon(os), IconName::SquareTerminal, "{os:?}");
        }
        assert_eq!(os_icon(Some("other")), IconName::Server);
    }
}
