# Mobile UI specification (Android, first version)

Status: run 13, the design run (D214); the layout and navigation rewritten
in run 15 after the owner's reference descriptions (D220). Sources, and only
these: the owner's words (`docs/mobile/owner-notes.md`, D211), the clean-room
descriptions of the structure of the app the owner used before
(`docs/mobile/reference-1.md`, `reference-2.md`: where things are, how screens
connect, what a tap does; no icons, colours or wording of that app), the
architecture (`docs/mobile/architecture.md`, D213) and the accepted desktop
specifications (`docs/ui-behavior-spec.md`, `docs/design-system.md`,
`docs/data-model.md`, `docs/sync-protocol.md`). Everything else is marked
[decision]. Where the reference descriptions and this file disagree on
layout and navigation, the descriptions and D220 win; the data and sync
rules here stay.

The data, the records and their rules are the desktop's: the same hosts,
groups, keys, known hosts and sync, no new format. Where this spec says
"as on the desktop", the desktop section it names is the behaviour.

## 1. Principles

- **One person, now and then, away from home** (D211): the app opens fast on
  the master password, finds a host by search, connects, and the terminal
  is the main screen. Everything else serves that.
- **The terminal does what the owner missed** (D211): putting the cursor on
  the wanted character and selecting the wanted text are first-class
  gestures, not afterthoughts (section 9).
- **Portrait and landscape**, the soft keyboard and a hardware keyboard,
  one hand where it can be [decision].
- **No account features without the owner's server**: local use works
  without sync; sync is the owner's own server, as on the desktop.

## 2. Look: tokens on a phone

The desktop's design system (`docs/design-system.md`) with these
adaptations [decision]:

- **Colors**: the same tokens and both themes (dark default, light; "Auto"
  follows the system). Surfaces map as: `--bg-base` the screen background
  and top app bar, `--bg-surface` lists, `--bg-raised` cards and sheets,
  `--bg-overlay` menus, dialogs and the selection toolbar, `--scrim` under
  sheets and dialogs. Accent, semantic, border and text tokens are used as
  on the desktop; the contrast pairs (design-system 7.3) hold on the phone
  too.
- **Units**: the desktop's px values become dp, and text sizes sp (the
  system's font size setting scales them). Interface text: body 16 sp
  (the desktop's 14 px is too small to read at arm's length), dense lists
  14 sp, captions 12 sp, section headings 18 sp, dialog titles 22 sp. The
  interface font is the system's (Roboto, or the phone maker's).
