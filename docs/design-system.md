# Design system

> The application's own visual style. All colors are defined by tokens.
> Component code contains no "raw" hex values, only tokens from the theme (section 7).

Changes made by the stage 0 decisions (`docs/decisions.md`, 24–26) are marked
[decision].

## 1. Principles

- The base is a warm neutral graphite with no blue tint. There is a single accent:
  **jade**, a turquoise green.
- The accent is used only for the primary action on the screen, focus,
  selection and the active state. It is not used for decoration.
- Dangerous actions are indicated only by the `danger` color, never by the accent.
- Contrast of regular text is at least 4.5:1 (WCAG AA), of large text and icons
  at least 3:1. `text-muted` is allowed only for placeholders and secondary
  captions that repeat information shown elsewhere; its threshold is 3:1. [decision]
- The checked pairs and their thresholds are listed in 7.3. Ratios are computed
  with the WCAG 2.x formula and compared without rounding. [decision]
- Interface theme: Auto (follows the system), Light or Dark.

---

## 2. Interface tokens

### 2.1 Surfaces

| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--bg-base` | window background, top bar with tabs | `#121211` | `#F2F1EC` |
| `--bg-sidebar` | sidebar | `#171716` | `#ECEBE5` |
| `--bg-surface` | main content area | `#1B1B19` | `#F8F7F3` |
| `--bg-raised` | cards, details panel blocks | `#242422` | `#FFFFFF` |
| `--bg-raised-hover` | hover over a card, row or button | `#2C2C29` | `#F0EFEA` |
| `--bg-pressed` | pressed card, row or button [decision] | `#343431` | `#EDECE6` |
| `--bg-input` | input fields inside blocks | `#1E1E1C` | `#F6F5F1` |
| `--bg-overlay` | menus, dropdowns, modal windows, toasts | `#2A2A27` | `#FFFFFF` |
| `--bg-overlay-hover` | hover over a menu or dropdown item [decision] | `#333330` | `#F0EFEA` |
| `--scrim` | dimming under a modal window | `rgba(0,0,0,0.55)` | `rgba(28,28,26,0.35)` |

- The seven base surfaces are `--bg-base`, `--bg-sidebar`, `--bg-surface`,
  `--bg-raised`, `--bg-raised-hover`, `--bg-input` and `--bg-overlay`.
  `--bg-pressed` and `--bg-overlay-hover` are state surfaces. [decision]
- `--bg-pressed` is lighter than hover in the dark theme and darker in the light
  theme. In the light theme it stays light enough for the semantic text colors
  to keep 4.5:1. [decision]
- `--bg-overlay-hover` exists because hover with `--bg-raised-hover` is not visible
  on the dark `--bg-overlay` (1.03:1). In the light theme it equals
  `--bg-raised-hover`. [decision]

### 2.2 Borders

| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--border-subtle` | dividers, card borders, progress track | `#2F2F2C` | `#E2E0D9` |
| `--border-strong` | field borders, menu dividers, chip borders, off toggle track | `#3E3E3A` | `#CBC8BF` |

### 2.3 Text

| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--text-primary` | primary text | `#EDEDE8` | `#1C1C1A` |
| `--text-secondary` | card subtitles, captions, table columns, icons | `#A6A59E` | `#5A5953` |
| `--text-muted` | placeholders, secondary captions | `#76756F` | `#88867F` [decision] |
| `--text-disabled` | text of inactive menu items only (see 6) [decision] | `#55554F` | `#B3B1A9` |

- Light `--text-muted` was `#8C8A83` (2.89:1 on `--bg-sidebar`). The new value
  has at least 3.05:1 on every base surface. [decision]

### 2.4 Accent

| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--accent` | primary button (Connect, Run, Continue), card selection, enabled toggle, active tab bar, progress | `#2EC4A6` | `#13856E` [decision] |
| `--accent-hover` | hover | `#4FD3B9` | `#0F7461` [decision] |
| `--accent-pressed` | press | `#23A68C` | `#0B6051` [decision] |
| `--accent-subtle` | background of the active navigation item, the selected row, the active filter chip | `rgba(46,196,166,0.14)` | `rgba(19,133,110,0.10)` [decision] |
| `--accent-text` | links and text buttons ("Add targets", "Share this host") | `#5AD6BE` | `#0F7461` [decision] |
| `--text-on-accent` | text on a button with an `--accent` background | `#08201B` | `#FFFFFF` |
| `--focus-ring` | focus outline: 2 px, 2 px offset | `#2EC4A6` | `#13856E` [decision] |

- Light `--accent` and `--focus-ring` were `#13866F`: white text on it is
  4.4994:1, which passes only after rounding. On `#13856E` it is 4.56:1. [decision]
