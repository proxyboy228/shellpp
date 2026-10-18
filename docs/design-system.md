# Design system

> The application's own visual style. All colors are defined by tokens.
> Component code contains no "raw" hex values, only variables from the tokens file.

## 1. Principles
- The base is a warm neutral graphite with no blue tint. There is a single accent:
  **jade**, a turquoise green.
- The accent is used only for the primary action on the screen, focus,
  selection and the active state. It is not used for decoration.
- Dangerous actions are indicated only by the `danger` color, never by the accent.
- Contrast of regular text is at least 4.5:1 (WCAG AA), of large text and icons
  at least 3:1. `text-muted` is allowed only for placeholders and secondary
  captions.
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
| `--bg-raised-hover` | hover over a card or row | `#2C2C29` | `#F0EFEA` |
| `--bg-input` | input fields inside blocks | `#1E1E1C` | `#F6F5F1` |
| `--bg-overlay` | menus, dropdowns, modal windows | `#2A2A27` | `#FFFFFF` |
| `--scrim` | dimming under a modal window | `rgba(0,0,0,0.55)` | `rgba(28,28,26,0.35)` |

### 2.2 Borders
| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--border-subtle` | dividers, card borders | `#2F2F2C` | `#E2E0D9` |
| `--border-strong` | field borders, menu dividers | `#3E3E3A` | `#CBC8BF` |

### 2.3 Text
| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--text-primary` | primary text | `#EDEDE8` | `#1C1C1A` |
| `--text-secondary` | card subtitles, captions, table columns | `#A6A59E` | `#5A5953` |
| `--text-muted` | placeholders, secondary captions | `#76756F` | `#8C8A83` |
| `--text-disabled` | inactive elements | `#55554F` | `#B3B1A9` |

### 2.4 Accent
| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--accent` | primary button (Connect, Run, Continue), card selection, enabled toggle | `#2EC4A6` | `#13866F` |
| `--accent-hover` | hover | `#4FD3B9` | `#0F7562` |
| `--accent-pressed` | press | `#23A68C` | `#0B6152` |
| `--accent-subtle` | background of the active navigation item, the selected row, tag chips | `rgba(46,196,166,0.14)` | `rgba(19,134,111,0.10)` |
| `--accent-text` | links and text buttons ("Add targets", "Share this host") | `#5AD6BE` | `#0F7562` |
| `--text-on-accent` | text on a button with an `--accent` background | `#08201B` | `#FFFFFF` |
| `--focus-ring` | focus outline: 2 px, 2 px offset | `#2EC4A6` | `#13866F` |

### 2.5 Semantic colors
| Token | Purpose | Dark | Light |
|---|---|---|---|
| `--danger` | Delete, Remove, Close in menus, errors | `#F0645C` | `#C0392F` |
| `--danger-hover` | hover over a dangerous action | `#F57E77` | `#A33027` |
| `--danger-subtle` | background of blocking warnings (host key change) | `rgba(240,100,92,0.14)` | `rgba(192,57,47,0.10)` |
| `--warning` | warnings, "connecting" | `#E3B04B` | `#9A6B0C` |
| `--warning-subtle` | warning background | `rgba(227,176,75,0.14)` | `rgba(154,107,12,0.10)` |
| `--success` | "connected", "rule running" | `#7CC46F` | `#3B8233` |
| `--info` | informational hints | `#7AA9DB` | `#2E6AA6` |

### 2.6 Shadows
| Token | Dark | Light |
|---|---|---|
| `--shadow-overlay` (menus, popovers) | `0 8px 24px rgba(0,0,0,0.50)` | `0 8px 24px rgba(28,28,26,0.14)` |
| `--shadow-modal` (modal windows) | `0 16px 48px rgba(0,0,0,0.60)` | `0 16px 48px rgba(28,28,26,0.18)` |

---

## 3. Object colors

### 3.1 Host and group labels
- The user picks the color of a host or group tile from 8 colors. The colors
  are the same in both themes.
- The icon on the tile is always white (`#FFFFFF`), with contrast against each color of at least 3:1.

| Token | Color |
|---|---|
| `--tag-graphite` | `#5B5A55` (default for hosts and groups) |
| `--tag-jade` | `#1E8A75` |
| `--tag-amber` | `#A87420` |
| `--tag-coral` | `#B8503F` |
| `--tag-plum` | `#85549A` |
| `--tag-moss` | `#5A7F35` |
| `--tag-ocean` | `#3A6890` |
| `--tag-rose` | `#A84B70` |

