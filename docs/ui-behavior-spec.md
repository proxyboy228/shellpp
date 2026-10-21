# UI Behavior Specification

> This document describes the **structure and behavior** of the interface: what is located where,
> and what happens on each action. The visual style (colors, icons, fonts,
> radii, shadows) is described separately in `docs/design-system.md`.
>
> Marks:
>
> - **[decision]**: behavior is not confirmed, chosen by us, can be changed;
> - **[clarify]**: awaiting an answer; until answered, implement as [decision].
>
> The decisions taken before stage 0 are recorded in `docs/decisions.md`. Behavior that they
> added or changed is marked [decision] in this document.

## 0. Platforms

- Windows (primary for manual testing), macOS, Linux. Later iOS and Android.
- Keyboard shortcuts are given for Windows/Linux. On macOS, Ctrl is replaced with Cmd
  where this is the platform convention; the Mac key set is described in 12.5. [decision]
- Elements of features that belong to later stages (see SPEC.md, "Work order") are hidden
  behind a stage flag rather than shown disabled. The window frame and the sidebar are the
  exception: the SFTP tab (1.1) and all six sidebar sections (1.2) are always shown, and one
  whose screen is not implemented yet opens an "In development" placeholder. [decision]
  Two more exceptions (D29): every field of the host panel (3.1) is shown and its value is
  saved before the feature works in sessions, and "Restore from sync" (13.2) is shown
  disabled until sync exists.
  Stage 2a adds one more (D30): menu items and buttons that lead to features of stage 2b
  and later (splits, the terminal sidebar, the new tab page and its "+" button, Ctrl+K
  search, the Known Hosts screen, SFTP and port forwarding) are shown disabled and do
  nothing. The sidebar sections and the SFTP tab keep their placeholders.
  Stage 2b turns on the splits, the terminal sidebar, the new tab page with its "+"
  button, Ctrl+K search, several windows and the Known Hosts screen (D31). Items of later
  stages inside them follow the D30 rule and are shown disabled: Broadcast input (4.4,
  stage 9), Create a workspace (6, stage 9) and the "Save" button of the history hint
  (4.2, stage 5). The Snippets tab of the terminal sidebar shows an empty state until
  stage 5. Start multiplayer (4.4) and Autocomplete (4.2) are not shown at all, as the
  owner asked. [decision]
- The interface is in English. Interface strings in this document define the meaning; the final
  texts live in the i18n catalog (`en`). UI tests find elements by element id, not by
  text. [decision]

---

## 1. Main window

- Minimum window size: 960 × 600 px (D29). At this size nothing is clipped and nothing
  overlaps, with the details panel (1.3) open at its default width of 380 px and with it
  closed.

### 1.1 Top bar (window title bar)

- Custom window title bar (frameless). On Windows/Linux, the minimize /
  maximize / close buttons are on the right; on macOS, the system buttons are on the left. [decision for macOS]
- From left to right:
  1. App menu button.
  2. Persistent **Vaults** tab: the data management workspace.
     Next to it is an arrow switcher for the current vault (personal, team). The tab and
     the arrow are one pill: the active highlight (the `--accent-subtle` pill and the bar
     under it) spans the arrow too; the arrow stays a click zone of its own inside the
     pill and is highlighted on hover. [decision]
  3. Persistent **SFTP** tab: the SFTP manager for manual work (section 5). [decision]
  4. Open session tabs: from left to right, the host's OS icon, the title, the state dot
     and the session type icon. [decision on the icons and the dot]
     - The left icon is the host's OS icon, as on the host card (2.3); while the OS is not
       known, it is the neutral server icon, and a local terminal has the monitor icon. It
       is never the terminal glyph, so it cannot be mistaken for the session type icon.
       [decision]
     - The title is the host name: the label, or the address if the label is empty. A local
       terminal tab shows its shell's display name (12.4). A title set with Rename (4.4)
       replaces it, and a tab with splits shows its active pane's host (4.7). [decision]
     - The state dot, 8 px (`docs/design-system.md`, 3.3), stands right after the title:
       connecting and reconnecting `--warning`, connected `--success`, a connection drop
       or a failed connection `--danger`. A session that ended with a non-zero exit code
       (4.6) is inactive, `--text-muted`. [decision]
     - While a session is dropped (4.6), its terminal is dimmed with the disabled opacity
       (`--opacity-disabled`, `docs/design-system.md`, 6) and its cursor is hidden. Both
       come back when the session reconnects. [decision]
     - On the active tab (and on any tab on hover), a close button
       appears in place of the OS icon.
     - The active tab is marked with an `--accent` bar. The colors of the host's terminal
       theme are not used for it. [decision]
     - On the right side of the tab, a session type icon: terminal (SSH sessions and local
       terminals) or SFTP. An SFTP session tab is opened by "Connect via SFTP" (see 5.1).
       [decision]
     - Right-click opens the tab menu (4.4).
     - Tabs that do not fit [decision]: session tabs shrink down to a minimum width, and
       beyond that the row of session tabs scrolls sideways (the mouse wheel over it,
       vertical or horizontal). The active tab always scrolls fully into view when it
       becomes active (a new session, a click, Ctrl+1…9, Alt+←/→, the neighbor rule of
       4.6). The "+" button stays right after the row.
  5. "+" button: a new tab with the start page (see section 6; Ctrl+T and Ctrl+Shift+T,
     see 12.5).
  Tabs are 32 px high inside the 44 px bar, with 10 px inside and 4 px between them
  (`docs/design-system.md`, 6). [decision]
- On the right:
  - update button (visible only when a check has found an update, see 12.7);
  - notification center (see 1.5);
  - button to show and hide the terminal sidebar (only when a session
    tab is active, see 4.2); it is shown selected while the sidebar is open; [decision]
  - window buttons.

### 1.2 Sidebar (inside the Vaults tab)

- Sections from top to bottom: **Hosts**, **Keychain**, **Port Forwarding**, **Snippets**,
  **Known Hosts**, **Logs**.
- The active section is highlighted. Each section opens its own screen in the main area.
- The sidebar is 196 px wide; its items are 32 px high and 2 px apart, with 12 px inside,
  a 16 px icon and 10 px between the icon and the name; the sidebar keeps 8 px from its
  edges (`docs/design-system.md`, 6). [decision]
- Narrow window [decision]: when the main area would be narrower than 600 px next to the
  full sidebar (for example, a small window with the details panel open), the sidebar
  collapses into a 56 px strip with only the section icons. Hovering an icon shows the
  section name in a tooltip; the active section stays highlighted, and a click opens the
  section as before. When the main area would again be at least 600 px wide next to the
  full sidebar (a wider window or a closed details panel), the sidebar expands back. The
  width is always measured as if the sidebar were full, so it does not flicker at 600 px.

### 1.3 Details panel

- A slide-out panel on the right. Opens when an object is selected or when a new one is created.
- In the header: a title ("host details" / "new host"), a "…" menu with additional
  actions (the set depends on the object; for a host see 3.3), a button to collapse the panel.
- The main area narrows; the panel does not overlap it. [decision]
- Saving rule for every object panel (hosts, groups, keys, identities, forwarding rules,
  snippets) [decision]:
  - a new object is created once all its required fields are filled in with valid values.
    Until then it is a draft, and closing the panel discards the draft without prompting;
  - after that, changes are saved automatically; there is no save button;
  - an invalid value is highlighted, with the reason under the field, and is not saved. The
    last valid value stays in effect;
  - wizards with an explicit "Create" button (8.3), the Keychain's forms that add a
    key (New key and Generate key, 7.3, D33) and the snippet panel (9.3) are the only
    exceptions.
- The snippet panel saves only with Create and Save: a run must use a saved script, and
  a script is often typed in several goes. [decision] (D66, D68)
  - while the panel holds changes that are not saved, its header shows "Not saved";
  - leaving it then (another card, a new snippet, the collapse button or Esc, another
    section of the sidebar) asks "Save", "Discard changes" or "Stay". Enter is Save, Esc
    is Stay; a Save the checks refuse keeps the panel open with the reasons under the
    fields. Switching to a session tab or the SFTP tab does not ask: the panel stays as
    it is;
  - Run with changes not saved asks "Save and run" (or Cancel).

### 1.4 App menu (the button with three bars)

