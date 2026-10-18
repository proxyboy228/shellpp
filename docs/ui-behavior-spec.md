# UI Behavior Specification

> This document describes the **structure and behavior** of the interface: what is located where,
> and what happens on each action. The visual style (colors, icons, fonts,
> radii, shadows) is described separately in `docs/design-system.md`.
>
> Marks:
> - **[decision]**: behavior is not confirmed, chosen by us, can be changed;
> - **[clarify]**: awaiting an answer; until answered, implement as [decision].

## 0. Platforms
- Windows (primary for manual testing), macOS, Linux. Later iOS and Android.
- Keyboard shortcuts are given for Windows/Linux. On macOS, Ctrl is replaced with Cmd
  where this is the platform convention.

---

## 1. Main window

### 1.1 Top bar (window title bar)
- Custom window title bar (frameless). On Windows/Linux, the minimize /
  maximize / close buttons are on the right; on macOS, the system buttons are on the left. [decision for macOS]
- From left to right:
  1. App menu button.
  2. Persistent **Vaults** tab: the data management workspace.
     Next to it is an arrow switcher for the current vault (personal, team).
  3. Persistent **SFTP** tab.
  4. Open session tabs: host OS icon + host name.
     - On the active tab (and on any tab on hover), a close button
       appears in place of the OS icon.
     - The active tab is highlighted with the accent color of this host's terminal theme. [decision]
     - On the right side of the tab, a session type icon: terminal or SFTP. [decision]
  5. "+" button: a new tab with the start page (see section 6).
- On the right:
  - update button (visible only when an update is available);
  - notification center;
  - button to show and hide the terminal sidebar (only when a session
    tab is active, see 4.2);
  - window buttons.

### 1.2 Sidebar (inside the Vaults tab)
- Sections from top to bottom: **Hosts**, **Keychain**, **Port Forwarding**, **Snippets**,
  **Known Hosts**, **Logs**.
- The active section is highlighted. Each section opens its own screen in the main area.

### 1.3 Details panel
- A slide-out panel on the right. Opens when an object is selected or when a new one is created.
- In the header: a title ("host details" / "new host"), a "…" menu with additional
  actions (the set depends on the object; for a host see 3.3), a button to collapse the panel.
- The main area narrows; the panel does not overlap it. [decision]

### 1.4 App menu (the button with three bars)
1. Vaults (Ctrl+1).
2. SFTP (Ctrl+2).
3. Settings (Ctrl+,).
4. — separator —
5. New Local Terminal.
6. New Serial Connection.
7. Edit ▸: undo, redo, cut, copy, paste, select all. [decision]
8. View ▸: interface zoom, full-screen mode, show or hide
   the sidebar. [decision]
9. Window ▸: new window, next and previous tab, close tab. [decision]
10. Help ▸: documentation, report an issue (link to issues), check for
    updates, about. [decision]
11. — separator —
12. Exit.

Additionally:
- Keys in hints are written as symbols (`Ctrl+,`), not as words in the system
  language. [decision]
- On macOS, the system menu bar with the same items is used instead of this
  menu. [decision]

---

## 2. Hosts screen

### 2.1 Search and quick connect
- Search bar at the top of the main area, full width. The placeholder hints
  that you can search for a host or enter `ssh user@hostname`.
- Typing filters groups and hosts in real time: by name, address,
  tags and username. [decision on the set of fields]
- If the string is recognized as a connection address (`host`, `user@host`,
  `user@host:port`, `ssh user@host -p port`), the Connect button on the right side of the bar
  becomes enabled. Enter or a click connects without saving the host.
- While the string is not an address, the Connect button is disabled.
- After a successful quick connect, an unobtrusive offer to save the host is shown. [decision]

### 2.2 Action bar
On the left:
- **New host** split button. The main part creates a host; the arrow opens a menu:
  1. **New Group**: create a group in the current location.
  2. **Import**: import dialog (formats are listed in SPEC.md, section "Import").
  3. Cloud integrations (AWS, DigitalOcean, Azure): connecting a provider
     account and importing its servers as hosts. **A separate later stage**;
     in the MVP these items are hidden.