- Light `--accent-hover`, `--accent-pressed` and `--accent-text` keep their offsets
  from the base: the base moved by one step in the green and blue channels, and so
  did they (`#0F7562` → `#0F7461`, `#0B6152` → `#0B6051`). `--accent-text` stays
  equal to `--accent-hover`. [decision]

### 2.5 Semantic colors

| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--danger` | Delete, Remove, Close in menus, errors | `#F0665E` [decision] | `#C0392F` |
| `--danger-hover` | dangerous action under the pointer: text, or the danger button background | `#F57E77` | `#A33027` |
| `--danger-pressed` | pressed danger button [decision] | `#ED4A41` | `#852720` |
| `--danger-subtle` | background of blocking warnings (host key change) | `rgba(240,102,94,0.14)` [decision] | `rgba(192,57,47,0.10)` |
| `--text-on-danger` | text on a button with a `--danger` background [decision] | `#08201B` | `#FFFFFF` |
| `--warning` | warnings, "connecting" | `#E3B04B` | `#8D620B` [decision] |
| `--warning-subtle` | warning background | `rgba(227,176,75,0.14)` | `rgba(141,98,11,0.10)` [decision] |
| `--success` | "connected", "rule running" | `#7CC46F` | `#36782F` [decision] |
| `--success-subtle` | success background [decision] | `rgba(124,196,111,0.14)` | `rgba(54,120,47,0.10)` |
| `--info` | informational hints | `#7AA9DB` | `#2E6AA6` |
| `--info-subtle` | background of informational hints [decision] | `rgba(122,169,219,0.14)` | `rgba(46,106,166,0.10)` |

- Dark `--danger` was `#F0645C`. Re-checking decision 24 showed 4.46:1 on
  `--bg-raised-hover`, so its lightness was raised by the smallest step that
  passes (4.51:1). The hue is unchanged. [decision]
- Light `--warning` and `--success` were `#9A6B0C` and `#3B8233` (below 4.5:1 as
  text). The new values have at least 4.52:1 on every base surface. [decision]
- `--danger-pressed`: in the dark theme the base is darkened by the same step
  that hover lightens it; in the light theme it is two hover steps darker than
  the base. [decision]
- `--text-on-danger`: white on the dark `--danger` gives only 3.10:1, so the dark
  theme uses the same dark text as `--text-on-accent`. [decision]
- The `*-subtle` backgrounds are translucent and are drawn over the surface
  below. Text on them is always `--text-primary`; the semantic color goes only
  to the icon. In a selection (`--accent-subtle`), secondary columns may stay
  `--text-secondary`. [decision]

### 2.6 Shadows

| Token | Dark | Light |
|---|---|---|
| `--shadow-overlay` (menus, popovers, toasts) | `0 8px 24px rgba(0,0,0,0.50)` | `0 8px 24px rgba(28,28,26,0.14)` |
| `--shadow-modal` (modal windows) | `0 16px 48px rgba(0,0,0,0.60)` | `0 16px 48px rgba(28,28,26,0.18)` |

---

## 3. Object colors

### 3.1 Host and group labels

- The user picks the color of a host or group tile from 8 colors. The colors
  are the same in both themes.
- The icon on the tile is always white (`--text-on-tag`), with contrast against
  each color of at least 3:1.

| Token | Color |
|---|---|
| `--tag-graphite` | `#5B5A55` (default for hosts and groups) |
| `--tag-jade` | `#1D8571` [decision] |
| `--tag-amber` | `#9D6C1E` [decision] |
| `--tag-coral` | `#B8503F` |
| `--tag-plum` | `#85549A` |
| `--tag-moss` | `#5A7F35` |
| `--tag-ocean` | `#3A6890` |
| `--tag-rose` | `#A84B70` |
| `--text-on-tag` | `#FFFFFF` (icons on tiles, letters on badges) [decision] |

- `--tag-jade` and `--tag-amber` were `#1E8A75` and `#A87420`: white letters on the
  badges (3.2) had 4.24:1 and 4.05:1. The new values give 4.52:1 and 4.57:1. [decision]
- OS icons are monochrome white on the host tile. Distribution brand colors
  are not used.
- A group is distinguished from a host by a folder icon, not by color.

### 3.2 Port forwarding type badges

White letter (`--text-on-tag`) on a tile, with contrast of at least 4.5:1:

- L (Local): `--tag-jade`;
- R (Remote): `--tag-plum`;
- D (Dynamic): `--tag-amber`.

### 3.3 Status indicators

Dot of 8 px (`--size-status-dot`), with contrast of at least 3:1 on the base surfaces:

- connected / running: `--success`;
- connecting: `--warning`;
- error / connection drop: `--danger`;
- inactive: `--text-muted`.

### 3.4 Tag chips [decision]

- Tag chips are neutral: no fill, a 1 px `--border-strong` border,
  `--text-secondary` text and a delete button with a `--text-secondary` icon.
- Only the chip of an active filter gets the `--accent-subtle` fill with
  `--text-primary` text.
- Tag chips do not use the tile colors (3.1).

---

## 4. Default terminal themes

The theme format is described in 7.2. The role names below are the keys of the
theme file. Colors are `#RRGGBB`; `selection` is `#RRGGBBAA` and is drawn over
`background`.

### 4.1 Graphite (dark, default)

| Role | Color | Role | Color |
|---|---|---|---|
| `background` | `#121211` | `foreground` | `#DCDCD5` |
| `cursor` | `#2EC4A6` | `cursorText` | `#121211` [decision] |
| `selection` | `#2EC4A64D` (alpha 0.30) | `selectionForeground` | not set |
| `searchMatch` | `#4A3E1C` [decision] | `searchMatchActive` | `#6A5720` [decision] |
| `black` | `#1E1E1C` | `brightBlack` | `#62625C` [decision] |
| `red` | `#E06C64` | `brightRed` | `#F08A83` |
| `green` | `#8FC46A` | `brightGreen` | `#A9D88A` |
| `yellow` | `#E3B65A` | `brightYellow` | `#F0CC7E` |
| `blue` | `#6EA2D9` | `brightBlue` | `#8DB9E8` |
| `magenta` | `#C08BD6` | `brightMagenta` | `#D3A6E5` |
| `cyan` | `#4FC9B4` | `brightCyan` | `#74DCC9` |
| `white` | `#CFCFC8` | `brightWhite` | `#F5F5F0` |

- `brightBlack` was `#5A5A55` (2.70:1); the new value has 3.05:1. [decision]

### 4.2 Paper (light)

| Role | Color | Role | Color |
|---|---|---|---|
| `background` | `#FAF9F6` | `foreground` | `#2A2A27` |
| `cursor` | `#13856E` [decision] | `cursorText` | `#FFFFFF` [decision] |
| `selection` | `#13856E33` (alpha 0.20) [decision] | `selectionForeground` | not set |
| `searchMatch` | `#F2E2B0` [decision] | `searchMatchActive` | `#E5C46E` [decision] |
| `black` | `#2A2A27` | `brightBlack` | `#5C5B55` |
| `red` | `#B3403A` | `brightRed` | `#C6483F` [decision] |
| `green` | `#3F7D2E` | `brightGreen` | `#458034` [decision] |
| `yellow` | `#8E6A12` | `brightYellow` | `#916D1A` [decision] |
| `blue` | `#2F62A0` | `brightBlue` | `#3E74B6` [decision] |
| `magenta` | `#8A4AA0` | `brightMagenta` | `#9B57B3` [decision] |
| `cyan` | `#157A6A` | `brightCyan` | `#1B8070` [decision] |
| `white` | `#8A8982` | `brightWhite` | `#B9B7AF` |

- The bright colors were below 4.5:1 (bright red 3.87:1, bright green 3.41:1);
  the new values have at least 4.52:1. [decision]
- The cursor and the selection follow the light `--accent` (`#13856E`): with the
  old `#13866F`, white text under the cursor had 4.4994:1. [decision]

### 4.3 Keyword highlighting (default colors) [decision]

- The column is chosen by the background of the terminal theme, not by the
  interface theme. A background is light when black text has a higher contrast
  on it than white text (relative luminance above about 0.1791); otherwise it is
  dark.
- Colors are stored as hex. The defaults have at least 4.5:1 on the Graphite and
  Paper backgrounds; the user's colors and imported themes are not checked.
- The defaults for Error, Warning, OK and Info equal `--danger`, `--warning`,
  `--success` and `--info` of the interface theme with the same brightness.

| Category | Dark background | Light background |
|---|---|---|
| Error | `#F0665E` | `#C0392F` |
| Warning | `#E3B04B` | `#8D620B` |
| OK | `#7CC46F` | `#36782F` |
| Info | `#7AA9DB` | `#2E6AA6` |
| Debug | `#A38FD9` | `#6B55B0` |
| IP and MAC | `#D883A6` | `#A84B70` |

### 4.4 Bundled open themes [decision]

- Besides Graphite and Paper, the app bundles popular open color schemes. The
  license of each was checked in its official repository or site;
  `THIRD_PARTY_NOTICES.md` ("Terminal themes") gives the author, license,
  copyright line and source. The registry `apps/desktop/src/theme/terminal.rs`
  holds the same data: id (the file name), dark or light, the "new" mark,
  license and source.