1. Vaults (Ctrl+1).
2. SFTP (Ctrl+2).
3. Settings (Ctrl+,).
4. — separator —
5. New Local Terminal.
6. New Serial Connection.
   Then, between separators, **Import Data…** and **Export Data…** (D38): the native JSON
   export of all data, plain or encrypted with a password, and its import (principle 4,
   [export-format.md](export-format.md)). The plain export warns first that the file holds
   every secret in clear text and asks for "I understand". Export Data also writes
   OpenSSH files (D43): a `shellpp-ssh-config` and, optionally, the key files into a
   folder the user chooses, with a passphrase offered for the keys that have none (off
   asks for "I understand"); files that are already there are replaced only after
   "Replace". The JSON export has "Include command history" and "Include the connection
   log", both off (D50). The import applies only the
   settings of how the app and the terminal look and behave; a setting that runs or opens
   something, points somewhere or guards the user stays as it is here, and the summary
   counts those whose value in the file differs (D41). "Full restore from my backup"
   (off) applies every setting of the file; turned on, it says in `--danger` that the
   file may hold a script that will then run on the servers (D50). A file that holds
   command history or a connection log asks next whether to import them ("Import command
   history", "Import the connection log", only those the file holds, both on); imported
   history goes through the secret detector of 4.2 (D51). The import takes snippets,
   snippet packages and forwarding rules too; a forwarding rule never starts on launch,
   and one that listens beyond loopback not on connect either, except in a full restore
   (D63). The summary also counts the keys kept as unknown, the commands left out and the
   rules whose autostart was turned off, when there are any. [decision]
7. Edit ▸: undo, redo, cut, copy, paste, select all. [decision]
8. View ▸: interface zoom, full-screen mode, show or hide
   the sidebar. [decision]
9. Window ▸: new window, next and previous tab, close tab. [decision]
10. Help ▸: documentation, report an issue (link to issues), check for
    updates, about. [decision] "Check for updates" is always available, even when the
    automatic check is off (see 12.7). [decision]
11. Lock (Ctrl+Shift+L): lock the app now (see 13.3).
12. — separator —
13. Exit. While sessions, port forwards or file transfers are active, Exit asks for
    confirmation (see 4.6). [decision]

Additionally:

- Keys in hints are written as symbols (`Ctrl+,`), not as words in the system
  language. [decision]
- On macOS, the system menu bar with the same items is used instead of this
  menu. [decision]
- All windows are equal: each window has its own tabs, and changes to data and settings
  appear in all windows at once. Moving tabs between windows is not in the MVP. [decision]
  Details [decision]:
  - every window has its own top bar, Vaults and SFTP tabs, session tabs, details panel
    and terminal sidebar (4.2). The data, the settings, the running sessions and the lock
    belong to the app and are shared by all windows;
  - a new window opens with Window ▸ New window, with Duplicate in a new window (4.4) and
    with Alt held on a connect action (2.4). It has the size of the current window and is
    placed with an offset of `--space-6` from it;
  - locking covers every window at once (13.3);
  - closing a window with active sessions asks for confirmation first (4.6); the dialog
    says how many sessions of that window are open and will be closed. The app quits when
    its last window closes.

### 1.5 Notification center [decision]

- The notification center button in the top bar (1.1) opens the list of notifications,
  newest first. A counter on the button shows the number of unread notifications.
- A new notification appears as a toast for 5 seconds and stays in the list. Opening the
  list marks all notifications as read.
- Example source: uploads of files opened from SFTP (5.4).
- Notifications are kept in memory only and disappear when the app exits. System (OS)
  notifications are not used.

---

## 2. Hosts screen

### 2.1 Search and quick connect

- Search bar at the top of the main area, full width. The placeholder hints
  that you can search for a host or enter `ssh user@hostname`. When the bar is too narrow
  for it (the minimum window with the details panel open), a shorter hint is shown instead of
  a cut-off one. [decision]
- Typing filters groups and hosts in real time: by name, address,
  tags and username. [decision on the set of fields]
- If the string is recognized as a connection address (`host`, `user@host`,
  `user@host:port`, `ssh user@host -p port`), the Connect button on the right side of the bar
  becomes enabled. Enter or a click connects without saving the host.
- A single word (the `host` form) counts as an address only if it contains a dot or a colon,
  is an IP address, or is `localhost`. Any other single word is only a search query. [decision]
- If the text exactly matches the label or the address of a saved host, Enter and the Connect
  button connect to that saved host with its settings. [decision]
- If the string has no username and the host does not set one, the username is asked for
  while connecting (see 4.8). [decision]
- While the string is not an address, the Connect button is disabled.
- If the string is recognized as an address and no group or host matches the search, the
  content area shows "Press Enter to connect to <address>" instead of "No hosts match the
  search", with a "Save as host" button. <address> is the recognized address in the
  `user@host:port` form, without the parts the string does not give. Enter or Connect
  connects as described above; "Save as host" opens the details panel with a new host whose
  address, port and username come from the string (1.3). [decision]
- After a successful quick connect, an unobtrusive offer to save the host is shown. [decision]

### 2.2 Action bar

On the left:

- **New host** split button. The main part creates a host; the arrow opens a menu:
  1. **New Group**: create a group in the current location. The group panel opens
     (see 3.4). [decision]
  2. **Import**: the import dialog (see 2.5); formats are listed in SPEC.md, section
     "Import".
  3. Cloud integrations (AWS, DigitalOcean, Azure): connecting a provider
     account and importing its servers as hosts. **A separate later stage**;
     in the MVP these items are hidden.
- **Terminal**: open a local terminal in a new tab. The default shell
  is set in the settings (on Windows, PowerShell / cmd / WSL). [decision]
- **Serial**: serial port connection, opens as a separate
  tab in the top bar.

On the right:

- View toggle: tiles / list.
- **Tag filter** (D29):
  - a button with the tag icon. A click opens a dropdown:
    - a search field at the top, with focus in it, that narrows the list while typing
      (case-insensitive substring); [decision]
    - every tag of the current vault, sorted by name, each with a checkbox and the number
      of hosts that carry it. Several tags can be checked;
    - a "Clear" button that unchecks every tag; disabled while none is checked. [decision]
  - Checking a tag does not close the dropdown; Esc or a click outside closes it. [decision]
  - The selected tags are shown as chips next to the button, in the order of selection,
    each with a delete button that removes the tag from the filter. The chips use the active
    filter style (`docs/design-system.md`, 3.4).
  - When the chips do not fit, the ones that fit are shown and the rest collapse into a
    "+N" chip, where N is the number of hidden selected tags. A click on "+N" opens the
    dropdown. A selected tag is never hidden without being counted.
  - While at least one tag is selected, the tag button shows its active state.
  - A host passes when it carries every selected tag (case-insensitive). The filter combines
    with the search text (2.1). Like a search, it shows the matching hosts of the current
    group and its subgroups instead of the folder; groups are not shown while a tag is
    selected, because groups carry no tags. A tag that no host carries any more leaves the
    selection. [decision]
- Sort menu: Manual, by name, by creation date, by modification date. Manual is the default
  on Hosts, and only in this mode does dragging change the order of cards (see 2.3). The icon
  is the same as on the other screens (see 7.1). [decision]
- Avatars of the current vault's members and a "+" button. A click opens a dropdown menu:
  an "Invite members" item, below it a list of current members (avatar + email).
  "Invite members" opens the invitation dialog (see 12.3). [decision]

### 2.3 Screen content

- Two sections: **Groups** on top, **Hosts** below it.
- Groups work like folders: a click opens a group, inside which its subgroups
  and hosts are visible. Above the content is a path (breadcrumbs) with return to any level. [decision]
- Group card: icon, name, number of hosts (including nested groups). [decision]
  Right-click opens the group menu (see 3.4). [decision]
- Host card:
  - OS icon. The distribution is detected on the first successful connection
    (see 12.4, block "OS detection"). Until then, a neutral icon is shown.
    OS icons are taken from a set with a suitable license, taking into account
    the trademark policies of the distributions;
    - Linux has the app's own simple penguin drawn on the Lucide grid (not a logo), and
      Windows and macOS the monitor icon; any other or unknown OS has the neutral server
      icon. No OS icon is the terminal glyph, which marks the session type on tabs (1.1);
      [decision]
  - title: the label, or the address if the label is empty;
  - subtitle in the secondary color, comma-separated: protocol(s), username,
    then the group or tags;
  - long text is truncated with an ellipsis.
- The grid is adaptive: the number of columns depends on the width of the main area.
- List view (the view toggle, 2.2) [decision]:
  - hosts are table rows with the columns Name | Address | User | Tags under a header row.
    Name is the OS icon and the title (as on the card), Address is the host's address, User
    is the username (empty when none is set), Tags are the host's tags, comma-separated.
    A host without tags shows its group in the Tags column instead: the folder icon and
    the group name (empty for a host without tags at the top level). The card subtitle is
    not repeated in a row;
  - groups stay above the hosts as rows with the icon, the name and the number of hosts;
  - column widths change by dragging the border between two column headers. Each column
    keeps a minimum width that fits its header, long text is truncated with an ellipsis,
    and the chosen widths are saved in the settings and kept after a restart;
  - selection, double click, Enter, the context menu, dragging and the keyboard arrows
    work on rows as on cards. A click on a header does not sort; sorting is in the sort
    menu (2.2).
- The selected card is visually highlighted.
- A card of an object with an unresolved sync conflict shows a conflict badge
  (see 13.6). [decision]
- Multiple selection is supported: Ctrl+click adds or removes
  a card, Shift+click selects a range. Context menu actions that
  apply to multiple hosts (Connect, Add to Workspace, Move to, Copy to,
  Remove) are performed for all selected ones. [decision on keys and set]
  - They act only on the selected cards on screen, in screen order (not in the order of
    the clicks), and so does a drag; a search or a tag filter that hides a selected card
    removes it from the selection. [decision]
- Cards can be dragged [decision on details]:
  - in the Manual sort mode (see 2.2), dragging within the list changes the order. In the
    other sort modes the order does not change; [decision]
  - when dropped onto a group card, the host is moved into that group, in any sort mode.
- Promo banners are not needed. Instead, an empty state with hints. [decision]
- Empty state (no hosts): a short explanation and the buttons "Create host"
  and "Import OpenSSH config" (the path `~/.ssh/config` in its tooltip). The second one
  opens the import dialog (2.5) with the `~/.ssh/config` source selected. [decision]
  - "Create host" is the screen's primary button, in the accent style. While the details
    panel (a host or group panel, 1.3) is open, the primary action is in the panel, so
    "Create host" takes the neutral style: a screen has one accent button
    (`docs/design-system.md`, 1). [decision]

### 2.4 Host card actions

- Single click: select the card and show details in the details panel.
- Double click: connect (new session tab).
- Enter on a selected card: quick connect.
- E on a selected card: edit (details with focus in the first field).
- Keyboard arrows move the selection across the grid. [decision]
- Right-click opens the context menu (order matters):
  1. **Quick Connect** (Enter): connect via the primary protocol.
  2. **Connect ▸**: submenu:
     - via SSH (terminal session);
     - via SFTP: opens a new SFTP session tab with the host in the right panel
       (see 5.1); [decision]
     - separator;
     - "in a new window" hint: if Alt is held while choosing any option,
       the session opens in a new app window (1.4). Alt + double click on
       a card, Alt held while choosing Quick Connect and Alt + Connect in the host panel
       (3.1) work the same way. [decision for double click, Quick Connect and the host
       panel]
  3. **Add to Workspace ▸**: add to a saved workspace
     (details in the "Terminal" section).
  4. **Edit Host Details** (E).
  5. **Collaborate**: opens the "Move or copy to a vault" dialog (see 2.6) with a team vault
     as the target. The unit of sharing is a whole vault, not a single host. [decision]
  6. **Move to ▸**: move to another vault or group. Moving to another vault goes through
     the dialog in 2.6. [decision]
  7. **Copy to ▸**: copy to a group or to another vault. The submenu lists "Top level" and
     the groups of the current vault; choosing one creates a copy with the same label there
     (D29). Other vaults join the submenu with team vaults (stage 8), and copying to another
     vault goes through the dialog in 2.6. [decision]
  8. **Duplicate**: a copy of the host; the suffix " Copy" is appended to the name.
  9. **Copy Link**: a link to the host in the app's own URL scheme
     (`shellpp://host/<id>`). An (i) icon explains who will be able to open the link.
  10. **Remove**: deletion with confirmation and the ability to undo
      the action for a few seconds. [decision]

### 2.5 Import dialog [decision]

- Opened from New host ▸ Import (2.2), from the Hosts empty state (2.3) and from the Import
  button on Known Hosts (10.1). A modal dialog with steps:
  1. **Source**: `~/.ssh/config`, PuTTY sessions (shown only on Windows), CSV, native JSON,
     known_hosts.
  2. **File**: a path field filled with the source's default path (for example
     `~/.ssh/config` or `~/.ssh/known_hosts`) and a button to choose another file.
     PuTTY sessions are read from the Windows registry, so this step is skipped for them.
  3. **Preview**: the records found, each with the status "new", "already exists" or
     "error" (with the reason). Only "new" records are imported.
  4. **Summary**: "added N, skipped M, errors K".
- CSV columns: `label`, `address`, `port`, `username`, `group`, `tags`.
- Import is idempotent. A record is matched by its source key: the source type plus the
  Host alias, the PuTTY session name, or the record's UUID. A record without a source key is
  matched by (address, port, username). Existing records are never overwritten, even if they
  were changed after the previous import.
- `~/.ssh/config`: keys referenced by `IdentityFile` are imported into the Keychain. An
  encrypted key asks for its passphrase (as in 7.3).
- known_hosts: hashed entries and markers are handled as described in 10.1.
- Until all sources exist (D39): PuTTY sessions and CSV are shown disabled; the native JSON
  source goes on in Import Data (1.4, D38) and known_hosts in the import of the Known Hosts
  screen (10.1). For `~/.ssh/config` the preview lists each host with its checkbox (new
  hosts are checked), its status, the reason when it is not imported (ProxyCommand) and the
  options that are not imported; then the key files with their status and, for an encrypted
  one, a passphrase field and "Save the passphrases in the vault"; then the lines that could
  not be read. An imported host keeps what was left out in its notes; a host whose ProxyJump
  names a host that is not imported keeps that hop in its chain as a missing host, with a
  note that says so, so it is not connected (D44), never silently connected directly. The summary adds, when there are any, the hosts left without
  their jump hosts and the commands of the file that were neither run nor imported
  (ProxyCommand, LocalCommand, KnownHostsCommand, Match exec). [decision]

### 2.6 Move or copy to a vault [decision]

- Opened by Collaborate, Move to ▸ and Copy to ▸ when the target is another vault (2.4, 8.2),
  Share this host (3.1) and Share this snippet (9.3).
- Fields: the target vault and the operation (Move or Copy).
- The object is re-encrypted with the target vault's key and becomes a new record with a new
  id. Links to the old record made with Copy Link (2.4) stop working.
- Linked objects (identity, key, startup snippet, jump hosts) are listed, with one choice for
  all of them:
  - **Unlink** (default): the linked objects stay where they are, and the moved or copied
    object no longer references them;
  - **Copy together**: the linked objects are copied to the target vault. A warning says that
    every member of the target vault, viewers included, will be able to read their secrets.