- **Terminal**: open a local terminal in a new tab. The default shell
  is set in the settings (on Windows, PowerShell / cmd / WSL). [decision]
- **Serial**: serial port connection, opens as a separate
  tab in the top bar.

On the right:
- View toggle: tiles / list.
- Tag filter.
- One more button. [clarify purpose]
- Avatars of the current vault's members and a "+" button. A click opens a dropdown menu:
  an "Invite members" item, below it a list of current members (avatar + email).

### 2.3 Screen content
- Two sections: **Groups** on top, **Hosts** below it.
- Groups work like folders: a click opens a group, inside which its subgroups
  and hosts are visible. Above the content is a path (breadcrumbs) with return to any level. [decision]
- Group card: icon, name, number of hosts (including nested groups). [decision]
- Host card:
  - OS icon. The distribution is detected on the first successful connection
    (see 12.4, block "OS detection"). Until then, a neutral icon is shown.
    OS icons are taken from a set with a suitable license, taking into account
    the trademark policies of the distributions;
  - title: the label, or the address if the label is empty;
  - subtitle in the secondary color, comma-separated: protocol(s), username,
    then the group or tags;
  - long text is truncated with an ellipsis.
- The grid is adaptive: the number of columns depends on the width of the main area.
- The selected card is visually highlighted.
- Multiple selection is supported: Ctrl+click adds or removes
  a card, Shift+click selects a range. Context menu actions that
  apply to multiple hosts (Connect, Add to Workspace, Move to, Copy to,
  Remove) are performed for all selected ones. [decision on keys and set]
- Cards can be dragged: within the list the order changes; when dropped
  onto a group card, the host is moved into that group. [decision on details]
- Promo banners are not needed. Instead, an empty state with hints. [decision]
- Empty state (no hosts): a short explanation and the buttons "Create host"
  and "Import from ~/.ssh/config". [decision]

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
     - via SFTP (the host opens in the SFTP manager);
     - separator;
     - "in a new window" hint: if Alt is held while choosing any option,
       the session opens in a new app window. Alt + double click on
       a card works the same way. [decision for double click]
  3. **Add to Workspace ▸**: add to a saved workspace
     (details in the "Terminal" section).
  4. **Edit Host Details** (E).
  5. **Collaborate**: share the host with team members.
  6. **Move to ▸**: move to another vault or group.
  7. **Copy to ▸**: copy to another vault.
  8. **Duplicate**: a copy of the host; the suffix " Copy" is appended to the name.
  9. **Copy Link**: a link to the host in the app's own URL scheme
     (`shellpp://host/<id>`). An (i) icon explains who will be able to open the link.
  10. **Remove**: deletion with confirmation and the ability to undo
      the action for a few seconds. [decision]

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
   - Backspace: what the Backspace key sends (default / options). [decision: Default, `^?` (DEL), `^H`]
4. **Share this host**: opens the sharing dialog. Disabled until the host is created.
5. **SSH**:
   - port: a field on the same line as the protocol name, placeholder 22;
   - Credentials:
     - a hint above the fields if the same login with a password or key is also used
       in N other hosts: a one-link offer to merge them into an identity.
       After the identity is created, all these hosts reference it, and changing the password
       in one place applies to all of them;
     - username;
     - password: masked, with a show/hide button;
     - a linked key is shown as a separate field with the key name from Keychain;
       it can be unlinked;
     - a "+" button to link an identity, key, certificate or FIDO2 key.
       A type that is already linked disappears from the list of options;
   - "Show more" expands additional settings, one field per line:
     - Agent Forwarding: on/off, off by default;
     - Startup snippet: a snippet that runs right after login;
     - Host Chaining: a chain of intermediate hosts (jump hosts);
     - Proxy: HTTP or SOCKS proxy for the connection;
     - Environment Variables: environment variables for the session;
     - Charset: encoding, UTF-8 by default;
     - Mosh: on/off, off by default;
     - terminal theme for this host: a preview thumbnail + name; clicking it
       opens theme selection. The theme set is our own; dark theme by default.
6. **Add Telnet**: add a second protocol with its own settings to the same host.
7. The **Connect** button is pinned to the bottom of the panel. Disabled until the address is filled in.

