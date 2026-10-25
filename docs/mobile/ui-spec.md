# Mobile UI specification (Android, first version)

Status: run 13, the design run (D214). Sources, and only these: the owner's
words (`docs/mobile/owner-notes.md`, D211), the architecture
(`docs/mobile/architecture.md`, D213) and the accepted desktop
specifications (`docs/ui-behavior-spec.md`, `docs/design-system.md`,
`docs/data-model.md`, `docs/sync-protocol.md`). Everything else is marked
[decision]. Nothing comes from another mobile client (clean room).

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
  default, Paper light) and JetBrains Mono (OFL) at 14 sp by default.
- **Touch**: every control has a touch target of at least 48 × 48 dp (the
  extra keys: 48 dp high, at least 44 dp wide, section 9.2);
  spacing follows design-system 6 in dp; rows of lists are 56 dp (one line)
  or 72 dp (two lines).
- **Shapes**: the desktop's radii in dp; bottom sheets have 16 dp top
  corners.

## 3. Navigation

- A bottom navigation bar with four places [decision]:
  1. **Hosts** (section 6), the start screen after unlock;
  2. **Sessions**: the open terminal and SFTP sessions as a list (title,
     host, state as on the desktop's tabs: connecting, connected, dropped
     with the reconnect countdown, ended); tapping one opens it;
  3. **Keychain** (section 7);
  4. **More**: Known hosts (section 8), Recently deleted (section 6.3),
     Account and sync (section 11), Settings (section 12), Lock.
- The terminal (section 9) and SFTP (section 10) are full screens over the
  bar; back returns to where they were opened from and leaves the session
  running.
- The system back gesture closes sheets and dialogs first, then the screen.

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
- With fingerprint unlock on (section 12), **at a cold start** (the app's
  process starting): the system's biometric prompt opens first; "Use
  password" in it, a cancel or a failure leaves the password field. After a
  lock (auto-lock, Lock) the password is asked, as on the desktop: the core
  takes the device key at launch only (architecture 4) [decision]. When no
  fingerprint is enrolled any more, the option is off and Settings says so.
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

### 6.1 List

- A top app bar with the vault's name and a search field that is always
  visible under it [decision]. Search filters hosts by label, address, user
  and tags as the desktop's 2.1 does; typing `user@host[:port]` that matches
  no host offers "Connect to user@host" at the top of the results (a quick
  connect, nothing saved, as on the desktop).
