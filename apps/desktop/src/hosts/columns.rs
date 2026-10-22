//! The columns of the list layout (owner's step 0.4): Name | Address |
//! User | Tags, under a header row. Dragging a divider of the header
//! changes the width of the column on its left; Tags takes the rest of the
//! row. The widths are kept in the core settings
//! `view.hosts.column_width.{name,address,user}` (whole pixels).
//!
//! The stored widths are what the user chose. A row narrower than them
//! (a narrow window, the details panel) shrinks Name, Address and User in
//! proportion so Tags keeps [`TAGS_MIN`]; nothing leaves the row. A drag
//! starts from the widths on display, so the divider follows the pointer.
//!
//! Everything here is pure and unit-tested; `hosts::view` draws the header
//! and the rows from [`ListColumns::fit`].

use std::collections::BTreeMap;

use serde_json::{Value, json};
use shellpp_core::CoreError;
use shellpp_core::service::dto::SettingsSetInput;

use crate::state::AppState;

/// Setting key of the Name column width.
pub const NAME_KEY: &str = "view.hosts.column_width.name";
/// Setting key of the Address column width.
pub const ADDRESS_KEY: &str = "view.hosts.column_width.address";
/// Setting key of the User column width.
pub const USER_KEY: &str = "view.hosts.column_width.user";

/// The space between two columns: the row's gap, `--space-3`
/// ([`crate::theme::metrics::SPACE_3`]; a test keeps the two equal).
pub const GAP: f32 = 12.;
/// The narrowest Name column: the 28 px tile, the gap and a few letters.
pub const NAME_MIN: f32 = 96.;
/// The narrowest Address and User columns.
pub const COLUMN_MIN: f32 = 64.;
/// The widest stored column (the setting's upper limit).
pub const COLUMN_MAX: f32 = 2000.;
/// The narrowest Tags column while the others shrink to make room.
pub const TAGS_MIN: f32 = 80.;
/// Width of a header divider's drag area, centered in the gap.
pub const DIVIDER_HIT: f32 = 8.;

/// A column whose right divider can be dragged (Tags takes the rest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Name,
    Address,
    User,
}

impl Column {
    /// Left to right.
    pub const ALL: [Self; 3] = [Self::Name, Self::Address, Self::User];

    fn index(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Address => 1,
            Self::User => 2,
        }
    }

    fn min(self) -> f32 {
        match self {
            Self::Name => NAME_MIN,
            Self::Address | Self::User => COLUMN_MIN,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Name => NAME_KEY,
            Self::Address => ADDRESS_KEY,
            Self::User => USER_KEY,
        }
    }

    /// Stable element id of the column's header divider (tests).
    pub fn divider_id(self) -> &'static str {
        match self {
            Self::Name => "list-divider-name",
            Self::Address => "list-divider-address",
            Self::User => "list-divider-user",
        }
    }
}

/// The value dragged by a header divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeColumn(pub Column);

/// The chosen widths of Name, Address and User, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListColumns {
    widths: [f32; 3],
}

impl Default for ListColumns {
    /// The defaults of the core settings (docs/data-model.md, 16.1).
    fn default() -> Self {
        Self {
            widths: [220., 200., 120.],
        }
    }
}

impl ListColumns {
    /// Widths of Name, Address and User, each clamped to its limits.
    pub fn new(name: f32, address: f32, user: f32) -> Self {
        let mut columns = Self::default();
        for (column, width) in Column::ALL.into_iter().zip([name, address, user]) {
            columns.set(column, width);
        }
        columns
    }

    pub fn width(&self, column: Column) -> f32 {
        self.widths[column.index()]
    }

    fn set(&mut self, column: Column, width: f32) {
        let width = if width.is_finite() { width } else { 0. };
        self.widths[column.index()] = width.round().clamp(column.min(), COLUMN_MAX);
    }

    /// The widths on display in a row whose content is `inner` px wide
    /// (inside its padding): Name, Address, User, Tags. When the chosen
    /// widths leave Tags less than [`TAGS_MIN`], the three shrink in
    /// proportion (not below their minimums).
    pub fn fit(&self, inner: f32) -> [f32; 4] {
        let room = (inner - 3. * GAP).max(0.);
        let fixed: f32 = self.widths.iter().sum();
        let [name, address, user] = if room - fixed >= TAGS_MIN {
            self.widths
        } else {
            let scale = ((room - TAGS_MIN).max(0.) / fixed).min(1.);
            let mut widths = self.widths;
            for column in Column::ALL {
                let width = &mut widths[column.index()];
                *width = (*width * scale).max(column.min());
            }
            widths
        };
        [name, address, user, (room - name - address - user).max(0.)]
    }

    /// The divider right of `column`, in px from the start of the row's
    /// content: the middle of the gap after the column.
    pub fn divider_x(fitted: &[f32; 4], column: Column) -> f32 {
        let index = column.index();
        fitted[..=index].iter().sum::<f32>() + GAP * index as f32 + GAP / 2.
    }

    /// The widths after the divider right of `column` was dragged to `x` px
    /// from the start of the content of a row `inner` px wide. Starts from
    /// the widths on display; the column stays within its limits and leaves
    /// Tags at least [`TAGS_MIN`].
    pub fn dragged(&self, column: Column, x: f32, inner: f32) -> Self {
        let fitted = self.fit(inner);
        let mut next = Self::new(fitted[0], fitted[1], fitted[2]);
        let index = column.index();
        let left: f32 = fitted[..index].iter().sum::<f32>() + GAP * index as f32;
        let others: f32 = Column::ALL
            .into_iter()
            .filter(|other| *other != column)
            .map(|other| fitted[other.index()])
            .sum();
        let room = (inner - 3. * GAP).max(0.);
        let widest = (room - others - TAGS_MIN).max(column.min());
        next.set(column, (x - left - GAP / 2.).min(widest));
        next
    }