### 3.2 Behavior
- Changes are saved automatically; there is no separate save button.
- A new host appears in the list as soon as the address is entered. [decision]
- If the address is empty and the panel is closed, the draft is discarded without prompting. [decision]

### 3.3 "…" menu in the host panel header
1. Connect.
2. Add Telnet.
3. Duplicate.
4. Remove.

---

## 4. Terminal session

### 4.1 Session window
- The terminal occupies the entire area below the top bar. There is no Vaults sidebar
  and no borders, only a small inner padding.
- The terminal theme from the host settings, or the default theme, is applied.
- After connecting, only what the server sends (MOTD, etc.) is displayed,
  without the application's own banners.
- Input focus is in the terminal immediately.

### 4.2 Terminal sidebar
- Opened and hidden with a button in the top bar (see 1.1). The sidebar is on the right;
  the terminal narrows rather than being overlapped.
- At the top there is a row of icon tabs: Tools, Snippets, History, Appearance.

**Tools tab:**
- search in terminal output (Ctrl+Shift+F) with "previous" and
  "next match" buttons. Matches are highlighted in the terminal;
- Autocomplete: a dropdown of modes, off by default.
  Command suggestions are built from the local history of this host. **Later stage**,
  marked as beta in the UI. [decision on the suggestion source]

**Snippets tab** [decision]:
- list of snippets with search;
- clicking inserts the snippet into the terminal without executing it;
- the "run" button inserts the snippet and immediately sends Enter.

**History tab:**
- history search;
- list of entered commands, newest first. Clicking inserts the command into the terminal
  without executing it; [decision]
- a hint card: the three most frequent commands with an offer to save them
  as snippets. Commands can be edited before saving; there are "Close"
  and "Save" buttons. A dismissed hint is no longer shown for this
  host; [decision]
- **Mandatory security requirements:**
  - do not record in history input entered while echo is off: password prompts
    from `su`, `sudo`, `passwd`, key passphrase prompts, etc.
    Indicators: the server does not echo the typed characters, the last line
    of output looks like a password prompt (`password:`, `passphrase`);
  - strip bracketed paste control sequences (`ESC[200~`,
    `ESC[201~`) and other escape codes. History stores only the clean text of the
    command;
  - history is stored encrypted, like the rest of the data. History sync
    between devices is off by default;
  - history recording can be disabled for an individual host. Any entry can be
    deleted, and the entire history of a host can be cleared;
  - the "save as snippets" hint never suggests entries that look
    like secrets.

**Appearance tab:**
  - the Font row with an arrow leads to font settings: family, size,
    line spacing; [decision]
  - Themes list: each theme has a preview thumbnail in its colors and a name.
    The current theme is highlighted. Clicking immediately applies the theme to the session and saves it
    in the host settings; [decision]
  - there are no theme popularity counters, because they would require telemetry.
    Themes added in the latest update are marked "new"; [decision]
  - theme set: our own dark and light defaults plus popular
    open color schemes with compatible licenses. Check the license of each scheme
    and list it in THIRD_PARTY_NOTICES.

### 4.3 Mouse in the terminal
- Right-click pastes text from the clipboard. Multi-line paste
  requires confirmation (see SPEC.md).
- If copy-on-select is enabled, selecting with the mouse copies the text immediately. [decision]

### 4.4 Session tab context menu
1. Duplicate: a new session to the same host in a new tab.
2. Duplicate in a new window: the same, but in a new application window.
3. Start multiplayer: real-time collaboration in a session.
   **Later stage.** Works via a self-hosted server, with E2E encryption,
   by invitation. The invitee sees the output and can enter commands only
   with the session owner's permission. [decision]
4. Rename: rename the tab. The host itself is not renamed.
5. Split horizontally: split the tab. [decision: Split vertically is also available]
6. Close.

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
  the tab closes by itself. The neighboring tab becomes active: the one on the left,
  or, if there is none, the one on the right. If no session tabs remain, Vaults opens.
