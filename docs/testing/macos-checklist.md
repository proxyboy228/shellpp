# macOS acceptance checklist

tauri-driver has no macOS support, so the UI e2e suite runs only on Linux and
Windows (`docs/decisions.md`, decision 6). On macOS a screen is accepted with
the CI build, the Rust tests and this manual checklist
(`docs/architecture.md`, section 9.1). The checklist covers every element that
is visible in the current stage; it grows with each stage.

Current stage: **2** (stage 1, the main window shell, master password and
lock, Settings with the Security section only, Hosts screen, host panel and
group panel, plus SSH terminal sessions with known hosts, session tabs,
splits, the terminal sidebar, the new tab page and search, Known Hosts, the
local terminal, quick connect, the system agent and agent forwarding).

## How to run it

1. Build on a Mac (Apple silicon or Intel) from a clean checkout:

   ```sh
   pnpm install --frozen-lockfile
   pnpm --filter @shellpp/desktop tauri build --no-bundle
   ```

   This is the same release build as the `macos` CI job. It is not signed, so
   it is an **unsigned build**: "Remember on this device" must be hidden.
2. Start the app from Terminal with a fresh, empty data directory, so that the
   first-run screen appears and nothing touches an existing installation.
   Started this way, the app inherits the `SSH_AUTH_SOCK` of the login session
   (the system ssh-agent, section 13):

   ```sh
   export SHELLPP_DATA_DIR="$(mktemp -d)"
   ./target/release/shellpp
   ```

   Section 16 is the only part that runs without `SHELLPP_DATA_DIR`.
3. Start the test SSH servers (below) in other Terminal windows.
4. Use only fictional test data (`CONTRIBUTING.md`, "Secrets and test data"):
   addresses from `example.com`, `example.org`, `192.0.2.0/24`,
   `198.51.100.0/24`, `203.0.113.0/24` or `2001:db8::/32`, the local test SSH
   servers on `127.0.0.1`, and a master password generated for the run (16 or
   more characters). Never enter real hosts or credentials.
5. Work through the sections in order. Mark each item `[x]` (pass) or write
   `FAIL:` and a short note after it. A failed item blocks the stage until it
   is fixed or the owner accepts it. Items that say "record" ask for the
   observed behavior; write it after the item.
6. Paste the filled-in list into the stage report or the pull request with
   this header:

   | Field | Value |
   |---|---|
   | Date | |
   | Commit | |
   | macOS version | |
   | Architecture | arm64 / x86_64 |
   | Build | unsigned release (`tauri build --no-bundle`) / signed |
   | Tester | |

Shortcuts below are written for macOS: Cmd replaces Ctrl for app actions,
Ctrl+letter always goes to the shell and Option is not used for app shortcuts
(`docs/ui-behavior-spec.md`, sections 0 and 12.5).

### Test SSH servers

SSH items use the project's russh test server (`tests/sshd`,
`shellpp-test-sshd`) on `127.0.0.1`. Its built-in shell is deterministic:
`echo <text>`, `exit [n]`, `term`, `size`, `lines <n>`, `read-secret`,
`alt-screen`, `agent-keys`, `later <s> <text>`, `bell`, `url`,
`title <text>`, `unicode`, `color`, `drop` (`tests/sshd/src/shell.rs`). The
server offers every authentication method that any of its users has, so the
keyboard-interactive user gets a server of its own. Generate the credentials
and the configurations once:

```sh
cargo build -p shellpp-test-sshd
export TEST_DIR="$(mktemp -d)"
export TEST_PASSWORD="$(openssl rand -hex 12)"
export TEST_KEY_PASSPHRASE="$(openssl rand -hex 12)"
export TEST_CODE_1="$(openssl rand -hex 3)" TEST_CODE_2="$(openssl rand -hex 3)"
./target/debug/shellpp-test-sshd keygen --type ed25519 \
  --out "$TEST_DIR/id_ed25519" --passphrase-env TEST_KEY_PASSPHRASE
cat > "$TEST_DIR/sshd.json" <<EOF
{"users": [{"name": "tester", "password": "$TEST_PASSWORD",
            "authorized_keys": ["$(cat "$TEST_DIR/id_ed25519.pub")"]}],
 "banner_comment": "Ubuntu-3ubuntu13"}
EOF
cat > "$TEST_DIR/otp.json" <<EOF
{"users": [{"name": "otp", "ki_name": "Test server", "ki_instruction": "Answer both codes.",
            "keyboard_interactive": [
              {"prompt": "First code: ", "echo": false, "answer": "$TEST_CODE_1"},
              {"prompt": "Second code: ", "echo": true, "answer": "$TEST_CODE_2"}]}]}
EOF
env | grep '^TEST_'   # the generated values to type into the app
```

Start the main server in this window, and the keyboard-interactive server in
a second Terminal window (there, `cd` to the checkout and
`export TEST_DIR=<the value printed above>` first):

```sh
./target/debug/shellpp-test-sshd serve --config "$TEST_DIR/sshd.json"   # main server
./target/debug/shellpp-test-sshd serve --config "$TEST_DIR/otp.json"    # keyboard-interactive
```

Each server prints one line with `"port"` and `"host_key_fingerprint"`, then
reads commands from its terminal: `rotate-host-key`, `drop`, `accept off`,
`accept on`, `events` and `quit`. It stops when its terminal closes. Below,
`<port>` is the main server's port.

If the system ssh-agent holds keys, the app offers them to the test server
before the password (only public keys leave the agent, and only to
`127.0.0.1`); the server refuses them.

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
- [ ] Write the recovery key down for section 14 (fictional run only).

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
- [ ] The Vaults tab is present and active; the "+" new tab button follows
      it. Session tabs and the terminal sidebar toggle are checked in
      sections 8 and 9.
- [ ] The notification center button is present (section 3.4).
- [ ] Not rendered (later stages): vault switcher arrow next to Vaults, SFTP
      tab, update button.

## 3. Main window

### 3.1 Sidebar (UI 1.2)

- [ ] The sidebar shows the Hosts and Known Hosts sections; the current one
      is highlighted.
- [ ] Not rendered: Keychain, Port Forwarding, Snippets, Logs.
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

- [ ] The menu bar holds every app menu item of UI 1.4 that is visible in
      this stage: Vaults (Cmd+1), Settings (Cmd+,), New Local Terminal,
      Edit ▸, View ▸, Window ▸ (new window, next tab, previous tab, close
      tab), Help ▸, Lock and Exit (Quit, Cmd+Q). Items that macOS places in
      standard menus (Settings and Quit in the app menu, Edit, View, Window,
      Help) may move there; within each submenu the order of UI 1.4 is kept.
      Shortcut hints use the macOS symbols.
- [ ] Edit ▸ Undo, Redo, Cut, Copy, Paste and Select All work in a text field
      of the host panel, both from the menu and with Cmd+Z, Shift+Cmd+Z,
      Cmd+X, Cmd+C, Cmd+V and Cmd+A.
- [ ] View ▸ Zoom In, Zoom Out and Reset Zoom change the interface zoom; the
      terminal stays sharp and refits after a zoom change.
- [ ] View ▸ Full Screen enters and leaves macOS full screen.
- [ ] Window ▸ New Window opens a second window with the same data. A change
      in one window (for example a new host) appears in the other. Session
      tabs are per window: the second window has none of the first window's
      tabs.
- [ ] Every menu item acts on the focused window.
- [ ] New Local Terminal opens a local terminal tab (section 12); so do
      Cmd+L outside a terminal and Cmd+Shift+L everywhere.
- [ ] Window ▸ Next Tab and Previous Tab switch between the session tabs of
      the focused window; Window ▸ Close Tab and Cmd+Shift+W close the active
      tab. Option+arrows do not switch tabs (Option types characters).
- [ ] Help ▸ Documentation and Help ▸ Report an Issue open https pages in the
      default browser; Help ▸ About shows the app version.
