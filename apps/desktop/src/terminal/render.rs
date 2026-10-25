//! Drawing the terminal grid with GPUI primitives (decision 30).
//!
//! Only the visible rows are read and drawn. Each row becomes a
//! [`RowLayout`]: background runs and shaped text segments. A run of
//! narrow cells is one shaped line with one `TextRun` per style (bold,
//! italic, color, underline, strikethrough), placed on the cell grid with
//! GPUI's `force_width`; a wide character (CJK, emoji) is its own segment two
//! cells wide. An emoji sequence that the shell counts over several cells
//! (a ZWJ sequence, a skin tone modifier, a flag) is one segment two cells
//! wide at its first cell, shaped as one glyph; its other cells stay blank,
//! so the columns stay those the shell counts (D90). Row layouts are
//! cached by the row's cells, so rows that did not change (or only moved,
//! as in scrolling) are not shaped again; the cache keeps the rows of the
//! last frame only. A lookup goes by a hash and
//! is confirmed by comparing the cells themselves, so two rows with the
//! same hash never share a layout (what is drawn is what is copied).
//!
//! The terminal lock is held only while the rows are read; shaping happens
//! after it is released.

use std::collections::HashMap;
use std::rc::Rc;

use alacritty_terminal::grid::{Dimensions, Row};
use alacritty_terminal::index::Column;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::{COUNT, Colors};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui::{
    App, Bounds, Hsla, Pixels, Point, ShapedLine, SharedString, StrikethroughStyle, TextAlign,
    TextRun, UnderlineStyle, Window, fill, outline, point, px, size,
};

use super::colors::{Palette, rgb_token, translucent_token};
use super::contrast::{ContrastCache, MinContrast};
use super::fonts::FontSet;
use super::journal::{Buffer as _, Lines};
use super::limits::MAX_ZEROWIDTH;
use super::model::{SharedTerm, TerminalHandle, TerminalSize};
use super::search::{MatchSpan, SearchState};
use super::selection::Selection;

/// Thickness of the underline and bar cursors and of text decorations.
const CURSOR_THICKNESS: Pixels = px(2.);
const DECORATION_THICKNESS: Pixels = px(1.);

/// Cell metrics of the current font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub font_size: Pixels,
}

/// What a frame is drawn with, besides the terminal content.
#[derive(Clone, Debug)]
pub struct FrameStyle {
    pub palette: Palette,
    pub fonts: Rc<FontSet>,
    pub font_size: Pixels,
    pub line_height: f32,
    pub bold_is_bright: bool,
    pub focused: bool,
    /// IME composition text, drawn at the cursor.
    pub marked_text: Option<String>,
    /// An autocomplete suggestion (D142), drawn muted from the cursor to
    /// the end of its row.
    pub ghost_text: Option<String>,
    /// `terminal.min_contrast`: which text is moved to 4.5:1.
    pub min_contrast: MinContrast,
    /// The cursor is not drawn (a dropped session, UI 1.1).
    pub cursor_hidden: bool,
    /// The search whose matches are highlighted (UI 4.2 Tools).
    pub search: Option<SearchState>,
    /// The selection (UI 4.3).
    pub selection: Option<Selection>,
    /// Keyword highlighting (`terminal.highlighting_enabled`, UI 12.4).
    pub highlighting: bool,
}

/// Geometry and caches shared by the view (events) and the element (frames).
#[derive(Default)]
pub struct LayoutState {
    /// Metrics of the last frame.
    pub metrics: Option<Metrics>,
    /// Window position of cell (0, 0) in the last frame.
    pub origin: Point<Pixels>,
    pub cols: usize,
    pub rows: usize,
    /// Viewport cell of the cursor in the last frame (IME placement).
    pub cursor: (usize, usize),
    cache: HashMap<u64, CachedRow>,
    /// What the cached rows were shaped with (font, palette, colors).
    cache_key: Vec<u64>,
    /// Rows shaped since the view was created (tests, benchmark).
    pub shaped_rows: u64,
    /// The grid changed its cell size in a prepaint (a reflow): an open
    /// search runs again at the next wake, since its positions moved.
    pub resized: bool,
    /// Adjusted text colors per (text, background) pair (minimum contrast).
    contrast: ContrastCache,
    /// The cache slot of each visible row in the last frame.
    #[cfg(test)]
    row_hashes: Vec<u64>,
}

impl LayoutState {
    /// The viewport cell under a window position, clamped to the grid,
    /// and whether the position is in the right half of that cell.
    pub fn cell_at(&self, position: Point<Pixels>) -> Option<(usize, usize, bool)> {
        let metrics = self.metrics?;
        if self.cols == 0 || self.rows == 0 {
            return None;
        }
        let x = (position.x - self.origin.x).as_f32().max(0.);
        let y = (position.y - self.origin.y).as_f32().max(0.);
        let width = metrics.cell_width.as_f32();
        let column = ((x / width) as usize).min(self.cols - 1);
        let line = ((y / metrics.line_height.as_f32()) as usize).min(self.rows - 1);
        let right_half = x - column as f32 * width > width / 2.;
        Some((line, column, right_half))
    }
}

/// One background run of a row.
#[derive(Clone, Debug)]
struct BackgroundRun {
    column: usize,
    cells: usize,
    color: Hsla,
}

/// One shaped piece of a row.
struct Segment {
    column: usize,
    line: ShapedLine,
}

/// A row ready to paint.
pub struct RowLayout {
    backgrounds: Vec<BackgroundRun>,
    segments: Vec<Segment>,
}