- **Unexpected drop** (network, keepalive timeout, non-zero exit code) [decision]:
  - the tab does not close, and the output is preserved;
  - a message with the reason and "Reconnect"
    and "Close" buttons is shown over the terminal;
  - automatic reconnection with increasing delay is enabled
    in the settings.

### 4.7 Splits [decision]
- A tab can be split into panes horizontally and vertically: via the tab
  menu or with keyboard shortcuts (configurable).
- The border between panes can be dragged. The active pane is highlighted.
  Focus is switched by clicking or with Ctrl+Alt+arrows (see 12.5).
- Each pane has its own session, to the same host or to a different one.

---

## 5. SFTP manager (SFTP tab)

### 5.1 Layout
- Two panels of equal width. Each panel is an independent file source:
  the local computer or a remote host.
- By default, the left panel shows the local file system and the right panel is empty.

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
  (see SPEC.md, SFTP section).

### 5.4 File context menu (right-click)
1. Open: open in the program the system assigns to this file type.
   A remote file is downloaded to a temporary folder, changes are tracked
   and uploaded back to the server, and a notification is shown for each upload. [decision]
2. Open with…: the same, but with a choice of program.
3. Copy to target directory: copy the selection to the current folder
   of the opposite panel.
4. Rename.
5. Delete: shown in the danger-action color, performed after confirmation.
6. Refresh.
7. New Folder.
8. Edit Permissions: access permissions dialog (see below).

Edit Permissions dialog [decision]:
- rwx checkboxes for the owner, group and others;
- a synchronized field with the octal value;
- for folders there is an "apply recursively" option.

### 5.5 Actions menu in the panel header
The same items as in the context menu (they act on the selection), plus:
- Show Hidden Files: a toggle for showing hidden files;
- Select All;
- Close: close the connection in this panel (danger-action color).
  The panel returns to the empty state from 5.2.

Items that require a selection are disabled without one. [decision]
In the local panel on Windows, the Edit Permissions item is hidden. [decision]

---

## 6. New tab page
- Opened with the "+" button in the top bar.
- Until a host is selected, the tab is named "New Tab". After selection, the session
  opens in the same tab. [decision]
- At the top center, a search field for hosts and open tabs with a hint for the
  Ctrl+K key; focus is placed in it immediately. Ctrl+K from anywhere in the app
  opens the same search. [decision on the global Ctrl+K]
- Below it, the Recent connections block:
  - row: OS icon, host name (or address), and on the right the path "vault / group";
  - clicking connects in this tab;
  - arrow keys and Enter work the same as a click. [decision]
- The block header has two buttons:
  - Create a workspace: save the currently open tabs as a workspace;
  - Restore: restore the tabs from the previous app launch. Disabled
    if there is nothing to restore. [decision based on meaning]
- When typing in the search, the list is replaced with results: open tabs first,
  then hosts. [decision]

---

## 7. Keychain screen

### 7.1 Action bar
On the left:
- **New key** split button. The main part opens, in the details panel, a form
  for adding an existing key: paste text, choose a file, or drag a
  file into the form. [decision on drag & drop] The arrow opens a menu:
  1. **Generate key**: generate a new pair. Parameters: type (Ed25519
     by default, ECDSA, RSA with a choice of length), label, optional
     passphrase. [decision on parameters]
  2. **New Identity**: create an identity.
- **Certificate**: add an SSH certificate.
- **FIDO2**: add a hardware key (`sk-ssh-ed25519`, `sk-ecdsa`). [decision on types]

On the right: search, a tile/list toggle, a sort menu (by name,
by creation date, by modification date). The same sort icon is used
on the Hosts screen. [decision]

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
- Empty state: buttons "Generate", "Import" and
  "Import from ~/.ssh". [decision]

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
to file, Remove. [decision]

**Mandatory security requirements:**
- The private key contents are **hidden** by default. Instead of the text, the
  type, fingerprint and a note that the key is hidden are shown.
- The key can be shown only with an explicit button. When the user leaves
  the panel, the key is hidden again.
- Copying the private key is done with a separate button. The clipboard
  is cleared after 30 seconds. [decision on the timeout]
- If the Public key field is empty, the public key is derived from the private key
  automatically.
