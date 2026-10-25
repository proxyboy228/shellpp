# Mobile reference, part 1: navigation, Vaults, Hosts, Connections, Settings, terminal bar

> Clean-room description written by Claude from the owner's screenshots
> of the SSH client he used on Android (2026-10-25). Agents never see the
> screenshots. This file describes **structure and behaviour only**: where
> things are, how screens connect, what a tap does. It deliberately leaves
> out the other product's icons, artwork, colours, logos and wording.
> Shell++ keeps its own design system (`docs/design-system.md`), its own
> icons and its own words; labels below that are generic terms of the
> trade (Hosts, Keychain, Known hosts…) match what the desktop already
> uses.
>
> Destination in the repository: `docs/mobile/reference-1.md`, beside
> `docs/mobile/owner-notes.md`. Where this file and `docs/mobile/ui-spec.md`
> disagree, this file wins for layout and navigation; `ui-spec.md` keeps
> the data rules, the terminal gestures that this file does not cover,
> and the sync behaviour.

## 1. App-wide navigation

- **A bottom navigation bar with three tabs**, always visible outside the
  terminal: **Vaults** (the vault's data), **Connections** (connecting
  without opening the vault lists), **Settings**.
- Each tab has an icon above its label. The active tab is marked by a
  filled pill shape behind its icon and a bolder label.
- The app opens on Vaults. Each tab keeps its own back stack: going to
  another tab and back returns to where the user was.
- No navigation drawer, no hamburger menu.

## 2. Vaults tab

### 2.1 Vault home

- **Header:** the current vault's name, centred, with a small icon before
  it and a drop-down arrow after it. Tapping it opens the vault switcher.
  In Shell++ 1.0 there is one personal vault; the header still shows it,
  the switcher lists just that vault (team vaults come with stage 8).
- **Below: one grouped list** (rows inside one rounded container, thin
  separators between rows), one row per section:
  1. Hosts — with the number of hosts on the right;
  2. Keychain;
  3. Port forwarding;
  4. Snippets;
  5. Known hosts — with the number of entries on the right;
  6. Logs — with the number of entries on the right.
- Each row: a section icon on the left, the section name, the count (muted
  text) where there is one, a chevron on the right. Tapping opens the
  section as a new screen.
- In Shell++ 1.0 for Android, sections that are not built yet (Port
  forwarding, Snippets, Logs) are not shown, rather than shown disabled
  [decision].

### 2.2 Hosts screen

**Top bar:**
- left: a back arrow followed by the vault's name (it returns to the vault
  home);
- centre: the screen title, Hosts;
- right: a search button, then a **sort button** that shows the current
  sort order as a small two-letter badge (A→Z shows "Az", Z→A shows "Za",
  dates show a calendar-like badge).

**Content, one scrolling column:**
1. **Groups** — a section header ("Groups") on the left. (The other client
   shows team member avatars and an invite button on the right of this
   header; Shell++ 1.0 shows nothing there — team sharing is stage 8.)
2. **Group cards** — each group is its own full-width rounded card with
   space between cards (not rows of one container):
   - a rounded square tile on the left with a group icon;
   - the group name as the title;
   - below it, muted: the number of hosts ("No hosts", "1 host",
     "N hosts").
   
   Tapping a group card opens that group (its subgroups and hosts, same
   layout, the top bar's back arrow then shows the parent's name).
3. **Hosts** — a section header ("Hosts").
4. **Host cards** — same card shape as groups:
   - a rounded square tile on the left. With OS detection on, the tile
     shows the detected operating system's mark on that OS's tile colour
     (Ubuntu, Debian, Alma and others); without it, a generic server icon
     on the default tile colour. Shell++ draws its own OS marks
     (simple, original glyphs; no distribution logos) [decision];
   - the host's label as the title (its address if it has no label);
   - below it, muted: the protocol, then the effective user name — the
     host's own, otherwise the one inherited from its groups ("ssh, root",
     "ssh, debian"); the protocol alone ("ssh") when there is none
     (confirmed by the owner; part 2, §10);
   - when the host has open sessions: the subtitle reads "Active", the
     tile gets a highlighted ring, and on the right of the card a small
     **session counter with a drop-down arrow** ("1 ▾").
