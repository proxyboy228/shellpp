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
- **Touch**: every control has a touch target of at least 48 × 48 dp;
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
  4. **More**: Known hosts (section 8), Account and sync (section 11),
     Settings (section 12), Lock.
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
    sign-in form, one step per screen: the server address (`https://`;
    plain `http` only for a loopback address, as on the desktop), the
    certificate check of 13.5 step 1 (a certificate the system does not
    trust offers the same choices: pin it, or add its CA for this server),
    the email and the account password, and the device name (default: the
    phone's model name) [decision on the device name default]. The vault
    comes from the account; the master password of this device is the
    account password (as on the desktop).
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
  takes the device key at launch only (architecture 4) [decision].
- A wrong password clears the field and says "Wrong password"; the pauses
  after 5 wrong passwords in a row are the desktop's and the core's.
- "Forgot password?" offers the desktop's choices: Restore from sync (with
  the recovery key), Start with a new vault (with the same confirmation;
  the data file is renamed and kept), Cancel.
- **Auto-lock**: the desktop's choices (never, 5, 15, 30, 60 minutes); on
  the phone the default is 15 minutes, and the time counts from the last
  touch in the app, the app in the background included [decision]. Live
  terminals keep running while locked (as on the desktop); opening one
  asks for the password first.

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
  icon when known, the color label as a leading stripe; a dot for a live
  session to it.
- **Tap a host: connect** (a terminal opens, section 9). Long-press: a
  sheet with Connect, Open SFTP, Edit, Duplicate, Move to group, Delete
  [decision]. Delete goes to Recently deleted as on the desktop (D93) and
  says so with an "Undo" snackbar.
- A floating "+" button: New host, New group [decision].
- Empty state: "No hosts yet" with "New host" and, when sync is not set up,
  "Restore from sync".
- Pull to refresh runs a sync when an account is connected [decision].

### 6.2 Host editor

A full screen with the desktop host panel's blocks (3.1) that the first
version needs, in this order [decision on the subset]:

- Address, Label, Group (a picker), Tags.
- SSH: Port (22), Username, Password (masked; saved in the vault as on the
  desktop), Key (a picker from the Keychain, or "none"), "Use the same
  credentials as the group" when the group has them (the desktop's
  inheritance rules).
