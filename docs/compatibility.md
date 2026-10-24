# Compatibility

What Shell++ was checked against, how, and what is known not to work.
Servers: section 1 (run 4, plan 9.1; D80). Terminal programs: section 2
(run 4, plan 9.2).

## 1. SSH servers

Each server runs in its own Docker container from `tests/openssh`
(`start.sh`, with `SHELLPP_OPENSSH_IMAGE` for the image and
`SHELLPP_OPENSSH_VARIANT` for a variant; README.md there). The suite is
`crates/ssh/tests/compat.rs`; CI runs it for every row (job "SSH
integration").

| Server | Image | Password | Key | Keyboard-interactive | Terminal | SFTP | Local forward | Notes |
|---|---|---|---|---|---|---|---|---|
| OpenSSH 10.0p2 (Debian 13) | `tests/openssh/Dockerfile` | yes | yes | yes | yes | yes | yes | the main test server of every suite |
| OpenSSH 7.4 (CentOS 7) | `compat/centos7.Dockerfile` | yes | yes | yes | yes | yes | yes | Ed25519 host key, curve25519, AES-GCM and ChaCha20 already there |
| Dropbear 2024.85 (Alpine 3.20) | `compat/dropbear.Dockerfile` | yes | yes | — | yes | yes (OpenSSH's sftp-server) | yes | Dropbear has no keyboard-interactive authentication |
| Only `ssh-rsa` (SHA-1) host keys, `diffie-hellman-group14-sha1`, `aes128-cbc` and `hmac-sha1` | variant `legacy-sha1` | with the host setting | with the host setting | with the host setting | yes | yes | yes | without the setting the connection fails with the mismatch error below |
| Key plus a keyboard-interactive second factor | variant `kbd-2fa` | — | yes, then the question | yes | yes | yes | yes | `AuthenticationMethods publickey,keyboard-interactive` |
| No SFTP subsystem | variant `no-sftp` | yes | yes | yes | yes | no: "The server refused to open a session." | yes | the terminal and forwarding work as usual |
| A banner before login and a 120-line message of the day | variant `motd-banner` | yes | yes | yes | yes | yes | yes | the banner reaches the connection's interaction, the message of the day the terminal |

Each check is: password login (`pwuser`), key login (`keyuser`, Ed25519),
keyboard-interactive (`kiuser`, a masked question), a shell with a computed
marker, `stty size` and a UTF-8 line both ways, the home folder over SFTP,
and a Local rule to the server's own port 22 that reads the SSH greeting.

### Legacy algorithms

Off by default; the host setting "Allow legacy algorithms" (Show more of
the host panel) turns them on for that host only, with a warning shown
while it is on (D80). Then Shell++ also offers, after all modern ones of
each list:

- key exchange: `diffie-hellman-group14-sha1`,
  `diffie-hellman-group-exchange-sha1`;
- host key signatures: `ssh-rsa` (SHA-1);
- RSA user key signatures with SHA-1, only for a server that accepts no
  `rsa-sha2-*` (or names none, as OpenSSH before 7.2);
- MACs: `hmac-sha1-etm@openssh.com`, `hmac-sha1`;
- ciphers: `aes256-cbc`, `aes192-cbc`, `aes128-cbc`.

Never offered: 3DES, RC4 (`arcfour*`), Blowfish, CAST, `ssh-dss`, the
1024-bit `diffie-hellman-group1-sha1`, MD5 and 96-bit MACs, `umac-*`.

Not covered by the matrix: an RSA user key signing with SHA-1 (the test
users have Ed25519 keys); its path is the same as the SHA-2 one with the
hash left out.

### When nothing matches

The failure screen names the negotiation ("no key exchange algorithm in
common", or host key, cipher, MAC, compression), then "The server offers:
…" and "Shell++ supports: …". When the server offers a legacy algorithm, it
adds how to turn the host setting on (for the jump host when the failure
names one; Quick connect never offers them, so the host is saved first).
The server's list shows only names
Shell++ knows (others are counted, "and 2 others"): no text the server sent
is shown or logged.

### Not checked

- Servers that need a real device (network equipment, embedded systems):
  the legacy-sha1 variant stands in for their algorithm lists.
- Tectia, Bitvise and the Windows OpenSSH server: no Linux container of
  them.
- GSSAPI/Kerberos authentication: not supported.

## 2. Terminal programs

Checked in an SSH session to the OpenSSH test image with the terminal
programs (`tests/openssh/programs.Dockerfile`, D83; bash,
`TERM=xterm-256color`, `LANG=C.UTF-8`) at 1280x800, in both themes: the
screenshot states `terminal-tmux`, `terminal-tmux-resize`, `terminal-mc`,
`terminal-vim`, `terminal-nano`, `terminal-less`, `terminal-emacs`,
`terminal-vttest`, `terminal-vttest-132` and `terminal-unicode` (`scripts/ui-screens.ps1 -Sizes
1280x800 -States …`, OpenSSH only; files in `target\ui-screens\`). Each
state checks what the program shows before the capture.

| Program | Result |
|---|---|
| tmux 3.5a | two panes, status line, colors, the pane border; after the app's pane is split beside it, tmux draws again for the new size with its status line on the last row |
| mc 4.8 | panels, frames (line drawing), menu bar, function key bar, colors |
| vim 9.1 | syntax colors (light and dark), `~` lines, the ruler |
| nano 8.4 | title bar, syntax colors, the two help rows, the status message in reverse video |
| less 668 | the `less -M` prompt on the last row, the alternate screen |
| emacs 30.1 (`-nw`) | menu bar, syntax colors, mode line, echo area |
| vttest 2.7 | test 1 (cursor movements) at 80 columns and, after Return, at 132 (DECCOLM, see below): the outer frame of `*` and `+`, the frame of `E`s and the text inside it are right in a wider window |
| Cyrillic | right, including `ё` and `ъ` |
| Combining marks | right: `é ä ñ ộ` from a letter plus marks, in one cell |
| CJK, Hangul, fullwidth Latin | two cells each, in the right columns |
| Box drawing | right |
| Emoji | a single emoji right (two cells); an emoji with the variation selector VS16 (`❤️`) two cells (run 12, D206); sequences below |

### Known limits

- **Emoji sequences** (run 5, D90; run 12, D206). A skin tone modifier
  (`👍🏽`), a ZWJ sequence (`👨‍👩‍👧`) and a flag (`🇺🇦`) are drawn as one picture
  at their first cell. The font's own placement of the glyphs stays when it
  fits the sequence's cells: Segoe UI Emoji on Windows 10 draws a ZWJ family
  from several glyphs it places together, and spread one per two cells they
  fell apart (until run 12). The grid still holds one character per cell as
  the shell's `wcwidth` counts them, so the columns stay right; the cells
  after the picture stay blank (`👍🏽` takes four columns, the family six).
- **Flags on Windows** (run 12: postponed). Windows ships no flag glyphs:
  Segoe UI Emoji draws a flag's two regional indicators as the two letters
  (`UA`), and so does the terminal. Drawing flags needs a font with them
  shipped with the app, under a license the clean room allows (OFL), in a
  format Windows 10 draws in color (COLR v0 or bitmaps); that choice is the
  owner's. Elsewhere the system's emoji font draws them (not checked on
  Linux yet).
- **VS16 emoji take two cells** (run 12, D206). An emoji that is one column
  wide by itself (`❤`, `⚠`, `✔`) followed by VS16 becomes two cells wide when
  the selector arrives, as an emoji presentation is: the text after it moves
  one column on. A program that counts such an emoji as one column (bash's
  readline with glibc's `wcwidth`) can put the cursor one column off while
  editing a command line that holds one. At the last column there is no
  second cell and the emoji stays one cell wide.
- **Known limitation: the blank cells after an emoji sequence** (D92). They
  stay: closing them up needs the terminal and the shell to agree on grapheme
  clusters (mode 2027, as in WezTerm and foot), which the shells in common
  use do not do yet; counting the sequence as two cells on the terminal's
  side alone would move readline's cursor out of step with the screen.
  Shell++ does not implement mode 2027.
- **vttest and 80/132 columns** (run 5, D90). vttest allows column
  switching (DECSET 40) and then sets 80 columns with DECCOLM: the grid
  takes 80 (or 132) columns at once and the remote side is told, so test 1's
  frame is right in a wider window too. The width stays until the pane is
  resized or the terminal is reset (RIS, `reset`); a DECCOLM without DECSET
  40 (as `tput reset` sends it) keeps the width, as in xterm.

Typing, mouse reporting (mc, htop), bracketed paste and resizing are also
covered by the terminal tests and the stage 2 screenshots; this section
adds the programs above.
