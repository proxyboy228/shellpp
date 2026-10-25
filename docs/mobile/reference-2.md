# Mobile reference, part 2: editors, pickers, Keychain, Known hosts, SFTP, terminal, vault switcher, terminal settings

> Clean-room description written by Claude from screenshots taken over adb
> on the owner's phone (2026-10-25). Agents never see the screenshots.
> Like part 1, this file describes **structure and behaviour only**: where
> things are, how screens connect, what a tap or a scroll does. It leaves
> out the other product's icons, artwork, colours, logos and wording, and
> every address, host name, login, key and password seen on the phone.
> Labels below are generic terms of the trade or plain descriptions; where
> the other client shows a sentence (empty states, explanations), this
> file says only that a sentence is there and what it is for.
>
> Destination in the repository: `docs/mobile/reference-2.md`, beside
> `docs/mobile/reference-1.md`. It continues part 1 and uses its section
> names (Hosts screen, session bar, extra-keys row…). §10 lists where this
> file corrects or extends part 1.
>
> Marks: **[not tested]** — seen on screen but not tapped, so what it does
> is a guess or unknown; **[owner to decide]** — a question for the owner
> that this file does not settle.
>
> The **[owner to decide]** items and the host card subtitle (§10) are
> settled by decision D219 (plan of run 15); where D219 differs from a
> recommendation here, D219 wins.

## 1. Host editor

Opened from the Hosts screen: long press on a host card (selection mode),
then the pencil in the top bar. "New host" in the floating "+" sheet opens
the same screen, empty (§2).

### 1.1 Top bar

- left: a back arrow;
- next to it: the title ("Edit host" / "New host");
- right: a check mark button that saves.
- No overflow menu, no delete here (deleting is in selection mode).
- Leaving with Back or the back arrow **when nothing was changed** closes
  the editor at once, without a prompt, and returns to the Hosts screen,
  scrolled back to the top. What happens with unsaved changes (a discard
  prompt or not) **[not tested]**.

### 1.2 Layout

One scrolling column of rounded cards. Inside cards, form rows use one
pattern:

- an **underlined text field** with a floating label (the label sits in
  the field as a placeholder when it is empty and moves above the value
  when it is focused or filled);
- optionally a **round action button** to the right of the field, in the
  accent colour. It opens a picker for that field (below);
- yes/no settings are a **plain label with a square check box** on the
  right (not switches);
- value-choice rows show the **label on the left and the current value
  on the right**, with no field underline below the value; tapping the row
  opens a picker.

Order from top to bottom:

1. **Vault selector** — a full-width rounded button above the first card:
   vault icon, vault name, drop-down arrow. It sets which vault the host
   is saved in **[not tested]**.
2. **Card: identity of the host**
   - Alias (label) — text field. On opening, this field is focused (the
     underline is highlighted), but the keyboard does not open by itself.
   - Address (host name or IP) — text field with a round button on its
     right. What the button does **[not tested]** (its glyph suggests
     looking up devices on the network).
   - Group — read-only field showing the group's name; tapping it or its
     round button opens the group picker (§3.1). With a group set, the
     round button turns into a **clear (×)** button that removes the
     group; without one, it shows the group glyph and opens the picker.
   - Tags — a text field with autocomplete for several tags, plus a round
     button that opens the tag picker (§3.2).
   - a check box that makes Backspace send Ctrl-H.
3. **Card: protocol and port** (separated from the next section by a thin
   full-width divider)
   - SSH — check box (on by default).
   - Mosh — check box, with an inline help link right after the label.
   - Port — text field, default 22.
4. **Credentials** (section with a bold header)
   - Username — text field + round button → identity picker (§3.3).
   - Password — masked text field + round button with a lock glyph.
     What the lock button does **[not tested]** (not pressed: it may show
     the password).
   - Key — one field for "SSH key, certificate, FIDO2 key or SSH ID",
     + round button → key picker (§3.4).