    /// The columns of the settings `values` (the defaults where missing).
    pub fn from_settings(values: &BTreeMap<String, Value>) -> Self {
        let defaults = Self::default();
        let width = |column: Column| {
            values
                .get(column.key())
                .and_then(Value::as_f64)
                .map_or(defaults.width(column), |width| width as f32)
        };
        Self::new(
            width(Column::Name),
            width(Column::Address),
            width(Column::User),
        )
    }

    /// The settings values that keep these widths.
    pub fn to_settings(&self) -> BTreeMap<String, Value> {
        Column::ALL
            .into_iter()
            .map(|column| {
                (
                    column.key().to_owned(),
                    json!(self.width(column).round() as i64),
                )
            })
            .collect()
    }
}

impl AppState {
    /// The saved column widths of the Hosts list layout.
    pub fn hosts_list_columns(&self) -> ListColumns {
        self.core()
            .and_then(|core| core.settings_get().ok())
            .map(|values| ListColumns::from_settings(&values))
            .unwrap_or_default()
    }

    /// Remembers the column widths of the Hosts list layout.
    pub fn set_hosts_list_columns(&mut self, columns: ListColumns) -> Result<(), CoreError> {
        self.core()
            .ok_or(CoreError::Unavailable)?
            .settings_set(SettingsSetInput {
                values: columns.to_settings(),
                reset: Vec::new(),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::metrics;

    #[test]
    fn the_gap_is_the_rows_gap() {
        assert_eq!(GAP, metrics::design_px(metrics::SPACE_3));
    }

    #[test]
    fn wide_rows_keep_the_chosen_widths_and_tags_take_the_rest() {
        let columns = ListColumns::default();
        // 1280 px window: 1012 px of content, 24 px of row padding.
        let fitted = columns.fit(988.);
        assert_eq!(fitted, [220., 200., 120., 988. - 36. - 540.]);
        assert_eq!(fitted.iter().sum::<f32>() + 3. * GAP, 988.);
        // The divider right of Name sits in the middle of the gap after it.
        assert_eq!(ListColumns::divider_x(&fitted, Column::Name), 226.);
        assert_eq!(
            ListColumns::divider_x(&fitted, Column::User),
            220. + 200. + 120. + 24. + 6.
        );
    }

    #[test]
    fn narrow_rows_shrink_the_columns_in_proportion() {
        let columns = ListColumns::default();
        // The minimum window with the details panel: 524 - 48 - 24 px.
        let fitted = columns.fit(452.);
        let total: f32 = fitted.iter().sum::<f32>() + 3. * GAP;
        assert!((total - 452.).abs() < 0.01, "{fitted:?}");
        assert!((fitted[3] - TAGS_MIN).abs() < 0.01, "{fitted:?}");
        assert!(
            (fitted[0] / fitted[1] - 220. / 200.).abs() < 0.01,
            "{fitted:?}"
        );
        // Never below the minimums, even when the row is too narrow.
        let tiny = columns.fit(100.);
        assert_eq!(&tiny[..3], &[NAME_MIN, COLUMN_MIN, COLUMN_MIN]);
        assert_eq!(tiny[3], 0.);
    }

    #[test]
    fn dragging_a_divider_moves_it_to_the_pointer_within_limits() {
        let columns = ListColumns::default();
        let inner = 988.;
        // Name to 300 px: the divider follows the pointer.
        let wider = columns.dragged(Column::Name, 306., inner);
        assert_eq!(wider.width(Column::Name), 300.);
        assert_eq!(wider.width(Column::Address), 200.);
        assert_eq!(
            ListColumns::divider_x(&wider.fit(inner), Column::Name),
            306.
        );
        // Address: measured from its own left edge.
        let address = columns.dragged(Column::Address, 220. + 12. + 150. + 6., inner);
        assert_eq!(address.width(Column::Address), 150.);
        // Not below the minimum; Tags keeps its minimum.
        assert_eq!(
            columns.dragged(Column::User, 0., inner).width(Column::User),
            COLUMN_MIN
        );
        let widest = columns.dragged(Column::Name, 5000., inner);
        assert_eq!(widest.fit(inner)[3], TAGS_MIN);
        // In a narrow row the drag starts from the widths on display.
        let narrow = columns.dragged(Column::User, 1000., 452.);
        let fitted = narrow.fit(452.);
        assert!((fitted[3] - TAGS_MIN).abs() < 0.51, "{fitted:?}");
        assert!(narrow.width(Column::Name) < 220.);
    }

    #[test]
    fn settings_round_trip_in_whole_pixels() {
        let columns = ListColumns::new(250.4, 180., 90.6);
        let values = columns.to_settings();
        assert_eq!(values[NAME_KEY], json!(250));
        assert_eq!(values[USER_KEY], json!(91));
        assert_eq!(ListColumns::from_settings(&values), columns);
        assert_eq!(
            ListColumns::from_settings(&BTreeMap::new()),
            ListColumns::default()
        );
        // Out of range values are clamped.
        let clamped = ListColumns::new(10., f32::NAN, 1e9);
        assert_eq!(clamped.width(Column::Name), NAME_MIN);
        assert_eq!(clamped.width(Column::Address), COLUMN_MIN);
        assert_eq!(clamped.width(Column::User), COLUMN_MAX);
    }
}