- [ ] Vaults (Cmd+1) shows the Vaults tab; Settings (Cmd+,) opens Settings.
- [ ] Lock shows the unlock screen (section 14).
- [ ] Exit / Cmd+Q quits the app; with an open session it asks first
      (section 9.3).
- [ ] Not present (later stages): SFTP (Cmd+2), New Serial Connection,
      Help ▸ Check for Updates.
- [ ] In-app shortcut hints (context menus, tooltips, the new tab page) use
      Cmd, not Ctrl.

### 3.4 Notification center (UI 1.5)

Stage 2 has no notification source outside the e2e mocks; the list itself is
covered by the Linux and Windows e2e suite.

- [ ] The button opens an empty list with its empty-state text; no unread
      counter is shown.

## 4. Hosts screen (UI 2)

Create at least two groups (one nested) and three hosts for this section.
One of them is "Test server": address `127.0.0.1`, the main test server's
port, username `tester` and the generated password (`TEST_PASSWORD`).

### 4.1 Search and quick connect (UI 2.1)

- [ ] The search bar spans the width; its placeholder mentions host search and
      `ssh user@hostname`.
- [ ] Typing filters groups and hosts in real time by label, address, tag and
      username (including a username inherited from a group); results include
      hosts in nested groups; clearing the search restores the folder.
- [ ] A search with no match shows the no-results text.
- [ ] The Connect button on the right of the search bar is disabled for a
      single word such as `web`, and enabled for `host.example.com`,
      `127.0.0.1:<port>`, `tester@127.0.0.1:<port>`,
      `ssh tester@127.0.0.1 -p <port>` and for the exact label of a saved
      host (`<port>`: the main test server).
- [ ] Return or Connect with `127.0.0.1:<port>` opens a session tab without
      saving a host: the username is asked, and the password form has no
      "Save" checkbox. After connecting, a toast offers to save the host;
      its action creates the host and opens its panel.
- [ ] The exact label of a saved host connects to that host with its settings
      (no prompt when its password is saved) and shows no save offer.

### 4.2 Action bar (UI 2.2)

- [ ] "New host" opens the host panel with focus in Address; its menu item
      "New Group" creates the group in the current folder and opens the group
      panel.
- [ ] The Terminal button opens a local terminal in a new tab (section 12).
- [ ] The tiles/list toggle switches the layout; the choice is kept after a
      restart.
- [ ] The tag filter combines tags with AND; active tags show as chips that
      can be removed.
- [ ] Sort: Manual (default), Name, Created, Modified; the choice is kept
      after a restart.
- [ ] Not rendered: Import, cloud integrations, Serial button, vault member
      avatars.

### 4.3 Content (UI 2.3)

- [ ] Groups are listed above hosts. Opening a group shows breadcrumbs; each
      breadcrumb returns to its level.
- [ ] A group card shows its icon, name and the number of hosts in its
      subtree.
- [ ] A host card shows the neutral OS icon until the first connection, the
      label (or the address when there is no label) and a subtitle with
      protocol, username and group or tags; long text ends with an ellipsis.
- [ ] After a connection to the test server (banner comment
      `Ubuntu-3ubuntu13`) the card shows the generic Linux icon (never a
      distribution or vendor logo); it stays after a restart.
- [ ] The grid adapts to the window width; the selected card is highlighted.
- [ ] Cmd+click toggles a card in the selection (the macOS convention for
      Ctrl+click); Shift+click selects a range; Ctrl+click opens the context
      menu, as a right click does. Record the actual behavior if it differs.
- [ ] Multi-selection offers Connect, Move to and Remove.
- [ ] In Manual sort, dragging a card reorders it and the order is kept after
      a restart; in Name sort, dragging does not reorder; dropping a host on a
      group card moves it into the group in any sort mode.
- [ ] An empty vault shows the empty state with "Create host"; an empty
      folder shows its own text.

### 4.4 Host card actions (UI 2.4)

- [ ] A click selects the card and opens its details; a double click connects
      in a new session tab.
