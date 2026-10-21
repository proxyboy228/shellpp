//! Colors of keyword highlighting in the terminal (UI 12.4; stage 6, D67):
//! one per category for a dark and one for a light terminal background
//! (the terminal theme's background decides, not the interface theme).

/// A category's colors as `(r, g, b)`: `[dark background, light background]`.
pub type CategoryColors = [(u8, u8, u8); 2];

pub const ERROR: CategoryColors = [(0xF1, 0x4C, 0x4C), (0xC7, 0x2E, 0x2E)];
pub const WARNING: CategoryColors = [(0xE5, 0xC0, 0x7B), (0x9A, 0x67, 0x00)];
pub const OK: CategoryColors = [(0x23, 0xD1, 0x8B), (0x1A, 0x7F, 0x37)];
pub const INFO: CategoryColors = [(0x3B, 0x8E, 0xEA), (0x09, 0x69, 0xDA)];
pub const DEBUG: CategoryColors = [(0x9D, 0xA5, 0xB4), (0x6E, 0x77, 0x81)];
pub const ADDRESS: CategoryColors = [(0x29, 0xB8, 0xDB), (0x0E, 0x74, 0x90)];

/// The colors a rule or a category can take in Settings → Terminal (D77):
/// the six categories' own, then violet and orange.
pub const PALETTE: [CategoryColors; 8] = [
    ERROR,
    WARNING,
    OK,
    INFO,
    DEBUG,
    ADDRESS,
    [(0xC5, 0x86, 0xF0), (0x82, 0x50, 0xDF)],
    [(0xF0, 0x8A, 0x4B), (0xBC, 0x4C, 0x00)],
];