- No inline buttons on cards; everything is a tap, a long press or the
  counter.

**Floating action button:** a large rounded-square button at the bottom
right, above the navigation bar, with a "+". It opens a **bottom sheet**
(with a drag handle at the top) listing, each row with an icon on the
left:
- New host;
- New local terminal (not in Shell++ for Android 1.0) [decision];
- New group;
- a separator;
- Quick import (in Shell++: import from a file or from the clipboard —
  ssh config, the native JSON export — whatever the core's import
  supports on Android; to decide in the run, may be left out of 1.0)
  [decision].

**Inside a group:** the same screen layout. The top bar changes:
- left: a plain back arrow (to the parent);
- centre: the group's icon, the group's name and a drop-down arrow
  (jumps to another group or up the tree — to confirm);
- right: search and sort, as on Hosts.

The content shows the group's subgroups (under "Groups") and hosts
(under "Hosts"); an empty section is not shown. The floating "+" creates
inside this group.

**Active session in the bottom bar:** while a terminal session is open,
the Connections tab's icon is replaced by the OS mark of the active
session's host (in the accent colour), so the user sees from any screen
that a session is running and can return to it from there.

**Tapping a host card connects at once** (opens a terminal session; no
intermediate screen; confirmed by the owner). Editing a host goes through
selection mode (long press, then the pencil). **Tapping the session
counter** opens a pop-up menu anchored to the card:
- Connect — a new session to this host;
- Connect via SFTP;
- one entry per open session of this host, by session name — switches to
  that session;
- Close — closes the menu.

**Long press → selection mode:**
- the top bar is replaced by: a close (×) button, "N selected", an edit
  (pencil) button, and an overflow (⋮) menu;
- the pressed card's tile turns into a check mark tile; tapping other
  cards adds or removes them;
- the floating button changes from "+" to **Connect** (connects to the
  selected hosts);
- × or Back leaves selection mode.

**Sort menu** (from the sort button): a pop-up with
- A–Z (current one has a check mark), Z–A;
- a separator;
- newest to oldest, oldest to newest.

Sorting applies to groups and to hosts, each within its section.

**Search** (from the search button): the top bar turns into a full-width
rounded search field with a close (×) on the left and a placeholder like
"Search hosts and groups"; the keyboard opens. Results are a flat list
under a "Hosts" header (groups that match under their own header), with
the same cards, the same tap and session-counter behaviour, and the same
floating button.

**Not copied:** the other client's in-list feedback survey banner.
Shell++ shows no promotional or survey content [decision].

## 3. Connections tab

A screen for connecting without browsing the vault.

- **Title** centred at the top (Connections).
- **Quick connect field:** a large rounded field across the width. It
  accepts either search text (matching saved hosts) or an ssh command line
  of the form `ssh user@host -p port`. A **Connect** button sits inside
  the field on its right. Submitting an ssh command opens a session
  (asking for the password or key as needed); submitting search text
  shows matching hosts.
- **Recent connections** — a card (rounded container) with a title and a
  one-line explanation, then rows of recent sessions:
  - the OS tile on the left (as on host cards);
  - the address or host label as the title;
  - below it, muted, the equivalent command, e.g.
    `ssh root@203.0.113.7 -p 22`.
  
  Tapping a row reconnects. (How many rows, and whether rows can be
  removed — to confirm.)
- **Other ways to connect** — a second card with a title and a short
  explanation, then rows, each with a small icon tile on the left:
  - a local terminal on the device;
  - connect via SFTP;
  - discover devices on the local network;
  - start a serial connection.
  
  Shell++ for Android 1.0 shows only "Connect via SFTP" here (it opens
  the same quick-connect flow straight into the file browser); local
  terminal, discovery and serial are later or not planned [decision].

## 4. Settings tab