impl std::fmt::Debug for RowLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowLayout")
            .field("backgrounds", &self.backgrounds.len())
            .field("segments", &self.segments.len())
            .finish()
    }
}

impl std::fmt::Debug for LayoutState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayoutState")
            .field("metrics", &self.metrics)
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("cached_rows", &self.cache.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("rows", &self.rows.len())
            .field("metrics", &self.metrics)
            .finish_non_exhaustive()
    }
}

/// A cell as the shaper needs it (colors resolved).
#[derive(Clone, Debug)]
struct CellOut {
    column: usize,
    text: SmallText,
    fg: Rgb,
    bg: Option<Rgb>,
    flags: Flags,
}

/// The character of a cell and its combining marks.
#[derive(Clone, Debug)]
struct SmallText(String);

/// A shaped row and the exact description of the cells it was shaped from.
#[derive(Clone)]
struct CachedRow {
    words: Rc<[u64]>,
    layout: Rc<RowLayout>,
}

/// A row read under the lock: cached already, or its description and cells.
enum RowInput {
    Cached(u64),
    Miss(u64, Rc<[u64]>, Vec<CellOut>),
}

/// The cursor of a frame.
#[derive(Clone, Debug)]
struct CursorOut {
    line: usize,
    column: usize,
    shape: CursorShape,
    wide: bool,
    text: String,
    bold: bool,
    italic: bool,
}

/// Everything paint needs.
pub struct Frame {
    rows: Vec<Rc<RowLayout>>,
    metrics: Metrics,
    background: Hsla,
    selection: Vec<(usize, usize, usize)>,
    selection_color: Hsla,
    /// Search matches on screen, and their two backgrounds.
    matches: Vec<MatchSpan>,
    match_color: Hsla,
    match_active_color: Hsla,
    cursor: Option<(CursorOut, Option<ShapedLine>)>,
    cursor_color: Hsla,
    focused: bool,
    marked: Option<(ShapedLine, usize)>,
    /// The autocomplete suggestion, painted at the cursor.
    ghost: Option<ShapedLine>,
}

/// An exact description of a row's cells (or of the frame style) as
/// words, and a fast hash of it. Equal words mean an equal layout; the hash
/// only picks the cache slot, and the words are compared on a hit, so a
/// precomputed hash collision cannot show one row's text for another.
#[derive(Default)]
struct Words(Vec<u64>);

impl Words {
    const SEED: u64 = 0x9E37_79B9_7F4A_7C15;
    const MULTIPLIER: u64 = 0x5851_F42D_4C95_7F2D;
    /// Tags that keep the words of different kinds apart (a cell's first
    /// word is below 2^48: a char and 16 bits of flags).
    const SELECTION: u64 = 1 << 48;
    const ZEROWIDTH: u64 = 3 << 48;

    fn write(&mut self, value: u64) {
        self.0.push(value);
    }

    fn color(&mut self, color: Color) {
        match color {
            Color::Named(name) => self.write(name as u64),
            Color::Indexed(index) => self.write((1 << 16) | u64::from(index)),
            Color::Spec(rgb) => self.write((2 << 32) | rgb_bits(rgb)),
        }
    }

    /// Four words and the zero-width characters (at most
    /// [`MAX_ZEROWIDTH`], as many as are drawn).
    fn cell(&mut self, cell: &Cell) {
        self.write(u64::from(cell.c) | (u64::from(cell.flags.bits()) << 32));
        self.color(cell.fg);
        self.color(cell.bg);
        let extra = cell.zerowidth().unwrap_or_default();
        let extra = &extra[..extra.len().min(MAX_ZEROWIDTH)];
        self.write(Self::ZEROWIDTH | extra.len() as u64);
        for ch in extra {
            self.write(u64::from(*ch));
        }
    }

    fn hash(&self) -> u64 {
        self.0.iter().fold(Self::SEED, |hash, value| {
            (hash.rotate_left(5) ^ value).wrapping_mul(Self::MULTIPLIER)
        })
    }
}

fn rgb_bits(rgb: Rgb) -> u64 {
    (u64::from(rgb.r) << 16) | (u64::from(rgb.g) << 8) | u64::from(rgb.b)
}

/// What every row of a frame is shaped with: the font, the palette and the
/// remote color overrides.
fn style_words(
    style: &FrameStyle,
    metrics: &Metrics,
    colors: &Colors,
    highlight: bool,
) -> Vec<u64> {
    let mut words = Words::default();
    words.write(u64::from(highlight));
    if highlight {
        // Rows are laid out again under new rules (D77).
        words.write(super::highlight::current().generation());
    }
    let family = style.fonts.family();
    words.write(family.len() as u64);
    for byte in family.bytes() {
        words.write(u64::from(byte));
    }
    words.write(u64::from(metrics.font_size.as_f32().to_bits()));
    words.write(u64::from(metrics.cell_width.as_f32().to_bits()));
    words.write(u64::from(style.bold_is_bright));
    words.write(style.min_contrast as u64);
    let palette = &style.palette;
    for index in 0..=255u8 {
        words.write(rgb_bits(palette.indexed(index)));
    }
    for rgb in [palette.foreground(), palette.background()] {
        words.write(rgb_bits(rgb));
    }
    for index in 0..COUNT {
        match colors[index] {
            Some(rgb) => words.write(rgb_bits(rgb) | (1 << 24)),
            None => words.write(0),
        }
    }
    words.0
}

/// Snaps a logical coordinate to the device pixel grid.
fn snap(value: Pixels, scale: f32) -> Pixels {
    px((value.as_f32() * scale).round() / scale)
}