- A host in a team vault may have no credentials. Each member attaches their own SSH ID or
  identity to it in the host panel. This binding is stored in the member's personal vault and
  is not visible to other members (see 12.3).

---

## 3. Host panel: view, create, edit

The same form is used for creating and editing; it opens in the details panel.

### 3.1 Blocks from top to bottom

1. **Vault**: a dropdown for where the host is stored (personal or team vault).
2. **Address**: OS icon + "IP or hostname" field. On creation, focus is placed here immediately.
3. **General**:
   - Label;
   - Parent Group: group selection with search and creation of a new group on the fly; [decision]
   - Tags: tags are shown as chips with a delete button; input with autocomplete
     and creation of new tags on the fly;
   - Color: one of the 8 tile colors (`--tag-*`, `docs/design-system.md`, 3.1), graphite by
     default; [decision]
   - Backspace: what the Backspace key sends (default / options); [decision: Default, `^?` (DEL), `^H`]
   - Notes: a multi-line free text field. [decision]
4. **Share this host**: opens the "Move or copy to a vault" dialog (see 2.6). Disabled until
   the host is created. [decision]
5. **SSH**:
   - port: a field on the same line as the protocol name, placeholder 22;
   - Credentials:
     - a hint above the fields if the same login with a password or key is also used
       in N other hosts: a one-link offer to merge them into an identity.
       After the identity is created, all these hosts reference it, and changing the password
       in one place applies to all of them. Shown since stage 3 (D40): for a saved host
       without an identity, when other hosts share its effective username and saved
       password, or its effective username and key; "Create an identity" makes one
       identity (label = the username, plus the password and the key when all of them have
       the same) and moves all of them onto it; a password or key the identity does not
       hold stays on its host; [decision]
     - username;
     - password: masked, with a show/hide button;
     - a linked key is shown as a separate field with the key name from Keychain;
       it can be unlinked;
     - a "+" button to link a key, an identity or a certificate (a key with an OpenSSH
       certificate). Each linked item is shown as its own field and can be unlinked.
       A type that is already linked disappears from the list of options. A FIDO2 key
       joins the list with FIDO2 support (D5) and is hidden until then. [decision]
       For a key, the list offers the existing keys and a short way to add a new one:
       paste the key text or choose a file. An encrypted key asks for its passphrase
       (as in 7.3), and the key is saved to the Keychain; [decision]
     - values inherited from the parent group (username, identity or key, port, Host
       Chaining) are shown as a placeholder "from group X". A value entered in the host
       overrides the inherited one (see 3.4); [decision]
     - in a team vault, a host may have no credentials: each member attaches their own
       SSH ID or identity (see 2.6). [decision]
   - "Show more" expands additional settings, one field per line. Every field below is
     shown and its value is saved; what a field changes in a session arrives with the
     stage that implements that feature (D29):
     - Agent Forwarding: on/off, off by default. While it is on, a `--warning` line says
       that root on the host can sign in elsewhere with the user's keys while a session is
       open, and two checkboxes follow, both off: "Also offer the system agent's keys" and
       "Ask before every signature": each signature the server asks of the forwarded
       agent shows a dialog in the session's tab with the key's SHA256 fingerprint,
       "Deny" and "Allow"; Enter and Esc deny, and no answer (the tab closed, another
       question open, the app locked) refuses. [decision, D60, D66] While the question
       waits (at most two minutes), its tab shows a key in `--warning` next to the state
       dot, also when another tab or screen is shown; when the app's window is not
       focused, a system notification names the host ("web-1 asks for a signature",
       nothing of the key), and a click on it brings the window and the tab forward. The
       answer takes both away. [decision, D68]
     - Startup snippet with `{{name}}` variables: after each login the session's tab asks
       their values in the form of the Snippets screen (9.4) and then runs it; Cancel skips
       it for that login. [decision, D66]
     - Startup snippet: a snippet that runs right after login;
     - Host Chaining: a chain of intermediate hosts (jump hosts);
     - Proxy: HTTP or SOCKS proxy for the connection;
     - Environment Variables: environment variables for the session;
     - Charset: encoding, UTF-8 by default;
     - Keepalive: interval in seconds; empty means the global value (see 12.4); [decision]
     - Record history: As in settings / On / Off, "As in settings" by default
       (see 4.2, 12.4); [decision]
     - Record output: As in settings / On / Off, "As in settings" by default
       (see 11.2, 12.6); [decision]
     - Mosh: on/off, off by default. Shown, although Mosh connections come in a later
       stage (D29);
     - terminal theme for this host: a preview thumbnail + name; clicking it
       opens theme selection. The theme set is our own; dark theme by default.
6. **Add Telnet**: add a second protocol with its own settings to the same host: a Telnet
   block (port 23 by default) that can be removed again. Its values are saved; Telnet
   connections come in stage 9 (D29).
7. The **Connect** button is pinned to the bottom of the panel. Disabled until the address is filled in.
   Alt + Connect opens the session in a new window (2.4). [decision]

### 3.2 Behavior

- The saving rule from 1.3 applies. The only required field of a host is the address. [decision]
- Changes are saved automatically; there is no separate save button.
- A new host appears in the list as soon as a valid address is entered. [decision]
- If the address is empty and the panel is closed, the draft is discarded without prompting. [decision]

### 3.3 "…" menu in the host panel header

1. Connect.
2. Add Telnet.
3. Duplicate.
4. Remove.

### 3.4 Group panel [decision]

- Opens in the details panel when a group is created (New Group, 2.2) or edited (Edit in the
  group menu). The saving rule from 1.3 applies; the only required field is Name.
- Blocks from top to bottom:
  1. **Vault**.
  2. **Name**: required.
  3. **Parent Group**: group selection with search; empty means the top level.
  4. **Color**: one of the 8 tile colors (`docs/design-system.md`, 3.1), graphite by default.
  5. **SSH**: username, identity or key, port, Host Chaining.
- Inheritance: the values of the SSH block apply to the hosts and subgroups inside the group.
  A host or subgroup without its own value uses the value of the nearest parent group that
  sets one. In host and group forms, an inherited value is shown as a placeholder
  "from group X", where X is the group the value comes from.
- Group menu (right-click on a group card, and the "…" menu in the panel header): Open, Edit,
  Move to, Remove.
- Removing a group asks for confirmation. If the group is not empty, the dialog says that its
  subgroups and hosts will move one level up; the group is removed after that.

---

## 4. Terminal session

### 4.1 Session window

- The terminal occupies the entire area below the top bar. There is no Vaults sidebar
  and no borders, only a small inner padding.
- The terminal theme from the host settings, or the default theme, is applied.
- While the session connects, the tab shows the connection screen (see 4.8). [decision]
- After connecting, only what the server sends (MOTD, etc.) is displayed,
  without the application's own banners.
- Input focus is in the terminal immediately.

### 4.2 Terminal sidebar

- Opened and hidden with a button in the top bar (see 1.1). The sidebar is on the right;
  the terminal narrows rather than being overlapped.
- Each window has one terminal sidebar. It acts on the session of the active pane (4.7)
  of the active session tab, and follows when another tab or pane becomes active.
  [decision]
- The sidebar is 320 px wide (`--width-details-min`, `docs/design-system.md`, 6).
  [decision]
- At the top there is a row of icon tabs: Tools, Snippets, History, Appearance.
  The name of each tab is in its tooltip. [decision on the tooltip]

**Tools tab:**

- search in terminal output (Ctrl+Shift+F) with "previous" and
  "next match" buttons. Matches are highlighted in the terminal;
  - Ctrl+Shift+F opens the sidebar on the Tools tab and puts focus in the search field.
    [decision]
  - The search is for plain text, case-insensitive, over the screen and the scrollback.
    All matches are highlighted with the theme's `searchMatch` color and the current one
    with `searchMatchActive` (`docs/design-system.md`, 7.2). A counter shows "N of M".
    Enter goes to the next match and Shift+Enter to the previous one, like the buttons;
    the current match is scrolled into view. [decision]
  - The search runs as you type, and the buttons wrap around at the first and the last
    match. At most 10,000 matches are kept; beyond that the counter shows "10000+". A word
    broken by a line wrap is found; text hidden by the program (SGR 8) is not. Esc in the
    field returns focus to the terminal, and hiding the sidebar removes the highlights.
    [decision]
  - While the search is open, new output is searched again at most every 250 ms, and the
    counter follows. The current match stays on its text: when the scrollback is full and
    its oldest lines drop off, its number goes down instead of the highlight moving to
    newer text; when its own line drops off, the oldest match left becomes current. When
    the terminal changes size (a resize, the sidebar, a split), the lines rewrap: the
    search runs again at once and then at most every 250 ms while the size keeps
    changing, the current match keeps its number, and until that search the highlights
    are hidden rather than shown on the moved text. [decision]
- Autocomplete: a dropdown of modes, off by default.
  Command suggestions are built from the local history of this host. **Later stage**,
  marked as beta in the UI. Until that stage it is not shown (section 0). [decision on
  the suggestion source]

**Snippets tab** [decision]:

- list of snippets with search;
- clicking inserts the snippet into the terminal without executing it;
- the "run" button inserts the snippet and immediately sends Enter.
- the values of `{{name}}` variables are asked first, in the form of the Snippets
  screen (9.4);
- both go as injected input, never recorded in history (`docs/security.md`, 10.1). A
  script of several lines goes as one bracketed paste, so the shell holds it until
  Enter; when the shell has no bracketed paste, each line would run as it arrives, so
  only "run" takes such a script and a click says so. [decision, D66]

**History tab:**

- history search;
- list of entered commands, newest first. Clicking inserts the command into the terminal
  without executing it; [decision]
  - the inserted text is marked as injected input and is never recorded in history again
    (`docs/security.md`, 10.1); [decision]
  - each entry can be deleted on its own, and "Clear history" removes the whole history
    of the host after a confirmation; [decision]
- a hint card: the three most frequent commands with an offer to save them
  as snippets. Commands can be edited before saving; there are "Close"
  and "Save" buttons. A dismissed hint is no longer shown for this
  host; [decision]
  - each of the three commands is checked again by the secret filter below before it is
    offered; [decision]
  - "Save" makes a snippet of each (edited) command, targeting this host, and the hint is
    not shown again for it; "Close" only hides it for good; [decision]
  - trivial commands are never offered: one simple command (no `;`, `&&`, pipe or
    redirection) of `cd`, `ls`, `ll`, `la`, `l`, `dir`, `pwd`, `clear`, `cls`, `reset`,
    `exit`, `logout`, `history`, `whoami`, `top` or `htop`; [decision, D64]
- layout and states of the tab [decision]:
  - the trash button next to the heading is "Clear history" (inactive while the list is
    empty or filtered by a search). Each row shows the command on one line in the terminal
    font, with its own delete button. A command run again shows once, at its newest
    use, and deleting the row deletes every repeat (D64). The list shows the newest 500
    commands; the search filters them as you type (plain text, case-insensitive);
  - the hint card is above the search field. It appears when the host has at least one
    command; commands are ranked by how often they were run and, at equal counts, by the
    latest use;
  - for a quick connect or a local terminal the tab says that history is kept for saved
    hosts only. When recording is off for the host (12.4, 3.1), a line says so and the
    stored entries can still be inserted and deleted;
  - a command is read when the cursor leaves its line after Enter, so the echo of keys
    typed just before Enter on a slow link is included. When keys are typed before the
    previous line ended on screen, neither line is recorded (`docs/security.md`, 10.1);