- OS icons are monochrome white on the host tile. Distribution brand colors
  are not used.
- A group is distinguished from a host by a folder icon, not by color.

### 3.2 Port forwarding type badges
White letter on a tile:
- L (Local): `--tag-jade`;
- R (Remote): `--tag-plum`;
- D (Dynamic): `--tag-amber`.

### 3.3 Status indicators
8 px dot:
- connected / running: `--success`;
- connecting: `--warning`;
- error / connection drop: `--danger`;
- inactive: `--text-muted`.

---

## 4. Default terminal themes

### 4.1 Graphite (dark, default)
| Role | Color | Role | Color |
|---|---|---|---|
| background | `#121211` | foreground | `#DCDCD5` |
| cursor | `#2EC4A6` | selection | `rgba(46,196,166,0.30)` |
| black | `#1E1E1C` | bright black | `#5A5A55` |
| red | `#E06C64` | bright red | `#F08A83` |
| green | `#8FC46A` | bright green | `#A9D88A` |
| yellow | `#E3B65A` | bright yellow | `#F0CC7E` |
| blue | `#6EA2D9` | bright blue | `#8DB9E8` |
| magenta | `#C08BD6` | bright magenta | `#D3A6E5` |
| cyan | `#4FC9B4` | bright cyan | `#74DCC9` |
| white | `#CFCFC8` | bright white | `#F5F5F0` |

### 4.2 Paper (light)
| Role | Color | Role | Color |
|---|---|---|---|
| background | `#FAF9F6` | foreground | `#2A2A27` |
| cursor | `#13866F` | selection | `rgba(19,134,111,0.20)` |
| black | `#2A2A27` | bright black | `#5C5B55` |
| red | `#B3403A` | bright red | `#CC5A52` |
| green | `#3F7D2E` | bright green | `#52973E` |
| yellow | `#8E6A12` | bright yellow | `#A87F1E` |
| blue | `#2F62A0` | bright blue | `#4078BC` |
| magenta | `#8A4AA0` | bright magenta | `#A262B8` |
| cyan | `#157A6A` | bright cyan | `#1F927F` |
| white | `#8A8982` | bright white | `#B9B7AF` |

### 4.3 Keyword highlighting (default colors)
| Category | Dark | Light |
|---|---|---|
| Error | `--danger` | `--danger` |
| Warning | `--warning` | `--warning` |
| OK | `--success` | `--success` |
| Info | `--info` | `--info` |
| Debug | `#A38FD9` | `#6B55B0` |
| IP and MAC | `#D883A6` | `#A84B70` |

---

## 5. Typography
- Interface: system font (Segoe UI Variable on Windows, SF Pro on macOS,
  system sans on Linux). Fallback: bundled Inter (OFL).
- Interface sizes:
  - 12 px: card subtitles, captions;
  - 13 px: dense lists and tables;
  - 14 px: body text;
  - 16 px: section headings ("Groups", "Hosts", "Keys");
  - 20 px: modal window titles.
- Font weights: 400, 500, 600.
- Terminal, script and key fields: bundled JetBrains Mono (OFL),
  14 px by default, line height 1.2.
- Numbers in tables (sizes, dates, times): `font-variant-numeric: tabular-nums`.

---

## 6. Shape and spacing
- 4 px grid. Spacing: 4, 8, 12, 16, 24, 32.
- Radii:
  - buttons, fields, chips: 6 px;
  - icon tiles: 8 px;
  - cards and panel blocks: 10 px;
  - menus and modal windows: 12 px.
- Heights:
  - top bar: 44 px;
  - buttons and fields: 36 px;
  - table rows: 44 px;
  - host cards: 64 px.
- Widths:
  - sidebar: 220 px;
  - details panel: 380 px, the width can be changed by dragging from 320 to 560 px.
- States:
  - hover: `--bg-raised-hover`;
  - pressed: one step darker than hover;
  - disabled: opacity 0.45, no reaction to hover;
  - keyboard focus: `--focus-ring`.
- Animations: 120–180 ms, ease-out. Disabled under `prefers-reduced-motion`.

---

## 7. Implementation
- Tokens live in `apps/desktop/src/theme/tokens.css`: CSS variables
  on `:root` (dark theme) and `[data-theme="light"]` (light).
- Terminal themes are stored as JSON in `apps/desktop/src/theme/terminal/`,
  in a format that supports importing and exporting color schemes (see SPEC.md).
- An automated test checks the contrast of all "text / background" pairs from this document
  against the thresholds from section 1.