/// The cell metrics of `style`'s font in this window.
pub fn metrics(style: &FrameStyle, window: &Window) -> Metrics {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(style.fonts.get(false, false));
    let cell_width = text_system
        .advance(font_id, style.font_size, 'm')
        .map(|advance| advance.width)
        .ok()
        .filter(|width| *width > px(0.))
        .unwrap_or(style.font_size * 0.6);
    let scale = window.scale_factor();
    let line_height = snap(style.font_size * style.line_height, scale).max(px(1.));
    Metrics {
        cell_width,
        line_height,
        font_size: style.font_size,
    }
}

/// The grid that fits a text area of `area` with `metrics` (whole cells,
/// at least 2 x 1, at most 1000 x 1000).
pub fn grid_size(area: gpui::Size<Pixels>, metrics: &Metrics) -> TerminalSize {
    let cols = ((area.width / metrics.cell_width).floor() as usize).clamp(2, 1000);
    let rows = ((area.height / metrics.line_height).floor() as usize).clamp(1, 1000);
    TerminalSize {
        cols: cols as u16,
        rows: rows as u16,
        pixel_width: (metrics.cell_width * cols as f32).as_f32().round() as u16,
        pixel_height: (metrics.line_height * rows as f32).as_f32().round() as u16,
    }
}