- Theme lists show Graphite and Paper first, then the bundled schemes in table
  order. Themes added in a release are marked "new" in that release (UI 4.2).
- Dark or light follows the background rule of 4.3.
- The colors are those of the source, unchanged. The schemes are not checked
  against the contrast pairs of 7.3; where a scheme has low-contrast pairs, the
  "Adjust indistinguishable colors" setting (UI 12.4) helps. Ours, where the
  source has no value:
  - `searchMatch` and `searchMatchActive`: ANSI `yellow` drawn over
    `background` at 25 % and 40 %, each channel rounded;
  - `cursor` = `foreground` where the source names no cursor color (Dracula),
    `cursorText` = `background` where it names no cursor text (Dracula,
    Tokyo Night).
- Selection: the source's selection color. Solarized uses its highlight pair
  (`base02` with `base1` text in the dark mode, `base2` with `base01` in the
  light mode) as `selection` and `selectionForeground`; Catppuccin uses
  Overlay 2 at 30 % (its style guide gives 20–30 %); Gruvbox uses `bg3`
  without the inversion of its Vim scheme.

| Theme | Id | Background | License | Source |
|---|---|---|---|---|
| Dracula | `dracula` | dark | MIT | draculatheme.com/spec, Dracula Classic |
| Nord | `nord` | dark | MIT | nordtheme/alacritty, `src/nord.yaml` |
| Solarized Dark | `solarized-dark` | dark | MIT | altercation/solarized, README and `vim-colors-solarized` |
| Solarized Light | `solarized-light` | light | MIT | altercation/solarized, README and `vim-colors-solarized` |
| Gruvbox Dark | `gruvbox-dark` | dark | MIT | morhetz/gruvbox, `colors/gruvbox.vim` |
| Tokyo Night | `tokyo-night` | dark | Apache-2.0 (the original VS Code theme: MIT) | folke/tokyonight.nvim, `extras/windows_terminal/tokyonight_night.json` |
| Catppuccin Mocha | `catppuccin-mocha` | dark | MIT | catppuccin/palette, `palette.json` 1.8.0, and the Catppuccin style guide |
| Catppuccin Latte | `catppuccin-latte` | light | MIT | catppuccin/palette, `palette.json` 1.8.0, and the Catppuccin style guide |

---

## 5. Typography

- Interface (`--font-ui`): system font (Segoe UI Variable on Windows, SF Pro on
  macOS, system sans on Linux). Fallback: bundled Inter (OFL).
- Interface sizes:
  - 12 px (`--font-size-caption`): card subtitles, captions;
  - 13 px (`--font-size-dense`): dense lists and tables;
  - 14 px (`--font-size-body`): body text;
  - 16 px (`--font-size-heading`): section headings ("Groups", "Hosts", "Keys");
  - 20 px (`--font-size-title`): modal window titles.
- Font weights: 400 (`--font-weight-regular`), 500 (`--font-weight-medium`),
  600 (`--font-weight-semibold`).
- Terminal, script and key fields (`--font-mono`): bundled JetBrains Mono (OFL),
  14 px (`--font-size-mono`) by default, line height 1.2 (`--line-height-mono`).
- JetBrains Mono is bundled from stage 2a (D30). Inter is not bundled yet; the
  interface font stack falls back to installed fonts.
- Numbers in tables (sizes, dates, times): `font-variant-numeric: tabular-nums`.

---

## 6. Shape and spacing

- Units [decision] (D96): every size of sections 5 and 6 is a rem size: 1 rem is 14 px at
  100 % (the body text), and the px values given here are the sizes at 100 %. The
  interface zoom (UI 1.4) multiplies all of them, text included, and comes on top of the
  display's scale factor (Windows at 150 % and a 125 % zoom draw at 187.5 %).

  | At 100 % | rem | 80 % | 125 % | 150 % | 200 % |
  |---|---|---|---|---|---|
  | 4 px | 0.286 | 3.2 px | 5 px | 6 px | 8 px |
  | 8 px | 0.571 | 6.4 px | 10 px | 12 px | 16 px |
  | 12 px | 0.857 | 9.6 px | 15 px | 18 px | 24 px |
  | 14 px | 1 | 11.2 px | 17.5 px | 21 px | 28 px |
  | 16 px | 1.143 | 12.8 px | 20 px | 24 px | 32 px |
  | 24 px | 1.714 | 19.2 px | 30 px | 36 px | 48 px |
  | 32 px | 2.286 | 25.6 px | 40 px | 48 px | 64 px |
  | 36 px | 2.571 | 28.8 px | 45 px | 54 px | 72 px |
  | 44 px | 3.143 | 35.2 px | 55 px | 66 px | 88 px |
  | 196 px | 14 | 156.8 px | 245 px | 294 px | 392 px |
  | 380 px | 27.143 | 304 px | 475 px | 570 px | 760 px |

  Not zoomed: the terminal's font and grid (their own setting, 12.4), its 8 px padding,
  the 8 px hit zone of the pane dividers and the offset of a new window. A zoomed window
  plans its layout like a window as many times narrower at 100 %: 960 × 600 at 150 % is
  laid out as 640 × 400, the sidebar collapses and the bars wrap, and nothing is clipped
  or overlaps.