- Groups first (rows with the group's color label and the count of hosts
  inside), then the hosts of the current level; tapping a group goes into
  it (the top bar shows the path and back goes up) [decision: a drill-down
  list instead of the desktop's sidebar tree].
- A host row: the label (or the address), under it `user@address`, the OS
  icon when known; the color label as a leading stripe and a dot for a live
  session to it [decision].
- **Tap a host: connect** (a terminal opens, section 9). Long-press: a
  sheet with Connect, Open SFTP, Edit, Duplicate, Move to group, Delete
  [decision]. Long-press on a group: Edit, Move, Delete, as the desktop's
  group panel (3.4) does them.
- **Delete** asks first, names what goes, and moves it to Recently deleted
  (section 6.3; D93); the snackbar after it has "Undo".
- A floating "+" button: New host, New group [decision].
- Empty state: "No hosts yet" with "New host" and, when sync is not set up,
  "Restore from sync" [decision].
- Pull to refresh runs a sync when an account is connected [decision];
  offline it says "No network" and changes nothing.

### 6.2 Host editor

The desktop host panel (3.1) on one screen, for the blocks the first version
edits [decision on the subset]:

- Address, Label, Group (a picker), Tags, Color.
- SSH: Port, Username, Password (masked; saved in the vault as on the
  desktop), Key (a picker from the Keychain, or none). Inheritance is the
  desktop's (3.1, 3.4): a field the host leaves empty shows the group's
  value as its placeholder ("from group Servers"), and a value of the host's
  own overrides it.
- Terminal: theme (the host's or the default), charset, Backspace sends
  (the desktop's choices).
- Changes save as they are made, as the desktop's panel does (3.2);
  required fields and their errors are the desktop's.
- **Everything else on the record is kept as it is** and listed read-only
  under "More settings (edit on the desktop)": identity, agent forwarding,
  environment, jump hosts and proxy, keepalive, the startup snippet, Telnet,
  notes, port forwarding rules [decision]. A host that signs in with an
  identity shows the identity's name read-only.
- A group's editor: name, color label, parent group, and the group's SSH
  defaults (port, username, key) as on the desktop's 3.4.

### 6.3 Recently deleted

Under More, as the desktop's section 14 for this device: the list (name,
type, when deleted), search, **Restore** [decision: restore only; deleting
forever stays on the desktop]. Empty state: "Nothing deleted in the last 90
days".

## 7. Keychain

As the desktop's section 7 for keys:

- A list of keys: label, type and size (Ed25519, ECDSA, RSA 3072...), the
  SHA-256 fingerprint shortened, how many hosts use it [decision]. Empty
  state: "No keys yet" with Generate and Import.
- "+" offers: **Generate** (Ed25519 by default, as on the desktop; label;
  optional passphrase) and **Import from file** (the system file picker;
  OpenSSH and PuTTY formats as on the desktop; a passphrase prompt when the
  file has one; a file that is not a key says so and imports nothing).
- A key's screen: label (editable), type, fingerprint (copy), the public
  key in OpenSSH format with **Copy** and **Share** (the system share
  sheet) [decision], the hosts using it, **Delete**: as the desktop's 7.3,
  it asks first and lists the hosts that use the key, removes it from them,
  and offers Undo.
- The private key is never shown and never shared; export of private keys
  is the desktop's.
- Identities, certificates and security keys are the desktop's in the
  first version [decision]; records of them are kept as they are.

## 8. Known hosts

As the desktop's section 10:

- A list: host and port, key type, fingerprint shortened, when first seen;
  hashed entries show as "hashed host" with their key. Search by host. Empty
  state: "No known hosts yet". A row's screen shows the full SHA-256
  fingerprint (copyable) and **Remove**, which asks first and offers Undo
  as on the desktop.
- **First connection and a changed key** are the desktop's 10.2 dialogs as
  full-screen sheets that cannot be dismissed by a tap outside: the host,
  the key type and the full fingerprint; for a first key "Accept and save",
  "Connect once", "Cancel"; for a changed key a danger-styled warning with
  the old and new fingerprints, "Cancel" first, and "Replace the key" behind
  a second confirmation [decision on the order and the confirmation].
- A **revoked** key blocks the connection with no way to continue, as on the
  desktop (10.2): the terminal shows why.

## 9. Terminal

The main screen of the app. The behaviour of the session (connecting,
prompts, the end and the reconnect policy, the lock) is the desktop's
4.6 and 4.8; this section is the phone's view of it.

### 9.1 Layout

- **Portrait**: a thin top bar (back, the session's title, the connection
  state dot, a "⋯" menu), the terminal, the **extra keys** (two rows,
  section 9.2), the soft keyboard. The extra keys stay when the soft
  keyboard is down and when a hardware keyboard is used [decision].
- **Landscape**: the top bar hides while the keyboard is up; "⋯" is then the
  last key of the extra keys' second row. The keyboard never takes the
  whole screen (`IME_FLAG_NO_EXTRACT_UI`, `IME_FLAG_NO_FULLSCREEN`)
  [decision].
- The app draws edge to edge (Android 15+): the terminal and the extra keys
  sit above the keyboard's and the navigation bar's insets.
- The terminal fills the space between the bar and the extra keys; its
  columns and rows follow from the cell size (section 9.8).
- "⋯": Copy all screen, Paste, Select mode (9.4), Font size (reset),
  Open SFTP to this host, Disconnect, Reconnect (when dropped) [decision].
- Prompts of the connection (password, keyboard-interactive, a key's
  passphrase, the host key decisions of section 8) are sheets over the
  terminal, as the desktop shows them in the pane; their "Save password"
  choice is the desktop's.

### 9.2 Extra keys

- **Two rows** above the soft keyboard, each key 48 dp high and at least
  44 dp wide; no scrolling, so every key is always in reach [decision]:
  - first row: **Esc, Tab, Ctrl, Alt, ←, ↓, ↑, →** (the owner's keys, D211);
  - second row: **⌨, |, /, ~, -, Home, End, PgUp, PgDn** [decision on the
    order and the last four; in landscape "⋯" follows]. **⌨** shows or hides
    the soft keyboard.
- **Ctrl and Alt are sticky** [decision]: a tap arms the modifier for the
  next key (from the rows or the soft keyboard), drawn in the accent color;
  a double tap locks it, drawn filled, until it is tapped again. Ctrl with
  a letter sends the control character (Ctrl+C is 0x03); Alt sends Esc
  before the key, as the desktop's input does.
- The arrows send the cursor keys in the mode the program asked for
  (normal or application cursor keys), Home/End/PgUp/PgDn their escape
  sequences, as on the desktop.
- A key held down repeats (after 400 ms, every 50 ms) [decision].
- The rows are excluded from the system's edge gestures, so a press near
  the screen's edge is a key, not "back".
- The keys and their order can be changed in Settings in a later version.

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
  - a floating toolbar above the selection: **Copy, Paste, Select line,
    Select screen**, and **Delete** or **Cut in nano** where 9.6 allows them.
- **Select line** extends the selection to the whole logical line (wrapped
  rows joined); **Select screen** to everything visible.
- **Select mode** ("⋯" menu): the next drag on the terminal selects from
  where it starts to where it ends, as a mouse drag on the desktop; useful
  for a block of a configuration file that starts in the middle of a word.
- A tap outside the selection ends it and does nothing else. New output
  keeps the selection on the same text while it stays in the buffer, as on
  the desktop.
- **Copy** puts the selected text on the clipboard (lines joined as on the
  desktop: wrapped rows without a line break, trailing blanks dropped); below
  Android 13 a short "Copied" toast says so (from Android 13 the system
  shows its own notice). **Paste** follows the desktop's paste rules
  (bracketed paste, the multi-line confirmation of 4.2).
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
  [decision]; the size is kept per device, not synced.
- The terminal's columns and rows follow the cell size and the space it
  has; rotation, the keyboard coming up or down, and the font size change
  them. The new size goes to the server (a window change on the SSH channel,
  as on the desktop) once: when a pinch ends, and 150 ms after the last
  change of a rotation or the keyboard's animation [decision].

### 9.9 In the background and without a network

- **The app in the background**: open sessions keep running in a foreground
  service with a notification ("2 SSH sessions open", "Disconnect all"); the
  first session asks for the notification permission (Android 13+). When it
  is denied, the service still runs and the Sessions screen offers
  "Disconnect all". The last session closed stops the service.
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
  tapping the notification opens Sessions.
- **The process killed by the system** (no service, memory): the sessions
  end; the next start asks for the master password.
- **Sessions screen**: the empty state "No open sessions"; with sessions,
  "Disconnect all" in its top bar.

## 10. SFTP

The desktop's section 5 for one remote panel [decision on the subset]:

- Opened from a host's sheet (6.1) or the terminal's "⋯": connecting shows
  the steps and the prompts as the terminal does; then the remote home
  directory, a path bar (tap a segment to go up), the list (folders first,
  name, size, modified date in the local zone), pull to refresh.
- Tap a folder: open it. Tap a file: a sheet with **Download**, Rename,
  Delete (asks first), Copy path [decision].
- **Download** asks where with the system's "Save to…" picker, every time
  [decision]; the file goes straight from the server to the chosen document
  (architecture 6.4); progress shows in the transfer sheet and in the
  notification while the app is in the background (the transfer keeps the
  foreground service running); a name conflict asks as the desktop's 5.7.
- "+" offers **Upload** (the system file picker, one or more files), New
  folder.
- The transfer queue of the desktop (5.6), shown as a bottom sheet:
  current transfer, waiting ones, cancel; a failed transfer says why
  (permission denied, no space, connection lost) and offers Retry.
- Errors of a listing or an action (permission denied, not found) show
  their reason under the path bar, as the desktop's panel does.
- Names that are not UTF-8 on the server are the desktop's known
  limitation.

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

- **Appearance**: Theme (Auto, Dark, Light); terminal theme (the default
  for hosts without one).
- **Terminal**: font size, Autoreconnect (on), Keepalive (the desktop's
  default), Bell (vibrate, off) [decision], copy on select (off) [decision].
- **Security**: Auto-lock (section 5); **Fingerprint unlock** (off by
  default; turning it on asks for the master password once and then the
  biometric prompt; it unlocks at a cold start, a locked app asks for the
  password; architecture 4) (D211). A copied password is cleared from the
  clipboard after 30 seconds, as on the desktop (`docs/security.md` 9.4);
  Android hides the clipboard from apps in the background, so it is cleared
  when the app next has focus.
- **About**: version, licenses (the third-party notices), the project's
  page.

## 13. Not in the first version

Snippets, command history and autocomplete (the owner uses them: the next
versions, D211), port forwarding, Mosh, workspaces and splits, broadcast,
the local terminal and serial ports, identities and certificates, editing
jump hosts and proxies, import from other apps, recordings, export, the
desktop's Logs screen, team vaults (stage 8). The phone keeps every record
it does not edit as it is.