- The SHA256 fingerprint is shown for each key.
- A key without a passphrase is marked with an unobtrusive hint "not protected
  by a passphrase". When exporting to a file, the user is offered to set a passphrase.
- The passphrase of an encrypted key is requested on import. The user
  chooses: save it in the encrypted storage or enter it on every
  connection.

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
  start on application launch.
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
  A new unsaved snippet has a placeholder prompting to set a label
  or a script.
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
   - **Script**: required, multi-line field, monospace font,
     shell syntax highlighting. `{{name}}` variables are highlighted
     and requested via a form at run time. [decision on highlighting]
4. **Share this snippet**: disabled until the snippet is saved.
5. **Targets for execution**:
   - a short explanation;
   - a list of target hosts (icon, address or label, protocol);
   - the "Add targets" link opens host selection with search
     and multiple selection.
6. The **Run** button is pinned at the bottom. Disabled until there is a script and at least
   one target.

### 9.4 Running on multiple hosts [decision]
- Up to 4 targets: a new tab split into panes, one session per host.
  The script runs in each.
- More than 4 targets: a tab with a results table. For each host, the
  status (waiting, running, success, error), exit code and output,
  which expands on click, are shown.
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
- Action bar: the **Import** button imports a known_hosts file in OpenSSH
  format (`~/.ssh/known_hosts` is suggested by default). On the right: search,
  view toggle, sorting.
- Grid of cards: fingerprint icon and address in OpenSSH format. For port 22
  it is `host`, for others `[host]:port`.
- A click opens the details panel [decision]: address, port, key type, SHA256
  fingerprint with a copy button, date added, saved hosts with this
  address, Remove button.
- Right-click [decision]: copy fingerprint, Remove.
- Export to the known_hosts format. [decision]

### 10.2 First connection and key change [decision]
- The first connection (TOFU) opens a dialog: address, key type, fingerprint.
  Buttons: "Accept and save" (default), "Connect once",
  "Cancel".
- If the host key has changed:
  - the connection is blocked, a warning is shown in the danger color;
  - the old and new fingerprints and an explanation about a possible attack are shown;
  - continuing is possible only via the explicit "Replace saved key" button;
  - there are no checkboxes like "don't ask again".

---

## 11. Logs screen
Connection log in the form of a table.

### 11.1 Columns
- **Date** with a sort order toggle:
  - session start date;
  - below it, start and end time. If the session ended on the next day,
    "(+1d)" is appended;
  - for an active session, "now" is shown instead of the end time. [decision]
- **User**: avatar and account (in a team vault you can see who connected),
  device name below it. The client IP address is not collected or shown. [decision]
- **Host**: OS icon, host name, protocol and user below it.
- **Saved**: bookmark button. Saved entries are not removed by auto-cleanup.
  Clicking the column header shows only saved entries. [decision]

### 11.2 Actions [decision]
- Clicking a row: if session output recording is enabled for the host, the
  session log opens read-only, with search and export to .txt. If recording
  is disabled, connection details are shown.
- The log is stored locally. Log sync is enabled separately (see 12.2).

---

## 12. Settings

### 12.1 Settings window
- Opens with Ctrl+, or from the application menu. A modal window on top of the
  application, closed with the X button or Esc. [decision on Esc]
- Navigation on the left, section content on the right. They scroll independently.
- Changes apply immediately; there is no "Save" button. [decision]
- Navigation sections from top to bottom:
  1. Account & Sync;
  2. SSH ID;
  3. Team;
  4. Terminal;
  5. SFTP;
  6. Logs;
  7. Shortcuts;
  8. Theme: a dropdown right in the navigation (Auto / Light / Dark);
  9. external links with an external-window icon: Documentation, Report a Bug,
     Changelog (they lead to the project repository);
  10. the application version at the bottom.

### 12.2 Account & Sync
The application works without an account. With no server connected, this section has only
the "Connect sync server" button and data export.

With a connected server:
- server address; account email marked "verified / not verified";
  email change;
- two-factor authentication (TOTP) on the server: toggle;
- **Log out**: sign out on this device. The user chooses whether to keep
  the local copy or delete it from the device. By default the copy is kept
  (principle 3 in SPEC.md);