- 4 px grid. Spacing: 4 px (`--space-1`), 8 px (`--space-2`), 12 px (`--space-3`),
  16 px (`--space-4`), 24 px (`--space-5`), 32 px (`--space-6`).
- Radii:
  - buttons, fields, chips: 6 px (`--radius-control`);
  - icon tiles: 8 px (`--radius-tile`);
  - cards and panel blocks: 10 px (`--radius-card`);
  - menus and modal windows: 12 px (`--radius-overlay`).
- Heights:
  - top bar: 44 px (`--height-top-bar`);
  - tabs in the top bar and its icon buttons: 32 px (`--height-tab`), 10 px
    (`--padding-tab`) inside on the left and right, 4 px between tabs (`--space-1`)
    [decision];
  - buttons and fields: 36 px (`--height-control`);
  - table rows: 44 px (`--height-row`);
  - host cards: 64 px (`--height-host-card`);
  - sidebar items: 32 px (`--height-nav-item`), 2 px apart (`--gap-nav-item`), 12 px
    inside on the left and right (`--space-3`), a 16 px icon and 10 px
    (`--gap-nav-icon`) between the icon and the text; the sidebar keeps 8 px
    (`--space-2`) from its edges [decision].
- Widths:
  - sidebar: 196 px (`--width-sidebar`) [decision]; a 56 px icon strip in a narrow
    window (UI 1.2) [decision];
  - details panel: 380 px (`--width-details`), the width can be changed by
    dragging from 320 px (`--width-details-min`) to 560 px (`--width-details-max`), but
    never so wide that the main area gets narrower than 360 px; the chosen width comes back
    when the window grows [decision].
- Window: minimum size 960 × 600 px (UI 1, D29). At this width, with the details panel at
  380 px open, the sidebar collapses to its icon strip (UI 1.2) and the main area is 524 px
  wide. Every screen also fits the narrowest main area the details panel allows, 360 px,
  without clipping or overlap.
- States:
  - hover: `--bg-raised-hover` for cards, rows and buttons; `--bg-overlay-hover`
    for menu and dropdown items [decision];
  - pressed: `--bg-pressed` [decision];
  - a dangerous action that is hovered or pressed uses `--danger-hover` text;
    plain `--danger` text is not placed on state surfaces [decision];
  - `--text-muted` is not placed on state surfaces. Captions and shortcut hints
    in menus use `--text-secondary` [decision];
  - disabled [decision]: the whole component gets opacity 0.45
    (`--opacity-disabled`) and does not react to hover. `--text-disabled` is used
    only for the text of inactive menu items, which keep full opacity. The two
    are never combined;
  - keyboard focus: `--focus-ring`, 2 px (`--focus-ring-width`) with an offset of
    2 px (`--focus-ring-offset`).
- Indicators and derived states [decision]:
  - active session tab: a bar of 2 px (`--size-tab-indicator`) in `--accent`; the
    colors of the host's terminal theme are not used for it;
  - the Vaults tab and its vault switcher arrow are one pill: the active state
    (`--accent-subtle` and the `--accent` bar) spans both, and the arrow is a click zone
    of its own inside the pill with its own hover state;
  - toggle: on, an `--accent` track; off, a `--border-strong` track;
  - progress: `--accent` fill on a `--border-subtle` track;
  - toast: `--bg-overlay` with `--shadow-overlay`.
- Buttons [decision]:
  - every button of an action bar or an empty state has a 16 px Lucide icon to the left
    of its text; buttons of dialogs (OK, Cancel, Save, Import, Close) have none;
  - buttons of one group that stand side by side (Import and Export, Terminal and
    Serial) are equally wide: as wide as the widest of them;
  - labels of buttons and menu items are plain words ("Import known hosts", not
    "known_hosts"); file names and paths (`~/.ssh/known_hosts`) appear only in tooltips
    and descriptions, in the monospaced font.
