//! Pure layout and text logic of the Hosts screen (unit-tested without a
//! window): grid columns, card width, keyboard navigation, the card subtitle
//! and the tile icon.

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

/// The card subtitle (UI 2.3): protocol(s), the username, then the group
/// or, without a group, the tags; comma-separated.
pub fn subtitle(host: &HostItem, group_name: Option<&str>) -> String {
    let mut parts: Vec<&str> = vec![strings::PROTOCOL_SSH];
    if let Some(user) = host.username.as_deref() {
        parts.push(user);
    }
    match group_name {
        Some(group) => parts.push(group),
        None => parts.extend(host.tags.iter().map(String::as_str)),
    }
    parts.join(", ")
}

/// The white monochrome tile icon (UI 2.3, design-system 3.1): an OS icon
/// once the OS is detected, else the neutral server icon. Never a letter.
///
/// The icons come from the app's Lucide subset (ISC license), which has no
/// distribution logos; the OS families get generic glyphs.
pub fn os_icon(os_family: Option<&str>) -> IconName {
    match os_family {
        Some("linux") => IconName::SquareTerminal,
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
        // 1920 px window: 1920 - 220 sidebar - 2 x 24 padding.
        assert_eq!(columns_for(1652.), 5);
        // 1280 px window.
        assert_eq!(columns_for(1012.), 3);
        // With the 380 px panel open at 1920 px.
        assert_eq!(columns_for(1272.), 4);
        for width in [300., 500., 760., 1012., 1272., 1652., 2400.] {
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
        host.username = None;
        host.tags.clear();
        assert_eq!(subtitle(&host, None), "SSH");
        assert_eq!(os_icon(None), IconName::Server);
        assert_eq!(os_icon(Some("linux")), IconName::SquareTerminal);
        assert_eq!(os_icon(Some("other")), IconName::Server);
    }
}