- [ ] Option+double click opens the session in a new window (UI 2.4 names
      Alt, which is Option on a Mac keyboard). Record the behavior.
- [ ] Return on a selected card connects; with several cards selected, Return
      (and Connect in the context menu) opens one tab per host.
- [ ] E opens the selected host with focus in Address; the arrow keys move the
      selection.
- [ ] The context menu (right click or two-finger click) lists, in order:
      Quick Connect (with the Return hint), Connect ▸, Edit Host Details (E),
      Move to ▸, Duplicate, Copy Link with an (i) info icon, Remove.
- [ ] Connect ▸ holds "via SSH", a separator and a hint about holding Option
      (Alt) to open a new window; holding Option while choosing "via SSH"
      opens a new window. Record the hint text.
- [ ] Move to ▸ moves the host into a group and back to the top level.
- [ ] Duplicate creates "<label> Copy" right after the original.
- [ ] Copy Link puts `shellpp://host/<id>` on the clipboard (paste into
      TextEdit).
- [ ] Remove asks for confirmation, removes the card and shows an undo toast
      for about 8 s; Undo restores the host with its password.
- [ ] Not rendered: Connect via SFTP, Add to Workspace, Collaborate, Move to
      another vault, Copy to ▸.

## 5. Host panel (UI 3.1-3.3)

- [ ] Address has the focus when the panel opens for a new host; the address
      row shows the host's OS icon (neutral until detected).
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
- [ ] Credentials "+" opens a menu with Key only. The key dialog lists the
      existing keys, accepts pasted key text (Cmd+V) and "choose a file"
      opens the macOS file picker; the encrypted test key
      (`$TEST_DIR/id_ed25519`; in the file picker Cmd+Shift+G opens a path)
      asks for its passphrase with "Save passphrase" (off by default). After
      adding, the linked key field shows the key label and type, and the "+"
      is hidden; Unlink removes the key and brings the "+" back.
- [ ] The Connect button is pinned to the bottom: disabled for a draft,
      connects for a saved host.
- [ ] Show more: Agent Forwarding, Environment Variables (a duplicate name is
      an error), Charset (UTF-8 default), Keepalive (empty = global; a value
      out of range is an error), Record history, Record output, terminal theme
      (preview, name, picker with default, Graphite and Paper); all are kept
      after a restart.
- [ ] The "..." menu lists Connect, Duplicate and Remove.
- [ ] Not rendered: vault field, Share this host, identity, certificate and
      FIDO2 in the "+" menu, identity merge hint, startup snippet, Host
      Chaining, Proxy, Mosh, Add Telnet.

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

## 7. SSH sessions (UI 4.1, 4.8, 10.2)

Use the host "Test server" of section 4. For the first-connection items,
remove its Known Hosts entry first if section 4 already connected.

### 7.1 Connecting and the host key

- [ ] A double click opens a session tab with the connection screen: host
      name, address with user and port, the current step, Cancel and the
      "connecting" status dot.
- [ ] First connection: the host key dialog appears inside the tab with the
      address `[127.0.0.1]:<port>`, key type `ssh-ed25519` and the fingerprint
      of the server's first line; "Accept and save" is the default button
      (Return).
- [ ] "Connect once" connects and stores nothing: the next connection asks
      again. "Accept and save" stores the key: the next connection does not
      ask.
- [ ] Cancel at the host key step shows the failure state with Retry and
      Close.