- Animations: 120 ms (`--duration-fast`: hover, press, toggles) to 180 ms
  (`--duration-normal`: menus, panels, modal windows), ease-out
  (`--easing-standard`). Under `prefers-reduced-motion: reduce` the durations are
  0 ms, so components must not wait for `transitionend` or `animationend`.

---

## 7. Implementation

### 7.1 Theme

- The interface theme goes through the gpui-component theme system, which
  gpui-kit re-exports: the `Theme` global and its `ThemeColor` fields. Kit
  components (inputs, menus, tabs, the sidebar, the title bar, scrollbars,
  dialogs) paint from it, and our views read the same values with `cx.theme()`.
- The theme file is `apps/desktop/themes/graphite.json` in the gpui-component
  theme schema: one theme set, Graphite, with two themes, `"mode": "dark"` and
  `"mode": "light"`. Its values match this document. The mapping of our tokens
  to `ThemeColor` fields is in 7.4.
- Auto follows the system appearance; Light and Dark set the mode explicitly.
  A mode change applies to all windows at once.
- Colors that have no `ThemeColor` field are constants in
  `apps/desktop/src/theme/palette.rs`:
  - the tile colors `--tag-*` and `--text-on-tag` (3.1) and the port forwarding
    type badges (3.2);
  - the status dot colors (3.3);
  - the keyword highlighting defaults (4.3);
  - the theme-dependent tokens marked "palette.rs" in 7.4, each with a dark and
    a light value.
- A palette entry that reuses another token refers to it instead of repeating
  its value: a badge uses a `--tag-*` color, and a status dot uses `--success`,
  `--warning`, `--danger` or `--text-muted`.
- Tokens that are not colors (spacing, sizes, radii, fonts, motion) are Rust
  constants in `apps/desktop/src/theme/`. `graphite.json` also sets the kit's
  base values from them: `font.size` is `--font-size-body`, `radius` is
  `--radius-control` and `radius.lg` is `--radius-overlay`.
- Raw color values are allowed only in `apps/desktop/themes/graphite.json` and
  in `apps/desktop/src/theme/`. Views take colors from `cx.theme()` and
  `palette.rs`, never from literals.
- A Rust test in `apps/desktop/src/theme` compares the values of both modes of
  `graphite.json` and the `palette.rs` constants with the tables in sections 2–4
  and 7.4, and checks that `docs/ru/design-system.md` has the same values.

### 7.2 Terminal theme format [decision]

- Built-in themes are stored as JSON in `apps/desktop/src/theme/terminal/`, one
  file per theme. Imported themes are stored as records and synced;
  `.itermcolors` files and Windows Terminal schemes are converted to this format
  on import.
- The built-in files are plain JSON without comments; the registry
  `apps/desktop/src/theme/terminal.rs` lists them, and the source of a bundled
  open scheme (4.4) is recorded there. [decision]
- A theme is a UTF-8 JSON object with these fields:

| Field | Value | Required | Meaning |
|---|---|---|---|
| `name` | string, 1–64 characters | yes | name in the theme lists |
| `version` | integer `1` | yes | format version |
| `background` | `#RRGGBB` | yes | terminal background |
| `foreground` | `#RRGGBB` | yes | default text |
| `cursor` | `#RRGGBB` | yes | cursor |
| `cursorText` | `#RRGGBB` | yes | text under a block cursor |
| `selection` | `#RRGGBB` or `#RRGGBBAA` | yes | selection background, drawn over `background` |
| `selectionForeground` | `#RRGGBB` | no | text in the selection; when absent, text keeps its colors |
| `searchMatch` | `#RRGGBB` | yes | background of search matches |
| `searchMatchActive` | `#RRGGBB` | yes | background of the current search match |
| `ansi` | object | yes | the 16 ANSI colors |

- `ansi` has 16 keys, in color index order 0–15: `black`, `red`, `green`,
  `yellow`, `blue`, `magenta`, `cyan`, `white`, `brightBlack`, `brightRed`,
  `brightGreen`, `brightYellow`, `brightBlue`, `brightMagenta`, `brightCyan`,
  `brightWhite`. Each value is `#RRGGBB`.
- Hex digits may be upper or lower case. Version 1 allows no other fields: a
  file with an unknown field or another `version` is rejected with an error.
- The keyword highlighting column is chosen from `background` (4.3).
- Example (Graphite):