- Terminal: theme (the host's or the default), charset, Backspace sends
  (the desktop's choices), startup snippet stays for later versions.
- Advanced (collapsed): keepalive, jump hosts and proxy are shown as they
  are on the record but are edited on the desktop in the first version
  [decision: kept, never dropped, by a phone edit].
- Save in the top bar; leaving with unsaved changes asks "Discard
  changes?". Required fields and their errors are the desktop's.
- A group's editor: name, color label, parent group, and the group's SSH
  defaults (port, username, key) as on the desktop's 3.4.

## 7. Keychain

As the desktop's section 7 for keys:

- A list of keys: label, type and size (Ed25519, ECDSA, RSA 3072...), the
  SHA-256 fingerprint shortened, the hosts using it as a count.
- "+" offers: **Generate** (Ed25519 by default, as on the desktop; label;
  optional passphrase) and **Import from file** (the system file picker;
  OpenSSH and PuTTY formats as on the desktop; a passphrase prompt when the
  file has one).
- A key's screen: label (editable), type, fingerprint (copy), the public
  key in OpenSSH format with **Copy** and **Share** (the system share
  sheet) [decision], the hosts using it, Delete (refused while hosts use
  it, as on the desktop).
- The private key is never shown and never shared; export of private keys
  is the desktop's.
- Identities, certificates and security keys are the desktop's in the
  first version [decision]; records of them are kept as they are.

## 8. Known hosts

As the desktop's section 10:

- A list: host and port, key type, fingerprint shortened, when first seen.
  Search by host. A row's screen shows the full SHA-256 fingerprint
  (copyable) and Delete.
- **First connection and a changed key** are the desktop's 10.2 dialogs as
  full-screen sheets that cannot be dismissed by a tap outside: the host,
  the key type and the full fingerprint; for a first key "Accept and save",
  "Connect once", "Cancel"; for a changed key a danger-styled warning with
  the old and new fingerprints, "Cancel" first, and "Replace the key" behind
  an extra confirmation [decision on the order].

## 9. Terminal

The main screen of the app. The behaviour of the session (connecting,
prompts, the end and the reconnect policy, the lock) is the desktop's
4.6 and 4.8; this section is the phone's view of it.

### 9.1 Layout

- **Portrait**: a thin top bar (back, the session's title, the connection
  state dot, a "⋯" menu), the terminal, the **extra keys row**, the soft
  keyboard. Landscape: the top bar hides while the keyboard is up; a swipe
  down from the top edge shows it [decision].
- The terminal fills the space between the bar and the extra keys row; its
  columns and rows follow from the cell size (section 9.8).
- Tapping the terminal when the keyboard is down brings the keyboard up
  (unless the tap moves the cursor, section 9.3).
- "⋯": Copy all screen, Paste, Select mode (9.4), Font size (reset),
  Open SFTP to this host, Disconnect, Reconnect (when dropped) [decision].
- Prompts of the connection (password, keyboard-interactive, a key's
  passphrase, the host key decisions of section 8) are sheets over the
  terminal, as the desktop shows them in the pane; their "Save password"
  choice is the desktop's.

### 9.2 Extra keys row

- One row of keys above the soft keyboard, 44 dp high, scrolling
  horizontally when the screen is narrow [decision]. The default set, in
  this order: **Esc, Tab, Ctrl, Alt, ←, ↓, ↑, →, |, /, ~, -, Home, End,
  PgUp, PgDn** (the owner's list, D211, then the editor keys) [decision on
  the order and the last four].
- **Ctrl and Alt are sticky modifiers** [decision]: one tap arms the
  modifier for the next key (from the row or the soft keyboard) and it is
  drawn in the accent color; a second tap locks it (drawn filled) until
  tapped again; a third tap or a key sent with a locked modifier keeps it
  locked. Ctrl with a letter sends the control character (Ctrl+C is 0x03);
  Alt sends Esc before the key, as the desktop's input does.
- The arrows send the cursor keys in the mode the program asked for
  (normal or application cursor keys), Home/End/PgUp/PgDn their escape
  sequences, as on the desktop.
- A key held down repeats (after 400 ms, every 50 ms) [decision].
- **Moving the cursor by dragging the row** [decision]: a horizontal drag
  that starts on the row (anywhere but Ctrl and Alt) moves the cursor one
  cell per step of a character's width, sending ← or →; a magnifier
  (section 9.5) shows the cursor's surroundings above the finger while
  dragging. The drag does not send the key it started on.
- The row's keys and order can be changed in Settings in a later version.

### 9.3 Putting the cursor on a character (the owner's first pain)

What a tap does depends on where it lands and what the program asked for
[decision]:

1. **The program asked for mouse reports** (nano with `set mouse`, `htop`,
   `mc`, vim with `mouse=a`; the terminal knows it, as the desktop's 4.3):
   a tap is a left click at that cell (press and release). The program puts
   its cursor there itself. A tap with Ctrl or Alt armed sends the click
   with that modifier.
2. **No mouse reports, on the shell's command line** (no alternate screen;
   the tap on the line being typed: the cursor's row or a row it wraps
   onto): the terminal walks the cursor there with ← and →. It works with
   every shell and line editor that understands the arrow keys (bash, zsh,
   fish, readline programs, the Python prompt).
3. **No mouse reports, in a full-screen program** (the alternate screen:
   nano as it comes, vim, `less`): the terminal walks the cursor with ↑ and ↓
   to the tapped row, then with ← and → to the tapped column.
4. **Anywhere else** (the shell's screen outside the typed line): the tap
   does nothing to the cursor; it brings up the keyboard if it is down.

- **Walks use feedback** (architecture 6.3): one arrow, then the terminal
  waits for the program to move its cursor and reads where it went, and so
  on, until the cursor is on the tapped character or an arrow no longer
  moves it. Tabs, wide characters and an editor's own line wrapping land
  right that way.
- **Limits**: a walk sends at most as many arrows as there are cells
  between the cursor and the tap and stops after an arrow that did not move
  the cursor. Where the arrows land is the program's business: a line
  editor that stops at the prompt's end stops there; in a program where
  arrows do not move a cursor (`less` scrolls with them) a tap sends at most
  a few arrows. While a walk runs, typing waits until it ends (a walk takes
  a round trip to the server per step, so a long one on a slow link takes a
  moment; the cursor visibly travels).
- **Precision**: a tap selects the cell under the finger's center; while
  the finger rests (a long press that does not move, 9.4) a magnifier
  shows the cells under it. Fine moves after that are the row drag (9.2)
  or the arrows.
- The cursor moves at once on the screen when the program echoes it; the
  terminal does not draw a cursor of its own anywhere else.

### 9.4 Selecting text (the owner's second pain)

[decision for the whole section]

- **Long press** on text selects the word under the finger (the desktop's
  double-click rule, `shellpp-terminal`'s selection: a word ends at a space
  or one of ``,│`|:"'()[]{}<>``, so a path is one word) and shows:
  - two handles at the selection's ends; dragging a handle extends or
    shrinks the selection cell by cell, across rows and into the
    scrollback (the view scrolls when the handle nears the top or the
    bottom edge); a magnifier shows the cells under the handle;
  - a floating toolbar above the selection: **Copy, Paste, Select line,
    Select screen**, and **Delete** where 9.6 allows it.
- **Select line** extends the selection to the whole logical line (wrapped
  rows joined); **Select screen** to everything visible.
- **Select mode** ("⋯" menu): the next drag on the terminal selects from
  where it starts to where it ends, as a mouse drag on the desktop; useful
  for a block of a configuration file that starts in the middle of a word.
- A tap outside the selection ends it. New output keeps the selection on
  the same text while it stays in the buffer, as on the desktop.
- **Copy** puts the selected text on the clipboard (lines joined as on the
  desktop: wrapped rows without a line break, trailing blanks dropped) and
  shows a short "Copied" toast. **Paste** follows the desktop's paste rules
  (bracketed paste, the multi-line confirmation of 4.2).
- When the program asked for mouse reports, a long press still selects
  (the terminal's own selection), as Shift+drag does on the desktop.

### 9.5 Magnifier

- Shown while a handle moves, while the extra keys row is dragged, and
  under a resting long press: a rounded lens 1.5× the text above the
  finger, from the system (`Magnifier`, Android 9+) [decision].

### 9.6 Deleting a selected piece in an editor (the owner's example)

The owner's example is a piece of a configuration file in nano. A terminal
shows the server's screen; it cannot delete text on the server. Only the
program can, with its own keys. So the app offers what really works and
says where it stops [decision]:

- **On the shell's command line** (no alternate screen, the selection on
  the line being typed): **Delete** moves the cursor to the selection's end
  (9.3, case 2) and sends one Backspace per selected character. The
  selection may not leave that line.
- **In a full-screen program** (the alternate screen is on): the toolbar
  offers **Cut in nano** instead of Delete. It walks the cursor to the
  selection's first character (9.3, a click when nano takes the mouse, the
  arrow walk otherwise), sets nano's mark (Alt+A), walks to just past the
  selection's last character and cuts (Ctrl+K). The cut text is in nano's
  cut buffer, as if done with nano's own keys, and Ctrl+U pastes it back.
- Other editors use their own keys from the extra keys row and the tap
  (vim: tap, `v`, tap, `d`).
- **The limits**, said in the toolbar's help line: the app does not know
  which program is running; "Cut in nano" sends nano's keys, which do other
  things in other programs. A selection that runs past what the editor
  shows (into the scrollback) cannot be cut this way: the toolbar offers
  Copy only then.

### 9.7 Scrolling

- A vertical swipe scrolls the scrollback (the desktop's 10 000 lines and
  journal, D85); a fling continues. When the program asked for mouse
  reports, the swipe sends wheel events instead (as the desktop's wheel
  does), and in the alternate screen without them it sends ↑/↓ [decision:
  as alacritty's alternate scroll].
- New output while scrolled back does not move the view; a "↓ New output"
  chip returns to the bottom [decision].

### 9.8 Font size, rotation and the terminal's size

- **Pinch** changes the font size from 8 to 28 sp in 1 sp steps
  [decision]; the size is kept per device, not synced.
- The terminal's columns and rows follow the cell size and the space it
  has; rotation, the keyboard coming up or down, and the font size change
  them. The new size goes to the server (a window change on the SSH channel,
  as on the desktop) when the change settles (after 150 ms) [decision], so
  the program redraws once, not at every frame of an animation.

### 9.9 In the background and without a network

- **The app in the background**: live SSH sessions keep running in a
  foreground service with a notification ("2 SSH sessions open",
  "Disconnect all"); the first session asks for the notification permission
  (Android 13+). The last session closed stops the service.
- **The network lost or changed**: the session follows the desktop's 4.6:
  a dropped connection shows "Connection lost" in the terminal and the
  reconnect countdown (1, 2, 4, 8, 16, 32 s, then every 60 s, with
  Autoreconnect on); "Reconnect now" and "Close" in the bar. When the phone
  gets a network back, a waiting attempt starts at once.
- **Locked** (auto-lock, section 5): sessions keep running; their screens
  show the unlock screen first.
- **Swiped away from recents** while sessions live: the service keeps them;
  tapping the notification opens Sessions.
- **The process killed by the system** (no service, memory): the sessions
  end; the next start asks for the master password.

## 10. SFTP

The desktop's section 5 for one remote panel [decision on the subset]:

- Opened from a host's sheet (6.1) or the terminal's "⋯": the remote home
  directory, a path bar (tap a segment to go up), the list (folders first,
  name, size, modified date in the local zone), pull to refresh.
- Tap a folder: open it. Tap a file: a sheet with **Download**, Rename,
  Delete (asks first), Copy path.
- **Download** saves through the system's file picker ("Save to...") or to
  Downloads by default [decision]; the file goes straight from the server to
  the chosen document (architecture 6.4); progress shows in the transfer
  sheet and in the notification while the app is in the background (the
  transfer keeps the foreground service running); a name conflict asks as
  the desktop's 5.7.
- "+" offers **Upload** (the system file picker, one or more files), New
  folder.
- The transfer queue of the desktop (5.6), shown as a bottom sheet:
  current transfer, waiting ones, cancel.
- Names that are not UTF-8 on the server are the desktop's known
  limitation.

## 11. Account and sync

The desktop's 13.4 and 13.5 that the first version needs:

- **Connect a server** (when none is): the connect wizard of section 4 in
  its full form (register or sign in), one step per screen.
- **Connected**: the server, the account email, this device's name, the
  last sync time and result, **Sync now**, the list of the account's
  devices (as on the desktop), **Sign out** (asks first; the local vault
  stays).
- The desktop's dialogs that sync can raise: a changed pinned certificate
  (12.2), conflicts (13.6: a conflict copy is made and shown as on the
  desktop, resolved in the first version on the desktop) [decision],
  duplicates (13.7, a notice only) [decision].
- Changing the password, the recovery key and "Lost or stolen" are the
  desktop's in the first version [decision]; the phone shows where to do
  them.

## 12. Settings

- **Appearance**: Theme (Auto, Dark, Light); terminal theme (the default
  for hosts without one).
- **Terminal**: font size, Autoreconnect (on), Keepalive (the desktop's
  default), Bell (vibrate, off) [decision], copy on select (off) [decision].
- **Security**: Auto-lock (section 5), **Fingerprint unlock** (off by
  default; turning it on asks for the master password once and then the
  biometric prompt; it unlocks at a cold start, a locked app asks for the
  password; architecture 4) (D211), Clear the clipboard after copying a
  password (as on the desktop; a copied password is cleared when the app
  next has focus, since Android hides the clipboard from apps in the
  background).
- **About**: version, licenses (the third-party notices), the project's
  page.

## 13. Not in the first version

Snippets, command history and autocomplete (the owner uses them: the next
versions, D211), port forwarding, Mosh, workspaces and splits, broadcast,
the local terminal and serial ports, identities and certificates, editing
jump hosts and proxies, import from other apps, recordings, export, the
desktop's Logs screen, team vaults (stage 8). The phone keeps every record
it does not edit as it is.
