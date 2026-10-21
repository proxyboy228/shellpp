# Shell++

English | [Русский](README.ru.md)

Shell++ is a self-hosted, local-first SSH client. Desktop platforms (Windows, macOS, Linux) come first; mobile is planned for later. It covers hosts and groups, a keychain for SSH keys and identities, a terminal with tabs and split panes, snippets, SFTP, and port forwarding. Sync between devices is optional and end-to-end encrypted, and it runs through a server you host yourself.

## Principles

1. **Local-first.** The app works fully offline, without an account or a server. Sync is optional.
2. **No license checks. No telemetry.** There are no license or subscription checks, trials, tiers, telemetry, phone-home calls or remote feature disabling.
3. **Your data always stays readable and exportable.** If your access to a shared vault is revoked, your account is deleted, you sign out or the server is unreachable, your local copy remains available and exportable. Revoking access only stops future sync. It never wipes or locks data you have already received.
4. **Open export formats.** You can export at any time to JSON (documented schema, plain or encrypted), `~/.ssh/config`, OpenSSH-format keys and `known_hosts`.
5. **Zero-knowledge sync.** The sync server stores only encrypted blobs and sync metadata. Encryption keys never leave the client in plaintext.

## Status

Shell++ is in early development. The specifications, the stage-0 design documents and the design tokens with base UI components exist; there is no application yet.

- The application is not usable yet, and there is nothing to install.
- Expect breaking changes to the specifications, data formats and interfaces.

## Running

Prerequisite: the Rust toolchain pinned in `rust-toolchain.toml` (Rust 1.99.1). Install it once with `rustup toolchain install 1.99.1`; inside the repository, `cargo` then uses it automatically.

Normal development build, with your permanent profile:

```sh
cargo run -p shellpp-desktop
```

The permanent profile is the data directory from [docs/storage-format.md](docs/storage-format.md), section 2.1:

- Windows: `%LOCALAPPDATA%\shellpp\`
- macOS: `~/Library/Application Support/shellpp/`
- Linux: `${XDG_DATA_HOME:-~/.local/share}/shellpp/`

Test data in a temporary folder (300 hosts, 5 groups, some hosts with tags and colors):

```sh
cargo run -p shellpp-desktop --features dev-tools -- --demo
```

- It creates a new folder `shellpp-demo-<random>` in the system temporary directory, on Windows `%TEMP%\shellpp-demo-<random>\`, and prints its path to the console.
- It unlocks the demo data automatically and deletes the folder when the app exits.
- It never touches the permanent profile.

To use another data folder, set the `SHELLPP_DATA_DIR` environment variable, for example in PowerShell:

```powershell
$env:SHELLPP_DATA_DIR = "D:\shellpp-data"; cargo run -p shellpp-desktop
```

The variable stays set until the PowerShell session ends; `Remove-Item Env:SHELLPP_DATA_DIR` returns to the permanent profile.

## Known limitations

- SFTP: a file or folder name on the server that is not valid UTF-8 is shown with the replacement character `�` (U+FFFD) and cannot be opened, renamed or downloaded; a folder transfer leaves it out and lists it at the end. The SFTP library decodes names that way (`docs/decisions.md`, D45, D50).

## Specifications

- [SPEC.md](SPEC.md): product specification, covering scope, principles, data model, security and sync.
- [docs/ui-behavior-spec.md](docs/ui-behavior-spec.md): how screens, controls and user flows behave.
- [docs/design-system.md](docs/design-system.md): visual language, design tokens and components.

The specifications were originally written in Russian. The Russian originals are in [docs/ru/](docs/ru/):
[SPEC.md](docs/ru/SPEC.md), [ui-behavior-spec.md](docs/ru/ui-behavior-spec.md), [design-system.md](docs/ru/design-system.md).

## Design documents

- [docs/decisions.md](docs/decisions.md): decisions taken before stage 0.
- [docs/architecture.md](docs/architecture.md): components, process model and dependency policy.
- [docs/data-model.md](docs/data-model.md): entities, fields and sync behavior.
- [docs/storage-format.md](docs/storage-format.md): on-disk format, encryption envelope and export formats.
- [docs/data-format.md](docs/data-format.md): the on-disk format version 1 as implemented now.
- [docs/sync-protocol.md](docs/sync-protocol.md): the end-to-end encrypted sync protocol.
- [docs/security.md](docs/security.md): threat model and security controls.

## Planned stack

- Desktop app: GPUI Kit, a native Rust UI without a webview.
- Terminal: alacritty_terminal with our own GPUI element.
- SSH: russh.
- Local storage: SQLite with record-level encryption.
- Sync server: Rust (axum) with PostgreSQL or SQLite, shipped as a Docker image.

## Clean-room statement

Shell++ is an independent implementation written from its own specifications. It is not affiliated with, endorsed by, or derived from any commercial product. It contains no code, assets, binaries, network protocols, storage formats or UI texts of any proprietary client.

Contributions must follow the same rules. See [Clean-room rules](CONTRIBUTING.md#clean-room-rules) in CONTRIBUTING.md.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening an issue or a pull request. All participants are expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Do not report security vulnerabilities in public issues. Report them privately as described in [SECURITY.md](SECURITY.md).

## License

Shell++ uses two licenses:

- The sync server in [`server/`](server/) is licensed under the
  [GNU Affero General Public License v3.0 only](server/LICENSE)
  (`AGPL-3.0-only`). Anyone who runs a modified server for others must
  publish their changes.
- Everything else, including the desktop app and the shared crates, is
  licensed under the [Apache License 2.0](LICENSE) (`Apache-2.0`).

Third-party components are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
