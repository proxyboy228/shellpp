//! Pure layout and filter logic of the Hosts screen (unit-tested without a
//! window).

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

/// Case-insensitive match of the search text against the label, address,
/// username and tags of a host. An empty query matches everything.
///
/// TODO(hosts): the full search of UI 2.1 (quick-connect address recognition,
/// group names, ranking) belongs to the Hosts screen work.
pub fn host_matches(host: &HostItem, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let contains = |value: &str| value.to_lowercase().contains(&query);
    host.label.as_deref().is_some_and(contains)
        || contains(&host.address)
        || host.username.as_deref().is_some_and(contains)
        || host.tags.iter().any(|tag| contains(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let card = (width - (cols as f32 - 1.) * 12.) / cols as f32;
            assert!(card <= 360. || cols == 1, "{width}: {cols} x {card}");
            assert!(card >= 240. || cols == 1, "{width}: {cols} x {card}");
        }
        assert_eq!(columns_for(0.), 1);
    }
}
