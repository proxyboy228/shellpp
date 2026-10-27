# Security Policy

Shell++ stores SSH credentials, private keys and host data, and can optionally sync them through a server you run yourself. We take reports that help keep this data safe seriously.

## Reporting a vulnerability

Report vulnerabilities privately, in either of two ways:

- **GitHub private vulnerability reporting** (preferred): open the repository's **Security** tab, go to **Advisories** and select **Report a vulnerability**. Direct link: [https://github.com/proxyboy228/shellpp/security/advisories/new](https://github.com/proxyboy228/shellpp/security/advisories/new)
- **Email:** [iinfocertx@gmail.com](mailto:iinfocertx@gmail.com), the project's contact address. Mention "Shell++ security" in the subject.

Do **not** open public issues, pull requests or discussions for vulnerabilities, and do not share details publicly until a fix or advisory has been published.

## What to include

- The affected component (for example: desktop app, local storage, SSH/SFTP, port forwarding, import/export, sync client, sync server, design documents).
- The affected version or commit hash.
- The impact: what an attacker can achieve and under which conditions (for example: local user, network attacker, malicious SSH server, malicious or compromised sync server, another member of a shared vault).
- Steps to reproduce, including the relevant configuration and operating system.
- A proof of concept, if you have one.
- Whether the issue is already public or known to others, and any disclosure deadline you are working with.
- Whether and how you would like to be credited.

Use test keys and test credentials only. Never send real private keys, passwords or tokens.

## What to expect

- We aim to acknowledge your report within 7 days.
- We will assess the report, keep you informed of progress and may ask follow-up questions.
- We follow coordinated disclosure: we work on a fix privately, agree on a disclosure date with you and publish a security advisory once a fix is available.
- We credit reporters in the advisory if they wish to be credited.

Shell++ is maintained by volunteers, so all of the above is best effort. There is no bug bounty program.

## Supported versions

Security fixes go to the latest release of the supported line. The desktop app and the sync server have the same version and are supported together.

| Version | Supported |
| ------- | --------- |
| 1.0.0-rc.1 (release candidate) | Yes, until 1.0.0 is published |
| 1.0.x, after 1.0.0 | The latest 1.0.x |
| Older versions and pre-releases | No |
| The latest commit on `main` | Yes (development) |

The Android app has versions of its own and no release yet; report issues with it the same way.

## Threat model in brief

The full threat model is [docs/security.md](docs/security.md). In short:

- **What is protected:** everything you store (hosts, keys, passwords, history, recordings) is encrypted at rest with keys that only your master password or recovery key opens (XChaCha20-Poly1305, Argon2id); while "Remember on this device" is on, the device key also sits in the OS keychain. The data directory also holds some plaintext: metadata and settings, the server trust choices, logs without secrets, and files opened from SFTP while they are open (section 9.3 of [docs/security.md](docs/security.md)).
- **Sync is zero-knowledge:** the server stores encrypted records and the metadata it needs to sync (who, how many, when, padded sizes); keys and plaintext never leave your devices. A server that is compromised can withhold, roll back or replay records, but cannot read them; records are not signed yet.
- **SSH:** a changed host key is never accepted silently; the first connection to a host is trust on first use, unless its key comes from an imported `known_hosts`.
- **Updates:** the app only accepts an update manifest signed with the release key built into it, and it never downloads or installs anything.
- **Out of scope:** malware running as your user while the app is unlocked, a compromised operating system, and hiding metadata from the sync server.

Known limitations that matter for security (section 17 of [docs/security.md](docs/security.md) has all of them):

- The Windows executable is not code-signed: check a download against `SHA256SUMS` and its minisign signature ([docs/release.md](docs/release.md#checking-a-download)).
- With "Remember on this device" (on by default), programs running as your user can read the device key on Windows and Linux, and the app opens without the password; with a roaming Windows profile the key also roams to your other machines.
- Auto-lock is off ("Never") by default; turn it on in Settings > Security.
- Anyone with a copy of the database or of the server's data can try to guess the master password offline; a short password is cheaper to guess.
- Some secrets can remain in memory longer than needed (terminal scrollback, copies inside libraries), and swap or crash dumps may hold them.
- Forwarded ports and the SOCKS proxy listen on 127.0.0.1 by default, where every local program can reach them.
- A sync server reached through Cloudflare Tunnel lets Cloudflare see every request, so it could sign in as you (records stay end-to-end encrypted; TOTP still guards sign-in when it is on).

## Scope

In scope:

- **Client:** encryption of local storage (data at rest, key derivation, OS keychain integration), SSH and SFTP handling, host key verification and `known_hosts` handling, port forwarding defaults (for example, binding to non-loopback interfaces), import and export (including encrypted JSON export), and handling of untrusted data from servers (terminal escape sequences, remote file names and paths).
- **Sync server and sync client:** authentication, access control for shared vaults (after 1.0), access revocation, and the zero-knowledge guarantee that the server only ever sees encrypted blobs and sync metadata.
- **Design documents:** design-level issues in the cryptographic and sync design described in `SPEC.md` and `docs/` are welcome, even where no code exists yet.
- **Core principles with security impact:** any way for a server or third party to disable features remotely, wipe or lock data already stored locally, or collect data through telemetry or phone-home behavior.

Out of scope:

- Vulnerabilities in third-party dependencies. Please report these to the upstream project. Do tell us if Shell++ uses the vulnerable code path or configuration, so we can update or mitigate.
- Reports produced only by automated scanners, without a demonstrated impact on Shell++.

## Secrets committed to the repository

If you find credentials, private keys, tokens or other secrets committed to this repository or its history, report them privately using the same process. Do not reference the secret in public issues, pull requests or discussions.
