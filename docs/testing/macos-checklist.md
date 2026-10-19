# macOS acceptance checklist

tauri-driver has no macOS support, so the UI e2e suite runs only on Linux and
Windows (`docs/decisions.md`, decision 6). On macOS a screen is accepted with
the CI build, the Rust tests and this manual checklist
(`docs/architecture.md`, section 9.1). The checklist covers every element that
is visible in the current stage; it grows with each stage.

Current stage: **1** (main window shell, master password and lock, Settings
with the Security section only, Hosts screen, host panel and group panel).

## How to run it

1. Build on a Mac (Apple silicon or Intel) from a clean checkout:

   ```sh
   pnpm install --frozen-lockfile
   pnpm --filter @shellpp/desktop tauri build --no-bundle
   ```

   This is the same release build as the `macos` CI job. It is not signed, so
   it is an **unsigned build**: "Remember on this device" must be hidden.
2. Start the app with a fresh, empty data directory, so that the first-run
   screen appears and nothing touches an existing installation:

   ```sh
   export SHELLPP_DATA_DIR="$(mktemp -d)"
   ./target/release/shellpp
   ```

   Section 9 is the only part that runs without `SHELLPP_DATA_DIR`.
3. Use only fictional test data (`CONTRIBUTING.md`, "Secrets and test data"):
   addresses from `example.com`, `example.org`, `192.0.2.0/24`,
   `198.51.100.0/24`, `203.0.113.0/24` or `2001:db8::/32`, and a master
   password generated for the run (16 or more characters). Never enter real
   hosts or credentials.
4. Work through the sections in order. Mark each item `[x]` (pass) or write
   `FAIL:` and a short note after it. A failed item blocks the stage until it
   is fixed or the owner accepts it.
5. Paste the filled-in list into the stage report or the pull request with
   this header:

   | Field | Value |
   |---|---|
   | Date | |
   | Commit | |
   | macOS version | |
   | Architecture | arm64 / x86_64 |
   | Build | unsigned release (`tauri build --no-bundle`) / signed |
   | Tester | |

Shortcuts below are written for macOS: Cmd replaces Ctrl for app actions
(`docs/ui-behavior-spec.md`, section 0).

## 1. Launch and first run (UI 13.1)

- [ ] The app starts without a crash dialog; the window opens centered at
      about 1280 x 800 and cannot be made smaller than about 900 x 560.
- [ ] The menu bar shows the app name "Shell++"; the Dock shows the app icon.
- [ ] The first-run screen is shown. Only the title bar drag area and the
      traffic lights are available; no tabs, sidebar or app content.
- [ ] Password and confirmation fields are masked; the show/hide control
      reveals and hides each value.
- [ ] Continue stays disabled until both fields are filled and equal.
- [ ] A password shorter than 10 characters shows an error under the field.
- [ ] The warning that lost passwords cannot be recovered is always visible
      and cannot be dismissed.
- [ ] The recovery key step shows the key in a monospace font, starting with
      `RK1-`.
- [ ] Copy puts the key on the clipboard (paste into TextEdit); about 30 s
      later the clipboard no longer holds it.
- [ ] Finish stays disabled until "I saved the recovery key" is checked;
      Finish opens the Vaults tab with the empty Hosts screen.
- [ ] Write the recovery key down for section 7 (fictional run only).

## 2. Title bar and window (UI 1.1)

- [ ] The system traffic lights (close, minimize, zoom) are on the left of the
      custom title bar; the window has no system title text.
- [ ] No in-app window buttons are drawn on the right, and there is no in-app
      app menu button (the native menu bar replaces it).
- [ ] App content starts to the right of the traffic lights; nothing is drawn
      under or over them, in both the light and the dark macOS appearance.
- [ ] Close, minimize and zoom work. Zoom toggles between the zoomed and the
      previous size.
- [ ] Dragging the empty title bar area moves the window; double-clicking it
      toggles zoom.
