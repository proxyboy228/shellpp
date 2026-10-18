# Security Policy

Shell++ stores SSH credentials, private keys and host data, and can optionally sync them through a server you run yourself. We take reports that help keep this data safe seriously.

## Reporting a vulnerability

Report vulnerabilities privately through GitHub private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Go to **Advisories** and select **Report a vulnerability**.

Direct link: [https://github.com/proxyboy228/shellpp/security/advisories/new](https://github.com/proxyboy228/shellpp/security/advisories/new)

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

The project is in early development and has no releases yet. Only the latest commit on the `main` branch is supported.

| Version                 | Supported |
| ----------------------- | --------- |
| Latest commit on `main` | Yes       |
| Older commits           | No        |

This section will be updated once releases are published.

## Scope

In scope:

- **Client:** encryption of local storage (data at rest, key derivation, OS keychain integration), SSH and SFTP handling, host key verification and `known_hosts` handling, port forwarding defaults (for example, binding to non-loopback interfaces), import and export (including encrypted JSON export), and handling of untrusted data from servers (terminal escape sequences, remote file names and paths).
- **Sync server and sync client:** authentication, access control for shared vaults, access revocation, and the zero-knowledge guarantee that the server only ever sees encrypted blobs and sync metadata.
- **Design documents:** design-level issues in the cryptographic and sync design described in `SPEC.md` and `docs/` are welcome, even where no code exists yet.
- **Core principles with security impact:** any way for a server or third party to disable features remotely, wipe or lock data already stored locally, or collect data through telemetry or phone-home behavior.

Out of scope:

- Vulnerabilities in third-party dependencies. Please report these to the upstream project. Do tell us if Shell++ uses the vulnerable code path or configuration, so we can update or mitigate.
- Reports produced only by automated scanners, without a demonstrated impact on Shell++.

## Secrets committed to the repository

If you find credentials, private keys, tokens or other secrets committed to this repository or its history, report them privately using the same process. Do not reference the secret in public issues, pull requests or discussions.