/// Reads the visible rows of `term`, resizing it to `bounds` first, and
/// shapes the rows that are not cached. Runs in the element's prepaint.
pub fn prepaint(
    bounds: Bounds<Pixels>,
    handle: &TerminalHandle,
    layout: &mut LayoutState,
    style: &FrameStyle,
    window: &mut Window,
) -> Frame {
    let metrics = metrics(style, window);
    let grid = grid_size(bounds.size, &metrics);
    let cols = usize::from(grid.cols);
    let rows = usize::from(grid.rows);
    // A new cell size reflows wrapped lines: the view's search positions
    // are stale (and not drawn) until it searches again (on the wake this
    // sends).
    if handle.resize(grid) && (layout.cols, layout.rows) != (cols, rows) {
        layout.resized = true;
        handle.outbox().wake();
    }
    layout.metrics = Some(metrics);
    layout.origin = bounds.origin;
    layout.cols = cols;
    layout.rows = rows;

    let term: &SharedTerm = handle.term();
    let mut inputs = Vec::with_capacity(rows);
    let mut words = Words::default();
    let selection;
    let matches;
    let cursor;
    {
        let term = term.lock();
        let content = term.renderable_content();
        // Keyword highlighting: never in the alternate screen (UI 12.4).
        let highlight = (style.highlighting && !content.mode.contains(TermMode::ALT_SCREEN))
            .then(|| super::contrast::luminance(style.palette.background()) < 0.5);
        let key = style_words(style, &metrics, content.colors, highlight.is_some());
        if key != layout.cache_key {
            layout.cache.clear();
            layout.cache_key = key;
        }
        let grid = term.grid();
        let journal = handle.outbox().journal();
        // The rows come from the grid and, scrolled far back, the journal
        // (D85); the view's offset covers both.
        let lines = Lines::new(&term, &journal);
        let offset = lines.view_offset() as i32;
        let top = lines.view_top();
        let visible = grid.screen_lines().min(rows);
        let columns = grid.columns();
        let origin = handle.search_origin(&term);
        selection = style
            .selection
            .as_ref()
            .map(|selection| selection.spans(&lines, origin.evicted, visible))
            .unwrap_or_default();
        matches = style
            .search
            .as_ref()
            .map(|search| search.spans(&lines, origin, visible, columns))
            .unwrap_or_default();
        // A theme's `selectionForeground` recolors the selected text, so the
        // selected columns become part of the row (design-system 7.2).
        let selection_fg = style.palette.selection_foreground();
        for line in 0..visible {
            let row = lines.row(top + line);
            let row: &Row<Cell> = &row;
            let selected = selection_fg.and_then(|_| {
                selection
                    .iter()
                    .find(|(row, _, _)| *row == line)
                    .map(|&(_, first, last)| (first, last))
            });
            words.0.clear();
            words.write(columns as u64);
            if let Some((first, last)) = selected {
                words.write(Words::SELECTION | ((first as u64) << 16) | last as u64);
            }
            for column in 0..columns {
                words.cell(&row[Column(column)]);
            }
            let hash = words.hash();
            if layout
                .cache
                .get(&hash)
                .is_some_and(|cached| *cached.words == words.0[..])
            {
                inputs.push(RowInput::Cached(hash));
                continue;
            }
            let cells = row_cells(
                row,
                columns,
                style,
                content.colors,
                selection_fg.zip(selected),
                highlight,
                &mut layout.contrast,
            );
            inputs.push(RowInput::Miss(hash, words.0.as_slice().into(), cells));
        }
        let point = content.cursor.point;
        let cursor_line = point.line.0 + offset;
        cursor = (content.cursor.shape != CursorShape::Hidden
            && !style.cursor_hidden
            && (0..visible as i32).contains(&cursor_line))
        .then(|| {
            let cell = &grid[point];
            CursorOut {
                line: cursor_line as usize,
                column: point.column.0,
                shape: content.cursor.shape,
                wide: cell.flags.contains(Flags::WIDE_CHAR),
                text: if cell.flags.contains(Flags::HIDDEN) {
                    String::new()
                } else {
                    let mut text = String::from(cell.c);
                    if let Some(extra) = cell.zerowidth() {
                        text.extend(extra.iter().take(MAX_ZEROWIDTH));
                    }
                    text
                },
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
            }
        });
    }

    // Shape what is new, outside the lock.
    let mut current = HashMap::with_capacity(inputs.len());
    let mut rows_out = Vec::with_capacity(inputs.len());
    #[cfg(test)]
    let mut hashes = Vec::with_capacity(inputs.len());
    for input in inputs {
        let (hash, row) = match input {
            RowInput::Cached(hash) => (hash, layout.cache[&hash].clone()),
            RowInput::Miss(hash, words, cells) => {
                layout.shaped_rows += 1;
                let layout = Rc::new(layout_row(&cells, style, &metrics, window));
                (hash, CachedRow { words, layout })
            }
        };
        rows_out.push(row.layout.clone());
        #[cfg(test)]
        hashes.push(hash);
        current.insert(hash, row);
    }
    layout.cache = current;
    #[cfg(test)]
    {
        layout.row_hashes = hashes;
    }

    if let Some(cursor) = &cursor {
        layout.cursor = (cursor.line, cursor.column);
    }
    let cursor = cursor.map(|cursor| {
        let glyph =
            (style.focused && cursor.shape == CursorShape::Block && !cursor.text.trim().is_empty())
                .then(|| {
                    let width = if cursor.wide {
                        metrics.cell_width * 2.
                    } else {
                        metrics.cell_width
                    };
                    shape(
                        window,
                        &cursor.text,
                        &[TextRun {
                            len: cursor.text.len(),
                            font: style.fonts.get(cursor.bold, cursor.italic).clone(),
                            color: rgb_token(style.palette.cursor_text()).hsla(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        metrics.font_size,
                        width,
                    )
                });
        (cursor, glyph)
    });
    let marked = style
        .marked_text
        .as_ref()
        .filter(|text| !text.is_empty())
        .map(|text| {
            let cells = text
                .chars()
                .map(|ch| if ch.is_ascii() { 1 } else { 2 })
                .sum::<usize>();
            let line = shape(
                window,
                text,
                &[TextRun {
                    len: text.len(),
                    font: style.fonts.get(false, false).clone(),
                    color: rgb_token(style.palette.foreground()).hsla(),
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        thickness: DECORATION_THICKNESS,
                        color: Some(rgb_token(style.palette.foreground()).hsla()),
                        wavy: false,
                    }),
                    strikethrough: None,
                }],
                metrics.font_size,
                metrics.cell_width,
            );
            (line, cells)
        });
    // The suggestion: from the cursor to the end of its row, muted, never
    // over IME composition text.
    let ghost = style
        .ghost_text
        .as_ref()
        .filter(|_| marked.is_none() && !style.cursor_hidden)
        .zip(cursor.as_ref())
        .and_then(|(text, (at, _))| {
            let room = cols.saturating_sub(at.column);
            let mut cells = 0;
            let text: String = text
                .chars()
                .take_while(|ch| {
                    cells += unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(1);
                    cells <= room
                })
                .collect();
            if text.is_empty() {
                return None;
            }
            let color = rgb_token(style.palette.foreground()).hsla().opacity(0.45);
            Some(shape(
                window,
                &text,
                &[TextRun {
                    len: text.len(),
                    font: style.fonts.get(false, false).clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                metrics.font_size,
                metrics.cell_width,
            ))
        });

    Frame {
        rows: rows_out,
        metrics,
        background: rgb_token(style.palette.background()).hsla(),
        selection,
        selection_color: translucent_token(style.palette.selection()).hsla(),
        matches,
        match_color: rgb_token(style.palette.search_match()).hsla(),
        match_active_color: rgb_token(style.palette.search_match_active()).hsla(),
        cursor,
        cursor_color: rgb_token(style.palette.cursor()).hsla(),
        focused: style.focused,
        marked,
        ghost,
    }
}

/// The cells of a row as the shaper needs them: colors resolved (theme,
/// remote overrides, attributes, minimum contrast), wide character spacers
/// left out, hidden text as blanks. `selected` is the theme's
/// `selectionForeground` and the selected columns of the row, if both
/// exist; selected text in that color is not adjusted.
fn row_cells(
    row: &Row<Cell>,
    columns: usize,
    style: &FrameStyle,
    colors: &Colors,
    selected: Option<(Rgb, (usize, usize))>,
    highlight: Option<bool>,
    contrast: &mut ContrastCache,
) -> Vec<CellOut> {
    let mut cells = Vec::with_capacity(columns);
    // Cells keyword highlighting may recolor: the default text color, not
    // hidden, not selected.
    let mut plain = Vec::with_capacity(columns);
    for column in 0..columns {
        let cell = &row[Column(column)];
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let (mut fg, bg) =
            style
                .palette
                .cell_colors(cell.fg, cell.bg, cell.flags, style.bold_is_bright, colors);
        // Minimum contrast (UI 12.4): never for hidden text, which must
        // stay invisible.
        if !cell.flags.contains(Flags::HIDDEN)
            && style
                .min_contrast
                .applies(Palette::text_is_truecolor(cell.fg, cell.bg, cell.flags))
        {
            fg = contrast.adjust(fg, bg.unwrap_or(style.palette.background()));
        }
        // A wide character counts as selected when either of its cells is.
        let last_cell = if cell.flags.contains(Flags::WIDE_CHAR) {
            column + 1
        } else {
            column
        };
        // Plain text only: the default colors, not reversed, dimmed or hidden
        // (a category color on a program's background could be unreadable).
        let mut is_plain = matches!(cell.fg, Color::Named(NamedColor::Foreground))
            && matches!(cell.bg, Color::Named(NamedColor::Background))
            && !cell
                .flags
                .intersects(Flags::HIDDEN | Flags::INVERSE | Flags::DIM);
        if let Some((color, (first, last))) = selected
            && last_cell >= first
            && column <= last
            && !cell.flags.contains(Flags::HIDDEN)
        {
            fg = color;
            is_plain = false;
        }
        plain.push(is_plain);
        let mut text = String::new();
        text.push(
            if cell
                .flags
                .intersects(Flags::HIDDEN | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                ' '
            } else {
                cell.c
            },
        );
        if let Some(extra) = cell.zerowidth() {
            text.extend(extra.iter().take(MAX_ZEROWIDTH));
        }
        cells.push(CellOut {
            column,
            text: SmallText(text),
            fg,
            bg,
            flags: cell.flags,
        });
    }
    if let Some(dark) = highlight {
        // One character per cell (spacers are left out above).
        let line: String = cells
            .iter()
            .map(|cell| cell.text.0.chars().next().unwrap_or(' '))
            .collect();
        let rules = super::highlight::current();
        for (range, color) in super::highlight::colored_spans(&line, &rules, dark) {
            for index in range {
                if plain.get(index).copied().unwrap_or(false) {
                    cells[index].fg = color;
                }
            }
        }
    }
    cells
}

fn shape(
    window: &Window,
    text: &str,
    runs: &[TextRun],
    font_size: Pixels,
    cell_width: Pixels,
) -> ShapedLine {
    window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        font_size,
        runs,
        Some(cell_width),
    )
}

/// The style of a cell's text run.
#[derive(Clone, Copy, PartialEq)]
struct RunStyle {
    fg: Rgb,
    bold: bool,
    italic: bool,
    underline: bool,
    wavy: bool,
    strike: bool,
}

impl RunStyle {
    fn of(cell: &CellOut) -> Self {
        Self {
            fg: cell.fg,
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            wavy: cell.flags.contains(Flags::UNDERCURL),
            strike: cell.flags.contains(Flags::STRIKEOUT),
        }
    }

    fn decorated(self) -> bool {
        self.underline || self.strike
    }

    fn run(self, len: usize, fonts: &FontSet) -> TextRun {
        let color = rgb_token(self.fg).hsla();
        TextRun {
            len,
            font: fonts.get(self.bold, self.italic).clone(),
            color,
            background_color: None,
            underline: self.underline.then_some(UnderlineStyle {
                thickness: DECORATION_THICKNESS,
                color: Some(color),
                wavy: self.wavy,
            }),
            strikethrough: self.strike.then_some(StrikethroughStyle {
                thickness: DECORATION_THICKNESS,
                color: Some(color),
            }),
        }
    }
}

/// A run of narrow cells being collected.
#[derive(Default)]
struct NarrowSegment {
    column: usize,
    text: String,
    runs: Vec<(RunStyle, usize)>,
}

impl NarrowSegment {
    fn push(&mut self, cell: &CellOut) {
        if self.text.is_empty() {
            self.column = cell.column;
        }
        let style = RunStyle::of(cell);
        let blank = cell.text.0 == " ";
        let len = cell.text.0.len();
        self.text.push_str(&cell.text.0);
        match self.runs.last_mut() {
            // A blank without decoration looks the same in any run.
            Some((last, run_len))
                if *last == style || (blank && !style.decorated() && !last.decorated()) =>
            {
                *run_len += len;
            }
            _ => self.runs.push((style, len)),
        }
    }

    /// Shapes the segment (trailing undecorated blanks dropped).
    fn finish(
        &mut self,
        style: &FrameStyle,
        metrics: &Metrics,
        window: &Window,
        segments: &mut Vec<Segment>,
    ) {
        while self.text.ends_with(' ') {
            let Some((run_style, len)) = self.runs.last_mut() else {
                break;
            };
            if run_style.decorated() {
                break;
            }
            self.text.pop();
            *len -= 1;
            if *len == 0 {
                self.runs.pop();
            }
        }
        if !self.text.trim_start().is_empty() || self.runs.iter().any(|(s, _)| s.decorated()) {
            let runs: Vec<TextRun> = self
                .runs
                .iter()
                .map(|(run_style, len)| run_style.run(*len, &style.fonts))
                .collect();
            segments.push(Segment {
                column: self.column,
                line: shape(
                    window,
                    &self.text,
                    &runs,
                    metrics.font_size,
                    metrics.cell_width,
                ),
            });
        }
        self.text.clear();
        self.runs.clear();
    }
}

/// Whether a cell whose text is `next` continues the emoji sequence `prev`
/// (the text of the cells before it): after a zero width joiner, a skin
/// tone modifier after an emoji, or the second regional indicator of a
/// flag.
fn continues_emoji(prev: &str, next: &str) -> bool {
    let Some(first) = next.chars().next() else {
        return false;
    };
    let regional = |ch: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch);
    prev.ends_with('\u{200D}')
        || (('\u{1F3FB}'..='\u{1F3FF}').contains(&first)
            && prev
                .chars()
                .next_back()
                .is_some_and(|last| !last.is_ascii()))
        || (regional(first) && prev.chars().filter(|ch| regional(*ch)).count() % 2 == 1)
}

/// An emoji sequence's glyphs (D90, D206). A font may draw one sequence as
/// several glyphs it places together (Segoe UI Emoji on Windows 10 draws a
/// ZWJ family so): spread one per two cells, as a wide character's, they
/// fall apart. So the font's own placement stays when it fits the
/// sequence's `columns`; otherwise one glyph per two cells.
fn shape_sequence(
    window: &Window,
    text: &str,
    runs: &[TextRun],
    metrics: &Metrics,
    columns: usize,
) -> ShapedLine {
    let natural = window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        metrics.font_size,
        runs,
        None,
    );
    if sequence_keeps_its_placement(natural.width, metrics.cell_width, columns) {
        natural
    } else {
        shape(
            window,
            text,
            runs,
            metrics.font_size,
            metrics.cell_width * 2.,
        )
    }
}