5. **Card: session options**
   - Snippet — field + round button → snippet picker (§3.5): a command
     run after connecting.
   - Agent forwarding — check box.
   - Host chaining — field + round button → chain editor (§3.6).
   - Proxy — field + round button → proxy editor (§3.7).
   - a text link in the accent colour, starting with "+", to add an
     environment variable; it opens a bottom sheet (§3.8). Added variables presumably appear as
     rows above the link **[not tested]**.
   - Colour scheme — value row (current theme name on the right) → theme
     picker (§3.9).
   - Charset — value row (UTF-8 by default) → charset picker (§3.10).
6. **Card: Telnet** — one check box. What appears when it is on (port?)
   **[not tested]**.

The editor ends after the Telnet card. The same fields and order are used
for an existing host and a new one.

**Screenshots of the editor were not blocked** on the owner's phone (both
password fields were empty). Part 1 said the other client blocks screens
with secrets by default; on this phone that is either turned off or does
not cover the editor.

## 2. New host and new group

### 2.1 New host

"+" (floating button on Hosts) → bottom sheet (part 1, §2.2) → New host.
Same screen as §1 with the title changed and all fields empty (Port 22,
SSH on, Colour scheme and Charset showing the app defaults). Leaving with
Back closed it without a prompt and saved nothing.

### 2.2 New group

"+" → New group. Same editor pattern, title "New group", check mark to
save. It is the host editor without the host-only rows:

1. vault selector;
2. card: Alias; **Parent group** (field + round button → the group
   picker, §3.1);
3. card: SSH, Mosh (with help link), Port;
4. Credentials: Username (+ identities), Password (+ lock), Key (+ keys);
5. card: Snippet, Agent forwarding, Host chaining, Proxy, Colour scheme,
   Charset;
6. card: Telnet.

Missing compared with a host: address, tags, the Ctrl-H check box,
environment variables. So everything in a group except its name and
parent is a default its hosts inherit (the inheritance rules are not
visible on screen; Shell++ keeps its own rules from `ui-spec.md`).
Leaving with Back saved nothing.

## 3. Pickers and sub-editors opened from the editors

All of these are full-screen pages pushed over the editor (except §3.8).
Back returns to the editor with its scroll position kept and nothing
chosen.

### 3.1 Group picker

- Top bar: back arrow, title, search, sort (the same two-letter sort
  badge as on Hosts), and a check mark (confirm) on the right.
- Content: a "Groups" section header and the same group cards as on the
  Hosts screen (tile, name, number of hosts).
- Floating "+" bottom right: create a group from here **[not tested]**.

### 3.2 Tag picker

- Top bar: back arrow, title, search, sort. No check mark.
- Content: one rounded container of rows; each row is a tag name on the
  left and a muted check mark on the right that lights up when the tag is
  selected (multi-select).
- Floating "+": new tag **[not tested]**.

### 3.3 Identity picker

- Top bar: back arrow, title, search, sort.
- Empty state (no identities on this phone): a rounded square icon tile in
  the middle of the screen, a bold title under it, and a two-line muted
  explanation of what identities are for.
- Floating "+": new identity **[not tested]**.

### 3.4 Key picker

- Top bar: back arrow, title, search, sort.
- With an empty keychain it shows only "create" rows, in one rounded
  container, each with a small glyph on the left:
  - set up an SSH ID (with a small "new" badge on the right);
  - new SSH key… ;
  - new certificate… ;
  - new biometric key;
  - new FIDO2 key.
- Existing keys would presumably be listed too **[not tested]** — the
  keychain on the phone was empty.

### 3.5 Snippet picker

Same as the identity picker: top bar with search and sort, an empty state
(icon tile, title, explanation), floating "+".

### 3.6 Chain editor (host chaining)