- **Terminal**: the terminal themes of design-system 4 (Graphite dark by
  default, Paper light) and JetBrains Mono (OFL) at 11 sp by default (about
  60 columns on the owner's S25, D223; a size chosen on the device stays).
- **Touch**: every control has a touch target of at least 48 × 48 dp (the
  extra keys: 48 dp high, at least 44 dp wide, section 9.2);
  spacing follows design-system 6 in dp; rows of lists are 56 dp (one line)
  or 72 dp (two lines).
- **Shapes**: the desktop's radii in dp; bottom sheets have 16 dp top
  corners.

## 3. Navigation

- A bottom bar with three tabs (reference-1 1): **Vaults** (the vault's
  data), **Connections** (connecting without browsing the vault),
  **Settings**. The active tab has a filled pill behind its icon and a bolder
  label. Each tab keeps its own back stack: another tab and back returns to
  the same page and scroll position. A second tap on the current tab goes to
  its first page; Back at the first page of Connections or Settings goes to
  Vaults [decision].
- While a session is open, the Connections tab shows the OS mark of the
  last session's host instead of its terminal glyph.
- Lists show the bar; editors, pickers, sub-pages of Settings and the
  session screen cover it.

### 3.1 Vault home

- The vault's name with its icon and a drop-down arrow at the top: the
  vault switcher, a bottom sheet with the one personal vault and its check
  mark (team vaults with stage 8).
- One grouped list with the sections: **Hosts** (count), **Keychain**,
  **Known hosts** (count), **Recently deleted** (count; Shell++'s own,
  data-model 16.6) [decision]. Port forwarding, Snippets and Logs are not
  shown until they are built [decision].

### 3.2 Connections

- **Quick connect**: a rounded field with **Connect** inside it. `ssh <word>`
  or `<word>` alone that is the label or the address of a saved host (the
  exact spelling first, then without case, as the desktop's UI 2.1)
  connects to that host with its settings; several such hosts are listed to
  pick one (D223). Other text that reads as an ssh command (`ssh user@host
  -p 22`, `ssh -p 22 user@host`, `user@host[:port]`, `ssh host`) connects
  without saving anything; other text searches the saved hosts and lists
  them (a tap connects).
- **While sessions are open**: one card per session, its live preview (the
  last lines of the output in the terminal's theme; an SFTP session shows
  the folder it shows and its last transfer, or "No transfers", D223) above a row with the host's tile, the session name, the
  protocol and **×** (closes at once). A tap on the preview opens the
  session.
- **Otherwise**: **Recent connections** (the connection log, data-model 13.3:
  the five newest, newest first, a saved host by its label, an ad-hoc one by
  its address, the full command under it; a tap connects again) and **Other
  ways to connect**: Connect via SFTP (a saved host). The local terminal,
  serial ports and device discovery are not on the phone.

## 4. First run

As the desktop's 13.1, on a phone:

- The app name and the line that the data is stored locally and encrypted.
- Two ways in, as two full-width buttons [decision]:
  - **Create a new vault**: the "Master password" and "Repeat password"
    fields (masked, show/hide), the strength indicator (advice only), the
    warning that a forgotten master password cannot be recovered, and
    "Create vault" (accent; enabled at 8 characters and matching fields).
    The desktop's "Remember on this device" is not offered here: the owner
    unlocks with the password (D211); fingerprint unlock is an option in
    Settings, off by default.
  - **Restore from sync**: the connect wizard of the desktop's 13.5 in its
    sign-in form, one step per screen (section 11.1).
- While the key is derived (about a second, longer on a phone), the fields
  are read-only and the button shows progress.

## 5. Unlock

As the desktop's 13.2:

- The app name, the "Master password" field with the keyboard up, "Unlock"
  (accent). The IME's Enter unlocks.
- With fingerprint unlock on (section 12), **at a cold start and after a
  lock** (auto-lock, Lock): the system's biometric prompt opens first; "Use
  password" in it, a cancel or a failure leaves the password field (D218,
  architecture 4). The password is asked instead when the fingerprint key
  became invalid (a fingerprint was added: the option goes off and Settings
  says why), when the system locked biometrics out, once every 7 days, and
  once after the vault's key changed while the prompt was not passed. When
  no fingerprint is enrolled any more, the option is off and Settings says
  so.
- A wrong password clears the field and says "Wrong password"; the pauses
  after 5 wrong passwords in a row are the desktop's and the core's.
- "Forgot password?" offers the desktop's choices: Restore from sync (with
  the recovery key), Start with a new vault (with the same confirmation;
  the data file is renamed and kept), Cancel.
- **Auto-lock**: the desktop's choices (never, 5, 15, 30, 60 minutes); on
  the phone the default is 15 minutes, counted from the last input in the
  app: a touch, a key of the soft keyboard or of a hardware keyboard; the
  time in the background counts too [decision]. Live terminals keep running
  while locked (as on the desktop); opening one asks for the password first.

## 6. Hosts and groups

### 6.1 Hosts screen

- **Top bar**: back with the vault's name, the title, search and the sort
  button (its badge "Az", "Za" or a calendar with an arrow). Inside a group:
  a plain back arrow, the group's icon, name and a drop-down up the tree
  (All hosts, its ancestors).
- **Groups** then **Hosts**, each a section header over full-width cards.
  A group card: a folder tile, the name, "No hosts" / "1 host" / "N hosts"
  (the whole subtree). A host card: the OS tile, the label (else the
  address), under it the protocol and the **effective user name** (the
  host's own, its identity's, else the groups' username or identity;
  "ssh" alone without one) (D220). With open sessions the subtitle reads
  "Active", the tile has a ring and a **session counter** ("1 ▾") sits on the
  right.
- **OS tiles**: Shell++'s own simple glyphs: a detected Linux distribution
  or BSD shows its name's first letters on a tile of one of the design
  system's tag colours darkened by a quarter, in white letters; undetected hosts a
  generic server glyph [decision]. No distribution logos.