/// Whether an emoji sequence `width` wide as its font places it fits its
/// `columns` cells.
fn sequence_keeps_its_placement(width: Pixels, cell_width: Pixels, columns: usize) -> bool {
    width <= cell_width * columns as f32
}

/// The columns a cell takes: two for a wide character.
fn cell_columns(cell: &CellOut) -> usize {
    if cell.flags.contains(Flags::WIDE_CHAR) {
        2
    } else {
        1
    }
}

/// Adds a cell's background to the row's runs.
fn add_background(backgrounds: &mut Vec<BackgroundRun>, cell: &CellOut) {
    let Some(bg) = cell.bg else {
        return;
    };
    let width = cell_columns(cell);
    let color = rgb_token(bg).hsla();
    match backgrounds.last_mut() {
        Some(last) if last.color == color && last.column + last.cells == cell.column => {
            last.cells += width;
        }
        _ => backgrounds.push(BackgroundRun {
            column: cell.column,
            cells: width,
            color,
        }),
    }
}

fn layout_row(
    cells: &[CellOut],
    style: &FrameStyle,
    metrics: &Metrics,
    window: &Window,
) -> RowLayout {
    let mut backgrounds: Vec<BackgroundRun> = Vec::new();
    let mut segments = Vec::new();
    let mut narrow = NarrowSegment::default();
    let mut index = 0;
    while index < cells.len() {
        let cell = &cells[index];
        add_background(&mut backgrounds, cell);
        // An emoji sequence over several cells: one glyph at its first cell.
        let mut end = index + 1;
        if !cell.flags.contains(Flags::HIDDEN) {
            let mut text = cell.text.0.clone();
            while end < cells.len() && continues_emoji(&text, &cells[end].text.0) {
                text.push_str(&cells[end].text.0);
                add_background(&mut backgrounds, &cells[end]);
                end += 1;
            }
            if end > index + 1 {
                narrow.finish(style, metrics, window, &mut segments);
                let runs = [RunStyle::of(cell).run(text.len(), &style.fonts)];
                let last = &cells[end - 1];
                let columns = last.column + cell_columns(last) - cell.column;
                segments.push(Segment {
                    column: cell.column,
                    line: shape_sequence(window, &text, &runs, metrics, columns),
                });
                index = end;
                continue;
            }
        }
        index += 1;
        if cell.flags.contains(Flags::WIDE_CHAR) {
            narrow.finish(style, metrics, window, &mut segments);
            let run_style = RunStyle::of(cell);
            let runs = [run_style.run(cell.text.0.len(), &style.fonts)];
            segments.push(Segment {
                column: cell.column,
                line: shape(
                    window,
                    &cell.text.0,
                    &runs,
                    metrics.font_size,
                    metrics.cell_width * 2.,
                ),
            });
        } else {
            narrow.push(cell);
        }
    }
    narrow.finish(style, metrics, window, &mut segments);
    RowLayout {
        backgrounds,
        segments,
    }
}