```json
{
  "name": "Graphite",
  "version": 1,
  "background": "#121211",
  "foreground": "#DCDCD5",
  "cursor": "#2EC4A6",
  "cursorText": "#121211",
  "selection": "#2EC4A64D",
  "searchMatch": "#4A3E1C",
  "searchMatchActive": "#6A5720",
  "ansi": {
    "black": "#1E1E1C",
    "red": "#E06C64",
    "green": "#8FC46A",
    "yellow": "#E3B65A",
    "blue": "#6EA2D9",
    "magenta": "#C08BD6",
    "cyan": "#4FC9B4",
    "white": "#CFCFC8",
    "brightBlack": "#62625C",
    "brightRed": "#F08A83",
    "brightGreen": "#A9D88A",
    "brightYellow": "#F0CC7E",
    "brightBlue": "#8DB9E8",
    "brightMagenta": "#D3A6E5",
    "brightCyan": "#74DCC9",
    "brightWhite": "#F5F5F0"
  }
}
```

### 7.3 Contrast pairs [decision]

- The automated test is a Rust test in `apps/desktop/src/theme`, run by
  `cargo test`. It reads both modes of `apps/desktop/themes/graphite.json`, the
  `palette.rs` constants and the built-in terminal themes, draws translucent
  colors over the surface below them, computes the WCAG 2.x contrast ratio and
  compares it with the threshold without rounding.
- The pair list is kept in the same test module and mirrors the tables below;
  change both together.
- "Base surfaces" are the seven surfaces of 2.1; "all surfaces" adds
  `--bg-pressed` and `--bg-overlay-hover`.

Interface pairs, checked in both themes:

| Group | Foreground | Background | Threshold |
|---|---|---|---|
| Text | `--text-primary`, `--text-secondary`, `--accent-text`, `--danger`, `--danger-hover`, `--warning`, `--success`, `--info` | base surfaces | 4.5 |
| Text on state surfaces | `--text-primary`, `--text-secondary`, `--accent-text`, `--danger-hover`, `--warning`, `--success`, `--info` | `--bg-pressed`, `--bg-overlay-hover` | 4.5 |
| Primary button | `--text-on-accent` | `--accent`, `--accent-hover`, `--accent-pressed` | 4.5 |
| Danger button | `--text-on-danger` | `--danger`, `--danger-hover`, `--danger-pressed` | 4.5 |
| Text on tinted backgrounds | `--text-primary` | each `*-subtle` over each base surface | 4.5 |
| Secondary text in a selection | `--text-secondary` | `--accent-subtle` over `--bg-sidebar`, `--bg-surface`, `--bg-raised`, `--bg-overlay` | 4.5 |
| Badge letters | `--text-on-tag` | `--tag-jade`, `--tag-plum`, `--tag-amber` | 4.5 |
| Muted text | `--text-muted` | base surfaces | 3 |
| Status dots | `--success`, `--warning`, `--danger`, `--text-muted` | base surfaces | 3 |
| Focus ring and accent indicators | `--focus-ring`, `--accent` | all surfaces | 3 |
| Icons on tinted backgrounds | `--accent`, `--danger`, `--warning`, `--success`, `--info` | its own `*-subtle` over each base surface | 3 |
| Tile icons | `--text-on-tag` | all eight `--tag-*` | 3 |
| Progress | `--accent` | `--border-subtle` | 3 |

Terminal pairs, checked for Graphite and Paper (the bundled open schemes of 4.4
keep their original colors):

| Group | Foreground | Background | Threshold |
|---|---|---|---|
| Text | `foreground` | `background` | 4.5 |
| ANSI colors 1–6 and 9–14 | `red` … `cyan`, `brightRed` … `brightCyan` | `background` | 4.5 |
| Bright black | `brightBlack` | `background` | 3 |
| Selected text | `selectionForeground`, or `foreground` when it is not set | `selection` over `background` | 4.5 |
| Text under the cursor | `cursorText` | `cursor` | 4.5 |
| Cursor | `cursor` | `background` | 3 |
| Search matches | `foreground` | `searchMatch`, `searchMatchActive` | 4.5 |
| Keyword highlighting | defaults of the column chosen for `background` (4.3) | `background` | 4.5 |

- Interface icons use `--text-primary`, `--text-secondary`, `--accent` or a
  semantic color, so they are covered by the pairs above.
- Not checked:
  - `--text-disabled` and anything at opacity 0.45 (inactive components);
  - `--text-muted` and plain `--danger` on state surfaces, where they are not
    used (6);
  - ANSI `black`, `white` and `brightWhite` of terminal themes, and ANSI colors
    on the selection or on search matches;
  - the bundled open schemes (4.4), imported themes and colors chosen by the
    user.

### 7.4 Tokens and `ThemeColor` fields

- Field names are those of `ThemeColor` in gpui-component 0.7.3; the last
  column gives the key in `graphite.json`. Both modes use the same mapping.
- gpui-component's `accent` is **not** our accent. It is the background of a
  hovered or highlighted menu or list item (menu items, command and completion
  lists, the hover of a ghost button). Our `--accent` maps to `primary` and the
  `button_primary` fields; `accent` takes `--bg-overlay-hover`.