One long scrolling page — no cards: section headers in a large bold
style, plain two-line rows (title, and the current value in muted text
underneath), a toggle on the right for on/off settings, a chevron for
settings that open a sub-page; a full-width separator between sections.

Sections in order, with what each row does. Rows Shell++ does not have
yet are marked; the agents decide per row whether to build it now or
leave it out of 1.0, and record that.

1. **Account**
   - Account settings → sign-in, security, sync (in Shell++: the account
     and sync screen of `ui-spec.md`), with the account's avatar or
     initials on the left.
   - (Team — stage 8; community link — not copied.)
2. **Terminal**
   - Font and colours → the terminal theme, font size, font; the value
     line shows all three, e.g. "Theme / 6 / Font name".
   - Terminal type (xterm-256color).
   - Hotkeys (how Ctrl is entered).
   - Customize keyboard → edits the extra-keys row of the terminal.
   - Scrollback size (lines).
   - Bell (vibration, sound).
   - Autocomplete — toggle.
   - Pinch to zoom — toggle.
   - Cursor speed (for moving the cursor by gesture).
   - Keep the screen on while in a terminal — toggle.
   - Alternative keyboard input (voice input, CJK layouts) — toggle.
3. **Session**
   - Keep-alive interval (seconds); keep-alive max count.
   - Post-quantum key exchange — toggle, with the note to turn it off for
     old devices.
   - Detect OS — toggle (drives the OS tiles of §2.2).
   - Import shell history — toggle, needed for autocomplete and history.
4. **Notifications**
   - Notifications → what the notification of open sessions shows.
5. **Security & Privacy**
   - Biometric unlock — toggle (Shell++: D218).
   - App lock code (PIN or pattern) — Shell++ uses the master password
     and the fingerprint instead; not built [decision].
   - Logs storage →.
   - (Diagnostic data — not copied: Shell++ sends none.)
   - Screenshots and screen recording — allowed or blocked on screens
     with secrets.
6. **Volume keys** — volume up, volume down: each can be bound to an
   action (off by default).
7. **Shake** — bind an action to shaking the phone (off by default).
8. **About** — version; the project's page; help.

## 5. Terminal screen: the session bar and the extra-keys row

- **Full screen terminal.** No top app bar; only the system status bar
  above. Text fills the screen from the top.
- **At the bottom, two bars** sit directly above the system keyboard when
  it is open, and at the bottom of the screen when it is closed:
  1. **Session bar** (one row of rounded controls):
     - a **back** button (rounded square) on the left — back to the lists;
       the session stays open;
     - a wide **session chip** in the middle: the host's OS mark, the
       session name, and a × that closes the session;
     - a **+** button (rounded square): a pop-up with connect again to
       this host, connect via SFTP (and via serial, not in Shell++);
     - a **keyboard** button (rounded square) on the right: shows or
       hides the system keyboard. Its icon changes with the state
       (keyboard icon when hidden, a different icon when shown).
     - With several sessions open, the chip switches between them — to
       confirm with the next screenshots.
  2. **Extra-keys row** below it: a row of keys without borders, in the
     terminal's accent colour. Seen, left to right: four icon keys (their
     functions — to confirm with the owner: an assistant or autocomplete
     key, a credentials key, a cursor-pad key that moves the cursor,
     an attachment or snippets key), then text keys "shift+tab", "ctrl",
     "esc", and an arrow key. The row is configurable (Settings →
     Customize keyboard); whether it scrolls sideways — to confirm.
- The system keyboard is the phone's own; the app adds nothing inside it.

## 6. Still to come (part 2, from the next screenshots)

- the host editor and the group editor (the other client blocks
  screenshots of screens with secrets by default);
- Keychain; Known hosts; SFTP; Snippets; Port forwarding; Logs;
- the terminal: what each icon key does, several sessions, selecting
  text, placing the cursor;
- connecting: the host key prompt, the password prompt;
- the unlock screen; first run;
- the vault switcher; Customize keyboard; Font and colours.