- [ ] After connecting, the tab shows only server output (the line "Welcome to
      shellpp-test-sshd", then the prompt `tester@shellpp-test:~$ `), without
      an app banner. The terminal fills the area below the title bar with a
      small padding and no Vaults sidebar.
- [ ] Focus is in the terminal at once: typing `echo hi` and Return prints
      `hi`. UTF-8 input (`echo é ✓`) is echoed correctly.
- [ ] `term` prints `xterm-256color`. `size` prints the terminal size and
      changes after resizing the window, entering full screen or changing the
      interface zoom.
- [ ] Changed key: type `rotate-host-key` in the server terminal and connect
      again. The connection is blocked with a warning in the danger color,
      the old and new fingerprints and an explanation of a possible attack;
      there is no "don't ask again" checkbox. Cancel ends in the failure
      state; "Replace saved key" connects, and Known Hosts shows the new
      fingerprint.

### 7.2 Prompts

- [ ] Clear the saved password of the host and connect: the password form
      appears in the tab (masked, Return = Continue, Cancel, "Save" off by
      default). A wrong password shows an error in the same form; the right
      one connects.
- [ ] With "Save" checked, the next connection does not ask.
- [ ] Quick connect to `127.0.0.1:<port>` asks for the username first.
- [ ] Keyboard-interactive: quick connect to `otp@127.0.0.1:<otp port>` (the
      second server). The server's name "Test server" and its instruction are
      shown in a frame above the question; the questions come one at a time;
      "First code" (`TEST_CODE_1`) is masked and "Second code"
      (`TEST_CODE_2`) is visible; right answers connect.
- [ ] A host with the linked encrypted key whose passphrase is not saved asks
      for the passphrase; a wrong one shows an error, the right one connects.
- [ ] Failure: type `accept off` in the server terminal and connect. The
      screen shows the reason (connection refused) with Retry and Close and
      makes no automatic attempts. After `accept on`, Retry connects.
- [ ] Cancelling a prompt ends in the failure state.

## 8. Terminal (UI 4.2, 4.3, 12.5)

### 8.1 Rendering in WKWebView

- [ ] Text is sharp on Retina and non-Retina displays (if available), in the
      Graphite and Paper themes; moving the window between displays keeps it
      sharp.
- [ ] `lines 5000` scrolls smoothly; the scrollback can be scrolled with the
      trackpad, and the scroll bar follows the theme.
- [ ] `unicode` shows `wide: 日本語 ✓ 👍` with the wide characters in two cells
      each; `color` shows 16 distinct colors.
- [ ] `alt-screen` switches to the alternate screen; `q` returns to the normal
      screen with the earlier output intact.
- [ ] Optional, debug build (`pnpm --filter @shellpp/desktop tauri build
      --debug --no-bundle`): in Web Inspector (Cmd+Option+I), the
      `terminal-view` element has `data-renderer="webgl"`, or `"dom"` when
      WebGL is not available. Record the value.

### 8.2 Keyboard (Mac set)

- [ ] Ctrl+C interrupts: after `sleep 30`, Ctrl+C prints `^C` and a new
      prompt. Ctrl+D on an empty line ends the session.
- [ ] Ctrl+L, Ctrl+K and Ctrl+P go to the shell; no app action fires.
- [ ] Option types characters (for example Option+e, then e gives `é` on the
      US layout) and fires no app shortcut.
- [ ] While the terminal has focus: Cmd+1 shows Vaults, Cmd+2...9 the session
      tabs from the left; Cmd+, opens Settings.
- [ ] Cmd+K on the Hosts screen and Cmd+Shift+K from a focused terminal open
      the search palette (section 10). Record what Cmd+K does while the
      terminal has focus.
- [ ] Cmd+Shift+C copies the selection (paste into TextEdit); Cmd+Shift+V
      pastes; Cmd+Shift+A selects all; Cmd+Shift+F opens the terminal sidebar
      on Tools with the search field focused.
- [ ] Record whether Edit ▸ Copy and Paste (Cmd+C, Cmd+V) copy and paste in
      the terminal.
- [ ] `url` prints `https://example.com/docs`; Cmd+click opens it in the
      default browser, a plain click does not.
- [ ] `bell` makes no sound (bell off by default); `title Test` does not
      change the tab title.

### 8.3 Mouse and paste (UI 4.3)

- [ ] A right click (or two-finger click) pastes a single line from the
      clipboard directly.
- [ ] Copy two lines from TextEdit; a right click opens the paste
      confirmation with a preview and the line count. Paste sends both lines,
      Cancel sends nothing.

### 8.4 Terminal sidebar (UI 4.2)

- [ ] The sidebar toggle at the right of the title bar is shown only while a
      session tab is active. The sidebar opens on the right and the terminal
      narrows (`size` reports fewer columns) instead of being covered.
- [ ] Tabs: Tools, History, Appearance. Not rendered: Snippets tab.
- [ ] Tools: after `lines 200`, searching `line 150` highlights the matches
      and shows their count; next and previous move between them. Not
      rendered: Autocomplete.
- [ ] History: after `echo one` and `echo two`, both are listed newest first.
      A click inserts the command into the terminal without running it.
- [ ] Not recorded: the secret typed after `read-secret`, a line typed while
      `alt-screen` is active, `export TOKEN=abc123`.
- [ ] An entry can be deleted; Clear removes the host's history after a
      confirmation. Not rendered: the "save as snippets" hint card.
- [ ] Appearance: the Font row leads to family, size and line spacing; the
      size buttons change the terminal at once and the size is kept after a
      restart. The themes Graphite and Paper have previews; the current one
      is highlighted; Paper applies at once and the host panel shows Paper
      afterwards.

## 9. Tabs, splits and session end (UI 1.1, 4.4, 4.6, 4.7)

### 9.1 Session tabs

- [ ] Session tabs appear to the right of the Vaults tab, never under the
      traffic lights, also in full screen and with ten or more tabs.
- [ ] A tab shows the OS icon and the host name; on the active tab and on
      hover the close button replaces the icon; a terminal icon is on the
      right of the tab.
- [ ] The active tab is marked with the accent bar, the same with the Paper
      theme (never a terminal theme color).
- [ ] The title bar between and after the tabs still moves the window and
      double-click still zooms; clicks on tabs do not move the window.
- [ ] The tab context menu (right click) lists, in order: Duplicate,
      Duplicate in a new window, Rename, Split horizontally, Split vertically,
      Close. Not rendered: Start multiplayer, Broadcast input.
- [ ] Duplicate opens a second session to the same host; Duplicate in a new
      window opens it in a new window; Rename changes only the tab title (the
      host label stays).

### 9.2 Splits

- [ ] Cmd+Shift+D adds a pane to the right, Cmd+Shift+E a pane below; each
      new pane shows the host picker with the search focused; choosing a
      recent host connects in that pane.
- [ ] Typing in one pane does not appear in the other; the divider can be
      dragged; the active pane is highlighted.
- [ ] Cmd+Option+arrows move the focus between panes; a click focuses a pane.
- [ ] `exit` in a pane closes only that pane.

### 9.3 Session end and reconnect

- [ ] `exit` closes the tab. With three session tabs, closing the middle one
      activates the left one, closing the leftmost activates the right one;
      closing the last one shows Vaults.
- [ ] `exit 3` keeps the tab with the output and "Session ended with code 3",
      Reconnect and Close; no automatic attempts.
- [ ] `drop` in the session, or `drop` in the server terminal: the output
      stays, the reason is shown with a countdown, and the automatic
      reconnect succeeds in the same tab.
- [ ] `accept off`, then `drop` in the server terminal: the attempts fail and
      the countdown grows (1, 2, 4 s ...); Cancel stops them; after
      `accept on`, Reconnect works.
- [ ] Cmd+Q with an open session asks for confirmation; Cancel keeps the app
      running. The red close button of a window with a session asks as well.

## 10. New tab page and search (UI 6)

- [ ] "+" opens a tab named "New Tab" with the focus in the search field and
      the hint "Cmd+K".
- [ ] Recent connections lists the test host with its OS icon, name and the
      path "Personal / <group>"; a click connects in this tab, and so do the
      arrow keys and Return.
- [ ] Typing lists the open tabs of this window first, then hosts.
- [ ] Restore is disabled at the first launch; after quitting with session
      tabs open and starting again, Restore reopens them.
- [ ] The search palette (Cmd+K) offers the same results; Esc closes it.
- [ ] Not rendered: Create a workspace, the workspaces block.

## 11. Known Hosts (UI 10.1)

- [ ] Known Hosts in the sidebar opens the screen; the test server is listed
      as `[127.0.0.1]:<port>` with its key type.
- [ ] A click opens the details panel: address, port, key type, SHA256
      fingerprint (monospace) with a copy button (paste into TextEdit), date
      added and the saved host with this address (a click opens it on the
      Hosts screen).
- [ ] The context menu (right click or two-finger click) offers Copy
      fingerprint and Remove; Remove asks for confirmation, and the next
      connection asks for the host key again.
- [ ] Search, the view toggle and the sort are kept after a restart.
- [ ] Not rendered: Import, Export.

## 12. Local terminal (UI 2.2, 12.4)

- [ ] The Terminal button, New Local Terminal in the menu bar and
      Cmd+Shift+L each open a local terminal tab with the user's shell
      (`echo $SHELL`, zsh by default) in the home directory.
- [ ] It is a login shell: in zsh `[[ -o login ]] && echo login` prints
      `login` (in bash: `shopt -q login_shell && echo login`).
- [ ] `echo $TERM` prints `xterm-256color` and `echo $COLORTERM` prints
      `truecolor`.
- [ ] `top` and `vim` (full-screen programs) work and reflow when the window
      is resized; quitting them restores the prompt.
- [ ] Ctrl+C interrupts `sleep 30`; `exit` closes the tab; `exit 3` shows
      "Session ended with code 3".
- [ ] Commands typed in the local terminal appear in the History tab.

## 13. System agent and agent forwarding

- [ ] In Terminal, add the test key to the system agent:
      `ssh-add "$TEST_DIR/id_ed25519"` (passphrase: `TEST_KEY_PASSPHRASE`);
      `ssh-add -l` lists it.
- [ ] A host for `tester` without a saved password and without a linked key
      connects with no prompt (the key comes from the agent through
      `SSH_AUTH_SOCK`).
- [ ] With Agent Forwarding on (host panel, Show more), `agent-keys` in the
      session prints `agent keys: 1`; with it off, `agent: not forwarded`.
- [ ] After `ssh-add -D` the same host asks for the password.
- [ ] Afterwards remove the test key from the agent (`ssh-add -D`, or
      `ssh-add -d "$TEST_DIR/id_ed25519"`).

## 14. Unlock, lock and auto-lock (UI 13.2, 13.3)

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

### 14.1 Lock with an open session

- [ ] In a session, run `later 5 after-lock` and lock at once: the window
      shows only the unlock screen; no session tabs and no terminal content
      are visible.
- [ ] After unlocking, the session tab is back and `after-lock` is in the
      terminal: the session kept running while locked.
- [ ] A password form that is open when the app locks is cancelled; the
      attempt shows the failure state after unlocking.
- [ ] Lock, then `drop` in the server terminal: no reconnect attempt and no
      prompt while locked; after unlocking, the reconnect resumes and
      succeeds.

## 15. Settings (UI 12.1, 13.4)

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

### 15.1 Signed build only

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

## 16. Files and logs

Quit the app, unset `SHELLPP_DATA_DIR`, start the app once, complete the
first run with a fictional password, create one host, connect to the test
server once (run `echo shellpp-marker`), and quit.

- [ ] The data directory is `~/Library/Application Support/shellpp/` with mode
      `drwx------`; its files are `-rw-------` (`ls -la`).
- [ ] Diagnostic logs are in `~/Library/Logs/shellpp/`.
- [ ] `grep` of the log files for the test host address, the usernames
      (`tester`, `otp`), the host password, the key passphrase, the
      keyboard-interactive codes, the master password and `shellpp-marker`
      finds nothing.
- [ ] `grep -r shellpp-marker` in the data directory finds nothing (history
      is stored encrypted).
- [ ] Stop the test servers (`quit` in their terminals) and remove `$TEST_DIR`,
      `~/Library/Application Support/shellpp/` and `~/Library/Logs/shellpp/`
      afterwards (test data only).