- **A tap on a host connects** (the session screen, section 9); a tap on a
  group opens it. **A long press selects**: the bar becomes ×, "N selected",
  the pencil (one record: its editor) and ⋮ (Connect via SFTP for one host,
  Duplicate, Move to group, Delete); a tap adds or removes; the floating
  button becomes **Connect** (the selected hosts, one session each). Back or
  × leaves selection.
- **The session counter's menu**: Connect, Connect via SFTP, the host's open
  sessions (switch to one), **Close**, which closes only the menu (D220).
- **Sort**: A–Z, Z–A, newest to oldest, oldest to newest, for groups and
  hosts each in their section; kept on this device.
- **Search**: the bar becomes a field with × on its left, the keyboard up;
  an empty query lists every group and host of the vault, typing filters
  them, the matching part of each title in the accent.
- **"+"**: a bottom sheet with New host and New group (inside the group
  shown). Import is not on the phone (the core has no import there).
- **Delete** asks first, names what goes, and moves it to Recently deleted
  (section 6.3; D93); the snackbar after it has "Undo".
- Empty state: "No hosts yet" with New host and, when sync is not set up,
  "Restore from sync" (D219).
- Pull to refresh runs a sync when an account is connected [decision];
  offline it says "No network" and changes nothing.

### 6.2 Host and group editors

reference-2 1–3 for the order, with only the fields of the desktop's data
model (data-model 7; D220):

- **Top bar**: back, "New host" / "Edit host", a check mark that saves and
  closes. Leaving without changes closes at once; with changes it asks
  "Discard changes?" [decision].
- **A save writes only what the editor changed** (`record_save` with the
  version it opened from, data-model 2.1, D221): a change that sync brings in
  while the editor is open keeps every field the user did not touch, and a
  value the edit overwrites is kept as a conflict copy.