/// Paints a prepared frame. The element's bounds are already filled with
/// the theme background by its container.
pub fn paint(bounds: Bounds<Pixels>, frame: &Frame, window: &mut Window, cx: &mut App) {
    let scale = window.scale_factor();
    let Metrics {
        cell_width,
        line_height,
        ..
    } = frame.metrics;
    let origin = bounds.origin;
    let cell_x = |column: usize| origin.x + cell_width * column as f32;
    let row_y = |row: usize| origin.y + line_height * row as f32;
    let cell_rect = |row: usize, column: usize, cells: usize| {
        let left = snap(cell_x(column), scale);
        let right = snap(cell_x(column + cells), scale);
        let top = snap(row_y(row), scale);
        let bottom = snap(row_y(row + 1), scale);
        Bounds::new(point(left, top), size(right - left, bottom - top))
    };

    window.paint_quad(fill(bounds, frame.background));
    for (row, layout) in frame.rows.iter().enumerate() {
        for run in &layout.backgrounds {
            window.paint_quad(fill(cell_rect(row, run.column, run.cells), run.color));
        }
    }
    for span in &frame.matches {
        let color = if span.current {
            frame.match_active_color
        } else {
            frame.match_color
        };
        window.paint_quad(fill(
            cell_rect(span.row, span.first, span.last - span.first + 1),
            color,
        ));
    }
    for (row, first, last) in &frame.selection {
        window.paint_quad(fill(
            cell_rect(*row, *first, last - first + 1),
            frame.selection_color,
        ));
    }
    if let Some((cursor, _)) = &frame.cursor {
        let cells = if cursor.wide { 2 } else { 1 };
        let rect = cell_rect(cursor.line, cursor.column, cells);
        match cursor.shape {
            // Filled while the terminal has the focus, hollow otherwise.
            CursorShape::Block if frame.focused => {
                window.paint_quad(fill(rect, frame.cursor_color));
            }
            CursorShape::Underline => {
                let bar = Bounds::new(
                    point(rect.origin.x, rect.bottom() - CURSOR_THICKNESS),
                    size(rect.size.width, CURSOR_THICKNESS),
                );
                window.paint_quad(fill(bar, frame.cursor_color));
            }
            CursorShape::Beam => {
                window.paint_quad(fill(
                    Bounds::new(rect.origin, size(CURSOR_THICKNESS, rect.size.height)),
                    frame.cursor_color,
                ));
            }
            _ => {
                window.paint_quad(outline(rect, frame.cursor_color, gpui::BorderStyle::Solid));
            }
        }
    }
    for (row, layout) in frame.rows.iter().enumerate() {
        for segment in &layout.segments {
            let at = point(cell_x(segment.column), row_y(row));
            let _ = segment
                .line
                .paint(at, line_height, TextAlign::Left, None, window, cx);
        }
    }
    if let Some((cursor, Some(glyph))) = &frame.cursor {
        // The character under the block cursor, in the cursor text color.
        let at = point(cell_x(cursor.column), row_y(cursor.line));
        let cells = if cursor.wide { 2 } else { 1 };
        window.paint_quad(fill(
            cell_rect(cursor.line, cursor.column, cells),
            frame.cursor_color,
        ));
        let _ = glyph.paint(at, line_height, TextAlign::Left, None, window, cx);
    }
    if let (Some(line), Some((cursor, _))) = (&frame.ghost, &frame.cursor) {
        // After the cursor's own glyph: the first character shows through
        // a bar or underline cursor and sits on a block one.
        let at = point(cell_x(cursor.column), row_y(cursor.line));
        let _ = line.paint(at, line_height, TextAlign::Left, None, window, cx);
    }
    if let (Some((line, cells)), Some((cursor, _))) = (&frame.marked, &frame.cursor) {
        let rect = cell_rect(cursor.line, cursor.column, *cells);
        window.paint_quad(fill(rect, frame.background));
        let at = point(cell_x(cursor.column), row_y(cursor.line));
        let _ = line.paint(at, line_height, TextAlign::Left, None, window, cx);
    }
}

