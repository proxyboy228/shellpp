# Releases

How a Shell++ release is built, signed and published, and how anyone can
check a download (run 8, D170). The workflow is
[`.github/workflows/release.yml`](../.github/workflows/release.yml).

## What a release holds

A tag `v<version>` (for example `v0.2.0`, or `v0.2.0-rc.1` for a
pre-release) builds, in a **draft** release:

| File | What it is |
|---|---|
| `shellpp-<version>-windows-x64.zip` | the portable app for Windows (`scripts/package-windows.ps1`, default features; FIDO2 keys sign through Windows' WebAuthn): `shellpp.exe`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `README-portable.txt` |
| `shellpp.pdb` | the symbols of that `shellpp.exe`, for crash reports |
| `shellpp-<version>-linux-x64.tar.gz` | the app for Linux, x86-64, glibc 2.35 or later (`scripts/package-linux.sh` in the Ubuntu 22.04 image of `tests/linux/package.Dockerfile`, default features: without `fido2-hid`, D173, D225): one folder `shellpp-<version>-linux-x64/` with `shellpp`, `shellpp.desktop`, `shellpp.png`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `README-linux.txt` |
| `shellpp-server-<version>-docker-image.tar.gz` | the sync server image (`server/Dockerfile`), tagged `shellpp-server:<version>` and `shellpp-server:latest`, with the server's license and notices in `/usr/share/doc/shellpp-server/`; `docker load -i` it |
| `shellpp-server-<version>-linux-x64.tar.gz` | the same server as a static Linux binary (taken from the image; `scripts/package-server.sh`), with `LICENSE`, `README.md`, `THIRD_PARTY_NOTICES.md` and `deploy/` |
| `latest.json` | the update manifest the app reads (D97): `version`, `url` of the release page, `published`, and every file with its SHA-256 and size |
| `SHA256SUMS` | the SHA-256 of every file above |
| `<file>.minisig` | the minisign signature of each file above, `SHA256SUMS` and `latest.json` included |

The draft's notes are the version's section of [`CHANGELOG.md`](../CHANGELOG.md)
(its heading `## [<version>]`, `scripts/release-notes.sh`), followed by the
list of files and how to check them; a version without a section stops the
workflow at its first job.

Nothing is published by the workflow: the owner checks the draft and
publishes it. GitHub adds the source archives of the tag, which the
server's license (AGPL-3.0) asks for.

The release builds without a build cache, from the tag alone. The Windows
executable is not code-signed (D7): SmartScreen may ask once. The Linux
package is built in a pinned Ubuntu 22.04 image, not on the runner's newer
system, so it runs on Ubuntu 22.04 and later and Debian 12 and later
(`scripts/package-linux.sh` refuses an executable that needs a newer glibc).

### A dry run

`scripts\release-dry-run.ps1` builds the same files locally from a clean
checkout of a commit (default `HEAD`), **without signing or publishing**:
the Windows zip and its symbols, the Linux package in the workflow's image,
the server image and its archive, `latest.json`, `SHA256SUMS` and the
CHANGELOG part of the draft's notes (`notes.md`; the workflow adds the list
of files). It checks what the workflow checks (the versions, the public
key, the notes), that the table above has a row for every file, and that
each archive holds exactly the files the script lists for it (lists written
after the table, kept in step with it by hand). It needs Docker and the network; the
files land in `target\release-dry-run\dist`. Unlike the workflow it keeps
its build folders between runs (`-Fresh` empties them).

```
powershell -ExecutionPolicy Bypass -File scripts\release-dry-run.ps1
```

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
   `server/Cargo.toml` (`[package] version`), update both lock files
   (`cargo update -w` and `cargo update -w --manifest-path server/Cargo.toml`),
   and write the version's section in `CHANGELOG.md` (`## [<version>]`).
2. Let the change pass the main criterion of `CLAUDE.md`, the dry run
   (`scripts\release-dry-run.ps1`) and CI, and merge it into `main`.
3. Tag the commit and push the tag (the owner only):

   ```
   git tag -a v0.2.0 -m "Shell++ 0.2.0"
   git push origin v0.2.0
   ```