- Top bar: back arrow, title, check mark.
- Content, top to bottom:
  1. a card with a one-line explanation and, inside it, a full-width
     **add-a-host button** (adds a jump host before this one);
  2. a down arrow;
  3. a card for the host being edited (its tile; its name — empty for a
     new host);
  4. a destructive (red) **Clear** button, left-aligned, that empties the
     chain.

So the chain reads top to bottom: jump hosts first, the target last.

### 3.7 Proxy editor

- Top bar: back arrow, title ("New proxy"), check mark.
- Type — value row with a drop-down (HTTP by default; the list
  **[not tested]**).
- Address — text field. Port — text field.
- Credentials section: Username (+ identity button), Password
  (+ lock button).

### 3.8 New environment variable (bottom sheet)

A bottom sheet with a drag handle, a bold title, two outlined text fields
(variable name, value) and a full-width Save button that stays disabled
until the fields are filled. Back dismisses it.

### 3.9 Theme picker (per host)

- Top bar: back arrow, title, check mark.
- A **three-column grid** of theme tiles. Each tile is a miniature terminal
  (a few rounded bars in the theme's colours on its background), with the
  theme name centred under it. The selected tile has an accent outline
  and its name in the accent colour.
- The first tile is **Default** — "use the app's theme" (§9.3) — and it is
  what a new host has selected; the editor row then shows the app theme's
  name.

### 3.10 Charset picker

- Top bar: back arrow, title. No check mark (a tap chooses and returns —
  presumably).
- Two rows at the top with an accent caption on the left and a value on
  the right: **Current** and **Use default**.
- Then a long plain list of charset names (no cards, no dividers),
  scrolling.

## 4. Keychain

Vault home → Keychain.

- **Top bar** like Hosts: back arrow + vault name on the left (returns to
  the vault home), title centred, search and sort on the right.
- **Empty state** (the phone's keychain was empty): icon tile, bold title,
  one-line explanation, centred in the screen. The filled list was not
  seen **[not tested]**.
- **Floating "+"** bottom right → bottom sheet with a drag handle and three
  groups of rows, separated by thin dividers, each row with a glyph on the
  left:
  1. new identity;
  2. new SSH key… ; new certificate… ;
  3. new biometric key; new FIDO2 key.
  
  The "…" suggests a second choice (generate or import) **[not tested]**.
  Back dismisses the sheet.

## 5. Known hosts

Vault home → Known hosts.

- **Top bar** like Keychain (back to the vault, title, search, sort).
- **No floating button** — entries are only added by connecting.
- **One card per entry**, same card shape as host cards:
  - a rounded square tile with the same glyph for every entry;
  - title: the address;
  - subtitle (muted), only when saved hosts use that address: their
    labels, comma-separated. A label is repeated when two saved hosts with
    the same label share the address ("A, A"). Entries no host refers to
    have no subtitle, and the title is then vertically centred.
- The default order is plain text order of the address, so "13.x" comes
  after "109.x". Shell++ may sort addresses numerically
  **[owner to decide]**.
- Tapping or long-pressing an entry **[not tested]**.

## 6. Connections tab while sessions are open

Part 1 (§3) described this tab with no session open. With a session open
it looks different:

- The quick-connect field stays at the top.
- Below it, **one card per open session**:
  - the upper part is a **live preview**: the last lines of that session's
    output in the terminal's font and colours, cropped to the card;
  - the lower part is a row: the host's OS tile, the session name, the
    protocol in muted text under it, and a **×** button on the right that
    closes the session.
  - Tapping the preview opens the session (terminal screen).
- **Recent connections and "other ways to connect" are not shown** while a
  session is open; the space below the session cards is empty.
- After the last session closes, the tab shows Recent connections again
  (with the session just closed at the top). Tapping a recent row
  reconnects at once, straight into the terminal, with no prompt when
  credentials are saved (confirmed).
- The tab's icon is the session host's OS mark while a session is open,
  and a generic terminal glyph when none is.

## 7. Terminal screen (continues part 1, §5)

### 7.1 Opening, keyboard, leaving

- Opening a session (from a host card, a recent row or a session card)
  shows the terminal **with the system keyboard already open**.
- The right-most button of the session bar shows a **keyboard glyph while
  the keyboard is hidden** and a **2×2 grid glyph while it is shown**.
  What tapping it does while the keyboard is shown **[not tested]**.
- System Back while the keyboard is open only hides the keyboard.
- System Back with the keyboard hidden **left the terminal** (tested with
  a text selection active, §7.2) to the screen the session was opened
  from; the session stays open. The "<" button of the session bar does
  the same: it returned to Connections, or to the Hosts search results,
  whichever the session had been opened from.

### 7.2 Selecting text

- **Long press on printed text** selects a short run of characters
  around the finger (part of a word: the start did not snap to the start
  of the word). The highlight is a block in the accent colour behind the
  characters.
- Two **teardrop handles** appear under the start and the end of the
  selection; dragging a handle extends or shrinks the selection one
  character at a time, across lines.
- A **floating action bar** appears above the selection with three text
  buttons: Copy, Paste, Select all. It moves to stay centred above the
  selection as it changes.
- Pressing system Back while text was selected did not just clear the
  selection: it left the terminal (§7.1). Shell++ should let Back clear
  the selection first **[owner to decide]**.

### 7.3 The four icon keys at the start of the extra-keys row

Left to right:

1. **AI command key** — toggles a panel that sits just above the session
   bar: a rounded card with a one-line title and a single-line text input
   (with a muted example as placeholder) plus a square "generate" button on
   its right. Opening it focuses the input and opens the keyboard;
   hiding the keyboard keeps the panel; tapping the key again closes the
   panel and gives the focus back to the terminal. Shell++ has no AI
   feature in 1.0 **[owner to decide]**.
2. **Password key** — one tap **types the host's saved password into the
   terminal at the cursor**, at once: no picker, no confirmation, no
   Enter. On the phone the password appeared on the shell's prompt line,
   in clear text (the shell echoes it), and it appeared twice after a
   single tap. Recommendation for Shell++: if it has such a key at all,
   it must not send a secret without a confirmation, and should only
   offer it while the remote side is asking for a password (for example
   at a `sudo` prompt) **[owner to decide]**.
3. **Cursor key** — a toggle: when on, the key gets a filled rounded
   background. With it on, dragging on the terminal presumably moves the
   cursor with arrow keys (part 1 called it a cursor-pad key). The drag
   itself **[not tested]** (it would send keys to the server).
4. **Attach key** — opens the **system file picker** (the Android
   document picker, not a screen of the app). Back cancels and returns to
   the terminal with nothing sent. What happens after picking a file (an
   upload into the current directory?) **[not tested]**.

### 7.4 The extra-keys row scrolls

- The row **scrolls sideways** (confirmed). The four icon keys are part of
  the scrolling content and scroll away with the rest; nothing is pinned.
- The row is all the groups of Settings → Customize keyboard (§9.2), in
  order, laid out one after another.
- After the visible "shift+tab, ctrl, esc, ←" come ↑ ↓ →, then the tmux
  prefix combinations, and so on through the groups.

### 7.5 Several sessions: the chip strip

When a second session is open (here: an SFTP session to the same host),
the session bar becomes:

- "<" on the left (unchanged);
- a **horizontally scrolling strip of session chips** in the middle:
  - the **active session's chip** is shown in full: its tile, its name, and
    a **×** that closes that session;
  - **inactive chips** have no ×; the strip scrolls so the active chip is
    in view, and neighbouring chips peek in at the strip's edges, cut off
    and faded (on the left the end of the name shows; on the right only
    the start of the chip with its tile);
  - an SFTP session's tile has a **folder shape** (the host's OS mark
    inside a folder outline), so terminal and file sessions to the same
    host can be told apart;
- "+" (unchanged);
- the keyboard button — **only on terminal sessions**; on an SFTP session
  the bar ends with "+".

**Tapping an inactive chip switches** to that session at once (terminal ↔
files in this test). With only one session, tapping the chip does
nothing visible.

**× closes the session at once, with no confirmation.** When the active
session closes and another remains, the bar switches to the remaining one.

### 7.6 The "+" pop-up

"+" in the session bar opens a pop-up anchored to the bottom right,
above the bar, overlapping the right part of the session chip:

- Connect (a new session to the same host — presumably);
- Connect via SFTP;
- Connect via Serial (not in Shell++, part 1).

**Incident to keep in mind:** choosing "Connect via SFTP" here closed the
open terminal session and returned to the Connections tab, with no SFTP
session opened. Either the item behaves that way, or the tap went through
to the × of the session chip lying under the pop-up. Either way, Shell++
should not let a menu sit over a close button, and closing a session
from the bar could ask for confirmation **[owner to decide]**.

## 8. SFTP screen

Opened from the Hosts screen: the host's session counter ("1 ▾") →
Connect via SFTP. It connected straight away (saved credentials; no
prompt), and opened **as a new session in the same terminal screen**, with
the same session bar (§7.5). There is no extra-keys row and no system
keyboard.

### 8.1 Layout

- **Top bar:**
  - left: the current path as a **breadcrumb** (segments separated by
    chevrons; the parent segments muted, the current folder emphasised);
  - right: an **Upload** button (a small outlined pill with an icon and a
    label), an overflow (⋮) button, a search button.
- **Content:** a large rounded panel (like a sheet) filling the screen
  below the top bar, holding the listing:
  - first row: ".." with a folder icon — up one level;
  - one row per folder or file: a file or folder icon on the left; the
    **name** as the title; under it, muted and in a monospace font, the
    **permission string** (`-rw-r--r--`); on the right, the **size**
    (human units: bytes, MB) and under it the **modification date and
    time**, both in monospace.
  - no dividers between rows.
- Tapping a row (open a folder, open or download a file) **[not tested]**.

### 8.2 Overflow (⋮) menu

A pop-up from the top right, rows with a glyph on the left, in three
groups separated by thin dividers:

1. New folder;
2. Sort by name / by date / by size / by kind — the active sort shows a
   direction indicator on the right of its row;
3. Change download folder (on the phone); Copy path; Show or hide hidden
   files; Show guide.

### 8.3 Selection mode

- **Long press on a row** selects it: its icon becomes a filled check
  circle.
- The top bar is replaced by: **×** (leave selection), "N selected", and
  an overflow (⋮).
- The ⋮ menu: Copy; Download; Edit (open in a text editor); Rename; Copy
  path; a divider; **Delete**.
- × returns to the normal top bar.
- Selecting more rows by tapping them while in selection mode — presumably,
  as on Hosts **[not tested]**.

## 9. Vault switcher and terminal settings

### 9.1 Vault switcher

The vault name with the drop-down arrow at the top of the vault home opens
a **bottom sheet** with a drag handle:

- one row per vault: vault icon, vault name;
- the current vault has a check mark on the right;
- a team vault row has, on its right, a muted "invite members" action
  with a person-plus glyph.

Back or a tap outside dismisses it. (Shell++ 1.0: one personal vault,
part 1, §2.1.)

### 9.2 Settings → Customize keyboard

- **Top bar:** back arrow, title centred, a **reset** button (circular
  arrow) on the right **[not tested]**.
- A one-line muted explanation under the title. The title and the
  explanation stay fixed; the list scrolls under them.
- **A list of key groups**, each its own full-width rounded card:
  - a **delete** (trash) button on the left;
  - **up to four keys** in the middle, each a small rounded chip with the
    key's label or glyph;
  - a **drag handle** (two horizontal lines) on the right — reordering
    groups changes the order in the extra-keys row.
- **Floating "+"** bottom right: add a group **[not tested]**.
- Tapping a key chip (edit the key?) **[not tested]**.
- No save button: changes presumably apply at once. Nothing was changed.

Groups on the phone (the default set), by kind, in order:

1. the four icon keys (§7.3);
2. shift+tab, ctrl, esc;
3. the four arrows;
4. two groups of tmux prefix combinations (the prefix followed by a pane
   split, switching to the next pane or closing a pane);
5. alt, tab, ins, del;
6. home, pgUp, pgDn, end;
7. six groups of punctuation and brackets that are awkward on a phone
   keyboard;
8. F1–F12 in three groups of four;
9. three groups of Ctrl combinations (ctrl+C, ctrl+L, ctrl+Z, ctrl+R…).

In total, about nineteen groups.

All nineteen default groups as seen (checked by Claude on every
Customize keyboard screenshot; these are generic keys and key
combinations, fine to reuse as Shell++'s default set):

1. the four icon keys;
2. shift+tab · ctrl · esc;
3. ← · ↑ · ↓ · →;
4. ctrl+B, % · ctrl+B, " (tmux: split);
5. ctrl+B, O · ctrl+B, X (tmux: next pane, close pane);
6. alt · tab · ins · del;
7. home · pgUp · pgDn · end;
8. | · \ · ? · -;
9. / · : · ; · ! (the first key is drawn like a bar at screenshot size;
   a slash is meant — paths need it);
10. ~ · @ · $ · *;
11. ^ · % · = · `;
12. < · > · ( · );
13. { · } · [ · ];
14. F1 · F2 · F3 · F4;
15. F5 · F6 · F7 · F8;
16. F9 · F10 · F11 · F12;
17. ^_ · ^W · ^R · ^X^X (labels "^" mean Ctrl; the last sends Ctrl-X
    twice);
18. ^C · ^L · ^S · ^Z;
19. ^X · ^G · ^N · ^P.

Each key is a small rounded chip; a group holds two to four chips,
centred in its card.

### 9.3 Settings → Font & colours

- **Top bar:** back arrow, title centred.
- **A fixed row** under it:
  - a font size stepper: "−" button, the size in a small box, "+" button;
  - a font drop-down (font name, truncated, with an arrow) taking the
    rest of the width.
- **Below, scrolling:** a **two-column grid** of the same theme tiles as
  in §3.9 (miniature terminal, name under it, accent outline and accent
  name on the selected one). No "Default" tile here: this is the default.
- No save or check button: a tap presumably applies the theme at once
  **[not tested]** — nothing was tapped.

## 10. Corrections and additions to part 1

- **Host card subtitle (part 1, §2.2).** Part 1 says the subtitle is the
  protocol, then the user name. On the phone, a host whose own Username
  field is **empty** still showed "ssh, <word>"; in every case seen the
  word matched the host's OS tile, and hosts with a generic tile showed
  only "ssh". So the word is either the **detected OS** or a user name
  **inherited from the host's group** (the group can hold credentials,
  §2.2). Settled (D219): the owner confirmed it is the user name; Shell++
  shows the **effective** user name — the host's own, otherwise the one
  inherited from its groups — and "ssh" alone when there is none.
- **Session counter menu (part 1, §2.2).** The menu is confirmed: Connect,
  Connect via SFTP, one row per open session, Close. Whether "Close"
  closes the menu or the host's sessions **[not tested]** — it was left
  alone on purpose.
- **Connections tab (part 1, §3).** With a session open, the tab shows
  session cards with live previews instead of Recent connections and the
  other ways (§6).
- **Keyboard button (part 1, §5).** Keyboard glyph when hidden, 2×2 grid
  glyph when shown (§7.1).
- **Several sessions (part 1, §5, "to confirm").** A scrolling strip of
  chips; tap to switch; × only on the active one (§7.5).
- **Extra-keys row (part 1, §5, "to confirm").** It scrolls sideways; the
  four icon keys are now identified (§7.3, §7.4).
- **Screenshots of secret screens (part 1, §6).** The host editor could be
  screenshotted on this phone (§1.2).
- **Vaults tab back stack (part 1, §1).** Confirmed: after switching tabs
  and back, Vaults reopened on the same screen at the same scroll
  position. When "<" in a terminal leaves a session opened from search,
  the search results are shown again with the query kept.
- **Search on Hosts (part 1, §2.2).** The matching part of each title is
  drawn in the accent colour.

## 11. Additions from Claude's check of every screenshot

Claude looked at all 99 screenshots (except the two that show a
password) after this file was written. Everything above matched; these
details were added:

1. **Text selection is Android's own.** The floating bar with Copy,
   Paste, Select all (upper-case system buttons) and the teardrop handles
   are the platform's standard text-selection toolbar and handles, not a
   custom bar. Shell++ should use the platform's floating selection
   toolbar and handles as well (`ActionMode` floating type), so selection
   feels like everywhere else on the phone [decision, D219].
2. **The session bar takes the active session's look.** On a terminal
   session the bar's buttons, chip and text use the terminal theme's
   background and accent colour; on an SFTP session they use the app's
   neutral colours. With two sessions, the inactive chip appears:
   - after the active one — collapsed to just its tile (an SFTP chip shows
     only its folder tile), at the right of the active chip;
   - before the active one — its name's end peeking in at the left edge,
     faded, without its tile.
3. **The Connections session card uses the terminal theme**: the preview
   area on the terminal's background; the row under it with a dark tile
   (the OS mark in the accent colour) and the session name and protocol in
   the accent colour; a × on a small rounded square on the right.
4. **Recent connections** hold five entries, newest on top. A saved
   host's row shows its label as the title; an ad-hoc connection shows the
   address. The subtitle is always the full command with the user and the
   port (`ssh user@host -p 22`). A reconnect moves the row to the top.
5. **Search opened with an empty query** shows the whole lists (groups,
   then hosts, under their headers) with the keyboard open; typing filters
   them; the matching part of each title is highlighted.
6. **Terminal themes.** Apart from the other client's own two themes, the
   grids list well-known open-source colour schemes (Flexoki, Kanagawa,
   Everforest, Night Owl / Light Owl, Aura, Rosé Pine, Solarized,
   Dracula, Monokai, Nord and others). Shell++ may ship open schemes
   whose licences allow it (check each scheme's licence; keep the
   attribution), never the other client's own themes; Shell++'s default
   stays its own theme from `docs/design-system.md` [decision].
7. **Proxy editor** is a flat page (fields on the background, no cards),
   unlike the host editor's cards.
8. **Tag picker** rows are separate rounded tiles stacked with a hairline
   gap, each with a muted check mark on the right.
9. **Font size** in Font & colours is a small integer (6 on the phone)
   in its own unit; Shell++ uses its own size scale.

## 12. Still to come (part 3)

- Port forwarding; Snippets; Logs;
- a filled Keychain list and the key and identity editors (the keychain
  on the phone is empty);
- the selection-mode overflow (⋮) menu on Hosts;
- an editor with unsaved changes (is there a discard prompt?); the Telnet
  card turned on; the vault selector in the editors;
- the address field's round button and the password field's lock button;
- connecting: the host key prompt, the password prompt;
- the terminal: cursor mode gestures, the grid button while the keyboard
  is shown, what happens after picking a file with the attach key;
- SFTP: opening folders and files, upload, download;
- whether "Connect via SFTP" in the terminal's "+" pop-up closes the
  terminal session (§7.6);
- what "Close" does in the session counter menu;
- Customize keyboard: adding, editing and reordering groups; reset;
- the unlock screen; first run; Team.