#[cfg(test)]
impl LayoutState {
    /// Files row `from`'s cached layout under row `to`'s slot, as a hash
    /// collision between the two rows' contents would.
    pub fn forge_collision(&mut self, from: usize, to: usize) {
        let entry = self.cache[&self.row_hashes[from]].clone();
        self.cache.insert(self.row_hashes[to], entry);
    }
}

#[cfg(test)]
impl Frame {
    /// The shaped pieces of a row: (column, text).
    pub fn segments(&self, row: usize) -> Vec<(usize, String)> {
        self.rows[row]
            .segments
            .iter()
            .map(|segment| (segment.column, segment.line.text.to_string()))
            .collect()
    }

    /// The background runs of a row: (column, cells).
    pub fn backgrounds(&self, row: usize) -> Vec<(usize, usize)> {
        self.rows[row]
            .backgrounds
            .iter()
            .map(|run| (run.column, run.cells))
            .collect()
    }

    /// The selected spans: (row, first column, last column).
    pub fn selection(&self) -> &[(usize, usize, usize)] {
        &self.selection
    }

    /// The search match spans on screen.
    pub fn matches(&self) -> &[MatchSpan] {
        &self.matches
    }

    /// The cursor cell and whether it is two cells wide.
    pub fn cursor(&self) -> Option<(usize, usize, bool)> {
        self.cursor
            .as_ref()
            .map(|(cursor, _)| (cursor.line, cursor.column, cursor.wide))
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::Line;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;
    use crate::terminal::colors::{FromTheme as _, token_rgb};
    use crate::terminal::model::GridDims;
    use crate::theme::terminal::builtin;

    fn style(selection_foreground: bool) -> FrameStyle {
        let mut theme = builtin()[0].clone();
        // Any theme color will do; the built-in themes leave it unset.
        theme.selection_foreground = selection_foreground.then_some(theme.ansi.0[3]);
        FrameStyle {
            palette: Palette::from_theme(&theme),
            fonts: Rc::new(FontSet::new("JetBrains Mono")),
            font_size: px(14.),
            line_height: 1.2,
            bold_is_bright: true,
            focused: true,
            marked_text: None,
            ghost_text: None,
            min_contrast: MinContrast::Off,
            cursor_hidden: false,
            search: None,
            selection: None,
            highlighting: false,
        }
    }

    fn term(text: &str) -> Term<VoidListener> {
        let mut term = Term::new(
            Config::default(),
            &GridDims { cols: 12, rows: 2 },
            VoidListener,
        );
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, text.as_bytes());
        term
    }

    fn fg_of(cells: &[CellOut]) -> Vec<(usize, Rgb)> {
        cells.iter().map(|cell| (cell.column, cell.fg)).collect()
    }

    /// An emoji sequence keeps the placement of its font's glyphs when they
    /// fit the sequence's cells (a ZWJ family has six), and is one glyph per
    /// two cells when they do not (D206).
    #[test]
    fn emoji_sequences_keep_the_fonts_placement_when_it_fits() {
        let cell = px(8.);
        assert!(sequence_keeps_its_placement(px(30.), cell, 6));
        assert!(sequence_keeps_its_placement(px(16.), cell, 2));
        assert!(!sequence_keeps_its_placement(px(16.5), cell, 2));
    }

    /// Wide characters are one cell in the output (spacers left out);
    /// hidden text becomes blanks; colors are resolved.
    #[test]
    fn row_cells_skip_spacers_and_resolve_colors() {
        let term = term("a你\x1b[8mz\x1b[0m\x1b[31mr");
        let style = style(false);
        let cells = row_cells(
            &term.grid()[Line(0)],
            12,
            &style,
            term.colors(),
            None,
            None,
            &mut ContrastCache::default(),
        );
        let texts: Vec<(usize, &str)> = cells[..4]
            .iter()
            .map(|cell| (cell.column, cell.text.0.as_str()))
            .collect();
        assert_eq!(texts, [(0, "a"), (1, "你"), (3, " "), (4, "r")]);
        assert_eq!(cells.len(), 11, "12 columns minus one spacer");
        let red = token_rgb(builtin()[0].ansi.0[1]);
        assert_eq!(cells[3].fg, red);
        assert_eq!(cells[0].fg, style.palette.foreground());
    }