- [ ] The Vaults tab is present and active.
- [ ] The notification center button is present (section 3.5).
- [ ] Not rendered (later stages): vault switcher arrow next to Vaults, SFTP
      tab, session tabs, "+" new tab button, update button, terminal sidebar
      toggle.

## 3. Main window

### 3.1 Sidebar (UI 1.2)

- [ ] The sidebar shows the Hosts section, highlighted as active.
- [ ] Not rendered: Keychain, Port Forwarding, Snippets, Known Hosts, Logs.
- [ ] View ▸ Show/Hide Sidebar in the menu bar hides and shows the sidebar.

### 3.2 Details panel (UI 1.3)

- [ ] Selecting a host opens the panel on the right; the host list narrows
      and is not covered by the panel.
- [ ] The header shows the title ("Host details", "New host", "Group
      details" or "New group"), the "..." menu and the collapse button;
      collapse closes the panel.
- [ ] Dragging the panel edge resizes it between about 320 and 560 px; the
      width is kept after quitting and starting the app again.
- [ ] A new host with an empty address is a draft: closing the panel discards
      it without a prompt.
- [ ] An invalid value (for example port `70000`) is highlighted with the
      reason under the field and is not saved; after reopening the panel the
      last valid value is shown.

### 3.3 Native menu bar (UI 1.4)

- [ ] The menu bar holds every app menu item of UI 1.4: Vaults (Cmd+1),
      Settings (Cmd+,), Edit ▸, View ▸, Window ▸, Help ▸, Lock and Exit
      (Quit, Cmd+Q). Items that macOS places in standard menus (Settings and
      Quit in the app menu, Edit, View, Window, Help) may move there; within
      each submenu the order of UI 1.4 is kept. Shortcut hints use the macOS
      symbols.
- [ ] Edit ▸ Undo, Redo, Cut, Copy, Paste and Select All work in a text field
      of the host panel, both from the menu and with Cmd+Z, Shift+Cmd+Z,
      Cmd+X, Cmd+C, Cmd+V and Cmd+A.
- [ ] View ▸ Zoom In, Zoom Out and Reset Zoom change the interface zoom.
- [ ] View ▸ Full Screen enters and leaves macOS full screen.
- [ ] Window ▸ New Window opens a second window with the same data. A change
      in one window (for example a new host) appears in the other.
- [ ] Every menu item acts on the focused window.
- [ ] Help ▸ Documentation and Help ▸ Report an Issue open https pages in the
      default browser; Help ▸ About shows the app version.
- [ ] Vaults (Cmd+1) shows the Vaults tab; Settings (Cmd+,) opens Settings.
- [ ] Lock shows the unlock screen (section 7).
- [ ] Exit / Cmd+Q quits the app.
- [ ] Not present (later stages): SFTP (Cmd+2), New Local Terminal, New
      Serial Connection, Window ▸ next tab / previous tab / close tab,
      Help ▸ Check for Updates.
- [ ] In-app shortcut hints (context menus, tooltips) use Cmd, not Ctrl.

### 3.4 Notification center (UI 1.5)

Stage 1 has no notification source outside the e2e mocks; the list itself is
covered by the Linux and Windows e2e suite.

- [ ] The button opens an empty list with its empty-state text; no unread
      counter is shown.

## 4. Hosts screen (UI 2)

Create at least two groups (one nested) and three hosts for this section.

### 4.1 Search (UI 2.1)

- [ ] The search bar spans the width; its placeholder mentions host search and
      `ssh user@hostname`.
- [ ] Typing filters groups and hosts in real time by label, address, tag and
      username (including a username inherited from a group); results include
      hosts in nested groups; clearing the search restores the folder.
- [ ] A search with no match shows the no-results text.
- [ ] Not rendered: the Connect button in the search bar; Enter does not
      connect.

### 4.2 Action bar (UI 2.2)

- [ ] "New host" opens the host panel with focus in Address; its menu item
      "New Group" creates the group in the current folder and opens the group
      panel.
- [ ] The tiles/list toggle switches the layout; the choice is kept after a
      restart.
- [ ] The tag filter combines tags with AND; active tags show as chips that
      can be removed.
- [ ] Sort: Manual (default), Name, Created, Modified; the choice is kept
      after a restart.
- [ ] Not rendered: Import, cloud integrations, Terminal and Serial buttons,
      vault member avatars.

### 4.3 Content (UI 2.3)

- [ ] Groups are listed above hosts. Opening a group shows breadcrumbs; each
      breadcrumb returns to its level.
- [ ] A group card shows its icon, name and the number of hosts in its
      subtree.
- [ ] A host card shows the neutral OS icon, the label (or the address when
      there is no label) and a subtitle with protocol, username and group or
      tags; long text ends with an ellipsis.
- [ ] The grid adapts to the window width; the selected card is highlighted.
- [ ] Cmd+click toggles a card in the selection (the macOS convention for
      Ctrl+click); Shift+click selects a range; Ctrl+click opens the context
      menu, as a right click does. Record the actual behavior if it differs.
- [ ] Multi-selection offers Move to and Remove.
- [ ] In Manual sort, dragging a card reorders it and the order is kept after
      a restart; in Name sort, dragging does not reorder; dropping a host on a
      group card moves it into the group in any sort mode.
- [ ] An empty vault shows the empty state with "Create host"; an empty
      folder shows its own text.

### 4.4 Host card actions (UI 2.4)

- [ ] A click selects the card and opens its details; a double click does
      the same (connecting is stage 2).
- [ ] E opens the selected host with focus in Address; the arrow keys move the
      selection.
- [ ] The context menu (right click or two-finger click) lists, in order:
      Edit Host Details (E), Move to ▸, Duplicate, Copy Link with an (i)
      info icon, Remove.
- [ ] Move to ▸ moves the host into a group and back to the top level.
- [ ] Duplicate creates "<label> Copy" right after the original.
- [ ] Copy Link puts `shellpp://host/<id>` on the clipboard (paste into
      TextEdit).
- [ ] Remove asks for confirmation, removes the card and shows an undo toast
      for about 8 s; Undo restores the host with its password.
- [ ] Not rendered: Quick Connect, Connect ▸, Connect via SFTP, Add to
      Workspace, Collaborate, Move to another vault, Copy to ▸.

## 5. Host panel (UI 3.1-3.3)

- [ ] Address has the focus when the panel opens for a new host and shows the
      neutral OS icon.
- [ ] The host appears in the list as soon as a valid address is typed; there
      is no save button; edits are kept after a restart.
- [ ] General: Label; Parent Group (search, and creating a group on the fly);
      Tags (Enter or comma adds a chip, autocomplete from other hosts, chips
      can be removed); Color (8 colors, graphite by default); Notes (several
      lines); Backspace (Default, `^?`, `^H`).
- [ ] SSH: the port field (placeholder 22) is on the same line as "SSH";
      username; password masked, and show reveals the saved value.
- [ ] Port and username inherited from a group show as placeholders
      "<value> (from group <name>)".
- [ ] Show more: Agent Forwarding, Environment Variables (a duplicate name is
      an error), Charset (UTF-8 default), Keepalive (empty = global; a value
      out of range is an error), Record history, Record output, terminal theme
      (preview, name, picker with default, Graphite and Paper); all are kept
      after a restart.
- [ ] The "..." menu lists Duplicate and Remove.
- [ ] Not rendered: vault field, Share this host, credentials "+" and linked
      key, identity merge hint, startup snippet, Host Chaining, Proxy, Mosh,
      Add Telnet, Connect (button and "..." item).

## 6. Group panel (UI 3.4)

- [ ] A group can be created with only a name; an empty name shows the
      required-field error.
- [ ] Parent Group search excludes the group itself and its subgroups.
- [ ] Color can be chosen and is kept.
- [ ] SSH username and port are inherited by hosts and subgroups, shown as
      "from group" placeholders; the nearest group wins.
- [ ] The group menu (right click on a group card, and the panel "..." menu)
      lists Open, Edit, Move to ▸, Remove.
- [ ] Removing an empty group asks for confirmation; removing a non-empty
      group says that its subgroups and hosts move one level up, and they do.
- [ ] Not rendered: vault field, credentials (identity or key), Host Chaining.

## 7. Unlock, lock and auto-lock (UI 13.2, 13.3)

- [ ] After quitting and starting again (same `SHELLPP_DATA_DIR`), the unlock
      screen covers the window: app name, the "Master password" field with
      focus, Unlock and "Use recovery key". Only the title bar and traffic
      lights remain.
- [ ] A wrong password shows an error under the field, clears it and keeps the
      focus; Enter with the right password unlocks.
- [ ] "Use recovery key": the key from section 1 plus a new password and its
      confirmation unlock; after a restart the old password fails and the new
      one works.
- [ ] Lock in the menu bar locks every open window at once (open a second
      window first).
- [ ] After unlocking, each window returns to its previous state: folder,
      selection, open panel and Settings if it was open; search text and
      revealed passwords are gone.
- [ ] Auto-lock: with 1 minute set in Settings ▸ Security, the app locks after
      a minute without keyboard or mouse input in any app window; input in any
      window restarts the timer; "Never" turns auto-lock off.

## 8. Settings (UI 12.1, 13.4)

- [ ] Settings opens with Cmd+, and from the menu bar as a modal over the
      window; X and Esc close it.
- [ ] The navigation on the left shows only Security; navigation and content
      scroll independently.
- [ ] Below the navigation: Documentation, Report a Bug and Changelog open
      https pages in the default browser; the app version is shown at the
      bottom.
- [ ] Not rendered: Account & Sync, SSH ID, Team, Terminal, SFTP, Logs,
      Shortcuts, Updates, the theme dropdown.
- [ ] Auto-lock offers 1, 5, 15 (default), 30 and 60 minutes and Never; a
      change applies immediately and is kept after reopening Settings.
- [ ] Change master password: a wrong current password, a mismatched
      confirmation and a too short password each show an error; a valid
      change works immediately and after a restart.
- [ ] Recovery key ▸ Show asks for the master password and shows the current
      key; Reissue asks for it, shows a new key with the required "I saved
      it" checkbox, and the old key no longer works in "Use recovery key".
- [ ] **Unsigned build: "Remember on this device" is not shown**
      (`docs/decisions.md`, decision 7; UI 13.4).
- [ ] Biometric unlock is not shown.
- [ ] Not rendered: the warning about signing out other devices.

### 8.1 Signed build only

Run these only with a build signed with a Developer ID certificate and built
with the `SHELLPP_MACOS_SIGNED` compile-time constant (stage 9 release
workflow). Skip and write "no signed build" otherwise.

- [ ] "Remember on this device" is shown, off by default; turning it on shows
      the warning that any program of this macOS user can unlock the data.
- [ ] Keychain Access shows one item with service `shellpp` and an account
      starting with `device-key:`.
- [ ] After a restart the app unlocks without the password; Lock and
      auto-lock still ask for it.
- [ ] Turning the option off removes the Keychain item.

## 9. Files and logs

Quit the app, unset `SHELLPP_DATA_DIR`, start the app once, complete the
first run with a fictional password, create one host, and quit.

- [ ] The data directory is `~/Library/Application Support/shellpp/` with mode
      `drwx------`; its files are `-rw-------` (`ls -la`).
- [ ] Diagnostic logs are in `~/Library/Logs/shellpp/`.
- [ ] `grep` of the log files for the test host address, the username, the
      host password and the master password finds nothing.
- [ ] Remove `~/Library/Application Support/shellpp/` and
      `~/Library/Logs/shellpp/` afterwards (test data only).