- **Host**, top to bottom: the vault (one personal vault); a card with
  Label, Address, Group (a picker; × clears it), Tags (typed, or a picker of
  the vault's tags), "Backspace sends Ctrl+H"; a card with Port; Credentials
  with Username (a round button picks an identity), Password (masked; the
  round button shows it; "Remove the saved password"), Key (a picker of the
  Keychain's keys, with New SSH key…); a card with Agent forwarding, Host
  chaining (the chain editor: jump hosts top to bottom, then this host;
  Clear connects directly, `[]`; "Use the group's chain" inherits), Proxy
  (type, address, port, username, password), environment variables (a
  bottom sheet), Colour scheme (a grid of the built-in themes, Default
  first; a theme imported on the desktop is kept), Charset (Current, Use
  default, the list); a card with Telnet (its port); last, Notes (several
  lines, D223).
- **Group**: the vault; Name and Parent group; Port; Username (with an
  identity), Key; Host chaining; last, Notes (several lines, D223).
- Fields the host or group leaves empty show what it inherits under them
  ("deploy, from group Servers"), from the group chosen in the form.
- **Not shown** (kept as they are; the desktop edits them; D223 item 4):
  color label, certificate and FIDO2 key slots, the startup snippet, the system
  agent and confirmation of forwarding, legacy algorithms, keepalive per
  host, Mosh, recording settings, the primary protocol [decision].

### 6.3 Recently deleted

A row of the vault home (3.1), as the desktop's section 14 for this device: the list (name,
type, when deleted), search, **Restore** [decision: restore only; deleting
forever stays on the desktop]. Empty state: "Nothing deleted in the last 90
days".

## 7. Keychain

reference-2 4, with the Keychain of the desktop's section 7 behind it:

- Top bar like Hosts (back to the vault, search, sort). **Keys** then
  **Identities** as cards: a key's label, type and shortened fingerprint; an
  identity's label, username and whether a password is saved. Empty state:
  an icon, "No keys yet" and what keys are for.
- **"+"**: a bottom sheet with New identity; Generate a new SSH key, Import
  a key from a file (the system file picker; OpenSSH and PuTTY formats; a
  passphrase prompt when the file has one). Biometric and FIDO2 keys come
  after the first version.
- A key's screen: label (editable), type, fingerprint (copy), the public
  key with Copy and Share, the hosts using it, Delete (asks first, lists
  the hosts that use it, Undo), as the desktop's 7.3. The private key is
  never shown or shared.
- An identity opens a dialog: label, username, password (replace or
  remove).

## 8. Known hosts

reference-2 5, with the desktop's section 10 behind it:

- Top bar like Keychain; **no "+"**: entries come only from connecting.
- One card per entry: the same shield tile for each, the address (`[host]:port`
  when the port is not 22; "hashed host" for a hashed one), under it the
  labels of the saved hosts that use it, comma-separated (none: no
  subtitle).
- Sorted with numbers compared as numbers, an IPv4 address by its octets
  (D220); the four orders of the sort menu.
- A tap shows the key type, the full SHA-256 fingerprint (copyable), when it
  was first seen and **Remove**. A long press selects; ⋮ → Remove asks
  first and offers Undo.
- **First connection and a changed key** are the desktop's 10.2 dialogs that
  a tap outside does not dismiss: the host, the key type and the full
  fingerprint; for a first key "Accept and save", "Connect once", "Cancel";
  for a changed key a danger-styled warning with the old and new
  fingerprints, "Cancel" first, and "Replace the key" behind a second
  confirmation.
- A **revoked** key blocks the connection with no way to continue, as on the
  desktop (10.2).

## 9. Terminal

The main screen of the app. The behaviour of the session (connecting,
prompts, the end and the reconnect policy, the lock) is the desktop's
4.6 and 4.8; this section is the phone's view of it.

### 9.1 The session screen

- **Full screen**: the output from the top (no app bar); at the bottom,
  right above the keyboard when it is up, the **session bar** and, on a
  terminal session, the **extra-keys row**. Opening a session shows it with
  the keyboard up.
- **The session bar** (reference-1 5, reference-2 7.5–7.6): "<" back to the
  lists (every session stays open); the strip of **session chips**, every
  open session, terminal and SFTP, scrolling so the active one is in view:
  the active chip with the host's tile, the session's name (and its state
  while not connected) and **×**, which closes the session at once (D220);
  the others faded, without ×, a tap switches to them; an SFTP session's
  tile is the OS mark inside a folder; **"+"** with Connect (a new session
  to the same saved host) and Connect via SFTP (a new SFTP session; the
  terminal session stays, D220); the **keyboard button** on terminal
  sessions only, a keyboard glyph while the keyboard is down and a grid
  glyph while it is up. The "+" menu opens above the bar and never over a
  ×; the × of the chip is set apart from "+" (D220).
- The bar takes the terminal theme's background and accent on a terminal
  session and the app's colours on an SFTP session.
- Back: hides the keyboard when it is up (the system's), then clears a text
  selection, then goes back to the lists (D220).
- Prompts of the connection (password, keyboard-interactive, a key's
  passphrase, the host key decisions of section 8) are dialogs over the
  session; a prompt of another session brings it to the front first.
- The app draws edge to edge (Android 15+): the bars sit above the
  keyboard's and the navigation bar's insets.

### 9.2 Extra keys

- **One row that scrolls sideways**, the groups of Settings → Customize
  keyboard one after another, keys without borders in the accent, 48 dp
  high (reference-2 7.3–7.4, 9.2). The default groups: password, cursor,
  attach; shift+tab, ctrl, esc; the four arrows; two groups of tmux prefix
  combinations; alt, tab, ins, del; home, pgUp, pgDn, end; six groups of
  signs and brackets; F1–F12 in three groups; three groups of Ctrl
  combinations (^_ ^W ^R ^X^X, ^C ^L ^S ^Z, ^X ^G ^N ^P). No AI key in the
  first version (D220).
- **The password key** (D220, D223): a tap asks "Send the saved password?"
  naming the host; only **Send** types the host's effective password into
  the session (its own, else its identity's, also one its group names,
  data-model 8.3), once and without Enter, straight from the vault (it
  never passes through the interface's memory; the buffers on its way are
  wiped once it is written, security.md 9.5). It goes in as injected
  input, which the history capture never takes for a command. It cannot
  send when the session is not connected to its host, when the session
  records its output (the session's own answer for its current connection;
  until it has decided, a moment after connecting, it counts as recording:
  an echoing prompt would put the password in the recording), when the host
  has no password, and for a quick connect: the key then looks disabled, a
  tap says why and TalkBack reads the reason. Send checks again in Rust and
  also refuses a password with a control character (a line break would be
  an Enter). The confirmation belongs to the session it was asked for:
  switching sessions meanwhile sends nothing to the other one.
- **The cursor key**: a toggle (filled while on); while on, a drag on the
  terminal sends the arrow keys the way the finger moves, at Settings'
  cursor speed; a tap still puts the cursor (9.3).
- **The attach key** (D220): the system file picker, then the folder to
  upload into (the user's home folder on this server, editable); an SFTP
  session to the host opens beside the terminal for it and its chip shows
  the transfers. The shell's current folder is not guessed. Disabled for a
  quick connect.
- **Ctrl and Alt are sticky**: a tap arms the modifier for the next key
  (from the row or the soft keyboard), a double tap locks it (drawn filled)
  until it is tapped again. Ctrl with a letter sends the control character;
  Alt sends Esc before the key.
- The arrows send the cursor keys in the mode the program asked for;
  Home/End/PgUp/PgDn, Ins and F1–F12 their escape sequences, as on the
  desktop. Held arrows, Delete and the page keys repeat (after 400 ms, every
  60 ms).

### 9.3 Putting the cursor on a character (the owner's first pain)

**The gestures on the terminal** [decision]:

| Gesture | On the shell's screen | In a full-screen program |
|---|---|---|
| Tap | on the typed line: puts the cursor there; elsewhere: nothing | puts the cursor there (a click when the program takes the mouse) |
| Tap while the keyboard is down | as above, and the keyboard comes up | as above, and the keyboard comes up |
| Drag the cursor handle | moves a mark cell by cell with the magnifier; release: puts the cursor there | the same |
| Long press | selects the word (9.4) | selects the word (9.4) |
| Vertical swipe | scrolls the scrollback (9.7) | wheel events or ↑/↓ (9.7) |
| Pinch | font size (9.8) | font size (9.8) |

- **Where the cursor goes**:
  1. when the program asked for mouse reports (nano with `set mouse`,
     `htop`, `mc`, vim with `mouse=a`; the terminal tracks the mouse modes
     as the desktop does, `docs/guide/04-terminal.md`), a tap is a left
     click at that cell, and the program puts its cursor there itself; a tap
     with Ctrl or Alt armed sends the click with that modifier;
  2. on the shell's command line (no alternate screen; the tap on the line
     being typed: the cursor's row or a row it wraps onto), the terminal
     walks the cursor there with ← and →; it works with every shell and line
     editor that understands the arrow keys (bash, zsh, fish, readline
     programs, the Python prompt);
  3. in a full-screen program without mouse reports (the alternate screen:
     nano as it comes, vim), the terminal walks the cursor with ↑ and ↓ to
     the tapped row, then with ← and → to the tapped column.
- **The cursor handle, for precision**: a fingertip covers several cells,
  so after a tap that moved the cursor (or on the cursor's own cell) a handle
  hangs under the cursor for 4 seconds, as Android's text fields show one.
  Dragging it moves a mark from cell to cell with the magnifier (9.5) above
  the finger; releasing it puts the cursor on the marked cell, the same way
  as a tap [decision].
- **Walks use feedback** (architecture 6.3): the first step is one arrow;
  the terminal waits for the program to move its cursor and reads where it
  went; if it moved, the next steps are a few arrows at a time, each read
  again, until the cursor is on the tapped character or an arrow no longer
  moves it. Tabs, wide characters and an editor's own line wrapping land
  right that way.
- **Limits**: a walk sends at most as many arrows as there are cells
  between the cursor and the tap, and stops after a step that did not move
  the cursor. In a program where arrows do not move a cursor (`less`, which
  scrolls one line with an arrow), a tap sends one arrow and stops. While a
  walk runs, typing waits until it ends; each step is a round trip to the
  server, so a long walk on a slow link takes a moment and the cursor
  visibly travels.
- The cursor moves on the screen when the program echoes it; the terminal
  draws no cursor of its own anywhere else.

### 9.4 Selecting text (the owner's second pain)

[decision for the whole section]

- **Long press** on text selects the word under the finger (the desktop's
  double-click rule, `crates/terminal/src/selection.rs`: a word ends at a
  space or one of ``,│`|:"'()[]{}<>``, so a path is one word) and shows:
  - two handles at the selection's ends; dragging a handle extends or
    shrinks the selection cell by cell, across rows and into the
    scrollback (the view scrolls when the handle nears the top or the
    bottom edge); a magnifier shows the cells under the handle;
  - the platform's own floating toolbar above the selection (`ActionMode`,
    floating; reference-2 11.1): **Copy, Paste, Select all** (everything
    visible), and **Delete** or **Cut in nano** where 9.6 allows them.
- **Back** with a selection clears it first (D220).
- A tap outside the selection ends it and does nothing else. New output
  keeps the selection on the same text while it stays in the buffer, as on
  the desktop.
- **Copy** puts the selected text on the clipboard (lines joined as on the
  desktop: wrapped rows without a line break, trailing blanks dropped); below
  Android 13 a short "Copied" toast says so (from Android 13 the system
  shows its own notice). **Paste** follows the desktop's paste rules
  (bracketed paste; the multi-line confirmation of UI 4.3 with its first
  lines while `terminal.confirm_multiline_paste` is on, Settings 12). So
  does a keyboard's clipboard: text it commits with a line break is a
  paste, not typing (D223) [decision].
- When the program asked for mouse reports, a long press still selects (the
  terminal's own selection), as Shift with the mouse does on the desktop
  (`docs/guide/04-terminal.md`).

### 9.5 Magnifier

- Shown while the cursor handle or a selection handle moves: a rounded lens
  1.5× the text above the finger, from the system (`Magnifier`, Android 9+;
  below Android 9 there is no lens) [decision].

### 9.6 Deleting a selected piece in an editor (the owner's example)

The owner's example is a piece of a configuration file in nano. A terminal
shows the server's screen; it cannot delete text on the server. Only the
program can, with its own keys. So the app offers what really works and
says where it stops [decision]:

- **On the shell's command line** (no alternate screen, the selection on
  the line being typed): **Delete** walks the cursor to just past the
  selection's last character (9.3) and sends one Backspace per selected
  character. The selection may not leave that line.
- **In nano**: when the screen shows nano's shortcut rows (the last two rows
  hold `^G` and `^X`), the toolbar offers **Cut in nano**. It puts the
  cursor on the selection's first character (a click when nano takes the
  mouse, a walk otherwise), sets nano's mark (Alt+A), puts the cursor just
  past the selection's last character and cuts (Ctrl+K). The cut text is in
  nano's cut buffer, as with nano's own keys, and Ctrl+U pastes it back.
- **Nothing is deleted unless every walk landed exactly** on its target:
  when a walk stops short, Delete sends no Backspace and Cut in nano sends
  no Ctrl+K (it unsets the mark with Alt+A if it set it), and a message
  says "The cursor could not reach the selection; nothing was deleted."
- Other editors use their own keys from the extra keys and the tap (vim:
  tap, `v`, tap, `d`).
- **The limits**, said in the toolbar's help line: the app does not know
  which program runs; it offers Cut in nano where nano's shortcut rows are
  on the screen, and nano's keys do other things elsewhere. A selection that
  runs past what the editor shows (into the scrollback) cannot be cut this
  way: the toolbar offers Copy only then.

### 9.7 Scrolling

- A vertical swipe scrolls the scrollback (the desktop's 10 000 lines and
  journal, D85); a fling continues. When the program asked for mouse
  reports, the swipe sends wheel events instead (as the desktop's wheel
  does), and on the alternate screen without them it sends ↑/↓ (xterm's
  alternate scroll mode) [decision].
- New output while scrolled back does not move the view; a "↓ New output"
  chip returns to the bottom [decision].

### 9.8 Font size, rotation and the terminal's size

- **Pinch** changes the font size from 8 to 28 sp in 1 sp steps
  [decision] for the session shown; the size kept per device, not synced,
  is the one of Settings → Font and colours (11 sp by default, D223).
- The terminal's columns and rows follow the cell size and the space it
  has; rotation, the keyboard coming up or down, and the font size change
  them. The new size goes to the server (a window change on the SSH channel,
  as on the desktop) once: when a pinch ends, and 150 ms after the last
  change of a rotation or the keyboard's animation [decision].

### 9.8a TalkBack

- The terminal's screen is read row by row: each row is a node with its
  text, and the cursor's row says that it is the cursor's line; when the
  output has been quiet for 0.7 s, the cursor's row is announced once (a
  row already announced is not repeated) (run 16, D224) [decision].
- Every key of the extra-keys row is a button with a spoken name (words,
  not the glyph: "Up arrow", "Control C", "tmux: next pane"), its state
  (Ctrl and Alt: on, locked) and a double tap that sends it to the session
  in front; the password key's name says why it cannot send when it
  cannot.

### 9.9 In the background and without a network

- **The app in the background**: open sessions keep running in a foreground
  service with a notification ("2 SSH sessions open", "Disconnect all"); the
  first session asks for the notification permission (Android 13+). When it
  is denied, the service still runs and Connections lists the sessions with
  their ×. The last session closed stops the service.
- **The network lost or changed**: the session follows the desktop's 4.6:
  a dropped connection shows "Connection lost" in the terminal and the
  reconnect countdown (1, 2, 4, 8, 16, 32 s, then every 60 s, with
  Autoreconnect on); "Reconnect now", "Cancel" (stops the automatic
  attempts) and "Close" in the bar. When the phone gets a network back, a
  waiting attempt starts at once.
- **Locked** (auto-lock, section 5): sessions keep running; reconnects and
  new connections wait for unlock, as on the desktop (4.6, 13.3); the
  terminal shows the unlock screen first.
- **Swiped away from recents** while sessions live: the service keeps them;
  tapping the notification opens the app.
- **The process killed by the system** (no service, memory): the sessions
  end; the next start asks for the master password.
- **Snapshots of the screen** are taken only while the session screen is
  seen; the output keeps flowing into the model meanwhile.

## 10. SFTP

reference-2 8, with the desktop's section 5 for one remote panel behind it:

- A session of the session screen (opened from the session counter, the
  selection's ⋮, the terminal's "+", Connections or the attach key), with
  the same session bar and no extra-keys row or keyboard.
- **Top bar**: the path as a breadcrumb (parents muted, the folder shown
  emphasised; a tap on a segment goes there), **Upload** (the system file
  picker, one or more files), ⋮ and search (filters the folder shown).
- **⋮**: New folder; Sort by name, date, size or kind (the active one shows
  its direction; a second tap reverses it; folders first); the download
  folder on the phone (a folder picked once; downloads then go there without
  asking; "Ask where to save each time" goes back) [decision]; Copy path;
  Show or hide hidden files (the setting `sftp.show_hidden`); Transfers.
- **The listing** on one large rounded panel: ".." first, then a row per
  entry: a folder, file or link icon, the name, under it the permissions in
  monospace (`drwxr-xr-x`), on the right the size and the date and time,
  in monospace; no dividers.
- **Rename** never replaces another entry; a change of case only
  (`readme` → `README`) on a server without case, where the server finds
  the new name but the folder lists no entry of it, is the same file and
  is renamed (through a free name of its own when the server refuses;
  D223) [decision].
- Tap a folder: open it. Tap a file: a sheet with Download, Rename, Delete
  (asks first), Copy path. **A long press selects**: ×, "N selected", ⋮ with
  Download (one file), Rename and Copy path (one entry), Delete (asks first;
  several at once). Copying on the server and editing a file come after the
  first version [decision].
- **Download** without a download folder asks with the system's "Save to…"
  every time; the file goes through the app's cache to the chosen document
  (architecture 6.4); progress in the transfers sheet and in the
  notification; a name conflict asks as the desktop's 5.7.
- The transfer queue of the desktop (5.6) as a bottom sheet: the current
  transfer, waiting ones, cancel; a failed transfer says why and offers
  Retry.
- Errors of a listing or an action show their reason under the bar; names
  that are not UTF-8 on the server are the desktop's known limitation.

## 11. Account and sync

### 11.1 Signing in and registering

The desktop's connect wizard (13.5), one step per screen:

1. The server address (`https://`; plain `http` only for a loopback
   address) and the certificate check: a certificate the system does not
   trust offers the same choices (pin it, or add its CA for this server).
2. The email and the account password; registering also asks for the
   invite code when the server wants one.
3. TOTP: on sign-in, the one-time code when the account has TOTP; on
   registering, the offer to enable it (QR code and the secret as text),
   which can be skipped.
4. Registering shows the recovery key with "I saved it" (as 13.5 step 4).
5. The device name (default: the phone's model name) [decision on the
   default].

- What the desktop shows on the way shows here too: the "Weaker key
  derivation" dialog (D189), the message that the settings differ (D200),
  the refusal of key derivation settings above what a phone may run
  (`docs/sync-protocol.md` 4.2), and the server's error texts (UI 12.2).

### 11.2 Connected

- The server, the account email, this device's name, the last sync time and
  result, **Sync now**, the list of the account's devices (as on the
  desktop), **Sign out** as the desktop's 13.4 does it (it asks first, with
  its choice to also delete the local copy).
- The dialogs sync can raise: a changed pinned certificate (12.2);
  conflicts as the desktop's 13.6 (the other version is kept and the record
  shows a badge; resolving them is the desktop's in the first version)
  [decision]; duplicates (13.7, a notice only) [decision].
- **Sync that stopped** (the server rolled back, a forged key epoch, the
  account deleted, the storage limit): the row says what happened in the
  desktop's words and that it is handled on the desktop [decision].
- Changing the password, the recovery key and "Lost or stolen" are the
  desktop's in the first version [decision]; the phone shows where to do
  them.

## 12. Settings

One long page (reference-1 4; D220): bold section headers, two-line rows
(the title, the current value under it), a switch for on/off, a chevron for
a sub-page. Only rows whose setting works on the phone:

- **Account**: Account and sync (section 11), with the account's email or
  "Local only".
- **Terminal**: Font and colours → (the font size, −/+; the font, JetBrains
  Mono; a grid of the built-in themes, "Graphite / Paper (system)" first;
  new terminals take it, a host's own theme wins; a device setting
  [decision]); Terminal type (`terminal.term`); Customize keyboard → (the
  row's groups as cards: delete, drag by the handle, "+" adds a group of up
  to four keys, the circular arrow puts back the default set; at once in the
  row; a device setting [decision]); Pinch to zoom; Cursor speed (slow,
  normal, fast); Keep the screen on while a terminal is shown; **Bell**
  (Off, Vibrate, Sound in turn; Off by default as on the desktop: the sound
  is `terminal.bell_sound`, the vibration a device setting; only the
  session on the screen rings, at most once a second, and the sound follows
  the ringer mode; D223); **Confirm a paste of several lines**
  (`terminal.confirm_multiline_paste`, on by default; 9.4).
- **Session**: Keep-alive interval (`terminal.keepalive_s`); Detect OS
  (`terminal.os_detection_enabled`, the tiles of 6.1); Reconnect
  (`terminal.autoreconnect`).
- **Security & Privacy**: Lock and fingerprint → (Auto-lock, section 5;
  Fingerprint unlock, off by default; turning it on asks for the master
  password once and then the biometric prompt; architecture 4; D211, D218);
  **Lock now**. A copied password is cleared from the clipboard after 30
  seconds, as on the desktop (`docs/security.md` 9.4).
- **About**: the version and the commit the APK is built from; the
  project's page (`https://github.com/proxyboy228/shellpp`); **Open-source
  licenses**, the list of the components the APK ships with their licenses,
  the Android part of `THIRD_PARTY_NOTICES.md` (D223).
- Not on the phone yet: hotkeys, autocomplete, alternative keyboard
  input, post-quantum key exchange, importing the shell's history, what the
  notification shows, an app lock code (the master password and the
  fingerprint stand for it), logs, the screenshot setting (screens with
  secrets always block screenshots), volume keys and shake (after the first
  version, D220) [decision].

## 13. Not in the first version

Snippets, command history and autocomplete (the owner uses them: the next
versions, D211), port forwarding, Mosh, the AI command key (with the
desktop's command generation, D144, D220), workspaces and splits,
broadcast, the local terminal and serial ports, device discovery,
certificates and FIDO2 or biometric keys, import from other apps,
recordings and logs, export, team vaults (stage 8), volume keys and shake.
The phone keeps every record it does not edit as it is.
