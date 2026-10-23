# Releases

How a Shell++ release is built, signed and published, and how anyone can
check a download (run 8, D170). The workflow is
[`.github/workflows/release.yml`](../.github/workflows/release.yml).

## What a release holds

A tag `v<version>` (for example `v0.2.0`, or `v0.2.0-rc.1` for a
pre-release) builds, in a **draft** release:

| File | What it is |
|---|---|
| `shellpp-<version>-windows-x64.zip` | the portable app for Windows (`scripts/package-windows.ps1`: `shellpp.exe`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `README-portable.txt`) |
| `shellpp.pdb` | the symbols of that `shellpp.exe`, for crash reports |
| `shellpp-server-<version>-docker-image.tar.gz` | the sync server image (`server/Dockerfile`), tagged `shellpp-server:<version>` and `shellpp-server:latest`; `docker load -i` it |
| `shellpp-server-<version>-linux-x64.tar.gz` | the same server as a static Linux binary (taken from the image), with `LICENSE`, `README.md` and `deploy/` |
| `latest.json` | the update manifest the app reads (D97): `version`, `url` of the release page, `published`, and every file with its SHA-256 and size |
| `SHA256SUMS` | the SHA-256 of every file above |
| `<file>.minisig` | the minisign signature of each file above, `SHA256SUMS` and `latest.json` included |

Nothing is published by the workflow: the owner checks the draft and
publishes it. GitHub adds the source archives of the tag, which the
server's license (AGPL-3.0) asks for.

The release builds without a build cache, from the tag alone. The Windows
executable is not code-signed (D7): SmartScreen may ask once.

## The signing key

The release files are signed with [minisign](https://jedisct1.github.io/minisign/)
(ISC license), an Ed25519 signature tool. The key pair belongs to the
owner; the workflow only ever sees the secret half through GitHub secrets,
and the repository holds only the public half.

### Making the key (once, by the owner)

On a computer you trust, with minisign installed (`brew install minisign`,
`apt install minisign`, or on Windows the release binary from
<https://github.com/jedisct1/minisign/releases>):

```
minisign -G -p shellpp-release.pub -s shellpp-release.key
```

minisign asks for a password that encrypts the secret key. Then:

1. **Public half.** Replace the whole content of
   [`release/minisign.pub`](../release/minisign.pub) with the two lines of
   `shellpp-release.pub` (the `untrusted comment:` line first, as minisign
   reads it) and commit it. The app builds this file in: it
   checks `latest.json` with it (below), and the release workflow refuses
   to run without it.
2. **Secret half.** In the repository's GitHub settings, Environments,
   create the environment `release` (give it a required reviewer, yourself,
   so that no workflow reads the key without your approval), and add two
   environment secrets:
   - `MINISIGN_SECRET_KEY`: the whole content of `shellpp-release.key`;
   - `MINISIGN_PASSWORD`: its password.
3. Keep `shellpp-release.key` and its password offline (a password manager
   and an encrypted backup), never in the repository. Delete any other
   copy.

### If the key is lost or leaks

The public key is in every installed app, so a new key reaches users only
with a new app version, and older versions then report every update as
"not signed with the Shell++ release key" until the user updates by hand.

- Make a new key as above, replace `release/minisign.pub` and both
  secrets, and release a new version; say in its notes that it changes the
  key and that older versions have to be updated by hand.
- If the old secret key leaked, also delete the old secrets at once and
  remove releases that may have been signed by someone else.

## Making a release

1. Set the same version in `Cargo.toml` (`[workspace.package] version`) and
   `server/Cargo.toml` (`[package] version`), and update both lock files
   (`cargo update -w` and `cargo update -w --manifest-path server/Cargo.toml`).
2. Let the change pass the main criterion of `CLAUDE.md` and CI, and merge
   it into `main`.
3. Tag the commit and push the tag (the owner only):

   ```
   git tag -a v0.2.0 -m "Shell++ 0.2.0"
   git push origin v0.2.0
   ```

4. The workflow "Release" checks that the tag matches both versions and
   that `release/minisign.pub` holds a key, builds the Windows zip and the
   server, writes `latest.json` and `SHA256SUMS`, waits for your approval
   of the environment `release`, signs every file, verifies every
   signature with `release/minisign.pub`, and creates the draft release.
5. Check the draft: download the files and check them as below, read the
   notes, then **Publish release**. A tag with a `-` (for example
   `v0.2.0-rc.1`) makes a pre-release.

Once published (and not a pre-release), the release is GitHub's "latest",
and `https://github.com/proxyboy228/shellpp/releases/latest/download/latest.json`
with its `latest.json.minisig` is what the app's update check reads.

A failed run leaves no release. A run that failed after the draft was
created (or one to repeat) needs the draft deleted first.

## How the app checks for updates

The update check (Help > **Check for Updates**, Settings > **Updates**; D97)
downloads `latest.json` and `latest.json.minisig` from the same place (the
address set in Settings with `.minisig` added), and shows an update only
when:

- the signature is a minisign signature (the prehashed kind minisign makes
  by default) of exactly that `latest.json`, made with the key of
  `release/minisign.pub` built into the app, its trusted comment included;
- the trusted comment is the workflow's for this manifest,
  `shellpp <version> latest.json` with the manifest's own version, so the
  signature of another file of a release (or a manifest signed by hand
  with minisign's default comment) does not count;
- and the manifest's `version` is newer than the app's.

When "latest" changes between the two requests (a release published in
that second), the app asks for both once more. A signature that the server
cannot send right now (5xx, 429) is "the update source did not answer", not
an unsigned update.

Otherwise the check says why ("the update is not signed with the Shell++
release key", "this build has no key to check updates with"). A different
update source in Settings changes where the manifest comes from, never the
key. The app still downloads and installs nothing: it opens the release
page.

## Checking a download

With minisign and the public key from the repository
([`release/minisign.pub`](../release/minisign.pub)):

```
minisign -V -p minisign.pub -m shellpp-0.2.0-windows-x64.zip
```

It prints `Signature and comment signature verified` and the trusted
comment, which names the version and the file. Without minisign, compare
the SHA-256 with `SHA256SUMS` (whose own signature `SHA256SUMS.minisig`
you then trust): in PowerShell `Get-FileHash shellpp-0.2.0-windows-x64.zip`,
on Linux `sha256sum -c SHA256SUMS --ignore-missing`.