    /// Keyword highlighting (UI 12.4, D67): words in the default text color
    /// take their category's color; text a program colored keeps it.
    #[test]
    fn keyword_highlighting_recolors_plain_text_only() {
        use crate::terminal::highlight::Category;

        let term = term("ok \x1b[31merror\x1b[0m");
        let style = style(false);
        let cells = row_cells(
            &term.grid()[Line(0)],
            12,
            &style,
            term.colors(),
            None,
            Some(true),
            &mut ContrastCache::default(),
        );
        let ok = Category::Ok.color(true);
        let red = token_rgb(builtin()[0].ansi.0[1]);
        assert_eq!(cells[0].fg, ok);
        assert_eq!(cells[1].fg, ok);
        assert_eq!(cells[2].fg, style.palette.foreground());
        // "error" is red from the program: not recolored.
        assert!(cells[3..8].iter().all(|cell| cell.fg == red));
        // Off: nothing changes.
        let off = row_cells(
            &term.grid()[Line(0)],
            12,
            &style,
            term.colors(),
            None,
            None,
            &mut ContrastCache::default(),
        );
        assert_eq!(off[0].fg, style.palette.foreground());
    }

    /// design-system 7.2: `selectionForeground` recolors selected text (a
    /// wide character when either of its cells is selected); without it the
    /// text keeps its colors.
    #[test]
    fn selection_foreground_recolors_selected_cells() {
        let term = term("ab你cd");
        let with = style(true);
        let selected_fg = with.palette.selection_foreground().expect("set");
        let foreground = with.palette.foreground();
        // Columns 1..=2: "b" and the first cell of 你.
        let cells = row_cells(
            &term.grid()[Line(0)],
            12,
            &with,
            term.colors(),
            Some((selected_fg, (1, 2))),
            None,
            &mut ContrastCache::default(),
        );
        assert_eq!(
            fg_of(&cells)[..5],
            [
                (0, foreground),
                (1, selected_fg),
                (2, selected_fg),
                (4, foreground),
                (5, foreground),
            ]
        );
        // Only the second cell of 你 (column 3) selected.
        let cells = row_cells(
            &term.grid()[Line(0)],
            12,
            &with,
            term.colors(),
            Some((selected_fg, (3, 3))),
            None,
            &mut ContrastCache::default(),
        );
        assert_eq!(fg_of(&cells)[2], (2, selected_fg));
        assert_eq!(fg_of(&cells)[1], (1, foreground));

        let without = style(false);
        assert_eq!(without.palette.selection_foreground(), None);
    }

    /// UI 12.4 "Adjust indistinguishable colors": black on ANSI blue in
    /// Paper (htop, mc) reaches 4.5:1 with "Palette only"; truecolor text
    /// is adjusted only with "Always"; "Off" and hidden text stay as sent.
    #[test]
    fn minimum_contrast_follows_the_setting() {
        use crate::terminal::contrast::{MIN_RATIO, ratio};

        let paper = builtin()[1].clone();
        assert_eq!(paper.name, "Paper");
        let mut style = style(false);
        style.palette = Palette::from_theme(&paper);
        let black = token_rgb(paper.ansi.0[0]);
        let blue = token_rgb(paper.ansi.0[4]);
        assert!(ratio(black, blue) < MIN_RATIO, "the pair is hard to read");
        // Palette black on palette blue, the same colors as truecolor, and
        // hidden palette text.
        let term =
            term("\x1b[30;44mP\x1b[0m\x1b[38;2;42;42;39;48;2;47;98;160mT\x1b[0m\x1b[8;30;44mH");
        let colors = |mode: MinContrast| {
            let mut style = style.clone();
            style.min_contrast = mode;
            let cells = row_cells(
                &term.grid()[Line(0)],
                12,
                &style,
                term.colors(),
                None,
                None,
                &mut ContrastCache::default(),
            );
            cells[..3]
                .iter()
                .map(|cell| (cell.fg, cell.bg.expect("a background")))
                .collect::<Vec<_>>()
        };
        let off = colors(MinContrast::Off);
        assert_eq!(off[0], (black, blue));
        assert_eq!(off[1], (black, blue), "truecolor as sent");
        let palette = colors(MinContrast::Palette);
        assert!(ratio(palette[0].0, blue) >= MIN_RATIO);
        assert_eq!(palette[0].1, blue, "the background never changes");
        assert_eq!(palette[1], (black, blue), "truecolor untouched");
        assert_eq!(palette[2].0, blue, "hidden text stays invisible");
        let always = colors(MinContrast::Always);
        assert!(ratio(always[0].0, blue) >= MIN_RATIO);
        assert!(ratio(always[1].0, blue) >= MIN_RATIO, "truecolor adjusted");
        assert_eq!(always[2].0, blue);
        // Readable pairs are left alone in every mode.
        let term = super::tests::term("\x1b[31mred");
        for mode in MinContrast::ALL {
            let mut style = style.clone();
            style.min_contrast = mode;
            let cells = row_cells(
                &term.grid()[Line(0)],
                12,
                &style,
                term.colors(),
                None,
                None,
                &mut ContrastCache::default(),
            );
            assert_eq!(cells[0].fg, token_rgb(paper.ansi.0[1]));
        }
    }
}
