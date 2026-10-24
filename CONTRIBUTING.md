# Contributing to Shell++

Thank you for your interest in Shell++, an open-source,
cross-platform, self-hosted, local-first SSH client.

The project is in early development. Build and test instructions are in
[Toolchain and build](#toolchain-and-build) and [Tests](#tests).

## Table of contents

- [Ways to contribute](#ways-to-contribute)
- [Specifications are the source of truth](#specifications-are-the-source-of-truth)
- [Clean-room rules](#clean-room-rules)
- [Development workflow](#development-workflow)
- [Toolchain and build](#toolchain-and-build)
- [Unsafe code](#unsafe-code)
- [Tests](#tests)
- [Commit messages](#commit-messages)
- [Pull requests](#pull-requests)
- [Secrets and test data](#secrets-and-test-data)
- [Secret scanning](#secret-scanning)
- [If you accidentally committed a secret](#if-you-accidentally-committed-a-secret)
- [Reporting security issues](#reporting-security-issues)
- [Code of Conduct](#code-of-conduct)
- [Licensing of contributions](#licensing-of-contributions)

## Ways to contribute

- Report bugs and unclear or contradictory parts of the specifications.
- Propose features or changes to existing behavior.
- Review open pull requests and test proposed changes.
- Improve documentation and translations.
- Write code and tests once implementation work starts.

For anything beyond a small, obvious fix (a typo, a broken link, a one-line
bug fix), open an issue first and describe what you want to change and why.
This lets maintainers confirm the direction before you spend time on an
implementation that may not be accepted.

## Specifications are the source of truth

The design of Shell++ is defined by these documents:

- `SPEC.md`: scope, architecture, principles, security and quality
  requirements.
- `docs/ui-behavior-spec.md`: screens, navigation, scenarios, shortcuts and
  UI reactions.
- `docs/design-system.md`: colors, typography, spacing, radii and other
  visual tokens.

The Russian originals are in `docs/ru/`.

When you propose or discuss a change, refer to the relevant section of the
specifications. Code must follow the specifications. If you think a
specification is wrong or incomplete, change the specification first:

1. Open an issue that describes the problem and the proposed change.
2. Submit a pull request that edits the specification file(s).
3. Implement the change in code after the specification change is merged,
   or in the same pull request if a maintainer agrees.

If you change a specification file, update its counterpart in `docs/ru/` in
the same pull request, or state in the pull request description that the
translation needs a follow-up.

All changes must respect the core principles in `SPEC.md`. In particular,
pull requests that add license or subscription checks, telemetry, phone-home
behavior, remote feature disabling, or anything that can remotely wipe or
lock data already stored on a user's device will not be accepted.

## Clean-room rules

Shell++ is an independent, clean-room implementation. To keep it
that way, every contributor must follow these rules. Pull requests and issues
that break them will be closed, and affected content will be removed from the
history where possible.

### What you must not contribute

Do not submit anything taken or derived from a proprietary or commercial
client, including:

- source code, binaries or decompiled code;
- details obtained by decompiling, disassembling or otherwise reverse
  engineering such software, including its network protocols and storage
  formats;
- screenshots, images, icons, logos, illustrations, fonts or other assets;
- UI texts, labels, error messages or documentation;
- product names, brand colors or other brand elements.

Do not paste screenshots of other clients into issues, pull requests,
discussions or commit messages, and do not link to such screenshots as a
reference for how something should look.

### How to describe what you want

Describe the desired behavior in your own words: what the user does, what
the application should do in response, and why. Refer to the specifications
and to open standards (for example the OpenSSH documentation or the relevant
RFCs) rather than to the behavior of another product. Do not compare
Shell++ to named products in issues, pull requests or code
comments.

### Origin of contributions

Every contribution must be one of the following:

- your own original work; or
- code under a permissive license that is compatible with the project,
  included with its copyright notice and license text, and with its origin
  stated in the pull request description.

Do not submit code that you are not entitled to contribute, for example code
owned by your employer without permission.

### Dependencies

Any new dependency (Rust crate, font, icon set or other third-party asset) that ships with the application or the server must use a
permissive license:

- MIT
- Apache-2.0
- BSD (2-clause or 3-clause)
- ISC
- MPL-2.0
- Zlib, BSL-1.0, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, CDLA-Permissive-2.0
- public domain (for example SQLite)
- OFL-1.1 (fonts only)

Development-only dependencies that never ship (build, lint and test tools)
may additionally use BlueOak-1.0.0 and CC-BY-4.0. GPL and AGPL code must not
be shipped in the client. See `docs/decisions.md` (decision 4) for the
background and for crate-specific rules, for example the `fips` feature ban.

Add the dependency to `THIRD_PARTY_NOTICES.md` in the same pull request that
introduces it, including its name, version, license and source URL.
Dependencies under any other license, or with an unclear license, need
explicit approval from a maintainer in an issue before you submit the pull
request.

## Development workflow

1. Fork the repository at https://github.com/proxyboy228/shellpp and clone your fork.
2. Create a branch from the default branch with a short descriptive name,
   for example `fix/sftp-resume` or `feat/known-hosts-export`.
3. Make small, focused changes. One pull request should address one issue or
   one logical change. Split large work into a series of pull requests.
4. Run the tests, linters and formatters locally before you push.
5. Open a pull request against the default branch and link the related issue.
6. Keep CI green. A pull request with failing checks will not be merged.
7. Address review comments by pushing new commits to the same branch.

Write code, comments, commit messages and documentation in English.

## Toolchain and build

- Install Rust with [rustup](https://rustup.rs/). The repository pins the
  toolchain in `rust-toolchain.toml` (Rust 1.99.1; `gpui-pre`, the GPUI crate
  behind GPUI Kit, needs Rust 1.93 or later). Install it once with
  `rustup toolchain install 1.99.1`; inside the repository, `cargo` then uses
  it automatically.
- The desktop app is the crate `shellpp-desktop` in `apps/desktop`, built on
  GPUI Kit. Run it with `cargo run -p shellpp-desktop`. The README section
  [Running](README.md#running) shows how to start it with demo data in a
  temporary folder and where the permanent profile lives.

## Unsafe code

The workspace denies `unsafe_code`, and most crates also
`#![forbid(unsafe_code)]`. `unsafe` is allowed only in platform modules: a
module that exists to call one operating system's API, such as `windows.rs`,
`unix.rs` or `macos.rs`, or a module of the same kind under `platform/`. Such
a module opts in with `#![allow(unsafe_code)]` (or a `cfg_attr` for its
platform) and keeps the `unsafe` calls behind safe functions.

Every `unsafe` block has:

- a `// SAFETY:` comment right above it that says why the call is sound:
  which pointers it reads or writes, how long they stay valid, who frees what;
- a test that runs the code path of that block on its platform.

The platform modules today: the advapi32 security calls of
`crates/private-file` (`src/windows.rs`), the job object of the WSL listing
in `crates/term-local` (`src/windows.rs`), the `libc` calls of process
hardening (`apps/desktop/src/hardening.rs`) and of the device name
(`apps/desktop/src/platform/device_name.rs`), and `GetLogicalDrives` for the
drive list of the local file panel (`apps/desktop/src/platform/drives.rs`),
and the time zone and date format calls of the dates on screen
(`apps/desktop/src/platform/local_time.rs`), and the WebAuthn calls of
security keys, `webauthn.dll` loaded at run time
(`apps/desktop/src/platform/security_key.rs`; the calls that show the
system dialog run in tests only with `SHELLPP_TEST_WEBAUTHN=1` and a person
at the key).

## Tests

Changes to behavior must include tests. Pure refactoring must not change
existing tests unless the tests themselves were wrong.

Run the tests with `cargo test`. While you iterate, run only the tests of the
crate or module you changed; run the full suite before each commit.

- **Unit tests** cover the Rust crates, for example encryption, sync merge,
  the `ssh_config` parser and the secret filter for command history.
- **Integration tests** cover SSH, SFTP and port forwarding against OpenSSH
  running in Docker.
- **UI tests** cover the interface. As described in `SPEC.md`, each screen
  and scenario in `docs/ui-behavior-spec.md` has GPUI tests
  (`TestAppContext` / `VisualTestContext`) that check the described behavior.
  They run headless as part of `cargo test` and never open a window. If you
  change UI behavior, add or update the matching UI tests in the same pull
  request.
- **UI screenshots** show our own application's window:
  `scripts/ui-screens.ps1` starts the app with demo data and saves the
  screenshots to `target/ui-screens/`. A screen is considered done when its
  tests pass and its screenshots match the specifications.
- **Sync end-to-end tests** run two clients against a server.

Tests and the screenshot script must never use the permanent profile. They
work in temporary data directories passed through `SHELLPP_DATA_DIR`, which
are deleted afterwards.

CI runs rustfmt, clippy, the license checks, all tests and builds for
Windows, macOS and Linux, plus the secret scan.

## Commit messages

Commit messages follow
[Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).

### Format

```text
<type>[optional scope][!]: <description>

[optional body]

[optional footer(s)]
```

- Write the description in the imperative mood, in lowercase, without a
  trailing period, for example `add host search`, not `Added host search.`.
- Keep the first line under 72 characters.
- Use the body to explain what changed and why, not how.
- Reference issues in the footer, for example `Refs: #123` or `Fixes: #123`.

### Types

| Type       | Use for                                                     |
| ---------- | ----------------------------------------------------------- |
| `feat`     | A new feature                                               |
| `fix`      | A bug fix                                                   |
| `docs`     | Documentation and specification changes only                |
| `style`    | Formatting and whitespace, no change in behavior            |
| `refactor` | Code changes that neither fix a bug nor add a feature       |
| `perf`     | Performance improvements                                    |
| `test`     | Adding or correcting tests                                  |
| `build`    | Build system, packaging or dependency changes               |
| `ci`       | CI configuration and scripts                                |
| `chore`    | Maintenance that does not fit any other type                |
| `revert`   | Reverting a previous commit                                 |

### Scopes

The scope is optional. Suggested scopes:

- `desktop`: the GPUI Kit desktop application in `apps/desktop`
- `core`: the shared Rust core
- `ssh`: SSH, SFTP and port forwarding
- `server`: the optional sync server
- `docs`: documentation and specifications
- `ci`: CI workflows and repository tooling

### Breaking changes

Mark a breaking change in one or both of these ways:

- add `!` after the type or scope, for example `feat(core)!: ...`;
- add a `BREAKING CHANGE:` footer that describes the change and the
  migration path.

### Examples

```text
feat(ssh): add support for ProxyJump chains
```

```text
fix(desktop): keep terminal focus after closing the search bar

Refs: #42
```

```text
docs: clarify conflict resolution rules in SPEC.md
```

```text
feat(core)!: change the export JSON schema to version 2

BREAKING CHANGE: the "identities" field is renamed to "keys".
Files exported with version 1 can still be imported.
```

```text
revert: feat(server): add rate limiting per account

This reverts commit 1a2b3c4.
```

## Pull requests

- Pull requests are merged with squash merge, so the pull request title
  becomes the commit message on the default branch. The title must follow
  the same Conventional Commits format, for example
  `fix(core): handle empty known_hosts file`.
- Describe what the pull request changes and why, and link the related issue
  and the relevant section of the specifications.
- For UI changes, describe the change in words and refer to the
  specification. Screenshots of Shell++ itself are welcome; the
  clean-room rules above still apply.
- Confirm in the description that the contribution follows the clean-room
  rules and that new dependencies are listed in `THIRD_PARTY_NOTICES.md`.

## Secrets and test data

Never commit real secrets. This includes:

- private keys, passphrases and passwords;
- API keys, access tokens and sync credentials;
- `.env` files and other local configuration with credentials;
- `known_hosts`, `~/.ssh/config` or other files copied from your machine;
- real hostnames, IP addresses or usernames of your own infrastructure in
  tests, logs or bug reports (redact them).

### Generate test secrets at runtime

The default for tests is to generate key material and passwords on the fly:

- Generate keys in the test setup, for example create a fresh Ed25519 key
  pair in a temporary directory before each test or test suite.
- Generate passwords and tokens randomly at runtime.
- Build `known_hosts` entries from the host key of the test server started
  for that run.
- Delete generated material when the test finishes.

For example, a shell-based integration test can create a key like this:

```sh
ssh-keygen -q -t ed25519 -N "" -C "test-only" -f "$TMPDIR/id_ed25519"
```

Fixtures must not contain real or long-lived key material.

### Static fixtures

Use a static fixture only when a test needs a file in a specific format that
cannot reasonably be generated at runtime, for example to test a parser
against a known input. In that case:

- generate a throwaway key or secret for that purpose only, and never use it
  anywhere else;
- mark it clearly as a test fixture, in the file name (for example
  `throwaway_ed25519_test_only`) and in a comment or a README next to it;
- add a narrowly scoped allowlist entry for that file to `.gitleaks.toml` in
  the same pull request (see [Secret scanning](#secret-scanning)).

## Secret scanning

CI runs [gitleaks](https://github.com/gitleaks/gitleaks) on every push and
pull request. Run the same check locally before each commit with a
pre-commit hook. `scripts\verify.ps1` also scans the whole committed history of every ref
(branches, tags, stash) with the workflow's gitleaks version in Docker
(the image pinned by digest, no network), and any finding fails it.

### Set up the pre-commit hook

1. Install [pre-commit](https://pre-commit.com/) with one of these commands:

   ```sh
   pipx install pre-commit
   ```

   ```sh
   pip install pre-commit
   ```

   ```sh
   brew install pre-commit
   ```

2. In the repository root, install the hook:

   ```sh
   pre-commit install
   ```

The hook configuration is in `.pre-commit-config.yaml`. It uses the official
gitleaks hook (`id: gitleaks`) pinned to `v8.30.1`. pre-commit builds
gitleaks from source, which requires Go. If Go is not installed, pre-commit
can bootstrap a Go toolchain for the hook.

To check all files without making a commit:

```sh
pre-commit run --all-files
```

### Alternatives

- If you already have the `gitleaks` binary on your `PATH`, you can use the
  `gitleaks-system` hook id instead of `gitleaks` in your local copy of
  `.pre-commit-config.yaml`. Do not commit that change.
- You can run gitleaks manually on staged changes before each commit:

  ```sh
  gitleaks git --pre-commit --staged --redact --verbose
  ```

- To scan the full history of your branch:

  ```sh
  gitleaks git --redact --verbose
  ```

### If the hook flags something

- **Real secret**: remove it from the change, and rotate (revoke and replace)
  it, because it may already have been exposed. Never bypass the hook.
- **False positive**: add a narrowly scoped allowlist entry to
  `.gitleaks.toml` in the same pull request, for example for one specific
  file path or one specific pattern, and explain in the pull request
  description why the finding is not a secret. Do not add broad allowlist
  entries such as whole directories or generic patterns.
- **False positive in a commit already pushed** (CI flags history that can
  no longer change): add the finding's fingerprint
  (`<commit>:<file>:<rule>:<line>`, printed by
  `gitleaks git --redact --verbose`) to `.gitleaksignore`, under a comment
  that says what the value is, never the value itself (docs/decisions.md,
  D203).

Never use `git commit --no-verify` to skip the hook.

## If you accidentally committed a secret

1. **Rotate it first.** Revoke the key, password or token and replace it.
   Assume it is compromised as soon as it has been pushed anywhere.
2. **Contact the maintainers privately** as described in
   [SECURITY.md](SECURITY.md), so they can help remove it from the
   repository history. Do not open a public issue about it.

Removing a secret in a later commit does not remove it from the history, and
forks or clones may already contain it. Rotation is the only reliable fix.

## Reporting security issues

Do not report security vulnerabilities in public issues, pull requests or
discussions. Follow the private reporting process in
[SECURITY.md](SECURITY.md).

## Code of Conduct

This project follows a Code of Conduct. By participating, you agree to abide
by it. See [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Licensing of contributions

The project uses two licenses:

- `server/` (the sync server) is licensed under the GNU Affero General
  Public License v3.0 only (`AGPL-3.0-only`), see `server/LICENSE`;
- everything else (the desktop app, `crates/`, docs and tooling) is licensed
  under the Apache License 2.0 (`Apache-2.0`), see `LICENSE`.

Contributions follow the inbound = outbound principle: by submitting a
contribution, you agree that it is licensed under the license that covers
the files you change or add. You confirm that you have the right to submit
the contribution under those terms.

Keep the boundary one-way: `server/` may depend on Apache-2.0 code such as
`crates/core`, but code outside `server/` must never import or copy code from
`server/`.