- **Mandatory security requirements:**
  - a command is the logical line in the terminal buffer at the moment Enter is pressed,
    without the prompt. The prompt ends at the cursor position where the first character of
    the command was typed; [decision]
  - nothing is recorded in history [decision]:
    - in a session without a saved host (a quick connect, 2.1) and in a local terminal:
      history is kept only for saved hosts;
    - in the alternate screen (full-screen programs such as vim or htop);
    - while the server does not echo the typed characters;
    - after a line of output that looks like a prompt for a password, passphrase, PIN or
      one-time code (`password:`, `passphrase`, `PIN`, `OTP`), for example from `su`,
      `sudo`, `passwd` or a key passphrase prompt;
  - detecting commands and prompts this way is a heuristic with known limits. Prompt
    marking with OSC 133 is not in the MVP; [decision]
  - strip bracketed paste control sequences (`ESC[200~`,
    `ESC[201~`) and other escape codes. History stores only the clean text of the
    command;
  - commands that look like secrets are not stored at all. A command looks like a secret
    if it contains `-p<value>` (a value right after `-p`), `--password`, `*PASSWORD=`,
    `TOKEN=`, `SECRET`, `API_KEY`, `user:pass@`, `Authorization:` or `Bearer`, a known token
    prefix, or a high-entropy string of 20 or more characters. The same filter applies when
    shell history is imported (see 12.4); [decision]
  - history is stored encrypted, like the rest of the data. History sync
    between devices is off by default;
  - history recording is set globally (see 12.4) and can be overridden for an individual
    host (Record history, see 3.1). Any entry can be deleted, and the entire history of a
    host can be cleared; [decision on the global setting]
  - the "save as snippets" hint never suggests entries that look
    like secrets.

**Appearance tab:**

- the Font row with an arrow leads to font settings: family, size,
  line spacing; [decision]
  - the family list has the bundled JetBrains Mono and the installed monospace fonts;
    line spacing is 1.2 by default. These are the same settings as the Font block of
    Settings → Terminal (12.4), and a change applies to every terminal; [decision]
- Themes list: each theme has a preview thumbnail in its colors and a name.
  The current theme is highlighted. Clicking immediately applies the theme to the session and saves it
  in the host settings. Imported themes (see 12.4) are in the same list; [decision]
  - a quick connect and a local terminal have no host record, so for them the click
    applies the theme to that session only; [decision]
- there are no theme popularity counters, because they would require telemetry.
  Themes added in the latest update are marked "new"; [decision]
- theme set: our own dark and light defaults plus popular
  open color schemes with compatible licenses. Check the license of each scheme
  and list it in THIRD_PARTY_NOTICES.
  - The list shows Graphite and Paper first, then the bundled schemes: Dracula, Nord,
    Solarized Dark, Solarized Light, Gruvbox Dark, Tokyo Night, Catppuccin Mocha and
    Catppuccin Latte (`docs/design-system.md`, 4.4). [decision]
  - The bundled schemes keep their original colors; the "Adjust indistinguishable colors"
    setting (12.4) helps where a scheme has low-contrast pairs. [decision]

### 4.3 Mouse in the terminal

- Right-click pastes text from the clipboard. Multi-line paste
  requires confirmation (see SPEC.md).
- If copy-on-select is enabled, selecting with the mouse copies the text immediately. [decision]

### 4.4 Session tab context menu

1. Duplicate: a new session to the same host in a new tab.
   On a tab with splits, the host is the active pane's host. A local terminal is
   duplicated as a new local terminal with the same shell, and a quick connect as a new
   connection to the same `user@host:port`. The new tab opens right after the
   duplicated one. [decision]
2. Duplicate in a new window: the same, but in a new application window (1.4).
3. Start multiplayer: real-time collaboration in a session.
   **Later stage.** Works via a self-hosted server, with E2E encryption,
   by invitation. The invitee sees the output and can enter commands only
   with the session owner's permission. [decision]
   Until that stage the item is not shown (section 0). [decision]
4. Rename: rename the tab. The host itself is not renamed.
   The title is edited in place on the tab: Enter saves, Esc cancels, and an empty title
   brings back the host name. Leaving the field (a click elsewhere) saves like Enter. The
   title stays for the lifetime of the tab and is saved for Restore (6). [decision]
   The field takes at most 256 characters and no control characters (a pasted tab).
   Saving the title unchanged gives the tab no title of its own: it keeps showing the
   host of its active pane (4.7). Enter and Esc put the focus back into the tab's pane; a
   click elsewhere leaves it where the click put it. While the field is open, a pane does
   not take the focus on its own (a session that connects meanwhile): it does so after
   the field closes. [decision]
5. Split horizontally: a new pane below the current one (Ctrl+Shift+E, see 4.7). [decision]
6. Split vertically: a new pane to the right of the current one (Ctrl+Shift+D,
   see 4.7). [decision]
7. Broadcast input: shown only on a tab with splits (see 4.7). [decision]
   Until stage 9 it is shown disabled (section 0). [decision]
8. Close: closes the tab with all its panes. The next active tab follows 4.6. [decision]

Items that do not apply stay in place, disabled: Duplicate, Duplicate in a new window
and the splits while the active pane has no session target (the new tab page, a pane
still choosing a host, SFTP), and a split that the limits of 4.7 do not allow. [decision]
The split shortcuts (Ctrl+Shift+E, Ctrl+Shift+D) do nothing in those cases either.
[decision]

### 4.5 Workspaces

- Workspace: a saved set of tabs and splits with linked hosts.
  Opening a workspace restores the tabs and layout and connects
  to the hosts.
- Add a host: host context menu → Add to Workspace ▸ → a list of
  existing workspaces and a New Workspace item (create it and immediately add the host).
  [decision on the list of existing ones]
- The current set of tabs can be saved as a workspace with a button on the
  new tab page (see section 6).
- Workspaces are shown as a separate block on the new tab page and can be found
  via Ctrl+K search. [decision]

### 4.6 Session termination and connection drop

- **Normal termination** (`exit`, `logout`, the server closed the channel with code 0):
  the tab closes by itself.
- **Non-zero exit code** [decision]: the session has ended, not dropped. The tab does not
  close, the output is preserved, and the message "Session ended with code N" with the
  "Reconnect" and "Close" buttons is shown over the terminal. Automatic reconnection does
  not start.
- **Unexpected drop**: only a transport error, such as a network failure, a keepalive
  timeout, or a connection closed without an exit code [decision]:
  - the tab does not close, and the output is preserved;
  - the terminal is dimmed and its cursor is hidden until the session reconnects (1.1);
    [decision]
  - a message with the reason and "Reconnect"
    and "Close" buttons is shown over the terminal;
  - if Autoreconnect is on (see 12.4, on by default), reconnection attempts start
    automatically with delays of 1, 2, 4, 8, 16 and 32 seconds, and then every
    60 seconds. The message shows a countdown to the next attempt and a "Cancel" button
    that stops the automatic attempts; [decision]
  - while the app is locked, reconnection waits for unlock (see 13.3). [decision]