- `muted_foreground` takes `--text-secondary`, not `--text-muted`: the kit uses
  it for menu shortcut hints, secondary labels, tab text and icons, which
  section 6 requires in `--text-secondary`.
- "palette.rs" means that the token has no `ThemeColor` field (7.1).
- A field that is not in the table is not used by our screens. `graphite.json`
  still sets it to the token closest in role, so no kit default color appears.
- Where a kit component paints a surface or a state differently from sections 2
  and 6 (for example, the kit dialog uses `background`, and the kit derives the
  input background from `input`), our view or wrapper component sets that color
  from the tokens.

| Token | `ThemeColor` fields | Keys in `graphite.json` |
|---|---|---|
| `--bg-base` | `background`, `title_bar`, `tab_bar` | `background`, `title_bar.background`, `tab_bar.background` |
| `--bg-sidebar` | `sidebar` | `sidebar.background` |
| `--bg-surface` | `list`, `table` | `list.background`, `table.background` |
| `--bg-raised` | `secondary`, `muted`, `group_box` | `secondary.background`, `muted.background`, `group_box.background` |
| `--bg-raised-hover` | `secondary_hover`, `button_hover`, `list_hover`, `table_hover` | `secondary.hover.background`, `button.hover.background`, `list.hover.background`, `table.hover.background` |
| `--bg-pressed` | `secondary_active`, `button_active` | `secondary.active.background`, `button.active.background` |
| `--bg-input` | palette.rs | — |
| `--bg-overlay` | `popover` | `popover.background` |
| `--bg-overlay-hover` | `accent` | `accent.background` |
| `--scrim` | `overlay` | `overlay` |
| `--border-subtle` | `border`, `sidebar_border`, `title_bar_border`, `window_border`, `table_row_border` | `border`, `sidebar.border`, `title_bar.border`, `window.border`, `table.row.border` |
| `--border-strong` | `input`, `switch` | `input.border`, `switch.background` |
| `--text-primary` | `foreground`, `accent_foreground`, `popover_foreground`, `secondary_foreground`, `sidebar_foreground`, `sidebar_accent_foreground`, `tab_active_foreground` | `foreground`, `accent.foreground`, `popover.foreground`, `secondary.foreground`, `sidebar.foreground`, `sidebar.accent.foreground`, `tab.active.foreground` |
| `--text-secondary` | `muted_foreground`, `tab_foreground` | `muted.foreground`, `tab.foreground` |
| `--text-muted` | palette.rs | — |
| `--text-disabled` | palette.rs | — |
| `--accent` | `primary`, `button_primary`, `sidebar_primary`, `progress_bar`, `caret` | `primary.background`, `button.primary.background`, `sidebar.primary.background`, `progress.bar.background`, `caret` |
| `--accent-hover` | `primary_hover`, `button_primary_hover` | `primary.hover.background`, `button.primary.hover.background` |
| `--accent-pressed` | `primary_active`, `button_primary_active` | `primary.active.background`, `button.primary.active.background` |
| `--accent-subtle` | `list_active`, `table_active`, `sidebar_accent`, `tab_active` | `list.active.background`, `table.active.background`, `sidebar.accent.background`, `tab.active.background` |
| `--accent-text` | `link`, `link_hover`, `link_active` | `link`, `link.hover`, `link.active` |
| `--text-on-accent` | `primary_foreground`, `button_primary_foreground`, `sidebar_primary_foreground` | `primary.foreground`, `button.primary.foreground`, `sidebar.primary.foreground` |
| `--focus-ring` | `ring` | `ring` |
| `--danger` | `danger`, `button_danger` | `danger.background`, `button.danger.background` |
| `--danger-hover` | `danger_hover`, `button_danger_hover` | `danger.hover.background`, `button.danger.hover.background` |
| `--danger-pressed` | `danger_active`, `button_danger_active` | `danger.active.background`, `button.danger.active.background` |
| `--danger-subtle` | palette.rs | — |
| `--text-on-danger` | `danger_foreground`, `button_danger_foreground` | `danger.foreground`, `button.danger.foreground` |
| `--warning` | `warning` | `warning.background` |
| `--warning-subtle` | palette.rs | — |
| `--success` | `success` | `success.background` |
| `--success-subtle` | palette.rs | — |
| `--info` | `info` | `info.background` |
| `--info-subtle` | palette.rs | — |
| `--shadow-overlay`, `--shadow-modal` | palette.rs (the kit theme has only an on/off `shadow` setting) | — |
| `--tag-*`, `--text-on-tag` | palette.rs | — |