- master password change (with key re-encryption);
- viewing and reissuing the recovery key;
- deleting the account on the server. The local copy is kept in that case.

**Synchronization** block:
- toggles for what to sync:
  - hosts and groups (on by default);
  - keys and identities (on by default);
  - snippets (on by default);
  - forwarding rules (on by default);
  - command history (**off** by default);
  - connection log (**off** by default);
- time of the last sync and errors, if any;
- "Sync now" button.

Also:
- a button to export all data (plain or encrypted JSON, ssh config).
  Always available;
- no trials, pricing plans or purchase buttons.

### 12.3 SSH ID and Team [decision]
- **SSH ID**: the user's personal key tied to the account on the server.
  When a colleague shares a host, you connect with your own key, and the owner's
  secrets are not transferred. Here you can view and copy the public key
  and upload it to hosts.
- **Team**: vault members, invitation by email or link, owner /
  editor / viewer roles, removing a member with vault key rotation (see SPEC.md).

### 12.4 Terminal

**"Terminal" block:**
- Autocomplete (beta): dropdown, off by default. Later stage.
- Autoreconnect: toggle, on by default (see 4.6).
- Import shell history: pull history from the server's `~/.bash_history` and
  `~/.zsh_history` into the History panel. **Off** by default,
  because such files often contain secrets. On import, the same
  filters as in 4.2 apply. [decision]
- Copy on select: toggle. Right-click always pastes text (see 4.3). [decision]
- Bell sound: off by default.
- Bright colors for bold text: toggle.
- Terminal emulation type: the TERM value, `xterm-256color` by default.
  Options: `xterm`, `xterm-256color`, `vt100`, `linux`.

**Font block:**
- font selection: search across installed monospace fonts and bundled
  open-source fonts;
- text size: −/+ buttons and a number field, 14 by default.

**"Terminal theme" block:** the default theme for new hosts. The selection is the same
as in 4.2.

**"Keyword highlighting" block:**
- global toggle, on by default;
- categories with color selection: Error, Warning, OK, Info, Debug, IP and MAC addresses;
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

**"Keepalive" block:**
- explanation: how often (in seconds) to send SSH keepalive; 0 means
  "off";
- Interval field, 30 by default, so that connection drops are detected in time. [decision]

### 12.5 Shortcuts
- At the top, key set selection: PC or Mac. [decision on key sets]
- Shortcut | Action table:
  - clicking a shortcut starts recording a new shortcut;
  - conflicts are highlighted;
  - any action can be set to "unassigned".

Default set for Windows/Linux:

| Shortcut | Action |
|---|---|
| Ctrl+1…9 | go to tab N (1 = Vaults, 2 = SFTP) |
| Ctrl+Alt+arrows | focus between split panes |
| Ctrl+Shift+C | copy from the terminal |
| Ctrl+Shift+V | paste into the terminal |
| Ctrl+Shift+A | select all in the terminal |
| unassigned | open the Hosts screen |
| Ctrl+L | new local terminal |
| Ctrl+Alt+S | new serial connection |
| Ctrl+Shift+W | close tab |
| Alt+→ | next tab |
| Alt+← | previous tab |
| Ctrl+P | open Port Forwarding |
| Ctrl+K | search hosts and tabs [decision] |
| Ctrl+Shift+F | search in the terminal |
| Ctrl+, | settings |

**Important** [decision]: shortcuts without Shift (Ctrl+L, Ctrl+P, Ctrl+K) coincide
with shell keys: `Ctrl+L` clears the screen, `Ctrl+P` recalls the previous
command, `Ctrl+K` deletes to the end of the line. When focus is in the terminal, such
shortcuts go to the shell, not to the application. Application actions from the terminal
are available via Ctrl+Shift shortcuts.

### 12.6 SFTP and Logs [decision, pages not reviewed]
- **SFTP:**
  - show hidden files by default;
  - delete confirmation;
  - action on name conflicts: ask, replace, skip or
    rename;
  - number of parallel transfers;
  - default program for Open with.
- **Logs:**
  - connection log retention period (N days);
  - session output recording. Off by default, with a warning
    that output can contain secrets;
  - clearing the log.
