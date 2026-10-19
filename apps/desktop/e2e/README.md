# UI e2e tests

WebdriverIO + tauri-driver against a debug build of Shell++ with the
`e2e-mocks` feature (docs/architecture.md, sections 5.4 and 9). Specs are
named after the UI section they cover (`specs/<section>-<name>.e2e.ts`) and
find elements by `data-testid` only. SSH specs connect to the russh test
server `shellpp-test-sshd` (`tests/sshd`, architecture 9.3) on 127.0.0.1,
never to a real host.

## Layout

| Path | Content |
|---|---|
| `wdio.conf.ts` | WebdriverIO config: mocha, spec reporter, one worker at a time, `tauri:options` capability, WebDriver classic |
| `support/paths.ts` | app, test server, log and driver cache paths |
| `support/drivers.ts` | finds tauri-driver and the native driver (downloads msedgedriver on Windows) |
| `support/session.ts` | per spec: fresh temporary `SHELLPP_DATA_DIR`, tauri-driver start and stop |
| `support/sshd.ts` | test SSH server: `startSshd()` (free port, generated host key, generated passwords and answers, `rotateHostKey`, `drop`, `setAccepting`, `setHolding`, `events`, `stop`) and `keygen()` |
| `helpers/` | selectors (`tid`, `el`, `els`), app helpers (`mock`, `firstRun`, `unlock`, `restartApp`, `newPassword`), hosts and panels, menus, pointer input, settings, terminal (`terminalText`, `waitForTerminalText`, `typeInTerminal`, sidebar), sessions (`createSshHost`, `connectHost`, `hostKey`, `answerPrompt`, tabs, `hashedHost`, `seedKnownHost`) |
| `specs/` | stage-1 and stage-2 specs (UI 1.1-1.5, 2.1-2.4, 3.1-3.4, 4.1-4.4, 4.6-4.8, 6, 10.1, 10.2, 12.1, 13.1-13.4; local terminal) |
| `smoke/harness.e2e.ts` | harness self-check (app start, restart, test server switches, keygen); not part of the suite |
| `tauri.e2e.conf.json` | Tauri config override for the e2e build |

Every spec runs in its own worker with a fresh temporary data directory and a
master password generated at run time (`newPassword()`). A restart inside a
spec is `browser.reloadSession()`: the app process exits and starts again with
the same data directory. The mock clock, clipboard, launcher and keychain are
driven through `window.__SHELLPP_E2E__.mock()` (`helpers/app.ts`, `mock()`);
known hosts are seeded with the mock action `known_host_add`
(`seedKnownHost()`).

### SSH sessions

A spec starts its own servers and stops them in `after`:

```ts
const sshd = await startSshd({ users: [{ name: 'alice' }], bannerComment: 'Ubuntu-3ubuntu13' });
const hostId = await createSshHost(sshd, sshd.users[0], { label: 'Test server' });
await openSshSession(hostId, 'alice'); // double click, "Accept and save", shell prompt
await runCommand('echo hi');
await waitForOutputLine('hi');
```

- Users get generated passwords (and keyboard-interactive answers) unless the
  spec passes its own generated values; `keygen()` makes client keys (encrypted
  with a generated passphrase on request). The server's config file and the
  keys live in temporary directories that `stop()`, the `afterSession` hook and
  a process exit hook delete; the server also exits when its stdin closes.
- The test server's shell is deterministic (stage 2 plan, section 4.1):
  prompt `<user>@shellpp-test:~$ `, commands `echo`, `exit [n]`,
  `read-secret`, `alt-screen`, `size`, `term`, `env`, `lines <n>`,
  `later <s> <text>`, `drop` and more.
- Terminal text comes from the e2e probe of each pane
  (`window.__SHELLPP_E2E__.terminalText(paneId)`, `src/ipc/e2e.ts`), because
  xterm.js draws on a canvas. Input is a click on the pane's `terminal-view`
  followed by WebDriver key actions.
- Reconnect attempts and `later` run in real time; waits are explicit
  (`waitForTerminalText`, `waitForPaneState`) with timeouts of 15-30 s.
- Local terminal specs look for markers only (`echo shellpp-<random>`):
  prompts and colors of PowerShell and login shells vary.

## Running on Windows

Prerequisites: Rust (MSVC), pnpm, the WebView2 runtime, Node >= 22.19 for
WebdriverIO (Node 20 can start the scripts; they find a newer Node through
`SHELLPP_E2E_NODE` or nvm), and tauri-driver:

```sh
cargo install tauri-driver --locked
```

Then, from the repository root:

```sh
pnpm --filter @shellpp/desktop e2e:build   # app (debug, e2e-mocks) and test server (target/e2e)
pnpm --filter @shellpp/desktop e2e:build -- --sshd-only   # or --app-only
pnpm --filter @shellpp/desktop e2e         # builds what is missing first
pnpm --filter @shellpp/desktop e2e -- --spec e2e/specs/13.1-first-run.e2e.ts
pnpm --filter @shellpp/desktop e2e -- --rebuild
```

msedgedriver must match the installed WebView2 major version. The harness
reads the WebView2 version from the registry and downloads the matching
msedgedriver once through the `edgedriver` package (MIT, a dependency of
WebdriverIO) into `target/e2e-drivers/msedgedriver-<version>/`. A matching
`msedgedriver` on PATH is used instead when present; `SHELLPP_E2E_NATIVE_DRIVER`
overrides both.

Close other Shell++ windows first: the e2e app shares the WebView2 profile of
the bundle identifier with a development build.

## Running on Linux (CI)

The native driver is WebKitWebDriver; there is no display on CI runners, so
the run is wrapped in `xvfb-run` (WebdriverIO's own display server support is
turned off in `wdio.conf.ts`).

```sh
sudo apt-get install -y webkit2gtk-driver xvfb \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
cargo install tauri-driver --locked
pnpm install --frozen-lockfile
pnpm --filter @shellpp/desktop e2e:build
xvfb-run -a pnpm --filter @shellpp/desktop e2e
```

On Fedora the driver comes with `webkit2gtk4.1-devel`. The "Remember on this
device" scenarios use the e2e keychain mock, so no Secret Service is needed.
Without a GPU, xterm.js falls back to its DOM renderer
(`terminal-view[data-renderer="dom"]`).

macOS has no WebDriver for WKWebView: the stage-1 and stage-2 items are
covered by the manual checklist (`docs/testing/macos-checklist.md`).

## Environment variables

| Variable | Meaning |
|---|---|
| `CARGO_TARGET_DIR` | target directory of the e2e build (default `<repo>/target/e2e`) |
| `SHELLPP_E2E_APP` | use this binary instead of `<target>/debug/shellpp(.exe)` |
| `SHELLPP_E2E_SSHD` | use this test server binary instead of `<target>/debug/shellpp-test-sshd(.exe)` |
| `CARGO` | cargo binary used to build the test server (default: PATH) |
| `SHELLPP_E2E_NODE` | Node >= 22.19 used for WebdriverIO when the current Node is older |
| `TAURI_DRIVER` | path of tauri-driver (default: PATH, then `~/.cargo/bin`) |
| `SHELLPP_E2E_NATIVE_DRIVER` | path of msedgedriver / WebKitWebDriver |
| `SHELLPP_E2E_WEBVIEW2_VERSION` | WebView2 version when the registry cannot be read |
| `SHELLPP_E2E_DRIVER_CACHE` | download cache for msedgedriver (default `<repo>/target/e2e-drivers`) |
| `SHELLPP_E2E_DRIVER_PORT`, `SHELLPP_E2E_NATIVE_PORT` | tauri-driver and native driver ports (4444, 4445) |
| `SHELLPP_E2E_LOG_DIR` | driver, test server and WebdriverIO logs (default `<repo>/target/e2e-logs`) |
| `SHELLPP_E2E_KEEP_DATA=1` | keep the temporary data directories for debugging |

The app's stderr (its diagnostic log lines) ends up in
`target/e2e-logs/<worker>-<spec>.tauri-driver.log`, the test servers' stderr
(no secrets) in `target/e2e-logs/<worker>-sshd-<n>.log`.

## Coverage of UI sections

```sh
node scripts/e2e-missing-specs.mjs        # sections and scope items of the current stage without a spec (exit 1 if any)
node scripts/e2e-missing-specs.mjs --all  # every section with its stage
node scripts/hidden-features.mjs          # features hidden in this build (stage report)
node scripts/hidden-features.mjs --visible  # ... and the features that became visible with this stage
```

`e2e-coverage.mjs` (same as `e2e-missing-specs.mjs`) also lists the stage
items that only Rust tests cover (system agent and agent forwarding,
keepalive timeouts, no SHA-1 algorithms, charset conversion, every history
detector rule, wildcard and IPv6 known host matching, reconnect waiting for
unlock).
