# App icon

Variant B of the run 4 drafts, chosen by the owner (D83, item 3): an accent
tile (`#2EC4A6` of the design system) with a terminal window, its prompt and a
plus.

| File | Use |
|---|---|
| `icon-b-window.svg` | the master, from 32 px up |
| `icon-b-window-small.svg` | 16 to 24 px: the same tile and window on the 16 px grid, without the title bar's dots and the plus, which do not survive at that size |

The platform files in `apps/desktop/assets/icons/` (`source.svg` and
`source-small.svg` are copies of these two) are made with
`node scripts/gen-icon.mjs`: `icon.ico` with 16 to 256 px, embedded in
`shellpp.exe` and used for its window (`apps/desktop/build.rs`), `icon.icns`
and the sized PNGs.