4. The workflow "Release" checks that the tag matches both versions, that
   `release/minisign.pub` holds a key and that `CHANGELOG.md` has the
   version's notes, builds the Windows zip, the Linux package and the
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
- and the manifest's `version` is newer than the app's by semver, and not
  a pre-release when the app is a stable version (D225): a stable build is
  never offered a release candidate, whatever source it reads; a release
  candidate is offered a newer one or the release.

When "latest" changes between the two requests (a release published in
that second), the app asks for both once more. A signature that the server
cannot send right now (5xx, 429) is "the update source did not answer", not
an unsigned update. A manifest address that answers 404 is "the update
source has no published release yet": GitHub's "latest" never points at a
pre-release, so until the first stable release is published every build of
a release candidate (`1.0.0-rc.1`) gets this answer, and a release candidate
is never told about the next one.

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

## Going public

The owner's steps, in this order, to publish the repository and release
1.0.0-rc.1 (D225). The repository becomes public with its whole history;
run 17 checked it (nothing private to remove, no rewrite).

1. **A rewritten history, if one was prepared** (D225, item 2): replace the
   history first, before anything else is public, with a new repository on
   GitHub (or delete the old one and create it again), not a force push:
   GitHub keeps the objects a force push leaves behind reachable by their
   hash. Then push the rewritten history to `devserver` too and clone it
   afresh.
2. **The signing key.** `release/minisign.pub` holds the owner's public key
   since run 17. If it ever has no key: make one ("Making the key" above),
   commit `release/minisign.pub`, run `scripts\verify.ps1` on that commit,
   and push `main`. The secret half and its password stay offline.
3. **Check before the push:** run `scripts\verify.ps1` on the commit of
   `main` once more (it scans the history of every ref), then push only
   `main` (D225, item 18): `wip/run9-archive`, the run branches and local
   tags stay local.
4. **Clean up GitHub while it is still private:** delete the old workflow
   runs and their artifacts (Actions, each workflow, "Delete workflow run";
   or `gh run list --limit 500 --json databaseId --jq '.[].databaseId'`
   and `gh run delete <id>` for each), and check that no issue, pull
   request, release, wiki page or other branch holds anything private:
   everything there becomes public with the repository.
5. **Make the repository public:** Settings, General, "Change repository
   visibility".
6. **Turn on GitHub Actions** (Settings, Actions, General) and let CI run
   on `main`; public repositories get the standard runners free (D210).
7. **Turn on private vulnerability reporting** (Settings, Code security),
   which `SECURITY.md` points to.
8. **The environment `release`** (Settings, Environments): the owner as its
   required reviewer and the secrets `MINISIGN_SECRET_KEY` and
   `MINISIGN_PASSWORD` ("The signing key", step 2).
9. **Rules** (Settings, Rules, Rulesets): for `main`, block force pushes
   and deletion only, with no required pull request or status check, so a
   fast-forward push of a run's result still works; for tags `v*`, restrict
   creation, update and deletion to the owner (a bypass for the repository
   admin), since a tag starts the release workflow and its secrets.
10. **Tag** `v1.0.0-rc.1` on `main` ("Making a release", step 3) and approve
    the environment `release` when the workflow waits for it.
11. **Check the draft** ("Making a release", step 5): download the files,
    check them as in "Checking a download", read the notes, and publish it.
    The tag has a `-`, so it is a pre-release: it is not GitHub's "latest",
    and no installed app is offered it (D225, item 11).

## From a release candidate to 1.0.0

Release 1.0.0 follows 1.0.0-rc.1 once the owner has checked it on the
owner's own installation (D225, item 15; ROADMAP 2.3, "Not obtained"):

1. Install the server of 1.0.0-rc.1 at home as `server/README.md` says
   (your own domain behind Caddy with Let's Encrypt, D204) and connect two
   devices: register on one, sign in on the other, edit on both, look at a
   conflict copy, show the recovery key, back the server up and restore it.
2. What the check finds goes into a fix and, if needed, another release
   candidate (`1.0.0-rc.2`, the same steps).
3. Then version `1.0.0` in a commit of its own ("Making a release", steps
   1 to 5), with its section `## [1.0.0]` in `CHANGELOG.md`. It is not a
   pre-release, so it becomes GitHub's "latest", and the builds of the
   release candidates are offered it.