- **Next active tab** [decision]: when the active session tab closes for any reason
  (normal termination, "Close", the tab's close button, Ctrl+Shift+W), the neighboring
  session tab becomes active: the one on the left, or, if there is none, the one on the
  right. The persistent Vaults and SFTP tabs do not count as neighbors. If no session tabs
  remain, Vaults opens.
- **Splits** [decision]: on a tab with splits, these rules apply to each pane (4.7). A
  normal termination closes the pane, and the message of a non-zero exit code or a drop
  is shown over that pane's terminal. The tab closes, by the rule above, when its last
  pane closes.
- **Exit confirmation** [decision]: Exit (1.4) and closing a window while sessions, port
  forwards or file transfers are active ask for confirmation first.

### 4.7 Splits [decision]

- A tab can be split into panes horizontally (a new pane below, Ctrl+Shift+E) and
  vertically (a new pane to the right, Ctrl+Shift+D): via the tab menu (4.4) or with
  keyboard shortcuts (configurable, see 12.5). [decision on the shortcuts]
  - A split divides the active pane into two equal parts, and the new pane becomes
    active. Any pane can be split again in either direction, so a tab holds a tree of
    panes. [decision]
- A new pane shows host selection as on the new tab page (section 6): a search field and
  Recent connections. Choosing a host connects in that pane. [decision]
  - It is a compact version of the page: a search field with focus in it and one list.
    The first row is the host of the pane that was split, so Enter opens another session
    to it; for a quick connect it is the same address, and for a local terminal a new
    local terminal with the same shell. The first row is not repeated among Recent
    connections below it. Typing filters the list as the page's search does, without open
    tabs, since the pane needs a new session. Esc closes the new pane, which undoes the
    split. [decision]
  - Alt held while choosing (2.4) opens the session in this pane as well, not in a new
    window: the pane is waiting for a session. [decision]
- The border between panes can be dragged. The active pane is highlighted.
  Focus is switched by clicking or with Ctrl+Alt+arrows (see 12.5).
  - Ctrl+Alt+arrow moves focus to the nearest pane in that direction on screen. At the
    edge of the tab it does nothing. [decision]
  - The border is a 1 px `--border-subtle` line that can be grabbed within `--space-2`
    around it. Each pane keeps at least 128 px in width and in height (four times
    `--space-6`; `docs/design-system.md` names no pane size). [decision]
  - With more than one pane, the active pane has a 2 px `--accent` bar along its top
    edge (`--size-tab-indicator`, as on the active tab). [decision]
  - A tab holds at most 16 panes, and a pane is split only while both halves keep the
    minimum size and the pane tree stays within the four levels of nesting that Restore
    keeps (`docs/data-model.md`, 16.3; up to four panes side by side in one direction count
    as one level). [decision]
- Each pane has its own session, to the same host or to a different one.
- **Closing panes** [decision]:
  - when a pane's session ends normally (exit code 0), the pane closes and its neighbors
    take its space. A non-zero exit code and a drop show their message inside the pane
    and keep it open (4.6). The "Close" button of that message closes the pane;
  - Ctrl+Shift+W closes the active pane when the tab has more than one pane, and the tab
    otherwise. Close in the tab menu closes the whole tab (4.4);
  - when the last pane closes, the tab closes as described in 4.6.
- **A tab with splits** [decision]: its title is the host of the active pane, unless the
  tab was renamed (4.4). Its state dot (1.1) shows the worst state of its panes: a drop
  first, then connecting, then an ended session, then connected.
- Restore (section 6) brings the panes back with their sizes; the first pane of each tab
  is active. [decision]
- **Broadcast input** [decision]: turned on and off with the "Broadcast input" item in the
  tab menu (4.4). While it is on, keyboard input in the active pane is also sent to all
  other panes of the tab, and these receiving panes are marked with a border. Until
  stage 9 the item is shown disabled (section 0).

### 4.8 Connecting and authentication [decision]

- **Connection screen.** While a session connects, the tab (or the split pane) shows a
  screen instead of the terminal: the host name and address, the current step (for example
  connecting, checking the host key, authenticating, opening the session) and a "Cancel"
  button. The status indicator shows "connecting" (`docs/design-system.md`, 3.3). "Cancel"
  stops the attempt, and the screen switches to the failure state described below.
- **Host key.** At the host key step, the dialogs from 10.2 appear when needed.
- **Prompts.** When a value is needed, a form appears on the connection screen:
  - username: when neither the address, the host, its group nor its identity sets one
    (for example, a quick connect to a bare `host`, see 2.1);
  - password;
  - key passphrase: when the key is encrypted and its passphrase is not saved (see 7.3);
  - keyboard-interactive: the server's questions are shown one at a time, with the
    server's name and instruction text above them. An answer is masked when the server
    marks the question as not echoed. One-time codes (2FA) are asked this way.
- Each form has "Continue" (Enter) and "Cancel" buttons. The password and passphrase forms
  have a "Save" checkbox, off by default. When it is checked, the value is saved in the
  encrypted storage for this host or key. For a quick connect without a saved host, the
  checkbox is not shown.
- When a prompt is needed outside a terminal tab (an SFTP panel, a forwarding rule, a
  snippet run with a results table), the same form opens as a modal dialog.
- A wrong password or passphrase shows an error in the same form, and the user can try
  again until the server closes the connection.
- **Failure.** If the connection cannot be established (the address is not found, the
  connection is refused or times out, authentication fails, the host key is rejected),
  the screen shows the reason with "Retry" and "Close" buttons. Automatic reconnection
  (4.6) does not apply to the first connection attempt.

---

## 5. SFTP manager (SFTP tab)

### 5.1 Layout

- Two panels of equal width. Each panel is an independent file source:
  the local computer or a remote host.
- By default, the left panel shows the local file system and the right panel is empty.
- **SFTP session tabs** [decision]: "Connect via SFTP" (2.4) opens a new session tab with
  the same dual-pane manager: Local on the left, the host on the right. The persistent SFTP
  tab (1.1) stays for manual work and is not changed.
- Every SFTP tab has the transfer queue panel at the bottom (see 5.6). [decision]

### 5.2 Empty panel

- In the center: a folder icon, a heading prompting the user to connect to a host,
  one line of explanation and a host selection button.
- The button opens a list of saved hosts with search. [decision]

### 5.3 Panel with files

- Header:
  - the source icon and name (Local or the host name). Clicking it lets you
    change the source; [decision]
  - on the right, Filter: a filter by name in the current folder;
  - the Actions menu (see 5.5).
- Navigation: back and forward buttons, the path as breadcrumbs. On Windows the path
  starts with the drive letter, and clicking the letter opens drive selection. [decision]
  Clicking any path segment navigates to that folder.
- A table with the columns Name, Date Modified, Size, Kind:
  - sorting by clicking a column header; the active column shows an arrow
    indicating the sort direction;
  - column widths are changed by dragging. [decision]
- The first row ".." goes up one level.
- For folders, the size is not shown (a dash), and the type is "folder".
- File row:
  - icon and name;
  - below the name, in a small font, the access permissions in the `-rw-r--r--` format
    (only in the remote panel); [decision]
  - size in human-readable units: bytes, KB, MB, GB;
  - Kind: the file extension, or "file" if there is no extension.
- The selected row is highlighted. Multiple selection via Ctrl/Shift. [decision]
- Dates are displayed in the system locale format.
- Double-clicking a folder opens it. Double-clicking a file performs
  the Open action (see 5.4). [decision]
- Dragging files between panels copies them through the transfer queue
  (see 5.6). Name conflicts are handled as described in 5.7. [decision]

### 5.4 File context menu (right-click)

1. Open: open in the program set in "Program for Open" (see 12.6) or, if it is empty, in
   the program the system assigns to this file type. [decision]
   A remote file is downloaded to a temporary folder, changes are tracked
   and uploaded back to the server, and a notification is shown for each upload (see 1.5). [decision]
   The temporary copy is not encrypted, so Open shows a warning about it. The copy is
   deleted when the file is closed and when the app exits. [decision]
2. Open with…: the same, but with a choice of program.
3. Copy to target directory: copy the selection to the current folder
   of the opposite panel.
4. Rename.
5. Delete: shown in the danger-action color, performed after confirmation
   (the confirmation can be turned off, see 12.6). [decision]
6. Refresh.
7. New Folder.
8. Edit Permissions: access permissions dialog (see below).

Edit Permissions dialog [decision]:

- rwx checkboxes for the owner, group and others;
- a synchronized field with the octal value;
- for folders there is an "apply recursively" option.

### 5.5 Actions menu in the panel header

The same items as in the context menu (they act on the selection), plus:

- Show Hidden Files: a toggle for showing hidden files (off by default, see 12.6); [decision]
- Select All;
- Close: close the connection in this panel (danger-action color).
  The panel returns to the empty state from 5.2.

Items that require a selection are disabled without one. [decision]
In the local panel on Windows, the Edit Permissions item is hidden. [decision]

### 5.6 Transfer queue [decision]

- A collapsible panel at the bottom of an SFTP tab. It appears when the first transfer is
  added. When collapsed, it shows only a header with the number of active transfers.
- One row per transfer: file or folder name, direction (upload, download, or remote to
  remote), progress, speed.
- Row buttons: "Pause" and "Resume" for an active transfer, "Cancel", and "Retry" for a
  failed one.
- At most N transfers run at the same time (see 12.6, 3 by default); the rest wait in the
  queue.
- Finished transfers stay in the list until they are removed with a "Clear finished" button.

### 5.7 Name conflict dialog [decision]

- Shown when a copied file has the same name as a file in the target folder and the action
  on name conflicts is "Ask" (see 12.6, the default).
- Shows the file name and, for both files, the size and the modification date.
- Buttons:
  - Replace: overwrite the file in the target folder;
  - Skip: do not copy this file;
  - Rename: keep both; the copied file gets a free name with a number, for example
    `report (1).txt`.
- The "Apply to all" checkbox applies the choice to the remaining conflicts of the same
  operation.

---

## 6. New tab page

- Opened with the "+" button in the top bar, with Ctrl+T outside the terminal and with
  Ctrl+Shift+T everywhere (see 12.5). [decision on the shortcuts]
- Until a host is selected, the tab is named "New Tab". After selection, the session
  opens in the same tab. [decision]
- At the top center, a search field for hosts and open tabs with a hint for the
  Ctrl+K key; focus is placed in it immediately. Ctrl+K from anywhere in the app
  opens the same search; when the terminal has focus, Ctrl+Shift+K does (see 12.5).
  [decision on the global Ctrl+K and on Ctrl+Shift+K]
  - Ctrl+K and Ctrl+Shift+K open the search as an overlay over the current window, not
    as a new tab. Esc or a click outside it closes it, and the focus goes back where it
    was; arrow keys and Enter choose a result. [decision]
  - The search covers the open tabs of all windows and the saved hosts. Choosing an
    open tab brings its window to the front and activates that tab. [decision]
  - Choosing a host or an address in the overlay opens the session in a new tab. With an
    empty field the overlay lists Recent connections. [decision]
- Below it, the Recent connections block:
  - row: OS icon, host name (or address), and on the right the path "vault / group";
  - clicking connects in this tab;
  - arrow keys and Enter work the same as a click. [decision]
  - The rows come from the connection log (section 11): SSH connections, newest first,
    one row per target (a saved host, or the address and port of a quick connect), at
    most 10. A row whose host has been removed connects to its address like a quick
    connect. While the log is empty, the block says that the hosts you connect to appear
    there. [decision]
- The block header has two buttons:
  - Create a workspace: save the currently open tabs as a workspace;
    until stage 9 it is shown disabled, with a tooltip (section 0); [decision]
  - Restore: restore the tabs from the previous app launch. Disabled
    if there is nothing to restore. [decision based on meaning]
    - Restore brings back the windows of the previous launch with their tabs, panes
      (4.7) and titles set with Rename (4.4), and connects their sessions. The first saved
      window is restored into the current window, and the others open as new windows.
      A saved host that has been removed since is not opened; when nothing can be
      opened, a notification says so. Restore works once per launch, and the "New Tab"
      tab it was chosen on gives way to the restored tabs. [decision]
    - The app saves this set when it exits, when its last window closes and before it
      locks (the set is encrypted with the vault's keys, so a quit while locked keeps the
      set saved at the lock). It holds
      only the structure and the targets: host record ids, `user@host:port` of quick
      connects, local shell ids, tab titles, and the pane tree with its sizes. Secrets and
      terminal content are never saved. The set is stored encrypted on this device and is
      not synced (`docs/data-model.md`). [decision]
- When typing in the search, the list is replaced with results: open tabs first,
  then hosts. [decision]
  - Open tabs match by their title or by their host's name, address, username or tags;
    hosts match by the same fields as the Hosts search (2.1). A host whose label or
    address is exactly the text comes first among the hosts. [decision]
  - When the text is recognized as a connection address by the rules of 2.1 and no host
    has exactly that label or address, the last row connects to it without saving the
    host. [decision]
  - Alt held while choosing a host or an address opens the session in a new window
    (2.4). Choosing an open tab only brings it to the front. [decision]
- Layout [decision]:
  - the page's column, the pane picker (4.7) and the overlay are at most 560 px wide
    (`--width-details-max`); the page starts `--space-6` below the tab row, and the
    overlay `--space-6` below the top bar over the `--scrim` dimming, with at most 10 rows
    visible before the list scrolls;
  - a row is 44 px (`--height-row`): a 28 px tile in the host's color (as in the Hosts list
    layout), the name and, on the right, "vault / group". A Recent connections row
    without a saved host shows its address on the right, without the port when it is 22.
    The selected row has the `--accent-subtle` fill. Section headings ("Recent
    connections", "Open tabs", "Hosts") are 28 px rows with a 12 px caption;
  - arrow keys stop at the first and the last row. When the data changes (a new
    connection log entry, a host edited elsewhere), the selection stays on its row only
    if the user moved it; otherwise the first row is selected.

---

## 7. Keychain screen

### 7.1 Action bar

On the left:

- **New key** split button. The main part opens, in the details panel, a form
  for adding an existing key: paste text, choose a file, or drag a
  file into the form. [decision on drag & drop] The arrow opens a menu:
  1. **Generate key**: generate a new pair. Parameters: type (Ed25519
     by default, ECDSA P-256, P-384 or P-521, RSA 3072 (the default length) or 4096;
     RSA 2048 is not offered, D32), label, optional passphrase. [decision on parameters]
     The passphrase field has a show/hide button; once a passphrase is typed, "Repeat
     passphrase" appears, and Generate refuses two different values ("The passphrases do
     not match."). [decision, D64]
  2. **New Identity**: create an identity.
- **Certificate**: add an SSH certificate.
- **FIDO2**: add a hardware key (`sk-ssh-ed25519`, `sk-ecdsa`). [decision on types]
  It opens the New key form for the private key file that `ssh-keygen -t ed25519-sk`
  made (a handle; the key stays on the security key). [decision, D64]
- **Certificate** opens "Add a certificate": the `*-cert.pub` text, pasted or chosen, goes
  onto the key whose public key it certifies, and that key's panel opens; a certificate of
  no key in the Keychain is refused with that reason. The key panel shows a key's
  certificate (key ID, principals or "Any user", the validity window, the CA fingerprint),
  a `--danger` line when it has expired or is not valid yet, and "Remove certificate".
  [decision, D65]
- While a part is not built, its button says so in a tooltip: what does not work yet and
  when it arrives (FIDO2: creating a key on a security key and signing with it come in a
  later stage). [decision, D64]

On the right: search, a tile/list toggle, a sort menu (by name,
by creation date, by modification date). The same sort icon is used
on the Hosts screen. [decision]

- The search takes the room left, from 200 to 320 px. When the bar cannot hold it at
  200 px next to the labelled buttons (an open details panel narrows it), Certificate and
  FIDO2 show only their icons, with their names in the tooltips; only when that is not
  enough either does the right group wrap below. [decision, D64]

### 7.2 Content

- The **Keys** section, with **Identities** below it. The card grid is the same as
  on the Hosts screen.
- Key card: icon, label, subtitle "Type" + type (ED25519, RSA,
  ECDSA, …). If the key could not be parsed, the type is "unknown" with a warning
  icon, and the reason is shown in the panel. [decision on the warning]
- Identity card: icon, name, subtitle with the authentication method
  (password, key or both). If nothing is set, it says exactly that: no authentication
  method.
- Identical keys (matching fingerprint) are detected automatically. A hint
  appears offering to merge them into one key; references from hosts are carried over. [decision]
- Empty state: buttons "Generate", "Import" and "Import from SSH folder" (the path
  `~/.ssh` in its tooltip). [decision] "Import from SSH folder" opens the New key form
  with the private key files found in `~/.ssh`; a click on one puts its text into the
  form (D33).

### 7.3 Key panel (creation and editing)

Blocks from top to bottom:

1. Vault.
2. Label, a required field.
3. Private key, a required field, multi-line, monospace font.
4. Public key, multi-line.
5. Certificate, multi-line.
6. Key export: a "?" icon with an explanation and an **Export to host** button.
   The user selects one or more hosts, and the public key
   is appended to `~/.ssh/authorized_keys` without duplicates (equivalent to `ssh-copy-id`).

The "…" menu in the header: Duplicate, copy public key, export
to file, Remove. [decision] Remove asks first ("Remove key?", Cancel and Remove in
`--danger`); when hosts, groups or identities use the key, the confirmation lists them
and warns that they lose it and connect with their other sign-in methods until another
key is set. After Remove a toast offers Undo for the undo window of 2.4; the users keep
a missing reference (data-model 3.4). [decision] (D50)

The saving rule from 1.3 applies, except for the creation: a new key is created with the
form's Import or Generate button once Label and Private key are valid, because creating
a key runs a key derivation or a key generation and settles the passphrase choice
(D33). After that, Label is saved as in 1.3. [decision]

**Mandatory security requirements:**

- The private key contents are **hidden** by default. Instead of the text, the
  type, fingerprint and a note that the key is hidden are shown.
- The key can be shown only with an explicit button. When the user leaves
  the panel, the key is hidden again.
- Copying the private key is done with a separate button. The clipboard
  is cleared after 30 seconds. [decision on the timeout]
- If the Public key field is empty, the public key is derived from the private key
  automatically.
- The SHA256 fingerprint is shown for each key: one line in the monospaced font, with a
  copy button; a narrow panel cuts it in the middle (`SHA256:nTM…j9e5Q`), never at a
  `/`. The same applies to the Keychain list and to Known Hosts (10). The TOFU and
  changed-key dialogs show the whole fingerprint for comparison. [decision, D64]
- A public key pasted or chosen instead of the private key (an OpenSSH `.pub` line, an
  RFC 4716 or a PEM public key) is named as such: "This is a public key. Import the
  private key: the file of the same name without .pub." [decision, D64]
- A key without a passphrase is marked with an unobtrusive hint "not protected
  by a passphrase". When exporting to a file, the user is offered to set a passphrase.
- The passphrase of an encrypted key is requested on import. The user
  chooses: save it in the encrypted storage or enter it on every
  connection (the prompt is described in 4.8).

### 7.4 Identity panel [decision]

- Fields: label, username, password, linked key or certificate.
- At the bottom, a list of hosts that use this identity. Clicking one opens the host.

---

## 8. Port Forwarding screen

### 8.1 Action bar

- **New forwarding** split button: the main part launches the wizard
  (8.3). The arrow opens a menu: create Local, Remote or Dynamic directly
  without the wizard. [decision]
- On the right: search, tile/list toggle, sorting.

### 8.2 Content

- A section with the list of rules as a grid of cards.
- Rule card:
  - a square badge with the type letter: L, R or D;
  - label;
  - a one-line subtitle, for example `127.0.0.1:8080 → db:5432 via <host>`; [decision]
  - a status indicator: running / stopped / error. [decision]
- A single click shows details in the details panel. A double click starts
  or stops the rule. [decision]
- Right-click [decision]: Start/Stop, Edit, Duplicate, Move to, Copy to, Remove.
  Move to and Copy to another vault go through the dialog in 2.6. [decision]

### 8.3 Wizard

**Step 1: choosing the type**

- Vault.
- Local / Remote / Dynamic segmented toggle.
- Below it, a traffic direction diagram for the selected type. The diagram is our own;
  participants: this computer, firewall, SSH host, target service or
  the internet. Below that, 1–2 sentences of explanation in our own words:
  - Local: a port reachable on the remote side becomes reachable on this
    computer as if it were local;
  - Remote: a port is opened on the remote machine, and connections to it
    are forwarded to this computer;
  - Dynamic: the application acts as a local SOCKS proxy, and all connections
    through it go via the selected SSH host.
- **Continue** (next step) and **Skip wizard** (straight to the full form, 8.4) buttons.

**Next steps** [decision]: one question per screen; you can go back
at any step.

- Local:
  1. via which host;
  2. local address and port (default address 127.0.0.1);
  3. destination address and port (as the SSH host sees them);
  4. label and summary.
- Remote:
  1. host;
  2. address and port on the remote side. Default 127.0.0.1; when
     0.0.0.0 is selected, a warning and a hint about `GatewayPorts` are shown;
  3. address and port on this computer;
  4. label and summary.
- Dynamic:
  1. host;
  2. local SOCKS address and port (default 127.0.0.1:1080);
  3. label and summary.
- The last step shows the rule in one line and the "Create" and
  "Create and start" buttons.

### 8.4 Full form (Skip wizard and editing) [decision]

- All wizard fields on one screen.
- Additionally: start the rule automatically when connecting to the host,
  start on application launch. While the app is locked, autostart waits for
  unlock (see 13.3). [decision]
- If the local port is busy, a clear error with the port number and a suggestion
  to choose another one.
- The default bind address is always 127.0.0.1. Opening the port on all interfaces
  (0.0.0.0) requires an explicit choice and is accompanied by a warning.

---

## 9. Snippets screen

### 9.1 Action bar

On the left:

- **New snippet** split button. The main part creates a snippet; the arrow
  opens a menu with the **New snippet package** item. A package is a folder for
  grouping snippets.
- **Shell History**: a command history screen across all hosts with search, filtering
  by host and a "save as snippet" action. All history security rules
  from 4.2 apply. [decision on the filter]

On the right: search, tile/list toggle, sorting.

### 9.2 Content

- Packages at the top (like groups on the Hosts screen), the Snippets section below. [decision on packages]
- Snippet card: icon, label, and if the label is empty, the first line of the script.
  While a new snippet is a draft (see 1.3), its card shows a placeholder prompting to set
  a label or a script. The snippet is created once the script is filled in. [decision]
- Clicks and the context menu are the same as for hosts. A double click runs
  the snippet on its target hosts. [decision]

### 9.3 Snippet panel (creation and editing)

Blocks from top to bottom:

1. Vault.
2. A hint about generating a script from a description. Shown once,
   dismissed with the "got it" button.
3. Main fields:
   - **Action description**: the snippet label. On the right inside the field, a button
     for generating a script from the description;
   - **Package**: package selection;
   - **Tags**: chips with autocomplete and creation of new tags on the fly, as in the host
     panel (3.1); [decision]
   - **Script**: required, multi-line field, monospace font,
     shell syntax highlighting. `{{name}}` variables are highlighted
     and requested via a form at run time. [decision on highlighting]
4. **Share this snippet**: opens the "Move or copy to a vault" dialog (see 2.6). Disabled
   until the snippet is saved. [decision]
5. **Targets for execution**:
   - a short explanation;
   - a list of target hosts (icon, address or label, protocol);
   - the "Add targets" link opens host selection with search
     and multiple selection.
6. The **Run** button is pinned at the bottom. Disabled until there is a script and at least
   one target.

### 9.4 Running on multiple hosts [decision]

- Any number of targets, one included: a results table. For each host, the
  status (waiting, running, success, error, timed out), exit code and output,
  which expands on click, are shown. The table opens in a dialog over the
  screen (D66). [decision] (D68)
- A tab split into panes, one session per host for up to 4 targets, comes
  with Broadcast (stage 9); until then the table is the only view of a run.
  [decision] (D68)
- Before running on 2+ hosts, a confirmation with the list of hosts is shown.

### 9.5 Script generation from a description [decision, later stage]

- Off by default. Enabled in settings by specifying a provider:
  your own API key for an LLM or a local model.
- The description is sent to the provider only on an explicit press of the
  generate button. The user is explicitly shown where the text will go.
- The generated script is only inserted into the Script field. It is never
  executed automatically.

---

## 10. Known Hosts screen

### 10.1 List

- Action bar: the **Import** button opens the import dialog (2.5) with the known_hosts
  source; `~/.ssh/known_hosts` is suggested by default. On the right: search,
  view toggle, sorting. [decision]
  - The **Export** button stands next to Import. The view toggle switches between tiles
    and a list with the columns Address, Key type, Fingerprint and Date added. Sorting is
    by address, by date added or by key type. [decision]
  - With no entries, the screen shows an empty state with an "Import known hosts"
    button (the path `~/.ssh/known_hosts` in its tooltip). [decision]
- Grid of cards: fingerprint icon and address in OpenSSH format. For port 22
  it is `host`, for others `[host]:port`.
- Hashed entries (`|1|…`) are stored as they are, because their address cannot be
  recovered. Their card shows "Hashed host" and the key type instead of the address. On
  connect, the address is hashed (HMAC) and compared with these entries. [decision]
- `@revoked` entries are imported as revoked keys and marked with a "revoked" badge.
  `@cert-authority` entries are skipped, and the import summary shows how many were
  skipped. [decision]
- A click opens the details panel [decision]: address, port, key type, SHA256
  fingerprint with a copy button, date added, saved hosts with this
  address, Remove button.
  - The panel also shows the comment of an imported line. For a hashed entry the port is
    "in the hash", and a note says that the address is stored as a hash and is still
    checked on every connection. For a revoked entry a note says that every connection
    that presents this key is blocked. [decision]
- Right-click [decision]: copy fingerprint, Remove.
- Keyboard and search [decision]: the arrows move the selection between cards (up and
  down by a row of the grid; in the list layout by one row), Delete or Backspace on the
  selected card opens the Remove confirmation, and Esc closes the details panel. The
  search matches the card title (the address, or "Hashed host"), the key type, the
  SHA256 fingerprint and the comment, case-insensitive. The details panel is 380 px wide
  and can be resized from 320 to 560 px, as the details panel of 1.3.
- Remove asks for confirmation, and the removal can be undone for a few seconds, as for
  hosts (2.4). The confirmation says that the next connection to this address will ask
  again whether to trust its key; for a revoked entry, that the key will no longer be
  blocked. [decision]
- The import summary shows four numbers: imported entries, duplicates of entries already
  in the list, skipped entries (`@cert-authority` lines and negated `!` host patterns) and
  invalid lines. A line with several comma-separated host patterns gives one entry per
  pattern. Comments and blank lines are not counted. A file larger than 16 MiB is refused.
  [decision]
  - A line longer than 64 KiB is invalid, and so is a line with a malformed host pattern
    (the whole line). An entry that appears twice in the file is imported once and then
    counted as a duplicate. The comment of a line is kept, cut to 256 characters, for the
    details panel and is not exported. [decision]
  - Limits against hostile files: a line with more than 256 host patterns is invalid; a
    host pattern repeated on its line counts as a duplicate; one file gives at most
    200,000 entries, holding at most 64 MiB of text (host, key type, key and comment of
    each entry, since every pattern of a line copies its key); a line past either limit
    is invalid. [decision]
  - Until the full import dialog of 2.5 arrives (stage 3), Import on this screen opens a
    dialog with the File step and the Summary only, without the Source and Preview steps.
    [decision]
- The view toggle is remembered (`view.known_hosts.layout`, `docs/data-model.md`); the
  sort is not, and the screen opens sorted by address. In the list layout the Key type
  column is 200 px and the Date added column 148 px wide. The date added is shown as
  `YYYY-MM-DD HH:MM UTC`. Remove in the details panel is a neutral button with text in the
  danger color. [decision]
- Export to the known_hosts format. Hashed entries are exported as they are. [decision]
  Revoked entries are exported with the `@revoked` marker. [decision]
- Connections check host keys only against this list. The system `~/.ssh/known_hosts` is
  not read when connecting; it can only be imported. [decision]

### 10.2 First connection and key change [decision]

- The first connection (TOFU) opens a dialog: address, key type, fingerprint.
  Buttons: "Accept and save" (default), "Connect once",
  "Cancel".
- If the host key has changed:
  - the connection is blocked, a warning is shown in the danger color;
  - the old and new fingerprints and an explanation about a possible attack are shown;
  - continuing is possible only via the explicit "Replace saved key" button;
  - the dialog opens with the keyboard focus on "Cancel", and Enter and Esc press
    "Cancel". "Replace saved key" works only with a mouse click, or with Space after Tab
    has moved the focus to it; Enter on it does nothing; [decision]
  - there are no checkboxes like "don't ask again".
- If the host key is in the list as revoked (10.1), the connection is blocked with a warning
  in the danger color. There is no button to continue. [decision]

---

## 11. Logs screen

Connection log in the form of a table.

### 11.1 Columns

- **Date** with a sort order toggle:
  - session start date;
  - below it, start and end time. If the session ended on the next day,
    "(+1d)" is appended;
  - for an active session, "now" is shown instead of the end time. [decision]
- **User**: avatar and account, device name below it. The client IP address is not
  collected or shown. [decision]
  - In a team vault, the log shows only the entries of members who turned on publishing
    for that vault (see 12.3). [decision]
  - Until shared vaults are implemented (stage 8), the column shows the current
    device. [decision]
- **Host**: OS icon, host name, protocol and user below it.
- **Saved**: bookmark button. Saved entries are not removed by auto-cleanup.
  Clicking the column header shows only saved entries. [decision]

### 11.2 Actions [decision]

- Clicking a row: if the session output was recorded (the global setting in 12.6 or the
  host's Record output override in 3.1), the recording opens read-only, with search and
  export to .txt. If it was not recorded, connection details are shown.
- Export to .txt writes unencrypted text, so a warning is shown first.
- Recordings are stored encrypted and are never synced.
- The log is always written to the personal vault and stored encrypted. Syncing it between
  devices is turned on separately (see 12.2).

---

## 12. Settings

### 12.1 Settings window

- Opens with Ctrl+, or from the application menu. A modal window on top of the
  application, closed with the X button or Esc. [decision on Esc]
- Navigation on the left, section content on the right. They scroll independently.
- Changes apply immediately; there is no "Save" button. [decision]
- Navigation sections from top to bottom:
  1. Account & Sync;
  2. Security (see 13.4); [decision]
  3. SSH ID;
  4. Team;
  5. Terminal;
  6. SFTP;
  7. Logs;
  8. Shortcuts;
  9. Updates (see 12.7); [decision]
  10. Theme: a dropdown right in the navigation (Auto / Light / Dark);
  11. external links with an external-window icon: Documentation, Report a Bug,
      Changelog (they lead to the project repository);
  12. the application version at the bottom.

### 12.2 Account & Sync

The application works without an account. With no server connected, this section has only
the "Connect sync server" button (it opens the wizard, see 13.5) and data export. [decision]

With a connected server:

- server address; account email marked "verified / not verified";
  email change. The mark is hidden when the server has no email (SMTP) set up; [decision]
- two-factor authentication (TOTP) on the server: toggle;
- **Log out**: sign out on this device. The user chooses whether to keep
  the local copy or delete it from the device. By default the copy is kept
  (principle 3 in SPEC.md);
- master password change is in Settings → Security (see 13.4), because it exists without
  a server too. The recovery key of the account is there as well; [decision]
- deleting the account on the server. The local copy is kept in that case.
  If the account is the only owner of a team vault, deletion is blocked until ownership is
  transferred to another member (see 12.3). [decision]

**Synchronization** block:

- the toggles are set for this device and apply both to sending and to receiving changes.
  Turning a toggle off stops syncing that type but does not delete data from the
  server; [decision]
- the toggles apply to the personal vault. Team vaults always sync in full; [decision]
- toggles for what to sync:
  - hosts and groups (on by default);
  - keys and identities (on by default);
  - snippets (on by default);
  - forwarding rules (on by default);
  - known hosts (on by default); [decision]
  - workspaces (on by default); [decision]
  - custom terminal themes (on by default); [decision]
  - keyword highlighting rules (on by default); [decision]
  - command history (**off** by default);
  - connection log (**off** by default);
- app settings are not synced; they stay on each device. Session output recordings are
  never synced (see 11.2); [decision]
- time of the last sync and errors, if any;
- a "Conflicts: N" row, shown while there are unresolved conflicts. A click opens the list
  of conflicts (see 13.6); [decision]
- "Sync now" button.

Also:

- a button to export all data (plain or encrypted JSON, ssh config).
  Always available. A plain JSON export contains secrets in unencrypted form, so a warning
  is shown first; [decision]
- no trials, pricing plans or purchase buttons.

### 12.3 SSH ID and Team [decision]

- **SSH ID**: the user's personal Ed25519 key, stored in the personal Keychain. Only its
  public part is published to the server. When a host in a team vault has no credentials,
  each member connects with their own SSH ID or identity (see 2.6), so no shared password or
  key is needed. Here you can view and copy the public key and upload it to hosts.
- **Team**:
  - vault members with the owner / editor / viewer roles. A vault can have several owners;
  - next to each member, the fingerprint of their key, for comparison outside the server
    (for example, in person or by phone). The key is pinned on first use;
  - invitation in two steps:
    1. the owner creates an invitation link. The link carries only a one-time id, never a
       key. If the server has email (SMTP) set up, the invitation can also be sent by
       email; otherwise only the link is available;
    2. after the invitee accepts, the owner compares the invitee's key fingerprint and
       confirms. Only then does the invitee get access to the vault;
  - the invitation dialog says plainly that every member, viewers included, receives all
    secrets of the vault (passwords and keys of its hosts);
  - removing a member creates a new vault key, and the owner's app re-encrypts the vault's
    records in the background (see SPEC.md). The dialog suggests changing the passwords and
    keys of the vault's hosts, because the removed member has seen them;
  - a removed member keeps the vault as a read-only local archive with "Export" and "Copy
    to personal vault" actions (principle 3 in SPEC.md);
  - "Publish my connection log": a toggle for each team vault, off by default. When it is
    on, your log entries for this vault's hosts are visible to its members (see 11.1).
    Viewers can publish too.

### 12.4 Terminal

**"Terminal" block:**

- Autocomplete (beta): dropdown, off by default. Later stage.
- Autoreconnect: toggle, on by default (see 4.6).
- Record command history: toggle, on by default. A host can override it (see 3.1). [decision]
  History is kept only for saved hosts (4.2). [decision]
- Import shell history: pull history from the server's `~/.bash_history` and
  `~/.zsh_history` into the History panel. **Off** by default,
  because such files often contain secrets. On import, the same
  filters as in 4.2 apply, and commands that look like secrets are skipped. [decision]
- Copy on select: toggle. Right-click always pastes text (see 4.3). [decision]
- Bell sound: off by default.
- Bright colors for bold text: toggle.
- Adjust indistinguishable colors: Off / Palette only / Always, Palette only by default.
  [decision]
  - When the contrast of a cell's text and background colors is below 4.5:1 (WCAG), the
    text color is moved toward white or toward black until the pair reaches 4.5:1. It
    moves away from the background (lighter text toward white, darker text toward
    black); when that direction cannot reach 4.5:1, it moves the other way. The color
    moves by the smallest step that reaches 4.5:1, so its hue stays as close to the
    original as possible. Only the display changes, not the terminal data.
  - Palette only: colors from the theme palette are adjusted, that is the 16 ANSI colors,
    the 256-color table and the default foreground and background, also when a program
    redefines them. Truecolor (24-bit) colors are never changed. Always: truecolor colors
    are adjusted too. Off: all colors are shown as they are.
  - The selection and the cursor keep their own colors, and text hidden by the program
    (SGR 8) stays hidden.
- Cursor style: block / bar / underline, block by default; Blinking: toggle, off by
  default. [decision]
- Terminal emulation type: the TERM value, `xterm-256color` by default.
  Options: `xterm`, `xterm-256color`, `vt100`, `linux`.

**Font block:**

- font selection: search across installed monospace fonts and bundled
  open-source fonts;
- text size: −/+ buttons and a number field, 14 by default;
- line spacing: 1.2 by default, from 1.0 to 2.0. The same font settings open from the
  Appearance tab of the terminal sidebar (4.2). [decision]

**"Terminal theme" block:**

- the default theme for new hosts. The selection is the same as in 4.2;
- "Import theme…": imports `.itermcolors` files and Windows Terminal color schemes. They
  are converted to the app's own theme format (`docs/design-system.md`, §7) and appear in
  the theme lists (4.2). Imported themes are stored as records and synced
  (see 12.2). [decision]

**"Keyword highlighting" block:**

- global toggle, on by default;
- categories with color selection: Error, Warning, OK, Info, Debug, IP and MAC addresses.
  Each category has a default color for dark and for light backgrounds; which one is used
  depends on the brightness of the terminal theme's background, not on the interface
  theme. Colors are stored as hex values; [decision]
- custom rules: regex + color + on/off; [decision]
- highlighting changes only the display, not the data. In full-screen programs
  (alternate screen: vim, htop) it is disabled. [decision]

**"OS detection" block:**

- explanation: on the first connection to a host, a short script runs
  that detects the OS for the icon. The script runs only if the server
  did not report the OS itself;
- toggle, on by default;
- "OS detection script": viewing and editing the script, a
  "Reset to default" button;
- the script runs in a separate exec channel, not in the interactive session,
  and does not end up in history. [decision]

**"Local terminal shell" block:** a dropdown of detected
shells plus a custom path.

- Windows: PowerShell, PowerShell 7, cmd, WSL distributions, Git Bash.
- macOS and Linux: shells from `/etc/shells`.
- Until a shell is chosen here, Windows uses PowerShell 7 (`pwsh`) when it is installed,
  and Windows PowerShell otherwise. A local terminal tab is titled with the shell's
  display name, for example "PowerShell 7" (1.1). [decision]

**"Keepalive" block:**

- explanation: how often (in seconds) to send SSH keepalive; 0 means
  "off";
- Interval field, 30 by default, so that connection drops are detected in time. [decision]
- A host can override the interval (see 3.1). [decision]

### 12.5 Shortcuts

- At the top, key set selection: PC or Mac. The default is Mac on macOS and PC on Windows
  and Linux. [decision on key sets]
- Shortcut | Action table:
  - clicking a shortcut starts recording a new shortcut;
  - conflicts are highlighted;
  - any action can be set to "unassigned".

Default set for Windows/Linux:

| Shortcut | Action |
|---|---|
| Ctrl+1…9 | go to tab N (1 = Vaults, 2 = SFTP) |
| Ctrl+Alt+arrows | focus between split panes |
| Ctrl+Shift+D | split vertically: new pane to the right [decision] |
| Ctrl+Shift+E | split horizontally: new pane below [decision] |
| Ctrl+Shift+C | copy from the terminal |
| Ctrl+Shift+V | paste into the terminal |
| Ctrl+Shift+A | select all in the terminal |
| unassigned | open the Hosts screen |
| Ctrl+L | new local terminal |
| Ctrl+Shift+L | lock the app (13.3) |
| Ctrl+Alt+S | new serial connection |
| Ctrl+T, Ctrl+Shift+T | new tab (section 6) [decision] |
| Ctrl+Shift+W | close tab; on a tab with splits, close the active pane (4.7) [decision on splits] |
| Alt+→ | next tab |
| Alt+← | previous tab |
| Ctrl+P, Ctrl+Shift+P | open Port Forwarding [decision on Ctrl+Shift+P] |
| Ctrl+K, Ctrl+Shift+K | search hosts and tabs [decision] |
| Ctrl+Shift+F | search in the terminal (the Tools tab of the terminal sidebar, 4.2) |
| Ctrl+, | settings |

**Important** [decision]: shortcuts without Shift (Ctrl+L, Ctrl+P, Ctrl+K, Ctrl+T)
coincide with shell keys: `Ctrl+L` clears the screen, `Ctrl+P` recalls the previous
command, `Ctrl+K` deletes to the end of the line, `Ctrl+T` swaps two characters. When
focus is in the terminal, such shortcuts go to the shell, not to the application.
Application actions from the terminal are available via Ctrl+Shift shortcuts:
Ctrl+Shift+P, Ctrl+Shift+K and Ctrl+Shift+T work everywhere.
Ctrl+Shift+L locks the app (13.3) and also works everywhere; from the terminal, a new local
terminal is opened with the "+" button or the app menu (D29).

**Terminal focus** [decision]:

- When focus is in the terminal, the app captures only: Ctrl+1…9, `Ctrl+,`, all Ctrl+Shift
  shortcuts, Ctrl+Alt+arrows, Alt+← and Alt+→. Every other combination goes to the shell,
  for example Ctrl+L or Ctrl+Alt+S.
- Alt+← and Alt+→ move by words in readline; while they are assigned to tabs, the shell does
  not get them. They can be reassigned in this table.
- No shortcut fires while AltGr is active. On Windows, Ctrl+Alt equals AltGr on many
  keyboard layouts, so the key types its character instead.

**Mac set** [decision]:

- App actions use Cmd instead of Ctrl (for example Cmd+1…9, `Cmd+,`, Cmd+K).
- Ctrl+letter always goes to the shell.
- Option types characters and is not used for app shortcuts.
- The full Mac table is added to this section before stage 6.

### 12.6 SFTP and Logs [decision]

- **SFTP:**
  - show hidden files: off by default;
  - delete confirmation: on by default;
  - action on name conflicts: ask (default, see 5.7), replace, skip or
    rename;
  - number of parallel transfers: 3 by default (see 5.6);
  - program for Open: empty by default, which means the program the system assigns to
    the file type (see 5.4). "Open with…" always asks for a program.
- **Logs:**
  - connection log retention period: 90 days by default. Saved entries are kept
    (see 11.1);
  - session output recording: a global toggle, off by default, with a warning
    that output can contain secrets. A host can override it (see 3.1). Recordings are
    stored encrypted and are never synced;
  - clearing the log.

### 12.7 Updates [decision]

- Automatic update check: toggle, off by default. When it is on, the app checks the
  update source at launch.
- The explanation under the toggle says that a check is a plain request for the update
  manifest, without any identifiers.
- Update source: a URL field, the project repository's releases by default, with a
  "Reset to default" button.
- "Check now" button: the same action as Help ▸ Check for updates (1.4).
- When an update is found, the update button appears in the top bar (1.1). An update whose
  signature does not match the project's signing key is not installed.

---

## 13. Lock and account

The master password is required from the first launch, even without an account and a
server. The same password later becomes the password of the sync account. Sections
13.1–13.4 follow the owner's description of 2026-10-20 (D29); details we added are marked
[decision]. The key hierarchy is described in `docs/security.md`, the files on disk in
`docs/data-format.md`.

### 13.1 First run

- Shown at the first launch, while the data directory holds no vault, and after "Start
  with a new vault" (13.2). It comes before any other screen and covers the whole window
  like the unlock screen; only the window buttons stay available. [decision]
- A centered block, from top to bottom:
  - the app name and one line saying that the data is stored locally and encrypted;
  - the "Master password" and "Repeat password" fields, masked, with show/hide buttons.
    Focus is in the first field; [decision on show/hide and focus]
  - a strength indicator under the password. The minimum is 8 characters; there are no
    rules about the kind of characters (digits, letters of both cases, symbols). The
    indicator is advice only and never blocks creation; [decision on the last sentence]
  - a warning that is always shown and cannot be dismissed: if the master password is
    forgotten, the data cannot be recovered;
  - the "Remember on this device" toggle, on by default: the key is kept in the OS
    keychain (crate keyring), and the password is not asked at launch (13.2). A short note
    under the toggle says that any program running under this OS user can then open the
    data (13.4). Where the OS has no keychain, the toggle is hidden and the password is
    asked at every launch (13.4); [decision on the note and the hidden toggle]
  - the "Create vault" button, accent style. Enabled when the password has at least 8
    characters and both fields match; Enter in either field does the same. [decision on
    when it is enabled and on Enter]
- Under a password that is too short: "At least 8 characters"; under the second field when
  the two differ: "Passwords do not match". They appear when the field loses focus or on an
  attempt to create the vault. [decision]
- While the vault is being created (the key derivation takes about a second), the fields
  are read-only and the button shows progress. [decision]
- No recovery key is shown or asked for in local mode (D29).
- After "Create vault", the Vaults tab opens with the Hosts screen in its empty state
  (see 2.3).

### 13.2 Unlock

- Shown at launch when the vault key is not remembered on this device (see 13.4) or the
  remembered key cannot be used, and after locking (see 13.3). It covers the whole content
  of the window, including the tabs and any open dialogs; only the window buttons stay
  available. [decision on what it covers]
- Contents: the app name, the "Master password" field with focus in it, the "Unlock"
  button (accent style) and a "Forgot password?" link. Enter in the field works the same
  as the button.
- While the password is being checked (about a second), the field and the button are
  read-only. [decision]
- A wrong password shows a message under the field ("Wrong password"); the field is
  cleared and keeps focus.
- After 5 wrong passwords in a row, unlocking pauses with a growing delay: the field and the
  button are disabled, and the message under the field counts down the time left. The
  pause is 5 seconds after the fifth wrong password and doubles after each further one
  (10, 20, 40 … seconds), up to 5 minutes. The core enforces the pause, not only this
  screen, and keeps the count across restarts; a correct password resets it. Wrong
  passwords never delete data. [decision on the numbers and the counter]
- **Forgot password?** opens a dialog. It explains that the master password cannot be
  recovered: it is not stored anywhere, and without it nobody can decrypt the data. Then it
  offers:
  - **Restore from sync**: restore the data from the sync server with the account's
    recovery key (see 13.5). Until a sync server is connected, it is shown disabled with
    the hint that sync is not set up. This is an exception to the stage flag rule of
    section 0 (D29).
  - **Start with a new vault**: asks for an explicit confirmation in a second dialog. It
    says that the current data is not deleted: the data file is renamed and kept next to
    the new one (`docs/data-format.md`), and the app starts with an empty vault. "Cancel"
    has the focus; the confirm button is a danger button labeled "Start with a new
    vault". After the confirmation, the app returns to the first run (13.1). [decision on
    the dialog]
  - "Cancel" closes the dialog.
- Restore from sync (stage 7) [decision]: the user enters the recovery key, then a new
  master password and its confirmation, and the app unlocks. The new password also becomes
  the account password, and other devices are signed out, as after a password change
  (see 13.4).
  - The dialog also shows the account email, pre-filled and editable. If the server does
    not accept the key, the dialog says that the email may have changed on another device,
    as well as that the key may be wrong.
  - When the server cannot be reached or does not accept the recovery, the dialog offers to
    recover only the data on this device: the new password unlocks the local copy, and this
    device is signed out of the account until the user signs in with the account password
    or recovers through the server.

### 13.3 Lock

- The app locks:
  - manually: with the Lock item in the app menu (see 1.4) and with Ctrl+Shift+L;
  - on inactivity: after the auto-lock interval without input (see 13.4): never, 5, 15, 30
    or 60 minutes, "never" by default. Only keyboard and mouse input in the app's windows
    counts (keys, mouse buttons and the scroll wheel, in dialogs and menus too); terminal
    output does not. [decision on what counts as input]
  - Before any lock, and before a window closes or the app quits, an edit in the details
    panel that is still waiting for the autosave is saved. [decision]
- Locking applies to all windows at once. [decision]
- On lock:
  - the unlock screen (13.2) covers every window. Open session tabs stay alive, but their
    content (terminal output, panels and dialogs) is hidden until unlock;
  - established sessions, port forwards and running file transfers keep running; nothing
    is disconnected;
  - the vault keys and decrypted secrets are wiped from memory; [decision]
  - everything that needs secrets waits for unlock: automatic reconnection (see 4.6) and
    the autostart of forwarding rules (see 8.4). [decision]
- After unlock, every window returns to the state it had before the lock. [decision]
- Lock and auto-lock always ask for the master password, also with "Remember on this
  device": the remembered key is used only at launch (see 13.4). [decision]

### 13.4 Security settings

Settings → Security (see 12.1):

- **Auto-lock**: a dropdown with Never, 5, 15, 30 and 60 minutes; Never by default.
- **Change master password**: the current password, the new password and its
  confirmation. The new password follows the rules of 13.1 (at least 8 characters, the
  strength indicator). [decision]
  - Works without a server too. The change takes effect immediately.
  - With a sync server, the change needs a connection to the server. Before confirming, a
    warning says that other devices will be signed out and will ask for the new password.
- **Recovery key**: only with a sync account (stage 7); in local mode the item is hidden
  and the key is never shown (D29). "Show" and "Reissue", both after entering the master
  password. "Reissue" creates a new key and shows it as in 13.5, with the required "I saved
  it" checkbox; the old key stops working. [decision]
- **Remember on this device**: toggle, on by default; the first-run screen sets it (13.1).
  When it is on, a device key is stored in the OS keychain, and the app unlocks at launch
  without the password (see 13.2). [decision on the details below]
  - The explanation under the toggle says that any program running under this OS user can
    then unlock the data. Turning it on later shows the same text as a warning. Turning it
    off deletes the key from the keychain, and the next launch asks for the password.
  - Hidden when the OS has no keychain (for example, Linux without Secret Service) and in
    unsigned macOS builds.
  - Biometric unlock is a later stage and is hidden until then.

### 13.5 Connect sync server wizard [decision]

- Opened with "Connect sync server" (see 12.2). A modal dialog with steps; every step has a
  "Back" button:
  1. **Server**: the server URL. The app checks that the server responds and shows an error
     if it does not.
  2. **Sign in or register**:
     - Sign in: the email and the account's master password;
     - Register: the email and the current master password, entered again to confirm. If
       the server allows registration only by invitation, an "Invite code" field is shown
       and is required.
  3. **TOTP**:
     - sign in: a one-time code, if TOTP is enabled for the account;
     - register: an offer to enable TOTP now (a QR code, the secret as text, and a code to
       confirm). It can be skipped and enabled later (see 12.2).
  4. **Recovery key** (registration only): the recovery key that protects the account is
     shown as text in a monospace font with a "Copy" button; the clipboard is cleared after
     30 seconds, as for private keys (see 7.3). The "I saved the recovery key" checkbox is
     required: "Finish" is disabled until it is checked. In local mode no recovery key is
     shown (13.1), so this is the first time the user sees it. If the data on this device
     was connected to another account before, registration creates a new recovery key, and
     this step shows the new one.
- The master password never leaves the device; the server receives only a key derived
  from it.
- Signing in to an existing account on a device that already has local data merges the
  data: the local records are re-encrypted with the account's keys and merged with the
  account's data, with deduplication as in import (see 2.5). From then on, this device uses
  the account's master password. The wizard says this before the merge.
- When the wizard finishes, sync starts, and Account & Sync (12.2) shows the connected
  server.

### 13.6 Sync conflicts [decision]

- When sync keeps a losing version of an object, the object's card shows a conflict badge
  in every list (hosts, groups, keys, identities, forwarding rules, snippets, known hosts),
  and Account & Sync (12.2) shows "Conflicts: N".
- A click on the badge, or on a conflict in the list opened from "Conflicts: N", opens the
  comparison dialog:
  - the two versions side by side: on the left, the current version; on the right, the
    other version. Each shows the device and the time of the change, and fields that
    differ are highlighted;
  - passwords stay masked and private keys stay hidden (see 7.3);
  - if one version is a deletion, its side says "Deleted";
  - the "Keep this" button keeps the current version; the "Keep other" button replaces it
    with the other version.
- Closing the dialog without a choice leaves the conflict unresolved. After a choice, the
  other version is deleted, and the badge disappears on all devices after the next sync.
